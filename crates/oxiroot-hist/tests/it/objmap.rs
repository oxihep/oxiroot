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
        .insert("lumi", &Parameter::f64("lumi", 137.5))
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
