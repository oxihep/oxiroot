//! Nullable / "late" `RNTuple` fields — `std::optional<T>` and `std::unique_ptr<T>`
//! — plus `std::atomic<T>`. On disk an `optional/unique_ptr` is a `Collection` of
//! 0-or-1 elements; oxiroot reads it back as a nullable [`Column::Opt`].
//! `std::atomic<T>` is stored as the bare `T`. Round-tripped here; the written
//! files are also read by ROOT C++ (`RNTupleReader`) and uproot.
use oxiroot_io_core::Column;
use oxiroot_io_core::DataType;

use crate::support::NtupleObject;
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn write(path: &str, compression: Compression) {
    oxiroot_io_core::FileWriter::create(path)
        .add(NtupleObject::new(
            "ntpl",
            vec![
                Field::new("maybe", vec![Some(1.5_f32), None, Some(3.5), None]),
                Field::new("late", vec![Some(10), None, Some(30), None])
                    .stored_as(DataType::UniquePtr(Box::new(DataType::I32))),
                Field::new("counter", vec![0, 100, 200, 300])
                    .stored_as(DataType::Atomic(Box::new(DataType::I32))),
            ],
        ))
        .compression(compression)
        .write()
        .unwrap();
}

#[test]
fn optional_and_unique_ptr_round_trip_as_nullable() {
    let path = std::env::temp_dir().join("oxiroot_opt.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Stored);
    let f = FileReader::open(path).unwrap();
    let nt = NtupleReader::open(&f, "ntpl").unwrap();

    // optional<float> reads back as a nullable.
    let maybe = nt.field("maybe").values().unwrap();
    assert!(matches!(maybe, Column::Optional { .. }));
    assert_eq!(
        nt.field("maybe").read::<Option<f32>>().unwrap(),
        vec![Some(1.5), None, Some(3.5), None]
    );
    // unique_ptr<int> reads back as a nullable too.
    assert_eq!(
        nt.field("late").read::<Option<i32>>().unwrap(),
        vec![Some(10), None, Some(30), None]
    );
}

#[test]
fn atomic_reads_as_a_scalar() {
    let path = std::env::temp_dir().join("oxiroot_atomic.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Stored);
    let f = FileReader::open(path).unwrap();
    let nt = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(
        nt.field("counter").values().unwrap(),
        Column::I32(vec![0, 100, 200, 300])
    );
}

#[test]
fn nullable_fields_compress() {
    let path = std::env::temp_dir().join("oxiroot_opt_zstd.root");
    let path = path.to_str().unwrap();
    write(path, Compression::Zstd(5));
    let f = FileReader::open(path).unwrap();
    let nt = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(
        nt.field("maybe").read::<Option<f32>>().unwrap(),
        vec![Some(1.5), None, Some(3.5), None]
    );
    assert_eq!(
        nt.field("counter").values().unwrap(),
        Column::I32(vec![0, 100, 200, 300])
    );
}

#[test]
fn all_present_and_all_absent() {
    let path = std::env::temp_dir().join("oxiroot_opt_edge.root");
    let path = path.to_str().unwrap();
    oxiroot_io_core::FileWriter::create(path)
        .add(NtupleObject::new(
            "ntpl",
            vec![
                Field::new("full", vec![Some(1.0), Some(2.0), Some(3.0)]),
                Field::new("empty", vec![None::<f64>, None, None]),
            ],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let f = FileReader::open(path).unwrap();
    let nt = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(
        nt.field("full").read::<Option<f64>>().unwrap(),
        vec![Some(1.0), Some(2.0), Some(3.0)]
    );
    assert_eq!(
        nt.field("empty").read::<Option<f64>>().unwrap(),
        vec![None, None, None]
    );
}
