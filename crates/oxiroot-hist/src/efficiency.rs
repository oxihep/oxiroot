//! `Efficiency` — an efficiency (passed/total) plot. It holds two embedded
//! `TH1D` histograms (`passed` and `total`) plus the parameters ROOT uses to put
//! a confidence interval on each bin's ratio. uproot cannot read a `TEfficiency`
//! (its `vector<pair<double,double>>` member uses memberwise serialization), so
//! ROOT C++ is the interop oracle here.

use oxiroot_io_core::raw::{
    K_BYTE_COUNT_MASK, K_CLASS_MASK, TagReader, read_object_base, skip_versioned,
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
    /// Whether this is one of the Bayesian options (ROOT's `kIsBayesian`).
    #[must_use]
    pub const fn is_bayesian(self) -> bool {
        matches!(self, Self::Jeffrey | Self::Uniform | Self::Bayesian)
    }

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
    pub(crate) conf_level: f64,
    /// How the confidence interval is computed (`fStatisticOption`).
    pub(crate) statistic_option: StatisticOption,
    /// Weight for combining efficiencies (`fWeight`).
    pub(crate) weight: f64,
    /// Bayesian prior α (`fBeta_alpha`).
    pub(crate) beta_alpha: f64,
    /// Bayesian prior β (`fBeta_beta`).
    pub(crate) beta_beta: f64,
    /// Whether the trials were filled with weights (ROOT's `kUseWeights`).
    pub(crate) weighted: bool,
    /// Whether a Bayesian interval is the shortest one rather than the
    /// central one (ROOT's `kShortestInterval`).
    pub(crate) shortest_interval: bool,
}

/// ROOT's `TEfficiency` status bits.
pub(crate) const K_IS_BAYESIAN: u32 = 1 << 14;
pub(crate) const K_SHORTEST_INTERVAL: u32 = 1 << 16;
pub(crate) const K_USE_WEIGHTS: u32 = 1 << 18;

impl Efficiency {
    /// Confidence level for the interval (`fConfLevel`).
    #[must_use]
    pub fn conf_level(&self) -> f64 {
        self.conf_level
    }

    /// Set [`conf_level`](Self::conf_level).
    #[must_use]
    pub fn with_conf_level(mut self, conf_level: f64) -> Self {
        self.conf_level = conf_level;
        self
    }

    /// How the confidence interval is computed (`fStatisticOption`).
    #[must_use]
    pub fn statistic_option(&self) -> StatisticOption {
        self.statistic_option
    }

    /// Set [`statistic_option`](Self::statistic_option).
    #[must_use]
    pub fn with_statistic_option(mut self, statistic_option: StatisticOption) -> Self {
        self.statistic_option = statistic_option;
        self
    }

    /// Weight for combining efficiencies (`fWeight`).
    #[must_use]
    pub fn weight(&self) -> f64 {
        self.weight
    }

