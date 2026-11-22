//! `Axis` — a histogram axis.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{TagReader, read_named, read_object_base, skip_versioned};
use oxiroot_io_core::{Error, RBuffer, Result};

use crate::base::end_record;

/// A histogram axis (ROOT's `TAxis`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TAxis")]
pub struct Axis {
    /// Axis name (`fName`, e.g. "xaxis").
    pub(crate) name: String,
    /// Axis title (`fTitle`).
    pub(crate) title: String,
    /// Number of bins (`fNbins`).
    pub(crate) nbins: usize,
    /// Low edge of the axis range (`fXmin`).
    pub(crate) xmin: f64,
    /// High edge of the axis range (`fXmax`).
    pub(crate) xmax: f64,
    /// Variable bin edges (`fXbins`); empty when the axis is uniform.
    pub(crate) xbins: Vec<f64>,
    /// Alphanumeric bin labels (`fLabels`), one per bin (`labels[i]` labels bin
    /// `i + 1`). Empty for an ordinary numeric axis; an unlabelled bin in an
    /// otherwise-labelled axis holds an empty string.
    pub(crate) labels: Vec<String>,
    /// Whether the axis values are times, to be drawn as dates and clock times
    /// rather than numbers (`fTimeDisplay`). Set it with
    /// [`set_time_format`](Self::set_time_format).
    pub(crate) time_display: bool,
    /// How a time axis formats its labels (`fTimeFormat`): a `strftime` format,
    /// optionally followed by `%F` and the epoch the values count from
    /// (`"%d/%m/%Y%F1970-01-01 00:00:00"`). Empty means ROOT's own default.
    pub(crate) time_format: String,
}

impl Axis {
    /// Create a uniform axis of `nbins` bins over `[xmin, xmax)`. Name it
    /// with [`named`](Self::named).
    #[must_use]
    pub fn new(nbins: usize, xmin: f64, xmax: f64) -> Self {
        Self {
            name: String::new(),
            title: String::new(),
            nbins,
            xmin,
            xmax,
            xbins: Vec::new(),
            labels: Vec::new(),
            time_display: false,
            time_format: String::new(),
        }
    }

    /// Create a variable-width axis from `edges` (the `nbins + 1` bin
    /// boundaries, ascending).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if `edges` has fewer than two values or is not
    /// strictly ascending.
    pub fn variable(edges: &[f64]) -> Result<Self> {
        if edges.len() < 2 {
            return Err(Error::InvalidInput(
                "a variable axis needs at least two edges".to_string(),
            ));
        }
        if !edges.windows(2).all(|w| w[0] < w[1]) {
            return Err(Error::InvalidInput(
                "variable axis edges must be strictly ascending".to_string(),
            ));
        }
        Ok(Self::from_edges("", edges))
    }

    /// A variable-width axis from edges already known to be valid: at least
    /// two, strictly ascending (the edges of another axis, or a regrouping of
    /// them). Fewer than two give an axis with no bins.
    pub(crate) fn from_edges(name: &str, edges: &[f64]) -> Self {
        Self {
            name: name.to_string(),
            title: String::new(),
            nbins: edges.len().saturating_sub(1),
            xmin: edges.first().copied().unwrap_or(0.0),
            xmax: edges.last().copied().unwrap_or(0.0),
            xbins: edges.to_vec(),
            labels: Vec::new(),
            time_display: false,
            time_format: String::new(),
        }
    }

