//! `Dataset`, `DatasetWriter` and `Batch` over both formats.

use std::io::Cursor;
use std::path::PathBuf;

use oxiroot_dataset::{Batch, Column, Dataset, DatasetObject, DatasetWriter, Format};
use oxiroot_io_core::{Compression, Error, FileReader, FileWriter};
use oxiroot_rntuple::FieldValues;
use oxiroot_tree::BranchValues;

fn ix(n: u64) -> usize {
    usize::try_from(n).unwrap()
}

/// One debug string per entry of a column.
fn column_rows(c: &Column) -> Option<Vec<String>> {
    fn each<T: std::fmt::Debug>(v: &[T]) -> Vec<String> {
        v.iter().map(|x| format!("{x:?}")).collect()
    }
    Some(match c {
        Column::Bool(v) => each(v),
        Column::I8(v) => each(v),
        Column::U8(v) => each(v),
        Column::I16(v) => each(v),
        Column::U16(v) => each(v),
        Column::I32(v) => each(v),
        Column::U32(v) => each(v),
        Column::I64(v) => each(v),
        Column::U64(v) => each(v),
        Column::F32(v) => each(v),
        Column::F64(v) => each(v),
        Column::String { offsets, bytes } => offsets
            .windows(2)
            .map(|w| {
                format!(
                    "{:?}",
                    std::str::from_utf8(&bytes[ix(w[0])..ix(w[1])]).unwrap()
                )
            })
            .collect(),
        Column::List { offsets, values } => {
            let inner = column_rows(values)?;
            offsets
                .windows(2)
                .map(|w| format!("[{}]", inner[ix(w[0])..ix(w[1])].join(", ")))
                .collect()
        }
        Column::Array { len, values } => column_rows(values)?
            .chunks(*len)
            .map(|c| format!("[{}]", c.join(", ")))
            .collect(),
        _ => return None,
    })
}

/// The same rendering of the low-level readers' values.
fn branch_rows(v: &BranchValues) -> Option<Vec<String>> {
    fn each<T: std::fmt::Debug>(v: &[T]) -> Vec<String> {
        v.iter().map(|x| format!("{x:?}")).collect()
    }
    fn rows<T: std::fmt::Debug>(v: &[Vec<T>]) -> Vec<String> {
        v.iter()
            .map(|r| format!("[{}]", each(r).join(", ")))
            .collect()
    }
    use BranchValues as B;
    Some(match v {
        B::Bool(v) => each(v),
        B::I8(v) => each(v),
        B::U8(v) => each(v),
        B::I16(v) => each(v),
        B::U16(v) => each(v),
        B::I32(v) => each(v),
        B::U32(v) => each(v),
        B::I64(v) => each(v),
        B::U64(v) => each(v),
        B::F32(v) => each(v),
        B::F64(v) => each(v),
        B::Str(v) => each(v),
        B::VecBool(v) => rows(v),
        B::VecI8(v) => rows(v),
        B::VecU8(v) => rows(v),
        B::VecI16(v) => rows(v),
        B::VecU16(v) => rows(v),
        B::VecI32(v) => rows(v),
        B::VecU32(v) => rows(v),
        B::VecI64(v) => rows(v),
        B::VecU64(v) => rows(v),
        B::VecF32(v) => rows(v),
        B::VecF64(v) => rows(v),
        B::VecStr(v) => rows(v),
        _ => return None,
    })
}

