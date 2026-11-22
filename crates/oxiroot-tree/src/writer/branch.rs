//! The branches a caller builds: [`Branch`], one named [`Column`] and how it
//! is stored.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::streamer_gen::{ClassDef, basic};
use oxiroot_io_core::{Column, Error, Result};

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

/// One named branch to write: a [`Column`] and how a tree stores it.
///
/// [`Branch::new`] stores the column as its shape says:
///
/// | column | branch |
/// |---|---|
/// | a scalar (`Vec<f64>`, …) | one value per entry |
/// | a string (`Vec<String>`) | a `TLeafC` string |
/// | an array (`Vec<[T; N]>`) | a fixed-size `x[N]` |
/// | a list (`Vec<Vec<T>>`) | a `std::vector<T>` |
/// | a list of strings | a `std::vector<std::string>` |
/// | a list of lists | a `std::vector<std::vector<T>>` |
/// | a list of structs of scalars | a split `std::vector<Struct>` |
///
/// [`counted`](Branch::counted) stores a list as a C-style `x[n]` beside a
/// count branch instead, and [`class_name`](Branch::class_name) names the
/// struct of a split branch. A column a tree cannot hold (an optional, a
/// record at the top level, a variant) is refused when the tree is written.
pub struct Branch {
    /// Branch (and leaf) name.
    pub(crate) name: String,
    /// The values. Their shape goes with `kind`, which [`Branch::new`] derives
    /// from them, so the two cannot drift apart.
    pub(super) values: Column,
    /// How the branch is serialized.
    pub(super) kind: BranchKind,
    /// What the leaf's header says of the values, when the writer knows it
    /// without holding them all: see [`LeafMeta`].
    pub(super) meta: Option<LeafMeta>,
}

/// How a [`Branch`] is written.
pub(super) enum BranchKind {
    /// A scalar or string (`TLeafC`) branch.
    Plain,
    /// A fixed-size array `x[N]` of this width.
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
    /// A column a tree cannot hold, with what it is: refused at write time.
    Unwritable(String),
}

/// A split `std::vector<MyStruct>` branch: the struct's class name and one
/// member per sub-branch.
pub(super) struct SplitSpec {
    pub(super) class_name: String,
    pub(super) members: Vec<SplitMember>,
}

/// One member of a split `std::vector<MyStruct>` branch: its name and its
/// values, a list of scalars with the struct's own offsets.
pub(super) struct SplitMember {
    pub(super) name: String,
    pub(super) values: Column,
}

/// Whether `column` holds numbers or `bool`s, one per entry.
pub(super) fn is_scalar(column: &Column) -> bool {
    scalar_type(column).is_some()
}

/// `(fStreamerType code, C++ type name, element byte size)` of a scalar
/// column's values.
pub(super) fn scalar_type(column: &Column) -> Option<(i32, &'static str, i32)> {
    Some(match column {
        Column::Bool(_) => (18, "bool", 1),
        Column::I8(_) => (1, "char", 1),
        Column::U8(_) => (11, "unsigned char", 1),
        Column::I16(_) => (2, "short", 2),
        Column::U16(_) => (12, "unsigned short", 2),
        Column::I32(_) => (3, "int", 4),
        Column::U32(_) => (13, "unsigned int", 4),
        Column::I64(_) => (16, "Long64_t", 8),
        Column::U64(_) => (17, "ULong64_t", 8),
        Column::F32(_) => (5, "float", 4),
        Column::F64(_) => (8, "double", 8),
        _ => return None,
    })
}

