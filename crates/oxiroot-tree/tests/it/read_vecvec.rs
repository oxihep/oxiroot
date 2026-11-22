//! Unsplit `std::vector<std::vector<T>>` branches: a doubly-nested collection
//! per entry. oxiroot reads them as [`Column::List`] — a flat list of
//! inner vectors partitioned per entry by cumulative `offsets`. Cross-checked
//! against ROOT C++ and uproot.

use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;

fn fixture(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open fixture")
}

#[test]
fn reads_vector_of_vector_int() {
    let f = fixture("tree_vecvec.root");
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(t.branch_names().collect::<Vec<_>>(), vec!["vi", "vd"]);
    assert!(t.unsupported_branches().collect::<Vec<_>>().is_empty());

    // vi = [[[0]], [[100], [110, 111]], [[200], [210, 211], [220, 221, 222]]]
    let vi = t.branch("vi").values().unwrap();
    assert_eq!(vi.len(), 3);
    assert_eq!(
        vi,
        Column::List {
            offsets: vec![0, 1, 3, 6],
            values: Box::new(Column::from(vec![
                vec![0],
                vec![100],
                vec![110, 111],
                vec![200],
                vec![210, 211],
                vec![220, 221, 222],
            ])),
        }
    );
}

#[test]
fn reads_vector_of_vector_double() {
    let f = fixture("tree_vecvec.root");
    let t = TreeReader::open(&f, "T").expect("open tree");
    // vd = [[[0.0]], [[1.0], [1.1, 1.11]], [[2.0], [2.1, 2.11], [2.2, 2.21, 2.22]]]
    assert_eq!(
        t.branch("vd").values().unwrap(),
        Column::List {
            offsets: vec![0, 1, 3, 6],
            values: Box::new(Column::from(vec![
                vec![0.0],
                vec![1.0],
                vec![1.1, 1.11],
                vec![2.0],
                vec![2.1, 2.11],
                vec![2.2, 2.21, 2.22],
            ])),
        }
    );
}

/// The outer offsets cut the inner vectors into entries, and the inner
/// offsets cut the values into vectors: this reconstructs entry 1's
/// `[[100], [110, 111]]`.
#[test]
fn nested_offsets_partition_entries() {
    let f = fixture("tree_vecvec.root");
    let t = TreeReader::open(&f, "T").expect("open tree");
    let rows: Vec<Vec<Vec<i32>>> = t.branch("vi").read().expect("vi as rows");
    assert_eq!(rows[1], [vec![100], vec![110, 111]]);
}
