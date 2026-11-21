//! A typed, per-entry view over `RNTuple` fields.
//!
//! [`FieldValues`] reconstructs a top-level field's values from its physical
//! column(s): scalar leaves map straight from a column, `std::string` combines
//! an index column with a char column, and `std::vector<T>` combines an index
//! column with the element field's column.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result};

use crate::page::ColumnValues;

/// One top-level field's values, one element per entry.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FieldValues {
    /// `bool`.
    Bool(Vec<bool>),
    /// 8-bit signed integer.
    I8(Vec<i8>),
    /// 8-bit unsigned integer.
    U8(Vec<u8>),
    /// 16-bit signed integer.
    I16(Vec<i16>),
    /// 16-bit unsigned integer.
    U16(Vec<u16>),
    /// 32-bit signed integer.
    I32(Vec<i32>),
    /// 64-bit signed integer.
    I64(Vec<i64>),
    /// Unsigned 32-bit integer.
    U32(Vec<u32>),
    /// Unsigned 64-bit integer.
    U64(Vec<u64>),
    /// 32-bit float.
    F32(Vec<f32>),
    /// 64-bit float.
    F64(Vec<f64>),
    /// `std::string`.
    Str(Vec<String>),
    /// `std::vector<bool>`.
    VecBool(Vec<Vec<bool>>),
    /// `std::vector<int8_t>`.
    VecI8(Vec<Vec<i8>>),
    /// `std::vector<uint8_t>`.
    VecU8(Vec<Vec<u8>>),
    /// `std::vector<int16_t>`.
    VecI16(Vec<Vec<i16>>),
    /// `std::vector<uint16_t>`.
    VecU16(Vec<Vec<u16>>),
    /// `std::vector<int32_t>`.
    VecI32(Vec<Vec<i32>>),
    /// `std::vector<int64_t>`.
    VecI64(Vec<Vec<i64>>),
    /// `std::vector<uint32_t>`.
    VecU32(Vec<Vec<u32>>),
    /// `std::vector<uint64_t>`.
    VecU64(Vec<Vec<u64>>),
    /// `std::vector<float>`.
    VecF32(Vec<Vec<f32>>),
    /// `std::vector<double>`.
    VecF64(Vec<Vec<f64>>),
    /// `std::vector<std::string>` — one inner `Vec<String>` per entry.
    VecStr(Vec<Vec<String>>),
    /// A record / struct: its sub-fields in declaration order, each holding one
    /// value per record instance (a struct-of-arrays). At top level every
    /// sub-field has one element per entry; inside a [`Nested`](Self::Nested)
    /// collection they hold the flattened record instances.
    Record(Vec<(String, Self)>),
    /// A collection whose element is itself a collection or a record — e.g.
    /// `std::vector<std::vector<T>>` or `std::vector<MyStruct>`. The cumulative
    /// `offsets` partition the flattened child `items`: element `k` spans
    /// `items[offsets[k]..offsets[k + 1]]`, so `offsets` holds one more value
    /// than there are elements and starts with `0` — the same convention as the
    /// tree's `BranchValues`.
    Nested {
        /// Cumulative element boundaries: a leading `0`, then one per element of
        /// the enclosing level.
        offsets: Vec<u64>,
        /// The flattened child values, partitioned by `offsets`.
        items: Box<Self>,
    },
    /// A `std::variant`: each element selects one `alternative` by its 1-based
    /// `tag` (`0` = valueless) and reads it at `index`. The alternatives are
    /// stored densely, so element `e`'s value is
    /// `alternatives[tags[e] - 1].1` at position `indices[e]`.
    Variant {
        /// The variant alternatives (name, densely-packed values), in order.
        alternatives: Vec<(String, Self)>,
        /// Per element: the 1-based active alternative (`0` = valueless).
        tags: Vec<u32>,
        /// Per element: the position within the active alternative's column.
        indices: Vec<u64>,
    },
    /// A nullable / "late" field (`std::optional<T>` or `std::unique_ptr<T>`):
    /// `present[i]` flags whether entry `i` holds a value, and `values` holds the
    /// present values densely (so `values.len()` equals the count of `true`s in
    /// `present`). Use the `opt_*` helpers to zip them into `Vec<Option<T>>`, or
    /// read `present` / `values` directly.
    Opt {
        /// Per-entry presence (length = entry count).
        present: Vec<bool>,
        /// The present values, densely packed; a scalar variant (e.g. `F32`).
        values: Box<Self>,
    },
}

