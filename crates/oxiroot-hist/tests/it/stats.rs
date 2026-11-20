//! Tier-1 statistics accessors (std_dev/maximum/find_bin/effective_entries/reset)
//! against hand-computed values.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{Hist, Hist1D};

fn filled_th1() -> Hist1D {
    // 4 bins over [0,4); fills at 0.5,0.5,1.5,2.5,2.5,2.5 → contents [2,1,3,0].
    let mut h = Hist::reg(4, 0.0, 4.0).double().to_root().named("h");
    for x in [0.5, 0.5, 1.5, 2.5, 2.5, 2.5] {
        h.fill(x);
    }
    h
}

#[test]
fn th1_stats() {
    let h = filled_th1();
    assert_eq!(h.values(), &[2.0, 1.0, 3.0, 0.0]);
    // mean = 10/6, var = 21.5/6 - (10/6)^2.
    assert!((h.mean() - 10.0 / 6.0).abs() < 1e-12);
    let var = 21.5 / 6.0 - (10.0 / 6.0_f64).powi(2);
    assert!((h.std_dev() - var.sqrt()).abs() < 1e-12);

    assert_eq!(h.maximum(), 3.0);
    assert_eq!(h.maximum_bin(), 3);
    assert_eq!(h.minimum(), 0.0);
    assert_eq!(h.minimum_bin(), 4);

    assert_eq!(h.find_bin(2.5), 3);
    assert_eq!(h.find_bin(-1.0), 0, "underflow");
    assert_eq!(h.find_bin(5.0), 5, "overflow = nbins+1");
    assert_eq!(h.bin_center(3), 2.5);
    assert_eq!(h.bin_width(3), 1.0);
    assert_eq!(h.bin_low_edge(3), 2.0);

    // Unweighted: Σw² = 6, so effective entries = 6²/6 = 6.
    assert!((h.effective_entries() - 6.0).abs() < 1e-12);
}

#[test]
fn th1_reset_clears_everything() {
    let mut h = filled_th1();
    h.track_sumw2();
    h.reset();
    assert_eq!(h.values(), &[0.0, 0.0, 0.0, 0.0]);
    assert_eq!(h.entries(), 0.0);
    assert_eq!(h.mean(), 0.0);
    assert_eq!(h.std_dev(), 0.0);
    assert!(
        h.sumw2()
            .expect("error tracking is on")
            .iter()
            .all(|&v| v == 0.0),
        "sumw2 zeroed, length kept"
    );
    assert_eq!(h.xaxis().nbins(), 4, "binning preserved");
}

#[test]
fn th2_extrema_and_find_bin() {
    let mut h = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .to_root()
        .named("h");
    h.fill(0.5, 0.5); // cell (1,1)
    h.fill(1.5, 1.5);
    h.fill(1.5, 1.5); // cell (2,2) = 2
    assert_eq!(h.maximum(), 2.0);
    // (2,2) global cell = 2 + (2+2)*2 = 10.
    assert_eq!(h.maximum_bin(), 10);
    assert_eq!(h.minimum(), 0.0);
    assert_eq!(h.find_bin(1.5, 1.5), 10);
}

#[test]
fn tprofile_mean_std_dev() {
    let mut p = Hist::reg(4, 0.0, 4.0).mean().to_root().named("p");
    for (x, y) in [(0.5, 10.0), (0.5, 20.0), (1.5, 5.0), (2.5, 30.0)] {
        p.fill(x, y);
    }
    // x values 0.5,0.5,1.5,2.5 → mean 1.25.
    assert!((p.mean() - 1.25).abs() < 1e-12);
    let var = (0.25 + 0.25 + 2.25 + 6.25) / 4.0 - 1.25_f64.powi(2);
    assert!((p.std_dev() - var.sqrt()).abs() < 1e-12);
}

#[test]
fn profiles_share_bin_value_error_and_effective_entries() {
    // The same two values (1 and 3) in one cell of a 1-D, 2-D and 3-D profile:
    // mean 2, spread² = (1+9)/2 − 4 = 1, two entries → error √(1/2).
    let mut p1 = Hist::reg(2, 0.0, 2.0).mean().to_root().named("p1");
    let mut p2 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .mean()
        .to_root()
        .named("p2");
    let mut p3 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .mean()
        .to_root()
        .named("p3");
    for v in [1.0, 3.0] {
        p1.fill(0.5, v);
        p2.fill(0.5, 0.5, v);
        p3.fill(0.5, 0.5, 0.5, v);
    }
    // Global cells of bin (1), (1,1) and (1,1,1), flow included.
    let (c1, c2, c3) = (1, 1 + 4, 1 + 4 * (1 + 4));
    let want_err = 0.5_f64.sqrt();
    for (value, error, neff) in [
        (p1.bin_value(c1), p1.bin_error(c1), p1.effective_entries(c1)),
        (p2.bin_value(c2), p2.bin_error(c2), p2.effective_entries(c2)),
        (p3.bin_value(c3), p3.bin_error(c3), p3.effective_entries(c3)),
    ] {
        assert_eq!(value, 2.0);
        assert!((error - want_err).abs() < 1e-12, "{error}");
        assert_eq!(neff, 2.0);
    }
    // An empty cell has value, error and effective entries 0.
    assert_eq!(p3.bin_value(0), 0.0);
    assert_eq!(p3.bin_error(0), 0.0);
    assert_eq!(p3.effective_entries(0), 0.0);

    // Weights 1 and 3: neff = 4²/(1+9) = 1.6.
    p3.fill_weight(1.5, 1.5, 1.5, 2.0, 1.0);
    p3.fill_weight(1.5, 1.5, 1.5, 2.0, 3.0);
    let c = 2 + 4 * (2 + 4 * 2);
    assert!((p3.effective_entries(c) - 1.6).abs() < 1e-12);
    assert_eq!(p3.bin_error(c), 0.0, "no spread");
}
