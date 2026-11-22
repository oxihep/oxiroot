//! An `RNTuple` next to other objects in one file, written without the histogram
//! crate: this crate depends only on io-core (and, for this test, linalg).
use oxiroot_io_core::Column;

use crate::support::NtupleObject;
use oxiroot_io_core::{Compression, FileReader, FileUpdater, FileWriter, Parameter};
use oxiroot_linalg::Matrix;
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

#[test]
fn a_matrix_a_parameter_and_an_rntuple_share_a_file() {
    let path = std::env::temp_dir().join("oxiroot_rntuple_mixed.root");
    let cov = Matrix::new(2, 2, vec![1.0, 0.5, 0.5, 2.0])
        .unwrap()
        .named("cov");
    FileWriter::create(&path)
        .add(&cov)
        .add(NtupleObject::new(
            "events",
            vec![Field::new("x", vec![0.5, 1.5])],
        ))
        .add(&Parameter::f64(12.5).named("lumi"))
        .dir("aux", |d| {
            d.add(NtupleObject::new("runs", vec![Field::new("run", vec![7])]))
        })
        .compression(Compression::Zstd(3))
        .write()
        .unwrap();

    let f = FileReader::open(&path).unwrap();
    let names: Vec<&str> = f.keys().iter().map(|k| k.name.as_str()).collect();
    assert_eq!(names, ["cov", "events", "lumi", "aux"]);
    assert_eq!(f.get::<Matrix>("cov").unwrap(), cov);
    assert_eq!(f.get::<Parameter>("lumi").unwrap().value().as_f64(), 12.5);
    assert_eq!(
        NtupleReader::open(&f, "events")
            .unwrap()
            .field("x")
            .values()
            .unwrap(),
        Column::F64(vec![0.5, 1.5])
    );
    assert_eq!(
        NtupleReader::open(&f, "aux/runs")
            .unwrap()
            .field("run")
            .values()
            .unwrap(),
        Column::I32(vec![7])
    );
    // Every class is described, and the histogram list is not needed.
    let registry = f.streamer_registry().unwrap();
    for class in ["ROOT::RNTuple", "TMatrixT<double>", "TParameter<double>"] {
        assert!(registry.get(class).is_some(), "{class} is described");
    }
    assert!(registry.get("TH1D").is_none());
}

#[test]
fn an_rntuple_can_be_appended_to_an_existing_file() {
    let path = std::env::temp_dir().join("oxiroot_rntuple_append_put.root");
    FileWriter::create(&path)
        .add(&Parameter::i32(3).named("n"))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    FileUpdater::open(&path)
        .unwrap()
        .add(NtupleObject::new("late", vec![Field::new("x", vec![4, 5])]))
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&path).unwrap();
    assert_eq!(f.get::<Parameter>("n").unwrap().value().as_f64(), 3.0);
    assert_eq!(
        NtupleReader::open(&f, "late")
            .unwrap()
            .field("x")
            .values()
            .unwrap(),
        Column::I32(vec![4, 5])
    );
    assert!(
        f.streamer_registry()
            .unwrap()
            .get("ROOT::RNTuple")
            .is_some()
    );
}

#[test]
fn a_forced_64_bit_file_with_rntuples_reads_back() {
    let path = std::env::temp_dir().join("oxiroot_rntuple_put_big.root");
    FileWriter::create(&path)
        .add(NtupleObject::new(
            "a",
            vec![Field::new("x", vec![1.0_f32, 2.0])],
        ))
        .dir("d", |d| {
            d.add(NtupleObject::new("b", vec![Field::new("y", vec![3_i64])]))
        })
        .compression(Compression::Stored)
        .large_above(0)
        .write()
        .unwrap();
    let f = FileReader::open(&path).unwrap();
    assert!(f.header().is_big());
    assert_eq!(
        NtupleReader::open(&f, "a")
            .unwrap()
            .field("x")
            .values()
            .unwrap(),
        Column::F32(vec![1.0, 2.0])
    );
    assert_eq!(
        NtupleReader::open(&f, "d/b")
            .unwrap()
            .field("y")
            .values()
            .unwrap(),
        Column::I64(vec![3])
    );
}
