//! Entry-range reads (B5): `read_branch_range` fetches only the baskets covering
//! the window. Uses the two-basket fixture so basket selection is exercised.

use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::BranchValues;

fn open(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open")
}

#[test]
fn reads_entry_ranges() {
    let f = open("tree_multibasket.root");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 5);

    // Full range equals read_branch.
    let full = t.branch("i4").values().expect("full");
    assert_eq!(
        t.branch("i4").range(0..5).values().expect("range full"),
        full
    );
    assert_eq!(full, BranchValues::I32(vec![0, 1, 2, 3, 4]));

    // A sub-window (may span the basket boundary).
    assert_eq!(
        t.branch("i4").range(1..4).values().expect("mid"),
        BranchValues::I32(vec![1, 2, 3])
    );

    // stop clamps to the entry count; start clamps to stop.
    assert_eq!(
        t.branch("i4").range(3..100).values().expect("tail"),
        BranchValues::I32(vec![3, 4])
    );
    assert_eq!(
        t.branch("i4").range(2..2).values().expect("empty"),
        BranchValues::I32(vec![])
    );
    assert_eq!(
        t.branch("i4").range(10..20).values().expect("past end"),
        BranchValues::I32(vec![])
    );
}
