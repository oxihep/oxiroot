//! `Profile3D` — a 3-D profile histogram. For each `(x, y, z)` cell it stores
//! the running sums to recover the mean of a fourth quantity `t`: `Σw`
//! (`bin_entries`), `Σ(w·t)` (`sums`, the `Hist3D` contents), and `Σ(w·t²)` (`sumt2`,
//! the `Hist3D` `fSumw2`). On disk it is a `TH3D` plus the profile members, exactly as
//! `Profile1D`/`Profile2D` extend `TH1D`/`TH2D`.

use crate::base::Moments;
use oxiroot_io_core::raw::TagReader;
use oxiroot_io_core::{Object, ReadRoot};
use oxiroot_io_core::{RBuffer, Result};

use crate::axis::Axis;
use crate::base::{
    BinContentType, cell_count, check_cells, end_record, read_tarray, unsupported_version,
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
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// The sums over the fills (`fTsumw` and the rest).
    pub(crate) moments: Moments,
    /// Per-cell `Σ(w·t)` (the `Hist3D` contents, `fArray`); length `ncells`.
    pub(crate) sums: Vec<f64>,
    /// Per-cell `Σ(w·t²)` (the `Hist3D` `fSumw2`); length `ncells`.
    pub(crate) sumt2: Vec<f64>,
    /// Per-cell `Σw` (`fBinEntries`); length `ncells`.
    pub(crate) bin_entries: Vec<f64>,
    /// Error computation mode (`fErrorMode`).
    pub(crate) error_mode: ErrorMode,
    /// Lower `t` accept bound (`fTmin`; `0` = no restriction when `tmin == tmax`).
    pub(crate) tmin: f64,
    /// Upper `t` accept bound (`fTmax`).
    pub(crate) tmax: f64,
    /// Sum of `w·t` over in-range fills (`fTsumwt`).
    pub(crate) tsumwt: f64,
    /// Sum of `w·t²` over in-range fills (`fTsumwt2`).
    pub(crate) tsumwt2: f64,
    /// Per-cell `Σw²` (`fBinSumw2`); empty unless weighted-error tracking is on.
    pub(crate) bin_sumw2: Vec<f64>,
}

impl Profile3D {
    /// Error computation mode (`fErrorMode`).
    #[must_use]
    pub fn error_mode(&self) -> ErrorMode {
        self.error_mode
    }

    /// Set [`error_mode`](Self::error_mode).
    #[must_use]
    pub fn with_error_mode(mut self, error_mode: ErrorMode) -> Self {
        self.error_mode = error_mode;
        self
    }

    /// Lower `t` accept bound (`fTmin`; `0` = no restriction when `tmin == tmax`).
    #[must_use]
    pub fn tmin(&self) -> f64 {
        self.tmin
    }

    /// Set [`tmin`](Self::tmin).
    #[must_use]
    pub fn with_tmin(mut self, tmin: f64) -> Self {
        self.tmin = tmin;
        self
    }

    /// Upper `t` accept bound (`fTmax`).
    #[must_use]
    pub fn tmax(&self) -> f64 {
        self.tmax
    }

    /// Set [`tmax`](Self::tmax).
    #[must_use]
    pub fn with_tmax(mut self, tmax: f64) -> Self {
        self.tmax = tmax;
        self
    }
}

impl Profile3D {
    /// Total cells including the flow bins (`fNcells`), derived from the per-bin
    /// vectors so it cannot disagree with them.
    #[must_use]
    pub fn ncells(&self) -> usize {
        self.bin_entries.len()
    }

    /// Create an empty `Profile3D` with uniform x/y/z bins and no t
    /// restriction. Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nx, ..).reg(ny, ..).reg(nz, ..).mean().to_root()?`.
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
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new(nx, xlo, xhi).named("xaxis"),
            yaxis: Axis::new(ny, ylo, yhi).named("yaxis"),
            zaxis: Axis::new(nz, zlo, zhi).named("zaxis"),
            entries: 0.0,
            moments: Moments::default(),
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

    pub(crate) fn read(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
        // fBinSumw2 arrived in class version 7 (ROOT 5.24). The class was added
        // at version 6 (ROOT 5.12), which already had the other members.
        let tp = r.read_version()?; // Profile3D wrapper
        let version = tp.version;
        if version < 6 {
            return Err(unsupported_version("TProfile3D", version));
        }
        let _th3d = r.read_version()?; // TH3D wrapper
        let (c, [tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz]) =
            crate::hist3d::read_th3_base(r, tags)?;

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
        (object.class == Self::CLASS).then(|| {
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, &mut tags)
        })
    }
}
