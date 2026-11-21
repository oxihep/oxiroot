//! [`Schema`] and [`DataType`]: what a dataset's columns are on disk.
//!
//! A [`Column`] says how values are held in memory: seven shapes. A
//! [`DataType`] says how a column is stored in the file, which is a finer
//! question: a `std::set<int>` and a `std::vector<int>` are the same list of
//! integers in memory, and a half-precision float is an `f32`.
//!
//! [`Dataset::schema`](crate::Dataset::schema) reports the types a file
//! holds. A writer infers them from the first batch, and takes a schema
//! ([`DatasetWriter::schema`](crate::DatasetWriter::schema)) when the stored
//! type should be something the data alone does not say.

use std::fmt;

use crate::Column;

/// How one column is stored in a file.
///
/// Each type has one in-memory shape, its [`storage`](Self::storage): the
/// [`Column`] a reader returns for it and a writer takes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DataType {
    /// `bool`.
    Bool,
    /// `i8`.
    I8,
    /// `u8`.
    U8,
    /// `i16`.
    I16,
    /// `u16`.
    U16,
    /// `i32`.
    I32,
    /// `u32`.
    U32,
    /// `i64`.
    I64,
    /// `u64`.
    U64,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// An `f32` stored as an IEEE half-precision float (`RNTuple` only).
    Half,
    /// An `f32` stored with its low mantissa bits dropped, in `bits` bits
    /// (10 to 31; `RNTuple` only).
    Truncated {
        /// Bits kept per value.
        bits: u16,
    },
    /// An `f32` stored as an integer of `bits` bits spanning `min..=max` (1 to
    /// 32 bits; `RNTuple` only). A value outside the range is clamped.
    Quantized {
        /// Bits per value.
        bits: u16,
        /// The smallest value the column can hold.
        min: f64,
        /// The largest value the column can hold.
        max: f64,
    },
    /// A UTF-8 string.
    String,
    /// A `std::vector<T>`: any number of values per entry.
    List(Box<Self>),
    /// A C-style array whose length another leaf counts, `x[n]` (`TTree`
    /// only). Held as a list.
    CArray(Box<Self>),
    /// A fixed number of values per entry: `x[N]` in a tree, `std::array<T, N>`
    /// in an `RNTuple`.
    Array(usize, Box<Self>),
    /// A `std::optional<T>` (`RNTuple` only).
    Optional(Box<Self>),
    /// A `std::unique_ptr<T>`, held as an optional (`RNTuple` only).
    UniquePtr(Box<Self>),
    /// A `std::atomic<T>`, held as the bare `T` (`RNTuple` only).
    Atomic(Box<Self>),
    /// A `std::set<T>`, held as a list (`RNTuple` only).
    Set(Box<Self>),
    /// A `std::map<K, V>`, held as a list of `{_0: K, _1: V}` structs
    /// (`RNTuple` only).
    Map(Box<Self>, Box<Self>),
    /// A `std::bitset<N>`, held as an array of `N` `bool`s (`RNTuple` only).
    Bitset(usize),
    /// A record: named members. A `std::pair` or `std::tuple` when the members
    /// are named `_0`, `_1`, …; a user class otherwise.
    Struct(Vec<(String, Self)>),
}

impl DataType {
    /// The in-memory shape of this type: itself with every storage choice
    /// removed, so a half-precision float is an `f32` and a set is a list.
    /// A [`Column`] can be written as this type when the column's own
    /// [`data_type`](Column::data_type) equals this.
    #[must_use]
    pub fn storage(&self) -> Self {
        let boxed = |t: &Self| Box::new(t.storage());
        match self {
            Self::Half | Self::Truncated { .. } | Self::Quantized { .. } => Self::F32,
            Self::List(t) | Self::CArray(t) | Self::Set(t) => Self::List(boxed(t)),
            Self::Array(n, t) => Self::Array(*n, boxed(t)),
            Self::Optional(t) | Self::UniquePtr(t) => Self::Optional(boxed(t)),
            Self::Atomic(t) => t.storage(),
            Self::Map(k, v) => Self::List(Box::new(Self::Struct(vec![
                ("_0".into(), k.storage()),
                ("_1".into(), v.storage()),
            ]))),
            Self::Bitset(n) => Self::Array(*n, Box::new(Self::Bool)),
            Self::Struct(members) => Self::Struct(
                members
                    .iter()
                    .map(|(n, t)| (n.clone(), t.storage()))
                    .collect(),
            ),
            scalar => scalar.clone(),
        }
    }

