//! Fitting for histograms and graphs.
//!
//! The fitting engine lives in the standalone [`oxifit`] crate. This module
//! teaches it to read a [`Hist1D`] and a [`Graph`] (by implementing
//! [`FitData`]) and a [`Hist2D`] and a [`Hist3D`] (by implementing
//! [`FitDataNd`]), so the `.fit(...)` methods (from [`FitExt`]) work directly —
//! a 2-D or 3-D histogram with a model of as many variables:
//!
//! ```ignore
//! use oxiroot_hist::{Model, FitExt};
//! let model = Model::gaussian("g").estimate_from(&h);
//! let fit = h.fit(&model); // or g.fit(&model) for a Graph
//! println!("mean = {} ± {}", fit.params[1], fit.errors[1]);
//! ```
//!
//! Behind the `fit` feature (it pulls in `oxifit` → the pure-Rust Minuit2
//! port). The whole `oxifit` surface is re-exported here for convenience.

pub use oxifit::{
    FitData, FitExt, FitMethod, FitOptions, FitResult, Minimizer, Model, Point, Points,
};

use crate::graph::{Graph, GraphErrors};

/// Each graph point becomes `(x, y, σ)`, with `σ` the y-error bar: the symmetric
/// `ey`, the mean of the asymmetric `(ey_low, ey_high)`, or `1.0` (an unweighted
/// least-squares fit) when the graph carries no errors.
impl FitData for Graph {
    fn points(&self) -> Vec<Point> {
        self.x
            .iter()
            .zip(&self.y)
            .enumerate()
            .map(|(i, (&x, &y))| Point::new(x, y, graph_sigma(&self.errors, i)))
            .collect()
    }
}

fn graph_sigma(errors: &GraphErrors, i: usize) -> f64 {
    match errors {
        GraphErrors::Plain => 1.0,
        GraphErrors::Symmetric { ey, .. } => ey.get(i).copied().unwrap_or(1.0),
        GraphErrors::Asymmetric {
            ey_low, ey_high, ..
        } => {
            let lo = ey_low.get(i).copied().unwrap_or(0.0);
            let hi = ey_high.get(i).copied().unwrap_or(0.0);
            (lo + hi) / 2.0
        }
    }
}
