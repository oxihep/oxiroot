//! Several `RNTuples` in one file, and an `RNTuple` inside a `TDirectory`
//! (`FileWriter::put`). Round-tripped through oxiroot here; the same files are
//! read by ROOT C++ (`RNTupleReader`) and uproot.

use crate::support::NtupleObject;
use oxiroot_io_core::{Compression, FileReader, FileWriter};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::{Field, FieldValues};

fn write(path: &str, compression: Compression) {
    FileWriter::create(path)
        .add(NtupleObject::new(
            "events",
            vec![
                Field::i32("x", vec![1, 2, 3]),
                Field::f64("y", vec![0.5, 1.5, 2.5]),
            ],
        ))
        .add(NtupleObject::new(
            "runs",
            vec![Field::i32("run", vec![7, 8])],
        ))
        .dir("cal", |d| {
            d.add(NtupleObject::new(
                "pedestals",
                vec![Field::f64("p", vec![9.5, 8.5])],
            ))
        })
        .compression(compression)
        .write()
        .unwrap();
}

#[test]
fn several_rntuples_in_top_directory() {
    let path = std::env::temp_dir().join("oxiroot_multi_top.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Stored);
    let f = FileReader::open(path).unwrap();

    // Both top-directory RNTuples open and read.
    let events = NtupleReader::open(&f, "events").unwrap();
    assert_eq!(events.entries(), 3);
    assert_eq!(events.field_names().collect::<Vec<_>>(), vec!["x", "y"]);
    assert_eq!(
        events.field("x").values().unwrap(),
        FieldValues::I32(vec![1, 2, 3])
    );
    let runs = NtupleReader::open(&f, "runs").unwrap();
    assert_eq!(runs.entries(), 2);
    assert_eq!(
        runs.field("run").values().unwrap(),
        FieldValues::I32(vec![7, 8])
    );
}

#[test]
fn rntuple_inside_a_subdirectory() {
    let path = std::env::temp_dir().join("oxiroot_multi_sub.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Stored);
    let f = FileReader::open(path).unwrap();

    // The subdirectory is navigable and its RNTuple opens by its path.
    let cal = f.subdir("cal").unwrap();
    assert!(cal.keys.iter().any(|k| k.name == "pedestals"));
    let ped = NtupleReader::open(&f, "cal/pedestals").unwrap();
    assert_eq!(ped.entries(), 2);
    assert_eq!(
        ped.field("p").values().unwrap(),
        FieldValues::F64(vec![9.5, 8.5])
    );
}

#[test]
fn compressed_multi_file_round_trips() {
    let path = std::env::temp_dir().join("oxiroot_multi_zstd.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Zstd(5));
    let f = FileReader::open(path).unwrap();
    assert_eq!(NtupleReader::open(&f, "events").unwrap().entries(), 3);
    assert_eq!(
        NtupleReader::open(&f, "cal/pedestals")
            .unwrap()
            .field("p")
            .values()
            .unwrap(),
        FieldValues::F64(vec![9.5, 8.5])
    );
}

#[test]
fn duplicate_names_are_rejected() {
    let dup = FileWriter::create("f.root")
        .add(NtupleObject::new("a", vec![Field::i32("x", vec![1])]))
        .add(NtupleObject::new("a", vec![Field::i32("x", vec![2])]));
    assert!(dup.compression(Compression::Stored).to_bytes().is_err());

    // A subdirectory name colliding with a top-level RNTuple is also rejected.
    let clash = FileWriter::create("f.root")
        .add(NtupleObject::new("cal", vec![Field::i32("x", vec![1])]))
        .dir("cal", |d| {
            d.add(NtupleObject::new("p", vec![Field::i32("y", vec![1])]))
        });
    assert!(clash.compression(Compression::Stored).to_bytes().is_err());
}
