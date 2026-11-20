//! Read access to the histogram, profile and graph types' state, and the
//! few writes that cannot break it: the fields are private so the cells,
//! their errors, the moment sums and the binning stay consistent.

use crate::axis::Axis;
use crate::graph::GraphErrors;
use crate::polyhist::PolyBin;
use crate::sparsehist::SparseBin;
use crate::*;

impl Hist1D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// The z axis.
    #[must_use]
    pub fn zaxis(&self) -> &Axis {
        &self.zaxis
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The bin contents, flow bins included (`x` fastest).
    #[must_use]
    pub fn contents(&self) -> &[f64] {
        &self.contents
    }

    /// The per-bin Σw² (`fSumw2`), flow bins included, when error tracking
    /// is on ([`track_sumw2`](Self::track_sumw2)); `None` otherwise.
    #[must_use]
    pub fn sumw2(&self) -> Option<&[f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_slice())
    }

    /// The per-bin Σw², to change in place, when error tracking is on.
    #[must_use]
    pub fn sumw2_mut(&mut self) -> Option<&mut [f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_mut_slice())
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }

    /// The z axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn zaxis_mut(&mut self) -> &mut Axis {
        &mut self.zaxis
    }

    /// The bin contents, to change in place (flow bins included; the number
    /// of cells stays fixed).
    #[must_use]
    pub fn contents_mut(&mut self) -> &mut [f64] {
        &mut self.contents
    }

    /// Set the content of cell `bin` (flow included); out of range does
    /// nothing.
    pub fn set_bin_content(&mut self, bin: usize, value: f64) {
        if let Some(c) = self.contents.get_mut(bin) {
            *c = value;
        }
    }

    /// Set the error of cell `bin`, turning on error tracking; out of range
    /// does nothing.
    pub fn set_bin_error(&mut self, bin: usize, error: f64) {
        if bin < self.contents.len() {
            self.track_sumw2();
            self.sumw2[bin] = error * error;
        }
    }
}

impl Hist2D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// The z axis.
    #[must_use]
    pub fn zaxis(&self) -> &Axis {
        &self.zaxis
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// The `tsumwxy` moment sum.
    #[must_use]
    pub fn tsumwxy(&self) -> f64 {
        self.tsumwxy
    }

    /// The bin contents, flow bins included (`x` fastest).
    #[must_use]
    pub fn contents(&self) -> &[f64] {
        &self.contents
    }

    /// The per-bin Σw² (`fSumw2`), flow bins included, when error tracking
    /// is on ([`track_sumw2`](Self::track_sumw2)); `None` otherwise.
    #[must_use]
    pub fn sumw2(&self) -> Option<&[f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_slice())
    }

    /// The per-bin Σw², to change in place, when error tracking is on.
    #[must_use]
    pub fn sumw2_mut(&mut self) -> Option<&mut [f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_mut_slice())
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }

    /// The z axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn zaxis_mut(&mut self) -> &mut Axis {
        &mut self.zaxis
    }

    /// The bin contents, to change in place (flow bins included; the number
    /// of cells stays fixed).
    #[must_use]
    pub fn contents_mut(&mut self) -> &mut [f64] {
        &mut self.contents
    }

    /// Set the content of cell `bin` (flow included); out of range does
    /// nothing.
    pub fn set_bin_content(&mut self, bin: usize, value: f64) {
        if let Some(c) = self.contents.get_mut(bin) {
            *c = value;
        }
    }

    /// Set the error of cell `bin`, turning on error tracking; out of range
    /// does nothing.
    pub fn set_bin_error(&mut self, bin: usize, error: f64) {
        if bin < self.contents.len() {
            self.track_sumw2();
            self.sumw2[bin] = error * error;
        }
    }
}