fn field_rows(v: &FieldValues) -> Option<Vec<String>> {
    fn each<T: std::fmt::Debug>(v: &[T]) -> Vec<String> {
        v.iter().map(|x| format!("{x:?}")).collect()
    }
    fn rows<T: std::fmt::Debug>(v: &[Vec<T>]) -> Vec<String> {
        v.iter()
            .map(|r| format!("[{}]", each(r).join(", ")))
            .collect()
    }
    use FieldValues as F;
    Some(match v {
        F::Bool(v) => each(v),
        F::I8(v) => each(v),
        F::U8(v) => each(v),
        F::I16(v) => each(v),
        F::U16(v) => each(v),
        F::I32(v) => each(v),
        F::U32(v) => each(v),
        F::I64(v) => each(v),
        F::U64(v) => each(v),
        F::F32(v) => each(v),
        F::F64(v) => each(v),
        F::Str(v) => each(v),
        F::VecBool(v) => rows(v),
        F::VecI8(v) => rows(v),
        F::VecU8(v) => rows(v),
        F::VecI16(v) => rows(v),
        F::VecU16(v) => rows(v),
        F::VecI32(v) => rows(v),
        F::VecU32(v) => rows(v),
        F::VecI64(v) => rows(v),
        F::VecU64(v) => rows(v),
        F::VecF32(v) => rows(v),
        F::VecF64(v) => rows(v),
        F::VecStr(v) => rows(v),
        _ => return None,
    })
}

fn sample(n: usize, offset: usize) -> Batch {
    let idx: Vec<i32> = (0..n).map(|i| i32::try_from(i + offset).unwrap()).collect();
    Batch::new()
        .column("idx", idx.clone())
        .unwrap()
        .column(
            "x",
            idx.iter().map(|&i| f64::from(i) * 0.5).collect::<Vec<_>>(),
        )
        .unwrap()
        .column(
            "v",
            idx.iter()
                .map(|&i| {
                    (0..i % 3)
                        .map(|k| f32::from(i16::try_from(k).unwrap()) + 0.25)
                        .collect()
                })
                .collect::<Vec<Vec<f32>>>(),
        )
        .unwrap()
        .column("s", idx.iter().map(|i| format!("e{i}")).collect::<Vec<_>>())
        .unwrap()
        .column("a", idx.iter().map(|&i| [i, -i]).collect::<Vec<[i32; 2]>>())
        .unwrap()
}

