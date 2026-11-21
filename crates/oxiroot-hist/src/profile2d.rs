//! `Profile2D` — a 2-D profile histogram. For each `(x, y)` cell it stores the
//! running sums needed to recover the mean (and spread) of a third quantity `z`:
//! `Σw` (`bin_entries`), `Σ(w·z)` (`sums`, the `Hist2D` contents), and `Σ(w·z²)`
//! (`sumz2`, the `Hist2D` `fSumw2`). On disk it is a `TH2D` plus the profile members,
//! exactly as `Profile1D` is a `TH1D` plus profile members.

use crate::base::Moments;
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::axis::Axis;
use crate::base::{
    BinContentType, cell_count, check_cells, end_record, in_range_sum, read_tarray, read_th1_base,
    unsupported_version,
};
use crate::profile1d::ErrorMode;

/// A 2-D profile histogram (ROOT `TProfile2D`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TProfile2D")]
pub struct Profile2D {
    /// Histogram name (`fName`).
    pub(crate) name: String,
    /// Histogram title (`fTitle`).
    pub(crate) title: String,
    /// X axis.
    pub(crate) xaxis: Axis,
    /// Y axis.
    pub(crate) yaxis: Axis,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// The sums over the fills (`fTsumw` and the rest).
    pub(crate) moments: Moments,
    /// Per-cell `Σ(w·z)` (the `Hist2D` contents, `fArray`); length `ncells`.
    pub(crate) sums: Vec<f64>,
    /// Per-cell `Σ(w·z²)` (the `Hist2D` `fSumw2`); length `ncells`.
    pub(crate) sumz2: Vec<f64>,
    /// Per-cell `Σw` (`fBinEntries`); length `ncells`.
    pub(crate) bin_entries: Vec<f64>,
    /// Error computation mode (`fErrorMode`: 0=mean, 1=spread, 2=spread-i, 3=spread-g).
    pub error_mode: ErrorMode,
    /// Lower `z` accept bound (`fZmin`; `0` = no restriction when `zmin == zmax`).
    pub zmin: f64,
    /// Upper `z` accept bound (`fZmax`).
    pub zmax: f64,
    /// Per-cell `Σw²` (`fBinSumw2`); empty unless weighted-error tracking is on.
    pub(crate) bin_sumw2: Vec<f64>,
}

impl Profile2D {
    /// Create an empty `Profile2D` with uniform x and y bins and no z
    /// restriction. Internal primitive behind the public builder:
    /// [`Hist::reg`](crate::Hist::reg)`(nx, xlo, xhi).reg(ny, ylo, yhi).mean().to_root()?`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(nx: usize, xlo: f64, xhi: f64, ny: usize, ylo: f64, yhi: f64) -> Self {
        let ncells = (nx + 2) * (ny + 2);
        Self {
            name: String::new(),
            title: String::new(),
            xaxis: Axis::new("xaxis", nx, xlo, xhi),
            yaxis: Axis::new("yaxis", ny, ylo, yhi),
            entries: 0.0,
            moments: Moments::default(),
            sums: vec![0.0; ncells],
            sumz2: vec![0.0; ncells],
            bin_entries: vec![0.0; ncells],
            error_mode: ErrorMode::Mean,
            zmin: 0.0,
            zmax: 0.0,
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

    /// Profiled `z` value per in-range cell as `values[ix][iy]`: `Σ(w·z) / Σw`,
    /// or `0` where a cell has no entries. Matches ROOT/uproot.
    #[must_use]
    pub fn values(&self) -> Vec<Vec<f64>> {
        let (nx, ny) = (self.nx(), self.ny());
        let stride = nx + 2;
        (1..=nx)
            .map(|ix| {
                (1..=ny)
                    .map(|iy| self.bin_value(ix + stride * iy))
                    .collect()
            })
            .collect()
    }

    pub(crate) fn read(r: &mut RBuffer<'_>) -> Result<Self> {
        // Older class versions lack the trailing members: fTsumwz/fTsumwz2 arrived
        // in version 5 (ROOT 4) and fBinSumw2 in version 7 (ROOT 5.24). Version 1
        // (ROOT 1) stored fZmin/fZmax as floats, and its TH2D its members in
        // another order.
        let tp = r.read_version()?; // Profile2D wrapper
        let version = tp.version;
        if version < 2 {
            return Err(unsupported_version("TProfile2D", version));
        }
        let th2d = r.read_version()?; // TH2D wrapper
        if th2d.version < 2 {
            return Err(unsupported_version("TH2D", th2d.version));
        }
        let th2 = r.read_version()?; // Hist2D wrapper (Hist1D base + Hist2D members)

        let c = read_th1_base(r, None)?;
        let _scalefactor = r.be_f64()?;
        let tsumwy = r.be_f64()?;
        let tsumwy2 = r.be_f64()?;
        let tsumwxy = r.be_f64()?;
        let end = th2
            .end
            .ok_or_else(|| Error::Format("TH2 record has no byte count".into()))?;
        r.seek(end)?;

        let sums = read_tarray(r, BinContentType::F64)?; // TH2D TArrayD = Σ(w·z)
        let bin_entries = read_tarray(r, BinContentType::F64)?;
        let error_mode = ErrorMode::from_code(r.be_i32()?)?;
        let zmin = r.be_f64()?;
        let zmax = r.be_f64()?;
        let stored_z_sums = if version >= 5 {
            Some((r.be_f64()?, r.be_f64()?)) // fTsumwz, fTsumwz2
        } else {
            None
        };
        let bin_sumw2 = if version >= 7 {
            read_tarray(r, BinContentType::F64)?
        } else {
            Vec::new() // no fBinSumw2: weights were not tracked
        };
        end_record(r, &tp, "TProfile2D")?;

        let cells = cell_count(&[c.xaxis.nbins, c.yaxis.nbins])?;
        check_cells("TProfile2D sums", sums.len(), cells, false)?;
        check_cells("TProfile2D fBinEntries", bin_entries.len(), cells, false)?;
        check_cells("TProfile2D fSumw2", c.sumw2.len(), cells, true)?;
        check_cells("TProfile2D fBinSumw2", bin_sumw2.len(), cells, true)?;

        // Before version 5 the z sums were not stored: take them from the bins,
        // as ROOT's GetStats does for a profile without stored statistics.
        let (tsumwz, tsumwz2) = stored_z_sums.unwrap_or_else(|| {
            let nbins = [c.xaxis.nbins, c.yaxis.nbins];
            (in_range_sum(&sums, &nbins), in_range_sum(&c.sumw2, &nbins))
        });

        Ok(Self {
            name: c.name,
            title: c.title,
            xaxis: c.xaxis,
            yaxis: c.yaxis,
            entries: c.entries,
            moments: Moments {
                wy: tsumwy,
                wy2: tsumwy2,
                wxy: tsumwxy,
                wz: tsumwz,
                wz2: tsumwz2,
                ..c.moments
            },
            sums,
            sumz2: c.sumw2,
            bin_entries,
            error_mode,
            zmin,
            zmax,
            bin_sumw2,
        })
    }
}

impl ReadRoot for Profile2D {
    const CLASS: &'static str = "TProfile2D";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| Self::read(&mut RBuffer::new(object.bytes)))
    }
}
