//! The flat, Arrow-style column a [`Batch`](crate::Batch) holds, and the typed
//! views over it.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result};

/// One column's values for every entry of a batch, stored flat.
///
/// Variable-length shapes keep cumulative `offsets` (one per entry plus a
/// leading `0`) into a single child, so a column of `std::vector<float>`
/// is two buffers however many entries it has. The variants' fields are
/// public for code that builds or walks columns directly; a
/// [`Batch`](crate::Batch) checks their invariants when a column is added.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Column {
    /// `bool`.
    Bool(Vec<bool>),
    /// `i8`.
    I8(Vec<i8>),
    /// `u8`.
    U8(Vec<u8>),
    /// `i16`.
    I16(Vec<i16>),
    /// `u16`.
    U16(Vec<u16>),
    /// `i32`.
    I32(Vec<i32>),
    /// `u32`.
    U32(Vec<u32>),
    /// `i64`.
    I64(Vec<i64>),
    /// `u64`.
    U64(Vec<u64>),
    /// `f32`.
    F32(Vec<f32>),
    /// `f64`.
    F64(Vec<f64>),
    /// One UTF-8 string per entry: entry `i` is
    /// `bytes[offsets[i]..offsets[i + 1]]`.
    String {
        /// Cumulative byte boundaries, one per entry plus a leading `0`.
        offsets: Vec<u64>,
        /// Every entry's bytes, back to back.
        bytes: Vec<u8>,
    },
    /// A variable number of values per entry: entry `i` is
    /// `values[offsets[i]..offsets[i + 1]]`.
    List {
        /// Cumulative boundaries into `values`, one per entry plus a leading `0`.
        offsets: Vec<u64>,
        /// Every entry's values, back to back.
        values: Box<Column>,
    },
    /// `len` values per entry: entry `i` is `values[i * len..(i + 1) * len]`.
    Array {
        /// Values per entry.
        len: usize,
        /// Every entry's values, back to back.
        values: Box<Column>,
    },
    /// An optional value per entry: `valid[i]` says whether entry `i` has one,
    /// and `values` holds only the present values, in entry order.
    Optional {
        /// Whether each entry holds a value.
        valid: Vec<bool>,
        /// The present values, densely packed.
        values: Box<Column>,
    },
}

/// Apply `$body` to the vector inside any scalar variant (bound as `$v`), and
/// rebuild the same variant from its result.
macro_rules! map_scalar {
    ($col:expr, $v:ident => $body:expr, $other:pat => $fallback:expr) => {
        match $col {
            Column::Bool($v) => Column::Bool($body),
            Column::I8($v) => Column::I8($body),
            Column::U8($v) => Column::U8($body),
            Column::I16($v) => Column::I16($body),
            Column::U16($v) => Column::U16($body),
            Column::I32($v) => Column::I32($body),
            Column::U32($v) => Column::U32($body),
            Column::I64($v) => Column::I64($body),
            Column::U64($v) => Column::U64($body),
            Column::F32($v) => Column::F32($body),
            Column::F64($v) => Column::F64($body),
            $other => $fallback,
        }
    };
}

fn to_usize(n: u64) -> usize {
    usize::saturating_from(n)
}

