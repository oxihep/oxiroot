//! `Profile1D` — a 1-D profile histogram.
//!
//! Streamed layout: `Profile1D{ TH1D{ … }, fBinEntries(TArrayD), fErrorMode,
//! fYmin, fYmax, fTsumwy, fTsumwy2, fBinSumw2(TArrayD) }`. The `TH1D` base's
//! bin contents are the per-bin sums of y; `fBinEntries` is the per-bin count.
//! The profiled value of a bin is `sum / entries`.
//!
//! Older class versions lack the trailing members: `fTsumwy`/`fTsumwy2` arrived
//! in version 4 (ROOT 4) and `fBinSumw2` in version 6 (ROOT 5.24). Versions 2
//! onwards are read; version 1 (ROOT 1) stored `fYmin`/`fYmax` as floats.

use crate::base::Moments;
use oxiroot_io_core::raw::TagReader;
use oxiroot_io_core::{Object, ReadRoot};
use oxiroot_io_core::{RBuffer, Result};

use crate::axis::Axis;
use crate::base::{
    BinContentType, cell_count, check_cells, end_record, in_range_sum, read_tarray,
    read_th1_object, unsupported_version,
};

/// How a profile's per-bin error bar is computed (ROOT's `fErrorMode`). Shared
/// by [`Profile1D`], [`crate::Profile2D`], and [`crate::Profile3D`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ErrorMode {
    /// `kERRORMEAN` (0): error on the mean, `spread / sqrt(N_eff)` (the default).
    #[default]
    Mean,
    /// `kERRORSPREAD` (1): the RMS spread itself.
    Spread,
    /// `kERRORSPREADI` (2): like `Mean`, but an empty spread uses the
    /// uniform-bin estimate `1 / sqrt(12 N_eff)`.
    SpreadI,
    /// `kERRORSPREADG` (3): the weighted error `1 / sqrt(Σw)`.
    SpreadG,
}

impl ErrorMode {
    /// The mode a ROOT `fErrorMode` code names (ROOT writes `0..=3`).
    ///
    /// # Errors
    ///
    /// [`Error::Format`](oxiroot_io_core::Error::Format) for any other code.
    pub fn from_code(code: i32) -> Result<Self> {
        Ok(match code {
            0 => Self::Mean,
            1 => Self::Spread,
            2 => Self::SpreadI,
            3 => Self::SpreadG,
            other => {
                return Err(oxiroot_io_core::Error::Format(format!(
                    "unknown profile error mode {other}"
                )));
            }
        })
    }

    /// The ROOT `fErrorMode` code written to disk.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Mean => 0,
            Self::Spread => 1,
            Self::SpreadI => 2,
            Self::SpreadG => 3,
        }
    }
}

/// A 1-D profile histogram (`Profile1D`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TProfile")]
pub struct Profile1D {
    /// Histogram name (`fName`).
    pub(crate) name: String,
    /// Histogram title (`fTitle`).
    pub(crate) title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// The sums over the fills (`fTsumw` and the rest).
    pub(crate) moments: Moments,
    /// Per-bin sums of weight*y (the `TH1D` base contents, length `ncells`).
    pub(crate) sums: Vec<f64>,
    /// Per-bin sums of weight*y^2 (the `Hist1D` base `fSumw2`, length `ncells`).
    pub(crate) sumy2: Vec<f64>,
    /// Per-bin entry counts / sums of weight (`fBinEntries`, length `ncells`).
    pub(crate) bin_entries: Vec<f64>,
    /// Error computation mode (`fErrorMode`).
    pub(crate) error_mode: ErrorMode,
    /// Lower y limit (`fYmin`).
    pub(crate) ymin: f64,
    /// Upper y limit (`fYmax`).
    pub(crate) ymax: f64,
    /// Per-bin sum of squared weights (`fBinSumw2`), possibly empty.
    pub(crate) bin_sumw2: Vec<f64>,
}

impl Profile1D {
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

    /// Lower y limit (`fYmin`).
    #[must_use]
    pub fn ymin(&self) -> f64 {
        self.ymin
    }