impl FieldValues {
    /// The number of elements at this level — entries, for a top-level field.
    #[must_use]
    pub fn len(&self) -> usize {
        use FieldValues::*;
        match self {
            Bool(v) => v.len(),
            I8(v) => v.len(),
            U8(v) => v.len(),
            I16(v) => v.len(),
            U16(v) => v.len(),
            I32(v) => v.len(),
            I64(v) => v.len(),
            U32(v) => v.len(),
            U64(v) => v.len(),
            F32(v) => v.len(),
            F64(v) => v.len(),
            Str(v) => v.len(),
            VecBool(v) => v.len(),
            VecI8(v) => v.len(),
            VecU8(v) => v.len(),
            VecI16(v) => v.len(),
            VecU16(v) => v.len(),
            VecI32(v) => v.len(),
            VecI64(v) => v.len(),
            VecU32(v) => v.len(),
            VecU64(v) => v.len(),
            VecF32(v) => v.len(),
            VecF64(v) => v.len(),
            VecStr(v) => v.len(),
            Record(fields) => fields.first().map_or(0, |(_, f)| f.len()),
            Nested { offsets, .. } => offsets.len().saturating_sub(1),
            Variant { tags, .. } => tags.len(),
            Opt { present, .. } => present.len(),
        }
    }

    /// Whether there are no elements at this level.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Truncate to the first `n` entries in place (a no-op if already `<= n`).
    ///
    /// Handles every variant, including the offset- and dense-packed collection
    /// forms: a [`Nested`](FieldValues::Nested) keeps only the child items its
    /// first `n` boundaries span, an [`Opt`](FieldValues::Opt) keeps only the
    /// present values among the first `n`, and a [`Variant`](FieldValues::Variant)
    /// keeps the entry-ordered prefix of each alternative it still references — so
    /// `values.truncate(n)` equals reading only those first `n` entries.
    pub fn truncate(&mut self, n: usize) {
        use FieldValues::*;
        if n >= self.len() {
            return;
        }
        match self {
            Bool(v) => v.truncate(n),
            I8(v) => v.truncate(n),
            U8(v) => v.truncate(n),
            I16(v) => v.truncate(n),
            U16(v) => v.truncate(n),
            I32(v) => v.truncate(n),
            I64(v) => v.truncate(n),
            U32(v) => v.truncate(n),
            U64(v) => v.truncate(n),
            F32(v) => v.truncate(n),
            F64(v) => v.truncate(n),
            Str(v) => v.truncate(n),
            VecBool(v) => v.truncate(n),
            VecI8(v) => v.truncate(n),
            VecU8(v) => v.truncate(n),
            VecI16(v) => v.truncate(n),
            VecU16(v) => v.truncate(n),
            VecI32(v) => v.truncate(n),
            VecI64(v) => v.truncate(n),
            VecU32(v) => v.truncate(n),
            VecU64(v) => v.truncate(n),
            VecF32(v) => v.truncate(n),
            VecF64(v) => v.truncate(n),
            VecStr(v) => v.truncate(n),
            // Every sub-field carries one value per entry.
            Record(fields) => {
                for (_, f) in fields.iter_mut() {
                    f.truncate(n);
                }
            }
            // `offsets` bounds each entry, leading `0` included; the child
            // holds the flattened items, so keep only those the first `n`
            // entries span.
            Nested { offsets, items } => {
                let items_len = offsets.get(n).map_or(0, |&o| usize::saturating_from(o));
                offsets.truncate(n + 1);
                items.truncate(items_len);
            }
            // Alternatives are densely packed in entry order, so each keeps the
            // prefix matching how many of the first `n` entries selected it (tags
            // are 1-based; `0` is valueless).
            Variant {
                alternatives,
                tags,
                indices,
            } => {
                let mut counts = vec![0usize; alternatives.len()];
                for &tag in tags.iter().take(n) {
                    if let Some(slot) = (tag as usize)
                        .checked_sub(1)
                        .and_then(|a| counts.get_mut(a))
                    {
                        *slot += 1;
                    }
                }
                tags.truncate(n);
                indices.truncate(n);
                for (kept, (_, values)) in counts.iter().zip(alternatives.iter_mut()) {
                    values.truncate(*kept);
                }
            }
            // `present` flags each entry; `values` holds only the present ones.
            Opt { present, values } => {
                let kept = present.iter().take(n).filter(|&&p| p).count();
                present.truncate(n);
                values.truncate(kept);
            }
        }
    }

