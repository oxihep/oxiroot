//! Write a split (`fSplitLevel > 0`) `std::vector<MyStruct>` `TBranchElement`,
//! then read it back through our own reader. The /tmp file is also checked by
//! uproot / ROOT C++ when run by hand (see `scripts/`).

use crate::support::write_tree_file;
use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::{Compression, Error, FileReader};
use oxiroot_tree::TreeReader;
use oxiroot_tree::raw::Branch;

fn split_branch() -> Branch {
    // Mirrors `fixtures/tree_split.root`: Hit = {float x; float y; int id;}.
    let x = vec![vec![0.0_f32], vec![0.0, 1.0], vec![0.0, 1.0, 2.0]];
    let y = vec![vec![0.5_f32], vec![0.5, 1.5], vec![0.5, 1.5, 2.5]];
    let id = vec![vec![0_i32], vec![0, 10], vec![0, 10, 20]];
    let hits = Column::list_of_structs([("x", x.into()), ("y", y.into()), ("id", id.into())]);
    Branch::new("hits", hits.expect("members of the same shape")).class_name("Hit")
}

fn assert_hits_roundtrip(f: &FileReader) {
    let t = TreeReader::open(f, "T").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        ["hits.x", "hits.y", "hits.id"]
    );
    assert_eq!(
        t.branch("hits.x").values().unwrap(),
        Column::from(vec![vec![0.0_f32], vec![0.0, 1.0], vec![0.0, 1.0, 2.0]])
    );
    assert_eq!(
        t.branch("hits.id").values().unwrap(),
        Column::from(vec![vec![0], vec![0, 10], vec![0, 10, 20]])
    );
}

#[test]
fn writes_split_vector_zstd() {
    // Compression is orthogonal to the split layout, but verify the basket
    // payloads round-trip when Zstd-compressed. (/tmp file also checked by ROOT.)
    let out = PathBuf::from("/tmp/oxiroot_tree_split_zstd.root");
    write_tree_file(&out, "T", &[split_branch()], Compression::Zstd(5)).expect("write");
    assert_hits_roundtrip(&FileReader::open(&out).expect("reopen"));
}

#[test]
fn writes_split_vector_of_struct() {
    let out = PathBuf::from("/tmp/oxiroot_tree_split.root");
    write_tree_file(&out, "T", &[split_branch()], Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_eq!(t.entries(), 3);
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        ["hits.x", "hits.y", "hits.id"]
    );

    assert_eq!(
        t.branch("hits.x").values().unwrap(),
        Column::from(vec![vec![0.0_f32], vec![0.0, 1.0], vec![0.0, 1.0, 2.0]])
    );
    assert_eq!(
        t.branch("hits.y").values().unwrap(),
        Column::from(vec![vec![0.5_f32], vec![0.5, 1.5], vec![0.5, 1.5, 2.5]])
    );
    assert_eq!(
        t.branch("hits.id").values().unwrap(),
        Column::from(vec![vec![0], vec![0, 10], vec![0, 10, 20]])
    );
}

/// A different struct (`double/float/Long64_t` members, an empty entry, more
/// entries) to prove the writer is not specialised to the `Hit` fixture.
#[test]
fn writes_split_vector_general_struct() {
    let px = vec![
        vec![],
        vec![1.0_f64],
        vec![2.0, 3.0],
        vec![4.0],
        vec![5.0, 6.0, 7.0],
    ];
    let py = vec![
        vec![],
        vec![1.5_f64],
        vec![2.5, 3.5],
        vec![4.5],
        vec![5.5, 6.5, 7.5],
    ];
    let w = vec![
        vec![],
        vec![0.5_f32],
        vec![1.0, 1.5],
        vec![2.0],
        vec![2.5, 3.0, 3.5],
    ];
    let id = vec![
        vec![],
        vec![10_i64],
        vec![20, 30],
        vec![40],
        vec![50, 60, 70],
    ];
    let parts = Column::list_of_structs([
        ("px", Column::from(px.clone())),
        ("py", py.clone().into()),
        ("w", w.clone().into()),
        ("id", id.clone().into()),
    ]);
    let branch = Branch::new("parts", parts.unwrap()).class_name("Particle");
    let out = PathBuf::from("/tmp/oxiroot_tree_split_particle.root");
    write_tree_file(&out, "T", &[branch], Compression::Stored).expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let t = TreeReader::open(&f, "T").expect("open tree");
    assert_eq!(t.entries(), 5);
    assert_eq!(t.branch("parts.px").values().unwrap(), Column::from(px));
    assert_eq!(t.branch("parts.py").values().unwrap(), Column::from(py));
    assert_eq!(t.branch("parts.w").values().unwrap(), Column::from(w));
    assert_eq!(t.branch("parts.id").values().unwrap(), Column::from(id));
}

/// A split branch is a list of structs, so its members share their offsets by
/// construction: a member list that is empty, or members that disagree on an
/// entry's element count, cannot be made into one.
#[test]
fn inconsistent_split_members_cannot_be_built() {
    let x = || ("x", Column::from(vec![vec![1.0_f32], vec![2.0, 3.0]]));

    let err = Column::list_of_structs(Vec::<(&str, Column)>::new()).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidInput(m) if m.contains("needs a member")),
        "{err:?}"
    );

    // Different entry counts, then a different element count in entry 1.
    for other in [
        Column::from(vec![vec![9.0_f32]]),
        Column::from(vec![vec![1_i32], vec![2]]),
    ] {
        let err = Column::list_of_structs([x(), ("y", other)]).unwrap_err();
        assert!(
            matches!(&err, Error::InvalidInput(m) if m.contains("\"y\"")),
            "{err:?}"
        );
    }

    // A consistent pair writes.
    let hits = Column::list_of_structs([x(), ("id", vec![vec![1_i32], vec![2, 3]].into())]);
    let branch = Branch::new("hits", hits.unwrap()).class_name("Hit");
    assert!(crate::support::tree_file_bytes("f.root", "T", &[branch], Compression::Stored).is_ok());
}

/// A list whose struct has a member a split branch cannot hold is refused when
/// the tree is written, with the branch named.
#[test]
fn a_struct_member_that_is_not_a_number_is_refused() {
    let names = Column::from(vec![vec!["a".to_string()], vec![]]);
    let hits = Column::list_of_structs([("name", names)]).unwrap();
    let err = crate::support::tree_file_bytes(
        "f.root",
        "T",
        &[Branch::new("hits", hits)],
        Compression::Stored,
    )
    .unwrap_err();
    assert!(
        matches!(&err, Error::Unsupported(m) if m.contains("hits")),
        "{err:?}"
    );
}