    /// Set [`weight`](Self::weight).
    #[must_use]
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = weight;
        self
    }

    /// Bayesian prior α (`fBeta_alpha`).
    #[must_use]
    pub fn beta_alpha(&self) -> f64 {
        self.beta_alpha
    }

    /// Set [`beta_alpha`](Self::beta_alpha).
    #[must_use]
    pub fn with_beta_alpha(mut self, beta_alpha: f64) -> Self {
        self.beta_alpha = beta_alpha;
        self
    }

    /// Bayesian prior β (`fBeta_beta`).
    #[must_use]
    pub fn beta_beta(&self) -> f64 {
        self.beta_beta
    }

    /// Set [`beta_beta`](Self::beta_beta).
    #[must_use]
    pub fn with_beta_beta(mut self, beta_beta: f64) -> Self {
        self.beta_beta = beta_beta;
        self
    }
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
            weighted: false,
            shortest_interval: false,
        }
    }

    /// Whether the trials were filled with weights (ROOT's `kUseWeights`, set
    /// by `TEfficiency::FillWeighted`). Its intervals then follow from the sums
    /// of weights and of their squares: the normal approximation for every
    /// frequentist option, a Beta posterior over effective counts for the
    /// Bayesian ones. [`weight`](Self::weight) alone does not make it weighted.
    #[must_use]
    pub fn is_weighted(&self) -> bool {
        self.weighted
    }

    /// Whether a Bayesian interval is the shortest one holding the confidence
    /// level, rather than the central one (ROOT's `SetShortestInterval`).
    #[must_use]
    pub fn shortest_interval(&self) -> bool {
        self.shortest_interval
    }

    /// Set [`shortest_interval`](Self::shortest_interval).
    #[must_use]
    pub fn with_shortest_interval(mut self, shortest: bool) -> Self {
        self.shortest_interval = shortest;
        self
    }

    /// ROOT's status bits for these settings, on top of `base`.
    pub(crate) fn status_bits(&self, base: u32) -> u32 {
        let mut bits = base;
        if self.statistic_option.is_bayesian() {
            bits |= K_IS_BAYESIAN;
        }
        if self.shortest_interval {
            bits |= K_SHORTEST_INTERVAL;
        }
        if self.weighted {
            bits |= K_USE_WEIGHTS;
        }
        bits
    }

    /// Record one trial at `x`: always increments `total`, and `passed` too when
    /// `passed == true` (ROOT's `Efficiency::Fill`).
    pub fn fill(&mut self, passed: bool, x: f64) {
        self.total.fill(x);
        if passed {
            self.passed.fill(x);
        }
    }

    /// The confidence interval of the efficiency in bin `bin` (1-based), at
    /// this object's [`conf_level`](Self::conf_level) and by its
    /// [`statistic_option`](Self::statistic_option): the `(lower, upper)`
    /// bounds ROOT's `GetEfficiencyErrorLow` and `GetEfficiencyErrorUp` are
    /// measured from. `None` when the bin has no trials, or does not exist.
    ///
    /// Clopper–Pearson (ROOT's default), the normal approximation, Wilson,
    /// Agresti–Coull and the three Bayesian options (the central interval of
    /// the Beta posterior, or the shortest one with
    /// [`shortest_interval`](Self::shortest_interval)) are computed. A
    /// [weighted](Self::is_weighted) efficiency is computed as ROOT computes
    /// it: every frequentist option by the normal approximation with the sums
    /// of weights and of their squares, the Bayesian ones over the effective
    /// counts `passed · Σw / Σw²` and `(total − passed) · Σw / Σw²`.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for the Feldman–Cousins and mid-P options of an
    /// unweighted efficiency.
    pub fn interval(&self, bin: usize) -> Result<Option<(f64, f64)>> {
        let (Some(&n), Some(&k)) = (self.total.contents.get(bin), self.passed.contents.get(bin))
        else {
            return Ok(None);
        };
        if n <= 0.0 {
            return Ok(None);
        }
        let level = self.conf_level;
        let tail = (1.0 - level) / 2.0;
        let (alpha, beta) = match self.statistic_option {
            StatisticOption::Jeffrey => (0.5, 0.5),
            StatisticOption::Uniform => (1.0, 1.0),
            _ => (self.beta_alpha, self.beta_beta),
        };
        // The central or shortest interval of a Beta(a, b) posterior.
        let posterior = |a: f64, b: f64| {
            if self.shortest_interval {
                beta_shortest_interval(a, b, level)
            } else {
                (
                    oxistats::betaincinv(a, b, tail),
                    oxistats::betaincinv(a, b, 1.0 - tail),
                )
            }
        };
        if self.weighted {
            let sumw2 = |h: &Hist1D, fallback: f64| {
                h.sumw2()
                    .and_then(|s| s.get(bin).copied())
                    .unwrap_or(fallback)
            };
            let (tw2, pw2) = (sumw2(&self.total, n), sumw2(&self.passed, k));
            if self.statistic_option.is_bayesian() {
                if tw2 <= 0.0 {
                    let p = k / n;
                    return Ok(Some((p, p)));
                }
                let norm = n / tw2;
                return Ok(Some(posterior(k * norm + alpha, (n - k) * norm + beta)));
            }
            let p = k / n;
            let variance = (pw2 * (1.0 - 2.0 * p) + tw2 * p * p) / (n * n);
            let half = oxistats::ndtri(1.0 - tail) * variance.max(0.0).sqrt();
            return Ok(Some(((p - half).max(0.0), (p + half).min(1.0))));
        }
        Ok(Some(match self.statistic_option {
            StatisticOption::ClopperPearson => oxistats::clopper_pearson(k, n, level),
            StatisticOption::Wilson => oxistats::wilson_interval(k, n, level),
            StatisticOption::AgrestiCoull => oxistats::agresti_coull_interval(k, n, level),
            StatisticOption::Normal => {
                let p = k / n;
                let half = oxistats::ndtri(1.0 - tail) * (p * (1.0 - p) / n).sqrt();
                ((p - half).max(0.0), (p + half).min(1.0))
            }
            StatisticOption::Jeffrey | StatisticOption::Uniform | StatisticOption::Bayesian => {
                posterior(k + alpha, n - k + beta)
            }
            other => {
                return Err(Error::Unsupported(format!(
                    "the {other:?} interval of a TEfficiency (oxiroot computes \
                     Clopper-Pearson, normal, Wilson, Agresti-Coull and the Bayesian ones)"
                )));
            }
        }))
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
        // TNamed, read here for its TObject's status bits.
        let named = r.read_version()?;
        let bits = read_object_base(r)?.bits;
        let (name, title) = (r.string()?, r.string()?);
        if let Some(end) = named.end {
            r.seek(end)?;
        }
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
            name,
            title,
            passed,
            total,
            conf_level,
            statistic_option,
            weight,
            beta_alpha,
            beta_beta,
            weighted: bits & K_USE_WEIGHTS != 0,
            shortest_interval: bits & K_SHORTEST_INTERVAL != 0,
        })
    }
}

