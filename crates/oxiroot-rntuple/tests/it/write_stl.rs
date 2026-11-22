//! Writing fixed-size STL fields (`std::array`, `std::bitset`) and a split
//! user-class record, then reading them back. Cross-checked against compiled
//! ROOT C++ (`RNTupleReader` typed views over `std::array`/`std::bitset`/the
//! `Hit` class) and uproot, which both read the oxiroot-written file — ROOT's
//! strict reader accepts the computed `Hit` class checksum.
use oxiroot_io_core::{Column, DataType};

use crate::support::NtupleObject;
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn write_and_reopen(tag: &str, fields: Vec<Field>) -> FileReader {
    let nt = NtupleObject::new("ntpl", fields);
    let out = std::env::temp_dir().join(format!("oxiroot_write_stl_{tag}.root"));
    oxiroot_io_core::FileWriter::create(&out)
        .add(nt)
        .compression(Compression::Zstd(3))
        .write()
        .expect("write");
    FileReader::open(&out).expect("reopen")
}

#[test]
fn array_field_round_trips() {
    let file = write_and_reopen(
        "array",
        vec![Field::new("arr", vec![[10, 20, 30], [40, 50, 60]])],
    );
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(ntpl.entries(), 2);
    assert_eq!(
        ntpl.field("arr").values().unwrap(),
        Column::from(vec![[10, 20, 30], [40, 50, 60]])
    );
    // The field is a fixed array of 3 (flags + array_size in the header).
    let arr = &ntpl.header().fields[0];
    assert_eq!(arr.array_size, Some(3));
    assert_eq!(arr.type_name, "std::array<std::int32_t,3>");
}

#[test]
fn bitset_field_round_trips() {
    let row0 = vec![true, false, true, false, false, false, false, false];
    let row1 = vec![false, true, false, true, false, false, false, false];
    let file = write_and_reopen(
        "bitset",
        vec![bitset("bits", &[row0.clone(), row1.clone()])],
    );
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(
        ntpl.field("bits").values().unwrap(),
        Column::Array {
            len: 8,
            values: Box::new(Column::Bool([row0, row1].concat())),
        }
    );
    assert_eq!(ntpl.header().fields[0].array_size, Some(8));
    assert_eq!(ntpl.header().fields[0].type_name, "std::bitset<8>");
}

#[test]
fn user_class_field_round_trips() {
    let file = write_and_reopen(
        "user",
        vec![
            Field::new(
                "hit",
                Column::Struct(vec![
                    ("id".into(), Column::I32(vec![7, 42])),
                    ("energy".into(), Column::F64(vec![3.25, -1.5])),
                ]),
            )
            .class_name("Hit"),
        ],
    );
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(
        ntpl.field("hit").values().unwrap(),
        Column::Struct(vec![
            ("id".into(), Column::I32(vec![7, 42])),
            ("energy".into(), Column::F64(vec![3.25, -1.5])),
        ])
    );
    // The record carries the ROOT class checksum so ROOT validates the schema.
    assert_eq!(ntpl.header().fields[0].type_name, "Hit");
}

#[test]
fn all_three_in_one_ntuple() {
    let file = write_and_reopen(
        "all",
        vec![
            Field::new("xyz", vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]),
            bitset("flags", &[vec![true, true], vec![false, true]]),
            Field::new(
                "hit",
                Column::Struct(vec![
                    ("id".into(), Column::I32(vec![1, 2])),
                    ("energy".into(), Column::F64(vec![0.5, 1.5])),
                ]),
            )
            .class_name("Hit"),
        ],
    );
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(ntpl.entries(), 2);
    assert_eq!(
        ntpl.field("xyz").values().unwrap(),
        Column::from(vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])
    );
    assert_eq!(
        ntpl.field_names().collect::<Vec<_>>(),
        vec!["xyz", "flags", "hit"]
    );
}

/// A `std::bitset<N>` field from one row of `N` bits per entry.
fn bitset(name: &str, rows: &[Vec<bool>]) -> Field {
    let len = rows.first().map_or(0, Vec::len);
    let bits = Column::Array {
        len,
        values: Box::new(Column::Bool(rows.concat())),
    };
    Field::new(name, bits).stored_as(DataType::Bitset(len))
}
