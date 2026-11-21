//! Appending trees — the `TTree` half of a `hadd`-style file merge, and the
//! read → write round trip of one branch.
//!
//! [`append_trees`] streams several [`TreeReader`]s' entries into a
//! [`TreeWriter`], holding one input's entries at a time, rebuilding each
//! branch's kind (scalar, fixed array, jagged, `std::vector`, or string) so the
//! result writes back the way it was read. The `oxiroot` facade's file merger
//! uses it for tree keys. [`BranchRead::to_branch`] does the same for one
//! branch.

use std::io::{Seek, Write};

use oxiroot_io_core::{Error, Result};

use crate::reader::{BranchMetaLite, BranchRead, TreeReader};
use crate::value::BranchValues;
use crate::writer::{Branch, TreeWriter};

impl BranchRead<'_, '_> {
    /// Read the branch (the [`range`](Self::range) set, if any) as a writable
    /// [`Branch`] of the same name and layout — scalar, fixed array, jagged
    /// array, `std::vector` or string — ready for a
    /// [`TreeWriter`](crate::raw::TreeWriter): the round trip of a skim. Layouts
    /// this crate reads but cannot write (split objects, leaflists) are an
    /// [`Error::Unsupported`].
    ///
    /// # Errors
    ///
    /// Any error reading the branch, and [`Error::Unsupported`] as described above.
    pub fn to_branch(self) -> Result<Branch> {
        let name = self.name();
        let meta = self
            .tree()
            .branch_meta(name)
            .ok_or_else(|| Error::NotFound {
                what: "branch",
                name: name.to_string(),
            })?;
        build_branch(name, &meta, self.values()?)
    }
}

/// Append every entry of `inputs` to `writer`, one input at a time: each input's
/// branches are read, written as one batch (one basket per branch), and dropped
/// before the next input is read. Returns the number of entries appended.
///
/// Every input must have the first input's branches, of the same kinds; the
/// first batch the writer receives fixes the schema.
///
/// # Errors
///
/// No inputs; an input missing a branch or holding one this crate cannot read
/// or write back; any error from [`TreeWriter::write_batch`].
pub fn append_trees<W: Write + Seek>(
    writer: &mut TreeWriter<W>,
    inputs: &[&TreeReader<'_>],
) -> Result<u64> {
    let &first = inputs
        .first()
        .ok_or_else(|| Error::InvalidInput("append_trees: no input trees".into()))?;
    let names = first.branch_names();
    let mut appended = 0;
    for (i, &tree) in inputs.iter().enumerate() {
        if let Some((name, reason)) = tree.unsupported_branches().first() {
            return Err(Error::Unsupported(format!(
                "append_trees: input #{i} ({:?}) has an unreadable branch {name:?} ({reason}); \
                 cannot merge it without losing data",
                tree.name(),
            )));
        }
        let mut batch = Vec::with_capacity(names.len());
        for &name in &names {
            let meta = tree.branch_meta(name).ok_or_else(|| Error::SchemaChanged {
                detail: format!(
                    "append_trees: input #{i} ({:?}) is missing branch {name:?} \
                     present in the first tree",
                    tree.name(),
                ),
            })?;
            batch.push(build_branch(name, &meta, tree.branch(name).values()?)?);
        }
        writer.write_batch(&batch)?;
        appended += tree.entries();
    }
    Ok(appended)
}

/// Rebuild a writable [`Branch`] of the correct kind from a branch's read
/// metadata and its concatenated values.
fn build_branch(name: &str, meta: &BranchMetaLite, values: BranchValues) -> Result<Branch> {
    // Layouts this crate can read but not yet write back.
    let reject = |what: &str| -> Result<Branch> {
        Err(Error::Unsupported(format!(
            "branch {name:?} is a {what}, which oxiroot cannot write yet",
        )))
    };
    if meta.has_object_member {
        return reject("split-object member column");
    }
    if meta.has_leaflist {
        return reject("multi-leaf (leaflist) branch");
    }
    if meta.has_nested {
        return reject("std::vector<std::vector<T>> branch");
    }
    if meta.dims_len > 1 {
        return reject("multidimensional fixed array");
    }

    // Which flavour a `Vec*` value writes back as: a single fixed dimension
    // (`x[N]`) → fixed array; else a `std::vector<T>` (marked by a per-entry
    // streamer header) → STL vector; else a jagged `x[n]`.
    enum Flavor {
        Fixed,
        Stl,
        Jagged,
    }
    let flavor = if meta.dims_len == 1 {
        Flavor::Fixed
    } else if meta.elem_header > 0 {
        Flavor::Stl
    } else {
        Flavor::Jagged
    };

    use BranchValues::*;
    // A `Vec*` variant with all three flavours available.
    macro_rules! vec3 {
        ($v:expr, $fixed:ident, $jagged:ident, $stl:ident) => {
            match flavor {
                Flavor::Fixed => Ok(Branch::$fixed(name, $v)),
                Flavor::Jagged => Ok(Branch::$jagged(name, $v)),
                Flavor::Stl => Ok(Branch::$stl(name, $v)),
            }
        };
    }

    match values {
        Bool(v) => Ok(Branch::bool(name, v)),
        I8(v) => Ok(Branch::i8(name, v)),
        U8(v) => Ok(Branch::u8(name, v)),
        I16(v) => Ok(Branch::i16(name, v)),
        U16(v) => Ok(Branch::u16(name, v)),
        I32(v) => Ok(Branch::i32(name, v)),
        U32(v) => Ok(Branch::u32(name, v)),
        I64(v) => Ok(Branch::i64(name, v)),
        U64(v) => Ok(Branch::u64(name, v)),
        F32(v) => Ok(Branch::f32(name, v)),
        F64(v) => Ok(Branch::f64(name, v)),
        Str(v) => Ok(Branch::string(name, v)),
        // `std::vector<bool>` has no writer constructor (bit-packed); the fixed
        // and jagged flavours do.
        VecBool(v) => match flavor {
            Flavor::Fixed => Ok(Branch::array_bool(name, v)),
            Flavor::Jagged => Ok(Branch::jagged_bool(name, v)),
            Flavor::Stl => reject("std::vector<bool> branch"),
        },
        VecI8(v) => vec3!(v, array_i8, jagged_i8, vector_i8),
        VecU8(v) => vec3!(v, array_u8, jagged_u8, vector_u8),
        VecI16(v) => vec3!(v, array_i16, jagged_i16, vector_i16),
        VecU16(v) => vec3!(v, array_u16, jagged_u16, vector_u16),
        VecI32(v) => vec3!(v, array_i32, jagged_i32, vector_i32),
        VecU32(v) => vec3!(v, array_u32, jagged_u32, vector_u32),
        VecI64(v) => vec3!(v, array_i64, jagged_i64, vector_i64),
        VecU64(v) => vec3!(v, array_u64, jagged_u64, vector_u64),
        VecF32(v) => vec3!(v, array_f32, jagged_f32, vector_f32),
        VecF64(v) => vec3!(v, array_f64, jagged_f64, vector_f64),
        VecStr(_) => reject("std::vector<std::string> branch"),
        Nested { .. } => reject("std::vector<std::vector<T>> branch"),
    }
}
