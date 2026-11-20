//! [`Dataset`]: one reader for a `TTree` or an RNTuple, yielding
//! [`Batch`]es.

use std::ops::Range;

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, FileReader, Result};
use oxiroot_rntuple::{ANCHOR_CLASS, FieldValues, NtupleReader};
use oxiroot_tree::{BranchLayout, BranchValues, LeafType, TreeReader};

use crate::batch::Batch;
use crate::column::{Column, IntoColumn};

/// Which of ROOT's two columnar formats a dataset is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// A `TTree`.
    Tree,
    /// An RNTuple.
    Ntuple,
}

/// A `TTree` or an RNTuple, read as [`Batch`]es of flat columns. The format
/// is detected when it is opened.
///
/// ```no_run
/// use oxiroot_dataset::Dataset;
/// use oxiroot_io_core::FileReader;
///
/// let file = FileReader::open("events.root")?;
/// let events = Dataset::open(&file, "events")?;
/// let all = events.read(["mass"])?;                    // one batch, every entry
/// for batch in events.batches(["mass", "jet_pt"]).step(1_000_000) {
///     let batch = batch?;
///     let mass: &[f64] = batch.values("mass")?;
/// }
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
#[derive(Debug)]
pub struct Dataset<'f> {
    source: Source<'f>,
}

