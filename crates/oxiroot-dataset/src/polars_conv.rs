//! [`Batch`] ↔ Polars [`DataFrame`] (the `polars` feature).
//!
//! Each column becomes a Polars series of the matching type: numbers and
//! booleans, `String`, `List` (from a list), `Array` (from a fixed array),
//! `Struct`, with nulls for an `Optional`. Polars has one level of nulls, so
//! an `Optional` of an `Optional` folds into it: `Some(None)` and `None` are
//! both null. Coming back, a series with nulls becomes an `Optional` over its
//! non-null values.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result};
use polars::prelude::{CompatLevel, DataFrame, IntoColumn as _, Series};
use polars_arrow::array::{
    Array, BooleanArray, FixedSizeListArray, ListArray, PrimitiveArray, StructArray, Utf8Array,
    Utf8ViewArray,
};
use polars_arrow::bitmap::Bitmap;
use polars_arrow::datatypes::{ArrowDataType, Field};
use polars_arrow::offset::OffsetsBuffer;

use crate::batch::Batch;
use crate::column::{Column, IntoColumn};

impl Batch {
    /// This batch as a Polars [`DataFrame`], copying the values.
    ///
    /// # Errors
    ///
    /// A column Polars cannot hold (an optional of an optional, say).
    pub fn to_polars(&self) -> Result<DataFrame> {
        self.clone().into_polars()
    }

    /// This batch as a Polars [`DataFrame`], moving the values where Polars
    /// allows.
    ///
    /// # Errors
    ///
    /// As [`to_polars`](Self::to_polars).
    pub fn into_polars(self) -> Result<DataFrame> {
        let height = self.len();
        let columns = self
            .into_columns()
            .map(|(name, c)| {
                let series = Series::from_arrow(name.into(), to_array(c)?).map_err(Error::other)?;
                Ok(series.into_column())
            })
            .collect::<Result<Vec<_>>>()?;
        DataFrame::new(height, columns).map_err(Error::other)
    }

    /// A Polars [`DataFrame`] as a batch, copying the values. Columns with
    /// nulls become [`Column::Optional`]s.
    ///
    /// # Errors
    ///
    /// A Polars type with no column counterpart (a date, a categorical, a
    /// decimal, …).
    pub fn from_polars(frame: &DataFrame) -> Result<Batch> {
        let names: Vec<String> = frame
            .get_column_names()
            .into_iter()
            .map(|n| n.to_string())
            .collect();
        let arrays = frame.rechunk_to_arrow(CompatLevel::oldest());
        let mut out = Batch::new();
        for (name, array) in names.into_iter().zip(arrays) {
            let column = from_array(array.as_ref(), &name)?;
            out.insert(name, column)?;
        }
        Ok(out)
    }
}

fn item(child: &dyn Array) -> Box<Field> {
    Box::new(Field::new("item".into(), child.dtype().clone(), true))
}

fn signed(offsets: &[u64]) -> Result<OffsetsBuffer<i64>> {
    let o: Vec<i64> = offsets
        .iter()
        .map(|&o| {
            i64::try_from(o).map_err(|_| Error::InvalidInput(format!("offset {o} exceeds i64")))
        })
        .collect::<Result<_>>()?;
    OffsetsBuffer::try_from(o).map_err(Error::other)
}

