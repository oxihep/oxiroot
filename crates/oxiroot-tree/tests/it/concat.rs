//! Concatenating trees with [`append_trees`] — the `hadd` building block.
use std::io::Cursor;

use crate::support::write_tree_file;
use oxiroot_io_core::{Compression, FileReader, Result};
use oxiroot_tree::{Branch, BranchValues, TreeReader, TreeWriter, append_trees};

/// Write `branches` as tree `Events` to a fresh temp file and reopen it.
fn write_open(tag: &str, branches: Vec<Branch>) -> (FileReader, &'static str) {
    let path = std::env::temp_dir().join(format!("oxiroot_concat_{tag}.root"));
    write_tree_file(&path, "Events", &branches, Compression::None).expect("write");
    (FileReader::open(&path).expect("reopen"), "Events")
}

/// Merge `inputs` into one in-memory file holding tree `Events`.
fn concat_trees(inputs: &[&TreeReader<'_>]) -> Result<Vec<u8>> {
    let mut w = TreeWriter::new(
        Cursor::new(Vec::new()),
        "merged.root",
        "Events",
        Compression::None,
    )?;
    append_trees(&mut w, inputs)?;
    Ok(w.finish()?.into_inner())
}

/// Open the merged bytes.
fn write_merged(_tag: &str, merged: &[u8]) -> FileReader {
    FileReader::from_bytes(merged.to_vec()).expect("reopen merged")
}

#[test]
fn concatenates_flat_scalar_branches() {
    let (fa, _) = write_open(
        "flat_a",
        vec![
            Branch::i32("i", vec![0, 1, 2]),
            Branch::f64("x", vec![0.0, 1.0, 2.0]),
            Branch::string("s", vec!["a".into(), "b".into(), "c".into()]),
        ],
    );
    let (fb, _) = write_open(
        "flat_b",
        vec![
            Branch::i32("i", vec![3, 4]),
            Branch::f64("x", vec![3.0, 4.0]),
            Branch::string("s", vec!["d".into(), "e".into()]),
        ],
    );
    let ta = TreeReader::open(&fa, "Events").unwrap();
    let tb = TreeReader::open(&fb, "Events").unwrap();

    let merged = concat_trees(&[&ta, &tb]).expect("concat");
    let fo = write_merged("flat_out", &merged);
    let to = TreeReader::open(&fo, "Events").unwrap();

    assert_eq!(to.entries(), 5);
    assert_eq!(
        to.branch("i").values().unwrap(),
        BranchValues::I32(vec![0, 1, 2, 3, 4])
    );
    assert_eq!(
        to.branch("x").values().unwrap(),
        BranchValues::F64(vec![0.0, 1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(
        to.branch("s").values().unwrap(),
        BranchValues::Str(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into()
        ])
    );
}

#[test]
fn preserves_and_concatenates_jagged_and_vector_branches() {
    let jag_a = vec![vec![1.0], vec![2.0, 3.0]];
    let jag_b = vec![vec![4.0, 5.0, 6.0]];
    let vec_a = vec![vec![1i32, 2], vec![3]];
    let vec_b = vec![vec![4, 5, 6]];

    let (fa, _) = write_open(
        "jag_a",
        vec![
            Branch::jagged_f64("j", jag_a.clone()),
            Branch::vector_i32("v", vec_a.clone()),
        ],
    );
    let (fb, _) = write_open(
        "jag_b",
        vec![
            Branch::jagged_f64("j", jag_b.clone()),
            Branch::vector_i32("v", vec_b.clone()),
        ],
    );
    let ta = TreeReader::open(&fa, "Events").unwrap();
    let tb = TreeReader::open(&fb, "Events").unwrap();

    let merged = concat_trees(&[&ta, &tb]).expect("concat");
    let fo = write_merged("jag_out", &merged);
    let to = TreeReader::open(&fo, "Events").unwrap();

    assert_eq!(to.entries(), 3);
    assert_eq!(
        to.branch("j").values().unwrap(),
        BranchValues::VecF64(vec![vec![1.0], vec![2.0, 3.0], vec![4.0, 5.0, 6.0]])
    );
    assert_eq!(
        to.branch("v").values().unwrap(),
        BranchValues::VecI32(vec![vec![1, 2], vec![3], vec![4, 5, 6]])
    );
}

#[test]
fn single_input_round_trips_the_values() {
    let (fa, _) = write_open("solo", vec![Branch::f32("q", vec![1.5, 2.5, 3.5])]);
    let ta = TreeReader::open(&fa, "Events").unwrap();

    let merged = concat_trees(&[&ta]).expect("concat");
    let fo = write_merged("solo_out", &merged);
    let to = TreeReader::open(&fo, "Events").unwrap();

    assert_eq!(to.entries(), 3);
    assert_eq!(
        to.branch("q").values().unwrap(),
        BranchValues::F32(vec![1.5, 2.5, 3.5])
    );
}

#[test]
fn rejects_a_missing_branch() {
    let (fa, _) = write_open(
        "miss_a",
        vec![Branch::i32("i", vec![1]), Branch::f64("x", vec![1.0])],
    );
    let (fb, _) = write_open("miss_b", vec![Branch::i32("i", vec![2])]);
    let ta = TreeReader::open(&fa, "Events").unwrap();
    let tb = TreeReader::open(&fb, "Events").unwrap();

    let Err(err) = concat_trees(&[&ta, &tb]) else {
        panic!("expected an error for a missing branch");
    };
    assert!(
        matches!(&err, oxiroot_io_core::Error::SchemaChanged { detail } if detail.contains("missing branch")),
        "{err:?}"
    );
}

#[test]
fn rejects_a_type_mismatch() {
    let (fa, _) = write_open("ty_a", vec![Branch::i32("v", vec![1, 2])]);
    let (fb, _) = write_open("ty_b", vec![Branch::f64("v", vec![3.0])]);
    let ta = TreeReader::open(&fa, "Events").unwrap();
    let tb = TreeReader::open(&fb, "Events").unwrap();

    let Err(err) = concat_trees(&[&ta, &tb]) else {
        panic!("expected an error for a type mismatch");
    };
    assert!(
        matches!(err, oxiroot_io_core::Error::SchemaChanged { .. }),
        "{err}"
    );
}
