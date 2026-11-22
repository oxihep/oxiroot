//! The `rayon` feature adds `read_branch_par` and friends. They must return
//! exactly what the serial methods return, and enabling the feature must not
//! change the serial methods (they used to switch to parallel decoding
//! silently).
#![cfg(feature = "rayon")]

use crate::support::write_tree_file_baskets;
use oxiroot_io_core::cast::SaturatingFrom;
use std::path::PathBuf;

use oxiroot_io_core::{Compression, FileReader};
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::Branch;

fn fixture(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open fixture")
}

fn assert_par_matches_serial(t: &TreeReader<'_>) {
    for name in t.branch_names() {
        let serial = t.branch(name).values();
        assert_eq!(
            format!("{:?}", t.branch(name).parallel(true).values()),
            format!("{:?}", serial),
            "branch {name}"
        );
        let n = t.entries();
        assert_eq!(
            format!(
                "{:?}",
                t.branch(name)
                    .range(1..n.saturating_sub(1))
                    .parallel(true)
                    .values()
            ),
            format!(
                "{:?}",
                t.branch(name).range(1..n.saturating_sub(1)).values()
            ),
            "range of {name}"
        );
    }
}

#[test]
fn parallel_reads_match_serial_on_a_root_written_tree() {
    let f = fixture("tree_multibasket.root");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_par_matches_serial(&t);
}

#[test]
fn parallel_reads_match_serial_across_many_baskets() {
    let n: i32 = 1000;
    let x: Vec<f64> = (0..n).map(|i| f64::from(i) * 0.5).collect();
    let k: Vec<i32> = (0..n).map(|i| i % 17).collect();
    let jag: Vec<Vec<f64>> = (0..n)
        .map(|i| vec![f64::from(i); usize::saturating_from(i % 4)])
        .collect();
    let out = std::env::temp_dir().join(format!("oxiroot_read_par_{}.root", std::process::id()));
    write_tree_file_baskets(
        &out,
        "T",
        &[
            Branch::new("x", x),
            Branch::new("k", k),
            Branch::new("jag", jag).counted(),
        ],
        Compression::Zstd(1),
        37, // many baskets per branch
    )
    .expect("write");
    let f = FileReader::open(&out).expect("open");
    let _ = std::fs::remove_file(&out);
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_par_matches_serial(&t);
}