impl Column {
    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Column::Bool(v) => v.len(),
            Column::I8(v) => v.len(),
            Column::U8(v) => v.len(),
            Column::I16(v) => v.len(),
            Column::U16(v) => v.len(),
            Column::I32(v) => v.len(),
            Column::U32(v) => v.len(),
            Column::I64(v) => v.len(),
            Column::U64(v) => v.len(),
            Column::F32(v) => v.len(),
            Column::F64(v) => v.len(),
            Column::String { offsets, .. } | Column::List { offsets, .. } => {
                offsets.len().saturating_sub(1)
            }
            Column::Array { len, values } => values.len().checked_div(*len).unwrap_or(0),
            Column::Optional { valid, .. } => valid.len(),
        }
    }

    /// Whether the column has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The column's type as Rust spells it: `f32`, `String`, `Vec<f32>`,
    /// `[f64; 3]`, `Option<i32>`, `Vec<Vec<i64>>`.
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Column::Bool(_) => "bool".into(),
            Column::I8(_) => "i8".into(),
            Column::U8(_) => "u8".into(),
            Column::I16(_) => "i16".into(),
            Column::U16(_) => "u16".into(),
            Column::I32(_) => "i32".into(),
            Column::U32(_) => "u32".into(),
            Column::I64(_) => "i64".into(),
            Column::U64(_) => "u64".into(),
            Column::F32(_) => "f32".into(),
            Column::F64(_) => "f64".into(),
            Column::String { .. } => "String".into(),
            Column::List { values, .. } => format!("Vec<{}>", values.type_name()),
            Column::Array { len, values } => format!("[{}; {len}]", values.type_name()),
            Column::Optional { values, .. } => format!("Option<{}>", values.type_name()),
        }
    }

    /// Roughly how many bytes the column's values take: what a writer counts
    /// to decide when a cluster is full.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Column::Bool(v) => v.len(),
            Column::I8(v) => v.len(),
            Column::U8(v) => v.len(),
            Column::I16(v) => 2 * v.len(),
            Column::U16(v) => 2 * v.len(),
            Column::I32(v) => 4 * v.len(),
            Column::U32(v) => 4 * v.len(),
            Column::F32(v) => 4 * v.len(),
            Column::I64(v) => 8 * v.len(),
            Column::U64(v) => 8 * v.len(),
            Column::F64(v) => 8 * v.len(),
            Column::String { offsets, bytes } => 8 * offsets.len() + bytes.len(),
            Column::List { offsets, values } => 8 * offsets.len() + values.byte_size(),
            Column::Array { values, .. } => values.byte_size(),
            Column::Optional { valid, values } => valid.len() + values.byte_size(),
        }
    }

    /// Check the column's invariants: offsets start at `0`, never decrease
    /// and end at the child's length; an array's child holds a whole number of
    /// entries; an optional's child holds one value per valid entry; strings
    /// are UTF-8 entry by entry.
    pub(crate) fn validate(&self, name: &str) -> Result<()> {
        let bad = |why: String| Err(Error::InvalidInput(format!("column {name:?}: {why}")));
        let check_offsets = |offsets: &[u64], inner: usize| -> Result<()> {
            if offsets.first() != Some(&0) {
                return bad("offsets must start with 0".into());
            }
            if offsets.windows(2).any(|w| w[0] > w[1]) {
                return bad("offsets must not decrease".into());
            }
            if offsets.last().map(|&o| to_usize(o)) != Some(inner) {
                return bad(format!("offsets must end at {inner}, the number of values"));
            }
            Ok(())
        };
        match self {
            Column::String { offsets, bytes } => {
                check_offsets(offsets, bytes.len())?;
                for w in offsets.windows(2) {
                    if std::str::from_utf8(&bytes[to_usize(w[0])..to_usize(w[1])]).is_err() {
                        return bad("an entry is not UTF-8".into());
                    }
                }
                Ok(())
            }
            Column::List { offsets, values } => {
                check_offsets(offsets, values.len())?;
                values.validate(name)
            }
            Column::Array { len, values } => {
                if *len == 0 || values.len() % len != 0 {
                    return bad(format!(
                        "{} values do not make whole arrays of {len}",
                        values.len()
                    ));
                }
                values.validate(name)
            }
            Column::Optional { valid, values } => {
                let present = valid.iter().filter(|&&v| v).count();
                if present != values.len() {
                    return bad(format!(
                        "{present} valid entries but {} values",
                        values.len()
                    ));
                }
                values.validate(name)
            }
            _ => Ok(()),
        }
    }

    /// Entries `[start, end)` as a new column (clamped to the column).
    #[must_use]
    pub fn slice(&self, start: usize, end: usize) -> Column {
        let end = end.min(self.len());
        let start = start.min(end);
        map_scalar!(self, v => v[start..end].to_vec(), other => match other {
            Column::String { offsets, bytes } => {
                let (lo, hi) = (to_usize(offsets[start]), to_usize(offsets[end]));
                Column::String {
                    offsets: offsets[start..=end].iter().map(|&o| o - offsets[start]).collect(),
                    bytes: bytes[lo..hi].to_vec(),
                }
            }
            Column::List { offsets, values } => {
                let (lo, hi) = (to_usize(offsets[start]), to_usize(offsets[end]));
                Column::List {
                    offsets: offsets[start..=end].iter().map(|&o| o - offsets[start]).collect(),
                    values: Box::new(values.slice(lo, hi)),
                }
            }
            Column::Array { len, values } => Column::Array {
                len: *len,
                values: Box::new(values.slice(start * len, end * len)),
            },
            Column::Optional { valid, values } => {
                let lo = valid[..start].iter().filter(|&&v| v).count();
                let hi = lo + valid[start..end].iter().filter(|&&v| v).count();
                Column::Optional {
                    valid: valid[start..end].to_vec(),
                    values: Box::new(values.slice(lo, hi)),
                }
            }
            _ => unreachable!("scalars are handled above"),
        })
    }

    /// The entries whose `mask` value is `true`, as a new column. `mask` must
    /// have one value per entry.
    pub(crate) fn filter(&self, mask: &[bool]) -> Column {
        fn keep<T: Copy>(v: &[T], mask: &[bool]) -> Vec<T> {
            v.iter()
                .zip(mask)
                .filter(|&(_, &m)| m)
                .map(|(&x, _)| x)
                .collect()
        }
        map_scalar!(self, v => keep(v, mask), other => match other {
            Column::String { offsets, bytes } => {
                let mut out_off = vec![0u64];
                let mut out = Vec::new();
                for (w, _) in offsets.windows(2).zip(mask).filter(|&(_, &m)| m) {
                    out.extend_from_slice(&bytes[to_usize(w[0])..to_usize(w[1])]);
                    out_off.push(out.len() as u64);
                }
                Column::String { offsets: out_off, bytes: out }
            }
            Column::List { offsets, values } => {
                let mut out_off = vec![0u64];
                let mut child = Vec::with_capacity(values.len());
                for (w, &m) in offsets.windows(2).zip(mask) {
                    let n = to_usize(w[1] - w[0]);
                    child.extend(std::iter::repeat_n(m, n));
                    if m {
                        out_off.push(out_off.last().copied().unwrap_or(0) + (w[1] - w[0]));
                    }
                }
                Column::List { offsets: out_off, values: Box::new(values.filter(&child)) }
            }
            Column::Array { len, values } => {
                let child: Vec<bool> = mask.iter().flat_map(|&m| std::iter::repeat_n(m, *len)).collect();
                Column::Array { len: *len, values: Box::new(values.filter(&child)) }
            }
            Column::Optional { valid, values } => {
                let child: Vec<bool> = valid.iter().zip(mask).filter(|&(&v, _)| v).map(|(_, &m)| m).collect();
                Column::Optional { valid: keep(valid, mask), values: Box::new(values.filter(&child)) }
            }
            _ => unreachable!("scalars are handled above"),
        })
    }

    /// Append `other`'s entries; it must be the same type.
    pub(crate) fn append(&mut self, other: Column) -> std::result::Result<(), Column> {
        fn rebase(offsets: &mut Vec<u64>, more: &[u64]) {
            let base = offsets.last().copied().unwrap_or(0);
            offsets.extend(more.iter().skip(1).map(|&o| o + base));
        }
        match (self, other) {
            (Column::Bool(a), Column::Bool(b)) => a.extend(b),
            (Column::I8(a), Column::I8(b)) => a.extend(b),
            (Column::U8(a), Column::U8(b)) => a.extend(b),
            (Column::I16(a), Column::I16(b)) => a.extend(b),
            (Column::U16(a), Column::U16(b)) => a.extend(b),
            (Column::I32(a), Column::I32(b)) => a.extend(b),
            (Column::U32(a), Column::U32(b)) => a.extend(b),
            (Column::I64(a), Column::I64(b)) => a.extend(b),
            (Column::U64(a), Column::U64(b)) => a.extend(b),
            (Column::F32(a), Column::F32(b)) => a.extend(b),
            (Column::F64(a), Column::F64(b)) => a.extend(b),
            (
                Column::String { offsets, bytes },
                Column::String {
                    offsets: o2,
                    bytes: b2,
                },
            ) => {
                rebase(offsets, &o2);
                bytes.extend(b2);
            }
            (
                Column::List { offsets, values },
                Column::List {
                    offsets: o2,
                    values: v2,
                },
            ) => {
                values.append(*v2)?;
                rebase(offsets, &o2);
            }
            (
                Column::Array { len, values },
                Column::Array {
                    len: l2,
                    values: v2,
                },
            ) if *len == l2 => {
                values.append(*v2)?;
            }
            (
                Column::Optional { valid, values },
                Column::Optional {
                    valid: m2,
                    values: v2,
                },
            ) => {
                values.append(*v2)?;
                valid.extend(m2);
            }
            (_, other) => return Err(other),
        }
        Ok(())
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A scalar type a column can hold: `bool`, `i8` … `u64`, `f32`, `f64`.
pub trait Primitive: Copy + sealed::Sealed + 'static {
    /// The type's name, as in [`Column::type_name`].
    const NAME: &'static str;

    /// The values, if `column` holds this type.
    fn slice(column: &Column) -> Option<&[Self]>;

    /// A column of these values.
    fn into_column(values: Vec<Self>) -> Column;
}

