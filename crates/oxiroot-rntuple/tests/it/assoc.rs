//! Associative containers. `std::set` and `std::map` round-trip both ways: ROOT
//! writes the `rntuple_set_*` and `rntuple_map_*` fixtures (read here), and the
//! interop matrix has ROOT read oxiroot's own output; uproot reads the map too.
//! (ROOT 6.40 needs the map's dictionary compiled in to write or read a
//! `std::map` RNTuple: `scripts/rntuple_map_LinkDef.h`.)

use crate::support::NtupleObject;
use std::path::PathBuf;

use oxiroot_io_core::{Compression, FileReader};
use oxiroot_rntuple::{Field, FieldValues, NtupleReader, StructRole};

fn open(name: &str) -> FileReader {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    FileReader::open(path).expect("open fixture")
}

/// A `std::set<int32_t>` ROOT wrote reads as a collection of its elements.
fn check_set_fixture(name: &str) {
    let file = open(name);
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(ntpl.entries(), 4, "{name}");
    // ROOT records the field as a Collection named with the set type.
    let s = &ntpl.header().fields[0];
    assert_eq!(s.struct_role, StructRole::Collection);
    assert_eq!(s.type_name, "std::set<std::int32_t>");
    assert_eq!(
        ntpl.field("s").values().unwrap(),
        FieldValues::VecI32(vec![
            vec![1, 2, 3],
            vec![4, 5],
            vec![],
            vec![10, 20, 30, 40]
        ]),
        "{name}"
    );
}

#[test]
fn reads_root_set_uncompressed() {
    check_set_fixture("rntuple_set_uncompressed.root");
}

#[test]
fn reads_root_set_zstd() {
    check_set_fixture("rntuple_set_zstd.root");
}

/// A `std::map<int32_t, double>` ROOT wrote reads as a collection of key/value
/// records, sorted by key as ROOT stores them.
fn check_map_fixture(name: &str) {
    let file = open(name);
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open ntpl");
    assert_eq!(ntpl.entries(), 3, "{name}");
    let m = &ntpl.header().fields[0];
    assert_eq!(m.struct_role, StructRole::Collection);
    assert_eq!(m.type_name, "std::map<std::int32_t,double>");
    match ntpl.field("m").values().unwrap() {
        FieldValues::Nested { offsets, items } => {
            assert_eq!(offsets, vec![0, 2, 2, 5], "{name}");
            assert_eq!(
                *items,
                FieldValues::Record(vec![
                    ("_0".into(), FieldValues::I32(vec![1, 2, -7, 3, 40])),
                    (
                        "_1".into(),
                        FieldValues::F64(vec![1.5, 2.5, 0.25, 3.5, -1.0])
                    ),
                ]),
                "{name}"
            );
        }
        other => panic!("{name}: expected a nested map collection, got {other:?}"),
    }
}

#[test]
fn reads_root_map_uncompressed() {
    check_map_fixture("rntuple_map_uncompressed.root");
}

#[test]
fn reads_root_map_zstd() {
    check_map_fixture("rntuple_map_zstd.root");
}

/// oxiroot writes a `std::set` that round-trips (and ROOT C++ reads it — checked
/// out of band). The on-disk schema is a collection tagged with the set type.
#[test]
fn writes_set_round_trip() {
    let nt = NtupleObject::new(
        "ntpl",
        vec![Field::set_i32("s", vec![vec![1, 2, 3], vec![4, 5]])],
    );
    let out = std::env::temp_dir().join("oxiroot_set_rt.root");
    nt.write_root(&out, Compression::None).expect("write");
    let file = FileReader::open(&out).unwrap();
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();
    assert_eq!(ntpl.header().fields[0].type_name, "std::set<std::int32_t>");
    assert_eq!(
        ntpl.field("s").values().unwrap(),
        FieldValues::VecI32(vec![vec![1, 2, 3], vec![4, 5]])
    );
    let _ = std::fs::remove_file(&out);
}

/// oxiroot writes a `std::map<int32_t, double>` (a collection of key/value
/// records, tagged with the map type) that round-trips; uproot reads it as pairs.
#[test]
fn writes_map_round_trip() {
    let nt = NtupleObject::new(
        "ntpl",
        vec![Field::map_i32_f64(
            "m",
            vec![vec![(1, 1.5), (2, 2.5)], vec![(3, 3.5)]],
        )],
    );
    let out = std::env::temp_dir().join("oxiroot_map_rt.root");
    nt.write_root(&out, Compression::None).expect("write");
    let file = FileReader::open(&out).unwrap();
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();

    let m = &ntpl.header().fields[0];
    assert_eq!(m.struct_role, StructRole::Collection);
    assert_eq!(m.type_name, "std::map<std::int32_t,double>");
    // The element is a std::pair record of key (`_0`) and value (`_1`).
    assert_eq!(
        ntpl.header().fields[1].type_name,
        "std::pair<std::int32_t,double>"
    );

    // Read back as a nested collection of key/value records.
    match ntpl.field("m").values().unwrap() {
        FieldValues::Nested { offsets, items } => {
            assert_eq!(offsets, vec![0, 2, 3]);
            assert_eq!(
                *items,
                FieldValues::Record(vec![
                    ("_0".into(), FieldValues::I32(vec![1, 2, 3])),
                    ("_1".into(), FieldValues::F64(vec![1.5, 2.5, 3.5])),
                ])
            );
        }
        other => panic!("expected a nested map collection, got {other:?}"),
    }
    let _ = std::fs::remove_file(&out);
}
