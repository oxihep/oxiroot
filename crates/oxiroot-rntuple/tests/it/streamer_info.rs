//! `RNTuple` files carry the streamer info ROOT writes: the `ROOT::RNTuple` anchor
//! class, plus each user class with plain-number members. The entries must be
//! the ones ROOT wrote for the same classes in `rntuple_user_uncompressed.root`.
use oxiroot_io_core::Column;

use crate::support::NtupleObject;
use std::path::PathBuf;

use oxiroot_io_core::{Compression, FileReader, FileWriter, StreamerRegistry};
use oxiroot_rntuple::raw::{Field, NtupleWriter};

fn root_written() -> StreamerRegistry {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rntuple_user_uncompressed.root");
    FileReader::open(path)
        .unwrap()
        .streamer_registry()
        .unwrap()
        .clone()
}

fn hit(ids: Vec<i32>) -> Column {
    let energy = ids.iter().map(|&i| f64::from(i) + 0.5).collect();
    Column::Struct(vec![
        ("id".into(), Column::I32(ids)),
        ("energy".into(), Column::F64(energy)),
    ])
}

fn registry(bytes: Vec<u8>) -> StreamerRegistry {
    FileReader::from_bytes(bytes)
        .unwrap()
        .streamer_registry()
        .unwrap()
        .clone()
}

#[test]
fn one_shot_file_describes_the_anchor_and_user_classes_like_root() {
    let fields = vec![
        Field::new("hit", hit(vec![0, 1])).class_name("Hit"),
        // The same class again, inside a vector: described once.
        Field::new(
            "vhit",
            Column::List {
                offsets: vec![0, 1, 3],
                values: Box::new(hit(vec![0, 1, 2])),
            },
        )
        .class_name("Hit"),
        Field::new("x", Column::F32(vec![1.0, 2.0])),
    ];
    let bytes = NtupleObject::new("ntpl", fields)
        .to_root_bytes("t.root", Compression::Zstd(5))
        .unwrap();
    let ours = registry(bytes);
    let root = root_written();
    assert_eq!(
        ours.class_names().collect::<Vec<_>>(),
        ["ROOT::RNTuple", "Hit"]
    );
    assert_eq!(ours.get("ROOT::RNTuple"), root.get("ROOT::RNTuple"));
    assert_eq!(ours.get("Hit"), root.get("Hit"));
}

#[test]
fn streamed_file_describes_the_anchor_and_user_classes() {
    let path = std::env::temp_dir().join("oxiroot_rntuple_stream_streamers.root");
    let mut w = NtupleWriter::create(&path, "ntpl", Compression::Stored).unwrap();
    for k in 0..2 {
        w.write_batch(&[Field::new("hit", hit(vec![k, k + 1])).class_name("Hit")])
            .unwrap();
    }
    w.finish().unwrap();
    let ours = FileReader::open(&path)
        .unwrap()
        .streamer_registry()
        .unwrap()
        .clone();
    let root = root_written();
    assert_eq!(
        ours.class_names().collect::<Vec<_>>(),
        ["ROOT::RNTuple", "Hit"]
    );
    assert_eq!(ours.get("Hit"), root.get("Hit"));
}

#[test]
fn classes_with_other_members_are_not_described() {
    // A string member has no plain-number streamer element here, so the class is
    // left to the reader's dictionary; the anchor class is still described.
    let tagged = Column::Struct(vec![
        ("id".into(), Column::I32(vec![1])),
        ("label".into(), Column::from(vec!["a".to_string()])),
    ]);
    let bytes = FileWriter::create("t.root")
        .add(NtupleObject::new(
            "a",
            vec![Field::new("t", tagged).class_name("Tagged")],
        ))
        .dir("d", |d| {
            d.add(NtupleObject::new(
                "b",
                vec![Field::new("hit", hit(vec![3])).class_name("Hit")],
            ))
        })
        .compression(Compression::Stored)
        .to_bytes()
        .unwrap();
    assert_eq!(
        registry(bytes).class_names().collect::<Vec<_>>(),
        ["ROOT::RNTuple", "Hit"]
    );
}

#[test]
fn schema_extended_files_are_described_too() {
    let path = std::env::temp_dir().join("oxiroot_rntuple_ext_streamers.root");
    NtupleObject::new(
        "ext",
        vec![Field::new("hit", hit(vec![1, 2])).class_name("Hit")],
    )
    .write_root_extended(
        &path,
        &[(1, Field::new("y", Column::F64(vec![0.5])))],
        Compression::Stored,
    )
    .unwrap();
    let ours = FileReader::open(&path)
        .unwrap()
        .streamer_registry()
        .unwrap()
        .clone();
    assert_eq!(
        ours.class_names().collect::<Vec<_>>(),
        ["ROOT::RNTuple", "Hit"]
    );
}
