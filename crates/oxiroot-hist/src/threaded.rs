//! Merging, and filling from several threads.
//!
//! [`Mergeable`] is oxihist's reduction trait. It is implemented here for the
//! ROOT types, with [`oxiroot_io_core::Error`] as the error: `merge` is
//! [`add(&other, 1.0)`](crate::Hist1D::add), an exact sum of bin contents,
//! per-bin `Sumw2`, the entry count and every statistical sum. Its
//! [`merge_all`](Mergeable::merge_all) folds an iterator of records into one,
//! an in-memory `hadd`.
//!
//! [`ThreadedHist`] is oxihist's accumulator: share `&ThreadedHist`, fill from
//! any thread (each gets its own copy), then
//! [`merge`](ThreadedHist::merge). Fill an oxihist histogram that way and
//! convert the result with [`ToRoot::to_root`](crate::ToRoot::to_root). The
//! types that are filled directly ([`Efficiency`], [`PolyHist`],
//! [`SparseHist`]) go through [`with_local`](ThreadedHist::with_local).

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
        /// `merge` is `add(&other, 1.0)`: contents, `Sumw2`, the entry count and
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
impl_mergeable!(
    Hist1D, Hist2D, Hist3D, Profile1D, Profile2D, Profile3D, PolyHist, SparseHist
);

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