    /// Drop the first `n` entries in place (everything, if `n >= len`) — the
    /// counterpart of [`truncate`](Self::truncate), so
    /// `skip(a); truncate(b - a)` keeps entries `[a, b)`. Collection offsets
    /// are rebased to start at `0`, and each variant alternative or optional
    /// keeps only the values the remaining entries reference.
    pub fn skip(&mut self, n: usize) {
        use FieldValues::*;
        let n = n.min(self.len());
        if n == 0 {
            return;
        }
        fn drop_front<T>(v: &mut Vec<T>, n: usize) {
            v.drain(..n.min(v.len()));
        }
        match self {
            Bool(v) => drop_front(v, n),
            I8(v) => drop_front(v, n),
            U8(v) => drop_front(v, n),
            I16(v) => drop_front(v, n),
            U16(v) => drop_front(v, n),
            I32(v) => drop_front(v, n),
            I64(v) => drop_front(v, n),
            U32(v) => drop_front(v, n),
            U64(v) => drop_front(v, n),
            F32(v) => drop_front(v, n),
            F64(v) => drop_front(v, n),
            Str(v) => drop_front(v, n),
            VecBool(v) => drop_front(v, n),
            VecI8(v) => drop_front(v, n),
            VecU8(v) => drop_front(v, n),
            VecI16(v) => drop_front(v, n),
            VecU16(v) => drop_front(v, n),
            VecI32(v) => drop_front(v, n),
            VecI64(v) => drop_front(v, n),
            VecU32(v) => drop_front(v, n),
            VecU64(v) => drop_front(v, n),
            VecF32(v) => drop_front(v, n),
            VecF64(v) => drop_front(v, n),
            VecStr(v) => drop_front(v, n),
            Record(fields) => {
                for (_, f) in fields.iter_mut() {
                    f.skip(n);
                }
            }
            Nested { offsets, items } => {
                let first = offsets.get(n).copied().unwrap_or(0);
                drop_front(offsets, n);
                for o in offsets.iter_mut() {
                    *o = o.saturating_sub(first);
                }
                items.skip(usize::saturating_from(first));
            }
            Variant {
                alternatives,
                tags,
                indices,
            } => {
                let mut dropped = vec![0u64; alternatives.len()];
                for &tag in tags.iter().take(n) {
                    if let Some(slot) = (tag as usize)
                        .checked_sub(1)
                        .and_then(|a| dropped.get_mut(a))
                    {
                        *slot += 1;
                    }
                }
                drop_front(tags, n);
                drop_front(indices, n);
                for (&tag, index) in tags.iter().zip(indices.iter_mut()) {
                    if let Some(&d) = (tag as usize).checked_sub(1).and_then(|a| dropped.get(a)) {
                        *index = index.saturating_sub(d);
                    }
                }
                for (&d, (_, values)) in dropped.iter().zip(alternatives.iter_mut()) {
                    values.skip(usize::saturating_from(d));
                }
            }
            Opt { present, values } => {
                let dropped = present.iter().take(n).filter(|&&p| p).count();
                drop_front(present, n);
                values.skip(dropped);
            }
        }
    }

    /// How this field's type reads in an error: `f64`, `Vec<f32>`, `String`,
    /// `Option<i32>`, `record`, ….
    #[must_use]
    pub fn type_name(&self) -> String {
        use FieldValues::*;
        match self {
            Bool(_) => "bool".into(),
            I8(_) => "i8".into(),
            U8(_) => "u8".into(),
            I16(_) => "i16".into(),
            U16(_) => "u16".into(),
            I32(_) => "i32".into(),
            I64(_) => "i64".into(),
            U32(_) => "u32".into(),
            U64(_) => "u64".into(),
            F32(_) => "f32".into(),
            F64(_) => "f64".into(),
            Str(_) => "String".into(),
            VecBool(_) => "Vec<bool>".into(),
            VecI8(_) => "Vec<i8>".into(),
            VecU8(_) => "Vec<u8>".into(),
            VecI16(_) => "Vec<i16>".into(),
            VecU16(_) => "Vec<u16>".into(),
            VecI32(_) => "Vec<i32>".into(),
            VecI64(_) => "Vec<i64>".into(),
            VecU32(_) => "Vec<u32>".into(),
            VecU64(_) => "Vec<u64>".into(),
            VecF32(_) => "Vec<f32>".into(),
            VecF64(_) => "Vec<f64>".into(),
            VecStr(_) => "Vec<String>".into(),
            Record(_) => "record".into(),
            Nested { items, .. } => format!("Vec<{}>", items.type_name()),
            Variant { .. } => "variant".into(),
            Opt { values, .. } => format!("Option<{}>", values.type_name()),
        }
    }

