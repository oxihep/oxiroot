//! `Graph` and its error-bar variants `TGraphErrors` / `TGraphAsymmErrors`.
//!
//! A `Graph` is an (x, y) scatter of points. `TGraphErrors` adds symmetric x/y
//! error bars and `TGraphAsymmErrors` adds independent low/high errors on each
//! axis; both are a `Graph` base followed by inline `double* //[fNpoints]`
//! error arrays. One [`Graph`] type covers all three ROOT classes, the variant
//! recorded in [`errors`](Graph::errors).
//!
//! On disk: `Graph` (v5) is `Named`, `TAttLine`, `TAttFill`, `TAttMarker`,
//! `fNpoints`, `fX`, `fY`, then a trailer: `fFunctions` (a `TList` of attached
//! `Func1D`s — fitted functions, parsed into [`functions`](Graph::functions)),
//! `fHistogram` (an optional display frame), `fMinimum`, `fMaximum`, `fOption`.

use oxiroot_io_core::raw::{
    PointerHeader, TagReader, read_named, read_object_base, read_pointer_header, skip_versioned,
};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};

use crate::base::{BinContentType, bin_content_type_of, check_len};
use crate::record::Hist1D;
use oxiroot_io_core::cast::SaturatingFrom;

/// Error bars attached to a graph's points.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GraphErrors {
    /// No error bars: a plain `TGraph`.
    Plain,
    /// Symmetric per-point x and y errors — a `TGraphErrors`.
    Symmetric {
        /// x error (`fEX`).
        ex: Vec<f64>,
        /// y error (`fEY`).
        ey: Vec<f64>,
    },
    /// Independent low/high errors on each axis — a `TGraphAsymmErrors`.
    Asymmetric {
        /// Low x error (`fEXlow`).
        ex_low: Vec<f64>,
        /// High x error (`fEXhigh`).
        ex_high: Vec<f64>,
        /// Low y error (`fEYlow`).
        ey_low: Vec<f64>,
        /// High y error (`fEYhigh`).
        ey_high: Vec<f64>,
    },
}

/// A function (ROOT `TF1`) attached to a graph — e.g. a fitted curve stored in
/// the graph's `fFunctions` list. Formula-based: the formula references
/// parameters as `[p0]`, `[p1]`, … (ROOT also accepts `[0]`, `[1]`, which
/// [`GraphFunction::new`] normalizes), with values in [`params`](Self::params).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphFunction {
    /// Function name (`fName`).
    pub name: String,
    /// Function title — ROOT's convention is the `[0]`-form formula (`fTitle`).
    pub title: String,
    /// The formula in `[p0]`/`[p1]` form (`TFormula::fFormula`).
    pub formula: String,
    /// Current parameter values (`TFormula::fClingParameters`).
    pub params: Vec<f64>,
    /// Per-parameter fit errors (`Func1D::fParErrors`).
    pub par_errors: Vec<f64>,
    /// Per-parameter lower limits (`Func1D::fParMin`).
    pub par_min: Vec<f64>,
    /// Per-parameter upper limits (`Func1D::fParMax`).
    pub par_max: Vec<f64>,
    /// Lower bound of the function's range (`Func1D::fXmin`).
    pub xmin: f64,
    /// Upper bound of the function's range (`Func1D::fXmax`).
    pub xmax: f64,
    /// Fit chi-square (`Func1D::fChisquare`).
    pub chi2: f64,
    /// Fit degrees of freedom (`Func1D::fNDF`).
    pub ndf: i32,
}

impl GraphFunction {
    /// Build a formula function over `[xmin, xmax]`. `formula` may use either
    /// `[0]`/`[1]` or `[p0]`/`[p1]` for parameters; it is stored in `[pN]` form.
    /// Errors/limits default to zero, sized to `params`.
    pub fn new(
        name: impl Into<String>,
        formula: impl Into<String>,
        params: Vec<f64>,
        xmin: f64,
        xmax: f64,
    ) -> Self {
        let title = formula.into();
        let n = params.len();
        Self {
            name: name.into(),
            formula: normalize_formula(&title),
            title,
            params,
            par_errors: vec![0.0; n],
            par_min: vec![0.0; n],
            par_max: vec![0.0; n],
            xmin,
            xmax,
            chi2: 0.0,
            ndf: 0,
        }
    }

