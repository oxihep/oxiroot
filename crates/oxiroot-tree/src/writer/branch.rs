//! The branches a caller builds: [`Branch`] and its constructors, and the
//! members of a split `std::vector<MyStruct>` branch ([`SplitMember`]).

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::streamer_gen::{ClassDef, basic};
use oxiroot_io_core::{Error, Result};

use crate::value::BranchValues;

/// What a leaf's header records about every value of its column. A streaming
/// writer tracks it across batches instead of keeping the values; the
/// one-batch writer leaves it unset and reads the values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct LeafMeta {
    /// `fMaximum`: the largest value of a count leaf; for a string leaf, the
    /// longest length + 1.
    pub(super) max: i64,
    /// `fLen` of a string leaf: the longest length + 1.
    pub(super) str_len: i32,
    /// The largest element count of a split `std::vector<Struct>` branch.
    pub(super) split_max: i32,
}

/// One named branch to write. Use the typed constructors: [`Branch::i32`] … for
/// scalars, [`Branch::array_f64`] … for fixed-size arrays (`x[N]`),
/// [`Branch::jagged_f64`] … for C-style variable-length arrays (`x[n]`),
/// [`Branch::vector_f64`] … for `std::vector<T>`, [`Branch::string`] for
/// strings — or [`BranchRead::to_branch`](crate::raw::BranchRead::to_branch) to
/// write back a branch as it was read.
pub struct Branch {
    /// Branch (and leaf) name.
    pub(crate) name: String,
    /// Branch values (a [`BranchValues`] variant — scalar, array, or string).
    /// Private: a branch is built only through the typed constructors
    /// ([`Branch::i32`], [`Branch::jagged_f64`], …) so the payload can never
    /// drift from the `kind` that decides how it is serialized.
    pub(super) values: BranchValues,
    /// How the branch is serialized; one private enum instead of three
    /// independent flags, so an invalid combination is unrepresentable.
    pub(super) kind: BranchKind,
    /// What the leaf's header says of the values, when the writer knows it
    /// without holding them all: see [`LeafMeta`].
    pub(super) meta: Option<LeafMeta>,
}

/// How a [`Branch`] is written, replacing the old `jagged`/`stl_vector`/`split`
/// flag triple. The constructors pick the variant; the payload type is the
/// `Branch`'s `values`.
pub(super) enum BranchKind {
    /// A scalar or string (`TLeafC`) branch.
    Plain,
    /// A fixed-size array `x[N]` of this width. Kept apart from the rows so a
    /// branch with none still knows `N`.
    Array(usize),
    /// The `n<name>` count branch of a jagged array: a plain `Int_t` scalar
    /// whose leaf, like ROOT's, is a range (`fIsRange`) whose `fMaximum` is the
    /// largest count.
    Count,
    /// A variable-length (jagged) array: rows may differ in length, written
    /// with a paired `n<name>` count branch and an `fLeafCount` reference.
    Jagged,
    /// A `std::vector<T>` branch, written as a `TBranchElement` (each basket
    /// entry carries a 10-byte streamer header instead of a count branch).
    StlVector,
    /// A split `std::vector<MyStruct>` branch — a parent `TBranchElement` whose
    /// per-member data lives in sub-branches; the `Branch`'s `values` is unused.
    Split(SplitSpec),
}

/// A split `std::vector<MyStruct>` branch: the struct's class name and one
/// member per sub-branch.
pub(super) struct SplitSpec {
    pub(super) class_name: String,
    pub(super) members: Vec<SplitMember>,
}

impl Branch {
    /// The branch's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn jagged(&self) -> bool {
        matches!(self.kind, BranchKind::Jagged)
    }
    pub(super) fn stl_vector(&self) -> bool {
        matches!(self.kind, BranchKind::StlVector)
    }
    pub(super) fn split(&self) -> Option<&SplitSpec> {
        match &self.kind {
            BranchKind::Split(spec) => Some(spec),
            _ => None,
        }
    }
    /// The kind a chunk/copy of this branch takes — the same kind unless it is a
    /// split branch (which is never chunked), in which case [`BranchKind::Plain`].
    pub(super) fn chunk_kind(&self) -> BranchKind {
        match self.kind {
            BranchKind::Array(n) => BranchKind::Array(n),
            BranchKind::Jagged => BranchKind::Jagged,
            BranchKind::StlVector => BranchKind::StlVector,
            BranchKind::Count => BranchKind::Count,
            _ => BranchKind::Plain,
        }
    }
}

