//! 1-D histograms (`TH1D`, `TH1F`).
//!
//! Streamed layout: `TH1x{ Hist1D{ … }, TArray }`. The `Hist1D` base is shared via
//! the crate's `base` module; the inline `TArray` holds the bin contents.

use crate::base::Moments;
use crate::graph::GraphFunction;
use oxiroot_io_core::raw::TagReader;
use oxiroot_io_core::{Object, ReadRoot};
use oxiroot_io_core::{RBuffer, Result};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_th1_object,
};

use crate::record::Hist1D;

impl Hist1D {
    /// Create an empty `TH1D` with `nbins` uniform bins over `[xmin, xmax)`.
    /// Internal primitive behind the public builder: construct histograms with
    /// [`Hist::reg`](crate::Hist::reg)`(nbins, xmin, xmax).double()`.
    pub(crate) fn new(nbins: usize, xmin: f64, xmax: f64) -> Self {
        let cells = nbins + 2;
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nbins, xmin, xmax),
            yaxis: Axis::new("yaxis", 1, 0.0, 1.0),
            zaxis: Axis::new("zaxis", 1, 0.0, 1.0),
            entries: 0.0,
            moments: Moments::default(),
            contents: vec![0.0; cells],
            sumw2: Vec::new(),
            functions: Vec::new(),
        }
    }

    /// Fill the histogram with `x` (weight 1).
    pub(crate) fn fill(&mut self, x: f64) {
        let nbins = self.xaxis.nbins;
        let bin = self.xaxis.find_bin(x);
        if let Some(c) = self.contents.get_mut(bin) {
            *c += 1.0;
        }
        if let Some(s) = self.sumw2.get_mut(bin) {
            *s += 1.0;
        }
        self.entries += 1.0;
        if (1..=nbins).contains(&bin) {
            self.moments.w += 1.0;
            self.moments.w2 += 1.0;
            self.moments.wx += x;
            self.moments.wx2 += x * x;
        }
    }

    /// Read a `TH1` at the cursor. `tags` is the class-tag map of the buffer;
    /// see [`read_th1_base`].
    pub(crate) fn read(
        r: &mut RBuffer<'_>,
        bin_content_type: BinContentType,
        tags: &mut TagReader,
    ) -> Result<Self> {
        let (c, contents) = read_th1_object(r, bin_content_type, tags)?;
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
            entries: c.entries,
            moments: c.moments,
            contents,
            sumw2: c.sumw2,
            functions: c.functions,
        })
    }

    /// The functions attached to the histogram (`fFunctions`): the curve a fit
    /// left with it, as ROOT's `TH1::Fit` does.
    #[must_use]
    pub fn functions(&self) -> &[GraphFunction] {
        &self.functions
    }

    /// Attach a function, stored with the histogram as a `TF1` and drawn over
    /// it by ROOT. Chainable; call it again to attach several.
    #[must_use]
    pub fn with_function(mut self, function: GraphFunction) -> Self {
        self.functions.push(function);
        self
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
        is_hist_class(object.class, 1).then(|| {
            let bin_content_type = bin_content_type_of(object.class)?;
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, bin_content_type, &mut tags)
        })
    }
}
