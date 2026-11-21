//! 3-D histograms (`TH3D`, `TH3F`).
//!
//! Streamed layout: `TH3x{ Hist3D{ Hist1D{ … }, TAtt3D, fTsumwy, fTsumwy2, fTsumwxy,
//! fTsumwz, fTsumwz2, fTsumwxz, fTsumwyz }, TArray }`. `TAtt3D` is an empty base
//! (skipped via its byte count); the inline `TArray` holds the
//! `(nx+2)*(ny+2)*(nz+2)` cells with x fastest, then y, then z.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::skip_versioned;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, bin_content_type_of, cell_count, check_cells, is_hist_class, read_tarray,
    read_th1_base, unsupported_version,
};

/// A 3-D classic histogram (`TH3D` or `TH3F`); contents are widened to `f64`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TH3", alias = "TH3D", alias = "TH3F")]
pub struct Hist3D {
    /// On-disk [`BinContentType`] (the class suffix); read the class name via
    /// [`class_name`](Hist3D::class_name).
    pub(crate) bin_content_type: BinContentType,
    /// Histogram name (`fName`).
    pub name: String,
    /// Histogram title (`fTitle`).
    pub title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis.
    pub(crate) yaxis: Axis,
    /// Z axis.
    pub(crate) zaxis: Axis,
    /// Total cells, including flow (`fNcells = (nx+2)*(ny+2)*(nz+2)`). Read via
    /// [`ncells`](Hist3D::ncells); `pub(crate)` so it cannot drift from `contents`.
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
    /// Sum of weight*z (`fTsumwz`).
    pub(crate) tsumwz: f64,
    /// Sum of weight*z^2 (`fTsumwz2`).
    pub(crate) tsumwz2: f64,
    /// Sum of weight*x*z (`fTsumwxz`).
    pub(crate) tsumwxz: f64,
    /// Sum of weight*y*z (`fTsumwyz`).
    pub(crate) tsumwyz: f64,
    /// Bin contents including flow (length `ncells`, x fastest then y then z).
    pub(crate) contents: Vec<f64>,
    /// Per-bin sum of squared weights (`fSumw2`); empty until error tracking is
    /// turned on by [`Hist3D::sumw2`], [`Hist3D::scale`], or a weighted fill (see
    /// [`Hist1D::fill_weight`](crate::Hist1D::fill_weight)).
    pub(crate) sumw2: Vec<f64>,
}

