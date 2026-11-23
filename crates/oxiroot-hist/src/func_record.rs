//! The `Func1D` object body codec: one reader and one writer for the record a
//! graph's `fFunctions` entries and the standalone `Func1D`/`Func2D`/`Func3D` keys (in
//! `oxiroot-hist-func`) share.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{K_BYTE_COUNT_MASK, TagReader, read_named, skip_versioned, write_named};
use oxiroot_io_core::{RBuffer, Result, WBuffer};

use crate::graph::GraphFunction;
use crate::write::write_object_ptr;

impl GraphFunction {
    /// Decode a streamed `Func1D` object body (version 12), starting at its
    /// byte-count/version header (no class tag). The `TFormula` it embeds gives
    /// [`formula`](Self::formula) and [`params`](Self::params). `tags` is the
    /// class-tag map of the buffer `r` reads.
    ///
    /// # Errors
    /// Returns an error if the buffer ends early or a header is malformed.
    pub fn read_tf1_body(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
        let tf1 = r.read_version()?; // Func1D v12
        let named = read_named(r)?;
        skip_versioned(r)?; // TAttLine
        skip_versioned(r)?; // TAttFill
        skip_versioned(r)?; // TAttMarker
        let xmin = r.be_f64()?;
        let xmax = r.be_f64()?;
        let _npar = r.be_i32()?;
        let _ndim = r.be_i32()?;
        let _npx = r.be_i32()?;
        let _ftype = r.be_i32()?;
        let _npfits = r.be_i32()?;
        let ndf = r.be_i32()?;
        let chi2 = r.be_f64()?;
        let _min = r.be_f64()?;
        let _max = r.be_f64()?;
        let par_errors = read_vector_f64(r)?;
        let par_min = read_vector_f64(r)?;
        let par_max = read_vector_f64(r)?;
        let _save = read_vector_f64(r)?;
        let _normalized = r.u8()?;
        let _norm_integral = r.be_f64()?;
        let (formula, params, param_names) = read_tformula_ptr(r, tags)?;
        if let Some(end) = tf1.end {
            r.seek(end)?; // skip fParams + fComposition
        }
        Ok(Self {
            name: named.name,
            title: named.title,
            formula,
            param_names,
            params,
            par_errors,
            par_min,
            par_max,
            xmin,
            xmax,
            chi2,
            ndf,
        })
    }

    /// Encode this function as a `TF1` object body (version 12), byte-for-byte as
    /// ROOT writes it: the `TNamed`/`TAtt*` bases, the scalar members, the
    /// parameter `vector<double>`s, the `fFormula` `TFormula*`, and null
    /// `fParams`/`fComposition` pointers.
    ///
    /// `ndim` is `fNdim` (1 for a `TF1`, 2 or 3 for a `TF2`/`TF3` base) and `npx`
    /// is `fNpx` (ROOT writes 100 for a `TF1` and 30 for a `TF2`/`TF3` base).
    pub fn write_tf1_body(&self, w: &mut WBuffer, ndim: i32, npx: i32) {
        let npar = self.params.len();
        let obj = w.begin_object(12); // Func1D version 12
        write_named(w, 0, &self.name, &self.title);

        let line = w.begin_object(2); // TAttLine
        w.be_i16(2); // fLineColor (ROOT's Func1D default)
        w.be_i16(1); // fLineStyle
        w.be_i16(2); // fLineWidth (ROOT's Func1D default)
        w.end_object(line);
        let fill = w.begin_object(2); // TAttFill
        w.be_i16(19); // fFillColor (ROOT's Func1D default)
        w.be_i16(0); // fFillStyle
        w.end_object(fill);
        let marker = w.begin_object(3); // TAttMarker
        w.be_i16(1); // fMarkerColor
        w.be_i16(1); // fMarkerStyle
        w.be_f32(1.0); // fMarkerSize
        w.end_object(marker);

        w.be_f64(self.xmin); // fXmin
        w.be_f64(self.xmax); // fXmax
        w.be_i32(i32::saturating_from(npar)); // fNpar
        w.be_i32(ndim); // fNdim
        w.be_i32(npx); // fNpx
        w.be_i32(0); // fType (kFormula)
        w.be_i32(0); // fNpfits
        w.be_i32(self.ndf); // fNDF
        w.be_f64(self.chi2); // fChisquare
        w.be_f64(-1111.0); // fMinimum
        w.be_f64(-1111.0); // fMaximum
        write_vector_f64(w, &self.par_errors); // fParErrors
        write_vector_f64(w, &self.par_min); // fParMin
        write_vector_f64(w, &self.par_max); // fParMax
        write_vector_f64(w, &[]); // fSave (empty)
        w.u8(0); // fNormalized
        w.be_f64(0.0); // fNormIntegral
        write_object_ptr(w, "TFormula", |w| write_tformula_body(w, self, ndim)); // fFormula
        w.be_u32(0); // fParams (TF1Parameters*) = null
        w.be_u32(0); // fComposition (TF1AbsComposition*) = null
        w.end_object(obj);
    }
}