/// The shortest interval holding `level` of a Beta(`a`, `b`) distribution.
/// When the density does not fall on both sides (`a ≤ 1` or `b ≤ 1`), it runs
/// from the end where the density is highest; otherwise its lower tail
/// probability is the one that makes it shortest, found by golden-section
/// search (the width is unimodal in it).
fn beta_shortest_interval(a: f64, b: f64, level: f64) -> (f64, f64) {
    let quantile = |p: f64| oxistats::betaincinv(a, b, p.clamp(0.0, 1.0));
    if a <= 1.0 && b > 1.0 {
        return (0.0, quantile(level));
    }
    if b <= 1.0 && a > 1.0 {
        return (quantile(1.0 - level), 1.0);
    }
    if a <= 1.0 && b <= 1.0 {
        // A density highest at both ends: ROOT keeps the central interval.
        let tail = (1.0 - level) / 2.0;
        return (quantile(tail), quantile(1.0 - tail));
    }
    let width = |t: f64| quantile(t + level) - quantile(t);
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
    let (mut lo, mut hi) = (0.0, 1.0 - level);
    for _ in 0..200 {
        if hi - lo < 1e-14 {
            break;
        }
        let x1 = hi - golden * (hi - lo);
        let x2 = lo + golden * (hi - lo);
        if width(x1) < width(x2) {
            hi = x2;
        } else {
            lo = x1;
        }
    }
    let t = (lo + hi) / 2.0;
    (quantile(t), quantile(t + level))
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

#[cfg(test)]
mod tests {
    use super::{Efficiency, StatisticOption};

    /// Bins holding 0/10, 3/10, 7/7 and 50/200.
    fn sample(option: StatisticOption) -> Efficiency {
        let mut e = Efficiency::new(4, 0.0, 4.0)
            .with_statistic_option(option)
            .with_conf_level(0.683);
        for (bin, (passed, total)) in [(0, 10), (3, 10), (7, 7), (50, 200)]
            .into_iter()
            .enumerate()
        {
            for i in 0..total {
                e.fill(i < passed, bin as f64 + 0.5);
            }
        }
        e
    }

    /// The bounds `TEfficiency::ClopperPearson`, `Wilson`, `AgrestiCoull`,
    /// `Normal` and `Bayesian` of ROOT 6.40 give at a level of 0.683.
    #[test]
    fn the_intervals_are_those_of_root() {
        use StatisticOption::{AgrestiCoull, ClopperPearson, Jeffrey, Normal, Uniform, Wilson};
        let expected: [(StatisticOption, [(f64, f64); 4]); 6] = [
            (
                ClopperPearson,
                [
                    (0.0, 0.168_230_623_728),
                    (0.141_609_285_811, 0.508_362_076_487),
                    (0.768_632_009_595, 1.0),
                    (0.218_283_246_323, 0.284_414_585_847),
                ],
            ),
            (
                Wilson,
                [
                    (0.0, 0.091_015_199_966_3),
                    (0.178_759_350_075, 0.457_646_729_911),
                    (0.874_859_577_441, 1.0),
                    (0.220_658_140_438, 0.281_832_600_042),
                ],
            ),
            (
                AgrestiCoull,
                [
                    (0.0, 0.108_383_637_895),
                    (0.177_683_520_439, 0.458_722_559_548),
                    (0.851_755_206_708, 1.0),
                    (0.220_632_924_274, 0.281_857_816_206),
                ],
            ),
            (
                Normal,
                [
                    (0.0, 0.0),
                    (0.154_993_222_714, 0.445_006_777_286),
                    (1.0, 1.0),
                    (0.219_361_726_303, 0.280_638_273_697),
                ],
            ),
            (
                Jeffrey,
                [
                    (0.001_948_247_754_76, 0.092_399_613_671_9),
                    (0.179_861_906_722, 0.457_850_918_635),
                    (0.871_968_950_176, 0.997_247_515_409),
                    (0.220_668_614_887, 0.281_826_261_09),
                ],
            ),
            (
                Uniform,
                [
                    (0.015_565_697_557_8, 0.154_184_990_168),
                    (0.198_803_783_358, 0.468_895_315_42),
                    (0.794_334_925_849, 0.978_659_835_35),
                    (0.221_925_703_457, 0.283_032_004_47),
                ],
            ),
        ];
        for (option, bounds) in expected {
            let e = sample(option);
            for (i, (lo, hi)) in bounds.into_iter().enumerate() {
                let (got_lo, got_hi) = e.interval(i + 1).unwrap().unwrap();
                assert!(
                    (got_lo - lo).abs() < 1e-9 && (got_hi - hi).abs() < 1e-9,
                    "{option:?} bin {}: ({got_lo}, {got_hi}) is not ({lo}, {hi})",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn a_bayesian_interval_uses_the_prior_of_the_object() {
        let jeffrey = sample(StatisticOption::Jeffrey);
        let bayesian = sample(StatisticOption::Bayesian)
            .with_beta_alpha(0.5)
            .with_beta_beta(0.5);
        assert_eq!(bayesian.interval(2).unwrap(), jeffrey.interval(2).unwrap());
    }

    #[test]
    fn an_empty_or_missing_bin_has_no_interval_and_two_options_are_refused() {
        let e = Efficiency::new(2, 0.0, 2.0);
        assert_eq!(e.interval(1).unwrap(), None);
        assert_eq!(e.interval(9).unwrap(), None);
        for option in [StatisticOption::FeldmanCousins, StatisticOption::MidP] {
            assert!(sample(option).interval(2).is_err());
        }
    }
}