    /// Number of parameters (`fNpar`).
    #[must_use]
    pub fn npar(&self) -> usize {
        self.params.len()
    }
}

/// Rewrite bare `[N]` parameter references to ROOT's `[pN]` form (leaving an
/// already-`[pN]` reference, or any non-numeric `[…]`, untouched).
fn normalize_formula(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        if let Some(close) = rest.find(']') {
            let inner = &rest[1..close];
            if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) {
                out.push_str("[p");
                out.push_str(inner);
                out.push(']');
            } else {
                out.push_str(&rest[..=close]);
            }
            rest = &rest[close + 1..];
        } else {
            out.push_str(rest);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// An (x, y) graph, optionally with error bars (ROOT `TGraph` /
/// `TGraphErrors` / `TGraphAsymmErrors`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TGraph", alias = "TGraphErrors", alias = "TGraphAsymmErrors")]
pub struct Graph {
    /// Graph name (`fName`).
    pub(crate) name: String,
    /// Graph title (`fTitle`).
    pub(crate) title: String,
    /// Point x coordinates (`fX`).
    pub(crate) x: Vec<f64>,
    /// Point y coordinates (`fY`).
    pub(crate) y: Vec<f64>,
    /// Error bars, selecting the concrete ROOT class.
    pub(crate) errors: GraphErrors,
    /// Optional display frame (`fHistogram`, a `TH1F`): the axis frame ROOT
    /// builds when a graph is drawn. `None` (the default) writes a null pointer,
    /// matching a freshly-created ROOT graph; set one with
    /// [`with_histogram`](Graph::with_histogram) to persist axis ranges/titles.
    pub(crate) histogram: Option<Hist1D>,
    /// Functions attached to the graph (`fFunctions`) — typically the `Func1D`s
    /// produced by fitting it. Empty (the default) writes an empty list, matching
    /// a freshly-created ROOT graph; attach one with
    /// [`with_function`](Graph::with_function).
    pub(crate) functions: Vec<GraphFunction>,
}

impl Graph {
    /// Create a plain `Graph` from paired `x`/`y` points.
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if `y` is not as long as `x`.
    pub fn new(x: Vec<f64>, y: Vec<f64>) -> Result<Self> {
        check_len("TGraph y", x.len(), y.len())?;
        Ok(Self {
            name: String::new(),
            title: String::new(),
            x,
            y,
            errors: GraphErrors::Plain,
            histogram: None,
            functions: Vec::new(),
        })
    }

    /// Attach error bars: [`GraphErrors::Symmetric`] makes a `TGraphErrors`,
    /// [`GraphErrors::Asymmetric`] a `TGraphAsymmErrors`.
    ///
    /// ```
    /// # use oxiroot_hist::{Graph, GraphErrors};
    /// let g = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0])?.with_errors(GraphErrors::Symmetric {
    ///     ex: vec![0.1, 0.1],
    ///     ey: vec![0.5, 0.4],
    /// })?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if an error vector is not as long as `x`.
    pub fn with_errors(mut self, errors: GraphErrors) -> Result<Self> {
        let n = self.x.len();
        match &errors {
            GraphErrors::Plain => {}
            GraphErrors::Symmetric { ex, ey } => {
                check_len("TGraphErrors ex", n, ex.len())?;
                check_len("TGraphErrors ey", n, ey.len())?;
            }
            GraphErrors::Asymmetric {
                ex_low,
                ex_high,
                ey_low,
                ey_high,
            } => {
                check_len("TGraphAsymmErrors ex_low", n, ex_low.len())?;
                check_len("TGraphAsymmErrors ex_high", n, ex_high.len())?;
                check_len("TGraphAsymmErrors ey_low", n, ey_low.len())?;
                check_len("TGraphAsymmErrors ey_high", n, ey_high.len())?;
            }
        }
        self.errors = errors;
        Ok(self)
    }

    /// The display frame (`fHistogram`), if the graph has one: the axes ROOT
    /// draws the graph in.
    #[must_use]
    pub fn histogram(&self) -> Option<&Hist1D> {
        self.histogram.as_ref()
    }

    /// The functions attached to the graph (`fFunctions`), such as a fitted
    /// curve.
    #[must_use]
    pub fn functions(&self) -> &[GraphFunction] {
        &self.functions
    }

    /// Attach a display frame (`fHistogram`) — the axis-frame ROOT would build on
    /// draw. Stored (and persisted) as a `TH1F`, ROOT's declared type for
    /// `fHistogram`, so the bin content type is coerced to `F32`. Chainable.
    #[must_use]
    pub fn with_histogram(mut self, histogram: Hist1D) -> Self {
        self.histogram = Some(histogram.with_bin_content_type(BinContentType::F32));
        self
    }

    /// Attach a function (`fFunctions`) — e.g. a fitted `Func1D`. Persisted as a
    /// `Func1D`/`TFormula` inside the graph's function list. Chainable; call more
    /// than once to attach several.
    #[must_use]
    pub fn with_function(mut self, function: GraphFunction) -> Self {
        self.functions.push(function);
        self
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

    /// The ROOT class this graph serializes as.
    #[must_use]
    pub fn class_name(&self) -> &'static str {
        match self.errors {
            GraphErrors::Plain => "TGraph",
            GraphErrors::Symmetric { .. } => "TGraphErrors",
            GraphErrors::Asymmetric { .. } => "TGraphAsymmErrors",
        }
    }
}

/// The start of a `TGraph` record, after its version header: the name and
/// title, the three attribute bases, then the points. Every class built on
/// `TGraph` reads this much the same way; they differ in what they take from
/// the rest of the record.
pub(crate) fn read_graph_points(
    r: &mut RBuffer<'_>,
) -> Result<(oxiroot_io_core::raw::Named, Vec<f64>, Vec<f64>)> {
    let named = read_named(r)?;
    skip_versioned(r)?; // TAttLine
    skip_versioned(r)?; // TAttFill
    skip_versioned(r)?; // TAttMarker
    let npoints = usize::saturating_from(r.be_i32()?.max(0));
    let x = read_basic_array(r, npoints)?;
    let y = read_basic_array(r, npoints)?;
    Ok((named, x, y))
}

/// Read the `Graph` base (`fName`/`fTitle`/`fX`/`fY`), then seek past the
/// trailer (`fFunctions`/`fHistogram`/…) to the base object's end.
///
/// `tags` resolves the class tags of the attached functions. It is `None` for
/// a graph decoded from a slice of a larger buffer (a collection member),
/// whose class back-references point outside the slice.
fn read_tgraph_base(r: &mut RBuffer<'_>, tags: Option<&mut TagReader>) -> Result<Graph> {
    let base = r.read_version()?; // Graph v5
    let (named, x, y) = read_graph_points(r)?;
    let functions = match tags {
        Some(tags) => read_functions(r, tags)?, // fFunctions (TList<Func1D>)
        None => read_functions_untracked(r)?,
    };
    let histogram = read_opt_th1(r)?; // fHistogram (TH1F*, or null)
    if let Some(end) = base.end {
        r.seek(end)?; // skip fMinimum/fMaximum/fOption
    }
    Ok(Graph {
        name: named.name,
        title: named.title,
        x,
        y,
        errors: GraphErrors::Plain,
        histogram,
        functions,
    })
}

/// Read the `fFunctions` `TList` object pointer, decoding its `TF1` elements
/// (formula functions) and skipping any other class. A null pointer or empty
/// list yields an empty `Vec`.
///
/// ROOT names a class in full the first time it appears in a buffer and by a
/// back-reference afterwards, so the second and later functions of a
/// ROOT-written graph carry no class name: `tags` resolves them.
fn read_functions(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Vec<GraphFunction>> {
    let list = tags.read_header(r)?;
    let (Some(_), Some(end)) = (&list.class_name, list.end) else {
        // A null pointer, or a list written elsewhere: nothing inline.
        if let Some(end) = list.end {
            r.seek(end)?;
        }
        return Ok(Vec::new());
    };
    let functions = read_function_list(r, tags)?;
    r.seek(end)?; // skip any remaining TList payload
    Ok(functions)
}

/// Read a streamed `TList` of functions at the cursor (its version header
/// first): the `TF1` elements, skipping any other class. This is the body of a
/// graph's `fFunctions` pointer, and a histogram's `fFunctions` as it stands.
pub(crate) fn read_function_list(
    r: &mut RBuffer<'_>,
    tags: &mut TagReader,
) -> Result<Vec<GraphFunction>> {
    let list = r.read_version()?; // TList v5
    let _obj = read_object_base(r)?; // TObject base
    r.skip_string()?; // fName (empty)
    let nfns = usize::saturating_from(r.be_i32()?.max(0));
    let mut functions = Vec::new();
    for _ in 0..nfns {
        let element = tags.read_header(r)?;
        if element.class_name.as_deref() == Some("TF1") {
            functions.push(GraphFunction::read_tf1_body(r)?);
        }
        if let Some(end) = element.end {
            r.seek(end)?;
        }
        // Each element carries an option TString trailer in the TList.
        r.skip_string()?;
    }
    if let Some(end) = list.end {
        r.seek(end)?;
    }
    Ok(functions)
}

/// [`read_functions`] without a class-tag map, for a graph decoded from a slice
/// of a larger buffer: a function named by a class back-reference cannot be
/// resolved there and is skipped.
fn read_functions_untracked(r: &mut RBuffer<'_>) -> Result<Vec<GraphFunction>> {
    let Some(PointerHeader { end, .. }) = read_pointer_header(r)? else {
        return Ok(Vec::new()); // null pointer
    };
    let _ver = r.read_version()?; // TList v5
    let _obj = read_object_base(r)?; // TObject base
    r.skip_string()?; // fName (empty)
    let nfns = usize::saturating_from(r.be_i32()?.max(0));
    let mut functions = Vec::new();
    for _ in 0..nfns {
        if let Some(element) = read_pointer_header(r)? {
            if element.class_name.as_deref() == Some("TF1") {
                functions.push(GraphFunction::read_tf1_body(r)?);
            }
            r.seek(element.end)?;
        }
        // Each element carries an option TString trailer in the TList.
        r.skip_string()?;
    }
    r.seek(end)?; // skip any remaining TList payload
    Ok(functions)
}

/// Read an optional embedded `TH1*` (the `fHistogram` display frame): `None`
/// for a null pointer. A header that names no class refers back to one named
/// earlier; `fHistogram` is declared a `TH1F`, so that is what it is.
fn read_opt_th1(r: &mut RBuffer<'_>) -> Result<Option<Hist1D>> {
    let Some(header) = read_pointer_header(r)? else {
        return Ok(None);
    };
    let bin_content_type = match header.class_name.as_deref() {
        Some(class) => bin_content_type_of(class)?,
        None => BinContentType::F32,
    };
    Ok(Some(Hist1D::read(r, bin_content_type, None)?))
}

/// Read a `Double_t* //[n]` member: a presence-marker byte then (if present)
/// `n` big-endian doubles.
pub(crate) fn read_basic_array(r: &mut RBuffer<'_>, n: usize) -> Result<Vec<f64>> {
    let present = r.u8()?;
    if present == 0 {
        return Ok(Vec::new());
    }
    (0..n).map(|_| r.be_f64()).collect()
}

impl ReadRoot for Graph {
    const CLASS: &'static str = "TGraph";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        matches!(
            object.class,
            "TGraph" | "TGraphErrors" | "TGraphAsymmErrors"
        )
        .then(|| match object.within {
            // A member of a collection: read where it lies, so that a class
            // named in an earlier member is found.
            Some(within) => {
                let mut r = RBuffer::new(within.buffer);
                r.seek(within.start)?;
                let mut tags = TagReader::new(within.key_len);
                read_tgraph(&mut r, object.name, object.class, Some(&mut tags))
            }
            None => decode_tgraph(object.name, object.class, object.bytes, object.key_len),
        })
    }
}

