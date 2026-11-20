//! [`Batch`] ↔ Arrow [`RecordBatch`] (the `arrow` feature, arrow-rs).
//!
//! | Column | Arrow |
//! |---|---|
//! | `bool`, `i8` … `u64`, `f32`, `f64` | `Boolean`, `Int8` … `UInt64`, `Float32`, `Float64` |
//! | `String` | `LargeUtf8` (`Utf8` and `Utf8View` also read) |
//! | `List` | `LargeList` (`List` also read) |
//! | `Array` | `FixedSizeList` |
//! | `Optional` | the value's type, with nulls |
//! | `Struct` | `Struct` |
//!
//! Going to Arrow moves the value buffers (no copy) when the batch is given
//! by value; offsets are converted to Arrow's signed form. Coming back, any
//! array with nulls becomes an `Optional` over its non-null values.

use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, LargeListArray, LargeStringArray,
    PrimitiveArray, RecordBatch, RecordBatchOptions, StructArray, UInt64Array,
};
use arrow_buffer::{Buffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{DataType, Field, Fields, Schema};
use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result};

use crate::batch::Batch;
use crate::column::{Column, IntoColumn};

impl Batch {
    /// This batch as an Arrow [`RecordBatch`], copying the values.
    ///
    /// # Errors
    ///
    /// A column Arrow cannot represent (offsets past `i64::MAX`, say).
    pub fn to_arrow(&self) -> Result<RecordBatch> {
        self.clone().into_arrow()
    }

    /// This batch as an Arrow [`RecordBatch`], moving the value buffers into
    /// Arrow without copying them.
    ///
    /// # Errors
    ///
    /// As [`to_arrow`](Self::to_arrow).
    pub fn into_arrow(self) -> Result<RecordBatch> {
        let entries = self.len();
        let mut fields = Vec::new();
        let mut arrays = Vec::new();
        for (name, column) in self.into_columns() {
            let array = to_array(column)?;
            fields.push(Field::new(
                name,
                array.data_type().clone(),
                array.null_count() > 0,
            ));
            arrays.push(array);
        }
        let options = RecordBatchOptions::new().with_row_count(Some(entries));
        RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), arrays, &options)
            .map_err(Error::other)
    }

    /// An Arrow [`RecordBatch`] as a batch, copying the values. Arrays with
    /// nulls become [`Column::Optional`]s.
    ///
    /// # Errors
    ///
    /// An Arrow type with no column counterpart (a dictionary, a timestamp,
    /// a union, …).
    pub fn from_arrow(batch: &RecordBatch) -> Result<Batch> {
        let mut out = Batch::new();
        for (field, array) in batch.schema().fields().iter().zip(batch.columns()) {
            out.insert(
                field.name().clone(),
                from_array(array.as_ref(), field.name())?,
            )?;
        }
        Ok(out)
    }
}

fn signed(offsets: &[u64]) -> Result<OffsetBuffer<i64>> {
    let o: Vec<i64> = offsets
        .iter()
        .map(|&o| {
            i64::try_from(o).map_err(|_| Error::InvalidInput(format!("offset {o} exceeds i64")))
        })
        .collect::<Result<_>>()?;
    Ok(OffsetBuffer::new(ScalarBuffer::from(o)))
}

fn item_field(child: &ArrayRef) -> Arc<Field> {
    Arc::new(Field::new(
        "item",
        child.data_type().clone(),
        child.null_count() > 0,
    ))
}

fn prim<T: arrow_array::ArrowPrimitiveType>(v: Vec<T::Native>) -> ArrayRef {
    Arc::new(PrimitiveArray::<T>::new(ScalarBuffer::from(v), None))
}

fn to_array(column: Column) -> Result<ArrayRef> {
    Ok(match column {
        Column::Bool(v) => Arc::new(BooleanArray::from(v)),
        Column::I8(v) => prim::<Int8Type>(v),
        Column::U8(v) => prim::<UInt8Type>(v),
        Column::I16(v) => prim::<Int16Type>(v),
        Column::U16(v) => prim::<UInt16Type>(v),
        Column::I32(v) => prim::<Int32Type>(v),
        Column::U32(v) => prim::<UInt32Type>(v),
        Column::I64(v) => prim::<Int64Type>(v),
        Column::U64(v) => prim::<UInt64Type>(v),
        Column::F32(v) => prim::<Float32Type>(v),
        Column::F64(v) => prim::<Float64Type>(v),
        Column::String { offsets, bytes } => Arc::new(
            LargeStringArray::try_new(signed(&offsets)?, Buffer::from_vec(bytes), None)
                .map_err(Error::other)?,
        ),
        Column::List { offsets, values } => {
            let child = to_array(*values)?;
            Arc::new(
                LargeListArray::try_new(item_field(&child), signed(&offsets)?, child, None)
                    .map_err(Error::other)?,
            )
        }
        Column::Array { len, values } => {
            let child = to_array(*values)?;
            let size = i32::try_from(len)
                .map_err(|_| Error::InvalidInput(format!("array width {len} exceeds i32")))?;
            Arc::new(
                FixedSizeListArray::try_new(item_field(&child), size, child, None)
                    .map_err(Error::other)?,
            )
        }
        Column::Optional { valid, values } => {
            // Spread the dense values out to one slot per entry, null where
            // the entry has none.
            let dense = to_array(*values)?;
            let mut next = 0u64;
            let indices: UInt64Array = valid
                .iter()
                .map(|&v| {
                    v.then(|| {
                        next += 1;
                        next - 1
                    })
                })
                .collect();
            arrow_select::take::take(dense.as_ref(), &indices, None).map_err(Error::other)?
        }
        Column::Struct(members) => {
            let mut fields = Vec::with_capacity(members.len());
            let mut arrays = Vec::with_capacity(members.len());
            for (name, c) in members {
                let a = to_array(c)?;
                fields.push(Field::new(name, a.data_type().clone(), a.null_count() > 0));
                arrays.push(a);
            }
            Arc::new(
                StructArray::try_new(Fields::from(fields), arrays, None).map_err(Error::other)?,
            )
        }
    })
}

