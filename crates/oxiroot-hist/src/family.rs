//! The methods the plain histograms — [`Hist1D`], [`Hist2D`], [`Hist3D`] —
//! share, written once: they differ only in their axes and moment sums, and
//! hand-copied versions drift apart (the profiles' `add` in `ops.rs` is the
//! same pattern).

use oxiroot_io_core::{Error, Result};

use crate::axis::Axis;
use crate::base::BinContentType;
use crate::{Hist1D, Hist2D, Hist3D, Profile1D, Profile2D, Profile3D};

/// The number of in-range bins of `axis` (none for a malformed negative
/// count).
pub(crate) fn nbins(axis: &Axis) -> usize {
    usize::try_from(axis.nbins).unwrap_or(0)
}

/// Effective per-bin error² for `other`: its `fSumw2[i]` if tracked, else the
/// content (for an unweighted histogram, `Σw² == Σw == content`).
pub(crate) fn err2(sumw2: &[f64], contents: &[f64], i: usize) -> f64 {
    sumw2.get(i).copied().unwrap_or_else(|| contents[i].abs())
}

macro_rules! impl_hist_family {
    (
        $ty:ident, $class:literal,
        axes: [$($axis:ident),+],
        moments: [$($moment:ident),+ $(,)?] $(,)?
    ) => {
        impl $ty {
            /// The number of cells, flow included, the binning implies.
            pub(crate) fn expected_cells(&self) -> usize {
                [$(nbins(&self.$axis) + 2),+].iter().product()
            }

            /// Total cells, flow included (ROOT's `fNcells`).
            pub fn ncells(&self) -> i32 {
                i32::try_from(self.contents.len()).unwrap_or(i32::MAX)
            }

            #[doc = concat!("The ROOT class name: `", $class, "D`, `", $class, "F`, … per the [`BinContentType`].")]
            pub fn class_name(&self) -> String {
                self.bin_content_type.class_name($class)
            }

            /// The on-disk content type (the class suffix).
            pub fn bin_content_type(&self) -> BinContentType {
                self.bin_content_type
            }

            /// The same histogram, written with another content type.
            #[must_use]
            pub fn with_bin_content_type(mut self, bin_content_type: BinContentType) -> Self {
                self.bin_content_type = bin_content_type;
                self
            }

            /// Add `c * other` into this histogram (a bin-by-bin merge when
            /// `c == 1`). Returns [`Error::BinningMismatch`] and makes no change if
            /// the binnings differ. Errors are tracked if either side tracks them
            /// (or `c != 1`).
            pub fn add(&mut self, other: &$ty, c: f64) -> Result<()> {
                if !($(self.$axis.same_binning(&other.$axis))&&+)
                    || self.contents.len() != other.contents.len()
                {
                    return Err(Error::BinningMismatch {
                        detail: "add: histograms have different binning".into(),
                    });
                }
                if self.sumw2.is_empty() && (!other.sumw2.is_empty() || c != 1.0) {
                    self.sumw2 = self.contents.iter().map(|v| v.abs()).collect();
                }
                for (d, s) in self.contents.iter_mut().zip(&other.contents) {
                    *d += c * s;
                }
                for i in 0..self.sumw2.len() {
                    self.sumw2[i] += c * c * err2(&other.sumw2, &other.contents, i);
                }
                self.entries += c * other.entries;
                self.tsumw += c * other.tsumw;
                self.tsumw2 += c * c * other.tsumw2;
                $(self.$moment += c * other.$moment;)+
                Ok(())
            }

        }
    };
}

impl_hist_family!(Hist1D, "TH1", axes: [xaxis], moments: [tsumwx, tsumwx2]);
impl_hist_family!(
    Hist2D, "TH2",
    axes: [xaxis, yaxis],
    moments: [tsumwx, tsumwx2, tsumwy, tsumwy2, tsumwxy],
);
impl_hist_family!(
    Hist3D, "TH3",
    axes: [xaxis, yaxis, zaxis],
    moments: [
        tsumwx, tsumwx2, tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz,
    ],
);

/// The per-cell core the profiles ([`Profile1D`], [`Profile2D`],
/// [`Profile3D`]) share: they differ in their axes, the name of their `Σw·v²`
/// array and their moment sums, not in how a cell accumulates, averages or
/// errs.
macro_rules! impl_profile_family {
    ($ty:ident, value_sq: $sumv2:ident $(,)?) => {
        impl $ty {
            /// Seed per-cell `Σw²` from the weight sums (exact for unit
            /// weights) the first time it is needed.
            pub(crate) fn track_bin_sumw2(&mut self) {
                if self.bin_sumw2.is_empty() {
                    self.bin_sumw2 = self.bin_entries.clone();
                }
            }

            /// The profiled value of cell `cell` (flow included): `Σw·v / Σw`,
            /// or 0 where the cell has no entries (ROOT's `GetBinContent`).
            #[must_use]
            pub(crate) fn bin_value(&self, cell: usize) -> f64 {
                let e = self.bin_entries.get(cell).copied().unwrap_or(0.0);
                if e == 0.0 {
                    0.0
                } else {
                    self.sums.get(cell).copied().unwrap_or(0.0) / e
                }
            }
        }
    };
}

impl_profile_family!(Profile1D, value_sq: sumy2);
impl_profile_family!(Profile2D, value_sq: sumz2);
impl_profile_family!(Profile3D, value_sq: sumt2);