/// One member of a split `std::vector<MyStruct>` branch: its name and the
/// per-entry jagged values (`Vec<Vec<T>>` via a `VecXxx` [`BranchValues`]).
pub struct SplitMember {
    pub(super) name: String,
    pub(super) values: BranchValues,
}

macro_rules! split_member_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl SplitMember {
            $(
                #[doc = concat!("A `", stringify!($elem), "` member of a split struct branch.")]
                pub fn $method(name: impl Into<String>, values: Vec<Vec<$elem>>) -> SplitMember {
                    SplitMember { name: name.into(), values: BranchValues::$variant(values) }
                }
            )*
        }
    };
}
split_member_ctors! {
    bool => VecBool(bool), i8 => VecI8(i8), u8 => VecU8(u8), i16 => VecI16(i16), u16 => VecU16(u16),
    i32 => VecI32(i32), u32 => VecU32(u32), i64 => VecI64(i64), u64 => VecU64(u64),
    f32 => VecF32(f32), f64 => VecF64(f64),
}

macro_rules! branch_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl Branch {
            $(
                #[doc = concat!("A branch holding `", stringify!($variant), "` values.")]
                pub fn $method(name: impl Into<String>, values: Vec<$elem>) -> Branch {
                    Branch { name: name.into(), values: BranchValues::$variant(values), kind: BranchKind::Plain, meta: None }
                }
            )*
        }
    };
}
branch_ctors! {
    bool => Bool(bool), i8 => I8(i8), u8 => U8(u8), i16 => I16(i16), u16 => U16(u16),
    i32 => I32(i32), u32 => U32(u32), i64 => I64(i64), u64 => U64(u64),
    f32 => F32(f32), f64 => F64(f64),
}

/// Generate `Branch::array_<name>` shortcuts for fixed-size array branches (each
/// inner vector must have the same length `N`, written as `x[N]`).
macro_rules! vec_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl Branch {
            $(
                #[doc = concat!("A fixed-size array branch holding `", stringify!($variant), "` rows, ")]
                #[doc = "all as long as the first (give a branch with no rows its width with [`Branch::with_width`])."]
                pub fn $method(name: impl Into<String>, values: Vec<Vec<$elem>>) -> Branch {
                    let width = values.first().map_or(0, Vec::len);
                    Branch { name: name.into(), values: BranchValues::$variant(values), kind: BranchKind::Array(width), meta: None }
                }
            )*
        }
    };
}
vec_ctors! {
    array_bool => VecBool(bool), array_i8 => VecI8(i8), array_u8 => VecU8(u8),
    array_i16 => VecI16(i16), array_u16 => VecU16(u16), array_i32 => VecI32(i32),
    array_u32 => VecU32(u32), array_i64 => VecI64(i64), array_u64 => VecU64(u64),
    array_f32 => VecF32(f32), array_f64 => VecF64(f64),
}

/// Generate `Branch::jagged_<name>` shortcuts for variable-length array branches
/// (rows may differ in length; written as `y[n<name>]` with a paired count
/// branch). Same backing variants as the fixed-array constructors.
macro_rules! jagged_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl Branch {
            $(
                #[doc = concat!("A variable-length array branch holding `", stringify!($variant), "` rows.")]
                pub fn $method(name: impl Into<String>, values: Vec<Vec<$elem>>) -> Branch {
                    Branch { name: name.into(), values: BranchValues::$variant(values), kind: BranchKind::Jagged, meta: None }
                }
            )*
        }
    };
}
jagged_ctors! {
    jagged_bool => VecBool(bool), jagged_i8 => VecI8(i8), jagged_u8 => VecU8(u8),
    jagged_i16 => VecI16(i16), jagged_u16 => VecU16(u16), jagged_i32 => VecI32(i32),
    jagged_u32 => VecU32(u32), jagged_i64 => VecI64(i64), jagged_u64 => VecU64(u64),
    jagged_f32 => VecF32(f32), jagged_f64 => VecF64(f64),
}