impl Hist3D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// The z axis.
    #[must_use]
    pub fn zaxis(&self) -> &Axis {
        &self.zaxis
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// The `tsumwxy` moment sum.
    #[must_use]
    pub fn tsumwxy(&self) -> f64 {
        self.tsumwxy
    }

    /// The `tsumwz` moment sum.
    #[must_use]
    pub fn tsumwz(&self) -> f64 {
        self.tsumwz
    }

    /// The `tsumwz2` moment sum.
    #[must_use]
    pub fn tsumwz2(&self) -> f64 {
        self.tsumwz2
    }

    /// The `tsumwxz` moment sum.
    #[must_use]
    pub fn tsumwxz(&self) -> f64 {
        self.tsumwxz
    }

    /// The `tsumwyz` moment sum.
    #[must_use]
    pub fn tsumwyz(&self) -> f64 {
        self.tsumwyz
    }

    /// The bin contents, flow bins included (`x` fastest).
    #[must_use]
    pub fn contents(&self) -> &[f64] {
        &self.contents
    }

    /// The per-bin Σw² (`fSumw2`), flow bins included, when error tracking
    /// is on ([`track_sumw2`](Self::track_sumw2)); `None` otherwise.
    #[must_use]
    pub fn sumw2(&self) -> Option<&[f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_slice())
    }

    /// The per-bin Σw², to change in place, when error tracking is on.
    #[must_use]
    pub fn sumw2_mut(&mut self) -> Option<&mut [f64]> {
        (!self.sumw2.is_empty()).then_some(self.sumw2.as_mut_slice())
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }

    /// The z axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn zaxis_mut(&mut self) -> &mut Axis {
        &mut self.zaxis
    }

    /// The bin contents, to change in place (flow bins included; the number
    /// of cells stays fixed).
    #[must_use]
    pub fn contents_mut(&mut self) -> &mut [f64] {
        &mut self.contents
    }

    /// Set the content of cell `bin` (flow included); out of range does
    /// nothing.
    pub fn set_bin_content(&mut self, bin: usize, value: f64) {
        if let Some(c) = self.contents.get_mut(bin) {
            *c = value;
        }
    }

    /// Set the error of cell `bin`, turning on error tracking; out of range
    /// does nothing.
    pub fn set_bin_error(&mut self, bin: usize, error: f64) {
        if bin < self.contents.len() {
            self.track_sumw2();
            self.sumw2[bin] = error * error;
        }
    }
}

