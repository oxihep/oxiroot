//! Checking fields before anything is written: that each column holds
//! together, that it has the shape of the type it is stored as, and that the
//! fields agree on the number of entries.

use oxiroot_io_core::{Column, DataType, Error, Result};

use super::fields::Field;

/// Reject fields the writer would lay out inconsistently: a column whose
/// parts disagree (record members of different lengths, offsets that do not
/// end at their values), a storage type of another shape than the data, and
/// fields whose entry counts differ (a cluster has one entry count).
pub(super) fn check_fields(fields: &[Field]) -> Result<()> {
    let mut expected = None;
    for f in fields {
        let what = format!("field {:?}", f.name);
        f.data.validate(&f.name)?;
        check_type(&what, &f.data, &f.data_type)?;
        if let Some(n) = entry_count(&f.data) {
            match expected {
                None => expected = Some(n),
                Some(e) if e != n => {
                    return Err(Error::LengthMismatch {
                        what: format!("{what} entries"),
                        expected: e,
                        found: n,
                    });
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// The number of entries `col` holds, or `None` for a zero-size array, whose
/// data does not tell (it holds no values for any number of entries).
pub(super) fn entry_count(col: &Column) -> Option<usize> {
    match col {
        Column::Array { len: 0, .. } => None,
        Column::Struct(members) => members.iter().find_map(|(_, c)| entry_count(c)),
        _ => Some(col.len()),
    }
}

/// Check that `data_type` is a way to store `data`, and one this writer has.
fn check_type(what: &str, data: &Column, data_type: &DataType) -> Result<()> {
    if data_type.storage() != data.data_type() {
        return Err(Error::InvalidInput(format!(
            "{what} holds {}, which cannot be stored as {data_type}",
            data.data_type()
        )));
    }
    match unwritable(data_type) {
        Some(why) => Err(Error::Unsupported(format!(
            "{what} is stored as {data_type}: {why}"
        ))),
        None => Ok(()),
    }
}

/// Why this writer cannot store `data_type` in an `RNTuple`, if it cannot.
fn unwritable(data_type: &DataType) -> Option<&'static str> {
    match data_type {
        DataType::CArray(_) => Some("a counted array is a TTree layout"),
        DataType::Truncated { bits } if !(10..=31).contains(bits) => {
            Some("a truncated float keeps 10 to 31 bits")
        }
        DataType::Quantized { bits, .. } if !(1..=32).contains(bits) => {
            Some("a quantized float takes 1 to 32 bits")
        }
        DataType::Quantized { min, max, .. } if !(min.is_finite() && max.is_finite()) => {
            Some("a quantized float needs a finite range")
        }
        DataType::Quantized { min, max, .. } if min > max => {
            Some("a quantized float's range is empty")
        }
        DataType::List(item)
        | DataType::Set(item)
        | DataType::Array(_, item)
        | DataType::Optional(item)
        | DataType::UniquePtr(item)
        | DataType::Atomic(item) => unwritable(item),
        DataType::Map(key, value) => unwritable(key).or_else(|| unwritable(value)),
        DataType::Struct(members) => members.iter().find_map(|(_, t)| unwritable(t)),
        DataType::Variant(alternatives) => alternatives.iter().find_map(unwritable),
        _ => None,
    }
}
