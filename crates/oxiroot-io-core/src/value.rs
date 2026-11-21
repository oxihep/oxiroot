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

    /// Render this value as an indented tree (rootprint-style). `indent` is the
    /// current depth; `label` is the member name prefix (`None` at the root).
    fn render_into(&self, out: &mut String, indent: usize, label: Option<&str>) -> fmt::Result {
        use fmt::Write as _;

        const PAD: &str = "  ";
        let pad = PAD.repeat(indent);
        let name = label.map_or(String::new(), |l| format!("{l}: "));
        match self {
            Self::Object { class, members } => {
                writeln!(out, "{pad}{name}{class}")?;
                for (member, value) in members {
                    value.render_into(out, indent + 1, Some(member))?;
                }
            }
            Self::Array(items) => {
                if items.iter().all(Self::is_scalar) {
                    // A short scalar array prints inline; a long one is truncated.
                    let shown: Vec<String> =
                        items.iter().take(20).map(Self::render_scalar).collect();
                    let more = if items.len() > 20 { ", …" } else { "" };
                    writeln!(
                        out,
                        "{pad}{name}[{}] {}{}",
                        items.len(),
                        shown.join(", "),
                        more
                    )?;
                } else {
                    writeln!(out, "{pad}{name}[{}]", items.len())?;
                    for item in items {
                        item.render_into(out, indent + 1, None)?;
                    }
                }
            }
            Self::Ref { class } => {
                writeln!(out, "{pad}{name}<{class} written above>")?;
            }
            Self::Unsupported { class, reason } => {
                writeln!(out, "{pad}{name}<unsupported {class}: {reason}>")?;
            }
            scalar => writeln!(out, "{pad}{name}{}", scalar.render_scalar())?,
        }
        Ok(())
    }
}

impl fmt::Display for Value {
    /// Render as an indented tree.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        self.render_into(&mut out, 0, None)?;
        f.write_str(out.trim_end())
    }
}
