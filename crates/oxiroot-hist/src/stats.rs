//! Statistics accessors mirroring ROOT's `TH1`/`TH2`/`TH3`/`TProfile` API —
//! `GetStdDev`, `GetMaximum`/`GetMaximumBin`, `FindBin`, `GetBinCenter`,
//! `GetEffectiveEntries`, `Reset`, … All are pure derivations from the already
//! stored bin contents and statistical moment sums (no new on-disk state).

use crate::{Hist1D, Hist2D, Hist3D, Profile1D};
use oxiroot_io_core::cast::SaturatingFrom;

/// Population standard deviation along one axis from its moment sums
/// (`sqrt(<x²> − <x>²)`), matching ROOT's `GetStdDev`. `0.0` for an empty axis.
fn std_dev_axis(tsumw: f64, tsumwx: f64, tsumwx2: f64) -> f64 {
    if tsumw == 0.0 {
        return 0.0;
    }
    let mean = tsumwx / tsumw;
    (tsumwx2 / tsumw - mean * mean).max(0.0).sqrt()
}

impl Hist1D {
    /// Standard deviation of the filled x distribution (ROOT `GetStdDev`, also
    /// exposed as `GetRMS`).
    pub fn std_dev(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwx, self.tsumwx2)
    }
    /// Bin holding `x` (0 = underflow, `1..=nbins`, `nbins+1` = overflow).
    pub fn find_bin(&self, x: f64) -> usize {
        self.xaxis.find_bin(x)
    }
    /// Center of bin `bin` (1-based).
    pub fn bin_center(&self, bin: usize) -> f64 {
        self.xaxis.bin_center(bin)
    }
    /// Width of bin `bin` (1-based).
    pub fn bin_width(&self, bin: usize) -> f64 {
        self.xaxis.bin_width(bin)
    }
    /// Low edge of bin `bin` (1-based).
    pub fn bin_low_edge(&self, bin: usize) -> f64 {
        self.xaxis.bin_low_edge(bin)
    }
    /// Linearly interpolate the bin content at `x` between adjacent bin centers
    /// (ROOT `Hist1D::Interpolate`). Returns the first/last bin content when `x` is
    /// at or beyond the first/last bin center.
    #[must_use]
    pub fn interpolate(&self, x: f64) -> f64 {
        let n = usize::saturating_from(self.xaxis.nbins.max(0));
        if n == 0 {
            return 0.0;
        }
        let content = |bin: usize| self.contents.get(bin).copied().unwrap_or(0.0);
        if x <= self.bin_center(1) {
            return content(1);
        }
        if x >= self.bin_center(n) {
            return content(n);
        }
        let xbin = self.find_bin(x).clamp(1, n);
        let (lo, hi) = if x <= self.bin_center(xbin) {
            (xbin - 1, xbin)
        } else {
            (xbin, xbin + 1)
        };
        let (x0, x1) = (self.bin_center(lo), self.bin_center(hi));
        let (y0, y1) = (content(lo), content(hi));
        y0 + (x - x0) * (y1 - y0) / (x1 - x0)
    }

    /// The `x` values where the cumulative bin-content distribution reaches each
    /// probability in `probs` (ROOT `Hist1D::GetQuantiles`); `probs` should lie in
    /// `[0, 1]`. Within a bin the inverse CDF is interpolated linearly across the
    /// bin's edges; a probability landing exactly on a cumulative bin boundary
    /// returns that bin's center, matching ROOT.
    #[must_use]
    pub fn quantiles(&self, probs: &[f64]) -> Vec<f64> {
        let n = usize::saturating_from(self.xaxis.nbins.max(0));
        // Cumulative (normalized) in-range contents: integral[i] = Σ_{1..i} / total.
        let mut integral = vec![0.0; n + 1];
        for i in 1..=n {
            integral[i] = integral[i - 1] + self.contents.get(i).copied().unwrap_or(0.0);
        }
        let total = integral[n];
        if total <= 0.0 {
            return vec![self.xaxis.xmin; probs.len()];
        }
        for v in &mut integral {
            *v /= total;
        }
        probs
            .iter()
            .map(|&p| {
                // Largest `ibin` in `0..n` with integral[ibin] <= p (ROOT's
                // TMath::BinarySearch searches the first `n` cumulative values).
                let ibin = integral[..n].iter().rposition(|&c| c <= p).unwrap_or(0);
                if integral[ibin] == p {
                    return self.bin_center(ibin); // exact cumulative tie → bin center
                }
                let dint = integral[ibin + 1] - integral[ibin];
                if dint > 0.0 {
                    self.bin_low_edge(ibin + 1)
                        + (p - integral[ibin]) / dint * self.bin_width(ibin + 1)
                } else {
                    self.bin_low_edge(ibin + 1)
                }
            })
            .collect()
    }
}

impl Hist2D {
    /// Standard deviation of the filled x distribution (ROOT `GetStdDev(1)`).
    pub fn std_dev_x(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwx, self.tsumwx2)
    }
    /// Standard deviation of the filled y distribution (ROOT `GetStdDev(2)`).
    pub fn std_dev_y(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwy, self.tsumwy2)
    }
    /// Global cell index `ix + (nx+2)*iy` for `(x, y)` (flow bins included).
    pub fn find_bin(&self, x: f64, y: f64) -> usize {
        self.xaxis.find_bin(x) + (self.nx() + 2) * self.yaxis.find_bin(y)
    }
}

impl Hist3D {
    /// Standard deviation of the filled x distribution.
    pub fn std_dev_x(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwx, self.tsumwx2)
    }
    /// Standard deviation of the filled y distribution.
    pub fn std_dev_y(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwy, self.tsumwy2)
    }
    /// Standard deviation of the filled z distribution.
    pub fn std_dev_z(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwz, self.tsumwz2)
    }
}

impl Profile1D {
    /// Mean of the filled x distribution (ROOT `GetMean`).
    pub fn mean(&self) -> f64 {
        if self.tsumw == 0.0 {
            0.0
        } else {
            self.tsumwx / self.tsumw
        }
    }
    /// Standard deviation of the filled x distribution (ROOT `GetStdDev`).
    pub fn std_dev(&self) -> f64 {
        std_dev_axis(self.tsumw, self.tsumwx, self.tsumwx2)
    }
    /// Center of bin `bin` (1-based).
    pub fn bin_center(&self, bin: usize) -> f64 {
        self.xaxis.bin_center(bin)
    }
    /// Bin holding `x` (0 = underflow, `1..=nbins`, `nbins+1` = overflow).
    pub fn find_bin(&self, x: f64) -> usize {
        self.xaxis.find_bin(x)
    }
    /// Clear all per-bin sums, entries, and moment sums, keeping the binning.
    pub fn reset(&mut self) {
        for arr in [
            &mut self.sums,
            &mut self.sumy2,
            &mut self.bin_entries,
            &mut self.bin_sumw2,
        ] {
            arr.iter_mut().for_each(|v| *v = 0.0);
        }
        self.entries = 0.0;
        for v in [
            &mut self.tsumw,
            &mut self.tsumw2,
            &mut self.tsumwx,
            &mut self.tsumwx2,
            &mut self.tsumwy,
            &mut self.tsumwy2,
        ] {
            *v = 0.0;
        }
    }
}
