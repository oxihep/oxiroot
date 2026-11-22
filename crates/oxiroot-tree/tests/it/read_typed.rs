//! The `BranchRead` builder: typed reads, the type-mismatch error, ranged flat
//! reads, basket boundaries, and the read → write round trip.
use std::io::Cursor;

use oxiroot_io_core::Column;
use oxiroot_io_core::{Compression, Error, FileReader};
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::{Branch, TreeWriter};

use crate::support::write_tree_file_baskets;

fn file(tag: &str) -> FileReader {
    let path = std::env::temp_dir().join(format!(
        "oxiroot_read_typed_{tag}_{}.root",
        std::process::id()
    ));
    write_tree_file_baskets(
        &path,
        "T",
        &[
            Branch::new("pt", vec![1.0_f32, 2.0, 3.0, 4.0, 5.0]),
            Branch::new(
                "idx",
                vec![vec![1], vec![], vec![2, 3], vec![4], vec![5, 6, 7]],
            ),
            Branch::new(
                "w",
                vec![vec![0.5], vec![1.5, 2.5], vec![], vec![3.5], vec![4.5]],
            )
            .counted(),
            Branch::new(
                "tag",
                vec![
                    "a".to_string(),
                    "b".into(),
                    "c".into(),
                    "d".into(),
                    "e".into(),
                ],
            ),
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
    assert_eq!(
        t.branch("pt").read::<f32>().unwrap(),
        [1.0, 2.0, 3.0, 4.0, 5.0]
    );
    assert_eq!(
        t.branch("idx").range(2..4).read::<Vec<i32>>().unwrap(),
        [vec![2, 3], vec![4]]
    );
    assert_eq!(
        t.branch("tag").range(3..10).read::<String>().unwrap(),
        ["d", "e"]
    );
}

#[test]
fn a_wrong_type_is_an_error_not_a_conversion() {
    let f = file("mismatch");
    let t = TreeReader::open(&f, "T").unwrap();
    let err = t.branch("pt").read::<f64>().unwrap_err();
    assert_eq!(
        format!("{:?}", err),
        format!(
            "{:?}",
            Error::TypeMismatch {
                name: "pt".into(),
                found: "f32".into(),
                expected: "f64".into(),
            }
        )
    );
    assert!(err.to_string().contains("holds f32, not f64"), "{err}");
    assert!(t.branch("idx").read::<i32>().is_err());
}

#[test]
fn reads_a_flat_window() {
    let f = file("flat");
    let t = TreeReader::open(&f, "T").unwrap();
    let Column::List { offsets, .. } = t.branch("w").values().unwrap() else {
        panic!("a counted array reads as a list");
    };
    assert_eq!(offsets, [0, 1, 3, 3, 4, 5]);
    // A window's offsets start again at 0, over its own values.
    assert_eq!(
        t.branch("w").range(1..4).values().unwrap(),
        Column::List {
            offsets: vec![0, 2, 2, 3],
            values: Box::new(Column::F64(vec![1.5, 2.5, 3.5])),
        }
    );
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
    let mut w =
        TreeWriter::new(Cursor::new(Vec::new()), "o.root", "T", Compression::Stored).unwrap();
    w.write_batch(&branches).unwrap();
    assert_eq!(w.entries(), 2);
    let out = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let o = TreeReader::open(&out, "T").unwrap();
    assert_eq!(o.branch("pt").read::<f32>().unwrap(), [2.0, 3.0]);
    assert_eq!(
        o.branch("idx").read::<Vec<i32>>().unwrap(),
        [vec![], vec![2, 3]]
    );
    assert_eq!(
        o.branch("w").read::<Vec<f64>>().unwrap(),
        [vec![1.5, 2.5], vec![]]
    );
    assert_eq!(
        o.branch_title("w"),
        t.branch_title("w"),
        "still a jagged x[n] leaf"
    );
    assert_eq!(o.branch("tag").read::<String>().unwrap(), ["b", "c"]);
}

#[test]
fn reports_branch_layouts() {
    use oxiroot_tree::BranchLayout;
    let f = file("layout");
    let t = TreeReader::open(&f, "T").unwrap();
    assert_eq!(t.branch_layout("pt"), Some(BranchLayout::Scalar));
    assert_eq!(t.branch_layout("idx"), Some(BranchLayout::List));
    assert_eq!(t.branch_layout("w"), Some(BranchLayout::CArray));
    assert_eq!(
        t.branch_layout("nw"),
        Some(BranchLayout::Scalar),
        "the count branch"
    );
    assert_eq!(t.branch_layout("tag"), Some(BranchLayout::Scalar));
    assert_eq!(t.branch_layout("nope"), None);
}

#[test]
fn write_batches_equals_one_batch_at_a_time() {
    let batches: Vec<Vec<Branch>> = (0..3)
        .map(|k| {
            vec![
                Branch::new("x", vec![f64::from(k); 4]),
                Branch::new("j", vec![vec![k], vec![], vec![k, k], vec![1]]).counted(),
                Branch::new(
                    "s",
                    vec!["a".to_string(), "bb".into(), String::new(), "c".into()],
                ),
            ]
        })
        .collect();
    let mut a =
        TreeWriter::new(Cursor::new(Vec::new()), "t.root", "T", Compression::Zstd(1)).unwrap();
    a.write_batches(&batches).unwrap();
    let mut b =
        TreeWriter::new(Cursor::new(Vec::new()), "t.root", "T", Compression::Zstd(1)).unwrap();
    for batch in &batches {
        b.write_batch(batch).unwrap();
    }
    assert_eq!(
        a.finish().unwrap().into_inner(),
        b.finish().unwrap().into_inner()
    );
}

#[test]
fn writes_vector_bool_string_and_vector_vector() {
    let vb = vec![vec![true, false], vec![], vec![true]];
    let vs = vec![
        vec!["a".to_string(), "bc".into()],
        vec![],
        vec!["x".repeat(300)],
    ];
    let vv = vec![
        vec![vec![1.5_f32], vec![]],
        vec![],
        vec![vec![2.0, 3.0], vec![4.0]],
    ];
    let vvi = vec![vec![vec![1_i64, 2]], vec![vec![]], vec![]];
    let chunk = |r: std::ops::Range<usize>| {
        vec![
            Branch::new("vb", vb[r.clone()].to_vec()),
            Branch::new("vs", vs[r.clone()].to_vec()),
            Branch::new("vv", vv[r.clone()].to_vec()),
            Branch::new("vvi", vvi[r].to_vec()),
        ]
    };
    // Two baskets, so each kind is also cut and stitched back.
    let mut w =
        TreeWriter::new(Cursor::new(Vec::new()), "o.root", "T", Compression::Zstd(1)).unwrap();
    w.write_batches(&[chunk(0..2), chunk(2..3)]).unwrap();
    let file = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let t = TreeReader::open(&file, "T").unwrap();
    assert_eq!(t.branch("vb").read::<Vec<bool>>().unwrap(), vb);
    assert_eq!(t.branch("vs").read::<Vec<String>>().unwrap(), vs);
    assert_eq!(t.branch("vv").read::<Vec<Vec<f32>>>().unwrap(), vv);
    assert_eq!(t.branch("vvi").read::<Vec<Vec<i64>>>().unwrap(), vvi);
    assert_eq!(
        t.branch_layout("vv"),
        Some(oxiroot_tree::BranchLayout::ListOfLists)
    );
}

#[test]
fn an_empty_first_batch_writes_a_tree_with_no_entries() {
    let mut w =
        TreeWriter::new(Cursor::new(Vec::new()), "o.root", "T", Compression::Stored).unwrap();
    w.write_batch(&[
        Branch::new("x", Vec::<f64>::new()),
        Branch::new("v", Vec::<Vec<i32>>::new()),
    ])
    .unwrap();
    let file = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let t = TreeReader::open(&file, "T").unwrap();
    assert_eq!(t.entries(), 0);
    assert_eq!(t.branch_names().collect::<Vec<_>>(), ["x", "v"]);
    assert_eq!(t.branch("x").read::<f64>().unwrap(), Vec::<f64>::new());
}
