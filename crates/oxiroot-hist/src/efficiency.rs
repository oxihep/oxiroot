//! `Efficiency` — an efficiency (passed/total) plot. It holds two embedded
//! `TH1D` histograms (`passed` and `total`) plus the parameters ROOT uses to put
//! a confidence interval on each bin's ratio. uproot cannot read a `TEfficiency`
//! (its `vector<pair<double,double>>` member uses memberwise serialization), so
//! ROOT C++ is the interop oracle here.

use oxiroot_io_core::raw::{
    K_BYTE_COUNT_MASK, K_CLASS_MASK, read_named, read_pointer_header, skip_versioned,
};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::base::BinContentType;
use crate::hist1d::Hist1D;

/// An efficiency plot (ROOT `TEfficiency`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TEfficiency")]
pub struct Efficiency {
    /// Name (`fName`).
    pub name: String,
    /// Title (`fTitle`).
    pub title: String,
    /// Numerator histogram — passed trials per bin (`fPassedHistogram`, a `TH1D`).
    pub(crate) passed: Hist1D,
    /// Denominator histogram — total trials per bin (`fTotalHistogram`, a `TH1D`).
    pub(crate) total: Hist1D,
    /// Confidence level for the interval (`fConfLevel`).
    pub conf_level: f64,
    /// Statistic option (`fStatisticOption`; 0 = Clopper–Pearson, ROOT's default).
    pub statistic_option: i32,
    /// Weight for combining efficiencies (`fWeight`).
    pub weight: f64,
    /// Bayesian prior α (`fBeta_alpha`).
    pub beta_alpha: f64,
    /// Bayesian prior β (`fBeta_beta`).
    pub beta_beta: f64,
}

impl Efficiency {
    /// ROOT's default confidence level: one Gaussian sigma.
    pub const DEFAULT_CONF_LEVEL: f64 = 0.682689492137086;

    /// Create an empty `Efficiency` with `nbins` uniform x bins over `[xlo, xhi)`
    /// and ROOT's default interval parameters.
    #[must_use]
    pub fn new(nbins: i32, xlo: f64, xhi: f64) -> Self {
        let total = Hist1D::new(nbins, xlo, xhi).named("total");
        let passed = Hist1D::new(nbins, xlo, xhi).named("passed");
        Self {
            name: String::new(),
            title: String::new(),
            passed,
            total,
            conf_level: Self::DEFAULT_CONF_LEVEL,
            statistic_option: 0,
            weight: 1.0,
            beta_alpha: 1.0,
            beta_beta: 1.0,
        }
    }

    /// Record one trial at `x`: always increments `total`, and `passed` too when
    /// `passed == true` (ROOT's `Efficiency::Fill`).
    pub fn fill(&mut self, passed: bool, x: f64) {
        self.total.fill(x);
        if passed {
            self.passed.fill(x);
        }
    }

    /// Efficiency in bin `bin` (1-based): `passed / total`, or `0` when the bin
    /// has no trials.
    #[must_use]
    pub fn efficiency(&self, bin: usize) -> f64 {
        let t = self.total.contents.get(bin).copied().unwrap_or(0.0);
        if t != 0.0 {
            self.passed.contents.get(bin).copied().unwrap_or(0.0) / t
        } else {
            0.0
        }
    }

    pub(crate) fn read(r: &mut RBuffer<'_>) -> Result<Self> {
        let te = r.read_version()?; // Efficiency wrapper (v2)
        let named = read_named(r)?;
        skip_versioned(r)?; // TAttLine
        skip_versioned(r)?; // TAttFill
        skip_versioned(r)?; // TAttMarker

        let beta_alpha = r.be_f64()?;
        let beta_beta = r.be_f64()?;
        skip_byte_counted(r)?; // fBeta_bin_params (vector<pair<double,double>>)
        let conf_level = r.be_f64()?;
        skip_byte_counted(r)?; // fFunctions (TList*)
        let passed = read_embedded_th1d(r)?; // fPassedHistogram
        let statistic_option = r.be_i32()?;
        let total = read_embedded_th1d(r)?; // fTotalHistogram
        let weight = r.be_f64()?;

        if let Some(end) = te.end {
            r.seek(end)?;
        }

        Ok(Self {
            name: named.name,
            title: named.title,
            passed,
            total,
            conf_level,
            statistic_option,
            weight,
            beta_alpha,
            beta_beta,
        })
    }
}

/// Skip a byte-counted member (`{kByteCountMask | len}{len bytes}`).
fn skip_byte_counted(r: &mut RBuffer<'_>) -> Result<()> {
    let count = r.be_u32()?;
    r.skip((count & !(K_BYTE_COUNT_MASK | K_CLASS_MASK)) as usize)
}

/// Read an embedded `TH1D*`: an object pointer whose class is known from the
/// member (the second histogram names it by a back-reference to the first).
fn read_embedded_th1d(r: &mut RBuffer<'_>) -> Result<Hist1D> {
    if read_pointer_header(r)?.is_none() {
        return Err(Error::Format(
            "a TEfficiency histogram pointer is null".into(),
        ));
    }
    Hist1D::read(r, BinContentType::F64)
}

impl ReadRoot for Efficiency {
    const CLASS: &'static str = "TEfficiency";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| Self::read(&mut RBuffer::new(object.bytes)))
    }
}
