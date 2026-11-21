//! `Graph2D` — an (x, y, z) scatter of points for 3-D surface/scatter display.
//!
//! On disk (v1): `Named`, `TAttLine`, `TAttFill`, `TAttMarker`, then the scalars
//! `fNpoints`/`fNpx`/`fNpy`/`fMaxIter`, the `fX`/`fY`/`fZ` `double* //[fNpoints]`
//! arrays, `fMinimum`/`fMaximum`/`fMargin`/`fZout`, an `fFunctions` list, and the
//! `fUserHisto` flag. We keep the point data and write ROOT's display defaults
//! for the rest (an empty `fFunctions`, like [`Graph`](crate::Graph)); the
//! `fHistogram` display frame is transient in ROOT and not persisted.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{read_named, skip_versioned};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::base::check_len;
use crate::graph::read_basic_array;

/// An (x, y, z) graph (ROOT `TGraph2D`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TGraph2D")]
pub struct Graph2D {
    /// Graph name (`fName`).
    pub name: String,
    /// Graph title (`fTitle`).
    pub title: String,
    /// Point x coordinates (`fX`).
    pub(crate) x: Vec<f64>,
    /// Point y coordinates (`fY`).
    pub(crate) y: Vec<f64>,
    /// Point z coordinates (`fZ`).
    pub(crate) z: Vec<f64>,
}

impl Graph2D {
    /// Create a `Graph2D` from paired `x`/`y`/`z` points.
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if `y` or `z` is not as long as `x`.
    pub fn new(x: Vec<f64>, y: Vec<f64>, z: Vec<f64>) -> Result<Self> {
        check_len("TGraph2D y", x.len(), y.len())?;
        check_len("TGraph2D z", x.len(), z.len())?;
        Ok(Self {
            name: String::new(),
            title: String::new(),
            x,
            y,
            z,
        })
    }

    /// Number of points (`fNpoints`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.x.len().min(self.y.len()).min(self.z.len())
    }

    /// Whether the graph has no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn decode_tgraph2d(name: &str, class: &str, object: &[u8]) -> Result<Graph2D> {
    if class != "TGraph2D" {
        return Err(Error::WrongClass {
            name: name.to_string(),
            found: class.to_string(),
            expected: "TGraph2D".to_string(),
        });
    }
    let mut r = RBuffer::new(object);
    let base = r.read_version()?; // Graph2D v1
    let named = read_named(&mut r)?;
    skip_versioned(&mut r)?; // TAttLine
    skip_versioned(&mut r)?; // TAttFill
    skip_versioned(&mut r)?; // TAttMarker
    let npoints = usize::saturating_from(r.be_i32()?.max(0));
    let _fnpx = r.be_i32()?;
    let _fnpy = r.be_i32()?;
    let _fmaxiter = r.be_i32()?;
    let x = read_basic_array(&mut r, npoints)?;
    let y = read_basic_array(&mut r, npoints)?;
    let z = read_basic_array(&mut r, npoints)?;
    if let Some(end) = base.end {
        r.seek(end)?; // skip fMinimum/fMaximum/fMargin/fZout/fFunctions/fUserHisto
    }
    Ok(Graph2D {
        name: named.name,
        title: named.title,
        x,
        y,
        z,
    })
}

impl ReadRoot for Graph2D {
    const CLASS: &'static str = "TGraph2D";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS)
            .then(|| decode_tgraph2d(object.name, object.class, object.bytes))
    }
}
