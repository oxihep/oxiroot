//! 2-D histograms (`TH2D`, `TH2F`).
//!
//! Streamed layout: `TH2x{ Hist2D{ Hist1D{ … }, fScalefactor, fTsumwy, fTsumwy2,
//! fTsumwxy }, TArray }`. The inline `TArray` holds the `(nx+2)*(ny+2)` cells
//! with the x index varying fastest.

use crate::base::Moments;
use oxiroot_io_core::raw::TagReader;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_tarray,
    read_th1_base, unsupported_version,
};

use crate::record::Hist2D;

impl Hist2D {
    pub(crate) fn read(
        r: &mut RBuffer<'_>,
        bin_content_type: BinContentType,
        tags: &mut TagReader,
    ) -> Result<Self> {
        let th2x = r.read_version()?; // TH2x wrapper
        // Class version 1 (ROOT 1) streamed the Hist1D base, the bin array, then the
        // Hist2D members, with no Hist2D record of their own.
        if th2x.version < 2 {
            return Err(unsupported_version("TH2", th2x.version));
        }
        let th2 = r.read_version()?; // Hist2D wrapper (Hist1D base + Hist2D members)

        let c = read_th1_base(r, tags)?;
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
            entries: c.entries,
            moments: Moments {
                wy: tsumwy,
                wy2: tsumwy2,
                wxy: tsumwxy,
                ..c.moments
            },
            contents,
            sumw2: c.sumw2,
            functions: Vec::new(),
        })
    }

    /// Number of x bins (excluding flow).
    #[must_use]
    pub fn nx(&self) -> usize {
        self.xaxis.nbins
    }

    /// Number of y bins (excluding flow).
    #[must_use]
    pub fn ny(&self) -> usize {
        self.yaxis.nbins
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
    pub(crate) fn new(nx: usize, xlo: f64, xhi: f64, ny: usize, ylo: f64, yhi: f64) -> Self {
        let ncells = (nx + 2) * (ny + 2);
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            zaxis: Axis::new("zaxis", 1, 0.0, 1.0),
            entries: 0.0,
            moments: Moments::default(),
            contents: vec![0.0; ncells],
            sumw2: Vec::new(),
            functions: Vec::new(),
        }
    }
}

impl ReadRoot for Hist2D {
    const CLASS: &'static str = "TH2";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        is_hist_class(object.class, 2).then(|| {
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, bin_content_type_of(object.class)?, &mut tags)
        })
    }
}
