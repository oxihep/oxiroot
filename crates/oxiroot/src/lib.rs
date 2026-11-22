//! `oxiroot`: pure-Rust IO for the CERN ROOT file format.
//!
//! Read and write [`RNTuple`](oxiroot_rntuple) (ROOT's columnar event-data
//! format), classic [`TTree`](oxiroot_tree), the [histogram](oxiroot_hist)
//! family (`Hist1D`/`Hist2D`/`Hist3D`, `Profile1D`/`2D`/`3D`, `Efficiency`, `SparseHist`,
//! `PolyHist`), and [graphs](oxiroot_hist::Graph) (`Graph`/`TGraphErrors`/
//! `TGraphAsymmErrors`, plus `Graph2D` and `MultiErrorGraph`) in the ROOT (`TFile`) container, with no C++/libROOT
//! dependency. Files written here are read by official ROOT and uproot, and
//! vice versa.
//!
//! # Quick start
//!
//! ```no_run
//! use oxiroot::prelude::*;
//!
//! // Fill a histogram (the Hist builder; `weight()` tracks Sumw2), then save
//! // it: `to_root()` is the TH1D a file stores.
//! let mut h = Hist::reg(50, 0.0, 100.0).name("pt").title("transverse momentum").weight();
//! h.fill_weight(42.0, 1.5);
//! FileWriter::create("out.root").add(&h.to_root()?).write()?;
//!
//! // Write a columnar dataset — an RNTuple here, `Format::Tree` for a TTree —
//! // next to the histogram, then read it back.
//! let batch = Batch::new().column("mass", vec![91.2_f64, 125.0])?;
//! FileWriter::create("data.root")
//!     .add(&h.to_root()?)
//!     .add(&NamedBatch::new("events", Format::Ntuple, batch))
//!     .write()?;
//! let f = FileReader::open("data.root")?;
//! let events = Dataset::open(&f, "events")?;
//! assert_eq!(events.read(["mass"])?.values::<f64>("mass")?, [91.2, 125.0]);
//! # Ok::<(), oxiroot::Error>(())
//! ```
//!
//! # Datasets
//!
//! [`data`] reads and writes ROOT's two columnar formats, `TTree` and `RNTuple`,
//! through one API: a [`Batch`](data::Batch) of flat columns is what
//! [`Dataset`](data::Dataset) reads and what a [`NamedBatch`](data::NamedBatch) holds,
//! whatever the format. A named batch is added to a [`FileWriter`] like a
//! histogram; [`DatasetWriter`](data::DatasetWriter) streams a dataset too
//! large to hold. The format-specific readers and writers in
//! [`tree`] and [`ntuple`] remain for what only one format has.
//!
//! # Naming
//!
//! Types are named as Rust, not as C++: no `T` prefix. Types that read a file
//! end in `Reader` ([`FileReader`], [`TreeReader`](tree::TreeReader),
//! [`NtupleReader`](ntuple::NtupleReader)), and types that write one end in
//! `Writer` ([`FileWriter`], [`DatasetWriter`](data::DatasetWriter)). A
//! columnar dataset, in either format, is a [`Dataset`](data::Dataset). The
//! rest is in-memory data named
//! for what it is — [`Hist1D`](hist::Hist1D), [`Profile1D`](hist::Profile1D),
//! [`Graph`](hist::Graph), [`Func1D`](hist::Func1D), [`Batch`](data::Batch).
//!
//! Every ROOT class name is a doc alias of the type that models it, so searching
//! the docs for `TH1`, `TProfile`, `TGraph`, `TF1`, `TFile`, `TTree`, `TChain`
//! or `RNTuple` lands on the right page. None of this touches the file: a
//! `Hist1D` still reads and writes a `TH1D`, and `class_name()` reports the ROOT
//! class it came from. ROOT's own spelling survives in one place in the API —
//! the dynamic [`Value`] tree, whose member keys (`fName`, `fBins`, …) are the
//! file's own field names, read from its `TStreamerInfo`.
//!
//! The flat [`prelude`] covers the common read/write surface; the [`data`],
//! [`hist`], [`ntuple`], [`tree`], [`compress`], and [`file`](mod@file)
//! modules expose everything else.

mod error;

pub use error::{Error, Result};
#[doc(inline)]
pub use oxiroot_io_core::{Compression, FileReader, FileUpdater, FileWriter, Value};

/// The ROOT file container and the object framework: reading and writing files,
/// keys and directories, streamer info, and the traits objects implement
/// (re-exported from `oxiroot-io-core`).
pub mod file {
    pub use oxiroot_io_core::*;
}

pub mod hadd;

/// One API to read and write `TTree` and `RNTuple`: [`Batch`](data::Batch)es of
/// flat columns, read by [`Dataset`](data::Dataset) and written as a
/// [`NamedBatch`](data::NamedBatch) in a [`FileWriter`], or streamed by a
/// [`DatasetWriter`](data::DatasetWriter) (from `oxiroot-dataset`).
pub mod data {
    pub use oxiroot_dataset::*;
}

