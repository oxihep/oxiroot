//! [`Batch`]: named columns with one entry count, the unit of reading and
//! writing.

use std::ops::Range;

use oxiroot_io_core::{Error, Result};

use crate::schema::Schema;
use crate::{Column, Jagged, Primitive, Strings};

/// Named columns, all holding the same number of entries: what
/// [`Dataset`](crate::Dataset) reads, and what a
/// [`NamedBatch`](crate::NamedBatch) or a [`DatasetWriter`](crate::DatasetWriter)
/// writes, for a `TTree` and an `RNTuple` alike.
///
/// Build one from your own data with [`column`](Self::column):
///
/// ```
/// use oxiroot_dataset::Batch;
///
/// let batch = Batch::new()
///     .column("mass", vec![91.2_f64, 125.1])?
///     .column("jet_pt", vec![vec![40.0_f32, 31.5], vec![]])?
///     .column("label", vec!["Z", "H"])?;
/// assert_eq!(batch.len(), 2);
/// assert_eq!(batch.values::<f64>("mass")?, [91.2, 125.1]);
/// assert_eq!(batch.jagged::<f32>("jet_pt")?.row(0), [40.0, 31.5]);
/// assert_eq!(batch.strings("label")?.get(1), "H");
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Batch {
    names: Vec<String>,
    columns: Vec<Column>,
    entries: usize,
}

