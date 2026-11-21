//! Adding and merging the 2-D and 3-D profiles (summing them across files is
//! tested with the file merger, in the `oxiroot` crate).
//!
//! `add` and `Mergeable` used to exist only for `Hist1D`/`Hist2D`/`Hist3D`/`Profile1D`, so a
//! file merge copied `Profile2D`/`Profile3D` from the first input where ROOT's
//! `hadd` sums them. The defining property checked here is the one `Mergeable`
//! documents: merging two profiles must give exactly the profile you would get by
//! filling one with all of their data. Every value below is a small dyadic
//! rational, so that equality is exact whatever order the sums happen in.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{Hist, Mergeable, Profile1D, Profile2D, Profile3D};
use oxiroot_io_core::Error;

// --------------------------------------------------------------------- data

/// A 2-D profile fill: `(x, y, z, weight)`.
type Point2 = (f64, f64, f64, f64);
/// A 3-D profile fill: `(x, y, z, t, weight)`.
type Point3 = (f64, f64, f64, f64, f64);

/// Four unit-weight 2-D fills, so this profile never tracks Σw² itself. Repeats
/// a cell and includes an x-underflow point, which fills a flow cell but no
/// moment sum.
const SET_A_2D: [Point2; 4] = [
    (0.5, 0.5, 1.0, 1.0),
    (1.5, 0.5, 2.0, 1.0),
    (0.5, 0.5, 3.0, 1.0),
    (-1.0, 0.5, 4.0, 1.0),
];
/// Mixes unit and non-unit weights, starting with a unit fill, so the Σw²
/// tracking its first weighted fill turns on must be seeded.
const SET_B_2D: [Point2; 4] = [
    (0.5, 0.5, 5.0, 1.0),
    (1.5, 1.5, 2.0, 2.0),
    (2.5, 0.5, 1.0, 0.5),
    (0.5, 0.5, 1.0, 2.0),
];

/// Three 3-D fills.
const SET_A_3D: [Point3; 3] = [
    (0.5, 0.5, 0.5, 1.0, 1.0),
    (1.5, 0.5, 1.5, 2.0, 1.0),
    (0.5, 0.5, 0.5, 3.0, 1.0),
];
const SET_B_3D: [Point3; 4] = [
    (0.5, 0.5, 0.5, 4.0, 1.0),
    (1.5, 1.5, 0.5, 2.0, 2.0),
    (0.5, 1.5, 1.5, 1.0, 0.5),
    (5.0, 0.5, 0.5, 1.0, 2.0), // x overflow
];

/// A 2-D profile filled with `sets`, built and filled as an oxihist histogram.
fn profile_2d(sets: &[&[Point2]]) -> Profile2D {
    let mut p = Hist::reg(3, 0.0, 3.0)
        .reg(2, 0.0, 2.0)
        .weighted_mean()
        .named("p2");
    for &(x, y, z, w) in sets.iter().copied().flatten() {
        p.fill_sample_weight(x, y, z, w);
    }
    p.to_root().unwrap()
}

/// A 3-D profile filled with `sets`.
fn profile_3d(sets: &[&[Point3]]) -> Profile3D {
    let mut p = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .weighted_mean()
        .named("p3");
    for &(x, y, z, t, w) in sets.iter().copied().flatten() {
        p.fill_sample_weight(x, y, z, t, w);
    }
    p.to_root().unwrap()
}

// ------------------------------------------------------ add is a combined fill

#[test]
fn adding_2d_profiles_equals_one_combined_fill() {
    let mut merged = profile_2d(&[&SET_A_2D]);
    merged.add(&profile_2d(&[&SET_B_2D]), 1.0).unwrap();
    let want = profile_2d(&[&SET_A_2D, &SET_B_2D]);
    assert_eq!(merged, want);
    // The weighted fills are reflected in the error bookkeeping, not dropped.
    assert!(!merged.bin_sumw2().is_empty());
}

#[test]
fn adding_3d_profiles_equals_one_combined_fill() {
    let mut merged = profile_3d(&[&SET_A_3D]);
    merged.add(&profile_3d(&[&SET_B_3D]), 1.0).unwrap();
    assert_eq!(merged, profile_3d(&[&SET_A_3D, &SET_B_3D]));
    assert!(!merged.bin_sumw2().is_empty());
}

#[test]
fn addition_order_does_not_matter() {
    let mut ab = profile_2d(&[&SET_A_2D]);
    ab.add(&profile_2d(&[&SET_B_2D]), 1.0).unwrap();
    let mut ba = profile_2d(&[&SET_B_2D]);
    ba.add(&profile_2d(&[&SET_A_2D]), 1.0).unwrap();
    assert_eq!(ab, ba);
}

#[test]
fn scaled_2d_add_is_linear_except_squared_weights() {
    let (mut dst, src) = (profile_2d(&[&SET_A_2D]), profile_2d(&[&SET_B_2D]));
    let before = dst.clone();
    let c = 3.0;
    dst.add(&src, c).unwrap();

    for (name, got, a, b) in [
        ("tsumw", dst.tsumw(), before.tsumw(), src.tsumw()),
        ("tsumwx2", dst.tsumwx2(), before.tsumwx2(), src.tsumwx2()),
        ("tsumwxy", dst.tsumwxy(), before.tsumwxy(), src.tsumwxy()),
        ("tsumwz", dst.tsumwz(), before.tsumwz(), src.tsumwz()),
        ("tsumwz2", dst.tsumwz2(), before.tsumwz2(), src.tsumwz2()),
    ] {
        assert_eq!(got, a + c * b, "{name} must scale by c");
    }
    assert_eq!(dst.tsumw2(), before.tsumw2() + c * c * src.tsumw2());

    // `before` had only unit weights, so its seeded Σw² equals its Σw.
    let cell = 1 + 5; // (x-bin 1, y-bin 1): stride nx + 2 = 5
    assert_eq!(
        dst.bin_sumw2()[cell],
        before.bin_entries()[cell] + c * c * src.bin_sumw2()[cell]
    );
    assert_eq!(
        dst.bin_entries()[cell],
        before.bin_entries()[cell] + c * src.bin_entries()[cell]
    );
}

