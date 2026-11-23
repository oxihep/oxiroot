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
    Formula(oxiformula::ParseError),
    /// A dataset query does not check, or its values do not fit
    /// ([`Dataset::query`](crate::data::Dataset::query)).
    Query(oxiroot_dataset::query::QueryError),
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
            Self::Root(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root(e) => fmt::Display::fmt(e, f),
            Self::Stat(e) => write!(f, "statistics: {e}"),
            Self::Formula(e) => write!(f, "invalid formula: {e}"),
            Self::Query(e) => write!(f, "query: {e}"),
            Self::Hist(e) => write!(f, "histogram: {e}"),
            #[cfg(feature = "plot")]
            Self::Plot(e) => write!(f, "plotting: {e}"),
            #[cfg(feature = "fit")]
            Self::Fit(e) => write!(f, "fitting: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Root(e) => Some(e),
            Self::Stat(e) => Some(e),
            Self::Formula(e) => Some(e),
            Self::Query(e) => Some(e),
            Self::Hist(e) => Some(e),
            #[cfg(feature = "plot")]
            Self::Plot(e) => Some(e),
            #[cfg(feature = "fit")]
            Self::Fit(e) => Some(e),
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
    Stat(oxistats::StatError),
    Formula(oxiformula::ParseError),
    Hist(oxihist::Error),
    #[cfg(feature = "plot")]
    Plot(oxiplot::Error),
    #[cfg(feature = "fit")]
    Fit(oxifit::FitError),
);

/// A ROOT-side error, except the formula and query errors one carries:
/// building a `Func1D` from a formula that does not parse is
/// [`Error::Formula`], and a dataset query that does not check is
/// [`Error::Query`], each saying where and how.
impl From<oxiroot_io_core::Error> for Error {
    fn from(e: oxiroot_io_core::Error) -> Self {
        match e.downcast::<oxiformula::ParseError>() {
            Ok(formula) => Self::Formula(formula),
            Err(e) => match e.downcast::<oxiroot_dataset::query::QueryError>() {
                Ok(query) => Self::Query(query),
                Err(e) => Self::Root(e),
            },
        }
    }
}

/// I/O errors (e.g. from `std::fs`) arrive as ROOT-side I/O errors.
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Root(e.into())
    }
}