macro_rules! primitive {
    ($($t:ty => $variant:ident),* $(,)?) => {$(
        impl sealed::Sealed for $t {}
        impl Primitive for $t {
            const NAME: &'static str = stringify!($t);
            fn slice(column: &Column) -> Option<&[Self]> {
                match column {
                    Column::$variant(v) => Some(v),
                    _ => None,
                }
            }
            fn into_column(values: Vec<Self>) -> Column {
                Column::$variant(values)
            }
        }
    )*};
}

primitive! {
    bool => Bool, i8 => I8, u8 => U8, i16 => I16, u16 => U16, i32 => I32,
    u32 => U32, i64 => I64, u64 => U64, f32 => F32, f64 => F64,
}

/// Cumulative offsets (with a leading `0`) for rows of these lengths.
fn offsets_of(lengths: impl Iterator<Item = usize>) -> Vec<u64> {
    let mut offsets = vec![0u64];
    let mut acc = 0u64;
    for n in lengths {
        acc += n as u64;
        offsets.push(acc);
    }
    offsets
}

fn strings_column<S: AsRef<str>>(values: &[S]) -> Column {
    let offsets = offsets_of(values.iter().map(|s| s.as_ref().len()));
    let mut bytes = Vec::with_capacity(offsets.last().map_or(0, |&o| to_usize(o)));
    for s in values {
        bytes.extend_from_slice(s.as_ref().as_bytes());
    }
    Column::String { offsets, bytes }
}

