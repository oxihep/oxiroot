//! A user-defined class written by official ROOT (with a rootcling dictionary).
//! ROOT splits a class with a dictionary into a Record of named sub-fields, so
//! it reads back through the existing recursive Record/Collection path.
use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_rntuple::NtupleReader;
use std::path::PathBuf;
fn open(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open")
}
#[test]
fn reads_user_class() {
    let file = open("rntuple_user_uncompressed.root");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    // Top-level struct -> Record of named sub-fields.
    assert_eq!(
        ntpl.field("hit").values().expect("hit"),
        Column::Struct(vec![
            ("id".into(), Column::I32(vec![0, 1, 2])),
            ("energy".into(), Column::F64(vec![0.5, 1.5, 2.5])),
        ])
    );
    // vector<struct> -> Nested over a Record.
    assert_eq!(
        ntpl.field("vhit").values().expect("vhit"),
        Column::List {
            offsets: vec![0, 1, 3, 6],
            values: Box::new(Column::Struct(vec![
                ("id".into(), Column::I32(vec![0, 0, 2, 0, 2, 4])),
                (
                    "energy".into(),
                    Column::F64(vec![0.0, 0.0, 1.0, 0.0, 1.0, 2.0])
                ),
            ])),
        }
    );
}