/// `name` as a C++ identifier: other characters become `_`, and a leading
/// digit gets a `_` in front.
fn identifier(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

impl Branch {
    /// A branch named `name` holding `data`, stored as its shape says (see
    /// [`Branch`]).
    pub fn new(name: impl Into<String>, data: impl Into<Column>) -> Self {
        let name = name.into();
        let values = data.into();
        let unwritable = |values: &Column| BranchKind::Unwritable(values.type_name());
        let (values, kind) = match values {
            Column::Array { len, values } if is_scalar(&values) => {
                (Column::Array { len, values }, BranchKind::Array(len))
            }
            Column::List { offsets, values } => match *values {
                // One sub-branch per member, each with the struct's offsets.
                Column::Struct(members)
                    if !members.is_empty() && members.iter().all(|(_, c)| is_scalar(c)) =>
                {
                    let members = members
                        .into_iter()
                        .map(|(name, values)| SplitMember {
                            name,
                            values: Column::List {
                                offsets: offsets.clone(),
                                values: Box::new(values),
                            },
                        })
                        .collect();
                    let spec = SplitSpec {
                        class_name: identifier(&name),
                        members,
                    };
                    (Column::I32(Vec::new()), BranchKind::Split(spec))
                }
                inner => {
                    let storable = match &inner {
                        Column::List { values, .. } => is_scalar(values),
                        Column::String { .. } => true,
                        other => is_scalar(other),
                    };
                    let values = Column::List {
                        offsets,
                        values: Box::new(inner),
                    };
                    let kind = if storable {
                        BranchKind::StlVector
                    } else {
                        unwritable(&values)
                    };
                    (values, kind)
                }
            },
            values @ Column::String { .. } => (values, BranchKind::Plain),
            values if is_scalar(&values) => (values, BranchKind::Plain),
            values => {
                let kind = unwritable(&values);
                (values, kind)
            }
        };
        Self {
            name,
            values,
            kind,
            meta: None,
        }
    }

    /// This list branch as a C-style variable-length array `x[n]`: the values
    /// beside a count branch `n<name>`, in place of a `std::vector`. Only a
    /// list of numbers or `bool`s has this form; another branch is refused
    /// when the tree is written.
    #[must_use]
    pub fn counted(mut self) -> Self {
        self.kind = match (&self.kind, &self.values) {
            (BranchKind::StlVector, Column::List { values, .. }) if is_scalar(values) => {
                BranchKind::Jagged
            }
            _ => BranchKind::Unwritable(format!("counted {}", self.values.type_name())),
        };
        self
    }

    /// The C++ class name of this split branch's struct, where
    /// [`Branch::new`] named it after the branch. Other branches have no
    /// class and are returned unchanged.
    #[must_use]
    pub fn class_name(mut self, class_name: impl Into<String>) -> Self {
        if let BranchKind::Split(spec) = &mut self.kind {
            spec.class_name = class_name.into();
        }
        self
    }

    /// The branch's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A branch this writer builds itself: `values` already has the shape
    /// `kind` takes.
    pub(super) fn raw(name: String, values: Column, kind: BranchKind) -> Self {
        Self {
            name,
            values,
            kind,
            meta: None,
        }
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

    /// Refuse a branch a tree cannot hold, and one whose values do not hold
    /// together.
    pub(super) fn check(&self) -> Result<()> {
        if let BranchKind::Unwritable(what) = &self.kind {
            return Err(Error::Unsupported(format!(
                "branch {:?} is a {what}, which a TTree written by oxiroot cannot hold",
                self.name
            )));
        }
        self.values.validate(&self.name)?;
        if let Some(spec) = self.split() {
            for member in &spec.members {
                member
                    .values
                    .validate(&format!("{}.{}", self.name, member.name))?;
            }
        }
        Ok(())
    }
}

/// `(fStreamerType code, C++ type name, element byte size)` for a split-vector
/// member, from the values its list holds.
pub(super) fn member_type_info(values: &Column) -> (i32, &'static str, i32) {
    let inner = match values {
        Column::List { values, .. } => &**values,
        other => other,
    };
    // `Branch::new` makes a split branch only of scalar members, and the
    // fields are private. A made-up type code would write a file ROOT
    // misreads, so this is a bug to stop at, not a value to return.
    scalar_type(inner)
        .unwrap_or_else(|| unreachable!("a split member holds a {}", values.type_name()))
}

/// Per-entry row counts of a list column (the element count per entry).
pub(super) fn vec_row_lengths(values: &Column) -> Vec<i32> {
    match values {
        Column::List { offsets, .. } => offsets
            .windows(2)
            .map(|w| i32::saturating_from(w[1].saturating_sub(w[0])))
            .collect(),
        _ => Vec::new(),
    }
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