    /// Set [`ymin`](Self::ymin).
    #[must_use]
    pub fn with_ymin(mut self, ymin: f64) -> Self {
        self.ymin = ymin;
        self
    }

    /// Upper y limit (`fYmax`).
    #[must_use]
    pub fn ymax(&self) -> f64 {
        self.ymax
    }

    /// Set [`ymax`](Self::ymax).
    #[must_use]
    pub fn with_ymax(mut self, ymax: f64) -> Self {
        self.ymax = ymax;
        self
    }
}

impl Profile1D {
    pub(crate) fn read(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
        let tprofile = r.read_version()?; // Profile1D wrapper
        let version = tprofile.version;
        if version < 2 {
            return Err(unsupported_version("TProfile", version));
        }

        // The TH1D base: its own wrapper, the Hist1D base, and the TArrayD sums.
        let (core, sums) = read_th1_object(r, BinContentType::F64, tags)?;

        let bin_entries = read_tarray(r, BinContentType::F64)?;
        let error_mode = ErrorMode::from_code(r.be_i32()?)?;
        let ymin = r.be_f64()?;
        let ymax = r.be_f64()?;
        let stored_y_sums = if version >= 4 {
            Some((r.be_f64()?, r.be_f64()?)) // fTsumwy, fTsumwy2
        } else {
            None
        };
        let bin_sumw2 = if version >= 6 {
            read_tarray(r, BinContentType::F64)?
        } else {
            Vec::new() // no fBinSumw2: weights were not tracked
        };
        end_record(r, &tprofile, "TProfile")?;

        let cells = cell_count(&[core.xaxis.nbins])?;
        check_cells("TProfile sums", sums.len(), cells, false)?;
        check_cells("TProfile fBinEntries", bin_entries.len(), cells, false)?;
        check_cells("TProfile fSumw2", core.sumw2.len(), cells, true)?;
        check_cells("TProfile fBinSumw2", bin_sumw2.len(), cells, true)?;

        // Before version 4 the y sums were not stored: take them from the bins,
        // as ROOT's GetStats does for a profile without stored statistics.
        let (tsumwy, tsumwy2) = stored_y_sums.unwrap_or_else(|| {
            let nbins = [core.xaxis.nbins];
            (
                in_range_sum(&sums, &nbins),
                in_range_sum(&core.sumw2, &nbins),
            )
        });

        Ok(Self {
            name: core.name,
            title: core.title,
            xaxis: core.xaxis,
            entries: core.entries,
            moments: Moments {
                wy: tsumwy,
                wy2: tsumwy2,
                ..core.moments
            },
            sums,
            sumy2: core.sumw2,
            bin_entries,
            error_mode,
            ymin,
            ymax,
            bin_sumw2,
        })
    }

    /// The profiled value per bin (excluding flow): `sum / entries`, or 0 where
    /// a bin has no entries. Matches ROOT/uproot `Profile1D::values()`.
    #[must_use]
    pub fn values(&self) -> Vec<f64> {
        let n = self.sums.len();
        if n < 2 {
            return Vec::new();
        }
        (1..n - 1).map(|i| self.bin_value(i)).collect()
    }

    /// The X-axis bin edges (`nbins + 1` values).
    #[must_use]
    pub fn edges(&self) -> Vec<f64> {
        self.xaxis.edges()
    }

    /// Create an empty `Profile1D` with `nbins` uniform x bins over `[xlo, xhi)`
    /// and no y restriction. Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nbins, xlo, xhi).mean().to_root()?`.
    pub(crate) fn new(nbins: usize, xlo: f64, xhi: f64) -> Self {
        let ncells = nbins + 2;
        Self {
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new(nbins, xlo, xhi).named("xaxis"),
            entries: 0.0,
            moments: Moments::default(),
            sums: vec![0.0; ncells],
            sumy2: vec![0.0; ncells],
            bin_entries: vec![0.0; ncells],
            error_mode: ErrorMode::Mean,
            ymin: 0.0,
            ymax: 0.0,
            bin_sumw2: Vec::new(),
        }
    }
}

impl ReadRoot for Profile1D {
    const CLASS: &'static str = "TProfile";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| {
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, &mut tags)
        })
    }
}
