//! A bare collection of objects under one key: `ObjList`, as a `TList` or
//! `TObjArray`. oxiroot reads the ROOT-C++-written `objlist.root` fixture — whose
//! members use the object protocol (repeats via class back-references) — and
//! round-trips its own writes; ROOT C++ and uproot read oxiroot's output
//! (checked out of band).

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Hist, Hist1D};
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_io_core::{FileWriter, ListKind, ObjList, ObjString, Parameter};

fn fixture() -> FileReader {
    FileReader::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/objlist.root"))
        .expect("open fixture")
}

#[test]
fn reads_root_written_list_and_array() {
    let f = fixture();

    let l = f.get::<ObjList>("mylist").unwrap();
    assert_eq!(l.name(), "mylist");
    assert_eq!(l.kind(), ListKind::List);
    assert_eq!(l.len(), 3);
    assert_eq!(
        l.class_names().collect::<Vec<_>>(),
        ["TH1F", "TObjString", "TParameter<double>"]
    );
    // Pull members out by type.
    assert_eq!(l.items::<Hist1D>().unwrap().len(), 1);
    assert_eq!(l.items::<ObjString>().unwrap()[0].value(), "hello");
    assert_eq!(l.items::<Parameter>().unwrap()[0].value().as_f64(), 12.5);

    let a = f.get::<ObjList>("myarr").unwrap();
    assert_eq!(a.kind(), ListKind::Array);
    assert_eq!(a.len(), 2);
    assert_eq!(a.items::<Hist1D>().unwrap().len(), 2);
}

#[test]
fn round_trips_list_and_array_through_oxiroot() {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("h");
    h.fill(0.5);
    let h = h.to_root().unwrap();
    let list = ObjList::new(ListKind::List)
        .named("mylist")
        .add(&h)
        .add(&ObjString::new("hello"))
        .add(&Parameter::f64(12.5).named("lumi"));
    let arr = ObjList::new(ListKind::Array)
        .named("myarr")
        .add(
            &Hist::reg(3, 0.0, 3.0)
                .double()
                .to_root()
                .unwrap()
                .named("a0"),
        )
        .add(
            &Hist::reg(3, 0.0, 3.0)
                .double()
                .to_root()
                .unwrap()
                .named("a1"),
        );

    let out = std::env::temp_dir().join("oxiroot_objlist_rt.root");
    FileWriter::create(&out)
        .add(&list)
        .add(&arr)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&out).unwrap();
    let l = f.get::<ObjList>("mylist").unwrap();
    assert_eq!(l.len(), 3);
    assert_eq!(l.items::<Hist1D>().unwrap().len(), 1);
    assert_eq!(l.items::<ObjString>().unwrap()[0].value(), "hello");
    assert_eq!(l.items::<Parameter>().unwrap()[0].value().as_f64(), 12.5);
    let a = f.get::<ObjList>("myarr").unwrap();
    assert_eq!(a.kind(), ListKind::Array);
    assert_eq!(a.items::<Hist1D>().unwrap().len(), 2);
    let _ = std::fs::remove_file(&out);
}
