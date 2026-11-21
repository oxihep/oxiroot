//! From an `RNTuple` field's physical columns to its [`Column`] of values.
//!
//! A scalar leaf maps straight from a column, a `std::string` combines an
//! index column with a char column, and a `std::vector<T>` combines an index
//! column with the element field's values. The result is flat, as the pages
//! are: no value is copied per entry.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Column, Error, Result};

use crate::page::ColumnValues;

/// Map a scalar leaf column to field values.
pub(crate) fn scalar(values: ColumnValues) -> Result<Column> {
    Ok(match values {
        ColumnValues::Bits(v) => Column::Bool(v),
        ColumnValues::I8(v) => Column::I8(v),
        ColumnValues::U8(v) => Column::U8(v),
        ColumnValues::I16(v) => Column::I16(v),
        ColumnValues::U16(v) => Column::U16(v),
        ColumnValues::I32(v) => Column::I32(v),
        ColumnValues::I64(v) => Column::I64(v),
        ColumnValues::U32(v) => Column::U32(v),
        ColumnValues::U64(v) => Column::U64(v),
        ColumnValues::F32(v) => Column::F32(v),
        ColumnValues::F64(v) => Column::F64(v),
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

/// The offsets a [`Column`] carries, from on-disk ones: the file holds one
/// cumulative end per element, a column those after a leading `0`. They must
/// not decrease and must end at `len`, the number of values they cut.
fn offsets_over(what: &str, ends: Vec<u64>, len: usize) -> Result<Vec<u64>> {
    let mut offsets = Vec::with_capacity(ends.len() + 1);
    offsets.push(0);
    offsets.extend(ends);
    let ordered = offsets.windows(2).all(|w| w[0] <= w[1]);
    if !ordered || offsets.last().copied() != Some(len as u64) {
        return Err(Error::Format(format!(
            "{what} offsets do not partition its {len} values"
        )));
    }
    Ok(offsets)
}

/// `std::string` values from cumulative end offsets and the char bytes.
pub(crate) fn strings(ends: Vec<u64>, bytes: Vec<u8>) -> Result<Column> {
    let text = String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
    let offsets = offsets_over("string", ends, text.len())?;
    if offsets
        .iter()
        .any(|&o| !text.is_char_boundary(usize::saturating_from(o)))
    {
        return Err(Error::InvalidUtf8);
    }
    Ok(Column::String { offsets, text })
}

/// A collection: its child `values`, cut into entries at the cumulative `ends`.
pub(crate) fn collect(ends: Vec<u64>, values: Column) -> Result<Column> {
    let offsets = offsets_over("collection", ends, values.len())?;
    Ok(Column::List {
        offsets,
        values: Box::new(values),
    })
}

/// An optional / `unique_ptr` field. On disk it is a collection of 0 or 1
/// element per entry: an entry holds a value where the cumulative count
/// `ends` grows, and `values` are the values held, in entry order.
pub(crate) fn optional(ends: &[u64], values: Column) -> Result<Column> {
    let mut valid = Vec::with_capacity(ends.len());
    let mut prev = 0u64;
    for &end in ends {
        valid.push(end > prev);
        prev = end;
    }
    let present = valid.iter().filter(|&&v| v).count();
    if present != values.len() {
        return Err(Error::Format(format!(
            "optional field marks {present} entries as holding a value, of {} values",
            values.len()
        )));
    }
    Ok(Column::Optional {
        valid,
        values: Box::new(values),
    })
}

/// A fixed-size array field (`std::array<T, n>` or `std::bitset<n>`): its
/// flattened `values`, `n` per entry.
pub(crate) fn chunk(values: Column, n: usize) -> Result<Column> {
    let total = values.len();
    if n == 0 {
        return Err(Error::Format("array field has zero repetition".into()));
    }
    if !total.is_multiple_of(n) {
        return Err(Error::Format(format!(
            "array field element count {total} is not a multiple of {n}"
        )));
    }
    Ok(Column::Array {
        len: n,
        values: Box::new(values),
    })
}
