//! Converting between [oxihist](https://github.com/oxihep/oxihist) histograms
//! and ROOT's, the way uproot converts scikit-hep `hist` objects: an oxihist
//! histogram becomes a `TH1`/`TH2`/`TH3` or `TProfile`/`2D`/`3D` that ROOT and
//! uproot read, and a ROOT histogram read from a file becomes an oxihist one.
//!
//! ```
//! use oxiroot_hist::{FromRoot, Hist, ToRoot};
//! # fn main() -> oxiroot_io_core::Result<()> {
//! # let dir = std::env::temp_dir();
//! # let path = dir.join("oxiroot-convert-doc.root");
//! let mut h = Hist::reg(10, 0.0, 10.0).name("pt").title("p_T").weight();
//! h.fill_weight(3.5, 2.0);
//! h.write_root(&path, oxiroot_io_core::Compression::None)?; // a TH1D with Sumw2
//!
//! let file = oxiroot_io_core::FileReader::open(&path)?;
//! let back = oxihist::Histogram::<oxihist::Weight, 1>::read_root(&file, "pt")?;
//! assert_eq!(back.value([3]), Some(2.0));
//! assert_eq!(back.variance([3]), Some(4.0));
//! # Ok(()) }
//! ```
//!
//! # How the two map
//!
//! | oxihist | ROOT |
//! |---|---|
//! | [`Double`] storage | `TH1D`/`TH2D`/`TH3D` without `Sumw2` |
//! | [`Int64`] storage | `TH1L`/`TH2L`/`TH3L` |
//! | [`Weight`] storage | `TH1D`/… with `Sumw2` (the variances) |
//! | [`Mean`] / [`WeightedMean`] storage | `TProfile`/`TProfile2D`/`TProfile3D` |
//! | `Regular` axis | a uniform axis |
//! | `Variable` axis | a variable axis (`fXbins`) |
//! | `Integer` axis `lo..hi` | a uniform axis over `[lo, hi)`, one bin per integer |
//! | `StrCategory`, `IntCategory`, `Boolean` axis | a labelled axis, one bin per category |
//! | histogram name / title, axis label | `fName` / `fTitle`, the axis title |
//!
//! # What does not survive the trip
//!
//! ROOT always has both flow bins; an oxihist axis without one leaves it empty.
//! ROOT's summary moments (`fTsumw`, `fTsumwx`, …) are recomputed from the bin
//! centers, and `fEntries` is the sum of the bin values, as uproot does: oxihist
//! does not count fills. Reading back gives `Regular`, `Variable` or
//! `StrCategory` axes (a labelled axis's underflow bin is dropped, since a
//! category axis has none), [`Weight`] storage for histograms and
//! [`WeightedMean`] for profiles. So, read back:
//!
//! - an `Integer` axis is a `Regular` one with the same bins;
//! - an `IntCategory` or `Boolean` axis is a `StrCategory` one over its labels;
//! - an axis's growth flag and its [`name`](oxihist::Axis::name) are gone (ROOT
//!   axes are always `xaxis`/`yaxis`/`zaxis`), only its label is kept;
//! - a category axis with no categories yet is written as one empty, unlabelled
//!   ROOT bin, and comes back as a one-bin `Regular` axis over `[0, 1)`;
//! - the fill count is not kept, since `fEntries` is written as the sum of the
//!   bin values.
//!
//! [`ToRoot::try_to_root`] refuses what ROOT cannot hold at all: an axis with
//! more than `i32::MAX` bins, or an axis kind this version does not know.
//!
//! A profile's sums convert exactly, but the two libraries put different error
//! bars on them: ROOT's error on a bin mean uses the population spread
//! (`Σwy²/Σw − ȳ²`), oxihist's (like boost-histogram's) the unbiased sample
//! variance, so ROOT's error bars are slightly smaller for bins with few
//! entries.

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::Path;

use oxihist::{
    AxisKind, Double, Histogram, Int64, Mean, MeanBin, Storage, Weight, WeightedMean,
    WeightedMeanBin, WeightedSum,
};
use oxiroot_io_core::{Compression, Error, FileReader, ReadRoot, Result, WriteRoot};

use crate::axis::Axis;
use crate::base::BinContentType;
use crate::{Hist1D, Hist2D, Hist3D, Profile1D, Profile2D, Profile3D};

/// Convert an oxihist histogram into the ROOT histogram that models it, and
/// write it to a ROOT file.
pub trait ToRoot {
    /// The ROOT histogram type it becomes.
    type Root: WriteRoot;

    /// The ROOT histogram, as a `TH1D`/`TH2D`/`TH3D` (`TH1L`/… for [`Int64`]
    /// storage) or a profile, stored with the given bin content type (`TH1F`
    /// for [`BinContentType::F32`], …; profiles have no content type, so it is
    /// ignored for them).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if an axis has more than `i32::MAX` bins, or is
    /// of a kind this version cannot map (oxihist's `AxisKind` is
    /// non-exhaustive).
    fn try_to_root_as(&self, content: BinContentType) -> Result<Self::Root>;

