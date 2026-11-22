//! [`Dataset`]: one reader for a `TTree` or an `RNTuple`, yielding
//! [`Batch`]es.

use std::num::NonZeroU64;
use std::ops::Range;

use oxiroot_io_core::{ClassKind, Error, FileReader, Result};
use oxiroot_rntuple::{ANCHOR_CLASS, NtupleReader};
use oxiroot_tree::{BranchLayout, TreeReader};

use crate::Column;
use crate::DataType;
use crate::batch::Batch;
use crate::schema::Schema;

/// Which of ROOT's two columnar formats a dataset is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// A `TTree`.
    Tree,
    /// An `RNTuple`.
    Ntuple,
}

/// A `TTree` or an `RNTuple`, read as [`Batch`]es of flat columns. The format
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
    /// Several datasets read one after the other; never empty.
    Chain(Vec<Dataset<'f>>),
}

impl<'f> Dataset<'f> {
    /// Open the `TTree` or `RNTuple` at `path` in `file`: `"events"`, or
    /// `"dir/sub/events;cycle"` for one in a subdirectory or at a given cycle.
    ///
    /// # Errors
    ///
    /// No such key, a key that is neither a `TTree` nor an `RNTuple`, or a
    /// malformed one.
    pub fn open(file: &'f FileReader, path: &str) -> Result<Self> {
        let class = file.key(path)?.class_name;
        Self::open_class(
            &class,
            path,
            || TreeReader::open(file, path),
            || NtupleReader::open(file, path),
        )
    }

    fn open_class(
        class: &str,
        name: &str,
        tree: impl FnOnce() -> Result<TreeReader<'f>>,
        ntuple: impl FnOnce() -> Result<NtupleReader<'f>>,
    ) -> Result<Self> {
        let source = match ClassKind::of(class) {
            ClassKind::Tree => Source::Tree(tree()?),
            ClassKind::Ntuple => Source::Ntuple(ntuple()?),
            _ => {
                return Err(Error::WrongClass {
                    name: Some(name.to_string()),
                    found: class.to_string(),
                    expected: format!("TTree or {ANCHOR_CLASS}"),
                });
            }
        };
        Ok(Dataset { source })
    }

    /// Several datasets with the same columns, read as one: their entries one
    /// after the other, in the order given, the way ROOT's `TChain` spans a
    /// dataset split over many files. The parts can be trees, `RNTuple`s, or
    /// both.
    ///
    /// ```no_run
    /// use oxiroot_dataset::Dataset;
    /// use oxiroot_io_core::FileReader;
    ///
    /// let (a, b) = (FileReader::open("a.root")?, FileReader::open("b.root")?);
    /// let events = Dataset::chain([Dataset::open(&a, "events")?, Dataset::open(&b, "events")?])?;
    /// let pt = events.read(["pt"])?; // the entries of both files
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// No parts, or a part whose columns are not the first's.
    pub fn chain(parts: impl IntoIterator<Item = Self>) -> Result<Self> {
        let parts: Vec<Self> = parts.into_iter().collect();
        let first = parts
            .first()
            .ok_or_else(|| Error::InvalidInput("a chain needs at least one dataset".into()))?;
        for (i, part) in parts.iter().enumerate().skip(1) {
            if part.columns() != first.columns() {
                return Err(Error::SchemaChanged {
                    detail: format!(
                        "dataset {i} of the chain has the columns {:?}, not the first's {:?}",
                        part.columns(),
                        first.columns()
                    ),
                });
            }
        }
        Ok(Self {
            source: Source::Chain(parts),
        })
    }

    /// The format the dataset is stored in; for a [`chain`](Self::chain), its
    /// first part's.
    #[must_use]
    pub fn format(&self) -> Format {
        match &self.source {
            Source::Tree(_) => Format::Tree,
            Source::Ntuple(_) => Format::Ntuple,
            Source::Chain(parts) => parts[0].format(),
        }
    }

    /// The number of entries.
    #[must_use]
    pub fn entries(&self) -> u64 {
        match &self.source {
            Source::Tree(t) => t.entries(),
            Source::Ntuple(n) => n.entries(),
            Source::Chain(parts) => parts.iter().map(Self::entries).sum(),
        }
    }

