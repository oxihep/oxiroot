//! Read access to the histogram, profile and graph types' state, and the
//! few writes that cannot break it: the fields are private so the cells,
//! their errors, the moment sums and the binning stay consistent.

use crate::axis::Axis;
use crate::graph::GraphErrors;
use crate::polyhist::PolyBin;
use crate::sparsehist::SparseBin;
use crate::*;

/// One definition of each accessor several types share: the same field, read
/// the same way, with the same documentation.
macro_rules! accessor {
    (xaxis) => {
        /// The x axis.
        #[must_use]
        pub fn xaxis(&self) -> &Axis {
            &self.xaxis
        }
    };
    (yaxis) => {
        /// The y axis.
        #[must_use]
        pub fn yaxis(&self) -> &Axis {
            &self.yaxis
        }
    };
    (zaxis) => {
        /// The z axis.
        #[must_use]
        pub fn zaxis(&self) -> &Axis {
            &self.zaxis
        }
    };
    (entries) => {
        /// The number of entries (`fEntries`).
        #[must_use]
        pub fn entries(&self) -> f64 {
            self.entries
        }
    };
    (tsumw) => {
        /// Σw over the fills.
        #[must_use]
        pub fn tsumw(&self) -> f64 {
            self.moments.w
        }
    };
    (tsumw2) => {
        /// Σw² over the fills.
        #[must_use]
        pub fn tsumw2(&self) -> f64 {
            self.moments.w2
        }
    };
    (tsumwx) => {
        /// The `tsumwx` moment sum.
        #[must_use]
        pub fn tsumwx(&self) -> f64 {
            self.moments.wx
        }
    };
    (tsumwx2) => {
        /// The `tsumwx2` moment sum.
        #[must_use]
        pub fn tsumwx2(&self) -> f64 {
            self.moments.wx2
        }
    };
    (contents) => {
        /// The bin contents, flow bins included (`x` fastest).
        #[must_use]
        pub fn contents(&self) -> &[f64] {
            &self.contents
        }
    };
    (sumw2) => {
        /// The per-bin Σw² (`fSumw2`), flow bins included, when the histogram
        /// tracks errors; `None` otherwise.
        #[must_use]
        pub fn sumw2(&self) -> Option<&[f64]> {
            (!self.sumw2.is_empty()).then_some(self.sumw2.as_slice())
        }
    };
    (xaxis_mut) => {
        /// The x axis, to change its title, labels or time display (its
        /// binning stays fixed).
        #[must_use]
        pub fn xaxis_mut(&mut self) -> &mut Axis {
            &mut self.xaxis
        }
    };
    (yaxis_mut) => {
        /// The y axis, to change its title, labels or time display (its
        /// binning stays fixed).
        #[must_use]
        pub fn yaxis_mut(&mut self) -> &mut Axis {
            &mut self.yaxis
        }
    };
    (zaxis_mut) => {
        /// The z axis, to change its title, labels or time display (its
        /// binning stays fixed).
        #[must_use]
        pub fn zaxis_mut(&mut self) -> &mut Axis {
            &mut self.zaxis
        }
    };
    (tsumwy) => {
        /// The `tsumwy` moment sum.
        #[must_use]
        pub fn tsumwy(&self) -> f64 {
            self.moments.wy
        }
    };
    (tsumwy2) => {
        /// The `tsumwy2` moment sum.
        #[must_use]
        pub fn tsumwy2(&self) -> f64 {
            self.moments.wy2
        }
    };
    (tsumwxy) => {
        /// The `tsumwxy` moment sum.
        #[must_use]
        pub fn tsumwxy(&self) -> f64 {
            self.moments.wxy
        }
    };
    (tsumwz) => {
        /// The `tsumwz` moment sum.
        #[must_use]
        pub fn tsumwz(&self) -> f64 {
            self.moments.wz
        }
    };
    (tsumwz2) => {
        /// The `tsumwz2` moment sum.
        #[must_use]
        pub fn tsumwz2(&self) -> f64 {
            self.moments.wz2
        }
    };
    (tsumwxz) => {
        /// The `tsumwxz` moment sum.
        #[must_use]
        pub fn tsumwxz(&self) -> f64 {
            self.moments.wxz
        }
    };
    (tsumwyz) => {
        /// The `tsumwyz` moment sum.
        #[must_use]
        pub fn tsumwyz(&self) -> f64 {
            self.moments.wyz
        }
    };
    (ncells) => {
        /// Total cells, flow included (ROOT's `fNcells`): the length of the
        /// per-cell vectors, so it cannot disagree with them.
        #[must_use]
        pub fn ncells(&self) -> usize {
            self.bin_entries.len()
        }
    };
    (sums) => {
        /// Per cell, the Σw·v of the profiled value.
        #[must_use]
        pub fn sums(&self) -> &[f64] {
            &self.sums
        }
    };
    (bin_entries) => {
        /// Per cell, Σw (`fBinEntries`).
        #[must_use]
        pub fn bin_entries(&self) -> &[f64] {
            &self.bin_entries
        }
    };
    (bin_sumw2) => {
        /// Per cell, Σw² (`fBinSumw2`); empty when not tracked.
        #[must_use]
        pub fn bin_sumw2(&self) -> &[f64] {
            &self.bin_sumw2
        }
    };
    (x) => {
        /// The x values.
        #[must_use]
        pub fn x(&self) -> &[f64] {
            &self.x
        }
    };
    (y) => {
        /// The y values.
        #[must_use]
        pub fn y(&self) -> &[f64] {
            &self.y
        }
    };
    (x_mut) => {
        /// The x values, to change in place (the number of points stays
        /// fixed).
        #[must_use]
        pub fn x_mut(&mut self) -> &mut [f64] {
            &mut self.x
        }
    };
    (y_mut) => {
        /// The y values, to change in place (the number of points stays
        /// fixed).
        #[must_use]
        pub fn y_mut(&mut self) -> &mut [f64] {
            &mut self.y
        }
    };
    (set_entries) => {
        /// Set the number of entries (`fEntries`).
        pub fn set_entries(&mut self, entries: f64) {
            self.entries = entries;
        }
    };
}

