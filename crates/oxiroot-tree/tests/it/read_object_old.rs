//! Old-style unsplit object branches (`TBranchObject`, leaf `TLeafObject`): a
//! whole object stored per entry, the pre-`TBranchElement` mechanism. oxiroot
//! synthesizes one `branch.member` column per (basic / string) member of the
//! object class, decoding it out of every entry's object. Cross-checked against
//! ROOT C++ and uproot. The classes here cover string, double, and int members:
//! `Named` (fName/fTitle), `Parameter<double>`, and `Parameter<int>`.

use std::path::PathBuf;

use oxiroot_io_core::Column;
use oxiroot_io_core::FileReader;
use oxiroot_tree::TreeReader;

fn fixture() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tree_object_old.root"),
    )
    .expect("open fixture")
}

#[test]
fn reads_tbranchobject_members() {
    let f = fixture();
    let t = TreeReader::open(&f, "t").expect("open tree");
    assert_eq!(t.entries(), 3);
    // Each TBranchObject expands into one column per (basic/string) member.
    assert_eq!(
        t.branch_names().collect::<Vec<_>>(),
        vec![
            "nm.fName",
            "nm.fTitle",
            "pd.fName",
            "pd.fVal",
            "pi.fName",
            "pi.fVal"
        ]
    );
    assert!(t.unsupported_branches().collect::<Vec<_>>().is_empty());

    // Named: two string members.
    assert_eq!(
        t.branch("nm.fName").values().unwrap(),
        Column::from(vec!["name0".to_string(), "name1".into(), "name2".into()])
    );
    assert_eq!(
        t.branch("nm.fTitle").values().unwrap(),
        Column::from(vec!["ttl0".to_string(), "ttl1".into(), "ttl2".into()])
    );
    // Parameter<double>: a string name and a double value.
    assert_eq!(
        t.branch("pd.fName").values().unwrap(),
        Column::from(vec!["pd".to_string(), "pd".into(), "pd".into()])
    );
    assert_eq!(
        t.branch("pd.fVal").values().unwrap(),
        Column::F64(vec![0.5, 1.5, 2.5])
    );
    // Parameter<int>: a string name and an int value.
    assert_eq!(
        t.branch("pi.fVal").values().unwrap(),
        Column::I32(vec![0, 10, 20])
    );
}

#[test]
fn object_member_introspection_and_range() {
    let f = fixture();
    let t = TreeReader::open(&f, "t").expect("open tree");
    // Introspection works on the synthesized columns.
    assert_eq!(t.branch_type("pd.fVal"), Some(oxiroot_tree::LeafType::F64));
    assert_eq!(t.branch_type("nm.fName"), Some(oxiroot_tree::LeafType::Str));
    // A range read slices the decoded column.
    assert_eq!(
        t.branch("pi.fVal").range(1..3).values().unwrap(),
        Column::I32(vec![10, 20])
    );
}
