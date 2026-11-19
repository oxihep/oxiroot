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
    FitData, FitDataNd, FitExt, FitMethod, FitOptions, FitResult, Minimizer, Model, Point, PointNd,
    Points,
};

use crate::graph::{Graph, GraphErrors};
use crate::hist1d::Hist1D;
use crate::hist2d::Hist2D;
use crate::hist3d::Hist3D;

/// Each in-range bin becomes a point at its `(x, y)` center with its content
/// and error (ROOT's global bin `ix + (nx + 2)·iy`, flow excluded).
impl FitDataNd for Hist2D {
    fn dim(&self) -> usize {
        2
    }
    fn points_nd(&self) -> Vec<PointNd> {
        let (nx, ny) = (
            self.xaxis.nbins.max(0) as usize,
            self.yaxis.nbins.max(0) as usize,
        );
        let mut out = Vec::with_capacity(nx * ny);
        for iy in 1..=ny {
            for ix in 1..=nx {
                let bin = ix + (nx + 2) * iy;
                let v = self.contents.get(bin).copied().unwrap_or(0.0);
                let at = [self.xaxis.bin_center(ix), self.yaxis.bin_center(iy), 0.0];
                out.push(PointNd::new(at, v, self.bin_error(bin)));
            }
        }
        out
    }
}

/// Each in-range bin becomes a point at its `(x, y, z)` center with its
/// content and error (global bin `ix + sx·(iy + sy·iz)`, flow excluded).
impl FitDataNd for Hist3D {
    fn dim(&self) -> usize {
        3
    }
    fn points_nd(&self) -> Vec<PointNd> {
        let (nx, ny, nz) = (
            self.xaxis.nbins.max(0) as usize,
            self.yaxis.nbins.max(0) as usize,
            self.zaxis.nbins.max(0) as usize,
        );
        let (sx, sy) = (nx + 2, ny + 2);
        let mut out = Vec::with_capacity(nx * ny * nz);
        for iz in 1..=nz {
            for iy in 1..=ny {
                for ix in 1..=nx {
                    let bin = ix + sx * (iy + sy * iz);
                    let v = self.contents.get(bin).copied().unwrap_or(0.0);
                    let at = [
                        self.xaxis.bin_center(ix),
                        self.yaxis.bin_center(iy),
                        self.zaxis.bin_center(iz),
                    ];
                    out.push(PointNd::new(at, v, self.bin_error(bin)));
                }
            }
        }
        out
    }
}

/// Each in-range bin becomes a point `(center, content, error)`. An empty bin
/// has error 0, so Neyman chi-square drops it (as ROOT does); the other costs
/// keep it.
impl FitData for Hist1D {
    fn points(&self) -> Vec<Point> {
        let n = self.xaxis.nbins.max(0) as usize;
        (1..=n)
            .map(|i| {
                let y = self.contents.get(i).copied().unwrap_or(0.0);
                Point::new(self.bin_center(i), y, self.bin_error(i))
            })
            .collect()
    }
}

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
        GraphErrors::None => 1.0,
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
