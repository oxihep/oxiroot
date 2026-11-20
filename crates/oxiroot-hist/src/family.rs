//! The methods the plain histograms — [`Hist1D`], [`Hist2D`], [`Hist3D`] —
//! share, written once: they differ only in their axes and moment sums, and
//! hand-copied versions drift apart (the profiles' `add` in `ops.rs` is the
//! same pattern).

use oxiroot_io_core::{Error, Result};

use crate::axis::Axis;
use crate::base::BinContentType;
use crate::{Hist1D, Hist2D, Hist3D};

/// The number of in-range bins of `axis` (none for a malformed negative
/// count).
pub(crate) fn nbins(axis: &Axis) -> usize {
    usize::try_from(axis.nbins).unwrap_or(0)
}

/// The flat cell indices (flow included in the layout, x varying fastest in
/// memory) of the in-range cells of a histogram with `n[k]` bins on axis `k`,
/// visited with the last axis varying fastest — the order the per-dimension
/// loops used, so sums and ties come out the same.
pub(crate) fn in_range_cells(n: &[usize]) -> impl Iterator<Item = usize> + use<> {
    let n = n.to_vec();
    let total: usize = n.iter().product();
    (0..total).map(move |mut t| {
        let mut idx = vec![0usize; n.len()];
        for k in (0..n.len()).rev() {
            idx[k] = t % n[k] + 1;
            t /= n[k];
        }
        let (mut off, mut stride) = (0, 1);
        for (k, &i) in idx.iter().enumerate() {
            off += i * stride;
            stride *= n[k] + 2;
        }
        off
    })
}

/// Effective per-bin error² for `other`: its `fSumw2[i]` if tracked, else the
/// content (for an unweighted histogram, `Σw² == Σw == content`).
fn err2(sumw2: &[f64], contents: &[f64], i: usize) -> f64 {
    sumw2.get(i).copied().unwrap_or_else(|| contents[i].abs())
}

/// `(cell index, value)` of the largest (or smallest) of the given cells.
fn extremum(contents: &[f64], cells: impl Iterator<Item = usize>, want_max: bool) -> (usize, f64) {
    let mut best = (
        0usize,
        if want_max {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        },
    );
    for c in cells {
        let v = contents.get(c).copied().unwrap_or(0.0);
        if (want_max && v > best.1) || (!want_max && v < best.1) {
            best = (c, v);
        }
    }
    best
}

macro_rules! impl_hist_family {
    (
        $ty:ident, $class:literal,
        axes: [$($axis:ident),+],
        moments: [$($moment:ident),+ $(,)?] $(,)?
    ) => {
        impl $ty {
            /// The flat indices of the in-range cells.
            fn in_range(&self) -> impl Iterator<Item = usize> + use<> {
                in_range_cells(&[$(nbins(&self.$axis)),+])
            }

            /// Enable per-bin error tracking (ROOT's `Sumw2`): allocate the
            /// `fSumw2` array and seed it from the current contents, after which
            /// every fill also accumulates `weight²`. Weighted fills and
            /// [`scale`](Self::scale) turn it on by themselves, so this is only
            /// needed to track a unit-weight histogram. Returns `&mut self` so it
            /// can chain (`h.sumw2().fill(…)`).
            pub fn sumw2(&mut self) -> &mut Self {
                if self.sumw2.len() != self.contents.len() {
                    self.sumw2 = self.contents.iter().map(|c| c.abs()).collect();
                }
                self
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

            /// Per-bin error: `√sumw2[bin]` when error tracking is on, otherwise
            /// the Poisson default `√content`. `bin` is the flat cell index, flow
            /// included (x fastest; for a 1-D histogram, the bin number with 0
            /// the underflow).
            pub fn bin_error(&self, bin: usize) -> f64 {
                if let Some(&s) = self.sumw2.get(bin) {
                    s.max(0.0).sqrt()
                } else {
                    self.contents
                        .get(bin)
                        .copied()
                        .unwrap_or(0.0)
                        .max(0.0)
                        .sqrt()
                }
            }

            /// Multiply all bin contents (and errors) by `c`; the means are
            /// preserved.
            ///
            /// Turns on per-bin error tracking first, as ROOT's `Scale` does: once
            /// scaled, the errors can no longer be derived as `√content`.
            pub fn scale(&mut self, c: f64) {
                self.sumw2();
                for v in &mut self.contents {
                    *v *= c;
                }
                for v in &mut self.sumw2 {
                    *v *= c * c;
                }
                self.tsumw *= c;
                self.tsumw2 *= c * c;
                $(self.$moment *= c;)+
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

            /// Sum of the in-range bin contents (flow excluded on every axis).
            pub fn integral(&self) -> f64 {
                self.in_range()
                    .map(|i| self.contents.get(i).copied().unwrap_or(0.0))
                    .sum()
            }

            /// Largest in-range bin content (ROOT `GetMaximum`).
            pub fn maximum(&self) -> f64 {
                extremum(&self.contents, self.in_range(), true).1
            }

            /// Smallest in-range bin content (ROOT `GetMinimum`).
            pub fn minimum(&self) -> f64 {
                extremum(&self.contents, self.in_range(), false).1
            }

            /// The flat cell index of the largest in-range content (ROOT
            /// `GetMaximumBin`; for a 1-D histogram, the 1-based bin).
            pub fn maximum_bin(&self) -> usize {
                extremum(&self.contents, self.in_range(), true).0
            }

            /// The flat cell index of the smallest in-range content (ROOT
            /// `GetMinimumBin`).
            pub fn minimum_bin(&self) -> usize {
                extremum(&self.contents, self.in_range(), false).0
            }

            /// Effective entry count `(Σw)²/Σw²` (ROOT `GetEffectiveEntries`).
            pub fn effective_entries(&self) -> f64 {
                if self.tsumw2 > 0.0 {
                    self.tsumw * self.tsumw / self.tsumw2
                } else {
                    0.0
                }
            }

            /// Clear all bin contents, errors, entries and moment sums (ROOT
            /// `Reset`), keeping the binning.
            pub fn reset(&mut self) {
                self.contents.iter_mut().for_each(|v| *v = 0.0);
                self.sumw2.iter_mut().for_each(|v| *v = 0.0);
                self.entries = 0.0;
                self.tsumw = 0.0;
                self.tsumw2 = 0.0;
                $(self.$moment = 0.0;)+
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

#[cfg(test)]
mod tests {
    use super::in_range_cells;

    #[test]
    fn in_range_cells_match_the_per_dimension_loops() {
        assert_eq!(in_range_cells(&[3]).collect::<Vec<_>>(), [1, 2, 3]);
        // 2-D, nx = 2, ny = 3: ix outer, iy inner; cell = ix + 4·iy.
        let want: Vec<usize> = (1..=2)
            .flat_map(|ix| (1..=3).map(move |iy| ix + 4 * iy))
            .collect();
        assert_eq!(in_range_cells(&[2, 3]).collect::<Vec<_>>(), want);
        let want: Vec<usize> = (1..=2)
            .flat_map(|ix| {
                (1..=2).flat_map(move |iy| (1..=3).map(move |iz| ix + 4 * (iy + 4 * iz)))
            })
            .collect();
        assert_eq!(in_range_cells(&[2, 2, 3]).collect::<Vec<_>>(), want);
        assert_eq!(in_range_cells(&[0, 3]).count(), 0);
    }
}
