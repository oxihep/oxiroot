//! 1-D histograms (`TH1D`, `TH1F`).
//!
//! Streamed layout: `TH1x{ Hist1D{ … }, TArray }`. The `Hist1D` base is shared via
//! the crate's `base` module; the inline `TArray` holds the bin contents.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Object, ReadRoot};
use oxiroot_io_core::{RBuffer, Result};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_th1_object,
};

/// A 1-D classic histogram (`TH1D` or `TH1F`); contents are widened to `f64`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TH1", alias = "TH1D", alias = "TH1F")]
pub struct Hist1D {
    /// On-disk [`BinContentType`] (the class suffix). Read the class name via
    /// [`class_name`](Hist1D::class_name); `pub(crate)` so the type stays a
    /// typed value rather than a free-form string.
    pub(crate) bin_content_type: BinContentType,
    /// Histogram name (`fName`).
    pub name: String,
    /// Histogram title (`fTitle`).
    pub title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis (degenerate for 1-D).
    pub(crate) yaxis: Axis,
    /// Z axis (degenerate for 1-D).
    pub(crate) zaxis: Axis,
    /// Total cells, including under/overflow (`fNcells = nbins + 2`). Read it
    /// through [`ncells`](Hist1D::ncells); `pub(crate)` so it cannot drift from
    /// `contents` via outside mutation.
    pub(crate) ncells: i32,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// Sum of weights (`fTsumw`).
    pub(crate) tsumw: f64,
    /// Sum of squared weights (`fTsumw2`).
    pub(crate) tsumw2: f64,
    /// Sum of weight*x (`fTsumwx`).
    pub(crate) tsumwx: f64,
    /// Sum of weight*x^2 (`fTsumwx2`).
    pub(crate) tsumwx2: f64,
    /// Bin contents including under/overflow (length `ncells`).
    pub(crate) contents: Vec<f64>,
    /// Per-bin sum of squared weights (`fSumw2`); empty when the histogram
    /// does not track errors. When present, a bin's error is
    /// `sqrt(sumw2[bin])`.
    pub(crate) sumw2: Vec<f64>,
}

impl Hist1D {
    /// Create an empty `TH1D` with `nbins` uniform bins over `[xmin, xmax)`.
    /// Internal primitive behind the public builder: construct histograms with
    /// [`Hist::reg`](crate::Hist::reg)`(nbins, xmin, xmax).double()`.
    pub(crate) fn new(nbins: i32, xmin: f64, xmax: f64) -> Self {
        let cells = usize::saturating_from(nbins.max(0)) + 2;
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nbins, xmin, xmax),
            yaxis: Axis::new("yaxis", 1, 0.0, 1.0),
            zaxis: Axis::new("zaxis", 1, 0.0, 1.0),
            ncells: i32::saturating_from(cells),
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: 0.0,
            tsumwx: 0.0,
            tsumwx2: 0.0,
            contents: vec![0.0; cells],
            sumw2: Vec::new(),
        }
    }

    /// The histogram's name (`fName`, its key when written).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The histogram's title (`fTitle`).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Fill the histogram with `x` (weight 1).
    pub(crate) fn fill(&mut self, x: f64) {
        self.fill_weight(x, 1.0);
    }

    /// Fill the histogram with `x` and weight `w`, updating bin contents,
    /// entry count, and the running statistics (ROOT `Fill` semantics: every
    /// fill increments `fEntries`; the moment sums accumulate for in-range
    /// fills only).
    ///
    /// The first fill with `w != 1` turns on per-bin error tracking, as ROOT's
    /// `Fill` does, so weighted errors are right without an explicit
    /// [`sumw2`](Self::sumw2) call.
    pub(crate) fn fill_weight(&mut self, x: f64, w: f64) {
        // Before the contents change: `sumw2` seeds from them, and every earlier
        // fill had unit weight (otherwise tracking would already be on).
        if w != 1.0 && self.sumw2.is_empty() {
            self.sumw2 = self.contents.iter().map(|c| c.abs()).collect();
        }
        let nbins = usize::saturating_from(self.xaxis.nbins.max(0));
        let bin = self.xaxis.find_bin(x);
        if let Some(c) = self.contents.get_mut(bin) {
            *c += w;
        }
        if let Some(s) = self.sumw2.get_mut(bin) {
            *s += w * w;
        }
        self.entries += 1.0;
        if (1..=nbins).contains(&bin) {
            self.tsumw += w;
            self.tsumw2 += w * w;
            self.tsumwx += w * x;
            self.tsumwx2 += w * x * x;
        }
    }

    pub(crate) fn read(r: &mut RBuffer<'_>, bin_content_type: BinContentType) -> Result<Self> {
        let (c, contents) = read_th1_object(r, bin_content_type)?;
        let cells = cell_count(&[c.xaxis.nbins])?;
        check_cells("TH1 contents", contents.len(), cells, false)?;
        check_cells("TH1 fSumw2", c.sumw2.len(), cells, true)?;
        Ok(Self {
            bin_content_type,
            name: c.name,
            title: c.title,
            xaxis: c.xaxis,
            yaxis: c.yaxis,
            zaxis: c.zaxis,
            ncells: c.ncells,
            entries: c.entries,
            tsumw: c.tsumw,
            tsumw2: c.tsumw2,
            tsumwx: c.tsumwx,
            tsumwx2: c.tsumwx2,
            contents,
            sumw2: c.sumw2,
        })
    }

    /// Bin contents excluding the under/overflow bins.
    #[must_use]
    pub fn values(&self) -> &[f64] {
        let n = self.contents.len();
        if n >= 2 {
            &self.contents[1..n - 1]
        } else {
            &self.contents
        }
    }

    /// The x-axis label (ROOT `fXaxis.fTitle`; scikit-hep `hist`'s axis `label`).
    #[must_use]
    pub fn x_label(&self) -> &str {
        &self.xaxis.title
    }

    /// Set the x-axis label. Round-trips through ROOT (`fXaxis.fTitle`).
    #[must_use]
    pub fn with_x_label(mut self, label: impl Into<String>) -> Self {
        self.xaxis.title = label.into();
        self
    }

    /// The y-axis label (ROOT `fYaxis.fTitle`; e.g. `"Events / 2 GeV"`).
    #[must_use]
    pub fn y_label(&self) -> &str {
        &self.yaxis.title
    }

    /// Set the y-axis label. Round-trips through ROOT (`fYaxis.fTitle`).
    #[must_use]
    pub fn with_y_label(mut self, label: impl Into<String>) -> Self {
        self.yaxis.title = label.into();
        self
    }

    /// The X-axis bin edges (`nbins + 1` values).
    #[must_use]
    pub fn edges(&self) -> Vec<f64> {
        self.xaxis.edges()
    }
}

impl ReadRoot for Hist1D {
    const CLASS: &'static str = "TH1";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        is_hist_class(object.class, "TH1").then(|| decode_th1(object.class, object.bytes))
    }
}
pub(crate) fn decode_th1(class: &str, object: &[u8]) -> Result<Hist1D> {
    Hist1D::read(&mut RBuffer::new(object), bin_content_type_of(class)?)
}
