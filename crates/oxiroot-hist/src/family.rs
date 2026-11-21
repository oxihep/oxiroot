//! The methods the profiles share, written once: they differ only in their
//! axes and the name of their `Σw·v²` array, and hand-copied versions drift
//! apart (their `add` in `ops.rs` is the same pattern).

use crate::{Profile1D, Profile2D, Profile3D};

/// The per-cell core the profiles ([`Profile1D`], [`Profile2D`],
/// [`Profile3D`]) share: they differ in their axes, the name of their `Σw·v²`
/// array and their moment sums, not in how a cell accumulates, averages or
/// errs.
macro_rules! impl_profile_family {
    ($ty:ident, value_sq: $sumv2:ident $(,)?) => {
        impl $ty {
            /// Seed per-cell `Σw²` from the weight sums (exact for unit
            /// weights) the first time it is needed.
            pub(crate) fn track_bin_sumw2(&mut self) {
                if self.bin_sumw2.is_empty() {
                    self.bin_sumw2 = self.bin_entries.clone();
                }
            }

            /// The profiled value of cell `cell` (flow included): `Σw·v / Σw`,
            /// or 0 where the cell has no entries (ROOT's `GetBinContent`).
            #[must_use]
            pub(crate) fn bin_value(&self, cell: usize) -> f64 {
                let e = self.bin_entries.get(cell).copied().unwrap_or(0.0);
                if e == 0.0 {
                    0.0
                } else {
                    self.sums.get(cell).copied().unwrap_or(0.0) / e
                }
            }
        }
    };
}

impl_profile_family!(Profile1D, value_sq: sumy2);
impl_profile_family!(Profile2D, value_sq: sumz2);
impl_profile_family!(Profile3D, value_sq: sumt2);