fn to_array(column: Column) -> Result<Box<dyn Array>> {
    Ok(match column {
        Column::Bool(v) => Box::new(BooleanArray::from_slice(&v)),
        Column::I8(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::U8(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::I16(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::U16(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::I32(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::U32(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::I64(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::U64(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::F32(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::F64(v) => Box::new(PrimitiveArray::from_vec(v)),
        Column::String { offsets, bytes } => Box::new(
            Utf8Array::<i64>::try_new(
                ArrowDataType::LargeUtf8,
                signed(&offsets)?,
                bytes.into(),
                None,
            )
            .map_err(Error::other)?,
        ),
        Column::List { offsets, values } => {
            let child = to_array(*values)?;
            Box::new(
                ListArray::<i64>::try_new(
                    ArrowDataType::LargeList(item(child.as_ref())),
                    signed(&offsets)?,
                    child,
                    None,
                )
                .map_err(Error::other)?,
            )
        }
        Column::Array { len, values } => {
            let entries = values.len().checked_div(len).unwrap_or(0);
            let child = to_array(*values)?;
            Box::new(
                FixedSizeListArray::try_new(
                    ArrowDataType::FixedSizeList(item(child.as_ref()), len),
                    entries,
                    child,
                    None,
                )
                .map_err(Error::other)?,
            )
        }
        Column::Optional { valid, values } => {
            let (valid, values) = one_null_level(valid, *values);
            let full = spread(values, &valid)?;
            to_array(full)?.with_validity(Some(Bitmap::from_iter(valid)))
        }
        Column::Struct(members) => {
            let entries = members.first().map_or(0, |(_, c)| c.len());
            let mut fields = Vec::with_capacity(members.len());
            let mut arrays = Vec::with_capacity(members.len());
            for (name, c) in members {
                let a = to_array(c)?;
                fields.push(Field::new(name.into(), a.dtype().clone(), true));
                arrays.push(a);
            }
            Box::new(
                StructArray::try_new(ArrowDataType::Struct(fields), entries, arrays, None)
                    .map_err(Error::other)?,
            )
        }
    })
}

/// An optional of an optional (of …) as one level of nulls, the only one
/// Polars has: an entry is present when every level is, so `Some(None)` and
/// `None` both become null. Returns the combined validity and the innermost
/// values, one per present entry.
fn one_null_level(valid: Vec<bool>, values: Column) -> (Vec<bool>, Column) {
    match values {
        Column::Optional {
            valid: inner,
            values,
        } => {
            let mut next = inner.into_iter();
            let combined = valid
                .into_iter()
                .map(|v| v && next.next().unwrap_or(false))
                .collect();
            one_null_level(combined, *values)
        }
        other => (valid, other),
    }
}

/// Spread `dense` (one value per `true` in `valid`) out to one slot per
/// entry, with a default value where `valid` is `false`; the validity bitmap
/// then hides those.
fn spread(dense: Column, valid: &[bool]) -> Result<Column> {
    fn fill<T: Copy + Default>(dense: Vec<T>, valid: &[bool]) -> Vec<T> {
        let mut it = dense.into_iter();
        valid
            .iter()
            .map(|&v| {
                if v {
                    it.next().unwrap_or_default()
                } else {
                    T::default()
                }
            })
            .collect()
    }
    Ok(match dense {
        Column::Bool(v) => Column::Bool(fill(v, valid)),
        Column::I8(v) => Column::I8(fill(v, valid)),
        Column::U8(v) => Column::U8(fill(v, valid)),
        Column::I16(v) => Column::I16(fill(v, valid)),
        Column::U16(v) => Column::U16(fill(v, valid)),
        Column::I32(v) => Column::I32(fill(v, valid)),
        Column::U32(v) => Column::U32(fill(v, valid)),
        Column::I64(v) => Column::I64(fill(v, valid)),
        Column::U64(v) => Column::U64(fill(v, valid)),
        Column::F32(v) => Column::F32(fill(v, valid)),
        Column::F64(v) => Column::F64(fill(v, valid)),
        // An absent string, list or struct is an empty one: repeat the
        // offset (the dense children already hold only present entries).
        Column::String { offsets, bytes } => Column::String {
            offsets: repeat_absent(&offsets, valid),
            bytes,
        },
        Column::List { offsets, values } => Column::List {
            offsets: repeat_absent(&offsets, valid),
            values,
        },
        Column::Array { len, values } => {
            let inner: Vec<bool> = valid
                .iter()
                .flat_map(|&v| std::iter::repeat_n(v, len))
                .collect();
            Column::Array {
                len,
                values: Box::new(spread(*values, &inner)?),
            }
        }
        Column::Struct(members) => Column::Struct(
            members
                .into_iter()
                .map(|(n, c)| Ok((n, spread(c, valid)?)))
                .collect::<Result<_>>()?,
        ),
        // A present entry of an optional member keeps its own nulls.
        Column::Optional {
            valid: inner,
            values,
        } => {
            let mut it = inner.into_iter();
            let spread_valid: Vec<bool> = valid
                .iter()
                .map(|&v| v && it.next().unwrap_or(false))
                .collect();
            let (valid, values) = one_null_level(spread_valid, *values);
            Column::Optional {
                valid,
                values: Box::new(values),
            }
        }
    })
}

/// Offsets for one entry per `valid` slot, empty where it is `false`.
fn repeat_absent(dense: &[u64], valid: &[bool]) -> Vec<u64> {
    let mut out = Vec::with_capacity(valid.len() + 1);
    out.push(0);
    let mut k = 0;
    for &v in valid {
        if v {
            k += 1;
        }
        out.push(dense.get(k).copied().unwrap_or(0));
    }
    out
}

fn downcast<'a, T: 'static>(array: &'a dyn Array, name: &str) -> Result<&'a T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::Format(format!(
            "column {name:?}: unexpected Polars array for {:?}",
            array.dtype()
        ))
    })
}

fn from_array(array: &dyn Array, name: &str) -> Result<Column> {
    let full = from_dense(array, name)?;
    match array.validity() {
        Some(bitmap) if bitmap.unset_bits() > 0 => {
            let valid: Vec<bool> = bitmap.iter().collect();
            let present = full.filter(&valid);
            Ok(Column::Optional {
                valid,
                values: Box::new(present),
            })
        }
        _ => Ok(full),
    }
}

fn unsigned_offsets(offsets: &[i64]) -> (Vec<u64>, usize, usize) {
    let first = offsets.first().copied().unwrap_or(0);
    let last = offsets.last().copied().unwrap_or(0);
    let rebased = offsets
        .iter()
        .map(|&o| (o - first).cast_unsigned())
        .collect();
    (
        rebased,
        usize::saturating_from(first),
        usize::saturating_from(last),
    )
}

fn from_dense(array: &dyn Array, name: &str) -> Result<Column> {
    macro_rules! prim {
        ($t:ty, $variant:ident) => {
            Column::$variant(downcast::<PrimitiveArray<$t>>(array, name)?.values()[..].to_vec())
        };
    }
    Ok(match array.dtype() {
        ArrowDataType::Boolean => Column::Bool(
            downcast::<BooleanArray>(array, name)?
                .values()
                .iter()
                .collect(),
        ),
        ArrowDataType::Int8 => prim!(i8, I8),
        ArrowDataType::UInt8 => prim!(u8, U8),
        ArrowDataType::Int16 => prim!(i16, I16),
        ArrowDataType::UInt16 => prim!(u16, U16),
        ArrowDataType::Int32 => prim!(i32, I32),
        ArrowDataType::UInt32 => prim!(u32, U32),
        ArrowDataType::Int64 => prim!(i64, I64),
        ArrowDataType::UInt64 => prim!(u64, U64),
        ArrowDataType::Float32 => prim!(f32, F32),
        ArrowDataType::Float64 => prim!(f64, F64),
        ArrowDataType::LargeUtf8 => {
            let s = downcast::<Utf8Array<i64>>(array, name)?;
            let (offsets, first, last) = unsigned_offsets(s.offsets().buffer());
            Column::String {
                offsets,
                bytes: s.values()[first..last].to_vec(),
            }
        }
        ArrowDataType::Utf8View => downcast::<Utf8ViewArray>(array, name)?
            .values_iter()
            .collect::<Vec<&str>>()
            .into_column(),
        ArrowDataType::LargeList(_) => {
            let l = downcast::<ListArray<i64>>(array, name)?;
            let (offsets, first, last) = unsigned_offsets(l.offsets().buffer());
            Column::List {
                offsets,
                values: Box::new(from_array(
                    l.values().sliced(first, last - first).as_ref(),
                    name,
                )?),
            }
        }
        ArrowDataType::FixedSizeList(_, len) => {
            let l = downcast::<FixedSizeListArray>(array, name)?;
            Column::Array {
                len: *len,
                values: Box::new(from_array(l.values().as_ref(), name)?),
            }
        }
        ArrowDataType::Struct(fields) => {
            let s = downcast::<StructArray>(array, name)?;
            Column::Struct(
                fields
                    .iter()
                    .zip(s.values())
                    .map(|(f, c)| Ok((f.name.to_string(), from_array(c.as_ref(), name)?)))
                    .collect::<Result<_>>()?,
            )
        }
        other => {
            return Err(Error::Unsupported(format!(
                "column {name:?}: Polars type {other:?} has no batch column"
            )));
        }
    })
}