#[derive(Debug)]
enum Source<'f> {
    Tree(TreeReader<'f>),
    Ntuple(NtupleReader<'f>),
}

impl<'f> Dataset<'f> {
    /// Open the `TTree` or RNTuple named `name` in `file`'s top directory.
    ///
    /// # Errors
    ///
    /// No such key, a key that is neither a `TTree` nor an RNTuple, or a
    /// malformed one.
    pub fn open(file: &'f FileReader, name: &str) -> Result<Dataset<'f>> {
        let class = file
            .key(name)
            .ok_or_else(|| Error::NotFound {
                what: "key",
                name: name.to_string(),
            })?
            .class_name
            .clone();
        Self::open_class(
            &class,
            name,
            || TreeReader::open(file, name),
            || NtupleReader::open(file, name),
        )
    }

    /// Open the `TTree` or RNTuple named `name` in subdirectory `subdir` (a
    /// `/`-separated path).
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open).
    pub fn open_in(file: &'f FileReader, subdir: &str, name: &str) -> Result<Dataset<'f>> {
        let dir = file.subdir(subdir)?;
        let class = oxiroot_io_core::raw::find_key(&dir.keys, name)
            .ok_or_else(|| Error::NotFound {
                what: "key",
                name: format!("{}/{name}", subdir.trim_end_matches('/')),
            })?
            .class_name
            .clone();
        Self::open_class(
            &class,
            name,
            || TreeReader::open_in(file, subdir, name),
            || NtupleReader::open_in(file, subdir, name),
        )
    }

    fn open_class(
        class: &str,
        name: &str,
        tree: impl FnOnce() -> Result<TreeReader<'f>>,
        ntuple: impl FnOnce() -> Result<NtupleReader<'f>>,
    ) -> Result<Dataset<'f>> {
        let source = match class {
            "TTree" | "TNtuple" | "TNtupleD" => Source::Tree(tree()?),
            c if c == ANCHOR_CLASS => Source::Ntuple(ntuple()?),
            other => {
                return Err(Error::WrongClass {
                    name: name.to_string(),
                    found: other.to_string(),
                    expected: format!("TTree or {ANCHOR_CLASS}"),
                });
            }
        };
        Ok(Dataset { source })
    }

    /// The format the dataset is stored in.
    #[must_use]
    pub fn format(&self) -> Format {
        match self.source {
            Source::Tree(_) => Format::Tree,
            Source::Ntuple(_) => Format::Ntuple,
        }
    }

    /// The number of entries.
    #[must_use]
    pub fn entries(&self) -> u64 {
        match &self.source {
            Source::Tree(t) => t.entries(),
            Source::Ntuple(n) => n.entries(),
        }
    }

    /// The column names: a tree's branches, an RNTuple's top-level fields.
    #[must_use]
    pub fn columns(&self) -> Vec<&str> {
        match &self.source {
            Source::Tree(t) => t.branch_names(),
            Source::Ntuple(n) => n.field_names(),
        }
    }

    /// The `TTree` reader underneath, for what only a tree has (friends,
    /// indices, split objects).
    #[must_use]
    pub fn as_tree(&self) -> Option<&TreeReader<'f>> {
        match &self.source {
            Source::Tree(t) => Some(t),
            Source::Ntuple(_) => None,
        }
    }

    /// The RNTuple reader underneath, for what only an RNTuple has (records,
    /// variants, the raw columns).
    #[must_use]
    pub fn as_ntuple(&self) -> Option<&NtupleReader<'f>> {
        match &self.source {
            Source::Ntuple(n) => Some(n),
            Source::Tree(_) => None,
        }
    }

    /// Every entry of `columns`, as one batch.
    ///
    /// # Errors
    ///
    /// A column that does not exist, or one of a shape a [`Batch`] cannot
    /// hold (an RNTuple record or variant); read those through
    /// [`as_ntuple`](Self::as_ntuple).
    pub fn read<I, S>(&self, columns: I) -> Result<Batch>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let names: Vec<String> = columns
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        self.read_window(&names, 0..self.entries(), false)
    }

    /// `columns` in batches of entries, in order. By default each batch is
    /// one cluster (an RNTuple cluster, or the entries where every selected
    /// branch of a tree starts a basket); [`step`](Batches::step) asks for
    /// bigger or smaller ones.
    pub fn batches<I, S>(&self, columns: I) -> Batches<'_, 'f>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Batches {
            dataset: self,
            columns: columns
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
            step: 0,
            range: 0..self.entries(),
            parallel: false,
            cuts: None,
            next: None,
        }
    }

    /// The entries at which every column in `names` can be cut without
    /// decompressing a basket or page twice, ascending, ending at the entry
    /// count.
    fn cut_points(&self, names: &[String]) -> Result<Vec<u64>> {
        match &self.source {
            Source::Ntuple(n) => Ok(n.cluster_boundaries()),
            Source::Tree(t) => {
                let mut common: Option<Vec<u64>> = None;
                for name in names {
                    let b = t.basket_boundaries(name)?;
                    common = Some(match common {
                        None => b,
                        Some(c) => c
                            .into_iter()
                            .filter(|x| b.binary_search(x).is_ok())
                            .collect(),
                    });
                }
                Ok(common.unwrap_or_else(|| vec![0, t.entries()]))
            }
        }
    }

    fn read_window(&self, names: &[String], range: Range<u64>, parallel: bool) -> Result<Batch> {
        let read = |name: &String| -> Result<Column> {
            match &self.source {
                Source::Tree(t) => tree_column(t, name, range.clone(), parallel),
                Source::Ntuple(n) => ntuple_column(n, name, range.clone(), parallel),
            }
        };
        #[cfg(feature = "rayon")]
        let columns: Vec<Column> = if parallel {
            use rayon::prelude::*;
            names.par_iter().map(read).collect::<Result<_>>()?
        } else {
            names.iter().map(read).collect::<Result<_>>()?
        };
        #[cfg(not(feature = "rayon"))]
        let columns: Vec<Column> = names.iter().map(read).collect::<Result<_>>()?;
        let mut batch = Batch::new();
        for (name, column) in names.iter().zip(columns) {
            batch.insert(name.clone(), column)?;
        }
        Ok(batch)
    }
}

/// Batches of a [`Dataset`]'s columns, in entry order, from
/// [`Dataset::batches`]. Set the size and range, then iterate.
#[must_use = "Batches is an iterator and reads nothing until iterated"]
#[derive(Debug)]
pub struct Batches<'d, 'f> {
    dataset: &'d Dataset<'f>,
    columns: Vec<String>,
    step: u64,
    range: Range<u64>,
    parallel: bool,
    cuts: Option<Vec<u64>>,
    next: Option<u64>,
}

impl Batches<'_, '_> {
    /// Aim for `entries` entries per batch. A batch still ends where a cluster
    /// (or aligned basket) does, the first such boundary at or past
    /// `entries` — unless that is more than twice as far, where it is cut at
    /// `entries` exactly.
    pub fn step(mut self, entries: u64) -> Self {
        self.step = entries;
        self
    }

