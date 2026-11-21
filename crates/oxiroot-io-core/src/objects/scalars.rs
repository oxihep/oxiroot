//! [`ObjString`] (ROOT's "collectable string") and [`Parameter`] (a named
//! scalar — a luminosity, an event count, …): small objects stored under their
//! own keys. Both read and write byte-for-byte as ROOT serializes them, so ROOT
//! and uproot read what oxiroot writes and vice versa.

use crate::buffer::{RBuffer, WBuffer};
use crate::error::{Error, Result};
use crate::object_io::{Object, ReadRoot, WriteRoot};
use crate::streamer::{read_object_base, write_object_base};
use crate::streamer_gen::{ClassDef, base, basic, string};
use std::borrow::Cow;

/// `fBits` ROOT writes for a `Parameter`'s embedded `TObject` (`ObjString`'s is
/// `0`). Cosmetic, but matched so written files equal ROOT's byte-for-byte.
const PARAM_BITS: u32 = 0x0020_0000;

// --- ObjString --------------------------------------------------------------

/// An `ObjString` — ROOT's wrapper for a single `TString`, stored under a key
/// (e.g. a metadata label). Build with [`ObjString::new`] then
/// [`named`](ObjString::named); write it with
/// [`FileWriter::add`](crate::FileWriter::add).
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(alias = "TObjString")]
pub struct ObjString {
    name: String,
    value: String,
}

impl ObjString {
    /// An `ObjString` holding `value` (give it a key name with
    /// [`named`](Self::named) before writing).
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            name: String::new(),
            value: value.into(),
        }
    }

    /// Set the key name this string is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The key name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The stored string.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl WriteRoot for ObjString {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Borrowed("TObjString")
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        // ROOT records the ObjString class description as the key title.
        "Collectable string class"
    }
    fn write_body(&self, w: &mut WBuffer) {
        let obj = w.begin_object(1); // ObjString version 1
        write_object_base(w, 0);
        w.string(&self.value); // fString
        w.end_object(obj);
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        vec![tobjstring_class()]
    }
}

pub(crate) fn decode_tobjstring(name: &str, class: &str, object: &[u8]) -> Result<ObjString> {
    if class != "TObjString" {
        return Err(Error::WrongClass {
            name: Some(name.to_string()),
            found: class.to_string(),
            expected: "TObjString".to_string(),
        });
    }
    let mut r = RBuffer::new(object);
    r.read_version()?; // ObjString version
    read_object_base(&mut r)?; // TObject base
    let value = r.string()?; // fString
    Ok(ObjString {
        name: name.to_string(),
        value,
    })
}

// --- Parameter<T> -----------------------------------------------------------

/// The scalar a [`Parameter`] holds, tagged with its C++ type (which selects the
/// `Parameter<…>` class name and the value's on-disk width).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    /// `Parameter<double>`.
    Double(f64),
    /// `Parameter<float>`.
    Float(f32),
    /// `Parameter<int>`.
    Int(i32),
    /// `Parameter<long long>` (a 64-bit integer; ROOT's `Long64_t`).
    Long64(i64),
}

impl ParamValue {
    /// The C++ type name ROOT uses in the `Parameter<…>` class name. `Long64_t`
    /// demangles to `long long` on disk (matching ROOT's own key class and
    /// streamer-info names, so uproot resolves the class).
    fn type_name(&self) -> &'static str {
        match self {
            Self::Double(_) => "double",
            Self::Float(_) => "float",
            Self::Int(_) => "int",
            Self::Long64(_) => "long long",
        }
    }
    fn write(&self, w: &mut WBuffer) {
        match *self {
            Self::Double(v) => w.be_f64(v),
            Self::Float(v) => w.be_f32(v),
            Self::Int(v) => w.be_i32(v),
            Self::Long64(v) => w.be_i64(v),
        }
    }
    /// The value as an `f64` (integers and floats widen losslessly except for a
    /// very large `Long64`).
    #[must_use]
    pub fn as_f64(&self) -> f64 {
        match *self {
            Self::Double(v) => v,
            Self::Float(v) => f64::from(v),
            Self::Int(v) => f64::from(v),
            Self::Long64(v) => v as f64,
        }
    }
}

/// A `Parameter<T>` — a named scalar value stored under a key, the way ROOT
/// stashes a luminosity, an event count, or a cut threshold alongside histograms.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TParameter")]
pub struct Parameter {
    name: String,
    value: ParamValue,
}

impl Parameter {
    /// A `Parameter<double>` named `name`.
    pub fn f64(name: impl Into<String>, value: f64) -> Self {
        Self::new(name, ParamValue::Double(value))
    }
    /// A `Parameter<float>` named `name`.
    pub fn f32(name: impl Into<String>, value: f32) -> Self {
        Self::new(name, ParamValue::Float(value))
    }
    /// A `Parameter<int>` named `name`.
    pub fn i32(name: impl Into<String>, value: i32) -> Self {
        Self::new(name, ParamValue::Int(value))
    }
    /// A `Parameter<Long64_t>` (64-bit integer) named `name`.
    pub fn i64(name: impl Into<String>, value: i64) -> Self {
        Self::new(name, ParamValue::Long64(value))
    }