/// ROOT compression framing and codecs (re-exported from `oxiroot-compress`).
pub mod compress {
    pub use oxiroot_compress::*;
}

/// Classic ROOT histograms, profiles, graphs and functions —
/// `Hist1D`/`Hist2D`/`Hist3D`/`Profile1D`/`Graph`/`Func1D`… (from `oxiroot-hist` and
/// `oxiroot-hist-func`).
pub mod hist {
    pub use oxiroot_hist::*;
    pub use oxiroot_hist_func::{Func1D, Func2D, Func3D};
}

/// ROOT linear-algebra objects — `Vector`/`Matrix`/`SymMatrix` (from
/// `oxiroot-linalg`), with byte-exact ROOT read/write.
pub mod linalg {
    pub use oxiroot_linalg::*;
}

/// `RNTuple`, ROOT's columnar event-data format, low level (from
/// `oxiroot-rntuple`). For one API over `TTree` and `RNTuple`, see [`data`].
pub mod ntuple {
    pub use oxiroot_rntuple::*;
}

/// Classic `TTree` columnar storage, low level (from `oxiroot-tree`). For one
/// API over `TTree` and `RNTuple`, see [`data`].
pub mod tree {
    pub use oxiroot_tree::*;
}

/// Pure-Rust statistics — special functions (`erf`, `gammainc`, `betainc`,
/// `ndtri`), distributions (`Normal`/`StudentT`/`ChiSquared`/`FisherF`),
/// descriptive stats (`skew`, `kurtosis`, `sem`, …), correlation (`pearsonr`,
/// `spearmanr`), and hypothesis tests (`ttest_ind`, `normaltest`) — from
/// `oxistats`, verified against `scipy.stats`.
pub mod stat {
    pub use oxistats::*;
}

/// PDG particle data — the [`PdgId`](oxiparticle::PdgId) Monte Carlo
/// numbering-scheme decoder (is it a lepton? meson? baryon? its charge/spin/quark
/// content) plus a bundled [`Particle`](oxiparticle::Particle) table (mass,
/// width, lifetime, quantum numbers). From `oxiparticle`, a Rust take on
/// scikit-hep `particle` and verified against it.
pub mod particle {
    pub use oxiparticle::*;
}

/// Curve fitting for any 1-D data — histograms, graphs, or custom points (from
/// `oxifit`). The [`FitData`](oxifit::FitData) trait + the blanket
/// [`FitExt`](oxifit::FitExt) give `data.fit(&model)` to every dataset;
/// `hist`'s `Hist1D`/`Graph` implement `FitData` (under the `fit` feature).
#[cfg(feature = "fit")]
pub mod fit {
    pub use oxifit::*;
}

/// Plotting — render histograms and graphs to SVG/PNG with a matplotlib-like
/// API and an mplhep histogram style (from `oxiplot`). `Axes::hist`/
/// `errorbar`/`hist2d` draw `Hist1D`/`Graph`/`Hist2D`; `$…$` labels are typeset as
/// LaTeX math. Enabled by the `plot` feature.
#[cfg(feature = "plot")]
pub mod plot {
    pub use oxiplot::*;
}

/// The common types for reading and writing ROOT files.
///
/// `use oxiroot::prelude::*;` brings in the file reader and writer
/// ([`FileReader`], [`FileWriter`], [`FileUpdater`]), [`Compression`], the
/// traits objects are read and written through, the common histogram,
/// profile, graph and function types, and the dataset API for `TTree` and
/// `RNTuple` ([`Dataset`](crate::data::Dataset),
/// [`NamedBatch`](crate::data::NamedBatch), [`DatasetWriter`](crate::data::DatasetWriter),
/// [`Batch`](crate::data::Batch), [`Format`](crate::data::Format)). Everything else is in its module.
pub mod prelude {
    // `Error` and `Result` are deliberately not here: a glob import would shadow
    // `std::result::Result`. Name them as `oxiroot::Error` / `oxiroot::Result`.
    //
    // Only what reading and writing a file takes. The rest has one path, in
    // its module: fitting in `oxiroot::fit`, statistics in `oxiroot::stat`,
    // collections and parameters in `oxiroot::file`, matrices in
    // `oxiroot::linalg`, merging in `oxiroot::hadd`, and the less common
    // histogram types in `oxiroot::hist`.
    pub use oxiroot_io_core::{
        Compression, FileReader, FileUpdater, FileWriter, ReadRoot, WriteRoot,
    };

    /// The ROOT-free histogram the `Hist` builder makes (from oxihist);
    /// `.to_root()?` turns it into a `Hist1D`/`Hist2D`/…
    pub use oxiroot_hist::oxihist::Histogram;
    pub use oxiroot_hist::{
        Axis, Efficiency, Graph, Hist, Hist1D, Hist2D, Hist3D, Mergeable, Profile1D, Profile2D,
        Profile3D, ToRoot,
    };
    pub use oxiroot_hist_func::{Func1D, Func2D, Func3D};

    pub use oxiroot_dataset::{Batch, Dataset, DatasetWriter, Format, NamedBatch};
}