    /// Only entries `range` (clamped to the entry count).
    pub fn range(mut self, range: Range<u64>) -> Self {
        let end = range.end.min(self.dataset.entries());
        self.range = range.start.min(end)..end;
        self
    }

    /// Read a batch's columns concurrently, on rayon's global thread pool.
    #[cfg(feature = "rayon")]
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// The end of the batch starting at `start`.
    fn batch_end(&self, cuts: &[u64], start: u64) -> u64 {
        let end = self.range.end;
        let target = start.saturating_add(self.step.max(1));
        let aligned = cuts.iter().copied().find(|&c| c >= target && c > start);
        let stop = match aligned {
            Some(c) if self.step == 0 || c - start <= 2 * self.step => c,
            _ if self.step == 0 => end,
            _ => target,
        };
        stop.min(end)
    }
}

impl Iterator for Batches<'_, '_> {
    type Item = Result<Batch>;

    fn next(&mut self) -> Option<Result<Batch>> {
        if self.cuts.is_none() {
            match self.dataset.cut_points(&self.columns) {
                Ok(c) => self.cuts = Some(c),
                Err(e) => {
                    self.next = Some(self.range.end);
                    self.cuts = Some(Vec::new());
                    return Some(Err(e));
                }
            }
        }
        let start = self.next.unwrap_or(self.range.start);
        if start >= self.range.end {
            return None;
        }
        let cuts = self.cuts.take().unwrap_or_default();
        let stop = self.batch_end(&cuts, start);
        self.cuts = Some(cuts);
        self.next = Some(stop);
        Some(
            self.dataset
                .read_window(&self.columns, start..stop, self.parallel),
        )
    }
}

/// Read one tree branch's window as a flat column: straight to offsets and
/// values where the layout allows, through [`BranchValues`] otherwise.
fn tree_column(
    t: &TreeReader<'_>,
    name: &str,
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    let layout = t.branch_layout(name).ok_or_else(|| Error::NotFound {
        what: "column",
        name: name.to_string(),
    })?;
    let is_str = t.branch_type(name) == Some(LeafType::Str);
    #[cfg(feature = "rayon")]
    let branch = || t.branch(name).range(range.clone()).parallel(parallel);
    #[cfg(not(feature = "rayon"))]
    let branch = || {
        let _ = parallel;
        t.branch(name).range(range.clone())
    };
    match layout {
        BranchLayout::Array(len) if !is_str => match branch().flat() {
            Ok(j) => Ok(Column::Array {
                len,
                values: Box::new(branch_column(j.values)?),
            }),
            Err(_) => array_of(branch_column(branch().values()?)?, len, name),
        },
        BranchLayout::List if !is_str => match branch().flat() {
            Ok(j) => Ok(Column::List {
                offsets: j.offsets,
                values: Box::new(branch_column(j.values)?),
            }),
            Err(_) => branch_column(branch().values()?),
        },
        _ => branch_column(branch().values()?),
    }
}

/// A list column whose every entry has `len` values, as an array column.
fn array_of(column: Column, len: usize, name: &str) -> Result<Column> {
    match column {
        Column::List { offsets, values }
            if offsets.windows(2).all(|w| w[1] - w[0] == len as u64) =>
        {
            Ok(Column::Array { len, values })
        }
        other => Err(Error::Format(format!(
            "column {name:?}: expected arrays of {len}, got {}",
            other.type_name()
        ))),
    }
}

