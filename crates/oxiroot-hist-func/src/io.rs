//! ROOT I/O for the function types: the `WriteRoot`/`ReadRoot` impls, the
//! `Func2D`/`Func3D` object bodies around the shared `Func1D` record, and the
//! `TStreamerInfo`s a file storing them embeds.

use oxiroot_hist::{GraphFunction, hist_streamer_classes};
use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::TagReader;
use oxiroot_io_core::streamer_gen::{
    ClassDef, any, base, basic, object_any_ptr, object_ptr, stl, string,
};
use oxiroot_io_core::{Object, RBuffer, ReadRoot, Result, StreamerSet, WBuffer, WriteRoot};
use std::borrow::Cow;

use crate::func::Func;

// --- write ------------------------------------------------------------------

/// The ROOT class of a `D`-dimensional function. A [`Func`] is only built
/// with one to three dimensions.
const fn class_of<const D: usize>() -> &'static str {
    match D {
        1 => "TF1",
        2 => "TF2",
        _ => "TF3",
    }
}

impl<const D: usize> WriteRoot for Func<D> {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Borrowed(class_of::<D>())
    }
    fn root_name(&self) -> &str {
        self.name()
    }
    fn root_title(&self) -> &str {
        self.title()
    }
    fn write_body(&self, w: &mut WBuffer) {
        match D {
            1 => self.record().write_tf1_body(w, 1, 100),
            2 => write_tf2_body(w, self.record(), 2, self.outer(1)),
            _ => write_tf3_body(w, self.record(), self.outer(1), self.outer(2)),
        }
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        function_classes(D)
    }
}

/// Write a `Func2D` object body (version 4): the `Func1D` base (`fNdim` = `ndim`,
/// `fNpx` = 30), then `fYmin`/`fYmax`, `fNpy`, and an empty `fContour`
/// (`TArrayD`).
fn write_tf2_body(w: &mut WBuffer, f: &GraphFunction, ndim: i32, (ymin, ymax): (f64, f64)) {
    let obj = w.begin_object(4); // TF2 version 4
    f.write_tf1_body(w, ndim, 30); // Func1D base
    w.be_f64(ymin); // fYmin
    w.be_f64(ymax); // fYmax
    w.be_i32(30); // fNpy (ROOT keeps npx == npy by default)
    w.be_i32(0); // fContour: empty TArrayD (fN = 0)
    w.end_object(obj);
}

/// Write a `Func3D` object body (version 3): the `Func2D` base (`fNdim` = 3), then
/// `fZmin`/`fZmax` and `fNpz`.
fn write_tf3_body(w: &mut WBuffer, f: &GraphFunction, y: (f64, f64), (zmin, zmax): (f64, f64)) {
    let obj = w.begin_object(3); // TF3 version 3
    write_tf2_body(w, f, 3, y); // Func2D base
    w.be_f64(zmin); // fZmin
    w.be_f64(zmax); // fZmax
    w.be_i32(30); // fNpz
    w.end_object(obj);
}

// ROOT C++ has these classes compiled in; uproot builds a function model from
// its streamer, so a file storing a TF1/TF2/TF3 embeds them (versions and
// checksums as ROOT writes them).

/// The classes a `dim`-dimensional function needs: ROOT's captured `TF1`, with
/// its `TFormula`, `TF1Parameters` and bases, then the generated `TF2`/`TF3`.
/// The generated `TFormula` and `TF1` are the same class versions as the
/// captured ones, which a file describes once.
fn function_classes(dim: usize) -> Vec<ClassDef<'static>> {
    let mut set = StreamerSet::default();
    set.add_classes(hist_streamer_classes(&["TF1"]));
    set.add_classes(tf_classes(dim));
    set.into_classes()
}

