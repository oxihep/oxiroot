//! The [oxiplot](https://github.com/oxihep/oxiplot) data traits for the
//! histogram and graph types (the `plot` feature), so `Axes::hist(&h)`,
//! `Axes::profile(&p)`, `Axes::hist2d(&h2)` and `Axes::errorbar(&g)` draw them.
//!
//! oxiplot knows nothing about ROOT; it plots anything that implements
//! [`Hist1dData`], [`Hist2dData`] or [`PointData`], and these impls are where
//! the ROOT types meet it.

use oxiplot::PointData;

use crate::{Graph, GraphErrors};

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
    use oxiplot::PointData;

    use crate::{Graph, Hist, ToRoot};

    #[test]
    fn graph_errors_are_padded_and_asymmetric_ones_kept() {
        let g = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0])
            .and_then(|g| {
                g.with_errors(crate::GraphErrors::Symmetric {
                    ex: vec![0.1, 0.1],
                    ey: vec![0.2, 0.3],
                })
            })
            .unwrap();
        assert_eq!(g.y_errors(), Some((vec![0.2, 0.3], vec![0.2, 0.3])));
        let plain = Graph::new(vec![1.0], vec![2.0]).unwrap();
        assert_eq!(plain.x_errors(), None);
    }

    #[test]
    fn a_graph_takes_its_time_format_from_its_frame() {
        // A graph takes it from its display frame, as ROOT stores it.
        let mut frame = Hist::reg(2, 0.0, 7200.0)
            .double()
            .to_root()
            .unwrap()
            .with_bin_content_type(crate::BinContentType::F32)
            .named("Graph");
        frame.xaxis.set_time_format("%H:%M");
        let mut g = Graph::new(vec![0.0, 3600.0], vec![1.0, 2.0]).unwrap();
        g.histogram = Some(frame);
        assert_eq!(PointData::x_time_format(&g).as_deref(), Some("%H:%M"));
    }
}
