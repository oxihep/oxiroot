//! The error type of the `oxiroot` facade.
//!
//! The ROOT crates share [`oxiroot_io_core::Error`] (re-exported as
//! [`file::Error`](crate::file::Error)), which knows nothing about statistics,
//! formulas, histograms or plotting. This type gathers all of them, so `?`
//! works across the whole API in a function returning [`crate::Result`]:
//!
//! ```no_run
//! use oxiroot::prelude::*;
//!
//! fn run() -> oxiroot::Result<()> {
//!     let f = FileReader::open("data.root")?;                      // a ROOT error
//!     let (r, _) = oxiroot::stat::pearsonr(&[1.0, 2.0], &[2.0, 4.0])?; // a statistics error
//!     # let _ = (f, r);
//!     Ok(())
//! }
//! ```

use std::fmt;

/// Any error the oxiroot API can return.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Reading or writing a ROOT file, or a ROOT object.
    Root(oxiroot_io_core::Error),
    /// A statistics function rejected its input.
    Stat(oxistats::StatError),
    /// A formula did not parse.
    Formula(oxifit_formula::ParseError),
    /// An oxihist histogram operation failed.
    Hist(oxihist::Error),
    /// Building or saving a figure failed (the `plot` feature).
    #[cfg(feature = "plot")]
    Plot(oxiplot::Error),
    /// A fit-side operation failed: a scan, a hypothesis test, a sampler or
    /// a template model (the `fit` feature).
    #[cfg(feature = "fit")]
    Fit(oxifit::FitError),
}

/// The result type of the oxiroot API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// The ROOT-side error, if this is one.
    #[must_use]
    pub fn as_root(&self) -> Option<&oxiroot_io_core::Error> {
        match self {
            Error::Root(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Root(e) => fmt::Display::fmt(e, f),
            Error::Stat(e) => write!(f, "statistics: {e}"),
            Error::Formula(e) => write!(f, "invalid formula: {e}"),
            Error::Hist(e) => write!(f, "histogram: {e}"),
            #[cfg(feature = "plot")]
            Error::Plot(e) => write!(f, "plotting: {e}"),
            #[cfg(feature = "fit")]
            Error::Fit(e) => write!(f, "fitting: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Root(e) => Some(e),
            Error::Stat(e) => Some(e),
            Error::Formula(e) => Some(e),
            Error::Hist(e) => Some(e),
            #[cfg(feature = "plot")]
            Error::Plot(e) => Some(e),
            #[cfg(feature = "fit")]
            Error::Fit(e) => Some(e),
        }
    }
}

macro_rules! from {
    ($($(#[$m:meta])* $variant:ident($ty:ty)),* $(,)?) => {$(
        $(#[$m])*
        impl From<$ty> for Error {
            fn from(e: $ty) -> Self {
                Error::$variant(e)
            }
        }
    )*};
}
from!(
    Root(oxiroot_io_core::Error),
    Stat(oxistats::StatError),
    Formula(oxifit_formula::ParseError),
    Hist(oxihist::Error),
    #[cfg(feature = "plot")]
    Plot(oxiplot::Error),
    #[cfg(feature = "fit")]
    Fit(oxifit::FitError),
);

/// I/O errors (e.g. from `std::fs`) arrive as ROOT-side I/O errors.
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Root(e.into())
    }
}
