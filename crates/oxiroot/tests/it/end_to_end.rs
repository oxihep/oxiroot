//! High-level end-to-end smoke tests over the whole `oxiroot` facade: build every
//! major object type through the public `prelude`, write it, read it back, and
//! exercise sampling and the `hadd` merge — one flow per subsystem, so a
//! regression anywhere (writer, reader, or a feature added on top) surfaces here.

use crate::support::{NtupleObject, TreeObject};
use oxiroot::file::ObjString;
use oxiroot::hadd::Merger;
use oxiroot::hist::Random;
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::{Field, FieldValues};
use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;
use oxiroot::tree::raw::{Branch, BranchValues};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxiroot_e2e_{name}.root"))
}

#[test]
fn writes_and_reads_a_mixed_object_file() {
    // A regular-binned and an irregular-binned histogram, a 2-D histogram, a
    // profile, a function, a graph, and a string — all in one file.
    let mut h = Hist::reg(20, 0.0, 100.0)
        .double()
        .named("pt")
        .titled("p_{T}");
    for x in [5.0, 15.0, 15.0, 95.0] {
        h.fill(x);
    }

    let mut hv = Hist::var(&[0.0, 1.0, 2.0, 5.0, 10.0, 100.0])
        .double()
        .named("hv");
    for x in [0.5, 3.0, 50.0] {
        hv.fill(x);
    }

    let mut h2 = Hist::reg(4, 0.0, 4.0).reg(3, 0.0, 3.0).double().named("h2");
    h2.fill(1.5, 1.5);

    let mut prof = Hist::reg(10, 0.0, 10.0).mean().named("prof");
    prof.fill_sample(1.0, 5.0);
    prof.fill_sample(1.0, 7.0);

    let f = Func1D::new("f", "[0]*sin([1]*x)", 0.0..=6.3)
        .unwrap()
        .with_params(&[2.0, 1.5])
        .unwrap();

    let g = Graph::new(vec![1.0, 2.0, 3.0], vec![10.0, 20.0, 30.0])
        .unwrap()
        .named("g");

    FileWriter::create(tmp("mixed"))
        .add(&h.to_root().unwrap())
        .add(&hv.to_root().unwrap())
        .add(&h2.to_root().unwrap())
        .add(&prof.to_root().unwrap())
        .add(&f)
        .add(&g)
        .add(&ObjString::new("skim v3").named("meta"))
        .write()
        .unwrap();

    // Every object reads back with its identity intact.
    let file = FileReader::open(tmp("mixed")).unwrap();
    assert_eq!(
        file.get::<Hist1D>("pt")
            .unwrap()
            .values()
            .iter()
            .sum::<f64>(),
        4.0
    );
    assert_eq!(
        file.get::<Hist1D>("hv").unwrap().edges(),
        vec![0.0, 1.0, 2.0, 5.0, 10.0, 100.0]
    );
    assert_eq!(
        file.get::<Hist2D>("h2")
            .unwrap()
            .values()
            .iter()
            .flatten()
            .sum::<f64>(),
        1.0
    );
    assert!(file.get::<Profile1D>("prof").is_ok());
    let f_back = file.get::<Func1D>("f").unwrap();
    assert!((f_back.eval(1.0) - f.eval(1.0)).abs() < 1e-9);
    assert_eq!(file.get::<Graph>("g").unwrap().len(), 3);
    assert!(file.get::<ObjString>("meta").is_ok());
}

#[test]
fn tree_round_trips_through_the_facade() {
    FileWriter::create(tmp("tree"))
        .add(TreeObject::new(
            "Events",
            vec![
                Branch::i32("i", vec![1, 2, 3]),
                Branch::f64("x", vec![1.5, 2.5, 3.5]),
                Branch::string("s", vec!["a".into(), "b".into(), "c".into()]),
                Branch::jagged_f64("v", vec![vec![1.0], vec![], vec![2.0, 3.0]]),
            ],
        ))
        .write()
        .unwrap();

    let f = FileReader::open(tmp("tree")).unwrap();
    let t = TreeReader::open(&f, "Events").unwrap();
    assert_eq!(t.entries(), 3);
    assert_eq!(
        t.branch("i").values().unwrap(),
        BranchValues::I32(vec![1, 2, 3])
    );
    assert_eq!(
        t.branch("s").values().unwrap(),
        BranchValues::Str(vec!["a".into(), "b".into(), "c".into()])
    );
    assert_eq!(
        t.branch("v").values().unwrap(),
        BranchValues::VecF64(vec![vec![1.0], vec![], vec![2.0, 3.0]])
    );
}

#[test]
fn rntuple_round_trips_through_the_facade() {
    FileWriter::create(tmp("rn"))
        .add(NtupleObject::new(
            "ntpl",
            vec![
                Field::f64("mass", vec![91.2, 125.0]),
                Field::vector_i32("hits", vec![vec![1, 2], vec![3]]),
                Field::string("tag", vec!["z".into(), "h".into()]),
            ],
        ))
        .write()
        .unwrap();

    let f = FileReader::open(tmp("rn")).unwrap();
    let nt = NtupleReader::open(&f, "ntpl").unwrap();
    assert_eq!(nt.entries(), 2);
    assert_eq!(
        nt.field("mass").values().unwrap(),
        FieldValues::F64(vec![91.2, 125.0])
    );
    assert_eq!(
        nt.field("hits").values().unwrap(),
        FieldValues::VecI32(vec![vec![1, 2], vec![3]])
    );
}

#[test]
fn sampling_a_histogram_reproduces_its_shape() {
    // A source distribution peaked at 3.0.
    let mut src = Hist::reg(50, 0.0, 10.0).double().named("src");
    for _ in 0..200 {
        src.fill(3.0);
    }
    for _ in 0..100 {
        src.fill(2.0);
        src.fill(4.0);
    }

    // Draw from it into a new histogram — reproducibly, with the seedable Random.
    let mut rng = Random::seed(7);
    let mut drawn = Hist::reg(50, 0.0, 10.0).double().named("drawn");
    drawn.fill_random(&src, 20_000, &mut rng);

    // All draws land in range, and the sampled mean tracks the source's.
    assert_eq!(drawn.integral(false), 20_000.0);
    assert!(
        (drawn.mean() - src.mean()).abs() < 0.1,
        "drawn mean {}",
        drawn.mean()
    );

    // Smoothing runs and keeps the bin count.
    let mut smoothed = src.clone();
    smoothed.smooth(1);
    assert_eq!(smoothed.values(false).len(), src.values(false).len());
}

#[test]
fn hadd_merges_histogram_files_end_to_end() {
    for (name, xs) in [("m1", [1.0, 2.0]), ("m2", [2.0, 3.0])] {
        let mut h = Hist::reg(5, 0.0, 5.0).double().named("h");
        for x in xs {
            h.fill(x);
        }
        FileWriter::create(tmp(name))
            .add(&h.to_root().unwrap())
            .write()
            .unwrap();
    }

    let report = Merger::new()
        .inputs([tmp("m1"), tmp("m2")])
        .merge(tmp("merged"))
        .unwrap();
    assert!(
        matches!(&report.kind, oxiroot::hadd::MergeKind::Histograms(o) if o.summed == ["h"]),
        "{report}"
    );

    let file = FileReader::open(tmp("merged")).unwrap();
    assert_eq!(
        file.get::<Hist1D>("h")
            .unwrap()
            .values()
            .iter()
            .sum::<f64>(),
        4.0
    );
}
