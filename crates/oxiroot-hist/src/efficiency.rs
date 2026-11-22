//! `Efficiency` — an efficiency (passed/total) plot. It holds two embedded
//! `TH1D` histograms (`passed` and `total`) plus the parameters ROOT uses to put
//! a confidence interval on each bin's ratio. uproot cannot read a `TEfficiency`
//! (its `vector<pair<double,double>>` member uses memberwise serialization), so
//! ROOT C++ is the interop oracle here.

use oxiroot_io_core::raw::{
    K_BYTE_COUNT_MASK, K_CLASS_MASK, TagReader, read_named, skip_versioned,
};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::base::{bin_content_type_of, is_hist_class};
use crate::record::Hist1D;

/// How a [`Efficiency`]'s confidence interval is computed (ROOT's
/// `TEfficiency::EStatOption`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StatisticOption {
    /// `kFCP` (0): Clopper–Pearson, the exact frequentist interval (the default).
    #[default]
    ClopperPearson,
    /// `kFNormal` (1): the normal approximation.
    Normal,
    /// `kFWilson` (2): the Wilson interval.
    Wilson,
    /// `kFAC` (3): the Agresti–Coull interval.
    AgrestiCoull,
    /// `kFFC` (4): the Feldman–Cousins interval.
    FeldmanCousins,
    /// `kBJeffrey` (5): Bayesian with the Jeffreys prior.
    Jeffrey,
    /// `kBUniform` (6): Bayesian with a uniform prior.
    Uniform,
    /// `kBBayesian` (7): Bayesian with the prior set by `beta_alpha` and `beta_beta`.
    Bayesian,
    /// `kMidP` (8): the mid-P Lancaster interval.
    MidP,
}

impl StatisticOption {
    /// The value a ROOT `fStatisticOption` code names.
    ///
    /// # Errors
    ///
    /// [`Error::Format`](oxiroot_io_core::Error::Format) for any other code.
    pub fn from_code(code: i32) -> Result<Self> {
        Ok(match code {
            0 => Self::ClopperPearson,
            1 => Self::Normal,
            2 => Self::Wilson,
            3 => Self::AgrestiCoull,
            4 => Self::FeldmanCousins,
            5 => Self::Jeffrey,
            6 => Self::Uniform,
            7 => Self::Bayesian,
            8 => Self::MidP,
            other => {
                return Err(oxiroot_io_core::Error::Format(format!(
                    "unknown efficiency statistic option {other}"
                )));
            }
        })
    }

    /// The ROOT `fStatisticOption` code written to disk.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::ClopperPearson => 0,
            Self::Normal => 1,
            Self::Wilson => 2,
            Self::AgrestiCoull => 3,
            Self::FeldmanCousins => 4,
            Self::Jeffrey => 5,
            Self::Uniform => 6,
            Self::Bayesian => 7,
            Self::MidP => 8,
        }
    }
}

/// An efficiency plot (ROOT `TEfficiency`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TEfficiency")]
pub struct Efficiency {
    /// Name (`fName`).
    pub(crate) name: String,
    /// Title (`fTitle`).
    pub(crate) title: String,
    /// Numerator histogram — passed trials per bin (`fPassedHistogram`, a `TH1D`).
    pub(crate) passed: Hist1D,
    /// Denominator histogram — total trials per bin (`fTotalHistogram`, a `TH1D`).
    pub(crate) total: Hist1D,
    /// Confidence level for the interval (`fConfLevel`).
    pub conf_level: f64,
    /// How the confidence interval is computed (`fStatisticOption`).
    pub statistic_option: StatisticOption,
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
    pub fn new(nbins: usize, xlo: f64, xhi: f64) -> Self {
        let total = Hist1D::new(nbins, xlo, xhi).named("total");
        let passed = Hist1D::new(nbins, xlo, xhi).named("passed");
        Self {
            name: String::new(),
            title: String::new(),
            passed,
            total,
            conf_level: Self::DEFAULT_CONF_LEVEL,
            statistic_option: StatisticOption::default(),
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

    /// Efficiency in bin `bin` (1-based; `0` and `nbins + 1` are the flow
    /// bins): `passed / total`. `None` when the bin has no trials, or does not
    /// exist.
    #[must_use]
    pub fn efficiency(&self, bin: usize) -> Option<f64> {
        let total = *self.total.contents.get(bin)?;
        let passed = *self.passed.contents.get(bin)?;
        (total != 0.0).then(|| passed / total)
    }

    pub(crate) fn read(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
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
        let passed = read_embedded_th1(r, tags)?; // fPassedHistogram
        let statistic_option = StatisticOption::from_code(r.be_i32()?)?;
        let total = read_embedded_th1(r, tags)?; // fTotalHistogram
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

/// Read an embedded `TH1*`: an object pointer whose header names the class
/// (the second histogram names it by a back-reference to the first).
fn read_embedded_th1(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Hist1D> {
    let Some(class) = tags.read_header(r)?.class_name else {
        return Err(Error::Format(
            "a TEfficiency histogram pointer is null".into(),
        ));
    };
    if !is_hist_class(&class, 1) {
        return Err(Error::Unsupported(format!(
            "a TEfficiency over {class} histograms (oxiroot reads the one-dimensional ones)"
        )));
    }
    Hist1D::read(r, bin_content_type_of(&class)?, tags)
}

impl ReadRoot for Efficiency {
    const CLASS: &'static str = "TEfficiency";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| {
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, &mut tags)
        })
    }
}
