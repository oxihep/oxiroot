//! ROOT delta-encodes a split index column one page at a time: every page's
//! first offset is stored whole. A fixture whose collection and string offsets
//! span about twenty pages per cluster (`scripts/gen_rntuple_pages.cpp`),
//! read in full, in windows, and in parallel.

use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_rntuple::NtupleReader;

fn open() -> FileReader {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rntuple_multipage.root");
    FileReader::open(path).expect("open fixture")
}

fn jets(i: usize) -> Vec<f64> {
    (0..i % 5).map(|j| (i * 5 + j) as f64 * 0.5).collect()
}

#[test]
fn offsets_restart_at_every_page() {
    let file = open();
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();
    assert_eq!(ntpl.entries(), 5000);
    assert_eq!(ntpl.cluster_boundaries(), [0, 2500, 5000]);

    let all: Vec<Vec<f64>> = ntpl.field("jets").read().unwrap();
    assert_eq!(all.len(), 5000);
    for (i, row) in all.iter().enumerate() {
        assert_eq!(*row, jets(i), "entry {i}");
    }
    let labels: Vec<String> = ntpl.field("label").read().unwrap();
    for (i, s) in labels.iter().enumerate() {
        assert_eq!(*s, format!("e{i}"), "entry {i}");
    }

    let window: Vec<Vec<f64>> = ntpl.field("jets").range(1234..3777).read().unwrap();
    assert_eq!(window, (1234..3777).map(jets).collect::<Vec<_>>());
}

#[cfg(feature = "rayon")]
#[test]
fn parallel_read_matches() {
    let file = open();
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();
    let serial: Vec<Vec<f64>> = ntpl.field("jets").read().unwrap();
    let parallel: Vec<Vec<f64>> = ntpl.field("jets").parallel(true).read().unwrap();
    assert_eq!(serial, parallel);
}