fn tf_classes(dim: usize) -> Vec<ClassDef<'static>> {
    let tformula = ClassDef {
        name: "TFormula".into(),
        version: 14,
        checksum: 3_342_972_029,
        elements: vec![
            base("TNamed", 1),
            stl("fClingParameters", "vector<double>", 1, 8),
            basic("fAllParametersSetted", 18, 1, "bool"),
            stl("fParams", "map<TString,int,TFormulaParamOrder>", 4, 61),
            string("fFormula"),
            basic("fNdim", 3, 4, "int"),
            basic("fNumber", 3, 4, "int"),
            stl("fLinearParts", "vector<TObject*>", 1, 63),
            basic("fVectorized", 18, 1, "bool"),
        ],
    };
    let tf1 = ClassDef {
        name: "TF1".into(),
        version: 12,
        checksum: 1_914_961_880,
        elements: vec![
            base("TNamed", 1),
            base("TAttLine", 2),
            base("TAttFill", 2),
            base("TAttMarker", 3),
            basic("fXmin", 8, 8, "double"),
            basic("fXmax", 8, 8, "double"),
            basic("fNpar", 3, 4, "int"),
            basic("fNdim", 3, 4, "int"),
            basic("fNpx", 3, 4, "int"),
            basic("fType", 3, 4, "TF1::EFType"),
            basic("fNpfits", 3, 4, "int"),
            basic("fNDF", 3, 4, "int"),
            basic("fChisquare", 8, 8, "double"),
            basic("fMinimum", 8, 8, "double"),
            basic("fMaximum", 8, 8, "double"),
            stl("fParErrors", "vector<double>", 1, 8),
            stl("fParMin", "vector<double>", 1, 8),
            stl("fParMax", "vector<double>", 1, 8),
            stl("fSave", "vector<double>", 1, 8),
            basic("fNormalized", 18, 1, "bool"),
            basic("fNormIntegral", 8, 8, "double"),
            object_ptr("fFormula", "TFormula*"),
            object_any_ptr("fParams", "TF1Parameters*"),
            object_ptr("fComposition", "TF1AbsComposition*"),
        ],
    };
    let tf2 = ClassDef {
        name: "TF2".into(),
        version: 4,
        checksum: 3_115_609_752,
        elements: vec![
            base("TF1", 12),
            basic("fYmin", 8, 8, "double"),
            basic("fYmax", 8, 8, "double"),
            basic("fNpy", 3, 4, "int"),
            any("fContour", 24, "TArrayD"),
        ],
    };
    let tf3 = ClassDef {
        name: "TF3".into(),
        version: 3,
        checksum: 3_522_165_386,
        elements: vec![
            base("TF2", 4),
            basic("fZmin", 8, 8, "double"),
            basic("fZmax", 8, 8, "double"),
            basic("fNpz", 3, 4, "int"),
        ],
    };
    let mut classes = vec![tformula, tf1, tf2, tf3];
    classes.truncate(dim + 1);
    classes
}

// --- read -------------------------------------------------------------------

/// Read a `Func2D` object body (version 4): the `Func1D` base then `fYmin`/`fYmax`.
fn read_tf2_body(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<(GraphFunction, (f64, f64))> {
    let tf2 = r.read_version()?; // Func2D v4
    let base = GraphFunction::read_tf1_body(r, tags)?;
    let ymin = r.be_f64()?;
    let ymax = r.be_f64()?;
    let _npy = r.be_i32()?;
    let ncontour = usize::saturating_from(r.be_i32()?.max(0)); // fContour: TArrayD (fN then fN f64s)
    for _ in 0..ncontour {
        r.be_f64()?;
    }
    if let Some(end) = tf2.end {
        r.seek(end)?;
    }
    Ok((base, (ymin, ymax)))
}

/// Read a function's object body: the `TF1` record, and for `TF2` and `TF3`
/// the ranges of the further axes around it.
fn decode<const D: usize>(object: &Object<'_>) -> Result<Func<D>> {
    let (mut r, mut tags) = object.reader()?;
    let tags = &mut tags;
    match D {
        1 => Func::from_record(GraphFunction::read_tf1_body(&mut r, tags)?, [(0.0, 0.0); 2]),
        2 => {
            let (record, y) = read_tf2_body(&mut r, tags)?;
            Func::from_record(record, [y, (0.0, 0.0)])
        }
        _ => {
            let _tf3 = r.read_version()?; // TF3 v3
            let (record, y) = read_tf2_body(&mut r, tags)?;
            let z = (r.be_f64()?, r.be_f64()?);
            Func::from_record(record, [y, z])
        }
    }
}

impl<const D: usize> ReadRoot for Func<D> {
    const CLASS: &'static str = class_of::<D>();

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| decode(object))
    }
}