    /// Append `other`'s entries onto this field in place, concatenating them.
    ///
    /// Both fields must be the same scalar or vector variant; otherwise an error
    /// is returned. Structural fields ([`Record`](FieldValues::Record),
    /// [`Nested`](FieldValues::Nested), [`Variant`](FieldValues::Variant), and
    /// [`Opt`](FieldValues::Opt)) are not concatenable here.
    pub fn append(&mut self, other: Self) -> Result<()> {
        use FieldValues::*;
        macro_rules! join {
            ($a:expr, $variant:ident) => {
                match other {
                    $variant(b) => {
                        $a.extend(b);
                        Ok(())
                    }
                    _ => Err(fv_mismatch(stringify!($variant))),
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
            I64(a) => join!(a, I64),
            U32(a) => join!(a, U32),
            U64(a) => join!(a, U64),
            F32(a) => join!(a, F32),
            F64(a) => join!(a, F64),
            Str(a) => join!(a, Str),
            VecBool(a) => join!(a, VecBool),
            VecI8(a) => join!(a, VecI8),
            VecU8(a) => join!(a, VecU8),
            VecI16(a) => join!(a, VecI16),
            VecU16(a) => join!(a, VecU16),
            VecI32(a) => join!(a, VecI32),
            VecI64(a) => join!(a, VecI64),
            VecU32(a) => join!(a, VecU32),
            VecU64(a) => join!(a, VecU64),
            VecF32(a) => join!(a, VecF32),
            VecF64(a) => join!(a, VecF64),
            VecStr(a) => join!(a, VecStr),
            Record(_) | Nested { .. } | Variant { .. } | Opt { .. } => Err(Error::Unsupported(
                "cannot concatenate a structural RNTuple field (record, nested \
                 collection, variant, or optional)"
                    .into(),
            )),
        }
    }
}

/// The error returned by [`FieldValues::append`] when the two fields are not the
/// same variant.
fn fv_mismatch(expected: &str) -> Error {
    Error::SchemaChanged {
        detail: format!(
            "cannot concatenate RNTuple fields of different types (expected another {expected} field)"
        ),
    }
}

/// Generate `opt_<ty>(&self) -> Option<Vec<Option<T>>>` accessors that zip an
/// [`Opt`](FieldValues::Opt) field's presence mask with its dense values.
macro_rules! opt_accessors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl FieldValues {
            $(
                #[doc = concat!("If this is an `Opt` over `", stringify!($variant),
                    "`, zip it into a per-entry `Vec<Option<", stringify!($elem),
                    ">>`; `None` if the value type does not match.")]
                #[must_use]
                pub fn $method(&self) -> Option<Vec<Option<$elem>>> {
                    match self {
                        FieldValues::Opt { present, values } => match values.as_ref() {
                            FieldValues::$variant(v) => {
                                let mut it = v.iter().cloned();
                                Some(
                                    present
                                        .iter()
                                        .map(|&p| if p { it.next() } else { None })
                                        .collect(),
                                )
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                }
            )*
        }
    };
}

opt_accessors! {
    opt_bool => Bool(bool),
    opt_i32 => I32(i32),
    opt_i64 => I64(i64),
    opt_u32 => U32(u32),
    opt_u64 => U64(u64),
    opt_f32 => F32(f32),
    opt_f64 => F64(f64),
}

/// Map a scalar leaf column to per-entry field values.
pub(crate) fn scalar(values: ColumnValues) -> Result<FieldValues> {
    Ok(match values {
        ColumnValues::Bits(v) => FieldValues::Bool(v),
        ColumnValues::I8(v) => FieldValues::I8(v),
        ColumnValues::U8(v) => FieldValues::U8(v),
        ColumnValues::I16(v) => FieldValues::I16(v),
        ColumnValues::U16(v) => FieldValues::U16(v),
        ColumnValues::I32(v) => FieldValues::I32(v),
        ColumnValues::I64(v) => FieldValues::I64(v),
        ColumnValues::U32(v) => FieldValues::U32(v),
        ColumnValues::U64(v) => FieldValues::U64(v),
        ColumnValues::F32(v) => FieldValues::F32(v),
        ColumnValues::F64(v) => FieldValues::F64(v),
        ColumnValues::Bytes(_) => {
            return Err(Error::Unsupported(
                "byte-typed scalar fields are not supported".into(),
            ));
        }
        ColumnValues::Switch(_) => {
            return Err(Error::Format(
                "a Switch column is not a scalar leaf (it belongs to a variant)".into(),
            ));
        }
    })
}

/// Reconstruct `std::string` values from cumulative offsets and char bytes.
pub(crate) fn strings(offsets: &[u64], bytes: &[u8]) -> Result<FieldValues> {
    let mut start = 0usize;
    let mut out = Vec::with_capacity(offsets.len());
    for &end in offsets {
        let end = usize::saturating_from(end);
        let slice = bytes
            .get(start..end)
            .ok_or_else(|| Error::Format("string offset out of range".into()))?;
        out.push(String::from_utf8(slice.to_vec()).map_err(|_| Error::InvalidUtf8)?);
        start = end;
    }
    Ok(FieldValues::Str(out))
}

/// Prepend the leading `0` that [`FieldValues::Nested`] carries to on-disk
/// offsets, which hold one cumulative end per element and no leading `0`.
pub(crate) fn leading_zero(offsets: Vec<u64>) -> Vec<u64> {
    let mut out = Vec::with_capacity(offsets.len() + 1);
    out.push(0);
    out.extend(offsets);
    out
}

/// Group a collection's flattened child `items` by its cumulative `offsets`.
/// Scalar and string children materialize into the ergonomic flat `Vec*`
/// variants; a collection- or record-valued child is wrapped in
/// [`FieldValues::Nested`] (so arbitrarily deep nesting stays representable).
pub(crate) fn collect(offsets: Vec<u64>, items: FieldValues) -> Result<FieldValues> {
    use FieldValues::*;
    Ok(match items {
        Bool(v) => VecBool(group(&offsets, &v)?),
        I8(v) => VecI8(group(&offsets, &v)?),
        U8(v) => VecU8(group(&offsets, &v)?),
        I16(v) => VecI16(group(&offsets, &v)?),
        U16(v) => VecU16(group(&offsets, &v)?),
        I32(v) => VecI32(group(&offsets, &v)?),
        I64(v) => VecI64(group(&offsets, &v)?),
        U32(v) => VecU32(group(&offsets, &v)?),
        U64(v) => VecU64(group(&offsets, &v)?),
        F32(v) => VecF32(group(&offsets, &v)?),
        F64(v) => VecF64(group(&offsets, &v)?),
        Str(v) => VecStr(group(&offsets, &v)?),
        // The flat variants above hold their elements grouped, so only this one
        // exposes offsets — in the public form, with a leading `0`.
        other => Nested {
            offsets: leading_zero(offsets),
            items: Box::new(other),
        },
    })
}

/// Present an optional / `unique_ptr` collection (each entry 0 or 1 elements) as a
/// nullable [`FieldValues::Opt`]: derive per-entry presence from the cumulative
/// `offsets` (entry `i` is present iff its count increased), keeping the
/// densely-packed present `values`.
pub(crate) fn optional(offsets: &[u64], values: FieldValues) -> Result<FieldValues> {
    let mut present = Vec::with_capacity(offsets.len());
    let mut prev = 0u64;
    for &o in offsets {
        present.push(o > prev);
        prev = o;
    }
    Ok(FieldValues::Opt {
        present,
        values: Box::new(values),
    })
}

/// Group a fixed-size array field's flattened `items` into one chunk of `n` per
/// entry (e.g. `std::array<T, n>` or `std::bitset<n>`), reusing [`collect`].
pub(crate) fn chunk(items: FieldValues, n: usize) -> Result<FieldValues> {
    let total = items.len();
    if n == 0 {
        return Err(Error::Format("array field has zero repetition".into()));
    }
    if !total.is_multiple_of(n) {
        return Err(Error::Format(format!(
            "array field element count {total} is not a multiple of {n}"
        )));
    }
    let offsets: Vec<u64> = (1..=total / n).map(|k| (k * n) as u64).collect();
    collect(offsets, items)
}

fn group<T: Clone>(offsets: &[u64], data: &[T]) -> Result<Vec<Vec<T>>> {
    let mut start = 0usize;
    let mut out = Vec::with_capacity(offsets.len());
    for &end in offsets {
        let end = usize::saturating_from(end);
        let slice = data
            .get(start..end)
            .ok_or_else(|| Error::Format("collection offset out of range".into()))?;
        out.push(slice.to_vec());
        start = end;
    }
    Ok(out)
}

/// A `std::vector` field viewed as cumulative `offsets` over one flat
/// [`FieldValues`] (a scalar variant), instead of `Vec<Vec<T>>`: what
/// [`FieldRead::flat`](crate::raw::FieldRead::flat) returns.
///
/// `offsets` has `entries + 1` values with `offsets[0] == 0`; entry `i`'s
/// elements are `values[offsets[i] .. offsets[i + 1]]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Jagged {
    /// Cumulative element boundaries, one per entry plus a leading `0`.
    pub offsets: Vec<u64>,
    /// The flattened element values (a scalar [`FieldValues`] variant).
    pub values: FieldValues,
}