impl Batch {
    /// An empty batch: no columns, no entries.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
    }

    /// Whether the batch has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// The column names, in order.
    pub fn names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// `(name, column)` pairs, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Column)> {
        self.names.iter().map(String::as_str).zip(&self.columns)
    }

    /// The column `name`, if there is one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Column> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|i| &self.columns[i])
    }

    /// This batch plus a column `name` holding `data` (a `Vec` of scalars,
    /// strings, vectors, arrays or optionals; or a [`Column`] itself).
    ///
    /// # Errors
    ///
    /// The name is taken, or the column has another entry count than the
    /// batch.
    pub fn column(mut self, name: impl Into<String>, data: impl Into<Column>) -> Result<Self> {
        self.insert(name, data.into())?;
        Ok(self)
    }

    /// This batch plus a list column `name` given flat: entry `i` holds
    /// `values[offsets[i]..offsets[i + 1]]`, and `offsets` starts at `0`.
    ///
    /// # Errors
    ///
    /// Offsets that do not partition `values`, a name that is taken, or another
    /// entry count than the batch.
    pub fn list<T: Primitive>(
        self,
        name: impl Into<String>,
        offsets: Vec<u64>,
        values: Vec<T>,
    ) -> Result<Self> {
        self.column(
            name,
            Column::List {
                offsets,
                values: Box::new(T::into_column(values)),
            },
        )
    }

    /// Add column `name`, checking its invariants and its entry count.
    ///
    /// # Errors
    ///
    /// As [`column`](Self::column), or a malformed `column`.
    pub fn insert(&mut self, name: impl Into<String>, column: Column) -> Result<()> {
        let name = name.into();
        column.validate(&name)?;
        if self.names.contains(&name) {
            return Err(Error::InvalidInput(format!(
                "the batch already has a column {name:?}"
            )));
        }
        if !self.names.is_empty() && column.len() != self.entries {
            return Err(Error::LengthMismatch {
                what: format!("column {name:?} entries"),
                expected: self.entries,
                found: column.len(),
            });
        }
        self.entries = column.len();
        self.names.push(name);
        self.columns.push(column);
        Ok(())
    }

    /// Remove column `name` and return it.
    pub fn remove(&mut self, name: &str) -> Option<Column> {
        let i = self.names.iter().position(|n| n == name)?;
        self.names.remove(i);
        if self.names.is_empty() {
            self.entries = 0;
        }
        Some(self.columns.remove(i))
    }

    fn require(&self, name: &str) -> Result<&Column> {
        self.get(name).ok_or_else(|| Error::NotFound {
            what: "column",
            name: name.to_string(),
        })
    }

    fn mismatch(name: &str, found: &Column, expected: String) -> Error {
        Error::TypeMismatch {
            name: name.to_string(),
            found: found.type_name(),
            expected,
        }
    }

    /// The values of the scalar column `name`, as a slice of `T`.
    ///
    /// # Errors
    ///
    /// No such column, or one that is not `T`.
    pub fn values<T: Primitive>(&self, name: &str) -> Result<&[T]> {
        let column = self.require(name)?;
        T::slice(column).ok_or_else(|| Self::mismatch(name, column, T::NAME.to_string()))
    }

    /// The list (or fixed-array) column `name` of `T`, viewed in place: one
    /// slice of values per entry.
    ///
    /// # Errors
    ///
    /// No such column, or one that is not a list or array of `T`.
    pub fn jagged<T: Primitive>(&self, name: &str) -> Result<Jagged<'_, T>> {
        let column = self.require(name)?;
        let expected = || format!("Vec<{}>", T::NAME);
        match column {
            Column::List { offsets, values } => T::slice(values)
                .map(|v| Jagged::new(offsets, v))
                .ok_or_else(|| Self::mismatch(name, column, expected())),
            Column::Array { .. } => Err(Error::InvalidInput(format!(
                "column {name:?} is a fixed array ({}); read it with Batch::arrays",
                column.type_name()
            ))),
            _ => Err(Self::mismatch(name, column, expected())),
        }
    }

    /// The fixed-array column `name` of `T`: every entry's `N` values, back
    /// to back, and `N`. Use `chunks_exact(N)` for one slice per entry.
    ///
    /// # Errors
    ///
    /// No such column, or one that is not an array of `T`.
    pub fn arrays<T: Primitive>(&self, name: &str) -> Result<(&[T], usize)> {
        let column = self.require(name)?;
        match column {
            Column::Array { len, values } => T::slice(values)
                .map(|v| (v, *len))
                .ok_or_else(|| Self::mismatch(name, column, format!("[{}; N]", T::NAME))),
            _ => Err(Self::mismatch(name, column, format!("[{}; N]", T::NAME))),
        }
    }

    /// The string column `name`, viewed in place.
    ///
    /// # Errors
    ///
    /// No such column, or one that is not strings.
    pub fn strings(&self, name: &str) -> Result<Strings<'_>> {
        let column = self.require(name)?;
        match column {
            Column::String { offsets, text } => Ok(Strings::new(offsets, text)),
            _ => Err(Self::mismatch(name, column, "String".to_string())),
        }
    }

    /// The entries whose `mask` value is `true`, every column kept.
    ///
    /// # Errors
    ///
    /// `mask` does not hold one value per entry.
    pub fn filter(&self, mask: &[bool]) -> Result<Self> {
        if mask.len() != self.entries {
            return Err(Error::LengthMismatch {
                what: "filter mask".into(),
                expected: self.entries,
                found: mask.len(),
            });
        }
        Ok(Self {
            names: self.names.clone(),
            columns: self.columns.iter().map(|c| c.filter(mask)).collect(),
            entries: mask.iter().filter(|&&m| m).count(),
        })
    }

    /// Entries `range` (clamped), every column kept.
    #[must_use]
    pub fn slice(&self, range: Range<usize>) -> Self {
        let end = range.end.min(self.entries);
        let start = range.start.min(end);
        Self {
            names: self.names.clone(),
            columns: self.columns.iter().map(|c| c.slice(start, end)).collect(),
            entries: end - start,
        }
    }

    /// Only the columns `names`, in that order.
    ///
    /// # Errors
    ///
    /// A name with no column.
    pub fn select<I, S>(&self, names: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Self::new();
        for name in names {
            let name = name.as_ref();
            out.insert(name, self.require(name)?.clone())?;
        }
        if out.names.is_empty() {
            out.entries = self.entries;
        }
        Ok(out)
    }

    /// Append `other`'s entries. It must have the same columns, in the same
    /// order, of the same types.
    ///
    /// # Errors
    ///
    /// The schemas differ.
    pub fn append(&mut self, other: Self) -> Result<()> {
        if self.names.is_empty() && self.entries == 0 {
            *self = other;
            return Ok(());
        }
        if self.names != other.names {
            return Err(Error::SchemaChanged {
                detail: format!(
                    "appending columns {:?} to a batch of {:?}",
                    other.names, self.names
                ),
            });
        }
        // Check every column before changing any: a mismatch found half way
        // would leave the earlier columns longer than the batch.
        for ((name, col), more) in self.names.iter().zip(&self.columns).zip(&other.columns) {
            let (ours, theirs) = (col.type_name(), more.type_name());
            if ours != theirs {
                return Err(Error::SchemaChanged {
                    detail: format!("column {name:?} is {ours}, not {theirs}"),
                });
            }
        }
        for ((name, col), more) in self.names.iter().zip(&mut self.columns).zip(other.columns) {
            let found = more.type_name();
            col.append(more).map_err(|_| Error::SchemaChanged {
                detail: format!("column {name:?} is {}, not {found}", col.type_name()),
            })?;
        }
        self.entries += other.entries;
        Ok(())
    }

    /// Each column's name and the type it is stored as when nothing says
    /// otherwise; see [`Column::data_type`].
    #[must_use]
    pub fn schema(&self) -> Schema {
        self.iter().map(|(n, c)| (n, c.data_type())).collect()
    }

    /// The columns, by value, in order.
    pub(crate) fn into_columns(self) -> impl Iterator<Item = (String, Column)> {
        self.names.into_iter().zip(self.columns)
    }

    /// Roughly how many bytes the values take.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.columns.iter().map(Column::byte_size).sum()
    }
}