/// Generate `Branch::vector_<name>` shortcuts for `std::vector<T>` branches,
/// written as `TBranchElement`s (one per inner vector, variable length).
macro_rules! vector_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl Branch {
            $(
                #[doc = concat!("A `std::vector<", stringify!($elem), ">` branch (a `TBranchElement`).")]
                pub fn $method(name: impl Into<String>, values: Vec<Vec<$elem>>) -> Branch {
                    Branch { name: name.into(), values: BranchValues::$variant(values), kind: BranchKind::StlVector, meta: None }
                }
            )*
        }
    };
}
vector_ctors! {
    vector_bool => VecBool(bool), vector_i8 => VecI8(i8), vector_u8 => VecU8(u8), vector_i16 => VecI16(i16),
    vector_u16 => VecU16(u16), vector_i32 => VecI32(i32), vector_u32 => VecU32(u32),
    vector_i64 => VecI64(i64), vector_u64 => VecU64(u64),
    vector_f32 => VecF32(f32), vector_f64 => VecF64(f64),
}

/// Generate `Branch::vector_vector_<name>` shortcuts for
/// `std::vector<std::vector<T>>` branches (a `TBranchElement`).
macro_rules! vector_vector_ctors {
    ($($method:ident => $variant:ident($elem:ty)),* $(,)?) => {
        impl Branch {
            $(
                #[doc = concat!("A `std::vector<std::vector<", stringify!($elem), ">>` branch (a `TBranchElement`).")]
                pub fn $method(name: impl Into<String>, values: Vec<Vec<Vec<$elem>>>) -> Branch {
                    let mut offsets = Vec::with_capacity(values.len() + 1);
                    offsets.push(0u64);
                    let mut acc = 0u64;
                    for row in &values {
                        acc += row.len() as u64;
                        offsets.push(acc);
                    }
                    let items = values.into_iter().flatten().collect();
                    Branch {
                        name: name.into(),
                        values: BranchValues::Nested { offsets, items: Box::new(BranchValues::$variant(items)) },
                        kind: BranchKind::StlVector,
                        meta: None,
                    }
                }
            )*
        }
    };
}
vector_vector_ctors! {
    vector_vector_bool => VecBool(bool), vector_vector_i8 => VecI8(i8),
    vector_vector_u8 => VecU8(u8), vector_vector_i16 => VecI16(i16),
    vector_vector_u16 => VecU16(u16), vector_vector_i32 => VecI32(i32),
    vector_vector_u32 => VecU32(u32), vector_vector_i64 => VecI64(i64),
    vector_vector_u64 => VecU64(u64), vector_vector_f32 => VecF32(f32),
    vector_vector_f64 => VecF64(f64),
}

impl Branch {
    /// This fixed-size array branch with width `n`: every row must hold `n`
    /// values. Rows give the width themselves, so this matters for a branch
    /// with no rows, such as the first, empty batch that fixes a
    /// [`TreeWriter`](crate::raw::TreeWriter)'s schema. Other kinds of branch are
    /// returned unchanged.
    #[must_use]
    pub fn with_width(mut self, n: usize) -> Self {
        if let BranchKind::Array(width) = &mut self.kind {
            *width = n;
        }
        self
    }

    /// A `std::vector<std::string>` branch (a `TBranchElement`).
    pub fn vector_string(name: impl Into<String>, values: Vec<Vec<String>>) -> Self {
        Self {
            name: name.into(),
            values: BranchValues::VecStr(values),
            kind: BranchKind::StlVector,
            meta: None,
        }
    }

    /// A string branch (`TLeafC`).
    pub fn string(name: impl Into<String>, values: Vec<String>) -> Self {
        Self {
            name: name.into(),
            values: BranchValues::Str(values),
            kind: BranchKind::Plain,
            meta: None,
        }
    }

    /// A split `std::vector<MyStruct>` branch: `class_name` is the struct's C++
    /// class name and `members` its fields (each a jagged sub-branch, all sharing
    /// per-entry lengths). Written as a parent `TBranchElement` (`fSplitLevel>0`)
    /// with one sub-branch per member and the struct's generated `TStreamerInfo`.
    pub fn split_vector(
        name: impl Into<String>,
        class_name: impl Into<String>,
        members: Vec<SplitMember>,
    ) -> Self {
        Self {
            name: name.into(),
            values: BranchValues::I32(Vec::new()),
            kind: BranchKind::Split(SplitSpec {
                class_name: class_name.into(),
                members,
            }),
            meta: None,
        }
    }
}