#[test]
fn scaled_3d_add_scales_its_extra_moments() {
    let (mut dst, src) = (profile_3d(&[&SET_A_3D]), profile_3d(&[&SET_B_3D]));
    let before = dst.clone();
    let c = 0.5;
    dst.add(&src, c).unwrap();
    for (name, got, a, b) in [
        ("tsumwxz", dst.tsumwxz(), before.tsumwxz(), src.tsumwxz()),
        ("tsumwyz", dst.tsumwyz(), before.tsumwyz(), src.tsumwyz()),
        ("tsumwt", dst.tsumwt(), before.tsumwt(), src.tsumwt()),
        ("tsumwt2", dst.tsumwt2(), before.tsumwt2(), src.tsumwt2()),
    ] {
        assert_eq!(got, a + c * b, "{name} must scale by c");
    }
    assert_eq!(dst.tsumw2(), before.tsumw2() + c * c * src.tsumw2());
}

#[test]
fn mismatched_binning_is_rejected_and_leaves_the_profile_unchanged() {
    let mut p = profile_2d(&[&SET_A_2D]);
    let before = p.clone();
    let other = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .mean()
        .to_root()
        .unwrap(); // 2 x-bins, not 3
    assert!(matches!(
        p.add(&other, 1.0),
        Err(Error::BinningMismatch { .. })
    ));
    assert_eq!(p, before);

    let mut q = profile_3d(&[&SET_A_3D]);
    let other3 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(3, 0.0, 3.0) // z differs
        .mean()
        .to_root()
        .unwrap();
    assert!(q.add(&other3, 1.0).is_err());
    assert_eq!(q, profile_3d(&[&SET_A_3D]));
}

/// A file may store a profile's `fSumw2` (Σw·v²) empty, meaning all zeros. The
/// 1-D `add` used to pair that array up to the shorter length, so an empty
/// receiving array silently dropped the other profile's values.
#[test]
fn an_empty_value_squared_array_is_treated_as_zeros() {
    let mut a = Hist::reg(2, 0.0, 2.0).mean();
    a.fill_sample(0.5, 2.0);
    let mut a = a.to_root().unwrap();
    a.optional_arrays_mut().0.clear(); // as stored by a file that omitted it
    let mut b = Hist::reg(2, 0.0, 2.0).mean();
    b.fill_sample(0.5, 3.0);
    let b = b.to_root().unwrap();

    a.add(&b, 1.0).unwrap();
    assert_eq!(a.sumy2().len(), a.sums().len());
    assert_eq!(a.sumy2()[1], 9.0, "b's Σw·y² must survive the merge");

    // And an empty array on the other side adds nothing.
    let mut c = Hist::reg(2, 0.0, 2.0).mean();
    c.fill_sample(0.5, 2.0);
    let mut c = c.to_root().unwrap();
    let mut d: Profile1D = c.clone();
    d.optional_arrays_mut().0.clear();
    c.add(&d, 1.0).unwrap();
    assert_eq!(c.sumy2()[1], 4.0);
}

// ------------------------------------------------------------------- Mergeable

#[test]
fn merge_all_folds_2d_and_3d_profiles() {
    let got = Profile2D::merge_all([profile_2d(&[&SET_A_2D]), profile_2d(&[&SET_B_2D])])
        .unwrap()
        .expect("two items");
    assert_eq!(got, profile_2d(&[&SET_A_2D, &SET_B_2D]));

    let got = Profile3D::merge_all([profile_3d(&[&SET_A_3D]), profile_3d(&[&SET_B_3D])])
        .unwrap()
        .expect("two items");
    assert_eq!(got, profile_3d(&[&SET_A_3D, &SET_B_3D]));
}

// ------------------------------------------------------- negative scale factor
//
// ROOT's `TProfileHelper::Add` scales the weights by |c| and only the weighted
// value sums by the signed c, so subtracting a profile flips its values while
// keeping every weight non-negative. The expected numbers below transcribe those
// formulas.

#[test]
fn negative_scale_of_weighted_profiles_squares_the_weights() {
    let (mut dst, src) = (profile_2d(&[&SET_B_2D]), profile_2d(&[&SET_B_2D]));
    let before = dst.clone();
    dst.add(&src, -2.0).unwrap();
    let cell = 1 + 5;
    assert_eq!(
        dst.sums()[cell],
        before.sums()[cell] - 2.0 * src.sums()[cell]
    );
    assert_eq!(
        dst.bin_entries()[cell],
        before.bin_entries()[cell] + 2.0 * src.bin_entries()[cell]
    );
    assert_eq!(
        dst.bin_sumw2()[cell],
        before.bin_sumw2()[cell] + 4.0 * src.bin_sumw2()[cell]
    );
    assert_eq!(dst.tsumwz(), before.tsumwz() + 2.0 * src.tsumwz());
    assert_eq!(dst.tsumw2(), before.tsumw2() + 4.0 * src.tsumw2());
}
