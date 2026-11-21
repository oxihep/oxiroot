//! Histogram arithmetic: scale, add (merge), multiply, divide, integral.
//!
//! These follow ROOT's `Hist1D::Scale`/`Add`/`Multiply`/`Divide` semantics,
//! including per-bin error (`Sumw2`) propagation. `add` with `c = 1` is the
//! bin-by-bin merge used to combine outputs across parallel jobs (`hadd`).

use std::collections::HashMap;

use oxiroot_io_core::{Error, Result};

use crate::graph::GraphErrors;
use crate::sparsehist::SparseBin;
use crate::{
    Efficiency, Graph, Hist1D, Hist2D, Hist3D, PolyHist, Profile1D, Profile2D, Profile3D,
    SparseHist,
};

/// Build a [`Error::BinningMismatch`] describing which operation rejected which
/// pair of incompatible histograms.
fn binning_mismatch(op: &str) -> Error {
    Error::BinningMismatch {
        detail: format!("{op}: histograms have different binning"),
    }
}

impl Hist1D {}

/// Add `c · src` into `dst` for a per-cell array a file may leave empty, where
/// empty means every cell is zero (a profile's `fSumw2`). An empty `dst` is
/// filled with zeros first so `src` is never dropped; an empty `src` adds nothing.
fn add_optional_cells(dst: &mut Vec<f64>, src: &[f64], c: f64, ncells: usize) {
    if src.is_empty() {
        return;
    }
    if dst.is_empty() {
        dst.resize(ncells, 0.0);
    }
    for (d, &s) in dst.iter_mut().zip(src) {
        *d += c * s;
    }
}

/// Whether an optional per-cell array is either absent or covers every cell.
fn optional_len_ok(len: usize, ncells: usize) -> bool {
    len == 0 || len == ncells
}

/// `add` for the whole profile family. The three profiles differ only in their
/// axes, the name of their per-cell `Σw·v²` array and their moment sums, so one
/// body serves them all: the per-cell error bookkeeping is exactly where
/// hand-copied versions had already drifted apart.
macro_rules! impl_profile_add {
    (
        $ty:ident, $dim:literal,
        axes: [$($axis:ident),+],
        value_sq: $sumv2:ident,
        moments: [$($moment:ident),+ $(,)?] $(,)?
    ) => {
        impl $ty {
            #[doc = concat!("Merge `c * other` into this ", $dim, " profile cell by cell (a")]
            /// `hadd`-style merge when `c == 1`). A profile cannot be merged like a
            /// plain histogram: its per-cell weight sums (`fBinEntries`), weighted-value
            /// sums (the base contents), weighted-value² sums (`fSumw2`) and `Σw²`
            /// (`fBinSumw2`) must all be summed so the profiled value `Σwv / Σw` and its
            /// error stay correct.
            ///
            /// As in ROOT's `TProfileHelper::Add`, `c` scales the weights by `|c|`
            /// and a negative `c` flips the sign of the profiled values: the
            /// weighted-value sums scale by `c`; the weight sums, weighted-value²
            /// sums, entries and moments by `|c|`; and `Σw²` by `c²`. Returns
            /// [`Error::BinningMismatch`], leaving `self` unchanged, if the binnings
            /// or per-cell array lengths differ.
            ///
            /// # Errors
            ///
            /// [`Error::BinningMismatch`] if the binnings differ; `self` is left unchanged.
            pub fn add(&mut self, other: &$ty, c: f64) -> Result<()> {
                let ncells = self.sums.len();
                let compatible = $(self.$axis.same_binning(&other.$axis))&&+
                    && other.sums.len() == ncells
                    && self.bin_entries.len() == ncells
                    && other.bin_entries.len() == ncells
                    && optional_len_ok(self.$sumv2.len(), ncells)
                    && optional_len_ok(other.$sumv2.len(), ncells)
                    && optional_len_ok(self.bin_sumw2.len(), ncells)
                    && optional_len_ok(other.bin_sumw2.len(), ncells);
                if !compatible {
                    return Err(binning_mismatch("add"));
                }
                // The weights of `other` enter scaled by |c|.
                let ac = c.abs();
                // Seed our Σw² from the weight sums (exact for unit weights) before
                // any sum changes, whenever it is about to stop being derivable from
                // them: `other` tracks it explicitly, or its weights are rescaled,
                // since Σ(|c|·w)² ≠ Σ(|c|·w) unless |c| == 1.
                if !other.bin_sumw2.is_empty() || ac != 1.0 {
                    self.track_bin_sumw2();
                }
                if !self.bin_sumw2.is_empty() {
                    // `other`'s Σw² is its `fBinSumw2` if tracked, else its weight sums.
                    let src: &[f64] = if other.bin_sumw2.is_empty() {
                        &other.bin_entries
                    } else {
                        &other.bin_sumw2
                    };
                    // Σw² is quadratic in the weight: Σ(c·w)² = c²·Σw².
                    for (d, &s) in self.bin_sumw2.iter_mut().zip(src) {
                        *d += c * c * s;
                    }
                }
                for (d, &s) in self.sums.iter_mut().zip(&other.sums) {
                    *d += c * s;
                }
                for (d, &s) in self.bin_entries.iter_mut().zip(&other.bin_entries) {
                    *d += ac * s;
                }
                add_optional_cells(&mut self.$sumv2, &other.$sumv2, ac, ncells);

                self.entries += ac * other.entries;
                // Σw² is quadratic in the weight; every other moment is linear in
                // it, so takes |c|.
                self.tsumw2 += c * c * other.tsumw2;
                $(self.$moment += ac * other.$moment;)+
                Ok(())
            }
        }
    };
}