fn write(format: Format, batches: &[Batch], cluster_bytes: usize) -> Vec<u8> {
    let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", format)
        .compression(Compression::Zstd(1))
        .cluster_bytes(cluster_bytes);
    for b in batches {
        w.write(b).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const COLUMNS: [&str; 5] = ["idx", "x", "v", "s", "a"];

#[test]
fn round_trips_through_both_formats() {
    let input = sample(50, 0);
    for format in [Format::Tree, Format::Ntuple] {
        let bytes = write(format, std::slice::from_ref(&input), usize::MAX);
        let file = FileReader::from_bytes(bytes).unwrap();
        let ds = Dataset::open(&file, "events").unwrap();
        assert_eq!(ds.format(), format);
        assert_eq!(ds.entries(), 50);
        let back = ds.read(COLUMNS).unwrap();
        assert_eq!(back, input, "{format:?}");
    }
}

#[test]
fn batches_cover_every_entry_once_in_order() {
    let parts: Vec<Batch> = (0..10).map(|k| sample(37, 37 * k)).collect();
    let mut whole = Batch::new();
    for p in &parts {
        whole.append(p.clone()).unwrap();
    }
    for format in [Format::Tree, Format::Ntuple] {
        // Small clusters: each buffered write is flushed on its own.
        let bytes = write(format, &parts, 1);
        let file = FileReader::from_bytes(bytes).unwrap();
        let ds = Dataset::open(&file, "events").unwrap();
        for step in [0, 1, 37, 50, 100, 1000] {
            let mut joined = Batch::new();
            let mut sizes = Vec::new();
            for b in ds.batches(COLUMNS).step(step) {
                let b = b.unwrap();
                sizes.push(b.len());
                joined.append(b).unwrap();
            }
            assert_eq!(joined, whole, "{format:?} step {step}");
            if step == 0 {
                assert!(
                    sizes.iter().all(|&n| n == 37),
                    "one cluster per batch: {sizes:?}"
                );
            }
        }
        let window: Vec<Batch> = ds
            .batches(["idx"])
            .range(40..95)
            .step(20)
            .collect::<Result<_, _>>()
            .unwrap();
        let mut got = Vec::new();
        for b in &window {
            got.extend_from_slice(b.values::<i32>("idx").unwrap());
        }
        assert_eq!(got, (40..95).collect::<Vec<i32>>(), "{format:?}");
    }
}

#[test]
fn typed_access_is_strict() {
    let b = sample(4, 0);
    assert_eq!(b.values::<i32>("idx").unwrap(), [0, 1, 2, 3]);
    let err = b.values::<f32>("x").unwrap_err();
    assert_eq!(
        err,
        Error::TypeMismatch {
            name: "x".into(),
            found: "f64".into(),
            expected: "f32".into()
        }
    );
    assert!(matches!(
        b.values::<f64>("nope"),
        Err(Error::NotFound { .. })
    ));
    let v = b.jagged::<f32>("v").unwrap();
    assert_eq!(v.len(), 4);
    assert_eq!(v.row(2), [0.25, 1.25]);
    assert_eq!(
        b.strings("s").unwrap().iter().collect::<Vec<_>>(),
        ["e0", "e1", "e2", "e3"]
    );
    assert_eq!(
        b.arrays::<i32>("a").unwrap(),
        (&[0, 0, 1, -1, 2, -2, 3, -3][..], 2)
    );
    assert!(b.jagged::<f32>("a").is_err(), "an array is not a list");
}

#[test]
fn filters_slices_and_selects() {
    let b = sample(6, 0);
    let even = b.filter(&[true, false, true, false, true, false]).unwrap();
    assert_eq!(even.values::<i32>("idx").unwrap(), [0, 2, 4]);
    assert_eq!(even.jagged::<f32>("v").unwrap().row(1), [0.25, 1.25]);
    assert_eq!(even.strings("s").unwrap().get(2), "e4");
    assert_eq!(even.arrays::<i32>("a").unwrap().0, [0, 0, 2, -2, 4, -4]);
    assert!(b.filter(&[true]).is_err());

    let mid = b.slice(2..4);
    assert_eq!(mid.values::<i32>("idx").unwrap(), [2, 3]);
    assert_eq!(mid.jagged::<f32>("v").unwrap().row(0), [0.25, 1.25]);
    assert_eq!(
        mid,
        b.filter(&[false, false, true, true, false, false]).unwrap()
    );

    let picked = b.select(["s", "idx"]).unwrap();
    assert_eq!(picked.names().collect::<Vec<_>>(), ["s", "idx"]);
}

#[test]
fn a_batch_checks_its_columns() {
    let b = Batch::new().column("x", vec![1_i32, 2]).unwrap();
    assert!(matches!(
        b.clone().column("y", vec![1.0_f64]),
        Err(Error::LengthMismatch { .. })
    ));
    assert!(
        b.clone().column("x", vec![3_i32, 4]).is_err(),
        "duplicate name"
    );
    assert!(
        Batch::new()
            .list("l", vec![0, 2, 1], vec![1_i32, 2])
            .is_err(),
        "decreasing offsets"
    );
    assert!(
        Batch::new().list("l", vec![0, 3], vec![1_i32, 2]).is_err(),
        "offsets past the values"
    );
    let l = Batch::new()
        .list("l", vec![0, 2, 2, 3], vec![1_i32, 2, 3])
        .unwrap();
    assert_eq!(l.jagged::<i32>("l").unwrap().row(2), [3]);
}

#[test]
fn a_column_the_format_cannot_hold_is_refused_before_writing() {
    let b = Batch::new().column("o", vec![Some(1_i32), None]).unwrap();
    let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", Format::Tree);
    let err = w.write(&b).unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("Option<i32>"), "{err}");
    assert_eq!(w.entries(), 0);
    assert!(w.finish().is_err(), "nothing was written");

    // An RNTuple holds an optional.
    let bytes = write(Format::Ntuple, std::slice::from_ref(&b), usize::MAX);
    let file = FileReader::from_bytes(bytes).unwrap();
    let back = Dataset::open(&file, "events").unwrap().read(["o"]).unwrap();
    assert_eq!(back, b);
}

#[test]
fn a_later_batch_must_match_the_first() {
    let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", Format::Ntuple);
    w.write(&sample(3, 0)).unwrap();
    let other = Batch::new().column("idx", vec![1.0_f64]).unwrap();
    assert!(matches!(w.write(&other), Err(Error::SchemaChanged { .. })));
}

#[test]
fn nested_lists_round_trip_through_an_rntuple() {
    let b = Batch::new()
        .column(
            "vv",
            vec![vec![vec![1_i32], vec![]], vec![], vec![vec![2, 3]]],
        )
        .unwrap()
        .column(
            "vs",
            vec![vec!["a".to_string()], vec![], vec!["b".into(), "c".into()]],
        )
        .unwrap();
    let bytes = write(Format::Ntuple, std::slice::from_ref(&b), usize::MAX);
    let file = FileReader::from_bytes(bytes).unwrap();
    let back = Dataset::open(&file, "events")
        .unwrap()
        .read(["vv", "vs"])
        .unwrap();
    assert_eq!(back, b);
    assert_eq!(round_trip(Format::Tree, &b), b, "a TTree holds them too");
}

#[test]
fn datasets_go_into_a_file_writer_next_to_each_other() {
    let b = sample(5, 0);
    let bytes = FileWriter::create("multi.root")
        .put(DatasetObject::new("tree", Format::Tree, &b).unwrap())
        .put(DatasetObject::new("ntuple", Format::Ntuple, &b).unwrap())
        .to_bytes(Compression::None)
        .unwrap();
    let file = FileReader::from_bytes(bytes).unwrap();
    for (name, format) in [("tree", Format::Tree), ("ntuple", Format::Ntuple)] {
        let ds = Dataset::open(&file, name).unwrap();
        assert_eq!(ds.format(), format);
        assert_eq!(ds.read(COLUMNS).unwrap(), b);
    }
    assert!(matches!(
        Dataset::open(&file, "nope"),
        Err(Error::NotFound { .. })
    ));
}

/// Every committed TTree and RNTuple fixture reads through `Dataset`, and each
/// column equals the low-level reader's values converted — the flat paths
/// agree with the general one.
#[test]
fn every_fixture_reads_the_same_as_the_low_level_readers() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut checked = 0;
    let mut compared = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "root") {
            continue;
        }
        let Ok(file) = FileReader::open(&path) else {
            continue;
        };
        let keys: Vec<(String, String)> = file
            .keys()
            .iter()
            .map(|k| (k.name.clone(), k.class_name.clone()))
            .collect();
        for (name, class) in keys {
            if !matches!(class.as_str(), "TTree" | "ROOT::RNTuple") {
                continue;
            }
            let Ok(ds) = Dataset::open(&file, &name) else {
                continue;
            };
            for column in ds.columns() {
                let Ok(got) = ds.read([column]) else { continue };
                let got = got.get(column).unwrap();
                assert_eq!(got.len() as u64, ds.entries(), "{path:?} {name} {column}");
                // Reading in windows of 3 gives the same column.
                let mut joined: Option<Column> = None;
                let mut glued = Batch::new();
                for b in ds.batches([column]).step(3) {
                    glued.append(b.unwrap()).unwrap();
                }
                if let Some(c) = glued.get(column) {
                    joined = Some(c.clone());
                }
                if ds.entries() > 0 {
                    assert_eq!(
                        joined.as_ref(),
                        Some(got),
                        "{path:?} {name} {column} in windows"
                    );
                }
                // The same entries as the format's own reader.
                let low = match (ds.as_tree(), ds.as_ntuple()) {
                    (Some(t), _) => t.branch(column).values().ok().and_then(|v| branch_rows(&v)),
                    (_, Some(n)) => n.field(column).values().ok().and_then(|v| field_rows(&v)),
                    _ => None,
                };
                if let (Some(low), Some(mine)) = (low, column_rows(got)) {
                    assert_eq!(
                        mine, low,
                        "{path:?} {name} {column} against the low-level reader"
                    );
                    compared += 1;
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 50, "only {checked} columns checked");
    assert!(compared > 50, "only {compared} columns compared");
    eprintln!("{checked} fixture columns read, {compared} compared with the low-level readers");
}

/// Parallel reads (columns, baskets and pages concurrently) give exactly what
/// serial ones do, on a file whose clusters are cut into many chunks.
#[cfg(feature = "rayon")]
#[test]
fn parallel_reads_match_serial_ones() {
    let parts: Vec<Batch> = (0..4).map(|k| sample(5000, 5000 * k)).collect();
    for format in [Format::Tree, Format::Ntuple] {
        let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", format)
            .cluster_bytes(64 * 1024);
        for p in &parts {
            w.write(p).unwrap();
        }
        let file = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
        let ds = Dataset::open(&file, "events").unwrap();
        let serial: Vec<Batch> = ds
            .batches(COLUMNS)
            .step(3000)
            .collect::<Result<_, _>>()
            .unwrap();
        let parallel: Vec<Batch> = ds
            .batches(COLUMNS)
            .step(3000)
            .parallel(true)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(serial, parallel, "{format:?}");
        assert!(serial.len() > 2, "{format:?}: several batches");
        let mut whole = Batch::new();
        for b in serial {
            whole.append(b).unwrap();
        }
        assert_eq!(whole, ds.read(COLUMNS).unwrap(), "{format:?}");
    }
}

fn round_trip(format: Format, batch: &Batch) -> Batch {
    let bytes = write(format, std::slice::from_ref(batch), usize::MAX);
    let file = FileReader::from_bytes(bytes).unwrap();
    let ds = Dataset::open(&file, "events").unwrap();
    let names: Vec<&str> = batch.names().collect();
    ds.read(names).unwrap()
}

#[test]
fn vectors_of_bools_strings_and_vectors_round_trip_through_both_formats() {
    let b = Batch::new()
        .column("vb", vec![vec![true, false], vec![], vec![true]])
        .unwrap()
        .column(
            "vs",
            vec![vec!["a".to_string(), "bc".into()], vec![], vec!["d".into()]],
        )
        .unwrap()
        .column(
            "vv",
            vec![vec![vec![1.5_f32], vec![]], vec![], vec![vec![2.0, 3.0]]],
        )
        .unwrap()
        .column("vu", vec![vec![1_u32, 2], vec![], vec![u32::MAX]])
        .unwrap()
        .column("vl", vec![vec![u64::MAX], vec![3], vec![]])
        .unwrap();
    for format in [Format::Tree, Format::Ntuple] {
        assert_eq!(round_trip(format, &b), b, "{format:?}");
    }
}

#[test]
fn structs_round_trip_and_a_tree_takes_them_in_a_list() {
    let hit = Column::Struct(vec![
        ("x".into(), Column::F32(vec![1.0, 2.0, 3.0])),
        ("id".into(), Column::I32(vec![7, 8, 9])),
    ]);
    let hits = Column::List {
        offsets: vec![0, 2, 2, 3],
        values: Box::new(hit.clone()),
    };
    let b = Batch::new()
        .column("hit", hit)
        .unwrap()
        .column("hits", hits)
        .unwrap();
    let back = round_trip(Format::Ntuple, &b);
    assert_eq!(back, b);
    assert_eq!(back.get("hit").unwrap().type_name(), "{x: f32, id: i32}");
    assert_eq!(
        back.get("hits").unwrap().type_name(),
        "Vec<{x: f32, id: i32}>"
    );
    assert_eq!(
        back.get("hit").unwrap().member("id"),
        Some(&Column::I32(vec![7, 8, 9]))
    );
    assert_eq!(back.filter(&[true, false, true]).unwrap().len(), 2);

    // A tree holds the list as a split std::vector<hits>, and reads it back
    // as one column; a struct on its own it refuses.
    let lists = b.select(["hits"]).unwrap();
    assert_eq!(round_trip(Format::Tree, &lists), lists);
    let err = DatasetObject::new("t", Format::Tree, &b).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("\"hit\"") && msg.contains("{x: f32, id: i32}"),
        "{msg}"
    );
    assert!(msg.contains("std::vector<Struct>"), "{msg}");
}

