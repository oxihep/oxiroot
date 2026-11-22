//! Reading several branches of one tree.

use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;

fn open() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join("tree_flat.root"),
    )
    .expect("open")
}

#[test]
fn reads_several_branches_at_once() {
    let f = open();
    let t = TreeReader::open(&f, "Events").expect("open tree");
    let cols: Vec<Column> = ["i4", "b1"]
        .iter()
        .map(|&n| t.branch(n).values().expect("read"))
        .collect();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0], Column::I32(vec![0, 1, 2, 3, 4]));
    assert_eq!(cols[1], Column::Bool(vec![true, false, true, false, true]));
}
