//! The [oxiplot](https://github.com/oxihep/oxiplot) data trait for a graph
//! and an efficiency (the `plot` feature), so `Axes::errorbar(&g)` draws
//! them.
//!
//! oxiplot knows nothing about ROOT; it plots anything that implements
//! `Hist1dData`, `Hist2dData` or [`PointData`]. oxihist implements the first
//! two for its `Histogram`, which is what `Axes::hist`, `Axes::profile` and
//! `Axes::hist2d` take: a record read from a file (`Hist1D`, `Profile1D`,
//! `Hist2D`) is drawn through its `to_hist()`. A graph and an efficiency
//! have no such counterpart, so they meet oxiplot here.

use oxiplot::PointData;

use crate::{Efficiency, Graph, GraphErrors};

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

/// The bins of an efficiency that hold trials, as `(bin, centre, half-width)`.
fn filled_bins(e: &Efficiency) -> Vec<(usize, f64, f64)> {
    let axis = e.total().xaxis();
    (1..=axis.nbins())
        .filter(|&bin| e.efficiency(bin).is_some())
        .filter_map(|bin| {
            let (centre, width) = (axis.bin_center(bin)?, axis.bin_width(bin)?);
            Some((bin, centre, 0.5 * width))
        })
        .collect()
}

/// An efficiency is drawn as one point per bin that holds trials: the
/// efficiency at the bin centre, the bin as its horizontal bar, and its
/// confidence interval ([`Efficiency::interval`]) as its vertical one. With
/// an interval this crate does not compute, the points have no vertical bar.
impl PointData for Efficiency {
    fn xs(&self) -> Vec<f64> {
        filled_bins(self).iter().map(|&(_, x, _)| x).collect()
    }
    fn ys(&self) -> Vec<f64> {
        filled_bins(self)
            .iter()
            .filter_map(|&(bin, ..)| self.efficiency(bin))
            .collect()
    }
    fn x_errors(&self) -> Option<(Vec<f64>, Vec<f64>)> {
        let half: Vec<f64> = filled_bins(self).iter().map(|&(.., h)| h).collect();
        Some((half.clone(), half))
    }
    fn y_errors(&self) -> Option<(Vec<f64>, Vec<f64>)> {
        let mut low = Vec::new();
        let mut high = Vec::new();
        for (bin, ..) in filled_bins(self) {
            let (value, (lo, hi)) = (self.efficiency(bin)?, self.interval(bin).ok()??);
            low.push((value - lo).max(0.0));
            high.push((hi - value).max(0.0));
        }
        Some((low, high))
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
    fn an_efficiency_is_a_point_per_bin_with_trials() {
        // Bin 1 holds 3 of 10, bin 2 nothing, bin 3 holds 7 of 7.
        let mut e = crate::Efficiency::new(3, 0.0, 6.0);
        for i in 0..10 {
            e.fill(i < 3, 1.0);
        }
        for _ in 0..7 {
            e.fill(true, 5.0);
        }
        assert_eq!(e.xs(), vec![1.0, 5.0]);
        assert_eq!(e.ys(), vec![0.3, 1.0]);
        assert_eq!(e.x_errors(), Some((vec![1.0, 1.0], vec![1.0, 1.0])));
        let (low, high) = e.y_errors().unwrap();
        let (lo, hi) = e.interval(1).unwrap().unwrap();
        assert!((low[0] - (0.3 - lo)).abs() < 1e-12 && (high[0] - (hi - 0.3)).abs() < 1e-12);
        // A bin where every trial passed has no upward bar.
        assert!(low[1] > 0.0 && high[1].abs() < 1e-12);
        // An interval this crate does not compute leaves the points bare.
        let bare = e.with_statistic_option(crate::StatisticOption::MidP);
        assert_eq!(bare.y_errors(), None);
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
