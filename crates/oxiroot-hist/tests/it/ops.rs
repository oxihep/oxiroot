//! Item 3: histogram arithmetic — merge (add), scale, multiply, divide,
//! integral — with Sumw2 error propagation. Merge + scale are cross-checked in
//! official ROOT.

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Hist, Hist1D};
use oxiroot_io_core::FileReader;

#[test]
fn adding_two_records_sums_bins_errors_entries_and_statistics() {
    // Two weighted histograms, as if from two parallel jobs.
    let mut a = Hist::reg(4, 0.0, 4.0).weight().named("h").titled("merged");
    a.fill_weight(0.5, 2.0);
    a.fill_weight(1.5, 1.0);
    let mut b = Hist::reg(4, 0.0, 4.0).weight().named("h").titled("merged");
    b.fill_weight(0.5, 3.0);
    b.fill_weight(2.5, 1.0);
    // What one job filling everything would have written.
    let mut both = a.clone();
    both.add(&b).unwrap();

    let mut merged = a.to_root().unwrap();
    merged
        .add(&b.to_root().unwrap(), 1.0)
        .expect("same binning");
    // bin 1: 2 + 3 = 5, with Σw² = 4 + 9 = 13.
    assert_eq!(merged.contents()[1], 5.0);
    assert_eq!(merged.sumw2().expect("errors are tracked")[1], 13.0);
    assert_eq!(merged.entries(), 4.0, "entries are summed");
    assert_eq!(merged, both.to_root().unwrap());

    let out = PathBuf::from("/tmp/rootrs_merged.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&merged)
        .compression(oxiroot_io_core::Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.get::<Hist1D>("h").unwrap(), merged, "round-trips");
}

#[test]
fn add_rejects_mismatched_binning() {
    // Different bin counts.
    let mut a = Hist::reg(4, 0.0, 4.0)
        .double()
        .to_root()
        .unwrap()
        .named("a");
    let b = Hist::reg(5, 0.0, 5.0)
        .double()
        .to_root()
        .unwrap()
        .named("b");
    assert!(a.add(&b, 1.0).is_err(), "different bin counts -> error");

    // Same bin count but different range — must also be rejected (cell-count
    // equality alone would have missed this).
    let mut c = Hist::reg(4, 0.0, 4.0)
        .double()
        .to_root()
        .unwrap()
        .named("c");
    let d = Hist::reg(4, 0.0, 8.0)
        .double()
        .to_root()
        .unwrap()
        .named("d");
    assert!(c.add(&d, 1.0).is_err(), "different edges -> error");
    assert_eq!(c.contents()[1], 0.0, "rejected add makes no change");
}

/// Regression: `Profile1D::add` scaled Σw·y² by `c²` while every other weighted
/// sum — and `Hist2D::add`'s identical `tsumwy2` line — scales by `c`. Σw·y² is
/// linear in the weight, so `c` is correct. Invisible at `c == 1` (the merge
/// path), which is why no existing test caught it.
#[test]
fn profile_add_scales_weighted_sums_linearly() {
    use oxiroot_hist::Hist;

    let build = || {
        let mut p = Hist::reg(4, 0.0, 4.0).mean();
        p.fill_sample(0.5, 2.0);
        p.fill_sample(1.5, 3.0);

        p.to_root().unwrap()
    };
    let (mut dst, src) = (build(), build());

    let (w0, w20, wx0, wx20, wy0, wy20) = (
        dst.tsumw(),
        dst.tsumw2(),
        dst.tsumwx(),
        dst.tsumwx2(),
        dst.tsumwy(),
        dst.tsumwy2(),
    );
    let c = 3.0;
    dst.add(&src, c).unwrap();

    // Σw² is quadratic in the scale factor; every Σw·(…) sum is linear.
    assert!((dst.tsumw() - (w0 + c * src.tsumw())).abs() < 1e-12);
    assert!((dst.tsumw2() - (w20 + c * c * src.tsumw2())).abs() < 1e-12);
    assert!((dst.tsumwx() - (wx0 + c * src.tsumwx())).abs() < 1e-12);
    assert!((dst.tsumwx2() - (wx20 + c * src.tsumwx2())).abs() < 1e-12);
    assert!((dst.tsumwy() - (wy0 + c * src.tsumwy())).abs() < 1e-12);
    assert!(
        (dst.tsumwy2() - (wy20 + c * src.tsumwy2())).abs() < 1e-12,
        "tsumwy2 scaled by c^2 instead of c: got {}, want {}",
        dst.tsumwy2(),
        wy20 + c * src.tsumwy2()
    );
}

/// The same law when both sides already carry `fBinSumw2` — the shape of a
/// weighted profile read back from a ROOT file.
#[test]
fn profile_add_scales_tracked_squared_weights_quadratically() {
    use oxiroot_hist::Hist;

    let mut dst = Hist::reg(4, 0.0, 4.0).mean();
    dst.fill_sample(0.5, 1.0);
    let mut dst = dst.to_root().unwrap();
    let mut src = Hist::reg(4, 0.0, 4.0).mean();
    src.fill_sample(0.5, 2.0);
    let mut src = src.to_root().unwrap();
    // As if both were filled with weights and read from disk.
    *dst.optional_arrays_mut().1 = vec![0.0, 4.0, 0.0, 0.0, 0.0, 0.0];
    *src.optional_arrays_mut().1 = vec![0.0, 9.0, 0.0, 0.0, 0.0, 0.0];

    let c = 2.0;
    dst.add(&src, c).unwrap();

    // 4 + c²·9 = 40, not 4 + c·9 = 22.
    assert!(
        (dst.bin_sumw2()[1] - 40.0).abs() < 1e-12,
        "got {}",
        dst.bin_sumw2()[1]
    );
}

/// The types ROOT's `hadd` merges beyond the histogram and profile families:
/// they merge the same way, and refuse an incompatible partner.
mod merges {

    use oxiroot_hist::{Graph, GraphFunction, Mergeable, PolyHist, SparseHist};
    use oxiroot_io_core::Error;

    fn poly() -> PolyHist {
        let mut p = PolyHist::new(0.0, 2.0, 0.0, 2.0);
        p.add_bin_rect(0.0, 0.0, 1.0, 1.0);
        p.add_bin_rect(1.0, 1.0, 2.0, 2.0);
        p
    }

    #[test]
    fn a_poly_histogram_sums_its_bins() {
        let mut a = poly();
        a.fill_weight(0.5, 0.5, 2.0);
        let mut b = poly();
        b.fill_weight(0.5, 0.5, 1.0);
        b.fill_weight(1.5, 1.5, 4.0);
        a.merge(&b).unwrap();
        assert_eq!(
            a.bins().iter().map(|x| x.content).collect::<Vec<_>>(),
            vec![3.0, 4.0]
        );
        assert_eq!(a.entries(), 3.0);

        // Different polygons change nothing.
        let mut other = PolyHist::new(0.0, 2.0, 0.0, 2.0);
        other.add_bin_rect(0.0, 0.0, 2.0, 2.0);
        assert!(matches!(
            a.merge(&other),
            Err(Error::BinningMismatch { .. })
        ));
        assert_eq!(a.bins()[0].content, 3.0);
    }

    #[test]
    fn a_sparse_histogram_sums_shared_bins_and_keeps_the_rest() {
        let mut a = SparseHist::new(&[(4, 0.0, 4.0)]).named("sp");
        a.fill(&[0.5]).unwrap();
        let mut b = SparseHist::new(&[(4, 0.0, 4.0)]).named("sp");
        b.fill(&[0.5]).unwrap();
        b.fill(&[2.5]).unwrap();
        a.merge(&b).unwrap();
        assert_eq!(
            a.bins().len(),
            2,
            "the shared bin is summed, the new one kept"
        );
        assert_eq!(a.bins().iter().map(|x| x.content).sum::<f64>(), 3.0);
        assert_eq!(a.entries(), 3.0);

        let other = SparseHist::new(&[(2, 0.0, 2.0)]).named("sp");
        assert!(matches!(
            a.merge(&other),
            Err(Error::BinningMismatch { .. })
        ));
    }

    #[test]
    fn graphs_append_their_points_and_errors() {
        let mut a = Graph::new(vec![1.0], vec![2.0])
            .and_then(|g| {
                g.with_errors(oxiroot_hist::GraphErrors::Symmetric {
                    ex: vec![0.1],
                    ey: vec![0.2],
                })
            })
            .unwrap()
            .named("g")
            .with_function(GraphFunction::new("f", "[0]*x", vec![1.0], 0.0, 1.0));
        let b = Graph::new(vec![3.0, 4.0], vec![5.0, 6.0])
            .and_then(|g| {
                g.with_errors(oxiroot_hist::GraphErrors::Symmetric {
                    ex: vec![0.3, 0.4],
                    ey: vec![0.5, 0.6],
                })
            })
            .unwrap();
        a.merge(&b).unwrap();
        assert_eq!(a.x(), vec![1.0, 3.0, 4.0]);
        assert_eq!(a.y(), vec![2.0, 5.0, 6.0]);
        assert_eq!(a.functions.len(), 1, "the attached function is kept");

        // Different error bars change nothing.
        let plain = Graph::new(vec![9.0], vec![9.0]).unwrap();
        assert!(matches!(a.merge(&plain), Err(Error::InvalidInput(_))));
        assert_eq!(a.x().len(), 3);
    }
}