/// A Rust type a field can be read as with
/// [`FieldRead::read`](crate::raw::FieldRead::read): the scalar types, `String`,
/// `Vec<T>` for `std::vector<T>`, `Vec<Vec<T>>` for
/// `std::vector<std::vector<T>>`, and `Option<T>` for `std::optional<T>`.
pub trait FromField: Sized {
    /// How the type is named in a type-mismatch error.
    const TYPE_NAME: &'static str;

    /// Take the values out of `values`, or fail with
    /// [`Error::TypeMismatch`] naming `name` if they are another type.
    fn from_values(name: &str, values: FieldValues) -> Result<Vec<Self>>;
}

fn mismatch(name: &str, found: &FieldValues, expected: &str) -> Error {
    Error::TypeMismatch {
        name: name.to_string(),
        found: found.type_name(),
        expected: expected.to_string(),
    }
}

/// Split `items` into one `Vec` per entry at the cumulative `offsets`.
fn split_by_offsets<T>(offsets: &[u64], items: Vec<T>) -> Vec<Vec<T>> {
    let mut out = Vec::with_capacity(offsets.len().saturating_sub(1));
    let mut items = items.into_iter();
    for w in offsets.windows(2) {
        out.push(
            items
                .by_ref()
                .take(usize::saturating_from(w[1].saturating_sub(w[0])))
                .collect(),
        );
    }
    out
}

