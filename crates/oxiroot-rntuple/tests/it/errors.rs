//! The typed errors a caller can match on when reading an `RNTuple`: a missing
//! `RNTuple`, field or column, a key holding another class, and corrupt data.

use crate::support::rntuple_file_bytes;
use oxiroot_io_core::{Compression, Error, FileReader, FileWriter, ObjString};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn bytes() -> Vec<u8> {
    let fields = [Field::f64("x", vec![1.0, 2.0, 3.0])];
    rntuple_file_bytes("f.root", "events", &fields, Compression::Stored).unwrap()
}

#[test]
fn a_missing_rntuple_field_or_column_is_not_found() {
    let f = FileReader::from_bytes(bytes()).unwrap();
    let Err(err) = NtupleReader::open(&f, "nope") else {
        panic!("a missing RNTuple opened");
    };
    assert!(
        matches!(&err, Error::NotFound { what: "key", name } if name == "nope"),
        "{err:?}"
    );

    let ntpl = NtupleReader::open(&f, "events").unwrap();
    let err = ntpl.field("y").values().unwrap_err();
    assert!(
        matches!(&err, Error::NotFound { what: "top-level field", name } if name == "y"),
        "{err:?}"
    );
    let err = ntpl.read_column(99).unwrap_err();
    assert!(
        matches!(&err, Error::NotFound { what: "column", name } if name == "99"),
        "{err:?}"
    );
}

#[test]
fn a_key_that_is_not_an_rntuple_is_the_wrong_class() {
    let bytes = FileWriter::create("unused.root")
        .add(&ObjString::new("hi").named("s"))
        .compression(Compression::Stored)
        .to_bytes()
        .unwrap();
    let f = FileReader::from_bytes(bytes).unwrap();
    let Err(err) = NtupleReader::open(&f, "s") else {
        panic!("a TObjString opened as an RNTuple");
    };
    assert_eq!(
        format!("{:?}", err),
        format!(
            "{:?}",
            Error::WrongClass {
                name: Some("s".into()),
                found: "TObjString".into(),
                expected: "ROOT::RNTuple".into()
            }
        )
    );
}

#[test]
fn a_corrupt_header_envelope_is_a_checksum_mismatch() {
    let good = bytes();
    let seek = {
        let f = FileReader::from_bytes(good.clone()).unwrap();
        let seek = NtupleReader::open(&f, "events")
            .unwrap()
            .anchor()
            .seek_header;
        usize::try_from(seek).expect("a small test file")
    };
    // A byte in the header envelope's payload, ahead of its trailing checksum.
    let mut bad = good;
    bad[seek + 16] ^= 0xff;
    let f = FileReader::from_bytes(bad).unwrap();
    let Err(err) = NtupleReader::open(&f, "events") else {
        panic!("a corrupt RNTuple opened");
    };
    assert!(
        matches!(&err, Error::ChecksumMismatch { what, computed, stored }
            if what == "RNTuple envelope" && computed != stored),
        "{err:?}"
    );
}
