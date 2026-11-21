//! End-to-end `hadd`-style file merging via [`oxiroot::hadd`].
use crate::support::{NtupleObject, TreeObject};
use oxiroot::file::ObjString;
use oxiroot::hadd::{MergeKind, Merger};
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::{Field, FieldValues};
use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;
use oxiroot::tree::raw::{Branch, BranchValues};
use oxiroot_io_core::cast::SaturatingFrom;

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxiroot_hadd_{name}.root"))
}

#[test]
fn sums_histograms_and_copies_other_objects() {
    // Two files, each a Hist1D "h" (2 in-range fills) plus an ObjString "meta".
    for (tag, xs) in [("hist_a", [0.5, 1.5]), ("hist_b", [2.5, 3.5])] {
        let mut h = Hist::reg(4, 0.0, 4.0).double().named("h").titled("h");
        for x in xs {
            h.fill(x);
        }
        let h = h.to_root().unwrap();
        FileWriter::create(tmp(tag))
            .add(&h)
            .add(&ObjString::new("provenance").named("meta"))
            .compression(Compression::Stored)
            .write()
            .unwrap();
    }

    let out = tmp("hist_out");
    let report = Merger::new()
        .inputs([tmp("hist_a"), tmp("hist_b")])
        .compression(Compression::Stored)
        .merge(&out)
        .unwrap();

    let MergeKind::Histograms(outcome) = &report.kind else {
        panic!("not a histogram merge: {report}");
    };
    assert!(outcome.summed.contains(&"h".to_string()), "{report}");
    assert!(outcome.copied.contains(&"meta".to_string()), "{report}");
    assert!(outcome.skipped.is_empty(), "{report}");

    // The merged histogram is the bin-by-bin sum: 4 in-range entries total.
    let fo = FileReader::open(&out).unwrap();
    let h = fo.get::<Hist1D>("h").unwrap();
    assert_eq!(h.values().iter().sum::<f64>(), 4.0);
    // The non-summable object was carried over verbatim.
    assert!(fo.get::<ObjString>("meta").is_ok());
}