/// Decode a graph from its streamed `object`. `keylen` is the header length
/// of the key the object was read from, or `None` when `object` is a slice of
/// a larger buffer (see [`read_tgraph_base`]).
pub(crate) fn decode_tgraph(
    name: &str,
    class: &str,
    object: &[u8],
    keylen: Option<usize>,
) -> Result<Graph> {
    let mut r = RBuffer::new(object);
    let mut tags = keylen.map(TagReader::new);
    read_tgraph(&mut r, name, class, tags.as_mut())
}

/// Read a graph of class `class` at the cursor. `tags` is the class-tag map of
/// the buffer `r` reads: the one of the key, or the one a collection is
/// reading its members with, so that a class named in an earlier member is
/// known in a later one.
pub(crate) fn read_tgraph(
    r: &mut RBuffer<'_>,
    name: &str,
    class: &str,
    mut tags: Option<&mut TagReader>,
) -> Result<Graph> {
    match class {
        "TGraph" => read_tgraph_base(r, tags),
        "TGraphErrors" => {
            let _wrapper = r.read_version()?; // TGraphErrors v3
            let mut g = read_tgraph_base(r, tags.as_deref_mut())?;
            let n = g.len();
            let ex = read_basic_array(r, n)?;
            let ey = read_basic_array(r, n)?;
            g.errors = GraphErrors::Symmetric { ex, ey };
            Ok(g)
        }
        "TGraphAsymmErrors" => {
            let _wrapper = r.read_version()?; // TGraphAsymmErrors v3
            let mut g = read_tgraph_base(r, tags)?;
            let n = g.len();
            let ex_low = read_basic_array(r, n)?;
            let ex_high = read_basic_array(r, n)?;
            let ey_low = read_basic_array(r, n)?;
            let ey_high = read_basic_array(r, n)?;
            g.errors = GraphErrors::Asymmetric {
                ex_low,
                ex_high,
                ey_low,
                ey_high,
            };
            Ok(g)
        }
        other => Err(Error::WrongClass {
            name: Some(name.to_string()),
            found: other.to_string(),
            expected: "TGraph, TGraphErrors or TGraphAsymmErrors".to_string(),
        }),
    }
}

