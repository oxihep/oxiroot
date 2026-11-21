//! Write a flat tree, read it back through our own reader.
use crate::support::{TreeObject, write_tree_file};
use oxiroot_io_core::Column;
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::Branch;

#[test]
fn write_then_read_roundtrips() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_written_tree.root");
    let branches = vec![
        Branch::new("i4", vec![0, 1, 2, 3, 4]),
        Branch::new("f8", vec![0.25, 1.25, 2.25, 3.25, 4.25]),
        Branch::new("b1", vec![true, false, true, false, true]),
        Branch::new("u4", vec![100_u32, 200, 300, 400, 4_000_000_000]),
    ];
    write_tree_file(&out, "Events", &branches, Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 5);
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        ["i4", "f8", "b1", "u4"]
    );
    assert_eq!(
        t.branch("i4").values().unwrap(),
        Column::I32(vec![0, 1, 2, 3, 4])
    );
    assert_eq!(
        t.branch("f8").values().unwrap(),
        Column::F64(vec![0.25, 1.25, 2.25, 3.25, 4.25])
    );
    assert_eq!(
        t.branch("b1").values().unwrap(),
        Column::Bool(vec![true, false, true, false, true])
    );
    assert_eq!(
        t.branch("u4").values().unwrap(),
        Column::U32(vec![100, 200, 300, 400, 4_000_000_000])
    );
}

#[test]
fn write_then_read_arrays_and_strings() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_written_arrays.root");
    let xs = vec![
        vec![1.0, 2.0, 3.0],
        vec![4.0, 5.0, 6.0],
        vec![7.0, 8.0, 9.0],
    ];
    let ss = vec!["alpha".to_string(), String::new(), "gamma!".to_string()];
    let ns = vec![
        vec![10i32, 11, 12, 13],
        vec![20, 21, 22, 23],
        vec![30, 31, 32, 33],
    ];
    let branches = vec![
        Branch::new("x", arrays(&xs)),
        Branch::new("s", ss.clone()),
        Branch::new("n", arrays(&ns)),
    ];
    write_tree_file(&out, "Events", &branches, Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(t.branch_names().collect::<Vec<_>>(), ["x", "s", "n"]);
    assert_eq!(t.branch("x").values().unwrap(), arrays(&xs));
    assert_eq!(t.branch("s").values().unwrap(), Column::from(ss));
    assert_eq!(t.branch("n").values().unwrap(), arrays(&ns));
}

#[test]
fn write_then_read_jagged() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_written_jagged.root");
    let ys = vec![vec![1.0, 2.0, 3.0], vec![], vec![4.0], vec![5.0, 6.0]];
    let ns = vec![vec![10i32, 11], vec![20, 21, 22], vec![], vec![30]];
    let branches = vec![
        Branch::new("e", vec![0.5, 1.5, 2.5, 3.5]),
        Branch::new("y", ys.clone()).counted(),
        Branch::new("n", ns.clone()).counted(),
    ];
    write_tree_file(&out, "Events", &branches, Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 4);
    // The writer inserts the count branch (ny / nn) before each jagged branch.
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        ["e", "ny", "y", "nn", "n"]
    );
    assert_eq!(t.branch("y").values().unwrap(), Column::from(ys));
    assert_eq!(t.branch("n").values().unwrap(), Column::from(ns));
    // The auto count branches hold the per-row lengths.
    assert_eq!(
        t.branch("ny").values().unwrap(),
        Column::I32(vec![3, 0, 1, 2])
    );
    assert_eq!(
        t.branch("nn").values().unwrap(),
        Column::I32(vec![2, 3, 0, 1])
    );
}

#[test]
fn write_then_read_zstd() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_written_zstd.root");
    let xs = vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]];
    let ys = vec![vec![7.0], vec![8.0, 9.0], vec![]];
    let branches = vec![
        Branch::new("a", vec![1, 2, 3]),
        Branch::new("x", arrays(&xs)),
        Branch::new("s", vec!["p".to_string(), "qq".into(), "rrr".into()]),
        Branch::new("y", ys.clone()).counted(),
    ];
    write_tree_file(&out, "Events", &branches, Compression::Zstd(5)).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(t.branch("a").values().unwrap(), Column::I32(vec![1, 2, 3]));
    assert_eq!(t.branch("x").values().unwrap(), arrays(&xs));
    assert_eq!(
        t.branch("s").values().unwrap(),
        Column::from(vec!["p".to_string(), "qq".into(), "rrr".into()])
    );
    assert_eq!(t.branch("y").values().unwrap(), Column::from(ys));
}

