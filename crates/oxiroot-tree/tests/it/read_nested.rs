//! A split `std::vector<Outer>` where `Outer { int id; Inner inner; float w }`
//! nests another struct `Inner { float a; int b }`. ROOT flattens the nested
//! member into per-member jagged sub-branches (`v.id`, `v.inner.a`, `v.inner.b`,
//! `v.w`), which oxiroot reads. Cross-checked against ROOT C++ and uproot.

use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::BranchValues;

fn fixture(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open fixture")
}

#[test]
fn reads_nested_struct_subbranches() {
    let f = fixture("tree_nested.root");
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_eq!(t.entries(), 3);
    // The nested `Inner` member is flattened to `v.inner.a` / `v.inner.b`.
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        vec!["v.id", "v.inner.a", "v.inner.b", "v.w"]
    );
    assert!(t.unsupported_branches().collect::<Vec<_>>().is_empty());

    assert_eq!(
        t.branch("v.id").values().unwrap(),
        BranchValues::VecI32(vec![vec![0], vec![0, 1], vec![0, 1, 2]])
    );
    assert_eq!(
        t.branch("v.inner.a").values().unwrap(),
        BranchValues::VecF32(vec![vec![0.5], vec![0.5, 1.5], vec![0.5, 1.5, 2.5]])
    );
    assert_eq!(
        t.branch("v.inner.b").values().unwrap(),
        BranchValues::VecI32(vec![vec![0], vec![0, 2], vec![0, 2, 4]])
    );
    assert_eq!(
        t.branch("v.w").values().unwrap(),
        BranchValues::VecF32(vec![vec![0.0], vec![0.0, 1.5], vec![0.0, 1.5, 3.0]])
    );
}
