//! Branch value types.

use oxiroot_io_core::{Error, Result};

/// The element type of a branch's leaf, derived from the leaf class and its
/// `fIsUnsigned` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeafType {
    /// `TLeafO` — `bool`.
    Bool,
    /// `TLeafB` — `int8_t`.
    I8,
    /// `TLeafB` (unsigned) — `uint8_t`.
    U8,
    /// `TLeafS` — `int16_t`.
    I16,
    /// `TLeafS` (unsigned) — `uint16_t`.
    U16,
    /// `TLeafI` — `int32_t`.
    I32,
    /// `TLeafI` (unsigned) — `uint32_t`.
    U32,
    /// `TLeafL` — `int64_t`.
    I64,
    /// `TLeafL` (unsigned) — `uint64_t`.
    U64,
    /// `TLeafF` — `float`.
    F32,
    /// `TLeafD` — `double`.
    F64,
    /// `TLeafC` — `char*` / `std::string` (length-prefixed, variable length).
    Str,
}

impl LeafType {
    /// Resolve a leaf class name + signedness to an element type.
    pub(crate) fn from_leaf(class: &str, unsigned: bool) -> Option<Self> {
        Some(match (class, unsigned) {
            ("TLeafO", _) => Self::Bool,
            ("TLeafB", false) => Self::I8,
            ("TLeafB", true) => Self::U8,
            ("TLeafS", false) => Self::I16,
            ("TLeafS", true) => Self::U16,
            ("TLeafI", false) => Self::I32,
            ("TLeafI", true) => Self::U32,
            ("TLeafL", false) => Self::I64,
            ("TLeafL", true) => Self::U64,
            ("TLeafF", _) => Self::F32,
            ("TLeafD", _) => Self::F64,
            ("TLeafC", _) => Self::Str,
            _ => return None,
        })
    }