// --- read -------------------------------------------------------------------

/// Read an objectwise `vector<double>` (`[bc][ver][count][count×f64]`).
fn read_vector_f64(r: &mut RBuffer<'_>) -> Result<Vec<f64>> {
    let _bc = r.be_i32()?;
    let _ver = r.be_i16()?;
    let count = usize::saturating_from(r.be_i32()?.max(0));
    (0..count).map(|_| r.be_f64()).collect()
}

/// Read the `fFormula` (`TFormula*`) object pointer, returning
/// `(fFormula string, fClingParameters, parameter names by index)`.
fn read_tformula_ptr(
    r: &mut RBuffer<'_>,
    tags: &mut TagReader,
) -> Result<(String, Vec<f64>, Vec<String>)> {
    let header = tags.read_header(r)?;
    let (Some(_), Some(end)) = (header.class_name, header.end) else {
        // A null pointer, or a formula shared with an earlier function.
        return Ok((String::new(), Vec::new(), Vec::new()));
    };
    let _ver = r.read_version()?; // TFormula v14
    let _named = read_named(r)?;
    let params = read_vector_f64(r)?; // fClingParameters
    let _all_set = r.u8()?;
    let names = read_param_map(r)?; // fParams
    let formula = positional(&r.string()?, &names); // fFormula
    r.seek(end)?;
    let by_index = names_by_index(&names, params.len());
    Ok((formula, params, by_index))
}

