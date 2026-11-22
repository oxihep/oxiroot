//! [`Schema`] and [`DataType`]: what a dataset's columns are on disk.
//!
//! A [`Column`] says how values are held in memory: seven shapes. A
//! [`DataType`] says how a column is stored in the file, which is a finer
//! question: a `std::set<int>` and a `std::vector<int>` are the same list of
//! integers in memory, and a half-precision float is an `f32`.
//!
//! [`Dataset::schema`](crate::Dataset::schema) reports the types a file
//! holds. A writer infers them from the first batch, and takes a schema
//! ([`DatasetWriter::with_schema`](crate::DatasetWriter::with_schema)) when the stored
//! type should be something the data alone does not say.

use std::fmt;

use crate::DataType;

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
    use crate::Column;

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
