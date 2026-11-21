//! Appending `RNTuples` — the `RNTuple` half of a `hadd`-style file merge, and
//! the read → write round trip of one field.
//!
//! [`append_ntuples`] streams several [`NtupleReader`]s' entries into an
//! [`NtupleWriter`], one cluster per input; the `oxiroot` facade's file merger
//! uses it for `RNTuple` keys. An `RNTuple` field's type is fully determined by its
//! [`FieldValues`] variant, so the mapping back to a [`Field`] is direct;
//! [`FieldRead::to_field`] does it for one field.

use std::io::{Seek, Write};

use oxiroot_io_core::{Error, Result};

use crate::field::FieldValues;
use crate::reader::{FieldRead, NtupleReader};
use crate::writer::{Field, NtupleWriter};

impl FieldRead<'_, '_> {
    /// Read the field (the [`range`](Self::range) set, if any) as a writable
    /// [`Field`] of the same name and type, ready for an [`NtupleWriter`]: the
    /// round trip of a skim. Types the writer cannot encode (records, nested
    /// collections, variants, optionals, `std::vector` of unsigned integers)
    /// are an [`Error::Unsupported`].
    pub fn to_field(self) -> Result<Field> {
        let name = self.name();
        build_field(name, self.values()?)
    }
}

/// Append every entry of `inputs` to `writer`, one input at a time: each input's
/// fields are read, written as one cluster, and dropped before the next input is
/// read. Returns the number of entries appended.
///
/// Every input must have the first input's fields, of the same types; the first
/// cluster the writer receives fixes the schema.
///
/// # Errors
///
/// No inputs, an input missing a field or holding a type the writer cannot
/// encode, or any error from [`NtupleWriter::write_batch`].
pub fn append_ntuples<W: Write + Seek>(
    writer: &mut NtupleWriter<W>,
    inputs: &[&NtupleReader<'_>],
) -> Result<u64> {
    let &first = inputs
        .first()
        .ok_or_else(|| Error::InvalidInput("append_ntuples: no input RNTuples".into()))?;
    let field_names = first.field_names();
    let mut appended = 0;
    for (i, &ntuple) in inputs.iter().enumerate() {
        let mut batch = Vec::with_capacity(field_names.len());
        for &field in &field_names {
            let values = ntuple.field(field).values().map_err(|e| {
                e.context(format_args!("append_ntuples: input #{i} field {field:?}"))
            })?;
            batch.push(build_field(field, values)?);
        }
        writer.write_batch(&batch)?;
        appended += ntuple.entries();
    }
    Ok(appended)
}

/// Rebuild a writable [`Field`] from a field's concatenated values.
fn build_field(name: &str, values: FieldValues) -> Result<Field> {
    use FieldValues::*;
    let reject = |what: &str| -> Result<Field> {
        Err(Error::Unsupported(format!(
            "field {name:?} is a {what}, which oxiroot cannot write yet",
        )))
    };
    match values {
        Bool(v) => Ok(Field::bool(name, v)),
        I8(v) => Ok(Field::i8(name, v)),
        U8(v) => Ok(Field::u8(name, v)),
        I16(v) => Ok(Field::i16(name, v)),
        U16(v) => Ok(Field::u16(name, v)),
        I32(v) => Ok(Field::i32(name, v)),
        I64(v) => Ok(Field::i64(name, v)),
        U32(v) => Ok(Field::u32(name, v)),
        U64(v) => Ok(Field::u64(name, v)),
        F32(v) => Ok(Field::f32(name, v)),
        F64(v) => Ok(Field::f64(name, v)),
        Str(v) => Ok(Field::string(name, v)),
        VecBool(v) => Ok(Field::vector_bool(name, v)),
        VecI8(v) => Ok(Field::vector_i8(name, v)),
        VecU8(v) => Ok(Field::vector_u8(name, v)),
        VecI16(v) => Ok(Field::vector_i16(name, v)),
        VecU16(v) => Ok(Field::vector_u16(name, v)),
        VecI32(v) => Ok(Field::vector_i32(name, v)),
        VecI64(v) => Ok(Field::vector_i64(name, v)),
        VecF32(v) => Ok(Field::vector_f32(name, v)),
        VecF64(v) => Ok(Field::vector_f64(name, v)),
        VecStr(v) => Ok(Field::vector_string(name, v)),
        // The writer has no column encoding for these vector element types.
        VecU32(_) => reject("std::vector<uint32_t> field"),
        VecU64(_) => reject("std::vector<uint64_t> field"),
        Record(_) => reject("record / struct field"),
        Nested { .. } => reject("nested collection field (vector<vector> or vector<struct>)"),
        Variant { .. } => reject("std::variant field"),
        Opt { .. } => reject("optional / unique_ptr field"),
    }
}