/// `(fStreamerType code, C++ type name, element byte size)` for a split-vector
/// member, from its jagged `BranchValues` variant.
pub(super) fn member_type_info(values: &BranchValues) -> (i32, &'static str, i32) {
    use BranchValues::*;
    match values {
        VecBool(_) => (18, "bool", 1),
        VecI8(_) => (1, "char", 1),
        VecU8(_) => (11, "unsigned char", 1),
        VecI16(_) => (2, "short", 2),
        VecU16(_) => (12, "unsigned short", 2),
        VecI32(_) => (3, "int", 4),
        VecU32(_) => (13, "unsigned int", 4),
        VecI64(_) => (16, "Long64_t", 8),
        VecU64(_) => (17, "ULong64_t", 8),
        VecF32(_) => (5, "float", 4),
        VecF64(_) => (8, "double", 8),
        // `SplitMember`'s constructors build these variants and no other, and
        // its fields are private. A made-up type code would write a file ROOT
        // misreads, so this is a bug to stop at, not a value to return.
        other => unreachable!("a split member holds {other:?}, which no constructor builds"),
    }
}

/// Per-entry row counts of a jagged `BranchValues` (the element count per entry).
pub(super) fn vec_row_lengths(values: &BranchValues) -> Vec<i32> {
    use BranchValues::*;
    macro_rules! lens {
        ($r:expr) => {
            $r.iter().map(|x| i32::saturating_from(x.len())).collect()
        };
    }
    match values {
        VecBool(r) => lens!(r),
        VecI8(r) => lens!(r),
        VecU8(r) => lens!(r),
        VecI16(r) => lens!(r),
        VecU16(r) => lens!(r),
        VecI32(r) => lens!(r),
        VecU32(r) => lens!(r),
        VecI64(r) => lens!(r),
        VecU64(r) => lens!(r),
        VecF32(r) => lens!(r),
        VecF64(r) => lens!(r),
        _ => Vec::new(),
    }
}

/// A split `std::vector<Struct>` branch stores one element per struct in every
/// member, and the writer takes the per-entry element counts from the first
/// member. So there must be at least one member, and every member must have the
/// same number of entries and the same element count in each entry; otherwise
/// the file would claim entries some members do not hold.
pub(super) fn check_split_members(branch: &str, spec: &SplitSpec) -> Result<()> {
    let Some(first) = spec.members.first() else {
        return Err(Error::InvalidInput(format!(
            "branch {branch:?}: a split {:?} branch needs at least one member; \
             add them with Branch::split_vector(.., vec![SplitMember::..])",
            spec.class_name
        )));
    };
    let counts = vec_row_lengths(&first.values);
    for m in &spec.members[1..] {
        let other = vec_row_lengths(&m.values);
        // Every member needs one value per struct: the first member's shape.
        if other.len() != counts.len() {
            return Err(Error::LengthMismatch {
                what: format!("branch {branch:?} split member {:?}", m.name),
                expected: counts.len(),
                found: other.len(),
            });
        }
        if let Some(entry) = other.iter().zip(&counts).position(|(a, b)| a != b) {
            return Err(Error::LengthMismatch {
                what: format!(
                    "branch {branch:?} split member {:?} in entry {entry}",
                    m.name
                ),
                expected: usize::saturating_from(counts[entry]),
                found: usize::saturating_from(other[entry]),
            });
        }
    }
    Ok(())
}

/// ROOT's class checksum: `id = id*3 + ch` over the class name, then each
/// member's name and type-name characters. Matches `TClass::GetCheckSum` for a
/// struct of plain members. (ROOT's split reader ignores it, but we match it.)
pub(super) fn class_checksum(class_name: &str, members: &[SplitMember]) -> u32 {
    let mut id: u32 = 0;
    let mut feed = |s: &str| {
        for ch in s.bytes() {
            id = id.wrapping_mul(3).wrapping_add(u32::from(ch));
        }
    };
    feed(class_name);
    for m in members {
        feed(&m.name);
        feed(member_type_info(&m.values).1);
    }
    id
}

/// The `TStreamerInfo` entry for a split branch's struct: version 1, one basic
/// member per struct member, as ROOT writes it for a class without `ClassDef`.
pub(super) fn split_class(spec: &SplitSpec) -> ClassDef<'_> {
    ClassDef {
        name: spec.class_name.as_str().into(),
        version: 1,
        checksum: class_checksum(&spec.class_name, &spec.members),
        elements: spec
            .members
            .iter()
            .map(|m| {
                let (ty, type_name, size) = member_type_info(&m.values);
                basic(&m.name, ty, size, type_name)
            })
            .collect(),
    }
}

impl std::fmt::Debug for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Branch")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SplitMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitMember")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
