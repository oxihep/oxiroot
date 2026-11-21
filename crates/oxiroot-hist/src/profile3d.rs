//! `Profile3D` — a 3-D profile histogram. For each `(x, y, z)` cell it stores
//! the running sums to recover the mean of a fourth quantity `t`: `Σw`
//! (`bin_entries`), `Σ(w·t)` (`sums`, the `Hist3D` contents), and `Σ(w·t²)` (`sumt2`,
//! the `Hist3D` `fSumw2`). On disk it is a `TH3D` plus the profile members, exactly as
//! `Profile1D`/`Profile2D` extend `TH1D`/`TH2D`.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::skip_versioned;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, cell_count, check_cells, end_record, read_tarray, read_th1_base,
    unsupported_version,
};
use crate::profile1d::ErrorMode;

/// A 3-D profile histogram (ROOT `TProfile3D`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TProfile3D")]
pub struct Profile3D {
    /// Histogram name (`fName`).
    pub(crate) name: String,
    /// Histogram title (`fTitle`).
    pub(crate) title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis.
    pub(crate) yaxis: Axis,
    /// Z axis.
    pub(crate) zaxis: Axis,
    /// Total cells, including flow (`fNcells = (nx+2)*(ny+2)*(nz+2)`). Read via
    /// [`ncells`](Profile3D::ncells); `pub(crate)` so it cannot drift from the
    /// per-bin vectors.
    pub(crate) ncells: i32,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// Sum of weights (`fTsumw`).
    pub(crate) tsumw: f64,
    /// Sum of squared weights (`fTsumw2`).
    pub(crate) tsumw2: f64,
    /// Sum of `w·x` (`fTsumwx`).
    pub(crate) tsumwx: f64,
    /// Sum of `w·x²` (`fTsumwx2`).
    pub(crate) tsumwx2: f64,
    /// Sum of `w·y` (`fTsumwy`).
    pub(crate) tsumwy: f64,
    /// Sum of `w·y²` (`fTsumwy2`).
    pub(crate) tsumwy2: f64,
    /// Sum of `w·x·y` (`fTsumwxy`).
    pub(crate) tsumwxy: f64,
    /// Sum of `w·z` (`fTsumwz`).
    pub(crate) tsumwz: f64,
    /// Sum of `w·z²` (`fTsumwz2`).
    pub(crate) tsumwz2: f64,
    /// Sum of `w·x·z` (`fTsumwxz`).
    pub(crate) tsumwxz: f64,
    /// Sum of `w·y·z` (`fTsumwyz`).
    pub(crate) tsumwyz: f64,
    /// Per-cell `Σ(w·t)` (the `Hist3D` contents, `fArray`); length `ncells`.
    pub(crate) sums: Vec<f64>,
    /// Per-cell `Σ(w·t²)` (the `Hist3D` `fSumw2`); length `ncells`.
    pub(crate) sumt2: Vec<f64>,
    /// Per-cell `Σw` (`fBinEntries`); length `ncells`.
    pub(crate) bin_entries: Vec<f64>,
    /// Error computation mode (`fErrorMode`).
    pub error_mode: ErrorMode,
    /// Lower `t` accept bound (`fTmin`; `0` = no restriction when `tmin == tmax`).
    pub tmin: f64,
    /// Upper `t` accept bound (`fTmax`).
    pub tmax: f64,
    /// Sum of `w·t` over in-range fills (`fTsumwt`).
    pub(crate) tsumwt: f64,
    /// Sum of `w·t²` over in-range fills (`fTsumwt2`).
    pub(crate) tsumwt2: f64,
    /// Per-cell `Σw²` (`fBinSumw2`); empty unless weighted-error tracking is on.
    pub(crate) bin_sumw2: Vec<f64>,
}

impl Profile3D {
    /// Total cells including the flow bins (`fNcells`), derived from the per-bin
    /// vectors so it cannot disagree with them.
    #[must_use]
    pub fn ncells(&self) -> i32 {
        i32::saturating_from(self.bin_entries.len())
    }

    /// Create an empty `Profile3D` with uniform x/y/z bins and no t
    /// restriction. Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nx, ..).reg(ny, ..).reg(nz, ..).mean().to_root()?`.
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
        let ncells = usize::saturating_from((nx.max(0) + 2) * (ny.max(0) + 2) * (nz.max(0) + 2));
        Self {
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            zaxis: Axis::new("zaxis", nz, zlo, zhi),
            ncells: i32::saturating_from(ncells),
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
            sums: vec![0.0; ncells],
            sumt2: vec![0.0; ncells],
            bin_entries: vec![0.0; ncells],
            error_mode: ErrorMode::Mean,
            tmin: 0.0,
            tmax: 0.0,
            tsumwt: 0.0,
            tsumwt2: 0.0,
            bin_sumw2: Vec::new(),
        }
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

    /// Profiled `t` value per in-range cell as `values[ix][iy][iz]`:
    /// `Σ(w·t) / Σw`, or `0` where a cell has no entries.
    #[must_use]
    pub fn values(&self) -> Vec<Vec<Vec<f64>>> {
        let (nx, ny, nz) = (self.nx(), self.ny(), self.nz());
        let (sx, sy) = (nx + 2, ny + 2);
        (1..=nx)
            .map(|ix| {
                (1..=ny)
                    .map(|iy| {
                        (1..=nz)
                            .map(|iz| self.bin_value(ix + sx * (iy + sy * iz)))
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }

    pub(crate) fn read(r: &mut RBuffer<'_>) -> Result<Self> {
        // fBinSumw2 arrived in class version 7 (ROOT 5.24). The class was added
        // at version 6 (ROOT 5.12), which already had the other members.
        let tp = r.read_version()?; // Profile3D wrapper
        let version = tp.version;
        if version < 6 {
            return Err(unsupported_version("TProfile3D", version));
        }
        let _th3d = r.read_version()?; // TH3D wrapper
        let th3 = r.read_version()?; // Hist3D wrapper

        let c = read_th1_base(r, None)?;
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

        let sums = read_tarray(r, BinContentType::F64)?; // TH3D TArrayD = Σ(w·t)
        let bin_entries = read_tarray(r, BinContentType::F64)?;
        let error_mode = ErrorMode::from_code(r.be_i32()?)?;
        let tmin = r.be_f64()?;
        let tmax = r.be_f64()?;
        let tsumwt = r.be_f64()?;
        let tsumwt2 = r.be_f64()?;
        let bin_sumw2 = if version >= 7 {
            read_tarray(r, BinContentType::F64)?
        } else {
            Vec::new() // no fBinSumw2: weights were not tracked
        };
        end_record(r, &tp, "TProfile3D")?;

        let cells = cell_count(&[c.xaxis.nbins, c.yaxis.nbins, c.zaxis.nbins])?;
        check_cells("TProfile3D sums", sums.len(), cells, false)?;
        check_cells("TProfile3D fBinEntries", bin_entries.len(), cells, false)?;
        check_cells("TProfile3D fSumw2", c.sumw2.len(), cells, true)?;
        check_cells("TProfile3D fBinSumw2", bin_sumw2.len(), cells, true)?;

        Ok(Self {
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
            sums,
            sumt2: c.sumw2,
            bin_entries,
            error_mode,
            tmin,
            tmax,
            tsumwt,
            tsumwt2,
            bin_sumw2,
        })
    }
}

impl ReadRoot for Profile3D {
    const CLASS: &'static str = "TProfile3D";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| Self::read(&mut RBuffer::new(object.bytes)))
    }
}