impl Profile1D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// Total cells, flow included.
    #[must_use]
    pub fn ncells(&self) -> i32 {
        self.ncells
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// Per cell, the Σw·v of the profiled value.
    #[must_use]
    pub fn sums(&self) -> &[f64] {
        &self.sums
    }

    /// `sumy2`.
    #[must_use]
    pub fn sumy2(&self) -> &[f64] {
        &self.sumy2
    }

    /// Per cell, Σw (`fBinEntries`).
    #[must_use]
    pub fn bin_entries(&self) -> &[f64] {
        &self.bin_entries
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// Per cell, Σw² (`fBinSumw2`); empty when not tracked.
    #[must_use]
    pub fn bin_sumw2(&self) -> &[f64] {
        &self.bin_sumw2
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }
}

impl Profile2D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// Total cells, flow included.
    #[must_use]
    pub fn ncells(&self) -> i32 {
        self.ncells
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// The `tsumwxy` moment sum.
    #[must_use]
    pub fn tsumwxy(&self) -> f64 {
        self.tsumwxy
    }

    /// Per cell, the Σw·v of the profiled value.
    #[must_use]
    pub fn sums(&self) -> &[f64] {
        &self.sums
    }

    /// `sumz2`.
    #[must_use]
    pub fn sumz2(&self) -> &[f64] {
        &self.sumz2
    }

    /// Per cell, Σw (`fBinEntries`).
    #[must_use]
    pub fn bin_entries(&self) -> &[f64] {
        &self.bin_entries
    }

    /// The `tsumwz` moment sum.
    #[must_use]
    pub fn tsumwz(&self) -> f64 {
        self.tsumwz
    }

    /// The `tsumwz2` moment sum.
    #[must_use]
    pub fn tsumwz2(&self) -> f64 {
        self.tsumwz2
    }

    /// Per cell, Σw² (`fBinSumw2`); empty when not tracked.
    #[must_use]
    pub fn bin_sumw2(&self) -> &[f64] {
        &self.bin_sumw2
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }
}

impl Profile3D {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// The z axis.
    #[must_use]
    pub fn zaxis(&self) -> &Axis {
        &self.zaxis
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// The `tsumwxy` moment sum.
    #[must_use]
    pub fn tsumwxy(&self) -> f64 {
        self.tsumwxy
    }

    /// The `tsumwz` moment sum.
    #[must_use]
    pub fn tsumwz(&self) -> f64 {
        self.tsumwz
    }

    /// The `tsumwz2` moment sum.
    #[must_use]
    pub fn tsumwz2(&self) -> f64 {
        self.tsumwz2
    }

    /// The `tsumwxz` moment sum.
    #[must_use]
    pub fn tsumwxz(&self) -> f64 {
        self.tsumwxz
    }

    /// The `tsumwyz` moment sum.
    #[must_use]
    pub fn tsumwyz(&self) -> f64 {
        self.tsumwyz
    }

    /// Per cell, the Σw·v of the profiled value.
    #[must_use]
    pub fn sums(&self) -> &[f64] {
        &self.sums
    }

    /// `sumt2`.
    #[must_use]
    pub fn sumt2(&self) -> &[f64] {
        &self.sumt2
    }

    /// Per cell, Σw (`fBinEntries`).
    #[must_use]
    pub fn bin_entries(&self) -> &[f64] {
        &self.bin_entries
    }

    /// The `tsumwt` moment sum.
    #[must_use]
    pub fn tsumwt(&self) -> f64 {
        self.tsumwt
    }

    /// The `tsumwt2` moment sum.
    #[must_use]
    pub fn tsumwt2(&self) -> f64 {
        self.tsumwt2
    }

    /// Per cell, Σw² (`fBinSumw2`); empty when not tracked.
    #[must_use]
    pub fn bin_sumw2(&self) -> &[f64] {
        &self.bin_sumw2
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }

    /// The z axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn zaxis_mut(&mut self) -> &mut Axis {
        &mut self.zaxis
    }
}

impl Axis {
    /// The number of in-range bins.
    #[must_use]
    pub fn nbins(&self) -> i32 {
        self.nbins
    }

    /// The low edge of the first bin.
    #[must_use]
    pub fn xmin(&self) -> f64 {
        self.xmin
    }

    /// The high edge of the last bin.
    #[must_use]
    pub fn xmax(&self) -> f64 {
        self.xmax
    }

    /// The bin edges of a variable-width axis; empty for a uniform one.
    #[must_use]
    pub fn xbins(&self) -> &[f64] {
        &self.xbins
    }
}

impl Graph {
    /// The x values.
    #[must_use]
    pub fn x(&self) -> &[f64] {
        &self.x
    }

    /// The y values.
    #[must_use]
    pub fn y(&self) -> &[f64] {
        &self.y
    }

    /// The error bars.
    #[must_use]
    pub fn errors(&self) -> &GraphErrors {
        &self.errors
    }

    /// The x values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn x_mut(&mut self) -> &mut [f64] {
        &mut self.x
    }

    /// The y values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn y_mut(&mut self) -> &mut [f64] {
        &mut self.y
    }
}

impl Graph2D {
    /// The x values.
    #[must_use]
    pub fn x(&self) -> &[f64] {
        &self.x
    }

    /// The y values.
    #[must_use]
    pub fn y(&self) -> &[f64] {
        &self.y
    }

    /// The z values.
    #[must_use]
    pub fn z(&self) -> &[f64] {
        &self.z
    }

    /// The x values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn x_mut(&mut self) -> &mut [f64] {
        &mut self.x
    }

    /// The y values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn y_mut(&mut self) -> &mut [f64] {
        &mut self.y
    }

    /// The z values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn z_mut(&mut self) -> &mut [f64] {
        &mut self.z
    }
}

impl MultiErrorGraph {
    /// The x values.
    #[must_use]
    pub fn x(&self) -> &[f64] {
        &self.x
    }

    /// The y values.
    #[must_use]
    pub fn y(&self) -> &[f64] {
        &self.y
    }

    /// `ex_low`.
    #[must_use]
    pub fn ex_low(&self) -> &[f64] {
        &self.ex_low
    }

    /// `ex_high`.
    #[must_use]
    pub fn ex_high(&self) -> &[f64] {
        &self.ex_high
    }

    /// `ey_low`.
    #[must_use]
    pub fn ey_low(&self) -> &[Vec<f64>] {
        &self.ey_low
    }