    /// On-disk byte width of one numeric element (1 for the string placeholder).
    pub(crate) fn size(self) -> usize {
        match self {
            Self::Bool | Self::I8 | Self::U8 | Self::Str => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

/// A branch's values across all entries.
///
/// Scalar branches yield a flat vector; fixed-size array (`x[N]`) and
/// variable-length (`x[n]`) branches yield a nested vector (one inner vector per
/// entry); `TLeafC` branches yield strings.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BranchValues {
    /// `bool` branch.
    Bool(Vec<bool>),
    /// `int8_t` branch.
    I8(Vec<i8>),
    /// `uint8_t` branch.
    U8(Vec<u8>),
    /// `int16_t` branch.
    I16(Vec<i16>),
    /// `uint16_t` branch.
    U16(Vec<u16>),
    /// `int32_t` branch.
    I32(Vec<i32>),
    /// `uint32_t` branch.
    U32(Vec<u32>),
    /// `int64_t` branch.
    I64(Vec<i64>),
    /// `uint64_t` branch.
    U64(Vec<u64>),
    /// `float` branch.
    F32(Vec<f32>),
    /// `double` branch.
    F64(Vec<f64>),
    /// Per-entry `bool` array.
    VecBool(Vec<Vec<bool>>),
    /// Per-entry `int8_t` array.
    VecI8(Vec<Vec<i8>>),
    /// Per-entry `uint8_t` array.
    VecU8(Vec<Vec<u8>>),
    /// Per-entry `int16_t` array.
    VecI16(Vec<Vec<i16>>),
    /// Per-entry `uint16_t` array.
    VecU16(Vec<Vec<u16>>),
    /// Per-entry `int32_t` array.
    VecI32(Vec<Vec<i32>>),
    /// Per-entry `uint32_t` array.
    VecU32(Vec<Vec<u32>>),
    /// Per-entry `int64_t` array.
    VecI64(Vec<Vec<i64>>),
    /// Per-entry `uint64_t` array.
    VecU64(Vec<Vec<u64>>),
    /// Per-entry `float` array.
    VecF32(Vec<Vec<f32>>),
    /// Per-entry `double` array.
    VecF64(Vec<Vec<f64>>),
    /// Per-entry string (`TLeafC`).
    Str(Vec<String>),
    /// Per-entry `std::vector<std::string>`.
    VecStr(Vec<Vec<String>>),
    /// A doubly-nested collection (`std::vector<std::vector<T>>`): the cumulative
    /// `offsets` (one per entry, plus a leading `0`) partition the flattened
    /// `items`, whose own per-element values are the inner vectors. Entry `i`'s
    /// value is `items[offsets[i] .. offsets[i+1]]` — a list of inner vectors.
    /// An `RNTuple`'s `FieldValues::Nested` carries its offsets the same way.
    Nested {
        /// Cumulative per-entry boundaries into `items`, with a leading `0`.
        offsets: Vec<u64>,
        /// The flattened inner collections (a `Vec*` variant, one element per
        /// inner vector across all entries).
        items: Box<Self>,
    },
}

/// A jagged (or array) branch viewed as cumulative `offsets` over one flat
/// [`BranchValues`], instead of the per-entry-allocated `Vec<Vec<_>>` form.
///
/// `offsets` has `num_entries + 1` values with `offsets[0] == 0`; entry `i`'s
/// elements are `values[offsets[i] .. offsets[i+1]]`. Built by
/// [`BranchRead::flat`](crate::raw::BranchRead::flat). `values` is
/// always a *scalar* variant (e.g. `F64`), never a `Vec*`.
#[derive(Debug, Clone, PartialEq)]
pub struct Jagged {
    /// Cumulative element boundaries, one per entry plus a leading `0`.
    pub offsets: Vec<u64>,
    /// The flattened element values (a scalar [`BranchValues`] variant).
    pub values: BranchValues,
}

impl Jagged {
    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A Rust type a branch can be read as with
/// [`BranchRead::read`](crate::raw::BranchRead::read): the scalar types for scalar
/// branches, `Vec<T>` for arrays and `std::vector<T>`, `String` and
/// `Vec<String>` for strings, `Vec<Vec<T>>` for `std::vector<std::vector<T>>`.
pub trait FromBranch: Sized {
    /// How the type is named in a type-mismatch error.
    const TYPE_NAME: &'static str;

    /// Take the values out of `values`, or fail with
    /// [`Error::TypeMismatch`] naming `name` if they are another type.
    fn from_values(name: &str, values: BranchValues) -> Result<Vec<Self>>;
}

fn mismatch(name: &str, found: &BranchValues, expected: &str) -> Error {
    Error::TypeMismatch {
        name: name.to_string(),
        found: found.type_name(),
        expected: expected.to_string(),
    }
}

macro_rules! from_branch {
    ($($t:ty => $scalar:ident, $vec:ident;)*) => {$(
        impl FromBranch for $t {
            const TYPE_NAME: &'static str = stringify!($t);
            fn from_values(name: &str, values: BranchValues) -> Result<Vec<Self>> {
                match values {
                    BranchValues::$scalar(v) => Ok(v),
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
        impl FromBranch for Vec<$t> {
            const TYPE_NAME: &'static str = concat!("Vec<", stringify!($t), ">");
            fn from_values(name: &str, values: BranchValues) -> Result<Vec<Self>> {
                match values {
                    BranchValues::$vec(v) => Ok(v),
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
        impl FromBranch for Vec<Vec<$t>> {
            const TYPE_NAME: &'static str = concat!("Vec<Vec<", stringify!($t), ">>");
            fn from_values(name: &str, values: BranchValues) -> Result<Vec<Self>> {
                match values {
                    BranchValues::Nested { offsets, items } => match *items {
                        BranchValues::$vec(inner) => Ok(split_by_offsets(&offsets, inner)),
                        other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                    },
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
    )*};
}

from_branch! {
    bool => Bool, VecBool;
    i8 => I8, VecI8;
    u8 => U8, VecU8;
    i16 => I16, VecI16;
    u16 => U16, VecU16;
    i32 => I32, VecI32;
    u32 => U32, VecU32;
    i64 => I64, VecI64;
    u64 => U64, VecU64;
    f32 => F32, VecF32;
    f64 => F64, VecF64;
    String => Str, VecStr;
}

/// Split `items` into one `Vec` per entry at the cumulative `offsets`.
fn split_by_offsets<T>(offsets: &[u64], items: Vec<T>) -> Vec<Vec<T>> {
    let mut out = Vec::with_capacity(offsets.len().saturating_sub(1));
    let mut items = items.into_iter();
    for w in offsets.windows(2) {
        let n = usize::try_from(w[1].saturating_sub(w[0])).unwrap_or(usize::MAX);
        out.push(items.by_ref().take(n).collect());
    }
    out
}

impl BranchValues {
    /// How this column's type reads in an error: `f32`, `Vec<f32>`,
    /// `Vec<Vec<f32>>`, `String`.
    #[must_use]
    pub fn type_name(&self) -> String {
        let elem = match self.leaf_type() {
            LeafType::Bool => "bool",
            LeafType::I8 => "i8",
            LeafType::U8 => "u8",
            LeafType::I16 => "i16",
            LeafType::U16 => "u16",
            LeafType::I32 => "i32",
            LeafType::U32 => "u32",
            LeafType::I64 => "i64",
            LeafType::U64 => "u64",
            LeafType::F32 => "f32",
            LeafType::F64 => "f64",
            LeafType::Str => "String",
        };
        use BranchValues::*;
        match self {
            Nested { .. } => format!("Vec<Vec<{elem}>>"),
            VecBool(_) | VecI8(_) | VecU8(_) | VecI16(_) | VecU16(_) | VecI32(_) | VecU32(_)
            | VecI64(_) | VecU64(_) | VecF32(_) | VecF64(_) | VecStr(_) => {
                format!("Vec<{elem}>")
            }
            _ => elem.to_string(),
        }
    }

    /// The number of entries (top-level rows).
    #[must_use]
    pub fn len(&self) -> usize {
        use BranchValues::*;
        match self {
            Bool(v) => v.len(),
            I8(v) => v.len(),
            U8(v) => v.len(),
            I16(v) => v.len(),
            U16(v) => v.len(),
            I32(v) => v.len(),
            U32(v) => v.len(),
            I64(v) => v.len(),
            U64(v) => v.len(),
            F32(v) => v.len(),
            F64(v) => v.len(),
            VecBool(v) => v.len(),
            VecI8(v) => v.len(),
            VecU8(v) => v.len(),
            VecI16(v) => v.len(),
            VecU16(v) => v.len(),
            VecI32(v) => v.len(),
            VecU32(v) => v.len(),
            VecI64(v) => v.len(),
            VecU64(v) => v.len(),
            VecF32(v) => v.len(),
            VecF64(v) => v.len(),
            Str(v) => v.len(),
            VecStr(v) => v.len(),
            Nested { offsets, .. } => offsets.len().saturating_sub(1),
        }
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// This branch's element type.
    #[must_use]
    pub fn leaf_type(&self) -> LeafType {
        use BranchValues::*;
        match self {
            Bool(_) | VecBool(_) => LeafType::Bool,
            I8(_) | VecI8(_) => LeafType::I8,
            U8(_) | VecU8(_) => LeafType::U8,
            I16(_) | VecI16(_) => LeafType::I16,
            U16(_) | VecU16(_) => LeafType::U16,
            I32(_) | VecI32(_) => LeafType::I32,
            U32(_) | VecU32(_) => LeafType::U32,
            I64(_) | VecI64(_) => LeafType::I64,
            U64(_) | VecU64(_) => LeafType::U64,
            F32(_) | VecF32(_) => LeafType::F32,
            F64(_) | VecF64(_) => LeafType::F64,
            Str(_) | VecStr(_) => LeafType::Str,
            Nested { items, .. } => items.leaf_type(),
        }
    }

    /// This branch's scalar integer values, widened to `i64` — for a column
    /// read as a key (an index's major or minor branch, an event or run number),
    /// whatever integer width it was written in. `None` for a column that is not
    /// a scalar integer.
    #[must_use]
    pub fn to_i64_vec(&self) -> Option<Vec<i64>> {
        use BranchValues::*;
        Some(match self {
            Bool(v) => v.iter().map(|&b| i64::from(b)).collect(),
            I8(v) => v.iter().map(|&x| i64::from(x)).collect(),
            U8(v) => v.iter().map(|&x| i64::from(x)).collect(),
            I16(v) => v.iter().map(|&x| i64::from(x)).collect(),
            U16(v) => v.iter().map(|&x| i64::from(x)).collect(),
            I32(v) => v.iter().map(|&x| i64::from(x)).collect(),
            U32(v) => v.iter().map(|&x| i64::from(x)).collect(),
            I64(v) => v.clone(),
            U64(v) => v.iter().map(|&x| x.cast_signed()).collect(),
            _ => return None,
        })
    }

    /// Append `other`'s entries onto this column in place, concatenating them.
    ///
    /// Both columns must be the same variant (same element type and nesting);
    /// otherwise an error is returned. This is the per-branch building block for
    /// concatenating trees, as a chain does. For a
    /// [`Nested`](BranchValues::Nested) column the inner `items` are appended and
    /// the `offsets` rebased so the two entry ranges join seamlessly.
    pub fn append(&mut self, other: Self) -> Result<()> {
        use BranchValues::*;
        macro_rules! join {
            ($a:expr, $variant:ident) => {
                match other {
                    $variant(b) => {
                        $a.extend(b);
                        Ok(())
                    }
                    _ => Err(type_mismatch(stringify!($variant))),
                }
            };
        }
        match self {
            Bool(a) => join!(a, Bool),
            I8(a) => join!(a, I8),
            U8(a) => join!(a, U8),
            I16(a) => join!(a, I16),
            U16(a) => join!(a, U16),
            I32(a) => join!(a, I32),
            U32(a) => join!(a, U32),
            I64(a) => join!(a, I64),
            U64(a) => join!(a, U64),
            F32(a) => join!(a, F32),
            F64(a) => join!(a, F64),
            VecBool(a) => join!(a, VecBool),
            VecI8(a) => join!(a, VecI8),
            VecU8(a) => join!(a, VecU8),
            VecI16(a) => join!(a, VecI16),
            VecU16(a) => join!(a, VecU16),
            VecI32(a) => join!(a, VecI32),
            VecU32(a) => join!(a, VecU32),
            VecI64(a) => join!(a, VecI64),
            VecU64(a) => join!(a, VecU64),
            VecF32(a) => join!(a, VecF32),
            VecF64(a) => join!(a, VecF64),
            Str(a) => join!(a, Str),
            VecStr(a) => join!(a, VecStr),
            Nested { offsets, items } => match other {
                Nested {
                    offsets: other_offsets,
                    items: other_items,
                } => {
                    // `other_offsets` starts with a leading `0`; drop it and
                    // rebase the rest onto this column's last boundary.
                    let base = offsets.last().copied().unwrap_or(0);
                    offsets.extend(other_offsets.into_iter().skip(1).map(|o| o + base));
                    items.append(*other_items)
                }
                _ => Err(type_mismatch("Nested")),
            },
        }
    }
}

/// The error returned by [`BranchValues::append`] when the two columns are not
/// the same variant.
fn type_mismatch(expected: &str) -> Error {
    Error::SchemaChanged {
        detail: format!(
            "cannot concatenate branches of different types (expected another {expected} column)"
        ),
    }
}

/// Generate `as_<ty>() -> Option<&[..]>` accessors for the common scalar types.
macro_rules! scalar_accessors {
    ($($method:ident => $variant:ident($ty:ty)),* $(,)?) => {
        impl BranchValues {
            $(
                #[doc = concat!("The values if this is a scalar `", stringify!($variant), "` branch.")]
                #[must_use]
                pub fn $method(&self) -> Option<&[$ty]> {
                    if let BranchValues::$variant(v) = self { Some(v) } else { None }
                }
            )*
        }
    };
}
scalar_accessors! {
    as_bool => Bool(bool), as_i8 => I8(i8), as_u8 => U8(u8),
    as_i16 => I16(i16), as_u16 => U16(u16), as_i32 => I32(i32), as_u32 => U32(u32),
    as_i64 => I64(i64), as_u64 => U64(u64), as_f32 => F32(f32), as_f64 => F64(f64),
    as_str => Str(String),
}
