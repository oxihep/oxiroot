//! The [oxiplot](https://github.com/oxihep/oxiplot) data traits for the
//! histogram and graph types (the `plot` feature), so `Axes::hist(&h)`,
//! `Axes::profile(&p)`, `Axes::hist2d(&h2)` and `Axes::errorbar(&g)` draw them.
//!
//! oxiplot knows nothing about ROOT; it plots anything that implements
//! [`Hist1dData`], [`Hist2dData`] or [`PointData`], and these impls are where
//! the ROOT types meet it.

use oxiplot::{Hist1dData, Hist2dData, PointData};

use crate::{Graph, GraphErrors, Hist1D, Hist2D, Profile1D};

impl Hist1dData for Hist1D {
    fn edges(&self) -> Vec<f64> {
        Hist1D::edges(self)
    }
    fn values(&self) -> Vec<f64> {
        Hist1D::values(self).to_vec()
    }
    fn error(&self, i: usize) -> f64 {
        self.bin_error(i + 1)
    }
    fn x_time_format(&self) -> Option<String> {
        self.xaxis
            .time_display
            .then(|| self.xaxis.time_format.clone())
    }
}

impl Hist1dData for Profile1D {
    fn edges(&self) -> Vec<f64> {
        Profile1D::edges(self)
    }
    fn values(&self) -> Vec<f64> {
        Profile1D::values(self)
    }
    fn error(&self, i: usize) -> f64 {
        self.bin_error(i + 1)
    }
}

impl Hist2dData for Hist2D {
    fn x_edges(&self) -> Vec<f64> {
        self.xaxis.edges()
    }
    fn y_edges(&self) -> Vec<f64> {
        self.yaxis.edges()
    }
    fn grid(&self) -> Vec<Vec<f64>> {
        self.values()
    }
}

/// Error arrays shorter than the graph are padded with zeros.
fn padded(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = v.to_vec();
    out.resize(n, 0.0);
    out
}

impl PointData for Graph {
    fn xs(&self) -> Vec<f64> {
        self.x[..self.len()].to_vec()
    }
    fn ys(&self) -> Vec<f64> {
        self.y[..self.len()].to_vec()
    }
    fn x_errors(&self) -> Option<(Vec<f64>, Vec<f64>)> {
        let n = self.len();
        match &self.errors {
            GraphErrors::Symmetric { ex, .. } => Some((padded(ex, n), padded(ex, n))),
            GraphErrors::Asymmetric {
                ex_low, ex_high, ..
            } => Some((padded(ex_low, n), padded(ex_high, n))),
            _ => None,
        }
    }
    fn y_errors(&self) -> Option<(Vec<f64>, Vec<f64>)> {
        let n = self.len();
        match &self.errors {
            GraphErrors::Symmetric { ey, .. } => Some((padded(ey, n), padded(ey, n))),
            GraphErrors::Asymmetric {
                ey_low, ey_high, ..
            } => Some((padded(ey_low, n), padded(ey_high, n))),
            _ => None,
        }
    }
    fn x_time_format(&self) -> Option<String> {
        // A graph's axes live on its display frame, as in ROOT.
        let axis = &self.histogram.as_ref()?.xaxis;
        axis.time_display.then(|| axis.time_format.clone())
    }
}

#[cfg(test)]
mod tests {
    use oxiplot::{Axes, Hist1dData, Hist2dData, PointData};

    use crate::{Graph, Hist, ToRoot};

    #[test]
    fn hist1d_exposes_in_range_bins_and_errors() {
        let mut h = Hist::reg(4, 0.0, 4.0).double().to_root().named("h");
        h.track_sumw2();
        h.fill(-1.0); // underflow: not drawn
        h.fill_weight(1.5, 4.0);
        assert_eq!(Hist1dData::edges(&h), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(Hist1dData::values(&h), vec![0.0, 4.0, 0.0, 0.0]);
        assert_eq!(Hist1dData::error(&h, 1), 4.0); // √Σw² = √16
    }

    #[test]
    fn profile_draws_its_bin_means() {
        let mut p = Hist::reg(2, 0.0, 2.0).mean().to_root().named("p");
        p.fill(0.5, 1.0);
        p.fill(0.5, 3.0);
        assert_eq!(Hist1dData::values(&p)[0], 2.0);
    }

    #[test]
    fn hist2d_grid_is_indexed_x_then_y() {
        let mut h = Hist::reg(2, 0.0, 2.0)
            .reg(3, 0.0, 3.0)
            .double()
            .to_root()
            .named("h2");
        h.fill_weight(1.5, 0.5, 7.0);
        let grid = h.grid();
        assert_eq!((grid.len(), grid[0].len()), (2, 3));
        assert_eq!(grid[1][0], 7.0);
    }

    #[test]
    fn graph_errors_are_padded_and_asymmetric_ones_kept() {
        let g = Graph::with_errors(
            vec![1.0, 2.0],
            vec![3.0, 4.0],
            vec![0.1, 0.1],
            vec![0.2, 0.3],
        )
        .unwrap();
        assert_eq!(g.y_errors(), Some((vec![0.2, 0.3], vec![0.2, 0.3])));
        let plain = Graph::new(vec![1.0], vec![2.0]).unwrap();
        assert_eq!(plain.x_errors(), None);
    }

    #[test]
    fn time_axes_carry_their_format() {
        let mut h = Hist::reg(24, 0.0, 86_400.0)
            .double()
            .to_root()
            .named("rate");
        h.xaxis.set_time_format("%H:%M%F2024-01-01 00:00:00");
        assert_eq!(
            Hist1dData::x_time_format(&h).as_deref(),
            Some("%H:%M%F2024-01-01 00:00:00")
        );
        let plain = Hist::reg(4, 0.0, 4.0).double().to_root().named("plain");
        assert_eq!(Hist1dData::x_time_format(&plain), None);

        // A graph takes it from its display frame, as ROOT stores it.
        let mut frame = Hist::reg(2, 0.0, 7200.0)
            .double()
            .to_root_as(crate::BinContentType::F32)
            .named("Graph");
        frame.xaxis.set_time_format("%H:%M");
        let mut g = Graph::new(vec![0.0, 3600.0], vec![1.0, 2.0]).unwrap();
        g.histogram = Some(frame);
        assert_eq!(PointData::x_time_format(&g).as_deref(), Some("%H:%M"));
    }

    #[test]
    fn every_type_renders() {
        let mut h = Hist::reg(10, 0.0, 10.0).double().to_root().named("h");
        h.fill(3.0);
        let mut p = Hist::reg(10, 0.0, 10.0).mean().to_root().named("p");
        p.fill(3.0, 1.0);
        let h2 = Hist::reg(4, 0.0, 1.0)
            .reg(4, 0.0, 1.0)
            .double()
            .to_root()
            .named("h2");
        let g = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0]).unwrap();

        let mut ax = Axes::new();
        ax.hist(&h).profile(&p).errorbar(&g);
        let mut ax2 = Axes::new();
        ax2.hist2d(&h2);
        for svg in [ax.to_svg_string(), ax2.to_svg_string()] {
            assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        }
    }
}
