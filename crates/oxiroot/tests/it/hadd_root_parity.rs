//! A `Merger` against ROOT's own `hadd`.
//!
//! `fixtures/hadd_a.root` and `fixtures/hadd_b.root` hold one object of every
//! class oxiroot's merge handles, written by ROOT
//! (`scripts/gen_hadd_inputs.cpp`). `fixtures/hadd_merged.root` is what ROOT's
//! `hadd` made of them. oxiroot must reach the same answer for every object
//! ROOT merges, and copy the first input's object for the rest — which is what
//! ROOT does too, except that it writes one key per input, which oxiroot cannot
//! do (it rejects two objects of the same name in one directory).

use std::path::PathBuf;

use oxiroot::file::Parameter;
use oxiroot::hadd::{MergeKind, Merger};
use oxiroot::hist::{
    Efficiency, Graph, Graph2D, GraphStack, Hist1D, HistStack, PolyHist, Profile1D, SparseHist,
};
use oxiroot::{Compression, FileReader};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}"))
}

/// oxiroot's merge of the two ROOT inputs, and ROOT's `hadd` of the same two.
fn merged(tag: &str) -> (FileReader, FileReader) {
    let out = std::env::temp_dir().join(format!("oxiroot_hadd_parity_{tag}.root"));
    let inputs = [fixture("hadd_a.root"), fixture("hadd_b.root")];
    let report = Merger::new()
        .inputs(&inputs)
        .compression(Compression::Stored)
        .merge(&out)
        .expect("merge");
    let MergeKind::Histograms(outcome) = &report.kind else {
        panic!("not a histogram merge: {report}");
    };
    for key in ["h", "p", "eff", "poly", "sp", "g", "ge", "ga", "st", "par"] {
        assert!(
            outcome.summed.iter().any(|k| k == key),
            "{key} was not merged: {report}"
        );
    }
    for key in ["fn", "g2", "mg", "s"] {
        assert!(
            outcome.copied.iter().any(|k| k == key),
            "{key} was not copied: {report}"
        );
    }
    assert!(outcome.skipped.is_empty(), "{report}");
    (
        FileReader::open(&out).expect("open oxiroot's merge"),
        FileReader::open(fixture("hadd_merged.root")).expect("open ROOT's merge"),
    )
}

#[test]
fn histograms_and_profiles_match_root() {
    let (ours, root) = merged("hists");
    let h = ours.get::<Hist1D>("h").unwrap();
    assert_eq!(h.contents(), root.get::<Hist1D>("h").unwrap().contents());
    assert_eq!(h.entries(), 2.0);

    let p = ours.get::<Profile1D>("p").unwrap();
    let root_p = root.get::<Profile1D>("p").unwrap();
    assert_eq!(p.sums(), root_p.sums());
    assert_eq!(p.bin_entries(), root_p.bin_entries());
}

#[test]
fn an_efficiency_sums_its_histograms_like_root() {
    let (ours, root) = merged("eff");
    let e = ours.get::<Efficiency>("eff").unwrap();
    let root_e = root.get::<Efficiency>("eff").unwrap();
    assert_eq!(e.passed().contents(), root_e.passed().contents());
    assert_eq!(e.total().contents(), root_e.total().contents());
    // One passed of two in the first file, two of three in the second.
    assert_eq!(e.passed().integral(), 3.0);
    assert_eq!(e.total().integral(), 5.0);
}

#[test]
fn a_poly_and_a_sparse_histogram_sum_their_bins_like_root() {
    let (ours, root) = merged("bins");
    let poly = ours.get::<PolyHist>("poly").unwrap();
    let root_poly = root.get::<PolyHist>("poly").unwrap();
    let contents: Vec<f64> = poly.bins().iter().map(|b| b.content).collect();
    let root_contents: Vec<f64> = root_poly.bins().iter().map(|b| b.content).collect();
    assert_eq!(contents, root_contents);
    assert_eq!(contents, vec![3.0, 4.0]);
    assert_eq!(poly.entries(), root_poly.entries());

    let sp = ours.get::<SparseHist>("sp").unwrap();
    let root_sp = root.get::<SparseHist>("sp").unwrap();
    let mut ours_bins: Vec<(Vec<i32>, f64)> = sp
        .bins()
        .iter()
        .map(|b| (b.coords.clone(), b.content))
        .collect();
    let mut root_bins: Vec<(Vec<i32>, f64)> = root_sp
        .bins()
        .iter()
        .map(|b| (b.coords.clone(), b.content))
        .collect();
    ours_bins.sort_by(|a, b| a.0.cmp(&b.0));
    root_bins.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(ours_bins, root_bins);
    assert_eq!(sp.bins().iter().map(|b| b.content).sum::<f64>(), 9.0);
}

#[test]
fn graphs_append_their_points_like_root() {
    let (ours, root) = merged("graphs");
    for key in ["g", "ge", "ga"] {
        let g = ours.get::<Graph>(key).unwrap();
        let root_g = root.get::<Graph>(key).unwrap();
        assert_eq!((key, &g.x(), &g.y()), (key, &root_g.x(), &root_g.y()));
        assert_eq!(g.errors(), root_g.errors(), "{key}");
        // Two points from the first file, three from the second.
        assert_eq!(g.x().len(), 5, "{key}");
    }
}

#[test]
fn a_stack_merges_its_histograms_and_a_parameter_sums_like_root() {
    let (ours, root) = merged("stack");
    let st = ours.get::<HistStack>("st").unwrap();
    let root_st = root.get::<HistStack>("st").unwrap();
    assert_eq!(st.hists().len(), 1);
    assert_eq!(st.hists()[0].contents(), root_st.hists()[0].contents());
    assert_eq!(st.hists()[0].integral(), 3.0);

    let par = ours.get::<Parameter>("par").unwrap();
    assert_eq!(par.value(), root.get::<Parameter>("par").unwrap().value());
}

#[test]
fn what_root_does_not_merge_is_the_first_input() {
    let (ours, _) = merged("copied");
    let first = FileReader::open(fixture("hadd_a.root")).unwrap();

    // ROOT writes one key per input for these; oxiroot keeps the first.
    let g2 = ours.get::<Graph2D>("g2").unwrap();
    assert_eq!(g2.x(), first.get::<Graph2D>("g2").unwrap().x());
    let mg = ours.get::<GraphStack>("mg").unwrap();
    assert_eq!(mg.graphs().len(), 1);
    assert_eq!(mg.graphs()[0].x(), [0.0, 1.0]);
}