impl Hist3D {
    pub(crate) fn read(r: &mut RBuffer<'_>, bin_content_type: BinContentType) -> Result<Self> {
        let th3x = r.read_version()?; // TH3x wrapper
        // Class version 1 (ROOT 1) streamed the Hist1D base, the bin array, then the
        // Hist3D members, with no Hist3D record of their own.
        if th3x.version < 2 {
            return Err(unsupported_version("TH3", th3x.version));
        }
        let th3 = r.read_version()?; // Hist3D wrapper

        let c = read_th1_base(r)?;
        skip_versioned(r)?; // TAtt3D base (empty)
        let tsumwy = r.be_f64()?;
        let tsumwy2 = r.be_f64()?;
        let tsumwxy = r.be_f64()?;
        let tsumwz = r.be_f64()?;
        let tsumwz2 = r.be_f64()?;
        let tsumwxz = r.be_f64()?;
        let tsumwyz = r.be_f64()?;

        let end = th3
            .end
            .ok_or_else(|| Error::Format("TH3 record has no byte count".into()))?;
        r.seek(end)?;
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
            ncells: c.ncells,
            entries: c.entries,
            tsumw: c.tsumw,
            tsumw2: c.tsumw2,
            tsumwx: c.tsumwx,
            tsumwx2: c.tsumwx2,
            tsumwy,
            tsumwy2,
            tsumwxy,
            tsumwz,
            tsumwz2,
            tsumwxz,
            tsumwyz,
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

    /// Number of z bins (excluding flow).
    #[must_use]
    pub fn nz(&self) -> usize {
        usize::saturating_from(self.zaxis.nbins.max(0))
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
        nx: i32,
        xlo: f64,
        xhi: f64,
        ny: i32,
        ylo: f64,
        yhi: f64,
        nz: i32,
        zlo: f64,
        zhi: f64,
    ) -> Self {
        let ncells = (nx.max(0) + 2) * (ny.max(0) + 2) * (nz.max(0) + 2);
        Self {
            bin_content_type: BinContentType::F64,
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            zaxis: Axis::new("zaxis", nz, zlo, zhi),
            ncells,
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: 0.0,
            tsumwx: 0.0,
            tsumwx2: 0.0,
            tsumwy: 0.0,
            tsumwy2: 0.0,
            tsumwxy: 0.0,
            tsumwz: 0.0,
            tsumwz2: 0.0,
            tsumwxz: 0.0,
            tsumwyz: 0.0,
            contents: vec![0.0; usize::saturating_from(ncells.max(0))],
            sumw2: Vec::new(),
        }
    }

    /// Fill `(x, y, z)` with unit weight.
    pub fn fill(&mut self, x: f64, y: f64, z: f64) {
        self.fill_weight(x, y, z, 1.0);
    }

    /// Fill `(x, y, z)` with weight `w`, matching ROOT's `Hist3D::Fill`: every fill
    /// counts toward `fEntries`, the cell (including flow) is incremented, and
    /// the moment sums accumulate only when all three coordinates are in range.
    pub fn fill_weight(&mut self, x: f64, y: f64, z: f64, w: f64) {
        // Before the contents change; see `Hist1D::fill_weight`.
        if w != 1.0 && self.sumw2.is_empty() {
            self.track_sumw2();
        }
        let (nx, ny, nz) = (self.nx(), self.ny(), self.nz());
        let binx = self.xaxis.find_bin(x);
        let biny = self.yaxis.find_bin(y);
        let binz = self.zaxis.find_bin(z);
        let bin = binx + (nx + 2) * (biny + (ny + 2) * binz);
        if let Some(c) = self.contents.get_mut(bin) {
            *c += w;
        }
        if let Some(s) = self.sumw2.get_mut(bin) {
            *s += w * w;
        }
        self.entries += 1.0;

        let in_range =
            (1..=nx).contains(&binx) && (1..=ny).contains(&biny) && (1..=nz).contains(&binz);
        if in_range {
            self.tsumw += w;
            self.tsumw2 += w * w;
            self.tsumwx += w * x;
            self.tsumwx2 += w * x * x;
            self.tsumwy += w * y;
            self.tsumwy2 += w * y * y;
            self.tsumwxy += w * x * y;
            self.tsumwz += w * z;
            self.tsumwz2 += w * z * z;
            self.tsumwxz += w * x * z;
            self.tsumwyz += w * y * z;
        }
    }

    /// Mean of the x projection (`fTsumwx / fTsumw`), 0 when empty.
    #[must_use]
    pub fn mean_x(&self) -> f64 {
        if self.tsumw == 0.0 {
            0.0
        } else {
            self.tsumwx / self.tsumw
        }
    }

    /// Mean of the y projection (`fTsumwy / fTsumw`), 0 when empty.
    #[must_use]
    pub fn mean_y(&self) -> f64 {
        if self.tsumw == 0.0 {
            0.0
        } else {
            self.tsumwy / self.tsumw
        }
    }

    /// Mean of the z projection (`fTsumwz / fTsumw`), 0 when empty.
    #[must_use]
    pub fn mean_z(&self) -> f64 {
        if self.tsumw == 0.0 {
            0.0
        } else {
            self.tsumwz / self.tsumw
        }
    }
}

impl ReadRoot for Hist3D {
    const CLASS: &'static str = "TH3";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        is_hist_class(object.class, "TH3").then(|| decode_th3(object.class, object.bytes))
    }
}
pub(crate) fn decode_th3(class: &str, object: &[u8]) -> Result<Hist3D> {
    Hist3D::read(&mut RBuffer::new(object), bin_content_type_of(class)?)
}