    /// [`try_to_root_as`](ToRoot::try_to_root_as) with the storage's own
    /// content type.
    ///
    /// # Errors
    ///
    /// As [`try_to_root_as`](ToRoot::try_to_root_as).
    fn try_to_root(&self) -> Result<Self::Root>;

    /// The ROOT histogram, as a `TH1D`/`TH2D`/`TH3D` (`TH1L`/… for [`Int64`]
    /// storage) or a profile.
    ///
    /// # Panics
    ///
    /// If ROOT cannot hold the histogram; see
    /// [`try_to_root`](ToRoot::try_to_root), which returns the error instead.
    #[must_use]
    fn to_root(&self) -> Self::Root {
        self.try_to_root().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like [`to_root`](ToRoot::to_root), stored with the given bin content
    /// type (`TH1F` for [`BinContentType::F32`], …). Profiles have no content
    /// type, so it is ignored for them.
    ///
    /// # Panics
    ///
    /// As [`to_root`](ToRoot::to_root).
    #[must_use]
    fn to_root_as(&self, content: BinContentType) -> Self::Root {
        self.try_to_root_as(content)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Write the histogram, under its name, as the only object of a new ROOT
    /// file at `path`.
    ///
    /// # Errors
    ///
    /// If ROOT cannot hold the histogram (see
    /// [`try_to_root`](ToRoot::try_to_root)), it has no name, or the file
    /// cannot be written.
    fn write_root(&self, path: impl AsRef<Path>, compression: Compression) -> Result<()> {
        self.try_to_root()?.write_root(path, compression)
    }
}

/// Read a ROOT histogram from a file as an oxihist histogram.
pub trait FromRoot: Sized {
    /// Read the histogram stored under key `name` in the file's top directory.
    ///
    /// # Errors
    ///
    /// If there is no such key, it is not a histogram of the matching kind, or
    /// an axis cannot be represented (no bins).
    fn read_root(file: &FileReader, name: &str) -> Result<Self>;

    /// Read the histogram stored under key `name` inside subdirectory `dir`.
    ///
    /// # Errors
    ///
    /// As [`read_root`](FromRoot::read_root).
    fn read_root_in(file: &FileReader, dir: &str, name: &str) -> Result<Self>;
}

// --- axes -------------------------------------------------------------------

/// The number of in-range bins the ROOT axis for `a` has: its size, but at
/// least one (a category axis with no categories yet still needs a bin).
fn root_nbins(a: &oxihist::Axis) -> usize {
    a.size().max(1)
}

/// `n` as a ROOT bin count.
fn root_count(n: usize) -> Result<i32> {
    i32::try_from(n).map_err(|_| {
        Error::InvalidInput(format!(
            "an axis of {n} bins does not fit a ROOT axis (at most i32::MAX)"
        ))
    })
}

/// The ROOT axis for an oxihist axis.
fn root_axis(a: &oxihist::Axis, name: &str) -> Result<Axis> {
    let labelled = |labels: Vec<String>| -> Result<Axis> {
        let n = labels.len().max(1);
        let mut ax = Axis::new(name, root_count(n)?, 0.0, n as f64);
        for (i, l) in labels.iter().enumerate() {
            ax.set_label(i + 1, l);
        }
        Ok(ax)
    };
    let mut ax = match a.kind() {
        AxisKind::Regular { bins, lo, hi } => Axis::new(name, root_count(*bins)?, *lo, *hi),
        AxisKind::Variable { edges } => {
            root_count(edges.len().saturating_sub(1))?;
            Axis::variable(name, edges)
        }
        AxisKind::Integer { lo, hi } => {
            Axis::new(name, root_count(a.size())?, *lo as f64, *hi as f64)
        }
        AxisKind::StrCategory { categories, .. } => labelled(categories.clone())?,
        AxisKind::IntCategory { categories, .. } => {
            labelled(categories.iter().map(ToString::to_string).collect())?
        }
        AxisKind::Boolean => labelled(vec!["false".into(), "true".into()])?,
        other => {
            return Err(Error::InvalidInput(format!(
                "no ROOT axis for the oxihist axis kind {other:?}"
            )));
        }
    };
    ax.title.clone_from(&a.label);
    Ok(ax)
}

/// The ROOT axes for a histogram's axes.
fn root_axes(axes: &[oxihist::Axis]) -> Result<Vec<Axis>> {
    ["xaxis", "yaxis", "zaxis"]
        .iter()
        .zip(axes)
        .map(|(n, a)| root_axis(a, n))
        .collect()
}

/// The oxihist axis for a ROOT axis.
fn hist_axis(a: &Axis) -> Result<oxihist::Axis> {
    let n = usize::saturating_from(a.nbins.max(0));
    let ax = if a.is_labelled() {
        let labels: Vec<String> = (1..=n)
            .map(|i| a.bin_label(i).unwrap_or_default().to_owned())
            .collect();
        oxihist::Axis::str_category(&labels)
    } else if !a.xbins.is_empty() {
        oxihist::Axis::try_variable(&a.xbins).map_err(|e| Error::InvalidInput(e.to_string()))?
    } else {
        oxihist::Axis::try_regular(n, a.xmin, a.xmax)
            .map_err(|e| Error::InvalidInput(e.to_string()))?
    };
    Ok(ax.with_label(a.title.clone()))
}

/// The ROOT cell (flow bins included, first axis fastest) of oxihist bin
/// `idx`, for oxihist axes of `sizes` written as ROOT axes of `nbins`
/// in-range bins (`nbins[k] >= sizes[k]`; they differ for an empty category
/// axis, whose overflow must still land in ROOT's overflow cell).
fn root_cell(idx: &[isize], sizes: &[usize], nbins: &[usize]) -> usize {
    let mut cell = 0;
    let mut stride = 1;
    for ((&i, &size), &n) in idx.iter().zip(sizes).zip(nbins) {
        let r = if i < 0 {
            0
        } else if i.cast_unsigned() >= size {
            n + 1
        } else {
            i.cast_unsigned() + 1
        };
        cell += r * stride;
        stride *= n + 2;
    }
    cell
}

/// The oxihist bin index of every ROOT cell, or `None` for a cell the oxihist
/// axes do not have (a labelled axis's underflow).
fn hist_index<const D: usize>(cell: usize, nbins: &[usize; D]) -> [isize; D] {
    let mut c = cell;
    std::array::from_fn(|k| {
        let e = nbins[k] + 2;
        let i = (c % e).cast_signed() - 1;
        c /= e;
        i
    })
}

// --- histograms ---------------------------------------------------------------

/// A count storage ROOT stores as a `TH1`/`TH2`/`TH3`.
trait RootCount: Storage {
    /// Whether the bins carry their own variances (`Sumw2`).
    const SUMW2: bool;
    /// The content type it is written with.
    const CONTENT: BinContentType;
}

impl RootCount for Double {
    const SUMW2: bool = false;
    const CONTENT: BinContentType = BinContentType::F64;
}

impl RootCount for Int64 {
    const SUMW2: bool = false;
    const CONTENT: BinContentType = BinContentType::I64;
}

impl RootCount for Weight {
    const SUMW2: bool = true;
    const CONTENT: BinContentType = BinContentType::F64;
}

/// A histogram's cells and summary moments, in ROOT's layout.
struct Cells {
    /// The oxihist axis sizes.
    sizes: Vec<usize>,
    /// The ROOT axes' in-range bin counts.
    nbins: Vec<usize>,
    contents: Vec<f64>,
    sumw2: Vec<f64>,
    entries: f64,
    /// `Σw`, `Σw²` over the in-range bins.
    sw: f64,
    sw2: f64,
    /// `Σw·xₖ` and `Σw·xₖ²` per axis, and `Σw·xᵢ·xⱼ` for (0,1), (0,2), (1,2).
    swx: [f64; 3],
    swx2: [f64; 3],
    swxx: [f64; 3],
}

fn cells<S: Storage, const D: usize>(
    h: &Histogram<S, D>,
    value: impl Fn(&S::Bin) -> f64,
    var: impl Fn(&S::Bin) -> f64,
) -> Cells {
    let sizes: Vec<usize> = h.axes().iter().map(oxihist::Axis::size).collect();
    let nbins: Vec<usize> = h.axes().iter().map(root_nbins).collect();
    let centers: Vec<Vec<f64>> = h.axes().iter().map(oxihist::Axis::centers).collect();
    let ncells: usize = nbins.iter().map(|n| n + 2).product();
    let mut c = Cells {
        sizes: sizes.clone(),
        nbins: nbins.clone(),
        contents: vec![0.0; ncells],
        sumw2: vec![0.0; ncells],
        entries: 0.0,
        sw: 0.0,
        sw2: 0.0,
        swx: [0.0; 3],
        swx2: [0.0; 3],
        swxx: [0.0; 3],
    };
    for (idx, b) in h.iter(true) {
        let (w, v) = (value(b), var(b));
        let cell = root_cell(&idx, &sizes, &nbins);
        c.contents[cell] = w;
        c.sumw2[cell] = v;
        c.entries += w;
        let in_range = idx
            .iter()
            .zip(&sizes)
            .all(|(&i, &n)| i >= 0 && (i).cast_unsigned() < n);
        if !in_range {
            continue;
        }
        let x: Vec<f64> = (0..D).map(|k| centers[k][idx[k].cast_unsigned()]).collect();
        c.sw += w;
        c.sw2 += v;
        for k in 0..D {
            c.swx[k] += w * x[k];
            c.swx2[k] += w * x[k] * x[k];
        }
        for (slot, (i, j)) in [(0, 1), (0, 2), (1, 2)].into_iter().enumerate() {
            if j < D {
                c.swxx[slot] += w * x[i] * x[j];
            }
        }
    }
    c
}

macro_rules! count_to_root {
    ($s:ty, $d:literal, $root:ty, |$h:ident, $c:ident, $axes:ident| $build:expr_2021) => {
        impl ToRoot for Histogram<$s, $d> {
            type Root = $root;

            fn try_to_root(&self) -> Result<$root> {
                self.try_to_root_as(<$s as RootCount>::CONTENT)
            }

            fn try_to_root_as(&self, content: BinContentType) -> Result<$root> {
                let $h = self;
                let $axes = root_axes(self.axes())?;
                let $c = cells(self, <$s>::value, <$s>::variance);
                let mut r: $root = $build;
                r.name.clone_from(&$h.name);
                r.title.clone_from(&$h.title);
                r.entries = $c.entries;
                r.tsumw = $c.sw;
                r.tsumw2 = $c.sw2;
                r.tsumwx = $c.swx[0];
                r.tsumwx2 = $c.swx2[0];
                r.contents = $c.contents;
                if <$s as RootCount>::SUMW2 {
                    r.sumw2 = $c.sumw2;
                }
                Ok(r.with_bin_content_type(content))
            }
        }
    };
}

macro_rules! counts_1d {
    ($($s:ty),*) => {$(
        count_to_root!($s, 1, Hist1D, |h, c, axes| {
            let mut r = Hist1D::new(i32::saturating_from(c.nbins[0]), 0.0, 1.0);
            r.xaxis = axes[0].clone();
            r
        });
    )*};
}
macro_rules! counts_2d {
    ($($s:ty),*) => {$(
        count_to_root!($s, 2, Hist2D, |h, c, axes| {
            let mut r = Hist2D::new(i32::saturating_from(c.nbins[0]), 0.0, 1.0, i32::saturating_from(c.nbins[1]), 0.0, 1.0);
            r.xaxis = axes[0].clone();
            r.yaxis = axes[1].clone();
            r.tsumwy = c.swx[1];
            r.tsumwy2 = c.swx2[1];
            r.tsumwxy = c.swxx[0];
            r
        });
    )*};
}
macro_rules! counts_3d {
    ($($s:ty),*) => {$(
        count_to_root!($s, 3, Hist3D, |h, c, axes| {
            let mut r = Hist3D::new(
                i32::saturating_from(c.nbins[0]), 0.0, 1.0,
                i32::saturating_from(c.nbins[1]), 0.0, 1.0,
                i32::saturating_from(c.nbins[2]), 0.0, 1.0,
            );
            r.xaxis = axes[0].clone();
            r.yaxis = axes[1].clone();
            r.zaxis = axes[2].clone();
            r.tsumwy = c.swx[1];
            r.tsumwy2 = c.swx2[1];
            r.tsumwxy = c.swxx[0];
            r.tsumwz = c.swx[2];
            r.tsumwz2 = c.swx2[2];
            r.tsumwxz = c.swxx[1];
            r.tsumwyz = c.swxx[2];
            r
        });
    )*};
}
counts_1d!(Double, Int64, Weight);
counts_2d!(Double, Int64, Weight);
counts_3d!(Double, Int64, Weight);

// --- profiles -------------------------------------------------------------------

/// A mean storage ROOT stores as a profile: per bin `Σw`, `Σw²` (`None` when
/// the weights are unit frequencies), `Σw·y` and `Σw·y²`.
trait RootMean: Storage {
    fn parts(bin: &Self::Bin) -> (f64, Option<f64>, f64, f64);
}

impl RootMean for Mean {
    fn parts(b: &MeanBin) -> (f64, Option<f64>, f64, f64) {
        let sum = b.count * b.value;
        (b.count, None, sum, b.sum_of_deltas_squared + sum * b.value)
    }
}

impl RootMean for WeightedMean {
    fn parts(b: &WeightedMeanBin) -> (f64, Option<f64>, f64, f64) {
        let sum = b.sum_of_weights * b.value;
        (
            b.sum_of_weights,
            Some(b.sum_of_weights_squared),
            sum,
            b.sum_of_weighted_deltas_squared + sum * b.value,
        )
    }
}

/// A profile's cells: per-cell `Σw`, `Σw²`, `Σw·y`, `Σw·y²`, plus the moments
/// of the bin weights (in [`Cells`]) and `Σw·y`, `Σw·y²` over the in-range bins.
struct ProfileCells {
    cells: Cells,
    sumw2: Option<Vec<f64>>,
    sums: Vec<f64>,
    sumy2: Vec<f64>,
    swy: f64,
    swy2: f64,
}

fn profile_cells<S: RootMean, const D: usize>(h: &Histogram<S, D>) -> ProfileCells {
    let cells = cells(
        h,
        |b| S::parts(b).0,
        |b| S::parts(b).1.unwrap_or(S::parts(b).0),
    );
    let n = cells.contents.len();
    let mut p = ProfileCells {
        sumw2: None,
        sums: vec![0.0; n],
        sumy2: vec![0.0; n],
        swy: 0.0,
        swy2: 0.0,
        cells,
    };
    let mut sumw2 = vec![0.0; n];
    let mut weighted = false;
    for (idx, b) in h.iter(true) {
        let (_, w2, sum, sum2) = S::parts(b);
        let cell = root_cell(&idx, &p.cells.sizes, &p.cells.nbins);
        p.sums[cell] = sum;
        p.sumy2[cell] = sum2;
        if let Some(w2) = w2 {
            sumw2[cell] = w2;
            weighted = true;
        }
        let in_range = idx
            .iter()
            .zip(&p.cells.sizes)
            .all(|(&i, &n)| i >= 0 && (i).cast_unsigned() < n);
        if in_range {
            p.swy += sum;
            p.swy2 += sum2;
        }
    }
    if weighted {
        p.sumw2 = Some(sumw2);
    }
    p
}

macro_rules! mean_to_root {
    ($s:ty, $d:tt, $root:ty, |$c:ident, $axes:ident| $build:expr_2021) => {
        impl ToRoot for Histogram<$s, $d> {
            type Root = $root;

            fn try_to_root(&self) -> Result<$root> {
                let $axes = root_axes(self.axes())?;
                let p = profile_cells(self);
                let $c = &p.cells;
                let mut r: $root = $build;
                r.name.clone_from(&self.name);
                r.title.clone_from(&self.title);
                r.entries = $c.entries;
                r.tsumw = $c.sw;
                r.tsumw2 = $c.sw2;
                r.tsumwx = $c.swx[0];
                r.tsumwx2 = $c.swx2[0];
                r.bin_entries = $c.contents.clone();
                r.sums = p.sums;
                if let Some(w2) = p.sumw2 {
                    r.bin_sumw2 = w2;
                }
                mean_to_root!(@moments $d, r, p, $c);
                Ok(r)
            }

            fn try_to_root_as(&self, _content: BinContentType) -> Result<$root> {
                self.try_to_root()
            }
        }
    };
    (@moments 1, $r:ident, $p:ident, $c:ident) => {
        $r.sumy2 = $p.sumy2;
        $r.tsumwy = $p.swy;
        $r.tsumwy2 = $p.swy2;
    };
    (@moments 2, $r:ident, $p:ident, $c:ident) => {
        $r.sumz2 = $p.sumy2;
        $r.tsumwy = $c.swx[1];
        $r.tsumwy2 = $c.swx2[1];
        $r.tsumwxy = $c.swxx[0];
        $r.tsumwz = $p.swy;
        $r.tsumwz2 = $p.swy2;
    };
    (@moments 3, $r:ident, $p:ident, $c:ident) => {
        $r.sumt2 = $p.sumy2;
        $r.tsumwy = $c.swx[1];
        $r.tsumwy2 = $c.swx2[1];
        $r.tsumwxy = $c.swxx[0];
        $r.tsumwz = $c.swx[2];
        $r.tsumwz2 = $c.swx2[2];
        $r.tsumwxz = $c.swxx[1];
        $r.tsumwyz = $c.swxx[2];
        $r.tsumwt = $p.swy;
        $r.tsumwt2 = $p.swy2;
    };
}

macro_rules! means {
    ($($s:ty),*) => {$(
        mean_to_root!($s, 1, Profile1D, |c, axes| {
            let mut r = Profile1D::new(i32::saturating_from(c.nbins[0]), 0.0, 1.0);
            r.xaxis = axes[0].clone();
            r
        });
        mean_to_root!($s, 2, Profile2D, |c, axes| {
            let mut r = Profile2D::new(i32::saturating_from(c.nbins[0]), 0.0, 1.0, i32::saturating_from(c.nbins[1]), 0.0, 1.0);
            r.xaxis = axes[0].clone();
            r.yaxis = axes[1].clone();
            r
        });
        mean_to_root!($s, 3, Profile3D, |c, axes| {
            let mut r = Profile3D::new(
                i32::saturating_from(c.nbins[0]), 0.0, 1.0,
                i32::saturating_from(c.nbins[1]), 0.0, 1.0,
                i32::saturating_from(c.nbins[2]), 0.0, 1.0,
            );
            r.xaxis = axes[0].clone();
            r.yaxis = axes[1].clone();
            r.zaxis = axes[2].clone();
            r
        });
    )*};
}
means!(Mean, WeightedMean);

// --- ROOT → oxihist ---------------------------------------------------------------

/// Fill an oxihist histogram over `axes` from ROOT cells, `bin(cell)` giving
/// each cell's accumulator.
fn from_cells<S: Storage, const D: usize>(
    axes: [oxihist::Axis; D],
    ncells: usize,
    name: &str,
    title: &str,
    bin: impl Fn(usize) -> S::Bin,
) -> Histogram<S, D> {
    let nbins: [usize; D] = std::array::from_fn(|k| axes[k].size());
    let mut h = Histogram::<S, D>::new(axes).named(name).titled(title);
    for cell in 0..ncells {
        if let Some(b) = h.bin_mut(hist_index(cell, &nbins)) {
            *b = bin(cell);
        }
    }
    h
}

fn weighted_sum(contents: &[f64], sumw2: &[f64], cell: usize) -> WeightedSum {
    let value = contents.get(cell).copied().unwrap_or(0.0);
    let variance = if sumw2.is_empty() {
        value.abs()
    } else {
        sumw2.get(cell).copied().unwrap_or(0.0)
    };
    WeightedSum { value, variance }
}

fn weighted_mean(
    entries: &[f64],
    sumw2: &[f64],
    sums: &[f64],
    sum2: &[f64],
    cell: usize,
) -> WeightedMeanBin {
    let sw = entries.get(cell).copied().unwrap_or(0.0);
    let sw2 = if sumw2.is_empty() {
        sw
    } else {
        sumw2.get(cell).copied().unwrap_or(0.0)
    };
    let (sum, s2) = (
        sums.get(cell).copied().unwrap_or(0.0),
        sum2.get(cell).copied().unwrap_or(0.0),
    );
    let value = if sw != 0.0 { sum / sw } else { 0.0 };
    WeightedMeanBin {
        sum_of_weights: sw,
        sum_of_weights_squared: sw2,
        value,
        sum_of_weighted_deltas_squared: (s2 - sum * value).max(0.0),
    }
}

impl Hist1D {
    /// This histogram as an oxihist one with [`Weight`] storage: the variances
    /// are the `Sumw2` array, or the contents when there is none.
    ///
    /// # Errors
    ///
    /// If the axis cannot be represented (no bins, or an empty range).
    pub fn to_hist(&self) -> Result<Histogram<Weight, 1>> {
        let axes = [hist_axis(&self.xaxis)?];
        Ok(from_cells(
            axes,
            self.contents.len(),
            &self.name,
            &self.title,
            |c| weighted_sum(&self.contents, &self.sumw2, c),
        ))
    }
}

impl Hist2D {
    /// This histogram as an oxihist one with [`Weight`] storage; see
    /// [`Hist1D::to_hist`].
    ///
    /// # Errors
    ///
    /// If an axis cannot be represented.
    pub fn to_hist(&self) -> Result<Histogram<Weight, 2>> {
        let axes = [hist_axis(&self.xaxis)?, hist_axis(&self.yaxis)?];
        Ok(from_cells(
            axes,
            self.contents.len(),
            &self.name,
            &self.title,
            |c| weighted_sum(&self.contents, &self.sumw2, c),
        ))
    }
}

impl Hist3D {
    /// This histogram as an oxihist one with [`Weight`] storage; see
    /// [`Hist1D::to_hist`].
    ///
    /// # Errors
    ///
    /// If an axis cannot be represented.
    pub fn to_hist(&self) -> Result<Histogram<Weight, 3>> {
        let axes = [
            hist_axis(&self.xaxis)?,
            hist_axis(&self.yaxis)?,
            hist_axis(&self.zaxis)?,
        ];
        Ok(from_cells(
            axes,
            self.contents.len(),
            &self.name,
            &self.title,
            |c| weighted_sum(&self.contents, &self.sumw2, c),
        ))
    }
}

impl Profile1D {
    /// This profile as an oxihist one with [`WeightedMean`] storage (unit
    /// weights when the profile tracks no `Σw²`).
    ///
    /// # Errors
    ///
    /// If the axis cannot be represented.
    pub fn to_hist(&self) -> Result<Histogram<WeightedMean, 1>> {
        let axes = [hist_axis(&self.xaxis)?];
        Ok(from_cells(
            axes,
            self.sums.len(),
            &self.name,
            &self.title,
            |c| {
                weighted_mean(
                    &self.bin_entries,
                    &self.bin_sumw2,
                    &self.sums,
                    &self.sumy2,
                    c,
                )
            },
        ))
    }
}

impl Profile2D {
    /// This profile as an oxihist one; see [`Profile1D::to_hist`].
    ///
    /// # Errors
    ///
    /// If an axis cannot be represented.
    pub fn to_hist(&self) -> Result<Histogram<WeightedMean, 2>> {
        let axes = [hist_axis(&self.xaxis)?, hist_axis(&self.yaxis)?];
        Ok(from_cells(
            axes,
            self.sums.len(),
            &self.name,
            &self.title,
            |c| {
                weighted_mean(
                    &self.bin_entries,
                    &self.bin_sumw2,
                    &self.sums,
                    &self.sumz2,
                    c,
                )
            },
        ))
    }
}

impl Profile3D {
    /// This profile as an oxihist one; see [`Profile1D::to_hist`].
    ///
    /// # Errors
    ///
    /// If an axis cannot be represented.
    pub fn to_hist(&self) -> Result<Histogram<WeightedMean, 3>> {
        let axes = [
            hist_axis(&self.xaxis)?,
            hist_axis(&self.yaxis)?,
            hist_axis(&self.zaxis)?,
        ];
        Ok(from_cells(
            axes,
            self.sums.len(),
            &self.name,
            &self.title,
            |c| {
                weighted_mean(
                    &self.bin_entries,
                    &self.bin_sumw2,
                    &self.sums,
                    &self.sumt2,
                    c,
                )
            },
        ))
    }
}

macro_rules! from_root {
    ($($hist:ty => $root:ty),* $(,)?) => {$(
        impl FromRoot for $hist {
            fn read_root(file: &FileReader, name: &str) -> Result<Self> {
                <$root as ReadRoot>::read_root(file, name)?.to_hist()
            }
            fn read_root_in(file: &FileReader, dir: &str, name: &str) -> Result<Self> {
                <$root as ReadRoot>::read_root_in(file, dir, name)?.to_hist()
            }
        }
    )*};
}
from_root!(
    Histogram<Weight, 1> => Hist1D,
    Histogram<Weight, 2> => Hist2D,
    Histogram<Weight, 3> => Hist3D,
    Histogram<WeightedMean, 1> => Profile1D,
    Histogram<WeightedMean, 2> => Profile2D,
    Histogram<WeightedMean, 3> => Profile3D,
);

#[cfg(test)]
mod tests {
    use oxihist::{Hist, Histogram, Weight, WeightedMean};

    use super::*;

    #[test]
    fn weight_1d_round_trips_through_root() {
        let mut h = Hist::var(&[0.0, 1.0, 5.0, 10.0])
            .label("x [cm]")
            .name("h")
            .title("t")
            .weight();
        h.fill_weight(0.5, 2.0);
        h.fill_weight(7.0, 3.0);
        h.fill(-1.0);
        h.fill(20.0);
        let r = h.to_root();
        assert_eq!(r.contents, vec![1.0, 2.0, 0.0, 3.0, 1.0]);
        assert_eq!(r.sumw2, vec![1.0, 4.0, 0.0, 9.0, 1.0]);
        assert_eq!((r.tsumw, r.tsumw2, r.entries), (5.0, 13.0, 7.0));
        assert_eq!(r.tsumwx, 2.0 * 0.5 + 3.0 * 7.5);
        assert_eq!(r.xaxis.title, "x [cm]");
        assert_eq!(r.class_name(), "TH1D");
        assert_eq!(r.to_hist().unwrap(), h);
    }

    #[test]
    fn storages_pick_the_root_class() {
        let mut d = Hist::reg(3, 0.0, 3.0).double();
        d.fill(1.5);
        let r = d.to_root();
        assert!(r.sumw2.is_empty());
        assert_eq!(r.contents[2], 1.0);
        assert_eq!(
            Hist::reg(3, 0.0, 3.0).int64().to_root().class_name(),
            "TH1L"
        );
        let f = d.to_root_as(BinContentType::F32);
        assert_eq!(f.class_name(), "TH1F");
    }

    #[test]
    fn categories_become_labelled_axes_and_back() {
        let mut h = Hist::str_cat(&["ee", "mumu"]).name("c").weight();
        h.fill("mumu");
        h.fill("tautau"); // overflow
        let r = h.to_root();
        assert_eq!(r.xaxis.bin_label(2), Some("mumu"));
        assert_eq!(r.contents, vec![0.0, 0.0, 1.0, 1.0]);
        let back = r.to_hist().unwrap();
        assert_eq!(back.axis(0).bin_label(1).as_deref(), Some("mumu"));
        assert_eq!(back.value([2]), Some(1.0)); // the overflow bin
    }

    #[test]
    fn two_d_layout_matches_root() {
        let mut h = Hist::reg(2, 0.0, 2.0).reg(3, 0.0, 3.0).weight();
        h.fill(1.5, 2.5);
        let r = h.to_root();
        // ROOT cell = ix + (nx + 2) * iy, with 1-based in-range bins.
        assert_eq!(r.contents[2 + 4 * 3], 1.0);
        assert_eq!(r.tsumwxy, 1.5 * 2.5);
        assert_eq!(r.to_hist().unwrap(), h);
        let h3 = Hist::reg(2, 0.0, 2.0)
            .reg(2, 0.0, 2.0)
            .reg(2, 0.0, 2.0)
            .weight();
        assert_eq!(h3.to_root().to_hist().unwrap(), h3);
    }

    #[test]
    fn profiles_round_trip() {
        let mut p = Hist::reg(2, 0.0, 2.0).name("p").weighted_mean();
        p.fill_sample_weight(0.5, 1.0, 2.0);
        p.fill_sample_weight(0.5, 4.0, 1.0);
        p.fill_sample(1.5, 3.0);
        let r = p.to_root();
        assert_eq!(r.bin_entries[1], 3.0);
        assert_eq!(r.sums[1], 6.0);
        assert_eq!(r.sumy2[1], 18.0);
        // ROOT's error on the mean uses the population spread over the
        // effective entries: here σ² = 18/3 − 2² = 2 and n_eff = 3²/5.
        assert!((r.bin_error(1) - (2.0f64 / 1.8).sqrt()).abs() < 1e-12);
        let back: Histogram<WeightedMean, 1> = r.to_hist().unwrap();
        for i in 0..2 {
            assert!((back.value([i]).unwrap() - p.value([i]).unwrap()).abs() < 1e-12);
            assert!((back.variance([i]).unwrap() - p.variance([i]).unwrap()).abs() < 1e-12);
        }
        let mut m = Hist::reg(2, 0.0, 2.0).mean();
        m.fill_sample(0.5, 1.0);
        m.fill_sample(0.5, 3.0);
        let r = m.to_root();
        assert!(r.bin_sumw2.is_empty());
        assert_eq!(r.to_hist().unwrap().value([0]), Some(2.0));
    }

    #[test]
    fn empty_category_axis_keeps_root_layout_consistent() {
        // No categories yet: ROOT still gets one (empty) bin, and the overflow
        // lands in ROOT's overflow cell, not in that bin.
        let empty: [&str; 0] = [];
        let mut h = Hist::str_cat(&empty).name("c").weight();
        h.fill("ee"); // overflow: the axis does not grow
        let r = h.try_to_root().unwrap();
        assert_eq!(r.xaxis.nbins, 1);
        assert_eq!(r.contents, vec![0.0, 0.0, 1.0]);
        assert_eq!(r.sumw2.len(), r.contents.len());
        assert_eq!((r.tsumw, r.entries), (0.0, 1.0));
        // Read back, it is a one-bin regular axis with the overflow kept.
        let back = r.to_hist().unwrap();
        assert_eq!(back.axis(0).size(), 1);
        assert_eq!(back.value([1]), Some(1.0));

        let mut p = Hist::str_cat(&empty).reg(2, 0.0, 2.0).mean();
        p.fill_sample("x", 0.5, 3.0);
        let r = p.try_to_root().unwrap();
        assert_eq!(r.sums.len(), 3 * 4);
        assert_eq!(r.sums[2 + 3], 3.0); // x overflow (cell 2), y bin 1 (stride 3)
    }

    #[test]
    fn lossy_axis_features_are_dropped_as_documented() {
        // Integer → Regular over the same bins.
        let mut h = Hist::int(-2, 3).weight();
        h.fill(0);
        let back = h.to_root().to_hist().unwrap();
        assert!(matches!(
            back.axis(0).kind(),
            AxisKind::Regular { bins: 5, lo, hi } if *lo == -2.0 && *hi == 3.0
        ));
        assert_eq!(back.value([2]), Some(1.0));

        // IntCategory → StrCategory over the labels; growth and the axis name
        // are not kept, the label is.
        let mut h = Hist::int_cat(&[11, 13])
            .growth()
            .axis_name("pdg")
            .label("PDG id")
            .weight();
        h.fill(13);
        let back = h.to_root().to_hist().unwrap();
        match back.axis(0).kind() {
            AxisKind::StrCategory { categories, growth } => {
                assert_eq!(categories, &["11".to_owned(), "13".to_owned()]);
                assert!(!growth);
            }
            other => panic!("expected a string category axis, got {other:?}"),
        }
        assert_eq!(back.axis(0).label, "PDG id");
        assert_ne!(back.axis(0).name, "pdg");
        assert_eq!(back.value([1]), Some(1.0));

        // fEntries is the sum of the bin values, not the fill count.
        let mut h = Hist::reg(2, 0.0, 2.0).weight();
        h.fill_weight(0.5, 2.5);
        assert_eq!(h.to_root().entries, 2.5);
    }

    #[test]
    fn bin_counts_beyond_i32_are_refused() {
        assert!(root_count(i32::MAX as usize).is_ok());
        let e = root_count(i32::MAX as usize + 1).unwrap_err();
        assert!(e.to_string().contains("i32::MAX"), "{e}");
    }

    #[test]
    fn written_files_read_back() {
        let dir = std::env::temp_dir().join(format!("oxiroot-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.root");
        let mut h = Hist::reg(4, 0.0, 4.0).reg(2, 0.0, 2.0).name("h2").weight();
        h.fill_weight(1.5, 0.5, 2.5);
        h.write_root(&path, Compression::Zstd(3)).unwrap();
        let file = FileReader::open(&path).unwrap();
        let back = Histogram::<Weight, 2>::read_root(&file, "h2").unwrap();
        assert_eq!(back, h);
        assert!(
            Hist::reg(1, 0.0, 1.0)
                .weight()
                .write_root(&path, Compression::None)
                .is_err()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
