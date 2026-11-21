//! [`HistRecord`], the one type behind [`Hist1D`], [`Hist2D`] and
//! [`Hist3D`], and what the three share whatever their dimension.

use oxiroot_io_core::{Error, Result};

use crate::axis::Axis;
use crate::base::{BinContentType, Moments};
use crate::graph::GraphFunction;

/// A classic ROOT histogram of `D` dimensions as a file stores it: the
/// binning, the cells, their errors and the sums over the fills. Use it
/// through [`Hist1D`], [`Hist2D`] and [`Hist3D`]; ROOT has no `TH4`, and none
/// can be built here.
///
/// Contents are widened to `f64` whatever the class stores.
#[derive(Debug, Clone, PartialEq)]
pub struct HistRecord<const D: usize> {
    /// On-disk [`BinContentType`] (the class suffix). Read the class name via
    /// [`class_name`](Self::class_name); `pub(crate)` so the type stays a
    /// typed value rather than a free-form string.
    pub(crate) bin_content_type: BinContentType,
    /// Histogram name (`fName`).
    pub(crate) name: String,
    /// Histogram title (`fTitle`).
    pub(crate) title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis (one bin, unused, below two dimensions: ROOT stores all three).
    pub(crate) yaxis: Axis,
    /// Z axis (one bin, unused, below three dimensions).
    pub(crate) zaxis: Axis,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// The sums over the fills (`fTsumw` and the rest).
    pub(crate) moments: Moments,
    /// Cell contents including flow (x fastest, then y, then z).
    pub(crate) contents: Vec<f64>,
    /// Per-cell sum of squared weights (`fSumw2`); empty when the histogram
    /// does not track errors. When present, a cell's error is
    /// `sqrt(sumw2[cell])`.
    pub(crate) sumw2: Vec<f64>,
    /// The functions attached to the histogram (`fFunctions`).
    pub(crate) functions: Vec<GraphFunction>,
}

/// A 1-D classic histogram (`TH1D`, `TH1F`, …).
#[doc(alias = "TH1", alias = "TH1D", alias = "TH1F")]
pub type Hist1D = HistRecord<1>;
/// A 2-D classic histogram (`TH2D`, `TH2F`, …).
#[doc(alias = "TH2", alias = "TH2D", alias = "TH2F")]
pub type Hist2D = HistRecord<2>;
/// A 3-D classic histogram (`TH3D`, `TH3F`, …).
#[doc(alias = "TH3", alias = "TH3D", alias = "TH3F")]
pub type Hist3D = HistRecord<3>;

/// Effective per-bin error² for `other`: its `fSumw2[i]` if tracked, else the
/// content (for an unweighted histogram, `Σw² == Σw == content`).
pub(crate) fn err2(sumw2: &[f64], contents: &[f64], i: usize) -> f64 {
    sumw2.get(i).copied().unwrap_or_else(|| contents[i].abs())
}

impl<const D: usize> HistRecord<D> {
    /// The class name without its content suffix.
    const CLASS: &'static str = match D {
        1 => "TH1",
        2 => "TH2",
        _ => "TH3",
    };

    /// The axes the histogram bins along: the first `D` of the three stored.
    pub(crate) fn axes(&self) -> impl Iterator<Item = &Axis> {
        [&self.xaxis, &self.yaxis, &self.zaxis].into_iter().take(D)
    }

    /// The number of cells, flow included, the binning implies.
    pub(crate) fn expected_cells(&self) -> usize {
        self.axes().map(|axis| axis.nbins + 2).product()
    }

    /// Total cells, flow included (ROOT's `fNcells`).
    #[must_use]
    pub fn ncells(&self) -> usize {
        self.contents.len()
    }

    /// The ROOT class name: `TH1D`, `TH2F`, … per the dimension and the
    /// [`BinContentType`].
    #[must_use]
    pub fn class_name(&self) -> String {
        self.bin_content_type.class_name(Self::CLASS)
    }

    /// The on-disk content type (the class suffix).
    #[must_use]
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
    /// `c == 1`). Errors are tracked if either side tracks them (or
    /// `c != 1`).
    ///
    /// # Errors
    ///
    /// [`Error::BinningMismatch`] if the binnings differ; `self` is left unchanged.
    pub fn add(&mut self, other: &Self, c: f64) -> Result<()> {
        let same = self
            .axes()
            .zip(other.axes())
            .all(|(a, b)| a.same_binning(b))
            && self.contents.len() == other.contents.len();
        if !same {
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
        for (i, error2) in self.sumw2.iter_mut().enumerate() {
            *error2 += c * c * err2(&other.sumw2, &other.contents, i);
        }
        self.entries += c * other.entries;
        self.moments.add_scaled(&other.moments, c);
        Ok(())
    }
}
