//! Objects held by pointer inside other objects, in a ROOT-written file where
//! the second of each names its classes by a back-reference to the first
//! (`fixtures/embedded_backrefs.root`, from `scripts/gen_embedded_backrefs.cpp`).
//! A class is spelled out once per key and named by its position afterwards,
//! so every one of these is read through the key's class-tag map.

use std::path::PathBuf;

use oxiroot_hist::{BinContentType, Efficiency, Graph, Hist1D, Profile1D, SparseHist};
use oxiroot_io_core::{Error, FileReader, ObjList};

fn fixture() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/embedded_backrefs.root"),
    )
    .expect("open fixture")
}

fn members() -> ObjList {
    fixture().get::<ObjList>("members").unwrap()
}

fn labels(h: &Hist1D) -> Vec<&str> {
    (1..=3).filter_map(|bin| h.xaxis().bin_label(bin)).collect()
}

/// The label list of the second histogram, and of the profile after it, name
/// `THashList` and `TObjString` by position.
#[test]
fn labels_of_every_member_are_read() {
    let l = members();
    let hists = l.items::<Hist1D>().unwrap();
    assert_eq!(hists.len(), 2);
    for (h, scale) in hists.iter().zip([1.0, 10.0]) {
        assert_eq!(labels(h), ["low", "mid", "high"], "{}", h.name());
        assert_eq!(h.contents()[1..4], [scale, 2.0 * scale, 3.0 * scale]);
    }
    let profiles = l.items::<Profile1D>().unwrap();
    assert_eq!(profiles[0].xaxis().bin_label(2), Some("only"));
}

/// An efficiency's two histograms are of the class its header names: here
/// `TH1D` twice over, then `TH1F`, the latter by position in the third member.
#[test]
fn an_efficiency_reads_the_histogram_class_it_names() {
    let l = members();
    let effs = l.items::<Efficiency>().unwrap();
    let per_bin: Vec<_> = effs.iter().map(|e| e.efficiency(2)).collect();
    assert_eq!(per_bin, [Some(0.5), Some(0.2), Some(0.3)]);
    assert_eq!(effs[1].total().bin_content_type(), BinContentType::F64);
    assert_eq!(effs[2].total().bin_content_type(), BinContentType::F32);

    // The same under a key of its own.
    let e = fixture().get::<Efficiency>("eff_f").unwrap();
    assert_eq!(e.total().bin_content_type(), BinContentType::F32);
    assert_eq!(e.total().contents()[1..5], [10.0, 20.0, 30.0, 40.0]);
    assert_eq!(e.efficiency(4), Some(0.3));
}

/// A two-dimensional efficiency is refused by name, not read as if its
/// histograms had one axis.
#[test]
fn an_efficiency_over_another_histogram_class_is_refused() {
    let err = fixture().get::<Efficiency>("eff_2d").unwrap_err();
    assert!(
        matches!(&err, Error::Unsupported(what) if what.contains("TH2D")),
        "{err}"
    );
}

/// The second sparse histogram names `TAxis`, its chunk class and `TArrayD`
/// by position.
#[test]
fn sparse_histograms_in_a_collection_are_read() {
    let sparse = members().items::<SparseHist>().unwrap();
    assert_eq!(sparse.len(), 2);
    for (s, weight) in sparse.iter().zip([1.0, 3.0]) {
        assert_eq!(s.ndim(), 2);
        let mut contents: Vec<f64> = s.bins().iter().map(|b| b.content).collect();
        contents.sort_by(f64::total_cmp);
        assert_eq!(contents, [weight, 2.0 * weight]);
    }
}

/// A graph's frame is the histogram its header names, in both graphs.
#[test]
fn the_frame_of_every_graph_is_read() {
    let graphs = members().items::<Graph>().unwrap();
    assert_eq!(graphs.len(), 2);
    for g in &graphs {
        let frame = g.histogram().expect("frame");
        assert_eq!(frame.bin_content_type(), BinContentType::F32);
        assert_eq!(frame.xaxis().nbins(), 100);
    }
}