impl_profile_add!(
    Profile1D, "1-D",
    axes: [xaxis],
    value_sq: sumy2,
    moments: [tsumw, tsumwx, tsumwx2, tsumwy, tsumwy2],
);
impl_profile_add!(
    Profile2D, "2-D",
    axes: [xaxis, yaxis],
    value_sq: sumz2,
    moments: [tsumw, tsumwx, tsumwx2, tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2],
);
impl_profile_add!(
    Profile3D, "3-D",
    axes: [xaxis, yaxis, zaxis],
    value_sq: sumt2,
    moments: [
        tsumw, tsumwx, tsumwx2, tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz,
        tsumwt, tsumwt2,
    ],
);

impl std::fmt::Display for Hist1D {
    /// A one-line summary, e.g. `TH1D "pt": 100 bins [0, 100), entries=4096`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:?}: {} bins [{}, {}), entries={}",
            self.class_name(),
            self.name,
            self.xaxis.nbins.max(0),
            self.xaxis.xmin,
            self.xaxis.xmax,
            self.entries
        )
    }
}

impl std::fmt::Display for Hist2D {
    /// A one-line summary, e.g. `TH2D "h": 100x100 bins, entries=4096`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:?}: {}x{} bins, entries={}",
            self.class_name(),
            self.name,
            self.xaxis.nbins.max(0),
            self.yaxis.nbins.max(0),
            self.entries
        )
    }
}

impl std::fmt::Display for Hist3D {
    /// A one-line summary, e.g. `TH3D "h": 20x20x20 bins, entries=4096`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:?}: {}x{}x{} bins, entries={}",
            self.class_name(),
            self.name,
            self.xaxis.nbins.max(0),
            self.yaxis.nbins.max(0),
            self.zaxis.nbins.max(0),
            self.entries
        )
    }
}

impl Efficiency {
    /// Add `other`'s counts: its passed and its total histogram, bin by bin, as
    /// ROOT's `Efficiency::Add` and `hadd` do. The confidence level, statistic
    /// option and weight stay as they are.
    ///
    /// # Errors
    ///
    /// [`Error::BinningMismatch`] and makes no change if the binnings
    /// differ.
    pub fn add(&mut self, other: &Self) -> Result<()> {
        // Both first, so a mismatch leaves the efficiency untouched.
        let mut passed = self.passed.clone();
        let mut total = self.total.clone();
        passed.add(&other.passed, 1.0)?;
        total.add(&other.total, 1.0)?;
        self.passed = passed;
        self.total = total;
        Ok(())
    }
}

