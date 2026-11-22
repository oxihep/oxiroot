//! The offsets+flat jagged view (B9): `read_branch_flat` avoids the Vec<Vec>
//! allocation and stays consistent with `read_branch`.

use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;

fn open() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join("tree_arrays.root"),
    )
    .expect("open")
}

#[test]
fn columns_are_flat_whatever_the_branch_shape() {
    let f = open();
    let t = TreeReader::open(&f, "T").expect("open tree");

    // Counted array y = [[1,2], [], [3,4,5]]: offsets over one buffer.
    let y = t.branch("y").values().expect("y");
    assert_eq!(
        y,
        Column::List {
            offsets: vec![0, 2, 2, 5],
            values: Box::new(Column::F64(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
        }
    );
    assert_eq!(y.len(), 3);

    // Fixed array x[3]: the width, and one buffer.
    assert_eq!(
        t.branch("x").values().expect("x"),
        Column::Array {
            len: 3,
            values: Box::new(Column::F64(vec![
                0.0, 1.0, 2.0, 10.0, 11.0, 12.0, 20.0, 21.0, 22.0
            ])),
        }
    );

    // Scalar branch ny: one value per entry.
    assert_eq!(
        t.branch("ny").values().expect("ny"),
        Column::I32(vec![2, 0, 3])
    );

    // The typed read of y gives the same rows.
    let rows: Vec<Vec<f64>> = t.branch("y").read().expect("rows of y");
    assert_eq!(Column::from(rows), y);

    // A string branch is a string column: offsets over one text.
    let Column::String { offsets, .. } = t.branch("s").values().expect("s") else {
        panic!("a string branch reads as a string column");
    };
    assert_eq!(offsets.len(), 4);
}