    /// Whether this is a number or a `bool`, stored plainly.
    pub(crate) const fn is_plain_scalar(&self) -> bool {
        matches!(
            self,
            Self::Bool
                | Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::F32
                | Self::F64
        )
    }

    /// The C++ spelling `RNTuple` records for this type, as ROOT writes it.
    pub(crate) fn cpp_name(&self) -> String {
        match self {
            Self::Bool => "bool".into(),
            Self::I8 => "std::int8_t".into(),
            Self::U8 => "std::uint8_t".into(),
            Self::I16 => "std::int16_t".into(),
            Self::U16 => "std::uint16_t".into(),
            Self::I32 => "std::int32_t".into(),
            Self::U32 => "std::uint32_t".into(),
            Self::I64 => "std::int64_t".into(),
            Self::U64 => "std::uint64_t".into(),
            Self::F32 | Self::Half | Self::Truncated { .. } | Self::Quantized { .. } => {
                "float".into()
            }
            Self::F64 => "double".into(),
            Self::String => "std::string".into(),
            Self::List(t) | Self::CArray(t) => format!("std::vector<{}>", t.cpp_name()),
            Self::Array(n, t) => format!("std::array<{},{n}>", t.cpp_name()),
            Self::Optional(t) => format!("std::optional<{}>", t.cpp_name()),
            Self::UniquePtr(t) => format!("std::unique_ptr<{}>", t.cpp_name()),
            Self::Atomic(t) => format!("std::atomic<{}>", t.cpp_name()),
            Self::Set(t) => format!("std::set<{}>", t.cpp_name()),
            Self::Map(k, v) => format!("std::map<{},{}>", k.cpp_name(), v.cpp_name()),
            Self::Bitset(n) => format!("std::bitset<{n}>"),
            Self::Struct(members) => {
                let inner: Vec<String> = members.iter().map(|(_, t)| t.cpp_name()).collect();
                match inner.len() {
                    2 => format!("std::pair<{}>", inner.join(",")),
                    _ => format!("std::tuple<{}>", inner.join(",")),
                }
            }
        }
    }
}

impl fmt::Display for DataType {
    /// The type in Rust's spelling, with the storage choice in front where
    /// there is one: `Vec<f32>`, `half f32`, `set Vec<i32>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("bool"),
            Self::I8 => f.write_str("i8"),
            Self::U8 => f.write_str("u8"),
            Self::I16 => f.write_str("i16"),
            Self::U16 => f.write_str("u16"),
            Self::I32 => f.write_str("i32"),
            Self::U32 => f.write_str("u32"),
            Self::I64 => f.write_str("i64"),
            Self::U64 => f.write_str("u64"),
            Self::F32 => f.write_str("f32"),
            Self::F64 => f.write_str("f64"),
            Self::Half => f.write_str("half f32"),
            Self::Truncated { bits } => write!(f, "{bits}-bit truncated f32"),
            Self::Quantized { bits, min, max } => {
                write!(f, "{bits}-bit f32 in {min}..={max}")
            }
            Self::String => f.write_str("String"),
            Self::List(t) => write!(f, "Vec<{t}>"),
            Self::CArray(t) => write!(f, "counted Vec<{t}>"),
            Self::Array(n, t) => write!(f, "[{t}; {n}]"),
            Self::Optional(t) => write!(f, "Option<{t}>"),
            Self::UniquePtr(t) => write!(f, "unique_ptr Option<{t}>"),
            Self::Atomic(t) => write!(f, "atomic {t}"),
            Self::Set(t) => write!(f, "set Vec<{t}>"),
            Self::Map(k, v) => write!(f, "map Vec<({k}, {v})>"),
            Self::Bitset(n) => write!(f, "bitset [bool; {n}]"),
            Self::Struct(members) => {
                f.write_str("{")?;
                for (i, (name, t)) in members.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{name}: {t}")?;
                }
                f.write_str("}")
            }
        }
    }
}

