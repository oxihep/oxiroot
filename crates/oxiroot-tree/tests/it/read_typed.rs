//! The `BranchRead` builder: typed reads, the type-mismatch error, ranged flat
//! reads, basket boundaries, and the read → write round trip.
use std::io::Cursor;

use oxiroot_io_core::{Compression, Error, FileReader};
use oxiroot_tree::{Branch, BranchValues, TreeReader, TreeWriter};

use crate::support::write_tree_file_baskets;

fn file(tag: &str) -> FileReader {
    let path = std::env::temp_dir().join(format!("oxiroot_read_typed_{tag}_{}.root", std::process::id()));
    write_tree_file_baskets(
        &path,
        "T",
        &[
            Branch::f32("pt", vec![1.0, 2.0, 3.0, 4.0, 5.0]),
            Branch::vector_i32("idx", vec![vec![1], vec![], vec![2, 3], vec![4], vec![5, 6, 7]]),
            Branch::jagged_f64("w", vec![vec![0.5], vec![1.5, 2.5], vec![], vec![3.5], vec![4.5]]),
            Branch::string("tag", vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()]),
        ],
        Compression::Zstd(1),
        2,
    )
    .unwrap();
    let f = FileReader::open(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    f
}

#[test]
fn reads_typed_values() {
    let f = file("typed");
    let t = TreeReader::open(&f, "T").unwrap();
    assert_eq!(t.branch("pt").read::<f32>().unwrap(), [1.0, 2.0, 3.0, 4.0, 5.0]);
    assert_eq!(
        t.branch("idx").range(2..4).read::<Vec<i32>>().unwrap(),
        [vec![2, 3], vec![4]]
    );
    assert_eq!(t.branch("tag").range(3..10).read::<String>().unwrap(), ["d", "e"]);
}

#[test]
fn a_wrong_type_is_an_error_not_a_conversion() {
    let f = file("mismatch");
    let t = TreeReader::open(&f, "T").unwrap();
    let err = t.branch("pt").read::<f64>().unwrap_err();
    assert_eq!(
        err,
        Error::TypeMismatch {
            name: "pt".into(),
            found: "f32".into(),
            expected: "f64".into(),
        }
    );
    assert!(err.to_string().contains("holds f32, not f64"), "{err}");
    assert!(t.branch("idx").read::<i32>().is_err());
}

#[test]
fn reads_a_flat_window() {
    let f = file("flat");
    let t = TreeReader::open(&f, "T").unwrap();
    let all = t.branch("w").flat().unwrap();
    assert_eq!(all.offsets, [0, 1, 3, 3, 4, 5]);
    let window = t.branch("w").range(1..4).flat().unwrap();
    assert_eq!(window.offsets, [0, 2, 2, 3]);
    assert_eq!(window.values, BranchValues::F64(vec![1.5, 2.5, 3.5]));
}

#[test]
fn reports_basket_boundaries() {
    let f = file("bounds");
    let t = TreeReader::open(&f, "T").unwrap();
    assert_eq!(t.basket_boundaries("pt").unwrap(), [0, 2, 4, 5]);
    assert!(t.basket_boundaries("nope").is_err());
}

#[test]
fn a_read_branch_writes_back_the_same_way() {
    let f = file("roundtrip");
    let t = TreeReader::open(&f, "T").unwrap();
    let branches: Vec<Branch> = ["pt", "idx", "w", "tag"]
        .iter()
        .map(|n| t.branch(n).range(1..3).to_branch().unwrap())
        .collect();
    let mut w = TreeWriter::new(Cursor::new(Vec::new()), "o.root", "T", Compression::None).unwrap();
    w.write_batch(&branches).unwrap();
    assert_eq!(w.entries(), 2);
    let out = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let o = TreeReader::open(&out, "T").unwrap();
    assert_eq!(o.branch("pt").read::<f32>().unwrap(), [2.0, 3.0]);
    assert_eq!(o.branch("idx").read::<Vec<i32>>().unwrap(), [vec![], vec![2, 3]]);
    assert_eq!(o.branch("w").read::<Vec<f64>>().unwrap(), [vec![1.5, 2.5], vec![]]);
    assert_eq!(o.branch_title("w"), t.branch_title("w"), "still a jagged x[n] leaf");
    assert_eq!(o.branch("tag").read::<String>().unwrap(), ["b", "c"]);
}