/// Give `$ty` the listed shared accessors.
macro_rules! accessors {
    ($ty:ty: $($name:ident),+ $(,)?) => {
        impl $ty {
            $(accessor!($name);)+
        }
    };
}

impl<const D: usize> HistRecord<D> {
    accessor!(xaxis);
    accessor!(yaxis);
    accessor!(zaxis);
    accessor!(entries);
    accessor!(tsumw);
    accessor!(tsumw2);
    accessor!(tsumwx);
    accessor!(tsumwx2);
    accessor!(contents);
    accessor!(sumw2);
    accessor!(xaxis_mut);
    accessor!(yaxis_mut);
    accessor!(zaxis_mut);
}
accessors!(Hist2D: tsumwy, tsumwy2, tsumwxy);
accessors!(
    Hist3D:
    tsumwy, tsumwy2, tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz
);
accessors!(
    Profile1D:
    xaxis, ncells, entries, tsumw, tsumw2, tsumwx, tsumwx2, sums, bin_entries, tsumwy,
    tsumwy2, bin_sumw2, xaxis_mut
);
accessors!(
    Profile2D:
    xaxis, yaxis, ncells, entries, tsumw, tsumw2, tsumwx, tsumwx2, tsumwy, tsumwy2,
    tsumwxy, sums, bin_entries, tsumwz, tsumwz2, bin_sumw2, xaxis_mut, yaxis_mut
);
accessors!(
    Profile3D:
    xaxis, yaxis, zaxis, entries, tsumw, tsumw2, tsumwx, tsumwx2, tsumwy, tsumwy2,
    tsumwxy, tsumwz, tsumwz2, tsumwxz, tsumwyz, sums, bin_entries, bin_sumw2, xaxis_mut,
    yaxis_mut, zaxis_mut
);
accessors!(
    Graph:
    x, y, x_mut, y_mut
);
accessors!(
    Graph2D:
    x, y, x_mut, y_mut
);
accessors!(
    MultiErrorGraph:
    x, y
);
accessors!(
    PolyHist:
    xaxis, yaxis, entries, tsumw, tsumw2, tsumwx, tsumwx2, tsumwy, tsumwy2, tsumwxy,
    set_entries, xaxis_mut, yaxis_mut
);
accessors!(
    SparseHist:
    entries, set_entries
);

impl SparseHist {
    /// Σw over the fills.
    #[must_use]
    pub fn tsumw(&self) -> f64 {
        self.tsumw
    }
}

impl Profile1D {
    /// `sumy2`.
    #[must_use]
    pub fn sumy2(&self) -> &[f64] {
        &self.sumy2
    }
}

impl Profile2D {
    /// `sumz2`.
    #[must_use]
    pub fn sumz2(&self) -> &[f64] {
        &self.sumz2
    }
}

impl Profile3D {
    /// `sumt2`.
    #[must_use]
    pub fn sumt2(&self) -> &[f64] {
        &self.sumt2
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
}

impl Axis {
    /// The number of in-range bins.
    #[must_use]
    pub fn nbins(&self) -> usize {
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
    /// The error bars.
    #[must_use]
    pub fn errors(&self) -> &GraphErrors {
        &self.errors
    }
}

impl Graph2D {
    /// The z values.
    #[must_use]
    pub fn z(&self) -> &[f64] {
        &self.z
    }

    /// The z values, to change in place (the number of points stays
    /// fixed).
    #[must_use]
    pub fn z_mut(&mut self) -> &mut [f64] {
        &mut self.z
    }
}

impl MultiErrorGraph {
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
}

impl SparseHist {
    /// The axes.
    #[must_use]
    pub fn axes(&self) -> &[Axis] {
        &self.axes
    }

    /// Σw² over the fills, or `None` when the histogram does not track its
    /// statistics (ROOT stores `-1` for that, and writes it for a new
    /// `THnSparse`).
    #[must_use]
    pub fn tsumw2(&self) -> Option<f64> {
        (self.tsumw2 >= 0.0).then_some(self.tsumw2)
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