impl Column {
    /// The type this column is stored as when nothing says otherwise: its
    /// in-memory shape, with lists as `std::vector` and optionals as
    /// `std::optional`.
    #[must_use]
    pub fn data_type(&self) -> DataType {
        let boxed = |c: &Self| Box::new(c.data_type());
        match self {
            Self::Bool(_) => DataType::Bool,
            Self::I8(_) => DataType::I8,
            Self::U8(_) => DataType::U8,
            Self::I16(_) => DataType::I16,
            Self::U16(_) => DataType::U16,
            Self::I32(_) => DataType::I32,
            Self::U32(_) => DataType::U32,
            Self::I64(_) => DataType::I64,
            Self::U64(_) => DataType::U64,
            Self::F32(_) => DataType::F32,
            Self::F64(_) => DataType::F64,
            Self::String { .. } => DataType::String,
            Self::List { values, .. } => DataType::List(boxed(values)),
            Self::Array { len, values } => DataType::Array(*len, boxed(values)),
            Self::Optional { values, .. } => DataType::Optional(boxed(values)),
            Self::Struct(members) => DataType::Struct(
                members
                    .iter()
                    .map(|(n, c)| (n.clone(), c.data_type()))
                    .collect(),
            ),
        }
    }
}

/// The columns of a dataset: each one's name and how it is stored, in order.
///
/// ```
/// use oxiroot_dataset::{DataType, Schema};
///
/// let schema = Schema::new([
///     ("mass", DataType::F64),
///     ("pt", DataType::Half), // an f32 column, stored in 16 bits
/// ]);
/// assert_eq!(schema.get("pt"), Some(&DataType::Half));
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    columns: Vec<(String, DataType)>,
}

impl Schema {
    /// A schema of the given columns, in order.
    pub fn new<N: Into<String>>(columns: impl IntoIterator<Item = (N, DataType)>) -> Self {
        columns.into_iter().collect()
    }

    /// The type of column `name`, if there is one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DataType> {
        self.columns
            .iter()
            .find_map(|(n, t)| (n == name).then_some(t))
    }

    /// Store column `name` as `data_type`, replacing its type or adding it at
    /// the end.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, data_type: DataType) -> Self {
        let name = name.into();
        match self.columns.iter_mut().find(|(n, _)| *n == name) {
            Some((_, t)) => *t = data_type,
            None => self.columns.push((name, data_type)),
        }
        self
    }

    /// Each column's name and type, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &DataType)> {
        self.columns.iter().map(|(n, t)| (n.as_str(), t))
    }

    /// The number of columns.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.columns.len()
    }

    /// Whether there are no columns.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

impl<N: Into<String>> FromIterator<(N, DataType)> for Schema {
    fn from_iter<I: IntoIterator<Item = (N, DataType)>>(iter: I) -> Self {
        Self {
            columns: iter.into_iter().map(|(n, t)| (n.into(), t)).collect(),
        }
    }
}

impl fmt::Display for Schema {
    /// One `name: type` per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, data_type) in &self.columns {
            writeln!(f, "{name}: {data_type}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_removes_every_storage_choice() {
        let list = |t| DataType::List(Box::new(t));
        assert_eq!(DataType::Half.storage(), DataType::F32);
        assert_eq!(
            DataType::Quantized {
                bits: 12,
                min: 0.0,
                max: 1.0
            }
            .storage(),
            DataType::F32
        );
        assert_eq!(
            DataType::Set(Box::new(DataType::I32)).storage(),
            list(DataType::I32)
        );
        assert_eq!(
            DataType::CArray(Box::new(DataType::Half)).storage(),
            list(DataType::F32)
        );
        assert_eq!(
            DataType::Atomic(Box::new(DataType::U8)).storage(),
            DataType::U8
        );
        assert_eq!(
            DataType::UniquePtr(Box::new(DataType::F64)).storage(),
            DataType::Optional(Box::new(DataType::F64))
        );
        assert_eq!(
            DataType::Bitset(4).storage(),
            DataType::Array(4, Box::new(DataType::Bool))
        );
        assert_eq!(
            DataType::Map(Box::new(DataType::I32), Box::new(DataType::F64)).storage(),
            list(DataType::Struct(vec![
                ("_0".into(), DataType::I32),
                ("_1".into(), DataType::F64)
            ]))
        );
    }

    #[test]
    fn a_columns_type_is_its_own_storage() {
        let column = Column::List {
            offsets: vec![0, 2],
            values: Box::new(Column::F32(vec![1.0, 2.0])),
        };
        let data_type = column.data_type();
        assert_eq!(data_type, DataType::List(Box::new(DataType::F32)));
        assert_eq!(data_type.storage(), data_type);
        assert_eq!(data_type.to_string(), "Vec<f32>");
    }

    #[test]
    fn with_replaces_a_type_or_adds_a_column() {
        let schema = Schema::new([("a", DataType::F32)])
            .with("a", DataType::Half)
            .with("b", DataType::I32);
        assert_eq!(
            schema.iter().collect::<Vec<_>>(),
            [("a", &DataType::Half), ("b", &DataType::I32)]
        );
    }
}