/// The names of `n` parameters in index order, from the stored name → index
/// map; `p0`, `p1`, … where the map does not give each index one name.
fn names_by_index(names: &[(String, i32)], n: usize) -> Vec<String> {
    let mut out = vec![None; n];
    for (name, index) in names {
        match usize::try_from(*index).ok().and_then(|i| out.get_mut(i)) {
            Some(slot @ None) => *slot = Some(name.clone()),
            _ => return (0..n).map(|i| format!("p{i}")).collect(),
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(i, name)| name.unwrap_or_else(|| format!("p{i}")))
        .collect()
}

/// Read a `map<TString,int>` (`[bc][ver][count]` then `count` `{TString}{i32}`):
/// each parameter's name and its index.
fn read_param_map(r: &mut RBuffer<'_>) -> Result<Vec<(String, i32)>> {
    let _bc = r.be_i32()?;
    let _ver = r.be_i16()?;
    let count = r.be_i32()?.max(0);
    let mut names = Vec::with_capacity(r.bounded_capacity(usize::saturating_from(count), 5));
    for _ in 0..count {
        let name = r.string()?;
        names.push((name, r.be_i32()?));
    }
    Ok(names)
}

/// `formula` with each parameter named by its position, `[p0]`, `[p1]`, …
///
/// ROOT stores a formula with the names its parameters have: `[p0]` for a
/// formula typed as `[0]`, but `[Constant]`, `[Mean]`, `[Sigma]` for a
/// predefined `gaus`. `names` is the stored map from name to index. The
/// positional form is the one this crate evaluates and writes, so a named
/// formula has to be turned into it: written back with its names but a map of
/// `p0`… it would give ROOT twice the parameters, all unset.
fn positional(formula: &str, names: &[(String, i32)]) -> String {
    let mut out = String::with_capacity(formula.len());
    let mut rest = formula;
    while let Some(open) = rest.find('[') {
        let Some(len) = rest[open..].find(']') else {
            break;
        };
        let name = &rest[open + 1..open + len];
        out.push_str(&rest[..open]);
        match names.iter().find(|(n, _)| n == name) {
            Some((_, index)) => {
                out.push_str("[p");
                out.push_str(&index.to_string());
                out.push(']');
            }
            None => out.push_str(&rest[open..=open + len]),
        }
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
}

// --- write ------------------------------------------------------------------

/// `TFormula::kNotGlobal` (`BIT(10)`): set on a formula owned by another object so
/// that, on read, ROOT does *not* register it in `gROOT`'s global function list
/// (which would re-JIT it and crash in a headless/JIT-less context). ROOT sets
/// this on every embedded `TFormula`; omitting it makes ROOT segfault on read.
const FORMULA_NOT_GLOBAL: u32 = 0x0000_0400;

/// Write a `TFormula` object body (version 14): `Named`, `fClingParameters`,
/// `fAllParametersSetted`, the `fParams` name→index map, the `[pN]`-form formula
/// string, and the trailing scalars/empty `fLinearParts`.
fn write_tformula_body(w: &mut WBuffer, f: &GraphFunction, ndim: i32) {
    let npar = f.params.len();
    let obj = w.begin_object(14); // TFormula version 14
    write_named(w, FORMULA_NOT_GLOBAL, &f.name, &f.title);
    write_vector_f64(w, &f.params); // fClingParameters
    w.u8(1); // fAllParametersSetted
    write_param_map(w, &f.param_names, npar); // fParams (map<TString,int>)
    w.string(&f.formula); // fFormula (in [pN] form)
    w.be_i32(ndim); // fNdim
    w.be_i32(0); // fNumber
    // fLinearParts: an empty objectwise vector<TObject*>.
    let lp = w.reserve::<4>();
    let start = w.len();
    w.be_i16(0x000a); // streamer version
    w.be_i32(0); // count
    let len = u32::saturating_from(w.len() - start);
    w.patch_be_u32(lp, K_BYTE_COUNT_MASK | len);
    w.u8(0); // fVectorized
    w.end_object(obj);
}

/// Write an objectwise `vector<double>`: `{byte count}{ver 0x000a}{count}{f64s}`.
fn write_vector_f64(w: &mut WBuffer, data: &[f64]) {
    let bc = w.reserve::<4>();
    let start = w.len();
    w.be_i16(0x000a); // streamer version
    w.be_i32(i32::saturating_from(data.len()));
    for &d in data {
        w.be_f64(d);
    }
    let len = u32::saturating_from(w.len() - start);
    w.patch_be_u32(bc, K_BYTE_COUNT_MASK | len);
}

/// Write a `TFormula::fParams` `map<TString,int>` for `n` parameters: each
/// name and its index, in index order (ROOT re-sorts on read by its own
/// comparator). `names` gives them, or, if it does not have `n`, `p0`, `p1`, ….
fn write_param_map(w: &mut WBuffer, names: &[String], n: usize) {
    let bc = w.reserve::<4>();
    let start = w.len();
    w.be_i16(0x000a); // streamer version
    w.be_i32(i32::saturating_from(n)); // count
    for i in 0..n {
        match names.get(i).filter(|_| names.len() == n) {
            Some(name) => w.string(name),
            None => w.string(&format!("p{i}")),
        }
        w.be_i32(i32::saturating_from(i));
    }
    let len = u32::saturating_from(w.len() - start);
    w.patch_be_u32(bc, K_BYTE_COUNT_MASK | len);
}

#[cfg(test)]
mod positional_tests {
    use super::positional;

    #[test]
    fn named_parameters_become_positional() {
        let names = [
            ("Constant".to_string(), 0),
            ("Mean".to_string(), 1),
            ("Sigma".to_string(), 2),
        ];
        assert_eq!(
            positional(
                "[Constant]*exp(-0.5*((x-[Mean])/[Sigma])*((x-[Mean])/[Sigma]))",
                &names
            ),
            "[p0]*exp(-0.5*((x-[p1])/[p2])*((x-[p1])/[p2]))"
        );
    }

    #[test]
    fn positional_parameters_and_unknown_names_are_kept() {
        let names = [("p0".to_string(), 0), ("p1".to_string(), 1)];
        assert_eq!(positional("[p0]+[p1]*x", &names), "[p0]+[p1]*x");
        assert_eq!(positional("[q]*x+a[", &names), "[q]*x+a[");
        // A name that is another parameter's positional form does not chain.
        let swapped = [("p1".to_string(), 0), ("a".to_string(), 1)];
        assert_eq!(positional("[p1]+[a]", &swapped), "[p0]+[p1]");
    }
}
