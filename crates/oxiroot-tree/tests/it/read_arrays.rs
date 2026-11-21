//! Read fixed-size arrays, variable/jagged arrays, and string branches.

use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn reads_arrays_and_strings() {
    let f = FileReader::open(fixture("tree_arrays.root")).expect("open");
    let t = TreeReader::open(&f, "T").expect("open tree");
    let read = |n| t.branch(n).values().expect("read branch");

    // Fixed-size array x[3].
    assert_eq!(
        read("x"),
        Column::from(vec![
            [0.0, 1.0, 2.0],
            [10.0, 11.0, 12.0],
            [20.0, 21.0, 22.0]
        ])
    );
    // The auto-generated count branch is a plain scalar.
    assert_eq!(read("ny"), Column::I32(vec![2, 0, 3]));
    // Variable-length (jagged) branch.
    assert_eq!(
        read("y"),
        Column::from(vec![vec![1.0, 2.0], vec![], vec![3.0, 4.0, 5.0]])
    );
    // String branch.
    assert_eq!(
        read("s"),
        Column::from(vec!["a".to_string(), "bb".into(), "ccc".into()])
    );
}