    fn new(name: impl Into<String>, value: ParamValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// The parameter name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Add `other`'s value to this one, as ROOT's `Parameter<T>::Merge` and
    /// `hadd` do. The name stays as it is.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] and makes no change if the two hold
    /// different types (a `Parameter<int>` and a `Parameter<double>`, say).
    pub fn add(&mut self, other: &Self) -> Result<()> {
        self.value = match (self.value, other.value) {
            (ParamValue::Double(a), ParamValue::Double(b)) => ParamValue::Double(a + b),
            (ParamValue::Float(a), ParamValue::Float(b)) => ParamValue::Float(a + b),
            (ParamValue::Int(a), ParamValue::Int(b)) => ParamValue::Int(a.wrapping_add(b)),
            (ParamValue::Long64(a), ParamValue::Long64(b)) => ParamValue::Long64(a.wrapping_add(b)),
            (a, b) => {
                return Err(Error::InvalidInput(format!(
                    "cannot add a TParameter<{}> to a TParameter<{}>",
                    b.type_name(),
                    a.type_name()
                )));
            }
        };
        Ok(())
    }

    /// The stored value (typed).
    #[must_use]
    pub fn value(&self) -> ParamValue {
        self.value
    }
}

impl WriteRoot for Parameter {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Owned(format!("TParameter<{}>", self.value.type_name()))
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        ""
    }
    fn write_body(&self, w: &mut WBuffer) {
        // [version 2][TObject][fName][fVal] — ROOT's Parameter omits the Named
        // version header and fTitle.
        let obj = w.begin_object(2); // Parameter version 2
        write_object_base(w, PARAM_BITS);
        w.string(&self.name); // fName (Named)
        self.value.write(w); // fVal
        w.end_object(obj);
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        vec![tparameter_class(self.value)]
    }
}

pub(crate) fn decode_tparameter(name: &str, class: &str, object: &[u8]) -> Result<Parameter> {
    let type_name = class
        .strip_prefix("TParameter<")
        .and_then(|s| s.strip_suffix('>'))
        .ok_or_else(|| Error::WrongClass {
            name: Some(name.to_string()),
            found: class.to_string(),
            expected: "TParameter".to_string(),
        })?;
    let mut r = RBuffer::new(object);
    r.read_version()?; // Parameter version
    read_object_base(&mut r)?; // TObject base
    r.skip_string()?; // fName (use the key name for consistency)
    let value = match type_name {
        "double" => ParamValue::Double(r.be_f64()?),
        "float" => ParamValue::Float(r.be_f32()?),
        "int" => ParamValue::Int(r.be_i32()?),
        "Long64_t" | "long" | "long long" => ParamValue::Long64(r.be_i64()?),
        other => {
            return Err(Error::Unsupported(format!(
                "TParameter element type {other:?} is not supported"
            )));
        }
    };
    Ok(Parameter {
        name: name.to_string(),
        value,
    })
}

// ROOT C++ has these classes compiled in, but uproot models a templated
// `Parameter<…>` only from its streamer, so files that store them embed these
// entries. Checksums and versions are ROOT's own (see `scripts/gen_*.cpp`).

/// The `TStreamerInfo` of `ObjString`.
fn tobjstring_class() -> ClassDef<'static> {
    ClassDef {
        name: "TObjString".into(),
        version: 1,
        checksum: 2_626_570_240,
        elements: vec![base("TObject", 1), string("fString")],
    }
}

/// The `TStreamerInfo` of the `Parameter<…>` holding `value`'s type.
fn tparameter_class(value: ParamValue) -> ClassDef<'static> {
    let (checksum, ty, size) = match value {
        ParamValue::Double(_) => (1_968_899_544, 8, 8),
        ParamValue::Float(_) => (1_396_280_242, 5, 4),
        ParamValue::Int(_) => (4_270_151_672, 3, 4),
        ParamValue::Long64(_) => (3_647_805_264, 16, 8),
    };
    let type_name = value.type_name();
    ClassDef {
        name: format!("TParameter<{type_name}>").into(),
        version: 2,
        checksum,
        elements: vec![
            base("TObject", 1),
            string("fName"),
            basic("fVal", ty, size, type_name),
        ],
    }
}

/// The `TStreamerInfo`s for a collection member known only by its class name,
/// when it is one of this module's classes.
pub(crate) fn member_classes(class: &str) -> Vec<ClassDef<'static>> {
    let param = |value| vec![tparameter_class(value)];
    match class {
        "TObjString" => vec![tobjstring_class()],
        "TParameter<double>" => param(ParamValue::Double(0.0)),
        "TParameter<float>" => param(ParamValue::Float(0.0)),
        "TParameter<int>" => param(ParamValue::Int(0)),
        "TParameter<long long>" => param(ParamValue::Long64(0)),
        _ => Vec::new(),
    }
}

impl ReadRoot for ObjString {
    const CLASS: &'static str = "TObjString";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == "TObjString")
            .then(|| decode_tobjstring(object.name, object.class, object.bytes))
    }
}

impl ReadRoot for Parameter {
    const CLASS: &'static str = "TParameter";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        object
            .class
            .starts_with("TParameter<")
            .then(|| decode_tparameter(object.name, object.class, object.bytes))
    }
}
