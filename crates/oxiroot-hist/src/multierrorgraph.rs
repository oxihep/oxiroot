//! `MultiErrorGraph` — an (x, y) graph with asymmetric x errors and *several*
//! independent layers of asymmetric y errors (e.g. statistical + systematic).
//!
//! On disk (v1): a full `Graph` base (v5), then `fNYErrors`, `fSumErrorsMode`,
//! the `fExL`/`fExH` `double* //[fNpoints]` arrays, the `fEyL`/`fEyH`
//! `vector<TArrayD>` (one `TArrayD` per y-error layer, streamed *objectwise*),
//! and finally `fAttFill`/`fAttLine` (`vector<TAttFill>`/`vector<TAttLine>`,
//! streamed *memberwise*) holding the per-layer draw attributes. We keep the
//! point + error data and write ROOT's default attributes; the attribute vectors
//! are display-only and skipped on read.
//!
//! Note: uproot cannot decode the memberwise attribute vectors, so this class is
//! cross-checked against compiled ROOT C++ only.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{read_named, skip_versioned};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::base::check_len;
use crate::graph::read_basic_array;

/// An (x, y) graph with asymmetric x errors and one or more layers of asymmetric
/// y errors (ROOT `TGraphMultiErrors`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TGraphMultiErrors")]
pub struct MultiErrorGraph {
    /// Graph name (`fName`).
    pub name: String,
    /// Graph title (`fTitle`).
    pub title: String,
    /// Point x coordinates (`fX`).
    pub(crate) x: Vec<f64>,
    /// Point y coordinates (`fY`).
    pub(crate) y: Vec<f64>,
    /// Low x errors (`fExL`).
    pub(crate) ex_low: Vec<f64>,
    /// High x errors (`fExH`).
    pub(crate) ex_high: Vec<f64>,
    /// Low y errors, one `Vec` per error layer (`fEyL`).
    pub(crate) ey_low: Vec<Vec<f64>>,
    /// High y errors, one `Vec` per error layer (`fEyH`).
    pub(crate) ey_high: Vec<Vec<f64>>,
    /// How the y-error layers combine when summed (`fSumErrorsMode`; 0 =
    /// `kOnlyFirst`, 1 = `kSquareSum`, 2 = `kSum`).
    pub sum_errors_mode: i32,
}

impl MultiErrorGraph {
    /// Create a `MultiErrorGraph` with x errors and a first y-error layer.
    /// Add further y-error layers with [`with_y_error`](Self::with_y_error).
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if `y` or an error vector is not as long as `x`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        x: Vec<f64>,
        y: Vec<f64>,
        ex_low: Vec<f64>,
        ex_high: Vec<f64>,
        ey_low: Vec<f64>,
        ey_high: Vec<f64>,
    ) -> Result<Self> {
        let n = x.len();
        check_len("TGraphMultiErrors y", n, y.len())?;
        check_len("TGraphMultiErrors ex_low", n, ex_low.len())?;
        check_len("TGraphMultiErrors ex_high", n, ex_high.len())?;
        check_len("TGraphMultiErrors ey_low", n, ey_low.len())?;
        check_len("TGraphMultiErrors ey_high", n, ey_high.len())?;
        Ok(Self {
            name: String::new(),
            title: String::new(),
            x,
            y,
            ex_low,
            ex_high,
            ey_low: vec![ey_low],
            ey_high: vec![ey_high],
            sum_errors_mode: 0,
        })
    }

    /// Add another independent layer of asymmetric y errors. Chainable.
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if `ey_low` or `ey_high` is not as long as `x`.
    pub fn with_y_error(mut self, ey_low: Vec<f64>, ey_high: Vec<f64>) -> Result<Self> {
        check_len("TGraphMultiErrors ey_low", self.x.len(), ey_low.len())?;
        check_len("TGraphMultiErrors ey_high", self.x.len(), ey_high.len())?;
        self.ey_low.push(ey_low);
        self.ey_high.push(ey_high);
        Ok(self)
    }

    /// Number of points (`fNpoints`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.x.len().min(self.y.len())
    }

    /// Whether the graph has no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of independent y-error layers (`fNYErrors`).
    #[must_use]
    pub fn n_y_errors(&self) -> usize {
        self.ey_low.len()
    }
}

/// Read an *objectwise* `vector<TArrayD>`: `[bc][ver]`, a count, then one
/// `TArrayD` (`fN` + `fN` doubles) per layer.
fn read_vector_tarrayd(r: &mut RBuffer<'_>) -> Result<Vec<Vec<f64>>> {
    let v = r.read_version()?;
    let count = usize::saturating_from(r.be_i32()?.max(0));
    // `count` comes from the file; every layer takes at least its 4-byte length.
    let mut layers = Vec::with_capacity(r.bounded_capacity(count, 4));
    for _ in 0..count {
        let fnn = usize::saturating_from(r.be_i32()?.max(0));
        let arr: Vec<f64> = (0..fnn).map(|_| r.be_f64()).collect::<Result<_>>()?;
        layers.push(arr);
    }
    if let Some(end) = v.end {
        r.seek(end)?;
    }
    Ok(layers)
}

fn decode_tgraphmultierrors(name: &str, class: &str, object: &[u8]) -> Result<MultiErrorGraph> {
    if class != "TGraphMultiErrors" {
        return Err(Error::WrongClass {
            name: name.to_string(),
            found: class.to_string(),
            expected: "TGraphMultiErrors".to_string(),
        });
    }
    let mut r = RBuffer::new(object);
    let outer = r.read_version()?; // MultiErrorGraph v1
    let base = r.read_version()?; // Graph v5 base
    let named = read_named(&mut r)?;
    skip_versioned(&mut r)?; // TAttLine
    skip_versioned(&mut r)?; // TAttFill
    skip_versioned(&mut r)?; // TAttMarker
    let npoints = usize::saturating_from(r.be_i32()?.max(0));
    let x = read_basic_array(&mut r, npoints)?;
    let y = read_basic_array(&mut r, npoints)?;
    if let Some(end) = base.end {
        r.seek(end)?; // skip the Graph base trailer (fFunctions/fHistogram/…)
    }
    let _n_y_errors = r.be_i32()?;
    let sum_errors_mode = r.be_i32()?;
    let ex_low = read_basic_array(&mut r, npoints)?;
    let ex_high = read_basic_array(&mut r, npoints)?;
    let ey_low = read_vector_tarrayd(&mut r)?;
    let ey_high = read_vector_tarrayd(&mut r)?;
    if let Some(end) = outer.end {
        r.seek(end)?; // skip fAttFill/fAttLine (display attributes)
    }
    Ok(MultiErrorGraph {
        name: named.name,
        title: named.title,
        x,
        y,
        ex_low,
        ex_high,
        ey_low,
        ey_high,
        sum_errors_mode,
    })
}

impl ReadRoot for MultiErrorGraph {
    const CLASS: &'static str = "TGraphMultiErrors";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS)
            .then(|| decode_tgraphmultierrors(object.name, object.class, object.bytes))
    }
}
