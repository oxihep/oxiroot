//! Branch value types.

use oxiroot_io_core::column::Primitive;
use oxiroot_io_core::{Column, Error, Result};

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

/// A Rust type a branch can be read as with
/// [`BranchRead::read`](crate::raw::BranchRead::read): the scalar types for scalar
/// branches, `Vec<T>` for arrays and `std::vector<T>`, `String` and
/// `Vec<String>` for strings, `Vec<Vec<T>>` for `std::vector<std::vector<T>>`.
pub trait FromBranch: Sized {
    /// How the type is named in a type-mismatch error.
    fn type_name() -> String;

    /// Take the values out of `column`, one per entry, or hand the column
    /// back if it holds another type.
    ///
    /// # Errors
    ///
    /// The column, unchanged, if it does not hold this type.
    fn from_column(column: Column) -> std::result::Result<Vec<Self>, Column>;
}

/// The values of branch `name` as `T`, or the [`Error::TypeMismatch`] that
/// says what the branch holds.
pub(crate) fn typed<T: FromBranch>(name: &str, column: Column) -> Result<Vec<T>> {
    T::from_column(column).map_err(|found| Error::TypeMismatch {
        name: name.to_string(),
        found: found.type_name(),
        expected: T::type_name(),
    })
}

/// The rows of a list or array column whose values are `inner`.
fn rows<T>(
    column: Column,
    inner: impl FnOnce(Column) -> std::result::Result<Vec<T>, Column>,
) -> std::result::Result<Vec<Vec<T>>, Column> {
    match column {
        Column::List { offsets, values } => match inner(*values) {
            Ok(values) => Ok(split_by_offsets(&offsets, values)),
            Err(values) => Err(Column::List {
                offsets,
                values: Box::new(values),
            }),
        },
        Column::Array { len, values } => match inner(*values) {
            Ok(values) => {
                let mut values = values.into_iter();
                let entries = values.len().checked_div(len).unwrap_or(0);
                Ok((0..entries)
                    .map(|_| values.by_ref().take(len).collect())
                    .collect())
            }
            Err(values) => Err(Column::Array {
                len,
                values: Box::new(values),
            }),
        },
        other => Err(other),
    }
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

macro_rules! from_branch {
    ($($t:ty => $variant:ident;)*) => {$(
        impl FromBranch for $t {
            fn type_name() -> String {
                <$t as Primitive>::NAME.to_string()
            }
            fn from_column(column: Column) -> std::result::Result<Vec<Self>, Column> {
                match column {
                    Column::$variant(v) => Ok(v),
                    other => Err(other),
                }
            }
        }
    )*};
}

from_branch! {
    bool => Bool; i8 => I8; u8 => U8; i16 => I16; u16 => U16; i32 => I32;
    u32 => U32; i64 => I64; u64 => U64; f32 => F32; f64 => F64;
}

impl FromBranch for String {
    fn type_name() -> String {
        "String".to_string()
    }
    fn from_column(column: Column) -> std::result::Result<Vec<Self>, Column> {
        match column {
            Column::String { offsets, text } => Ok(offsets
                .windows(2)
                .map(|w| {
                    let range = usize::try_from(w[0]).unwrap_or(usize::MAX)
                        ..usize::try_from(w[1]).unwrap_or(usize::MAX);
                    text.get(range).unwrap_or_default().to_owned()
                })
                .collect()),
            other => Err(other),
        }
    }
}

impl<T: FromBranch> FromBranch for Vec<T> {
    fn type_name() -> String {
        format!("Vec<{}>", T::type_name())
    }
    fn from_column(column: Column) -> std::result::Result<Vec<Self>, Column> {
        rows(column, T::from_column)
    }
}

/// A column's scalar integer values, widened to `i64`: for a column read as
/// a key (an index's major or minor branch, an event or run number),
/// whatever integer width it was written in. `None` for a column that is not
/// a scalar integer.
pub(crate) fn integers(column: &Column) -> Option<Vec<i64>> {
    Some(match column {
        Column::Bool(v) => v.iter().map(|&b| i64::from(b)).collect(),
        Column::I8(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::U8(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::I16(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::U16(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::I32(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::U32(v) => v.iter().map(|&x| i64::from(x)).collect(),
        Column::I64(v) => v.clone(),
        Column::U64(v) => v.iter().map(|&x| x.cast_signed()).collect(),
        _ => return None,
    })
}