macro_rules! from_field {
    ($($t:ty => $scalar:ident, $vec:ident;)*) => {$(
        impl FromField for $t {
            const TYPE_NAME: &'static str = stringify!($t);
            fn from_values(name: &str, values: FieldValues) -> Result<Vec<Self>> {
                match values {
                    FieldValues::$scalar(v) => Ok(v),
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
        impl FromField for Vec<$t> {
            const TYPE_NAME: &'static str = concat!("Vec<", stringify!($t), ">");
            fn from_values(name: &str, values: FieldValues) -> Result<Vec<Self>> {
                match values {
                    FieldValues::$vec(v) => Ok(v),
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
        impl FromField for Vec<Vec<$t>> {
            const TYPE_NAME: &'static str = concat!("Vec<Vec<", stringify!($t), ">>");
            fn from_values(name: &str, values: FieldValues) -> Result<Vec<Self>> {
                match values {
                    FieldValues::Nested { offsets, items } => match *items {
                        FieldValues::$vec(inner) => Ok(split_by_offsets(&offsets, inner)),
                        other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                    },
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
        impl FromField for Option<$t> {
            const TYPE_NAME: &'static str = concat!("Option<", stringify!($t), ">");
            fn from_values(name: &str, values: FieldValues) -> Result<Vec<Self>> {
                match values {
                    FieldValues::Opt { present, values } => match *values {
                        FieldValues::$scalar(dense) => {
                            let mut dense = dense.into_iter();
                            Ok(present.iter().map(|&p| if p { dense.next() } else { None }).collect())
                        }
                        other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                    },
                    other => Err(mismatch(name, &other, Self::TYPE_NAME)),
                }
            }
        }
    )*};
}

from_field! {
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