    /// `ey_high`.
    #[must_use]
    pub fn ey_high(&self) -> &[Vec<f64>] {
        &self.ey_high
    }
}

impl Efficiency {
    /// The passed trials per bin (`fPassedHistogram`).
    #[must_use]
    pub fn passed(&self) -> &Hist1D {
        &self.passed
    }

    /// The total trials per bin (`fTotalHistogram`).
    #[must_use]
    pub fn total(&self) -> &Hist1D {
        &self.total
    }
}

impl PolyHist {
    /// The x axis.
    #[must_use]
    pub fn xaxis(&self) -> &Axis {
        &self.xaxis
    }

    /// The y axis.
    #[must_use]
    pub fn yaxis(&self) -> &Axis {
        &self.yaxis
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> f64 {
        self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> f64 {
        self.tsumwx2
    }

    /// The `tsumwy` moment sum.
    #[must_use]
    pub fn tsumwy(&self) -> f64 {
        self.tsumwy
    }

    /// The `tsumwy2` moment sum.
    #[must_use]
    pub fn tsumwy2(&self) -> f64 {
        self.tsumwy2
    }

    /// The `tsumwxy` moment sum.
    #[must_use]
    pub fn tsumwxy(&self) -> f64 {
        self.tsumwxy
    }

    /// The nine flow regions around the polygons.
    #[must_use]
    pub fn overflow(&self) -> &[f64; 9] {
        &self.overflow
    }

    /// The bins.
    #[must_use]
    pub fn bins(&self) -> &[PolyBin] {
        &self.bins
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }

    /// The x axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn xaxis_mut(&mut self) -> &mut Axis {
        &mut self.xaxis
    }

    /// The y axis, to change its title, labels or time display (its
    /// binning stays fixed).
    #[must_use]
    pub fn yaxis_mut(&mut self) -> &mut Axis {
        &mut self.yaxis
    }
}

impl SparseHist {
    /// The axes.
    #[must_use]
    pub fn axes(&self) -> &[Axis] {
        &self.axes
    }

    /// The number of entries (`fEntries`).
    #[must_use]
    pub fn entries(&self) -> f64 {
        self.entries
    }

    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }

    /// Σw² over the fills.
    #[must_use]
    pub fn tsumw2(&self) -> f64 {
        self.tsumw2
    }

    /// The `tsumwx` moment sum.
    #[must_use]
    pub fn tsumwx(&self) -> &[f64] {
        &self.tsumwx
    }

    /// The `tsumwx2` moment sum.
    #[must_use]
    pub fn tsumwx2(&self) -> &[f64] {
        &self.tsumwx2
    }

    /// The bins.
    #[must_use]
    pub fn bins(&self) -> &[SparseBin] {
        &self.bins
    }

    /// Set the number of entries (`fEntries`).
    pub fn set_entries(&mut self, entries: f64) {
        self.entries = entries;
    }
}

impl Profile1D {
    /// Mutable access to the optional per-cell arrays (`sumy2` and
    /// `bin_sumw2`), which a file may omit; for tests that build such a
    /// profile. Changing their length makes the profile inconsistent.
    #[doc(hidden)]
    pub fn optional_arrays_mut(&mut self) -> (&mut Vec<f64>, &mut Vec<f64>) {
        (&mut self.sumy2, &mut self.bin_sumw2)
    }
}

impl Profile2D {
    /// Mutable access to the optional per-cell arrays (`sumz2` and
    /// `bin_sumw2`), which a file may omit; for tests that build such a
    /// profile. Changing their length makes the profile inconsistent.
    #[doc(hidden)]
    pub fn optional_arrays_mut(&mut self) -> (&mut Vec<f64>, &mut Vec<f64>) {
        (&mut self.sumz2, &mut self.bin_sumw2)
    }
}

impl Profile3D {
    /// Mutable access to the optional per-cell arrays (`sumt2` and
    /// `bin_sumw2`), which a file may omit; for tests that build such a
    /// profile. Changing their length makes the profile inconsistent.
    #[doc(hidden)]
    pub fn optional_arrays_mut(&mut self) -> (&mut Vec<f64>, &mut Vec<f64>) {
        (&mut self.sumt2, &mut self.bin_sumw2)
    }
}