/// Data that becomes a [`Column`]: `Vec<T>`, `Vec<Vec<T>>`,
/// `Vec<Vec<Vec<T>>>`, `Vec<[T; N]>` and `Vec<Option<T>>` of a
/// [`Primitive`] `T`; `Vec<String>`, `Vec<&str>` and `Vec<Vec<String>>`; or
/// a [`Column`] itself.
pub trait IntoColumn {
    /// The column.
    fn into_column(self) -> Column;
}

impl IntoColumn for Column {
    fn into_column(self) -> Column {
        self
    }
}

impl<T: Primitive> IntoColumn for Vec<T> {
    fn into_column(self) -> Column {
        T::into_column(self)
    }
}

impl<T: Primitive> IntoColumn for Vec<Vec<T>> {
    fn into_column(self) -> Column {
        let offsets = offsets_of(self.iter().map(Vec::len));
        Column::List {
            offsets,
            values: Box::new(T::into_column(self.concat())),
        }
    }
}

impl<T: Primitive> IntoColumn for Vec<Vec<Vec<T>>> {
    fn into_column(self) -> Column {
        let offsets = offsets_of(self.iter().map(Vec::len));
        Column::List {
            offsets,
            values: Box::new(self.into_iter().flatten().collect::<Vec<_>>().into_column()),
        }
    }
}

