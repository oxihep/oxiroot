//! Concatenating `RNTuples` with [`append_ntuples`] — the `hadd` building block.
use std::io::Cursor;

use crate::support::NtupleObject;
use oxiroot_io_core::{Compression, FileReader, Result};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::{Field, FieldValues, NtupleWriter, append_ntuples};

/// Merge `inputs` into one in-memory file holding `RNTuple` `name`.
fn concat_ntuples(name: &str, inputs: &[&NtupleReader<'_>]) -> Result<Vec<u8>> {
    let mut w = NtupleWriter::new(
        Cursor::new(Vec::new()),
        "merged.root",
        name,
        Compression::Stored,
    )?;
    append_ntuples(&mut w, inputs)?;
    Ok(w.finish()?.into_inner())
}

/// Write `fields` as `RNTuple` `ntpl` to a fresh temp file and reopen it.
fn write_open(tag: &str, fields: Vec<Field>) -> FileReader {
    let path = std::env::temp_dir().join(format!("oxiroot_rnconcat_{tag}.root"));
    oxiroot_io_core::FileWriter::create(&path)
        .add(NtupleObject::new("ntpl", fields))
        .compression(Compression::Stored)
        .write()
        .expect("write");
    FileReader::open(&path).expect("reopen")
}

#[test]
fn concatenates_scalar_vector_and_string_fields() {
    let fa = write_open(
        "a",
        vec![
            Field::i32("i", vec![0, 1, 2]),
            Field::f64("x", vec![0.0, 1.0, 2.0]),
            Field::string("s", vec!["a".into(), "b".into(), "c".into()]),
            Field::vector_i32("v", vec![vec![1], vec![], vec![2, 3]]),
        ],
    );
    let fb = write_open(
        "b",
        vec![
            Field::i32("i", vec![3, 4]),
            Field::f64("x", vec![3.0, 4.0]),
            Field::string("s", vec!["d".into(), "e".into()]),
            Field::vector_i32("v", vec![vec![4, 5], vec![6]]),
        ],
    );
    let na = NtupleReader::open(&fa, "ntpl").unwrap();
    let nb = NtupleReader::open(&fb, "ntpl").unwrap();

    let merged = concat_ntuples("ntpl", &[&na, &nb]).expect("concat");
    let fo = FileReader::from_bytes(merged).unwrap();
    let no = NtupleReader::open(&fo, "ntpl").unwrap();

    assert_eq!(no.entries(), 5);
    assert_eq!(
        no.field("i").values().unwrap(),
        FieldValues::I32(vec![0, 1, 2, 3, 4])
    );
    assert_eq!(
        no.field("x").values().unwrap(),
        FieldValues::F64(vec![0.0, 1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(
        no.field("s").values().unwrap(),
        FieldValues::Str(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into()
        ])
    );
    assert_eq!(
        no.field("v").values().unwrap(),
        FieldValues::VecI32(vec![vec![1], vec![], vec![2, 3], vec![4, 5], vec![6]])
    );
}

#[test]
fn rejects_a_type_mismatch() {
    let fa = write_open("ty_a", vec![Field::i32("v", vec![1, 2])]);
    let fb = write_open("ty_b", vec![Field::f64("v", vec![3.0])]);
    let na = NtupleReader::open(&fa, "ntpl").unwrap();
    let nb = NtupleReader::open(&fb, "ntpl").unwrap();

    let Err(err) = concat_ntuples("ntpl", &[&na, &nb]) else {
        panic!("expected an error for a type mismatch");
    };
    assert!(
        matches!(err, oxiroot_io_core::Error::SchemaChanged { .. }),
        "{err}"
    );
}
