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

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{FileReader, RBuffer, Result};

use crate::axis::Axis;
use crate::base::{
    BinContentType, cell_count, check_cells, end_record, in_range_sum, object_bytes,
    object_bytes_in, read_tarray, read_th1_object, unsupported_version,
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
    /// Map a ROOT `fErrorMode` code; an unrecognized code falls back to
    /// [`Mean`](ErrorMode::Mean) (ROOT only ever writes `0..=3`).
    #[must_use]
    pub fn from_code(code: i32) -> ErrorMode {
        match code {
            1 => ErrorMode::Spread,
            2 => ErrorMode::SpreadI,
            3 => ErrorMode::SpreadG,
            _ => ErrorMode::Mean,
        }
    }

    /// The ROOT `fErrorMode` code written to disk.
    #[must_use]
    pub fn to_code(self) -> i32 {
        match self {
            ErrorMode::Mean => 0,
            ErrorMode::Spread => 1,
            ErrorMode::SpreadI => 2,
            ErrorMode::SpreadG => 3,
        }
    }
}

/// A 1-D profile histogram (`Profile1D`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TProfile")]
pub struct Profile1D {
    /// Histogram name (`fName`).
    pub name: String,
    /// Histogram title (`fTitle`).
    pub title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Total cells, including flow (`fNcells = nbins + 2`).
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
    /// Per-bin sums of weight*y (the `TH1D` base contents, length `ncells`).
    pub(crate) sums: Vec<f64>,
    /// Per-bin sums of weight*y^2 (the `Hist1D` base `fSumw2`, length `ncells`).
    pub(crate) sumy2: Vec<f64>,
    /// Per-bin entry counts / sums of weight (`fBinEntries`, length `ncells`).
    pub(crate) bin_entries: Vec<f64>,
    /// Error computation mode (`fErrorMode`).
    pub error_mode: ErrorMode,
    /// Lower y limit (`fYmin`).
    pub ymin: f64,
    /// Upper y limit (`fYmax`).
    pub ymax: f64,
    /// Sum of weight*y (`fTsumwy`).
    pub(crate) tsumwy: f64,
    /// Sum of weight*y^2 (`fTsumwy2`).
    pub(crate) tsumwy2: f64,
    /// Per-bin sum of squared weights (`fBinSumw2`), possibly empty.
    pub(crate) bin_sumw2: Vec<f64>,
}

impl Profile1D {
    pub(crate) fn read(r: &mut RBuffer) -> Result<Profile1D> {
        let tprofile = r.read_version()?; // Profile1D wrapper
        let version = tprofile.version;
        if version < 2 {
            return Err(unsupported_version("TProfile", version));
        }

        // The TH1D base: its own wrapper, the Hist1D base, and the TArrayD sums.
        let (core, sums) = read_th1_object(r, BinContentType::F64)?;

        let bin_entries = read_tarray(r, BinContentType::F64)?;
        let error_mode = ErrorMode::from_code(r.be_i32()?);
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

        Ok(Profile1D {
            name: core.name,
            title: core.title,
            xaxis: core.xaxis,
            ncells: core.ncells,
            entries: core.entries,
            tsumw: core.tsumw,
            tsumw2: core.tsumw2,
            tsumwx: core.tsumwx,
            tsumwx2: core.tsumwx2,
            sums,
            sumy2: core.sumw2,
            bin_entries,
            error_mode,
            ymin,
            ymax,
            tsumwy,
            tsumwy2,
            bin_sumw2,
        })
    }

    /// The profiled value per bin (excluding flow): `sum / entries`, or 0 where
    /// a bin has no entries. Matches ROOT/uproot `Profile1D::values()`.
    pub fn values(&self) -> Vec<f64> {
        let n = self.sums.len();
        if n < 2 {
            return Vec::new();
        }
        (1..n - 1).map(|i| self.bin_value(i)).collect()
    }

    /// The X-axis bin edges (`nbins + 1` values).
    pub fn edges(&self) -> Vec<f64> {
        self.xaxis.edges()
    }

    /// Create an empty `Profile1D` with `nbins` uniform x bins over `[xlo, xhi)`
    /// and no y restriction. Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nbins, xlo, xhi).mean().to_root()`.
    pub(crate) fn new(nbins: i32, xlo: f64, xhi: f64) -> Profile1D {
        let ncells = usize::saturating_from(nbins.max(0) + 2);
        Profile1D {
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nbins, xlo, xhi),
            ncells: i32::saturating_from(ncells),
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: 0.0,
            tsumwx: 0.0,
            tsumwx2: 0.0,
            sums: vec![0.0; ncells],
            sumy2: vec![0.0; ncells],
            bin_entries: vec![0.0; ncells],
            error_mode: ErrorMode::Mean,
            ymin: 0.0,
            ymax: 0.0,
            tsumwy: 0.0,
            tsumwy2: 0.0,
            bin_sumw2: Vec::new(),
        }
    }

    /// Profile a point `(x, y)` with unit weight.
    pub fn fill(&mut self, x: f64, y: f64) {
        self.fill_weight(x, y, 1.0);
    }

    /// Profile a point `(x, y)` with weight `w`, matching ROOT's `Profile1D::Fill`:
    /// accumulate the per-bin sums of `w*y` and `w*y^2` and the per-bin weight,
    /// plus the x/y moment sums (the latter only when x is in range). A `y` range
    /// (`ymin != ymax`) rejects out-of-range points before they are counted.
    ///
    /// The first fill with `w != 1` turns on per-bin tracking of `Σw²`
    /// (`fBinSumw2`), as ROOT's `Fill` does; without it the effective entry count,
    /// and so every bin error, would assume unit weights.
    pub fn fill_weight(&mut self, x: f64, y: f64, w: f64) {
        if self.ymin != self.ymax && (y < self.ymin || y > self.ymax || y.is_nan()) {
            return;
        }
        if w != 1.0 {
            self.track_bin_sumw2();
        }
        let nbins = usize::saturating_from(self.xaxis.nbins.max(0));
        let bin = self.xaxis.find_bin(x);
        self.accumulate(bin, y, w);

        if (1..=nbins).contains(&bin) {
            self.tsumw += w;
            self.tsumw2 += w * w;
            self.tsumwx += w * x;
            self.tsumwx2 += w * x * x;
            self.tsumwy += w * y;
            self.tsumwy2 += w * y * y;
        }
    }
}

/// Read a `Profile1D` from an open ROOT file.
pub(crate) fn read_tprofile(file: &FileReader, name: &str) -> Result<Profile1D> {
    Profile1D::read(&mut RBuffer::new(&object_bytes(file, name, "TProfile")?))
}

/// Read a `Profile1D` from subdirectory `subdir`.
pub(crate) fn read_tprofile_in(file: &FileReader, subdir: &str, name: &str) -> Result<Profile1D> {
    Profile1D::read(&mut RBuffer::new(&object_bytes_in(
        file, subdir, name, "TProfile",
    )?))
}