/// Entry `i` holds `i % 4` hits, `x = i + k / 10` and `id = i * 10 + k`.
fn hits(n: usize) -> Column {
    let mut offsets = vec![0_u64];
    let (mut x, mut id, mut on) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..n {
        for k in 0..i % 4 {
            x.push(i as f64 + k as f64 / 10.0);
            id.push(i64::try_from(i * 10 + k).unwrap());
            on.push(k % 2 == 0);
        }
        offsets.push(x.len() as u64);
    }
    Column::List {
        offsets,
        values: Box::new(Column::Struct(vec![
            ("x".into(), Column::F64(x)),
            ("id".into(), Column::I64(id)),
            ("on".into(), Column::Bool(on)),
        ])),
    }
}

#[test]
fn a_list_of_structs_streams_into_a_tree_in_many_baskets() {
    // Small clusters: each `write` is flushed as a cluster of its own.
    let all = Batch::new()
        .column("n", (0..5000).collect::<Vec<i32>>())
        .unwrap()
        .column("hits", hits(5000))
        .unwrap();
    let parts = [
        all.slice(0..1700),
        all.slice(1700..3300),
        all.slice(3300..5000),
    ];
    let file = FileReader::from_bytes(write(Format::Tree, &parts, 4096)).unwrap();
    let ds = Dataset::open(&file, "events").unwrap();
    assert_eq!(ds.columns(), ["n", "hits"]);
    assert_eq!(ds.read(["n", "hits"]).unwrap(), all);

    let tree = ds.as_tree().unwrap();
    assert_eq!(
        tree.collection_members("hits"),
        ["hits.x", "hits.id", "hits.on"]
    );
    // One basket per member per `write`.
    assert_eq!(
        tree.basket_boundaries("hits.x").unwrap(),
        [0, 1700, 3300, 5000]
    );
    // A member reads on its own too.
    let x = ds.read(["hits.x"]).unwrap();
    let Some(Column::List { offsets, values }) = all.get("hits") else {
        panic!("a list");
    };
    assert_eq!(
        x.jagged::<f64>("hits.x").unwrap().offsets(),
        offsets.as_slice()
    );
    assert_eq!(
        Some(&Column::F64(
            x.jagged::<f64>("hits.x").unwrap().values().to_vec()
        )),
        values.member("x")
    );

    // Batches cut at basket boundaries put the pieces back together.
    let mut joined = Batch::new();
    for batch in ds.batches(["n", "hits"]).step(700) {
        joined.append(batch.unwrap()).unwrap();
    }
    assert_eq!(joined, all);
    let mut window = Batch::new();
    for batch in ds.batches(["hits"]).range(1234..2345) {
        window.append(batch.unwrap()).unwrap();
    }
    assert_eq!(window, all.slice(1234..2345).select(["hits"]).unwrap());
}

