//! A `std::variant<int32_t, float>` field written by official ROOT: the Switch
//! column (index + alternative tag) and the Variant field role.
use oxiroot_io_core::Column;

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_rntuple::NtupleReader;

fn open(name: &str) -> FileReader {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    FileReader::open(path).expect("open fixture")
}

#[test]
fn reads_variant_field() {
    let file = open("rntuple_variant_uncompressed.root");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open RNTuple");
    assert_eq!(ntpl.entries(), 5);

    let v = ntpl.field("v").values().expect("read v");

    // Raw columnar form: two densely-packed alternatives, per-entry tags+indices.
    assert_eq!(
        v,
        Column::Union {
            alternatives: vec![
                ("_0".to_string(), Column::I32(vec![0, 20, 40])),
                ("_1".to_string(), Column::F32(vec![1.5, 3.5])),
            ],
            tags: vec![1, 2, 1, 2, 1],
            indices: vec![0, 0, 1, 1, 2],
        }
    );

    // Resolving each entry (alternatives[tag-1] at index) gives the original
    // alternating int / float sequence: 0, 1.5, 20, 3.5, 40.
    let Column::Union {
        alternatives,
        tags,
        indices,
    } = v
    else {
        panic!("expected a variant");
    };
    let resolved: Vec<String> = tags
        .iter()
        .zip(&indices)
        .map(|(&tag, &idx)| {
            match (
                &alternatives[(tag - 1) as usize].1,
                usize::saturating_from(idx),
            ) {
                (Column::I32(xs), i) => xs[i].to_string(),
                (Column::F32(xs), i) => xs[i].to_string(),
                _ => unreachable!(),
            }
        })
        .collect();
    assert_eq!(resolved, ["0", "1.5", "20", "3.5", "40"]);
}

/// A variant over three clusters, written by ROOT
/// (`scripts/gen_rntuple_variant_clusters.cpp`): entry `i` is the int `i`
/// when `i % 3 == 0`, else the double `i + 0.5`. ROOT counts a switch index
/// within its cluster; the reader adds the earlier clusters' values.
#[test]
fn variant_indices_count_across_clusters() {
    let file = open("rntuple_variant_clusters.root");
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();
    assert_eq!(ntpl.cluster_boundaries(), [0, 100, 350, 500]);
    let check = |v: &Column, from: usize| {
        let Column::Union {
            tags,
            indices,
            alternatives,
        } = v
        else {
            panic!("a union");
        };
        for (k, (&tag, &index)) in tags.iter().zip(indices).enumerate() {
            let i = from + k;
            let index = usize::saturating_from(index);
            match (&alternatives[tag as usize - 1].1, i % 3) {
                (Column::I32(v), 0) => assert_eq!(v[index], i32::try_from(i).unwrap(), "entry {i}"),
                (Column::F64(v), 1 | 2) => assert_eq!(v[index], i as f64 + 0.5, "entry {i}"),
                other => panic!("entry {i}: {other:?}"),
            }
        }
        v.validate("v").unwrap();
    };
    check(&ntpl.field("v").values().unwrap(), 0);
    // A window across the second cluster's edges keeps only its own values.
    let window = ntpl.field("v").range(90..360).values().unwrap();
    check(&window, 90);
    let Column::Union { alternatives, .. } = &window else {
        panic!()
    };
    assert_eq!(
        alternatives.iter().map(|(_, c)| c.len()).sum::<usize>(),
        270
    );
}

/// oxiroot 0.10 and 0.11 wrote each cluster of a variant with every value of
/// the batch it was cut from, and indices into all of them. ROOT reads such a
/// file right (an index is valid within its cluster); so must oxiroot, by
/// moving past what each alternative holds in the earlier clusters.
#[test]
fn variant_clusters_holding_every_value_still_read() {
    use oxiroot_io_core::Compression;
    use oxiroot_rntuple::raw::{Field, NtupleWriter};
    use std::io::Cursor;

    let n = 30usize;
    let tags: Vec<u32> = (0..n).map(|i| 1 + u32::from(i % 2 == 1)).collect();
    let ints: Vec<i32> = (0..n / 2).map(|i| i32::try_from(i).unwrap()).collect();
    let doubles: Vec<f64> = (0..n / 2).map(|i| i as f64 + 0.5).collect();
    let whole = Column::variant(vec![ints.into(), doubles.into()], tags.clone());
    let Column::Union {
        indices,
        alternatives,
        ..
    } = whole
    else {
        panic!()
    };
    // Three clusters of ten entries, each with all the alternatives' values.
    let clusters: Vec<Vec<Field>> = (0..3)
        .map(|c| {
            let r = c * 10..c * 10 + 10;
            vec![Field::new(
                "v",
                Column::Union {
                    tags: tags[r.clone()].to_vec(),
                    indices: indices[r].to_vec(),
                    alternatives: alternatives.clone(),
                },
            )]
        })
        .collect();
    let mut w = NtupleWriter::new(
        Cursor::new(Vec::new()),
        "t.root",
        "ntpl",
        Compression::Stored,
    )
    .unwrap();
    w.write_batches(&clusters).unwrap();
    let file = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
    let ntpl = NtupleReader::open(&file, "ntpl").unwrap();
    assert_eq!(ntpl.cluster_boundaries(), [0, 10, 20, 30]);
    let Column::Union {
        tags: t,
        indices: i,
        alternatives: a,
    } = ntpl.field("v").values().unwrap()
    else {
        panic!("a union");
    };
    for k in 0..n {
        let index = usize::saturating_from(i[k]);
        match (&a[t[k] as usize - 1].1, k % 2) {
            (Column::I32(v), 0) => assert_eq!(v[index], i32::try_from(k / 2).unwrap(), "entry {k}"),
            (Column::F64(v), 1) => assert_eq!(v[index], (k / 2) as f64 + 0.5, "entry {k}"),
            other => panic!("entry {k}: {other:?}"),
        }
    }
}