impl<T: Primitive, const N: usize> IntoColumn for Vec<[T; N]> {
    fn into_column(self) -> Column {
        Column::Array {
            len: N,
            values: Box::new(T::into_column(self.concat())),
        }
    }
}

impl<T: Primitive> IntoColumn for Vec<Option<T>> {
    fn into_column(self) -> Column {
        Column::Optional {
            valid: self.iter().map(Option::is_some).collect(),
            values: Box::new(T::into_column(self.into_iter().flatten().collect())),
        }
    }
}

impl IntoColumn for Vec<String> {
    fn into_column(self) -> Column {
        strings_column(&self)
    }
}

impl IntoColumn for Vec<&str> {
    fn into_column(self) -> Column {
        strings_column(&self)
    }
}

impl IntoColumn for Vec<Vec<String>> {
    fn into_column(self) -> Column {
        let offsets = offsets_of(self.iter().map(Vec::len));
        let flat: Vec<String> = self.into_iter().flatten().collect();
        Column::List {
            offsets,
            values: Box::new(strings_column(&flat)),
        }
    }
}

/// A list column viewed in place: per-entry offsets over one slice of values.
/// From [`Batch::jagged`](crate::Batch::jagged).
#[derive(Debug, Clone, Copy)]
pub struct Jagged<'a, T> {
    offsets: &'a [u64],
    values: &'a [T],
}

impl<'a, T> Jagged<'a, T> {
    pub(crate) fn new(offsets: &'a [u64], values: &'a [T]) -> Self {
        Jagged { offsets, values }
    }

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

    /// Entry `i`'s values. Panics if `i` is out of range, like slice indexing.
    #[must_use]
    pub fn row(&self, i: usize) -> &'a [T] {
        &self.values[to_usize(self.offsets[i])..to_usize(self.offsets[i + 1])]
    }

    /// Every entry's values, in order.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &'a [T]> + 'a {
        let values = self.values;
        self.offsets
            .windows(2)
            .map(move |w| &values[to_usize(w[0])..to_usize(w[1])])
    }

    /// All values, back to back.
    #[must_use]
    pub fn values(&self) -> &'a [T] {
        self.values
    }

    /// The cumulative offsets: one per entry plus a leading `0`.
    #[must_use]
    pub fn offsets(&self) -> &'a [u64] {
        self.offsets
    }
}

/// A string column viewed in place. From
/// [`Batch::strings`](crate::Batch::strings).
#[derive(Debug, Clone, Copy)]
pub struct Strings<'a> {
    offsets: &'a [u64],
    bytes: &'a [u8],
}

impl<'a> Strings<'a> {
    pub(crate) fn new(offsets: &'a [u64], bytes: &'a [u8]) -> Self {
        Strings { offsets, bytes }
    }

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

    /// Entry `i`'s string. Panics if `i` is out of range, like slice indexing.
    #[must_use]
    pub fn get(&self, i: usize) -> &'a str {
        let b = &self.bytes[to_usize(self.offsets[i])..to_usize(self.offsets[i + 1])];
        // A batch checks every entry is UTF-8 when the column is added.
        std::str::from_utf8(b).unwrap_or_default()
    }

    /// Every entry's string, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a str> + 'a {
        let s = *self;
        (0..s.len()).map(move |i| s.get(i))
    }
}