#[test]
fn a_root_written_split_vector_reads_as_a_list_of_structs() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tree_split.root");
    let file = FileReader::open(path).unwrap();
    let ds = Dataset::open(&file, "T").unwrap();
    assert_eq!(ds.columns(), ["hits"]);
    let b = ds.read(["hits"]).unwrap();
    assert_eq!(
        b.get("hits").unwrap().type_name(),
        "Vec<{x: f32, y: f32, id: i32}>"
    );
    let low = ds.as_tree().unwrap();
    let x = low.branch("hits.x").flat().unwrap();
    let Some(Column::List { offsets, values }) = b.get("hits") else {
        panic!("a list");
    };
    assert_eq!(*offsets, x.offsets);
    assert_eq!(
        values.member("x"),
        Some(&Column::F32(match x.values {
            BranchValues::F32(v) => v,
            other => panic!("{other:?}"),
        }))
    );
}

#[test]
fn an_empty_dataset_is_written_with_its_columns() {
    let b = sample(0, 0);
    for format in [Format::Tree, Format::Ntuple] {
        let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", format);
        w.write(&b).unwrap();
        assert_eq!(w.entries(), 0);
        let file = FileReader::from_bytes(w.finish().unwrap().into_inner()).unwrap();
        let ds = Dataset::open(&file, "events").unwrap();
        assert_eq!(ds.entries(), 0, "{format:?}");
        let back = ds.read(COLUMNS).unwrap();
        assert_eq!(back.names().collect::<Vec<_>>(), COLUMNS, "{format:?}");
        // The fixed array keeps its width, in a TTree too.
        assert_eq!(back, b, "{format:?}");
        assert_eq!(ds.batches(COLUMNS).count(), 0);
    }
    // An empty list of structs keeps its members.
    let empty_hits = Batch::new().column("hits", hits(0)).unwrap();
    for format in [Format::Tree, Format::Ntuple] {
        assert_eq!(round_trip(format, &empty_hits), empty_hits, "{format:?}");
    }
    let w = DatasetWriter::new(Cursor::new(Vec::new()), "t.root", "events", Format::Tree);
    assert!(w.finish().is_err(), "no batch, no columns");
}

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use oxiroot_dataset::arrow_rs::array::{Array, RecordBatch};
    use oxiroot_dataset::arrow_rs::schema::DataType;

    fn every_shape() -> Batch {
        sample(5, 0)
            .column("o", vec![Some(1.5_f64), None, Some(2.5), None, Some(3.5)])
            .unwrap()
            .column(
                "hit",
                Column::Struct(vec![
                    ("x".into(), Column::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                    ("id".into(), Column::U16(vec![1, 2, 3, 4, 5])),
                ]),
            )
            .unwrap()
            .column(
                "vv",
                vec![
                    vec![vec![1_i64], vec![]],
                    vec![],
                    vec![vec![2, 3]],
                    vec![],
                    vec![vec![4]],
                ],
            )
            .unwrap()
            .column("flags", vec![true, false, true, true, false])
            .unwrap()
    }

    #[test]
    fn a_batch_round_trips_through_arrow() {
        let b = every_shape();
        let rb: RecordBatch = b.to_arrow().unwrap();
        assert_eq!(rb.num_rows(), 5);
        assert_eq!(rb.num_columns(), b.names().len());
        assert!(matches!(
            rb.schema().field_with_name("v").unwrap().data_type(),
            DataType::LargeList(_)
        ));
        assert!(matches!(
            rb.schema().field_with_name("a").unwrap().data_type(),
            DataType::FixedSizeList(_, 2)
        ));
        assert!(matches!(
            rb.schema().field_with_name("hit").unwrap().data_type(),
            DataType::Struct(_)
        ));
        assert_eq!(rb.column_by_name("o").unwrap().null_count(), 2);
        assert_eq!(Batch::from_arrow(&rb).unwrap(), b);
        assert_eq!(b.clone().into_arrow().unwrap(), rb);
    }

    #[test]
    fn a_sliced_record_batch_reads_back_as_its_slice() {
        let b = every_shape();
        let rb = b.to_arrow().unwrap().slice(1, 3);
        assert_eq!(Batch::from_arrow(&rb).unwrap(), b.slice(1..4));
    }

    #[test]
    fn a_dataset_converts_through_arrow() {
        let b = every_shape();
        let bytes = write(Format::Ntuple, std::slice::from_ref(&b), usize::MAX);
        let file = FileReader::from_bytes(bytes).unwrap();
        let ds = Dataset::open(&file, "events").unwrap();
        let names: Vec<&str> = b.names().collect();
        let rb = ds.read(names).unwrap().into_arrow().unwrap();
        assert_eq!(Batch::from_arrow(&rb).unwrap(), b);
    }
}