    /// Find the bin for value `x`: 0 = underflow, `1..=nbins` = in range,
    /// `nbins + 1` = overflow. Handles uniform and variable-width axes. `NaN`
    /// goes to overflow, matching ROOT's `Axis::FindBin`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "x lies in [xmin, xmax): a non-negative bin offset"
    )]
    #[must_use]
    pub fn find_bin(&self, x: f64) -> usize {
        let n = self.nbins;
        if n == 0 {
            return 0;
        }
        if x < self.xmin {
            return 0;
        }
        // Route `NaN` to overflow (ROOT does the same): a plain `x >= xmax`
        // would let it fall through to the in-range arithmetic below.
        if x.is_nan() || x >= self.xmax {
            return n + 1;
        }
        if self.xbins.is_empty() {
            let width = (self.xmax - self.xmin) / n as f64;
            // `x` is in range here, so clamp to `n` (the last in-range bin):
            // float rounding at the top edge must never spill into overflow.
            (1 + ((x - self.xmin) / width).floor() as usize).min(n)
        } else {
            // Variable edges: largest i with xbins[i] <= x, giving bin i+1.
            match self.xbins.partition_point(|&edge| edge <= x) {
                0 => 0,
                i => i.min(n),
            }
        }
    }

    /// Whether two axes describe the same binning — identical bin count and
    /// edges, ignoring name/title. This is the precondition for bin-by-bin
    /// histogram arithmetic (`add`/`multiply`/`divide`) and merging. Compared
    /// edge-by-edge (so a uniform axis and an equivalent variable one match)
    /// without allocating either edge array.
    #[must_use]
    pub fn same_binning(&self, other: &Self) -> bool {
        self.nbins == other.nbins && (0..=self.nbins).all(|i| self.edge_at(i) == other.edge_at(i))
    }

    /// Read an `Axis` from `r` (positioned at the axis's `{byte-count, version}`
    /// header), leaving the cursor at the axis's end. `tags` is the class-tag
    /// map of the buffer `r` reads, which resolves the classes of the labels.
    ///
    /// Class versions 6 to 10 are read (ROOT 3.02 onwards): version 7 added
    /// `fLabels`, version 8 `fBits2`, and version 10 `fModLabs`. Earlier versions
    /// used a hand-written streamer and are an error.
    ///
    /// # Errors
    ///
    /// If the buffer ends first, or the axis's edges do not match its bin count.
    pub fn read(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
        let vh = r.read_version()?; // Axis (e.g. version 10)
        if vh.version < 6 {
            return Err(Error::UnsupportedVersion {
                class: "TAxis".to_string(),
                version: i32::from(vh.version),
            });
        }
        let named = read_named(r)?; // Named base
        skip_versioned(r)?; // TAttAxis base (drawing attributes — not needed)

        let nbins = usize::try_from(r.be_i32()?)
            .map_err(|_| Error::Format("TAxis has a negative bin count".into()))?;
        let xmin = r.be_f64()?;
        let xmax = r.be_f64()?;

        // fXbins is a TArrayD member: a count followed by that many doubles.
        // Cap the reservation at the buffer size so a forged count can't drive a
        // huge allocation before the element reads fail.
        let n = usize::saturating_from(r.be_i32()?.max(0));
        let mut xbins = Vec::with_capacity(n.min(r.remaining()));
        for _ in 0..n {
            xbins.push(r.be_f64()?);
        }
        // Variable edges must be the `fNbins + 1` ascending boundaries: the
        // bin lookups, `edges()` and every derived histogram index them by bin.
        check_edges(nbins, &xbins)?;

        // fFirst, fLast, fBits2, fTimeDisplay, fTimeFormat precede the labels.
        let _first = r.be_i32()?;
        let _last = r.be_i32()?;
        if vh.version >= 8 {
            let _bits2 = r.be_u16()?;
        }
        let time_display = r.u8()? != 0;
        let time_format = r.string()?;
        let labels = if vh.version >= 7 {
            read_labels(r, tags, nbins)? // fLabels (THashList*)
        } else {
            Vec::new()
        };

        // Skip the remainder (fModLabs) via the axis byte count.
        end_record(r, &vh, "TAxis")?;

        Ok(Self {
            name: named.name,
            title: named.title,
            nbins,
            xmin,
            xmax,
            xbins,
            labels,
            time_display,
            time_format,
        })
    }

    /// This axis named `name` (`fName`).
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// This axis titled `title` (`fTitle`), the text drawn along it.
    #[must_use]
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// The axis name (`fName`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The axis title (`fTitle`).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Set the axis title, in place.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// The bin labels, one per bin (`labels()[i]` labels bin `i + 1`): empty
    /// for a numeric axis, and an empty string for an unlabelled bin of a
    /// labelled one.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Whether the axis values are times, drawn as dates and clock times
    /// (`fTimeDisplay`). Set with [`set_time_format`](Self::set_time_format).
    #[must_use]
    pub fn is_time(&self) -> bool {
        self.time_display
    }

    /// How a time axis formats its labels (`fTimeFormat`): a `strftime`
    /// format, optionally followed by `%F` and the epoch the values count
    /// from. Empty means ROOT's own default.
    #[must_use]
    pub fn time_format(&self) -> &str {
        &self.time_format
    }

    /// Alphanumeric label for bin `bin` (1-based), or `None` for an unlabelled
    /// bin / numeric axis.
    pub fn bin_label(&self, bin: usize) -> Option<&str> {
        self.labels
            .get(bin.checked_sub(1)?)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }

    /// The 1-based bin carrying label `label`, if any.
    #[must_use]
    pub fn find_label(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|l| l == label).map(|i| i + 1)
    }

    /// Whether the axis carries alphanumeric bin labels (`fLabels`).
    #[must_use]
    pub fn is_labelled(&self) -> bool {
        self.labels.iter().any(|l| !l.is_empty())
    }

    /// Draw this axis as times: the values are seconds, and `format` says how a
    /// label reads. It is a `strftime` format (`"%H:%M"`, `"%d/%m/%Y"`), and
    /// `%F` after it names the epoch the values count from, as ROOT writes it —
    /// `"%H:%M%F2024-01-01 00:00:00"` for values counted from the start of 2024.
    /// An empty `format` leaves ROOT to choose one. ROOT, uproot and oxiroot
    /// read the axis back as a time axis.
    ///
    /// ```
    /// use oxiroot_hist::ToRoot;
    /// # use oxiroot_hist::Hist;
    /// let mut h = Hist::reg(24, 0.0, 86_400.0).double().to_root().unwrap().named("rate");
    /// h.xaxis_mut().set_time_format("%H:%M%F2024-01-01 00:00:00");
    /// assert!(h.xaxis().is_time());
    /// ```
    pub fn set_time_format(&mut self, format: &str) {
        self.time_display = true;
        self.time_format = format.to_string();
    }

    /// Stop drawing this axis as times, and forget the format.
    pub fn clear_time_format(&mut self) {
        self.time_display = false;
        self.time_format.clear();
    }

    /// Set the alphanumeric label for bin `bin` (1-based), growing the label
    /// vector to `nbins` as needed.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if the axis has no bin `bin`.
    pub fn set_label(&mut self, bin: usize, label: &str) -> Result<()> {
        let n = self.nbins;
        if bin < 1 || bin > n {
            return Err(Error::InvalidInput(format!(
                "no bin {bin} to label on an axis of {n} bins (bins are numbered from 1)"
            )));
        }
        if self.labels.len() < n {
            self.labels.resize(n, String::new());
        }
        self.labels[bin - 1] = label.to_string();
        Ok(())
    }

    /// The `nbins + 1` bin edges, low to high (uniform when `xbins` is empty).
    #[must_use]
    pub fn edges(&self) -> Vec<f64> {
        if !self.xbins.is_empty() {
            return self.xbins.clone();
        }
        let n = self.nbins;
        if n == 0 {
            return vec![self.xmin, self.xmax];
        }
        let step = (self.xmax - self.xmin) / n as f64;
        (0..=n).map(|i| self.xmin + step * i as f64).collect()
    }

    /// The `i`-th bin boundary (`i` in `0..=nbins`: `0` is `xmin`, `nbins` is
    /// `xmax`), computed in O(1) without allocating — unlike [`edges`](Self::edges),
    /// which materializes the whole array. `None` past the last boundary.
    #[must_use]
    pub fn edge(&self, i: usize) -> Option<f64> {
        (i <= self.nbins).then(|| self.edge_at(i))
    }

    /// Boundary `i`, clamped to the last one.
    fn edge_at(&self, i: usize) -> f64 {
        if !self.xbins.is_empty() {
            return self.xbins[i.min(self.xbins.len() - 1)];
        }
        let n = self.nbins;
        if n == 0 {
            return if i == 0 { self.xmin } else { self.xmax };
        }
        let i = i.min(n) as f64;
        self.xmin + (self.xmax - self.xmin) / n as f64 * i
    }

    /// The low and high edges of bin `bin` (1-based), or `None` if the axis
    /// has no such bin (the flow bins have no edges).
    fn bin_edges(&self, bin: usize) -> Option<(f64, f64)> {
        let n = self.nbins;
        (1..=n)
            .contains(&bin)
            .then(|| (self.edge_at(bin - 1), self.edge_at(bin)))
    }

    /// Low edge of bin `bin` (1-based; bin 1 starts at `xmin`), or `None` if
    /// the axis has no such bin.
    #[must_use]
    pub fn bin_low_edge(&self, bin: usize) -> Option<f64> {
        self.bin_edges(bin).map(|(lo, _)| lo)
    }

    /// Width of bin `bin` (1-based), or `None` if the axis has no such bin.
    #[must_use]
    pub fn bin_width(&self, bin: usize) -> Option<f64> {
        self.bin_edges(bin).map(|(lo, hi)| hi - lo)
    }

    /// Center of bin `bin` (1-based), or `None` if the axis has no such bin.
    #[must_use]
    pub fn bin_center(&self, bin: usize) -> Option<f64> {
        self.bin_edges(bin).map(|(lo, hi)| 0.5 * (lo + hi))
    }
}