#[test]
fn write_then_read_std_vector() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_written_vector.root");
    let vf = vec![vec![1.0f32, 2.0, 3.0], vec![], vec![4.0], vec![5.0, 6.0]];
    let vi = vec![vec![10i32, 20], vec![30], vec![], vec![40, 50, 60]];
    let branches = vec![
        Branch::new("n", vec![0, 1, 2, 3]),
        Branch::new("vf", vf.clone()),
        Branch::new("vi", vi.clone()),
    ];
    write_tree_file(&out, "T", &branches, Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_eq!(t.entries(), 4);
    assert_eq!(t.branch_names().collect::<Vec<_>>(), ["n", "vf", "vi"]);
    assert_eq!(
        t.branch("n").values().unwrap(),
        Column::I32(vec![0, 1, 2, 3])
    );
    assert_eq!(t.branch("vf").values().unwrap(), Column::from(vf));
    assert_eq!(t.branch("vi").values().unwrap(), Column::from(vi));
}

#[test]
fn ragged_fixed_arrays_are_rejected() {
    use crate::support::tree_file_bytes;
    // An array column whose values do not make whole arrays is an error.
    let ragged = Column::Array {
        len: 2,
        values: Box::new(Column::I32(vec![1, 2, 3])),
    };
    let branches = vec![Branch::new("j", ragged)];
    let err = tree_file_bytes("f.root", "T", &branches, Compression::Stored).unwrap_err();
    assert!(format!("{err}").contains("whole arrays"), "got: {err}");
}

/// A tree with zero entries must still write a valid file and read back empty —
/// the empty-tree boundary (empty baskets).
#[test]
fn write_then_read_empty_tree() {
    let out = std::path::PathBuf::from("/tmp/oxiroot_empty_tree.root");
    let branches = vec![
        Branch::new("i4", Vec::<i32>::new()),
        Branch::new("f8", Vec::<f64>::new()),
    ];
    write_tree_file(&out, "Events", &branches, Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "Events").expect("open tree");
    assert_eq!(t.entries(), 0);
    assert_eq!(t.branch_names().collect::<Vec<_>>(), ["i4", "f8"]);
    assert_eq!(t.branch("i4").values().unwrap(), Column::I32(vec![]));
    assert_eq!(t.branch("f8").values().unwrap(), Column::F64(vec![]));
}

#[test]
fn branches_with_different_entry_counts_are_rejected() {
    use oxiroot_io_core::{Error, FileWriter};
    let branches = || {
        vec![
            Branch::new("x", vec![1, 2, 3]),
            Branch::new("y", vec![1.0, 2.0]),
        ]
    };
    let out = std::env::temp_dir().join("oxiroot_tree_uneven_branches.root");
    let err = write_tree_file(&out, "T", &branches(), Compression::Stored).unwrap_err();
    assert_eq!(
        format!("{:?}", err),
        format!(
            "{:?}",
            Error::LengthMismatch {
                what: "branch \"y\" entries".into(),
                expected: 3,
                found: 2
            }
        )
    );
    // The builder path (`FileWriter::put`) goes through the same check.
    let err = FileWriter::create(&out)
        .add(TreeObject::new("T", branches()))
        .compression(Compression::Stored)
        .to_bytes()
        .unwrap_err();
    assert!(matches!(err, Error::LengthMismatch { .. }), "{err:?}");
}

/// Rows of one length as a fixed-array column.
fn arrays<T: oxiroot_io_core::column::Primitive>(rows: &[Vec<T>]) -> Column {
    Column::Array {
        len: rows.first().map_or(0, Vec::len),
        values: Box::new(T::into_column(rows.concat())),
    }
}
