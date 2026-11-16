//! Multithreaded histogram fill — the pure-Rust analog of ROOT's
//! [`TThreadedObject`](https://root.cern/doc/master/classROOT_1_1TThreadedObject.html).
//!
//! ROOT fills histograms across threads by giving each thread a private copy and
//! merging them at the end. oxiroot is set up for exactly this: every histogram
//! type is [`Clone`] plain data, and [`add(other, 1.0)`](crate::Hist1D::add) is an
//! *exact* reduction — it combines bin contents, per-bin `Sumw2`, the entry
//! count, and every statistical moment sum. So a parallel fill is just "clone per
//! thread, fill locally without locking, merge at the end", and the merged
//! histogram is identical to a serial fill (up to floating-point summation order).
//!
//! The accumulator and the parallel fill are oxihist's, shared by both crates:
//!
//! - [`Mergeable`] — the reduction trait. oxiroot implements it for its ROOT
//!   types (`merge` == `add(other, 1.0)`), with [`oxiroot_io_core::Error`] as
//!   the error; its [`merge_all`](Mergeable::merge_all) folds an iterator of
//!   histograms into one (an in-memory `hadd`).
//! - [`ThreadedHist`] — the accumulator: share `&ThreadedHist`, fill from any
//!   thread (each gets its own copy), then [`merge`](ThreadedHist::merge) at the
//!   end. Works with [`std::thread::scope`].
//! - [`fill_par`] — one-call parallel fill of a slice (requires the `rayon`
//!   feature).
//!
//! `ThreadedHist` comes from oxihist, so the `fill` methods for ROOT types are
//! in four small traits, by the number of values a fill takes:
//! [`ThreadedFill1`] (`Hist1D`), [`ThreadedFill2`] (`Hist2D`, `Profile1D`),
//! [`ThreadedFill3`] (`Hist3D`, `Profile2D`) and [`ThreadedFill4`]
//! (`Profile3D`). They are in the prelude.

pub use oxihist::{Mergeable, ThreadedHist};

#[cfg(feature = "rayon")]
pub use oxihist::fill_par;

use oxiroot_io_core::{Error, Result};

use crate::{
    Efficiency, Graph, Hist1D, Hist2D, Hist3D, PolyHist, Profile1D, Profile2D, Profile3D,
    SparseHist,
};

macro_rules! impl_mergeable {
    ($($t:ty),+ $(,)?) => {$(
        /// `merge` is `add(other, 1.0)`: contents, `Sumw2`, the entry count and
        /// every moment sum add up, so the result equals one histogram filled
        /// with all the data. [`Error::BinningMismatch`] (leaving `self`
        /// unchanged) if the binnings differ.
        impl Mergeable for $t {
            type Error = Error;

            fn merge(&mut self, other: &Self) -> Result<()> {
                self.add(other, 1.0)
            }
        }
    )+};
}
impl_mergeable!(Hist1D, Hist2D, Hist3D, Profile1D, Profile2D, Profile3D, PolyHist, SparseHist);

impl Mergeable for Efficiency {
    type Error = Error;

    fn merge(&mut self, other: &Self) -> Result<()> {
        self.add(other)
    }
}

impl Mergeable for Graph {
    type Error = Error;

    /// Appends `other`'s points, the way ROOT's `hadd` merges graphs.
    fn merge(&mut self, other: &Self) -> Result<()> {
        self.append(other)
    }
}

/// `fill` for a [`ThreadedHist`] of a histogram filled with one value:
/// [`Hist1D`].
///
/// ```
/// use oxiroot_hist::{Hist, ThreadedFill1, ThreadedHist, ToRoot};
///
/// let data: Vec<f64> = (0..1000).map(|i| i as f64 % 100.0).collect();
/// let hist = ThreadedHist::new(Hist::reg(100, 0.0, 100.0).double().to_root().named("h"));
///
/// std::thread::scope(|s| {
///     for chunk in data.chunks(data.len().div_ceil(4)) {
///         let hist = &hist;
///         s.spawn(move || {
///             for &x in chunk {
///                 hist.fill(x); // each thread fills its own copy — no setup
///             }
///         });
///     }
/// });
///
/// let merged = hist.merge().unwrap(); // combine every thread's copy
/// assert_eq!(merged.entries, 1000.0);
/// ```
pub trait ThreadedFill1 {
    /// Fill the calling thread's copy with `x` (weight 1).
    fn fill(&self, x: f64);
    /// Fill the calling thread's copy with `x` and weight `w`.
    fn fill_weight(&self, x: f64, w: f64);
}

/// `fill` for a [`ThreadedHist`] of a histogram filled with two values:
/// [`Hist2D`] at `(x, y)`, or [`Profile1D`] profiling `y` at `x`.
pub trait ThreadedFill2 {
    /// Fill the calling thread's copy with `(x, y)` (weight 1).
    fn fill(&self, x: f64, y: f64);
    /// Fill the calling thread's copy with `(x, y)` and weight `w`.
    fn fill_weight(&self, x: f64, y: f64, w: f64);
}

/// `fill` for a [`ThreadedHist`] of a histogram filled with three values:
/// [`Hist3D`] at `(x, y, z)`, or [`Profile2D`] profiling `z` at `(x, y)`.
pub trait ThreadedFill3 {
    /// Fill the calling thread's copy with `(x, y, z)` (weight 1).
    fn fill(&self, x: f64, y: f64, z: f64);
    /// Fill the calling thread's copy with `(x, y, z)` and weight `w`.
    fn fill_weight(&self, x: f64, y: f64, z: f64, w: f64);
}

/// `fill` for a [`ThreadedHist`] of a histogram filled with four values:
/// [`Profile3D`] profiling `t` at `(x, y, z)`.
pub trait ThreadedFill4 {
    /// Fill the calling thread's copy with `(x, y, z, t)` (weight 1).
    fn fill(&self, x: f64, y: f64, z: f64, t: f64);
    /// Fill the calling thread's copy with `(x, y, z, t)` and weight `w`.
    fn fill_weight(&self, x: f64, y: f64, z: f64, t: f64, w: f64);
}

macro_rules! threaded_fill {
    ($tr:ident for $t:ty => ($($v:ident),+)) => {
        impl $tr for ThreadedHist<$t> {
            fn fill(&self, $($v: f64),+) {
                self.with_local(|h| h.fill($($v),+));
            }
            fn fill_weight(&self, $($v: f64,)+ w: f64) {
                self.with_local(|h| h.fill_weight($($v,)+ w));
            }
        }
    };
}
threaded_fill!(ThreadedFill1 for Hist1D => (x));
threaded_fill!(ThreadedFill2 for Hist2D => (x, y));
threaded_fill!(ThreadedFill2 for Profile1D => (x, y));
threaded_fill!(ThreadedFill3 for Hist3D => (x, y, z));
threaded_fill!(ThreadedFill3 for Profile2D => (x, y, z));
threaded_fill!(ThreadedFill4 for Profile3D => (x, y, z, t));
