//! Small persistable objects: `ObjString` (a labelled string) and
//! `Parameter<T>` (a named scalar). oxiroot reads the ROOT-C++-written
//! `persist_objs.root` fixture and round-trips its own writes; ROOT C++ reads
//! oxiroot's output (checked out of band), and uproot reads the `TObjString`.

use std::path::PathBuf;

use oxiroot_io_core::{Compression, FileReader};
use oxiroot_io_core::{FileWriter, ObjString, ParamValue, Parameter};

fn fixture() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/persist_objs.root"),
    )
    .expect("open fixture")
}

#[test]
fn reads_root_written_objects() {
    let f = fixture();
    assert_eq!(f.get::<ObjString>("label").unwrap().value(), "hello world");
    assert_eq!(
        f.get::<Parameter>("lumi").unwrap().value(),
        ParamValue::Double(137.5)
    );
    assert_eq!(
        f.get::<Parameter>("nevents").unwrap().value(),
        ParamValue::Int(42)
    );
    assert_eq!(
        f.get::<Parameter>("bignum").unwrap().value(),
        ParamValue::Long64(9_000_000_000)
    );
}

#[test]
fn round_trips_objects_through_oxiroot() {
    let out = std::env::temp_dir().join("oxiroot_persist_rt.root");
    FileWriter::create(&out)
        .add(&ObjString::new("hello world").named("label"))
        .add(&Parameter::f64("lumi", 137.5))
        .add(&Parameter::i32("nevents", 42))
        .add(&Parameter::i64("bignum", 9_000_000_000))
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&out).unwrap();
    assert_eq!(f.get::<ObjString>("label").unwrap().value(), "hello world");
    assert_eq!(f.get::<Parameter>("lumi").unwrap().value().as_f64(), 137.5);
    assert_eq!(
        f.get::<Parameter>("nevents").unwrap().value(),
        ParamValue::Int(42)
    );
    assert_eq!(
        f.get::<Parameter>("bignum").unwrap().value(),
        ParamValue::Long64(9_000_000_000)
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn byte_exact_against_root() {
    // oxiroot's serialized object bytes must equal ROOT's, key-for-key.
    use oxiroot_io_core::WriteRoot;
    let f = fixture();
    let cases: [(&str, &dyn WriteRoot); 4] = [
        ("label", &ObjString::new("hello world").named("label")),
        ("lumi", &Parameter::f64("lumi", 137.5)),
        ("nevents", &Parameter::i32("nevents", 42)),
        ("bignum", &Parameter::i64("bignum", 9_000_000_000)),
    ];
    for (name, obj) in cases {
        let (_, root_bytes, _) = oxiroot_io_core::raw::object_bytes(&f, name).unwrap();
        assert_eq!(
            obj.to_root_bytes(),
            root_bytes,
            "object bytes differ for {name:?}"
        );
    }
}
