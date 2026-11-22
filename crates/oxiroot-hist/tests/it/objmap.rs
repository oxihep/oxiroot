//! `ObjMap` — ROOT's keyed map of object → object. oxiroot reads the
//! ROOT-C++-written `tmap.root` fixture (string keys → a `TObjString`, a
//! `TParameter`, a TH1F) and round-trips its own writes; ROOT C++ reads oxiroot's
//! output (checked out of band). uproot has no `TMap` model, so an `ObjMap` is not
//! readable there — a uproot limitation that ROOT's own maps share.

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Hist, Hist1D};
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_io_core::{FileWriter, ObjMap, ObjString, Parameter};

fn fixture() -> FileReader {
    FileReader::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tmap.root"))
        .expect("open fixture")
}

#[test]
fn reads_root_written_map() {
    let f = fixture();
    let m = f.get::<ObjMap>("meta").unwrap();
    assert_eq!(m.name(), "meta");
    assert_eq!(m.len(), 3);
    assert_eq!(m.string_keys(), ["version", "lumi", "hist"]);
    assert_eq!(
        m.get::<ObjString>("version").unwrap().unwrap().value(),
        "2.1"
    );
    assert_eq!(
        m.get::<Parameter>("lumi")
            .unwrap()
            .unwrap()
            .value()
            .as_f64(),
        137.5
    );
    assert_eq!(m.values::<Hist1D>().unwrap().len(), 1);
    // A miss: no such key, or wrong value type.
    assert!(m.get::<ObjString>("missing").is_none());
    assert!(m.get::<Hist1D>("version").is_none());
}

#[test]
fn round_trips_map_through_oxiroot() {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("h");
    h.fill(0.5);
    let h = h.to_root().unwrap();
    let map = ObjMap::new()
        .named("meta")
        .insert("version", &ObjString::new("2.1"))
        .insert("lumi", &Parameter::f64(137.5).named("lumi"))
        .insert("hist", &h);

    let out = std::env::temp_dir().join("oxiroot_tmap_rt.root");
    FileWriter::create(&out)
        .add(&map)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&out).unwrap();
    let m = f.get::<ObjMap>("meta").unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m.string_keys(), ["version", "lumi", "hist"]);
    assert_eq!(
        m.get::<ObjString>("version").unwrap().unwrap().value(),
        "2.1"
    );
    assert_eq!(
        m.get::<Parameter>("lumi")
            .unwrap()
            .unwrap()
            .value()
            .as_f64(),
        137.5
    );
    assert_eq!(m.values::<Hist1D>().unwrap().len(), 1);
    let _ = std::fs::remove_file(&out);
}

/// A map read from a file is written back as it was read (its entries refer to
/// each other's class tags by position), and takes new entries after them.
#[test]
fn a_root_map_is_written_back_as_read_and_takes_new_entries() {
    use oxiroot_io_core::WriteRoot;

    let f = fixture();
    let map = f.get::<ObjMap>("meta").unwrap();
    let bytes = FileWriter::create("unused.root")
        .add(&map)
        .to_bytes()
        .unwrap();
    let back = FileReader::from_bytes(bytes).unwrap();
    assert_eq!(
        back.key("meta").unwrap().key_len,
        f.key("meta").unwrap().key_len
    );
    assert_eq!(
        back.get::<ObjMap>("meta").unwrap().to_root_bytes(),
        map.to_root_bytes()
    );

    let more = map.insert("tag", &ObjString::new("v7"));
    let bytes = FileWriter::create("unused.root")
        .add(&more)
        .to_bytes()
        .unwrap();
    let back = FileReader::from_bytes(bytes)
        .unwrap()
        .get::<ObjMap>("meta")
        .unwrap();
    assert_eq!(back.string_keys(), ["version", "lumi", "hist", "tag"]);
    assert_eq!(back.get::<ObjString>("tag").unwrap().unwrap().value(), "v7");
    assert!(back.get::<Hist1D>("hist").unwrap().is_ok());
}

/// A map read from a file cannot be renamed, and cannot be a value of another
/// map: both are refused when the file is written.
#[test]
fn a_root_map_refuses_a_key_its_references_do_not_fit() {
    let map = fixture().get::<ObjMap>("meta").unwrap();

    let err = FileWriter::create("unused.root")
        .add(&map.clone().named("elsewhere"))
        .to_bytes()
        .unwrap_err();
    assert!(err.to_string().contains("cannot be renamed"), "{err}");

    let outer = ObjMap::new().named("outer").insert("inner", &map);
    let err = FileWriter::create("unused.root")
        .add(&outer)
        .to_bytes()
        .unwrap_err();
    assert!(err.to_string().contains("under its own key"), "{err}");

    // A key of another header length is refused too.
    use oxiroot_io_core::KeyCheck;
    assert!(KeyCheck::Any.check("TMap", "m", 40).is_ok());
    assert!(KeyCheck::HeaderLen(40).check("TMap", "m", 40).is_ok());
    let err = KeyCheck::HeaderLen(40).check("TMap", "m", 48).unwrap_err();
    assert!(err.to_string().contains("40-byte header"), "{err}");
}
