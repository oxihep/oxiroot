//! 2-D histograms (`TH2D`, `TH2F`).
//!
//! Streamed layout: `TH2x{ Hist2D{ Hist1D{ … }, fScalefactor, fTsumwy, fTsumwy2,
//! fTsumwxy }, TArray }`. The inline `TArray` holds the `(nx+2)*(ny+2)` cells
//! with the x index varying fastest.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_tarray,
    read_th1_base, unsupported_version,
};

/// A 2-D classic histogram (`TH2D` or `TH2F`); contents are widened to `f64`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TH2", alias = "TH2D", alias = "TH2F")]
pub struct Hist2D {
    /// On-disk [`BinContentType`] (the class suffix); read the class name via
    /// [`class_name`](Hist2D::class_name).
    pub(crate) bin_content_type: BinContentType,
    /// Histogram name (`fName`).
    pub name: String,
    /// Histogram title (`fTitle`).
    pub title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis.
    pub(crate) yaxis: Axis,
    /// Z axis (degenerate for 2-D).
    pub(crate) zaxis: Axis,
    /// Total cells, including flow (`fNcells = (nx+2)*(ny+2)`). Read via
    /// [`ncells`](Hist2D::ncells); `pub(crate)` so it cannot drift from `contents`.
    pub(crate) ncells: i32,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// Sum of weights (`fTsumw`).
    pub(crate) tsumw: f64,
    /// Sum of weight^2 (`fTsumw2`).
    pub(crate) tsumw2: f64,
    /// Sum of weight*x (`fTsumwx`).
    pub(crate) tsumwx: f64,
    /// Sum of weight*x^2 (`fTsumwx2`).
    pub(crate) tsumwx2: f64,
    /// Sum of weight*y (`fTsumwy`).
    pub(crate) tsumwy: f64,
    /// Sum of weight*y^2 (`fTsumwy2`).
    pub(crate) tsumwy2: f64,
    /// Sum of weight*x*y (`fTsumwxy`).
    pub(crate) tsumwxy: f64,
    /// Bin contents including flow (length `ncells`, x fastest).
    pub(crate) contents: Vec<f64>,
    /// Per-bin sum of squared weights (`fSumw2`); empty when the histogram
    /// does not track errors.
    pub(crate) sumw2: Vec<f64>,
}

impl Hist2D {
    pub(crate) fn read(r: &mut RBuffer<'_>, bin_content_type: BinContentType) -> Result<Self> {
        let th2x = r.read_version()?; // TH2x wrapper
        // Class version 1 (ROOT 1) streamed the Hist1D base, the bin array, then the
        // Hist2D members, with no Hist2D record of their own.
        if th2x.version < 2 {
            return Err(unsupported_version("TH2", th2x.version));
        }
        let th2 = r.read_version()?; // Hist2D wrapper (Hist1D base + Hist2D members)

        let c = read_th1_base(r)?;
        let _scalefactor = r.be_f64()?;
        let tsumwy = r.be_f64()?;
        let tsumwy2 = r.be_f64()?;
        let tsumwxy = r.be_f64()?;

        let end = th2
            .end
            .ok_or_else(|| Error::Format("TH2 record has no byte count".into()))?;
        r.seek(end)?;
        let contents = read_tarray(r, bin_content_type)?;

        let cells = cell_count(&[c.xaxis.nbins, c.yaxis.nbins])?;
        check_cells("TH2 contents", contents.len(), cells, false)?;
        check_cells("TH2 fSumw2", c.sumw2.len(), cells, true)?;

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
            tsumwy,
            tsumwy2,
            tsumwxy,
            contents,
            sumw2: c.sumw2,
        })
    }

    /// Number of x bins (excluding flow).
    #[must_use]
    pub fn nx(&self) -> usize {
        usize::saturating_from(self.xaxis.nbins.max(0))
    }

    /// Number of y bins (excluding flow).
    #[must_use]
    pub fn ny(&self) -> usize {
        usize::saturating_from(self.yaxis.nbins.max(0))
    }

    /// Bin contents excluding flow as `values[ix][iy]` (`nx` rows, `ny` cols),
    /// matching uproot's `values(flow=False)`. Cell `(ix, iy)` is stored at
    /// `ix + (nx + 2) * iy` (indices include the underflow bin at 0).
    #[must_use]
    pub fn values(&self) -> Vec<Vec<f64>> {
        let (nx, ny) = (self.nx(), self.ny());
        let stride = nx + 2;
        (1..=nx)
            .map(|ix| (1..=ny).map(|iy| self.contents[ix + stride * iy]).collect())
            .collect()
    }

    /// Create an empty `TH2D` with uniform axes: `nx` bins over `[xlo, xhi)`
    /// and `ny` bins over `[ylo, yhi)`. Internal primitive behind the public
    /// builder: [`Hist::reg`](crate::Hist::reg)`(nx, xlo, xhi).reg(ny, ylo, yhi).double()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(nx: i32, xlo: f64, xhi: f64, ny: i32, ylo: f64, yhi: f64) -> Self {
        let ncells = (nx.max(0) + 2) * (ny.max(0) + 2);
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            zaxis: Axis::new("zaxis", 1, 0.0, 1.0),
            ncells,
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: 0.0,
            tsumwx: 0.0,
            tsumwx2: 0.0,
            tsumwy: 0.0,
            tsumwy2: 0.0,
            tsumwxy: 0.0,
            contents: vec![0.0; usize::saturating_from(ncells.max(0))],
            sumw2: Vec::new(),
        }
    }
}

impl ReadRoot for Hist2D {
    const CLASS: &'static str = "TH2";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        is_hist_class(object.class, "TH2").then(|| decode_th2(object.class, object.bytes))
    }
}
pub(crate) fn decode_th2(class: &str, object: &[u8]) -> Result<Hist2D> {
    Hist2D::read(&mut RBuffer::new(object), bin_content_type_of(class)?)
}
