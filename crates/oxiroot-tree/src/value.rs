//! Branch value types.

use oxiroot_io_core::Column;

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