impl PolyHist {
    /// Add `c * other`: every bin's content, the nine overflow regions, the
    /// entry count and the statistics sums, as ROOT's `PolyHist::Add` and `hadd`
    /// do.
    ///
    /// # Errors
    ///
    /// [`Error::BinningMismatch`] and makes no change unless the two
    /// hold the same bins, in the same order.
    pub fn add(&mut self, other: &Self, c: f64) -> Result<()> {
        let same = self.bins.len() == other.bins.len()
            && (self.bins.iter())
                .zip(&other.bins)
                .all(|(a, b)| a.x == b.x && a.y == b.y);
        if !same {
            return Err(binning_mismatch("TH2Poly::add"));
        }
        for (bin, from) in self.bins.iter_mut().zip(&other.bins) {
            bin.content += c * from.content;
        }
        for (region, from) in self.overflow.iter_mut().zip(other.overflow) {
            *region += c * from;
        }
        self.entries += c * other.entries;
        self.tsumw += c * other.tsumw;
        self.tsumw2 += c * c * other.tsumw2;
        self.tsumwx += c * other.tsumwx;
        self.tsumwx2 += c * other.tsumwx2;
        self.tsumwy += c * other.tsumwy;
        self.tsumwy2 += c * other.tsumwy2;
        self.tsumwxy += c * other.tsumwxy;
        Ok(())
    }
}

impl SparseHist {
    /// Add `c * other`: the filled bins they share, the bins only `other` holds,
    /// the entry count and the statistics sums, as ROOT's `SparseHist::Add` and
    /// `hadd` do.
    ///
    /// # Errors
    ///
    /// [`Error::BinningMismatch`] and makes no change if the axes
    /// differ.
    pub fn add(&mut self, other: &Self, c: f64) -> Result<()> {
        let same = self.axes.len() == other.axes.len()
            && (self.axes.iter())
                .zip(&other.axes)
                .all(|(a, b)| a.same_binning(b));
        if !same {
            return Err(binning_mismatch("THnSparse::add"));
        }
        let at: HashMap<Vec<i32>, usize> = (self.bins.iter().enumerate())
            .map(|(i, bin)| (bin.coords.clone(), i))
            .collect();
        for bin in &other.bins {
            match at.get(&bin.coords) {
                Some(&i) => self.bins[i].content += c * bin.content,
                None => self.bins.push(SparseBin {
                    coords: bin.coords.clone(),
                    content: c * bin.content,
                }),
            }
        }
        self.entries += c * other.entries;
        self.tsumw += c * other.tsumw;
        self.tsumw2 += c * c * other.tsumw2;
        for (sum, from) in self.tsumwx.iter_mut().zip(&other.tsumwx) {
            *sum += c * from;
        }
        for (sum, from) in self.tsumwx2.iter_mut().zip(&other.tsumwx2) {
            *sum += c * from;
        }
        Ok(())
    }
}

impl Graph {
    /// Append `other`'s points, as ROOT's `Graph::Merge` and `hadd` do: this
    /// graph's points keep their order and `other`'s follow, with their errors.
    /// The attached functions and the display frame stay as they are.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] and makes no change if the two carry
    /// different kinds of error bars (a `Graph` and a `TGraphErrors`, say).
    pub fn append(&mut self, other: &Self) -> Result<()> {
        match (&mut self.errors, &other.errors) {
            (GraphErrors::Plain, GraphErrors::Plain) => {}
            (
                GraphErrors::Symmetric { ex, ey },
                GraphErrors::Symmetric {
                    ex: from_ex,
                    ey: from_ey,
                },
            ) => {
                ex.extend_from_slice(from_ex);
                ey.extend_from_slice(from_ey);
            }
            (
                GraphErrors::Asymmetric {
                    ex_low,
                    ex_high,
                    ey_low,
                    ey_high,
                },
                GraphErrors::Asymmetric {
                    ex_low: from_ex_low,
                    ex_high: from_ex_high,
                    ey_low: from_ey_low,
                    ey_high: from_ey_high,
                },
            ) => {
                ex_low.extend_from_slice(from_ex_low);
                ex_high.extend_from_slice(from_ex_high);
                ey_low.extend_from_slice(from_ey_low);
                ey_high.extend_from_slice(from_ey_high);
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "cannot append a {} to a {}: they carry different error bars",
                    other.class_name(),
                    self.class_name()
                )));
            }
        }
        self.x.extend_from_slice(&other.x);
        self.y.extend_from_slice(&other.y);
        Ok(())
    }
}
