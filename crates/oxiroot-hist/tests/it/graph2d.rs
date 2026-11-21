//! `Graph2D`: read a ROOT fixture, self round-trip, and build from scratch.
//! (Cross-checked against compiled ROOT C++ and uproot, which read the
//! oxiroot-written `Graph2D` with the right class and values.)

use std::path::PathBuf;

use oxiroot_hist::Graph2D;
use oxiroot_io_core::Compression;
use oxiroot_io_core::FileReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn reads_root_written_graph2d() {
    let f = FileReader::open(fixture("graph2d.root")).expect("open");
    let g = f.get::<Graph2D>("g2d").expect("read g2d");
    assert_eq!(g.name(), "g2d");
    assert_eq!(g.title(), "surface fixture");
    assert_eq!(g.x(), vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(g.y(), vec![10.0, 20.0, 30.0, 40.0]);
    assert_eq!(g.z(), vec![100.0, 200.0, 300.0, 400.0]);
}

#[test]
fn graph2d_round_trip_from_fixture() {
    let f = FileReader::open(fixture("graph2d.root")).expect("open");
    let g = f.get::<Graph2D>("g2d").unwrap();
    let out = std::env::temp_dir().join("oxiroot_g2d_rt.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&g)
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let back = FileReader::open(&out)
        .unwrap()
        .get::<Graph2D>("g2d")
        .unwrap();
    assert_eq!(back, g);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn graph2d_build_from_scratch() {
    let g = Graph2D::new(
        vec![0.5, 1.5, 2.5],
        vec![-1.0, 0.0, 1.0],
        vec![3.0, 6.0, 9.0],
    )
    .unwrap()
    .named("scratch")
    .titled("built in Rust");
    let out = std::env::temp_dir().join("oxiroot_g2d_scratch.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&g)
        .compression(Compression::Zstd(3))
        .write()
        .expect("write");
    let back = FileReader::open(&out)
        .unwrap()
        .get::<Graph2D>("scratch")
        .unwrap();
    assert_eq!(back, g);
    assert_eq!(back.len(), 3);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn empty_graph2d_round_trip() {
    let g = Graph2D::new(vec![], vec![], vec![]).unwrap().named("empty");
    assert!(g.is_empty());
    let out = std::env::temp_dir().join("oxiroot_g2d_empty.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&g)
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let back = FileReader::open(&out)
        .unwrap()
        .get::<Graph2D>("empty")
        .unwrap();
    assert_eq!(back, g);
    let _ = std::fs::remove_file(&out);
}