#[test]
fn concatenates_tree_files() {
    FileWriter::create(tmp("tree_a"))
        .add(TreeObject::new(
            "Events",
            vec![Branch::i32("i", vec![0, 1])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    FileWriter::create(tmp("tree_b"))
        .add(TreeObject::new(
            "Events",
            vec![Branch::i32("i", vec![2, 3, 4])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let out = tmp("tree_out");
    let report = Merger::new()
        .input(tmp("tree_a"))
        .input(tmp("tree_b"))
        .compression(Compression::Stored)
        .merge(&out)
        .unwrap();

    assert_eq!(
        report.kind,
        MergeKind::Tree {
            name: "Events".into(),
            entries: 5
        }
    );

    let fo = FileReader::open(&out).unwrap();
    let t = TreeReader::open(&fo, "Events").unwrap();
    assert_eq!(t.entries(), 5);
    assert_eq!(
        t.branch("i").values().unwrap(),
        BranchValues::I32(vec![0, 1, 2, 3, 4])
    );
}

#[test]
fn concatenates_rntuple_files() {
    FileWriter::create(tmp("rn_a"))
        .add(NtupleObject::new(
            "ntpl",
            vec![Field::f64("x", vec![0.0, 1.0])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    FileWriter::create(tmp("rn_b"))
        .add(NtupleObject::new(
            "ntpl",
            vec![Field::f64("x", vec![2.0, 3.0, 4.0])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let out = tmp("rn_out");
    let report = Merger::new()
        .inputs([tmp("rn_a"), tmp("rn_b")])
        .compression(Compression::Stored)
        .merge(&out)
        .unwrap();

    assert_eq!(
        report.kind,
        MergeKind::Ntuple {
            name: "ntpl".into(),
            entries: 5
        }
    );

    let fo = FileReader::open(&out).unwrap();
    let nt = NtupleReader::open(&fo, "ntpl").unwrap();
    assert_eq!(nt.entries(), 5);
    assert_eq!(
        nt.field("x").values().unwrap(),
        FieldValues::F64(vec![0.0, 1.0, 2.0, 3.0, 4.0])
    );
}

#[test]
fn refuses_a_fileset_mixing_a_tree_with_histograms() {
    FileWriter::create(tmp("mix_tree"))
        .add(TreeObject::new(
            "Events",
            vec![Branch::i32("i", vec![0, 1])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("h");
    h.fill(1.5);
    let h = h.to_root().unwrap();
    FileWriter::create(tmp("mix_hist"))
        .add(&h)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let out = tmp("mix_out");
    let Err(err) = Merger::new()
        .inputs([tmp("mix_tree"), tmp("mix_hist")])
        .compression(Compression::Stored)
        .merge(&out)
    else {
        panic!("expected a mixed-fileset error");
    };
    assert!(err.to_string().contains("cannot combine"), "{err}");
}

#[test]
fn a_tree_merge_streams_one_batch_per_input() {
    let inputs: Vec<_> = (0..3)
        .map(|k| {
            let path = tmp(&format!("stream_tree_{k}"));
            let n = k + 1;
            FileWriter::create(&path)
                .add(TreeObject::new(
                    "Events",
                    vec![
                        Branch::i32("i", (0..n).map(|x| k * 10 + x).collect()),
                        Branch::jagged_f32(
                            "hits",
                            (0..n)
                                .map(|x| vec![x as f32; usize::saturating_from(x)])
                                .collect(),
                        ),
                        Branch::string("tag", (0..n).map(|x| format!("t{k}{x}")).collect()),
                        Branch::vector_f64("vec", (0..n).map(|x| vec![f64::from(x); 2]).collect()),
                    ],
                ))
                .compression(Compression::Zstd(3))
                .write()
                .unwrap();
            path
        })
        .collect();

    let out = tmp("stream_tree_out");
    let report = Merger::new()
        .inputs(&inputs)
        .compression(Compression::Zstd(3))
        .merge(&out)
        .unwrap();
    assert!(
        matches!(report.kind, MergeKind::Tree { entries: 6, .. }),
        "{report}"
    );

    let fo = FileReader::open(&out).unwrap();
    let t = TreeReader::open(&fo, "Events").unwrap();
    assert_eq!(
        t.branch("i").values().unwrap(),
        BranchValues::I32(vec![0, 10, 11, 20, 21, 22])
    );
    assert_eq!(
        t.branch("hits").values().unwrap(),
        BranchValues::VecF32(vec![
            vec![],
            vec![],
            vec![1.0],
            vec![],
            vec![1.0],
            vec![2.0, 2.0],
        ])
    );
    assert_eq!(
        t.branch("tag").values().unwrap(),
        BranchValues::Str(
            ["t00", "t10", "t11", "t20", "t21", "t22"]
                .map(String::from)
                .to_vec()
        )
    );
    assert_eq!(
        t.branch("vec").values().unwrap(),
        BranchValues::VecF64(vec![
            vec![0.0; 2],
            vec![0.0; 2],
            vec![1.0; 2],
            vec![0.0; 2],
            vec![1.0; 2],
            vec![2.0; 2],
        ])
    );
}

/// Small inputs are gathered into clusters of a useful size, not written
/// as one tiny cluster each.
#[test]
fn an_rntuple_merge_gathers_small_inputs_into_one_cluster() {
    let inputs: Vec<_> = (0..3)
        .map(|k| {
            let path = tmp(&format!("stream_rn_{k}"));
            FileWriter::create(&path)
                .add(NtupleObject::new(
                    "ntpl",
                    vec![
                        Field::i64("id", vec![k, k + 100]),
                        Field::string("s", vec![format!("a{k}"), format!("b{k}")]),
                    ],
                ))
                .compression(Compression::Stored)
                .write()
                .unwrap();
            path
        })
        .collect();

    let out = tmp("stream_rn_out");
    Merger::new()
        .inputs(&inputs)
        .compression(Compression::Zstd(1))
        .merge(&out)
        .unwrap();

    let fo = FileReader::open(&out).unwrap();
    let nt = NtupleReader::open(&fo, "ntpl").unwrap();
    assert_eq!(nt.footer().cluster_groups[0].num_clusters, 1);
    assert_eq!(
        nt.field("id").values().unwrap(),
        FieldValues::I64(vec![0, 100, 1, 101, 2, 102])
    );
    assert_eq!(
        nt.field("s").values().unwrap(),
        FieldValues::Str(
            ["a0", "b0", "a1", "b1", "a2", "b2"]
                .map(String::from)
                .to_vec()
        )
    );
}

#[test]
fn empty_inputs_merge_to_an_empty_tree() {
    for k in 0..2 {
        FileWriter::create(tmp(&format!("empty_tree_{k}")))
            .add(TreeObject::new("T", vec![Branch::f64("x", vec![])]))
            .compression(Compression::Stored)
            .write()
            .unwrap();
    }
    let out = tmp("empty_tree_out");
    let report = Merger::new()
        .inputs([tmp("empty_tree_0"), tmp("empty_tree_1")])
        .compression(Compression::Stored)
        .merge(&out)
        .unwrap();
    assert!(
        matches!(report.kind, MergeKind::Tree { entries: 0, .. }),
        "{report}"
    );
    let fo = FileReader::open(&out).unwrap();
    assert_eq!(TreeReader::open(&fo, "T").unwrap().entries(), 0);
}

#[test]
fn the_output_cannot_be_an_input() {
    let path = tmp("self_merge");
    FileWriter::create(&path)
        .add(TreeObject::new("T", vec![Branch::i32("x", vec![1])]))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let err = Merger::new()
        .inputs([&path])
        .compression(Compression::Stored)
        .merge(&path)
        .unwrap_err();
    assert!(err.to_string().contains("also an input"), "{err}");
    // The input is untouched.
    let f = FileReader::open(&path).unwrap();
    assert_eq!(TreeReader::open(&f, "T").unwrap().entries(), 1);
}

/// Every column type a `Batch` holds is merged: the merged dataset reads back
/// equal to the inputs read as a chain, with the same columns. These fixtures
/// hold what the low-level writers refuse (structs, nested vectors, optionals,
/// unsigned vectors, sets and maps, split objects in a tree).
#[test]
fn a_merge_keeps_every_column_a_batch_can_hold() {
    let fixture = |name: &str| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
    };
    let read_all = |d: &Dataset<'_>| {
        let columns: Vec<String> = d.columns().into_iter().map(str::to_string).collect();
        (columns.clone(), d.read(&columns).unwrap())
    };
    for (file, name) in [
        ("rntuple_coltypes_zstd.root", "ntpl"),
        ("rntuple_nested_zstd.root", "ntpl"),
        ("rntuple_stl_zstd.root", "ntpl"),
        ("rntuple_map_zstd.root", "ntpl"),
        ("rntuple_set_zstd.root", "ntpl"),
        ("rntuple_user_uncompressed.root", "ntpl"),
        ("rntuple_realprec_zstd.root", "ntpl"),
        ("tree_split.root", "T"),
        ("tree_nested.root", "T"),
        ("tree_clones.root", "T"),
        ("tree_vecvec.root", "T"),
        ("tree_arrays.root", "T"),
    ] {
        let input = fixture(file);
        let out = tmp(&format!("every_column_{file}"));
        Merger::new().inputs([&input, &input]).merge(&out).unwrap();

        let (a, b) = (
            FileReader::open(&input).unwrap(),
            FileReader::open(&input).unwrap(),
        );
        let chain = Dataset::chain([
            Dataset::open(&a, name).unwrap(),
            Dataset::open(&b, name).unwrap(),
        ])
        .unwrap();
        let merged_file = FileReader::open(&out).unwrap();
        let merged = Dataset::open(&merged_file, name).unwrap();
        assert_eq!(read_all(&merged), read_all(&chain), "{file}");
        std::fs::remove_file(out).unwrap();
    }
}

/// A `std::variant` field is the one thing a batch cannot hold: the merge
/// says so and writes nothing.
#[test]
fn a_merge_refuses_a_variant_field() {
    let input = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rntuple_variant_uncompressed.root");
    let out = tmp("variant_out");
    let err = Merger::new()
        .inputs([&input, &input])
        .merge(&out)
        .unwrap_err();
    assert!(err.to_string().contains("variant"), "{err}");
    assert!(!out.exists());
}

/// Inputs that do not hold the first input's columns, or hold one of them as
/// another type, are refused: nothing is merged across a changed schema.
#[test]
fn a_merge_refuses_inputs_whose_columns_differ() {
    let write = |tag: &str, format: Format, batch: Batch| {
        let path = tmp(tag);
        let mut w = DatasetWriter::create(&path, "events", format).unwrap();
        w.write(batch).unwrap();
        w.finish().unwrap();
        path
    };
    for format in [Format::Tree, Format::Ntuple] {
        let tag = |what: &str| format!("differ_{format:?}_{what}");
        let base = write(
            &tag("base"),
            format,
            Batch::new()
                .column("x", vec![1.0_f64, 2.0])
                .unwrap()
                .column("n", vec![1_i32, 2])
                .unwrap(),
        );
        let missing = write(
            &tag("missing"),
            format,
            Batch::new().column("x", vec![3.0_f64]).unwrap(),
        );
        let retyped = write(
            &tag("retyped"),
            format,
            Batch::new()
                .column("x", vec![3.0_f32])
                .unwrap()
                .column("n", vec![3_i32])
                .unwrap(),
        );
        let out = tmp(&tag("out"));
        let _ = std::fs::remove_file(&out);

        let err = Merger::new()
            .inputs([&base, &missing])
            .merge(&out)
            .unwrap_err();
        assert!(
            err.to_string().contains("\"n\""),
            "{format:?} missing: {err}"
        );

        let err = Merger::new()
            .inputs([&base, &retyped])
            .merge(&out)
            .unwrap_err();
        assert!(
            err.to_string().contains("\"x\""),
            "{format:?} retyped: {err}"
        );

        // A failed merge leaves no output, and nothing beside it.
        assert!(!out.exists());
        let leftovers = std::fs::read_dir(out.parent().unwrap())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            // Other tests merge into this directory at the same time.
            .filter(|name| name.contains("differ_") && name.ends_with(".partial"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
