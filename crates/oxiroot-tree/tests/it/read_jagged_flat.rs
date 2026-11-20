//! The offsets+flat jagged view (B9): read_branch_flat avoids the Vec<Vec>
//! allocation and stays consistent with read_branch.

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_tree::{BranchValues, TreeReader};

fn open() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join("tree_arrays.root"),
    )
    .expect("open")
}

#[test]
fn flat_view_matches_nested() {
    let f = open();
    let t = TreeReader::open(&f, "T").expect("open tree");

    // Jagged branch y = [[1,2], [], [3,4,5]].
    let j = t.branch("y").flat().expect("flat y");
    assert_eq!(j.offsets, [0, 2, 2, 5]);
    assert_eq!(j.values, BranchValues::F64(vec![1.0, 2.0, 3.0, 4.0, 5.0]));
    assert_eq!(j.len(), 3);

    // Fixed array x[3]: offsets step by 3.
    let x = t.branch("x").flat().expect("flat x");
    assert_eq!(x.offsets, [0, 3, 6, 9]);
    assert_eq!(
        x.values,
        BranchValues::F64(vec![0.0, 1.0, 2.0, 10.0, 11.0, 12.0, 20.0, 21.0, 22.0])
    );

    // Scalar branch ny: one element per entry.
    let ny = t.branch("ny").flat().expect("flat ny");
    assert_eq!(ny.offsets, [0, 1, 2, 3]);
    assert_eq!(ny.values, BranchValues::I32(vec![2, 0, 3]));

    // Reshaping the flat y by its offsets reproduces the nested read.
    let flat = match j.values {
        BranchValues::F64(v) => v,
        _ => unreachable!(),
    };
    let reshaped: Vec<Vec<f64>> = j
        .offsets
        .windows(2)
        .map(|w| flat[usize::saturating_from(w[0])..usize::saturating_from(w[1])].to_vec())
        .collect();
    assert_eq!(
        BranchValues::VecF64(reshaped),
        t.branch("y").values().expect("nested y")
    );

    // String branches aren't supported by the flat view.
    assert!(t.branch("s").flat().is_err());
}