#[cfg(feature = "polars")]
mod polars {
    use super::*;
    use oxiroot_dataset::IntoColumn;
    use oxiroot_dataset::polars_rs::prelude::DataType;

    fn every_shape() -> Batch {
        sample(5, 0)
            .column("o", vec![Some(1.5_f64), None, Some(2.5), None, Some(3.5)])
            .unwrap()
            .column(
                "os",
                Column::Optional {
                    valid: vec![true, false, false, true, true],
                    values: Box::new(vec!["a", "b", "c"].into_column()),
                },
            )
            .unwrap()
            .column(
                "hit",
                Column::Struct(vec![
                    ("x".into(), Column::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                    ("id".into(), Column::U16(vec![1, 2, 3, 4, 5])),
                ]),
            )
            .unwrap()
            .column(
                "vv",
                vec![
                    vec![vec![1_i64], vec![]],
                    vec![],
                    vec![vec![2, 3]],
                    vec![],
                    vec![vec![4]],
                ],
            )
            .unwrap()
            .column("flags", vec![true, false, true, true, false])
            .unwrap()
    }

    #[test]
    fn a_batch_round_trips_through_polars() {
        let b = every_shape();
        let df = b.to_polars().unwrap();
        assert_eq!(df.height(), 5);
        assert_eq!(df.width(), b.names().len());
        assert!(matches!(df.column("v").unwrap().dtype(), DataType::List(_)));
        assert!(matches!(
            df.column("a").unwrap().dtype(),
            DataType::Array(_, 2)
        ));
        assert!(matches!(
            df.column("hit").unwrap().dtype(),
            DataType::Struct(_)
        ));
        assert_eq!(df.column("s").unwrap().dtype(), &DataType::String);
        assert_eq!(df.column("o").unwrap().null_count(), 2);
        assert_eq!(df.column("os").unwrap().null_count(), 2);
        assert_eq!(Batch::from_polars(&df).unwrap(), b);
    }

    #[test]
    fn a_dataset_converts_through_polars() {
        let b = every_shape();
        let bytes = write(Format::Ntuple, std::slice::from_ref(&b), usize::MAX);
        let file = FileReader::from_bytes(bytes).unwrap();
        let names: Vec<&str> = b.names().collect();
        let df = Dataset::open(&file, "events")
            .unwrap()
            .read(names)
            .unwrap()
            .into_polars()
            .unwrap();
        assert_eq!(Batch::from_polars(&df).unwrap(), b);
    }

    #[test]
    fn an_optional_of_an_optional_folds_into_one_level_of_nulls() {
        // [Some(Some(1)), Some(None), None, Some(Some(4)), None] inside a
        // struct and on its own.
        let nested = || Column::Optional {
            valid: vec![true, true, false, true, false],
            values: Box::new(Column::Optional {
                valid: vec![true, false, true],
                values: Box::new(Column::I32(vec![1, 4])),
            }),
        };
        let b = Batch::new()
            .column("oo", nested())
            .unwrap()
            .column(
                "rec",
                Column::Struct(vec![
                    ("oo".into(), nested()),
                    ("n".into(), Column::U8(vec![0; 5])),
                ]),
            )
            .unwrap();
        let folded = Column::Optional {
            valid: vec![true, false, false, true, false],
            values: Box::new(Column::I32(vec![1, 4])),
        };
        let df = b.to_polars().unwrap();
        assert_eq!(df.column("oo").unwrap().null_count(), 3);
        let back = Batch::from_polars(&df).unwrap();
        assert_eq!(back.get("oo"), Some(&folded));
        assert_eq!(back.get("rec").unwrap().member("oo"), Some(&folded));
        #[cfg(feature = "arrow")]
        {
            let arrow_back = Batch::from_arrow(&b.to_arrow().unwrap()).unwrap();
            assert_eq!(arrow_back.get("oo"), Some(&folded));
            assert_eq!(arrow_back.get("rec").unwrap().member("oo"), Some(&folded));
        }
    }
}