#[cfg(test)]
mod function_list_tests {
    use oxiroot_io_core::WBuffer;
    use oxiroot_io_core::raw::{
        K_BYTE_COUNT_MASK, K_CLASS_MASK, K_NEW_CLASS_TAG, write_object_base,
    };

    use super::*;

    /// An `fFunctions` list as ROOT writes it: the first `TF1` named in full,
    /// the second by a back-reference to that class tag.
    fn root_style_list(keylen: usize, functions: &[GraphFunction]) -> Vec<u8> {
        let mut w = WBuffer::new();
        let list_count = w.reserve::<4>();
        let list_start = w.len();
        w.be_u32(K_NEW_CLASS_TAG);
        w.bytes(b"TList\0");
        let list = w.begin_object(5);
        write_object_base(&mut w, 0);
        w.string("");
        w.be_i32(i32::try_from(functions.len()).unwrap());

        let mut first_tag = None;
        for f in functions {
            let count = w.reserve::<4>();
            let tag_at = w.len();
            match first_tag {
                None => {
                    first_tag = Some(tag_at);
                    w.be_u32(K_NEW_CLASS_TAG);
                    w.bytes(b"TF1\0");
                }
                // ROOT's reference to a class: the offset of its first tag,
                // counted from the key (`+ keylen`), plus kMapOffset (2).
                Some(first) => w.be_u32(K_CLASS_MASK | u32::try_from(first + keylen + 2).unwrap()),
            }
            f.write_tf1_body(&mut w, 1, 100);
            let len = u32::try_from(w.len() - tag_at).unwrap();
            w.patch_be_u32(count, K_BYTE_COUNT_MASK | len);
            w.string(""); // the list's per-element option
        }
        w.end_object(list);
        let len = u32::try_from(w.len() - list_start).unwrap();
        w.patch_be_u32(list_count, K_BYTE_COUNT_MASK | len);
        w.into_vec()
    }

    #[test]
    fn functions_named_by_a_class_back_reference_are_read() {
        let keylen = 58;
        let functions = [
            GraphFunction::new("first", "[0]*x", vec![2.0], 0.0, 1.0),
            GraphFunction::new("second", "[0]+x", vec![3.0], 0.0, 1.0),
            GraphFunction::new("third", "[0]-x", vec![4.0], 0.0, 1.0),
        ];
        let bytes = root_style_list(keylen, &functions);

        let mut tags = TagReader::new(keylen);
        let read = read_functions(&mut RBuffer::new(&bytes), &mut tags).unwrap();
        let names: Vec<&str> = read.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["first", "second", "third"]);
        assert_eq!(read[2].params, [4.0]);

        // Without the class-tag map only the function named in full is found.
        let read = read_functions_untracked(&mut RBuffer::new(&bytes)).unwrap();
        assert_eq!(read.len(), 1);
    }

    #[test]
    fn a_null_function_list_is_empty() {
        let bytes = 0u32.to_be_bytes();
        let mut tags = TagReader::new(0);
        let read = read_functions(&mut RBuffer::new(&bytes), &mut tags).unwrap();
        assert!(read.is_empty());
    }
}
