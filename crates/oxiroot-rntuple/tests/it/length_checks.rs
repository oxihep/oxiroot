//! The writer rejects fields whose data does not line up — top-level fields of
//! different lengths, and composite fields whose parts disagree — instead of
//! writing a file whose columns do not match its entry count.
use oxiroot_io_core::{Column, DataType};

use crate::support::write_rntuple_file;
use oxiroot_io_core::{Compression, Error, FileReader};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn write(fields: &[Field]) -> oxiroot_io_core::Result<()> {
    let path = std::env::temp_dir().join("oxiroot_rntuple_length_checks.root");
    write_rntuple_file(&path, "events", fields, Compression::Stored)
}

fn mismatch(what: &str, expected: usize, found: usize) -> Error {
    Error::LengthMismatch {
        what: what.into(),
        expected,
        found,
    }
}

#[test]
fn fields_of_different_lengths_are_rejected() {
    let err = write(&[
        Field::new("x", vec![1, 2, 3]),
        Field::new("y", vec![1.0, 2.0]),
    ])
    .unwrap_err();
    assert_eq!(
        format!("{:?}", err),
        format!("{:?}", mismatch("field \"y\" entries", 3, 2))
    );
}

/// The error a column that does not hold together is refused with: it names
/// the field and says what is wrong.
fn refused(column: Column, field: &str, why: &str) {
    let err = write(&[Field::new(field, column)]).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidInput(m) if m.contains(field) && m.contains(why)),
        "{err:?}"
    );
}

#[test]
fn record_members_of_different_lengths_are_rejected() {
    let record = Column::Struct(vec![
        ("a".into(), Column::I32(vec![1, 2])),
        ("b".into(), Column::F64(vec![1.0])),
    ]);
    refused(record, "r", "member \"b\" has 1 entries, not 2");
}

#[test]
fn collection_offsets_must_end_at_the_item_count() {
    let values = || Box::new(Column::F64(vec![1.0, 2.0]));
    let list = |offsets| Column::List {
        offsets,
        values: values(),
    };
    refused(list(vec![0, 2, 3]), "v", "offsets must end at 2");
    // The offsets bound each entry, so they start at the first entry's start.
    refused(list(vec![1, 2]), "v", "start with 0");
    refused(list(vec![0, 2, 1]), "v", "decrease");

    // A map's keys and values are record members: they must match too.
    let map = Column::List {
        offsets: vec![0, 2],
        values: Box::new(Column::Struct(vec![
            ("_0".into(), Column::I32(vec![1, 2])),
            ("_1".into(), Column::F64(vec![0.5])),
        ])),
    };
    refused(map, "m", "member \"_1\" has 1 entries, not 2");
}

#[test]
fn variant_tags_must_match_their_alternatives() {
    // Entry 1 holds alternative 2, which has no value.
    let short = Column::variant(vec![Column::I32(vec![1]), Column::F64(vec![])], vec![1, 2]);
    refused(short, "v", "alternative 2 has no value 0");
    refused(
        Column::variant(vec![Column::I32(vec![1])], vec![3]),
        "v",
        "no alternative 3",
    );
}

#[test]
fn optional_values_must_match_the_presence_mask() {
    let optional = Column::Optional {
        valid: vec![true, false, true],
        values: Box::new(Column::F64(vec![1.0])),
    };
    refused(optional, "o", "2 valid entries but 1 values");
}

#[test]
fn fixed_size_fields_hold_whole_arrays() {
    // An array or a bitset whose values do not divide into whole entries.
    let array = Column::Array {
        len: 2,
        values: Box::new(Column::F64(vec![1.0, 2.0, 3.0])),
    };
    refused(array, "a", "whole arrays of 2");
    let bits = Column::Array {
        len: 2,
        values: Box::new(Column::Bool(vec![true, true, false])),
    };
    let err = write(&[Field::new("b", bits).stored_as(DataType::Bitset(2))]).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidInput(m) if m.contains("whole arrays")),
        "{err:?}"
    );
}

#[test]
fn a_storage_type_of_another_shape_is_refused() {
    // A list cannot be stored as a half-precision float.
    let err = write(&[Field::new("v", vec![vec![1.0_f32]]).stored_as(DataType::Half)]).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidInput(m) if m.contains("\"v\"")),
        "{err:?}"
    );
    // A truncated float keeps 10 to 31 bits.
    let narrow = Field::new("t", vec![1.0_f32]).stored_as(DataType::Truncated { bits: 4 });
    let err = write(&[narrow]).unwrap_err();
    assert!(
        matches!(&err, Error::Unsupported(m) if m.contains("10 to 31")),
        "{err:?}"
    );
}

#[test]
fn a_zero_size_array_does_not_set_the_entry_count() {
    // `std::array<T, 0>` holds no values, whatever the entry count, so it neither
    // conflicts with the other fields nor, when first, decides the entry count.
    let path = std::env::temp_dir().join("oxiroot_rntuple_zero_array.root");
    let fields = [
        Field::new(
            "empty",
            Column::Array {
                len: 0,
                values: Box::new(Column::F64(vec![])),
            },
        ),
        Field::new("x", vec![1, 2, 3]),
    ];
    write_rntuple_file(&path, "events", &fields, Compression::Stored).unwrap();
    let f = FileReader::open(&path).unwrap();
    let nt = NtupleReader::open(&f, "events").unwrap();
    assert_eq!(nt.entries(), 3);
    assert_eq!(nt.field("x").values().unwrap(), Column::I32(vec![1, 2, 3]));
}
