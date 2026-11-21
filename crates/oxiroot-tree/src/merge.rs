//! The read → write round trip of one branch: [`BranchRead::to_branch`]
//! rebuilds a branch's kind (scalar, fixed array, jagged, `std::vector`, or
//! string) so that it writes back the way it was read. Merging whole trees is
//! the `oxiroot` facade's `hadd`, which goes through the `Dataset` reader and
//! writer.

use oxiroot_io_core::{Error, Result};

use crate::reader::{BranchMetaLite, BranchRead};
use crate::value::BranchValues;
use crate::writer::Branch;

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
