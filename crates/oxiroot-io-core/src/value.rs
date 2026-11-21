//! A dynamic value tree — the output of the generic, streamer-info-driven object
//! reader ([`read_object`](crate::read_object::read_object)).
//!
//! Where the typed models (`Hist1D`, `Graph`, …) decode a *known* class into a Rust
//! struct, [`Value`] represents *any* class: a tree of named members whose shape
//! comes entirely from the file's `TStreamerInfo`. It is what powers rootls /
//! rootprint-style inspection of arbitrary ROOT files.

use core::fmt;

/// A dynamically-typed value decoded from a ROOT object.
///
/// Where the typed models (`Hist1D`, `Graph`, …) decode a *known* class into a Rust
/// struct, a `Value` represents *any* class: a tree of named members whose shape
/// comes entirely from the file's `TStreamerInfo` (see
/// [`read_object`](fn@crate::read_object)).
///
/// Numbers keep their exact on-disk width; strings, arrays, and nested objects
/// nest recursively. Members preserve their stream order (a `Vec`, not a map), so
/// a printed object matches ROOT's layout. A member the reader cannot decode
/// becomes [`Value::Unsupported`] rather than aborting the whole object.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A null object pointer.
    Null,
    /// A boolean (`Bool_t`).
    Bool(bool),
    /// A signed 8-bit integer (`Char_t`).
    I8(i8),
    /// A signed 16-bit integer (`Short_t`).
    I16(i16),
    /// A signed 32-bit integer (`Int_t`).
    I32(i32),
    /// A signed 64-bit integer (`Long64_t`).
    I64(i64),
    /// An unsigned 8-bit integer (`UChar_t`).
    U8(u8),
    /// An unsigned 16-bit integer (`UShort_t`).
    U16(u16),
    /// An unsigned 32-bit integer (`UInt_t`).
    U32(u32),
    /// An unsigned 64-bit integer (`ULong64_t`).
    U64(u64),
    /// A 32-bit float (`Float_t`).
    F32(f32),
    /// A 64-bit float (`Double_t`).
    F64(f64),
    /// A string (`TString`, `char*`, or `std::string`).
    Str(String),
    /// An array or STL container of values (a C array, `T[n]`, or `std::vector`).
    Array(Vec<Self>),
    /// A nested object: its class name and its members in stream order.
    Object {
        /// The object's class name.
        class: String,
        /// The members, `(name, value)`, in the order they appear on disk.
        members: Vec<(String, Self)>,
    },
    /// A slot holding an object that is written elsewhere in the same object:
    /// ROOT streams a shared object once and points at it from every other
    /// place it appears (a `PolyHist`'s `fBins` points at the bins its `fCells`
    /// grid holds in full). The value carries the class it points at; the object
    /// itself is in the tree where it was written.
    Ref {
        /// The class of the object this slot points at.
        class: String,
    },
    /// A member (or object) the reader could not decode — its class/type and why.
    /// The enclosing object's byte count lets decoding continue past it.
    Unsupported {
        /// The class or C++ type name that was not decoded.
        class: String,
        /// Why it was not decoded (e.g. an unhandled `fType`, or a class the
        /// file does not describe).
        reason: String,
    },
}

impl Value {
    /// The class name, for an [`Object`](Value::Object), a [`Ref`](Value::Ref)
    /// or an [`Unsupported`](Value::Unsupported).
    #[must_use]
    pub fn class(&self) -> Option<&str> {
        match self {
            Self::Object { class, .. } | Self::Ref { class } | Self::Unsupported { class, .. } => {
                Some(class)
            }
            _ => None,
        }
    }

