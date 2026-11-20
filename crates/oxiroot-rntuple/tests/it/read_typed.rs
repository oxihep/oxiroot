//! The `FieldRead` builder: typed reads, the type-mismatch error, entry
//! windows across clusters, cluster boundaries and the read → write round trip.
use std::io::Cursor;

use oxiroot_io_core::{Compression, Error, FileReader};
use oxiroot_rntuple::{Field, FieldValues, NtupleReader, NtupleWriter};

/// Three clusters: entries [0, 2), [2, 5), [5, 6).
fn file() -> FileReader {
    let mut w = NtupleWriter::new(
        Cursor::new(Vec::new()),
        "t.root",
        "ntpl",
        Compression::Zstd(1),
    )
    .unwrap();
    for (x, v, s, o) in [
        (
            vec![1.0, 2.0],
            vec![vec![1], vec![]],
            vec!["a", "b"],
            vec![Some(1), None],
        ),
        (
            vec![3.0, 4.0, 5.0],
            vec![vec![2, 3], vec![4], vec![]],
            vec!["c", "d", "e"],
            vec![None, Some(4), Some(5)],
        ),
        (vec![6.0], vec![vec![6, 7]], vec!["f"], vec![Some(6)]),
    ] {
        w.write_batch(&[
            Field::f64("x", x),
            Field::vector_i32("v", v),
            Field::string("s", s.into_iter().map(String::from).collect()),
            Field::optional_i32("o", o),
        ])
        .unwrap();
    }
    assert_eq!(w.entries(), 6);
    FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap()
}

#[test]
fn reads_typed_values() {
    let f = file();
    let n = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(
        n.field("x").read::<f64>().unwrap(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
    assert_eq!(n.field("s").read::<String>().unwrap()[5], "f");
    assert_eq!(
        n.field("o").read::<Option<i32>>().unwrap(),
        [Some(1), None, None, Some(4), Some(5), Some(6)]
    );
    let err = n.field("x").read::<f32>().unwrap_err();
    assert_eq!(
        err,
        Error::TypeMismatch {
            name: "x".into(),
            found: "f64".into(),
            expected: "f32".into(),
        }
    );
}

#[test]
fn windows_match_the_full_read_across_clusters() {
    let f = file();
    let n = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(n.cluster_boundaries(), [0, 2, 5, 6]);
    for name in ["x", "v", "s", "o"] {
        let full = n.field(name).values().unwrap();
        for a in 0..=6u64 {
            for b in a..=7u64 {
                let mut want = full.clone();
                want.skip(usize::try_from(a).unwrap());
                want.truncate(usize::try_from(b.min(6) - a).unwrap());
                let got = n.field(name).range(a..b).values().unwrap();
                assert_eq!(got, want, "field {name} [{a}, {b})");
            }
        }
    }
    assert_eq!(
        n.field("v").range(1..4).read::<Vec<i32>>().unwrap(),
        [vec![], vec![2, 3], vec![4]]
    );
}

#[test]
fn a_read_field_writes_back_the_same_way() {
    let f = file();
    let n = NtupleReader::open(&f, "ntpl").unwrap();
    let fields: Vec<Field> = ["x", "v", "s"]
        .iter()
        .map(|name| n.field(name).range(1..4).to_field().unwrap())
        .collect();
    let mut w =
        NtupleWriter::new(Cursor::new(Vec::new()), "o.root", "ntpl", Compression::None).unwrap();
    w.write_batch(&fields).unwrap();
    let out = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let o = NtupleReader::open(&out, "ntpl").unwrap();
    assert_eq!(
        o.field("x").values().unwrap(),
        FieldValues::F64(vec![2.0, 3.0, 4.0])
    );
    assert_eq!(
        o.field("v").read::<Vec<i32>>().unwrap(),
        [vec![], vec![2, 3], vec![4]]
    );
    assert_eq!(o.field("s").read::<String>().unwrap(), ["b", "c", "d"]);
}

#[test]
fn flat_matches_values_on_every_window() {
    let f = file();
    let n = NtupleReader::open(&f, "ntpl").unwrap();
    let full = n.field("v").read::<Vec<i32>>().unwrap();
    for a in 0..=6u64 {
        for b in a..=7u64 {
            let j = n.field("v").range(a..b).flat().unwrap();
            let FieldValues::I32(vals) = &j.values else {
                panic!("{:?}", j.values)
            };
            let rows: Vec<Vec<i32>> = j
                .offsets
                .windows(2)
                .map(|w| {
                    vals[usize::try_from(w[0]).unwrap()..usize::try_from(w[1]).unwrap()].to_vec()
                })
                .collect();
            let (lo, hi) = (
                usize::try_from(a).unwrap(),
                usize::try_from(b.min(6)).unwrap(),
            );
            assert_eq!(rows, full[lo..hi], "[{a}, {b})");
        }
    }
    assert!(
        n.field("x").flat().is_err(),
        "a scalar field has no flat form"
    );
    assert!(n.field("s").flat().is_err(), "nor does a string");
}