/// Check a `TAxis`'s variable edges (`fXbins`) against its bin count: none (a
/// uniform axis), or exactly `nbins + 1` strictly ascending values.
fn check_edges(nbins: usize, xbins: &[f64]) -> Result<()> {
    if xbins.is_empty() {
        return Ok(());
    }
    let expected = nbins + 1;
    if xbins.len() != expected {
        return Err(Error::Format(format!(
            "TAxis has {} bin edges for {nbins} bins, not {expected}",
            xbins.len()
        )));
    }
    if !xbins.windows(2).all(|w| w[0] < w[1]) {
        return Err(Error::Format(
            "TAxis bin edges are not strictly ascending".into(),
        ));
    }
    Ok(())
}

/// Read the `fLabels` member: a `THashList*` of `ObjString`, each carrying its
/// 1-based bin number in `fUniqueID` and the label text in `fString`. Returns a
/// `Vec` of length `nbins` (empty strings for unlabelled bins), or empty when
/// the pointer is null (an ordinary numeric axis).
fn read_labels(r: &mut RBuffer<'_>, tags: &mut TagReader, nbins: usize) -> Result<Vec<String>> {
    if tags.read_header(r)?.class_name.is_none() {
        return Ok(Vec::new()); // null fLabels pointer
    }
    r.read_version()?; // THashList (a TList, version 5)
    read_object_base(r)?;
    r.skip_string()?; // fName (empty)
    let size = usize::saturating_from(r.be_i32()?.max(0));

    let mut labels = vec![String::new(); nbins];
    for _ in 0..size {
        let entry = tags.read_header(r)?;
        if entry.class_name.is_none() {
            // A null entry, or a label shared with an earlier one: no body
            // here, only the option string every entry has.
            r.skip_string()?;
            continue;
        }
        let body = r.read_version()?; // ObjString body {byte count, version}
        let obj = read_object_base(r)?; // fUniqueID = 1-based bin number
        let label = r.string()?; // fString
        let bin = obj.unique_id as usize;
        if (1..=nbins).contains(&bin) {
            labels[bin - 1] = label;
        }
        if let Some(end) = body.end {
            r.seek(end)?;
        }
        r.skip_string()?; // per-element option string (empty), as in TList
    }
    if labels.iter().all(String::is_empty) {
        return Ok(Vec::new());
    }
    Ok(labels)
}

#[cfg(test)]
mod edge_tests {
    use super::check_edges;

    /// A uniform axis has no edges; a variable one has `nbins + 1` ascending
    /// ones. Anything else would be indexed out of range later.
    #[test]
    fn variable_edges_must_match_the_bin_count_and_ascend() {
        assert!(check_edges(4, &[]).is_ok());
        assert!(check_edges(2, &[0.0, 1.0, 3.0]).is_ok());
        assert!(check_edges(3, &[0.0, 1.0, 3.0]).is_err(), "one edge short");
        assert!(
            check_edges(1, &[0.0, 1.0, 3.0]).is_err(),
            "one edge too many"
        );
        assert!(check_edges(2, &[0.0, 0.0, 3.0]).is_err(), "repeated edge");
        assert!(check_edges(2, &[0.0, 2.0, 1.0]).is_err(), "descending");
        assert!(check_edges(2, &[0.0, f64::NAN, 1.0]).is_err(), "NaN edge");
    }
}