/// A tree branch's values as a column (moving scalar vectors, flattening
/// nested ones).
fn branch_column(values: BranchValues) -> Result<Column> {
    Ok(match values {
        BranchValues::Bool(v) => Column::Bool(v),
        BranchValues::I8(v) => Column::I8(v),
        BranchValues::U8(v) => Column::U8(v),
        BranchValues::I16(v) => Column::I16(v),
        BranchValues::U16(v) => Column::U16(v),
        BranchValues::I32(v) => Column::I32(v),
        BranchValues::U32(v) => Column::U32(v),
        BranchValues::I64(v) => Column::I64(v),
        BranchValues::U64(v) => Column::U64(v),
        BranchValues::F32(v) => Column::F32(v),
        BranchValues::F64(v) => Column::F64(v),
        BranchValues::Str(v) => v.into_column(),
        BranchValues::VecBool(v) => v.into_column(),
        BranchValues::VecI8(v) => v.into_column(),
        BranchValues::VecU8(v) => v.into_column(),
        BranchValues::VecI16(v) => v.into_column(),
        BranchValues::VecU16(v) => v.into_column(),
        BranchValues::VecI32(v) => v.into_column(),
        BranchValues::VecU32(v) => v.into_column(),
        BranchValues::VecI64(v) => v.into_column(),
        BranchValues::VecU64(v) => v.into_column(),
        BranchValues::VecF32(v) => v.into_column(),
        BranchValues::VecF64(v) => v.into_column(),
        BranchValues::VecStr(v) => v.into_column(),
        BranchValues::Nested { offsets, items } => Column::List {
            offsets,
            values: Box::new(branch_column(*items)?),
        },
        other => {
            return Err(Error::Unsupported(format!(
                "a {} branch cannot be held in a batch",
                other.type_name()
            )));
        }
    })
}

/// Read one RNTuple field's window as a flat column.
fn ntuple_column(
    n: &NtupleReader<'_>,
    name: &str,
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    #[cfg(feature = "rayon")]
    let field = || n.field(name).range(range.clone()).parallel(parallel);
    #[cfg(not(feature = "rayon"))]
    let field = || {
        let _ = parallel;
        n.field(name).range(range.clone())
    };
    let array_size = n
        .header()
        .fields
        .iter()
        .enumerate()
        .find(|(i, f)| f.name == name && f.parent_field_id as usize == *i)
        .ok_or_else(|| Error::NotFound {
            what: "column",
            name: name.to_string(),
        })?
        .1
        .array_size;
    if let Some(len) = array_size {
        return array_of(
            field_column(name, field().values()?)?,
            usize::saturating_from(len),
            name,
        );
    }
    match field().flat() {
        Ok(j) => Ok(Column::List {
            offsets: j.offsets,
            values: Box::new(field_column(name, j.values)?),
        }),
        Err(Error::Unsupported(_)) => field_column(name, field().values()?),
        Err(e) => Err(e),
    }
}

/// An RNTuple field's values as a column.
fn field_column(name: &str, values: FieldValues) -> Result<Column> {
    Ok(match values {
        FieldValues::Bool(v) => Column::Bool(v),
        FieldValues::I8(v) => Column::I8(v),
        FieldValues::U8(v) => Column::U8(v),
        FieldValues::I16(v) => Column::I16(v),
        FieldValues::U16(v) => Column::U16(v),
        FieldValues::I32(v) => Column::I32(v),
        FieldValues::U32(v) => Column::U32(v),
        FieldValues::I64(v) => Column::I64(v),
        FieldValues::U64(v) => Column::U64(v),
        FieldValues::F32(v) => Column::F32(v),
        FieldValues::F64(v) => Column::F64(v),
        FieldValues::Str(v) => v.into_column(),
        FieldValues::VecBool(v) => v.into_column(),
        FieldValues::VecI8(v) => v.into_column(),
        FieldValues::VecU8(v) => v.into_column(),
        FieldValues::VecI16(v) => v.into_column(),
        FieldValues::VecU16(v) => v.into_column(),
        FieldValues::VecI32(v) => v.into_column(),
        FieldValues::VecU32(v) => v.into_column(),
        FieldValues::VecI64(v) => v.into_column(),
        FieldValues::VecU64(v) => v.into_column(),
        FieldValues::VecF32(v) => v.into_column(),
        FieldValues::VecF64(v) => v.into_column(),
        FieldValues::VecStr(v) => v.into_column(),
        FieldValues::Nested { offsets, items } => Column::List {
            offsets,
            values: Box::new(field_column(name, *items)?),
        },
        FieldValues::Opt { present, values } => Column::Optional {
            valid: present,
            values: Box::new(field_column(name, *values)?),
        },
        FieldValues::Record(members) => Column::Struct(
            members
                .into_iter()
                .map(|(n, v)| Ok((n, field_column(name, v)?)))
                .collect::<Result<_>>()?,
        ),
        other => {
            return Err(Error::Unsupported(format!(
                "field {name:?} is a {}, which a batch cannot hold; read it with \
                 Dataset::as_ntuple",
                other.type_name()
            )));
        }
    })
}