    /// The members of an [`Object`](Value::Object).
    #[must_use]
    pub fn members(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object { members, .. } => Some(members),
            _ => None,
        }
    }

    /// Look up an object member by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Self> {
        self.members()?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// The elements of an [`Array`](Value::Array).
    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(v) => Some(v),
            _ => None,
        }
    }

    /// This value as an `f64`, coercing any numeric variant.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Self::Bool(b) => f64::from(b),
            Self::I8(v) => f64::from(v),
            Self::I16(v) => f64::from(v),
            Self::I32(v) => f64::from(v),
            Self::I64(v) => v as f64,
            Self::U8(v) => f64::from(v),
            Self::U16(v) => f64::from(v),
            Self::U32(v) => f64::from(v),
            Self::U64(v) => v as f64,
            Self::F32(v) => f64::from(v),
            Self::F64(v) => v,
            _ => return None,
        })
    }

    /// This value as an `i64`, coercing any integer variant.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Self::Bool(b) => i64::from(b),
            Self::I8(v) => i64::from(v),
            Self::I16(v) => i64::from(v),
            Self::I32(v) => i64::from(v),
            Self::I64(v) => v,
            Self::U8(v) => i64::from(v),
            Self::U16(v) => i64::from(v),
            Self::U32(v) => i64::from(v),
            Self::U64(v) => v.cast_signed(),
            _ => return None,
        })
    }

    /// This value as a `bool`.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Self::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// This value as a string slice.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Whether this is a null object pointer.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Whether every element of an array is a scalar (for compact one-line
    /// printing).
    fn is_scalar(&self) -> bool {
        !matches!(
            self,
            Self::Array(_) | Self::Object { .. } | Self::Ref { .. } | Self::Unsupported { .. }
        )
    }

    /// A short one-line rendering of a scalar/leaf value.
    fn render_scalar(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => b.to_string(),
            Self::I8(v) => v.to_string(),
            Self::I16(v) => v.to_string(),
            Self::I32(v) => v.to_string(),
            Self::I64(v) => v.to_string(),
            Self::U8(v) => v.to_string(),
            Self::U16(v) => v.to_string(),
            Self::U32(v) => v.to_string(),
            Self::U64(v) => v.to_string(),
            Self::F32(v) => v.to_string(),
            Self::F64(v) => v.to_string(),
            Self::Str(s) => format!("{s:?}"),
            _ => String::new(),
        }
    }

    /// Render this value as an indented tree (rootprint-style), one line per
    /// value and no newline after the last. `indent` is the current depth;
    /// `label` is the member name prefix (`None` at the root); `first` is
    /// whether no line has been written yet.
    fn render_into(
        &self,
        out: &mut impl fmt::Write,
        indent: usize,
        label: Option<&str>,
        first: &mut bool,
    ) -> fmt::Result {
        // Each line starts with the newline that ends the one before it, so
        // the text needs no trimming and goes straight to the formatter.
        if !std::mem::take(first) {
            out.write_char('\n')?;
        }
        for _ in 0..indent {
            out.write_str("  ")?;
        }
        if let Some(label) = label {
            write!(out, "{label}: ")?;
        }
        match self {
            Self::Object { class, members } => {
                out.write_str(class)?;
                for (member, value) in members {
                    value.render_into(out, indent + 1, Some(member), first)?;
                }
            }
            Self::Array(items) => {
                write!(out, "[{}]", items.len())?;
                if items.iter().all(Self::is_scalar) {
                    // A short scalar array prints inline; a long one is truncated.
                    for (i, item) in items.iter().take(20).enumerate() {
                        out.write_str(if i == 0 { " " } else { ", " })?;
                        out.write_str(&item.render_scalar())?;
                    }
                    if items.len() > 20 {
                        out.write_str(", …")?;
                    }
                } else {
                    for item in items {
                        item.render_into(out, indent + 1, None, first)?;
                    }
                }
            }
            Self::Ref { class } => write!(out, "<{class} written above>")?,
            Self::Unsupported { class, reason } => {
                write!(out, "<unsupported {class}: {reason}>")?;
            }
            scalar => out.write_str(&scalar.render_scalar())?,
        }
        Ok(())
    }
}

impl fmt::Display for Value {
    /// Render as an indented tree.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.render_into(f, 0, None, &mut true)
    }
}

#[cfg(test)]
mod display_tests {
    use super::Value;

    fn object(class: &str, members: Vec<(&str, Value)>) -> Value {
        Value::Object {
            class: class.to_string(),
            members: members
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
        }
    }

    /// One line per value, members indented under their object, a scalar
    /// array inline, no space after an empty array's count, and no newline
    /// after the last line.
    #[test]
    fn a_value_renders_as_an_indented_tree() {
        let value = object(
            "TThing",
            vec![
                ("fName", Value::Str("x".to_string())),
                ("fN", Value::I32(3)),
                (
                    "fData",
                    Value::Array(vec![Value::F64(1.5), Value::F64(2.0)]),
                ),
                (
                    "fParts",
                    Value::Array(vec![
                        object("TPart", vec![("fId", Value::U8(7))]),
                        Value::Ref {
                            class: "TPart".to_string(),
                        },
                    ]),
                ),
                (
                    "fOdd",
                    Value::Unsupported {
                        class: "TOdd".to_string(),
                        reason: "no streamer".to_string(),
                    },
                ),
                ("fEmpty", Value::Array(vec![])),
            ],
        );
        let expected = "\
TThing
  fName: \"x\"
  fN: 3
  fData: [2] 1.5, 2
  fParts: [2]
    TPart
      fId: 7
    <TPart written above>
  fOdd: <unsupported TOdd: no streamer>
  fEmpty: [0]";
        assert_eq!(value.to_string(), expected);
    }

    #[test]
    fn a_long_scalar_array_is_cut_at_twenty() {
        let long = Value::Array((0..25).map(Value::I32).collect());
        let text = long.to_string();
        assert!(text.starts_with("[25] 0, 1, 2"), "{text}");
        assert!(text.ends_with("18, 19, …"), "{text}");
        assert_eq!(Value::I64(-4).to_string(), "-4");
    }
}
