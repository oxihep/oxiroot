//! 3-D histograms (`TH3D`, `TH3F`).
//!
//! Streamed layout: `TH3x{ Hist3D{ Hist1D{ … }, TAtt3D, fTsumwy, fTsumwy2, fTsumwxy,
//! fTsumwz, fTsumwz2, fTsumwxz, fTsumwyz }, TArray }`. `TAtt3D` is an empty base
//! (skipped via its byte count); the inline `TArray` holds the
//! `(nx+2)*(ny+2)*(nz+2)` cells with x fastest, then y, then z.

use crate::base::Moments;
use oxiroot_io_core::raw::skip_versioned;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_tarray,
    read_th1_base, unsupported_version,
};

use crate::record::Hist3D;

/// Read a `TH3` record, the base of every 3-D histogram and of
/// `TProfile3D`: the `TH1` base, then the seven sums over `y` and `z`
/// (`fTsumwy`, `fTsumwy2`, `fTsumwxy`, `fTsumwz`, `fTsumwz2`, `fTsumwxz`,
/// `fTsumwyz`), in that order. The buffer is left at the record's end.
pub(crate) fn read_th3_base(r: &mut RBuffer<'_>) -> Result<(crate::base::HistBase, [f64; 7])> {
    let th3 = r.read_version()?;
    let base = read_th1_base(r, None)?;
    skip_versioned(r)?; // TAtt3D base (empty)
    let mut sums = [0.0; 7];
    for sum in &mut sums {
        *sum = r.be_f64()?;
    }
    let end = th3
        .end
        .ok_or_else(|| Error::Format("TH3 record has no byte count".into()))?;
    r.seek(end)?;
    Ok((base, sums))
}

impl Hist3D {
    pub(crate) fn read(r: &mut RBuffer<'_>, bin_content_type: BinContentType) -> Result<Self> {
        let th3x = r.read_version()?; // TH3x wrapper
        // Class version 1 (ROOT 1) streamed the Hist1D base, the bin array, then the
        // Hist3D members, with no Hist3D record of their own.
        if th3x.version < 2 {
            return Err(unsupported_version("TH3", th3x.version));
        }
        let (c, [tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz]) = read_th3_base(r)?;
        let contents = read_tarray(r, bin_content_type)?;

        let cells = cell_count(&[c.xaxis.nbins, c.yaxis.nbins, c.zaxis.nbins])?;
        check_cells("TH3 contents", contents.len(), cells, false)?;
        check_cells("TH3 fSumw2", c.sumw2.len(), cells, true)?;

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
                wz: tsumwz,
                wz2: tsumwz2,
                wxz: tsumwxz,
                wyz: tsumwyz,
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

    /// Number of z bins (excluding flow).
    #[must_use]
    pub fn nz(&self) -> usize {
        self.zaxis.nbins
    }

    /// Bin contents excluding flow as `values[ix][iy][iz]`, matching uproot's
    /// `values(flow=False)`. Cell `(ix, iy, iz)` is stored at
    /// `ix + (nx+2)*(iy + (ny+2)*iz)` (indices include the underflow bin at 0).
    #[must_use]
    pub fn values(&self) -> Vec<Vec<Vec<f64>>> {
        let (nx, ny, nz) = (self.nx(), self.ny(), self.nz());
        let (sx, sy) = (nx + 2, ny + 2);
        (1..=nx)
            .map(|ix| {
                (1..=ny)
                    .map(|iy| {
                        (1..=nz)
                            .map(|iz| self.contents[ix + sx * (iy + sy * iz)])
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }

    /// Create an empty `TH3D` with uniform axes (`nx` bins over `[xlo, xhi)`,
    /// etc.). Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nx, xlo, xhi).reg(ny, ylo, yhi).reg(nz, zlo, zhi).double()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        nx: usize,
        xlo: f64,
        xhi: f64,
        ny: usize,
        ylo: f64,
        yhi: f64,
        nz: usize,
        zlo: f64,
        zhi: f64,
    ) -> Self {
        let ncells = (nx + 2) * (ny + 2) * (nz + 2);
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            zaxis: Axis::new("zaxis", nz, zlo, zhi),
            entries: 0.0,
            moments: Moments::default(),
            contents: vec![0.0; ncells],
            sumw2: Vec::new(),
            functions: Vec::new(),
        }
    }
}

impl ReadRoot for Hist3D {
    const CLASS: &'static str = "TH3";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        is_hist_class(object.class, 3).then(|| decode_th3(object.class, object.bytes))
    }
}
pub(crate) fn decode_th3(class: &str, object: &[u8]) -> Result<Hist3D> {
    Hist3D::read(&mut RBuffer::new(object), bin_content_type_of(class)?)
}