    /// The column names: a tree's branches, an `RNTuple`'s top-level fields.
    /// A tree's split `std::vector<Struct>` is one column (`hits`), read as a
    /// list of structs, as in an `RNTuple`; its members (`hits.x`) can still be
    /// read on their own.
    #[must_use]
    pub fn columns(&self) -> Vec<&str> {
        match &self.source {
            Source::Tree(t) => {
                let mut out: Vec<&str> = Vec::new();
                for name in t.branch_names() {
                    match t.branch_collection(name) {
                        Some(c) if out.last() == Some(&c) => {}
                        Some(c) => out.push(c),
                        None => out.push(name),
                    }
                }
                out
            }
            Source::Ntuple(n) => n.field_names().collect::<Vec<_>>(),
            Source::Chain(parts) => parts[0].columns(),
        }
    }

    /// The `TTree` reader underneath, for what only a tree has (friends,
    /// indices, split objects).
    #[must_use]
    pub fn as_tree(&self) -> Option<&TreeReader<'f>> {
        match &self.source {
            Source::Tree(t) => Some(t),
            Source::Ntuple(_) | Source::Chain(_) => None,
        }
    }

    /// The `RNTuple` reader underneath, for what only an `RNTuple` has (the
    /// schema, the clusters, the raw columns).
    #[must_use]
    pub fn as_ntuple(&self) -> Option<&NtupleReader<'f>> {
        match &self.source {
            Source::Ntuple(n) => Some(n),
            Source::Tree(_) | Source::Chain(_) => None,
        }
    }

    /// How each column is stored in the file: `std::vector` or C-style array,
    /// plain or half-precision float, `std::set` or `std::map`. Hand it to
    /// [`DatasetWriter::with_schema`](crate::DatasetWriter::with_schema) to write the
    /// same types back.
    ///
    /// # Errors
    ///
    /// A read error.
    pub fn schema(&self) -> Result<Schema> {
        if let Source::Chain(parts) = &self.source {
            return parts[0].schema();
        }
        let names: Vec<String> = self.columns().into_iter().map(str::to_string).collect();
        // The shapes are those of an empty read; the file's metadata says how
        // each one is stored.
        let shapes = self.read_window(&names, 0..0, false)?;
        Ok(shapes
            .iter()
            .map(|(name, column)| {
                let shape = column.data_type();
                let stored = match &self.source {
                    Source::Tree(t) => tree_type(t, name, shape),
                    Source::Ntuple(n) => n.header().stored_type(name, shape),
                    Source::Chain(_) => unreachable!("a chain returned above"),
                };
                (name, stored)
            })
            .collect())
    }

    /// Every entry of `columns`, as one batch.
    ///
    /// # Errors
    ///
    /// A column that does not exist, or a read error.
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
    /// one cluster (an `RNTuple` cluster, or the entries where every selected
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
            step: None,
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
            Source::Chain(parts) => {
                // Each part's own cuts, shifted to where the part starts.
                let mut cuts = vec![0];
                let mut start = 0;
                for part in parts {
                    cuts.extend(part.cut_points(names)?.into_iter().map(|c| start + c));
                    start += part.entries();
                }
                cuts.dedup();
                Ok(cuts)
            }
            Source::Ntuple(n) => Ok(n.cluster_boundaries()),
            Source::Tree(t) => {
                let mut common: Option<Vec<u64>> = None;
                // A split collection's baskets are its members'.
                let members: Vec<&str> = names
                    .iter()
                    .flat_map(|n| {
                        let m = t.collection_members(n).collect::<Vec<_>>();
                        if m.is_empty() { vec![n.as_str()] } else { m }
                    })
                    .collect();
                for name in members {
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
        if let Source::Chain(parts) = &self.source {
            // The entries of each part the range covers, one after the other.
            let mut out: Option<Batch> = None;
            let mut start = 0;
            for part in parts {
                let end = start + part.entries();
                let (from, to) = (range.start.max(start), range.end.min(end));
                if from < to {
                    let batch = part.read_window(names, from - start..to - start, parallel)?;
                    match &mut out {
                        None => out = Some(batch),
                        Some(all) => all.append(batch)?,
                    }
                }
                start = end;
            }
            return match out {
                Some(batch) => Ok(batch),
                // No entries: the columns alone, as the first part has them.
                None => parts[0].read_window(names, 0..0, parallel),
            };
        }
        let read = |name: &String| -> Result<Column> {
            match &self.source {
                Source::Tree(t) => tree_column(t, name, range.clone(), parallel),
                Source::Ntuple(n) => ntuple_column(n, name, range.clone(), parallel),
                Source::Chain(_) => unreachable!("a chain returned above"),
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
    /// The entries asked for per batch; `None` is one cluster per batch.
    step: Option<NonZeroU64>,
    range: Range<u64>,
    parallel: bool,
    cuts: Option<Vec<u64>>,
    next: Option<u64>,
}

impl Batches<'_, '_> {
    /// Aim for `entries` entries per batch. A batch still ends where a cluster
    /// (or aligned basket) does, the first such boundary at or past
    /// `entries` — unless that is more than twice as far, where it is cut at
    /// `entries` exactly. Zero asks for nothing: a batch stays one cluster.
    pub fn step(mut self, entries: u64) -> Self {
        self.step = NonZeroU64::new(entries);
        self
    }

    /// Only entries `range` (clamped to the entry count).
    pub fn range(mut self, range: Range<u64>) -> Self {
        let end = range.end.min(self.dataset.entries());
        self.range = range.start.min(end)..end;
        self
    }

    /// Read a batch's columns concurrently, on rayon's global thread pool.
    /// Without the `rayon` feature there is no pool, and the read stays on
    /// the calling thread whatever is asked.
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// The end of the batch starting at `start`.
    fn batch_end(&self, cuts: &[u64], start: u64) -> u64 {
        let end = self.range.end;
        let stop = match self.step.map(NonZeroU64::get) {
            // The next cluster boundary, or the end past the last one.
            None => cuts.iter().copied().find(|&c| c > start).unwrap_or(end),
            Some(step) => {
                let target = start.saturating_add(step);
                match cuts.iter().copied().find(|&c| c >= target) {
                    Some(c) if c - start <= 2 * step => c,
                    _ => target,
                }
            }
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
        let stop = self.batch_end(self.cuts.as_deref().unwrap_or_default(), start);
        self.next = Some(stop);
        Some(
            self.dataset
                .read_window(&self.columns, start..stop, self.parallel),
        )
    }
}

/// Read one tree column's window: a branch as the reader gives it, or the
/// members of a split `std::vector<Struct>` joined into a list of structs.
fn tree_column(
    t: &TreeReader<'_>,
    name: &str,
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    if t.branch_layout(name).is_some() {
        return t.branch(name).range(range).parallel(parallel).values();
    }
    let members = t.collection_members(name).collect::<Vec<_>>();
    if members.is_empty() {
        return Err(Error::NotFound {
            what: "column",
            name: name.to_string(),
        });
    }
    tree_collection(t, name, &members, range, parallel)
}

/// A split `std::vector<Struct>` collection as a list of structs: each
/// member is a list, and they share the first member's offsets.
fn tree_collection(
    t: &TreeReader<'_>,
    name: &str,
    members: &[&str],
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    let lists = members.iter().map(|&member| {
        let short = member
            .strip_prefix(name)
            .and_then(|m| m.strip_prefix('.'))
            .unwrap_or(member);
        let read = t.branch(member).range(range.clone()).parallel(parallel);
        Ok((short, read.values()?))
    });
    let lists = lists.collect::<Result<Vec<_>>>()?;
    Column::list_of_structs(lists).map_err(|e| Error::Format(format!("column {name:?}: {e}")))
}

/// Read one `RNTuple` field's window.
fn ntuple_column(
    n: &NtupleReader<'_>,
    name: &str,
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    if n.header().top_level_field(name).is_none() {
        return Err(Error::NotFound {
            what: "column",
            name: name.to_string(),
        });
    }
    n.field(name).range(range).parallel(parallel).values()
}

/// A tree column's stored type, from its in-memory `shape`: a list counted by
/// another leaf is a C-style array.
fn tree_type(t: &TreeReader<'_>, name: &str, shape: DataType) -> DataType {
    match (t.branch_layout(name), shape) {
        (Some(BranchLayout::CArray), DataType::List(item)) => DataType::CArray(item),
        (_, shape) => shape,
    }
}