fn unsigned_offsets<O: arrow_array::OffsetSizeTrait>(offsets: &[O]) -> (Vec<u64>, usize, usize) {
    let first = offsets.first().map_or(0, |o| o.as_usize());
    let last = offsets.last().map_or(0, |o| o.as_usize());
    let rebased = offsets
        .iter()
        .map(|o| (o.as_usize() - first) as u64)
        .collect();
    (rebased, first, last)
}

fn from_array(array: &dyn Array, name: &str) -> Result<Column> {
    if array.null_count() == 0 {
        return from_dense(array, name);
    }
    let valid: Vec<bool> = (0..array.len()).map(|i| array.is_valid(i)).collect();
    let mask = BooleanArray::from(valid.clone());
    let present = arrow_select::filter::filter(array, &mask).map_err(Error::other)?;
    Ok(Column::Optional {
        valid,
        values: Box::new(from_dense(present.as_ref(), name)?),
    })
}

fn from_dense(array: &dyn Array, name: &str) -> Result<Column> {
    Ok(match array.data_type() {
        DataType::Boolean => Column::Bool(array.as_boolean().values().iter().collect()),
        DataType::Int8 => Column::I8(array.as_primitive::<Int8Type>().values().to_vec()),
        DataType::UInt8 => Column::U8(array.as_primitive::<UInt8Type>().values().to_vec()),
        DataType::Int16 => Column::I16(array.as_primitive::<Int16Type>().values().to_vec()),
        DataType::UInt16 => Column::U16(array.as_primitive::<UInt16Type>().values().to_vec()),
        DataType::Int32 => Column::I32(array.as_primitive::<Int32Type>().values().to_vec()),
        DataType::UInt32 => Column::U32(array.as_primitive::<UInt32Type>().values().to_vec()),
        DataType::Int64 => Column::I64(array.as_primitive::<Int64Type>().values().to_vec()),
        DataType::UInt64 => Column::U64(array.as_primitive::<UInt64Type>().values().to_vec()),
        DataType::Float32 => Column::F32(array.as_primitive::<Float32Type>().values().to_vec()),
        DataType::Float64 => Column::F64(array.as_primitive::<Float64Type>().values().to_vec()),
        DataType::Utf8 => {
            let s = array.as_string::<i32>();
            let (offsets, first, last) = unsigned_offsets(s.value_offsets());
            Column::String {
                offsets,
                bytes: s.value_data()[first..last].to_vec(),
            }
        }
        DataType::LargeUtf8 => {
            let s = array.as_string::<i64>();
            let (offsets, first, last) = unsigned_offsets(s.value_offsets());
            Column::String {
                offsets,
                bytes: s.value_data()[first..last].to_vec(),
            }
        }
        DataType::Utf8View => array
            .as_string_view()
            .iter()
            .map(|s| s.unwrap_or_default())
            .collect::<Vec<&str>>()
            .into_column(),
        DataType::List(_) => {
            let l = array.as_list::<i32>();
            let (offsets, first, last) = unsigned_offsets(l.value_offsets());
            Column::List {
                offsets,
                values: Box::new(from_array(
                    l.values().slice(first, last - first).as_ref(),
                    name,
                )?),
            }
        }
        DataType::LargeList(_) => {
            let l = array.as_list::<i64>();
            let (offsets, first, last) = unsigned_offsets(l.value_offsets());
            Column::List {
                offsets,
                values: Box::new(from_array(
                    l.values().slice(first, last - first).as_ref(),
                    name,
                )?),
            }
        }
        DataType::FixedSizeList(_, size) => {
            let l = array.as_fixed_size_list();
            let len = usize::saturating_from(*size);
            let start = l.value_offset_at(0);
            Column::Array {
                len,
                values: Box::new(from_array(
                    l.values().slice(start, l.len() * len).as_ref(),
                    name,
                )?),
            }
        }
        DataType::Struct(fields) => {
            let s = array.as_struct();
            let members = fields
                .iter()
                .zip(s.columns())
                .map(|(f, c)| Ok((f.name().clone(), from_array(c.as_ref(), name)?)))
                .collect::<Result<_>>()?;
            Column::Struct(members)
        }
        other => {
            return Err(Error::Unsupported(format!(
                "column {name:?}: Arrow type {other} has no batch column"
            )));
        }
    })
}
