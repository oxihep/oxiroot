//! Read a multidimensional fixed-array branch float m[2][3]. The data is stored
//! row-major flat (6 per entry); the [2,3] shape is exposed via `branch_shape`.
use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;
use std::path::PathBuf;

#[test]
fn reads_multidim_array() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join("tree_multidim.root");
    let f = FileReader::open(path).expect("open");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(t.branch_shape("m"), Some([2usize, 3].as_slice()));
    assert_eq!(
        t.branch("m").values().expect("read m"),
        Column::from(vec![
            [0.0_f32, 1.0, 2.0, 3.0, 4.0, 5.0],
            [10.0, 11.0, 12.0, 13.0, 14.0, 15.0],
            [20.0, 21.0, 22.0, 23.0, 24.0, 25.0],
        ])
    );
}
