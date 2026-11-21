//! [`Dataset`]: one reader for a `TTree` or an `RNTuple`, yielding
//! [`Batch`]es.

use std::ops::Range;

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{ClassKind, Error, FileReader, Result};
use oxiroot_rntuple::raw::FieldValues;
use oxiroot_rntuple::{ANCHOR_CLASS, ColumnType, Header, NtupleReader};
use oxiroot_tree::raw::BranchValues;
use oxiroot_tree::{BranchLayout, LeafType, TreeReader};

use crate::batch::Batch;
use crate::column::{Column, IntoColumn};
use crate::schema::{DataType, Schema};

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
            Source::Ntuple(n) => n.field_names(),
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

    /// The `RNTuple` reader underneath, for what only an `RNTuple` has (records,
    /// variants, the raw columns).
    #[must_use]
    pub fn as_ntuple(&self) -> Option<&NtupleReader<'f>> {
        match &self.source {
            Source::Ntuple(n) => Some(n),
            Source::Tree(_) | Source::Chain(_) => None,
        }
    }

    /// How each column is stored in the file: `std::vector` or C-style array,
    /// plain or half-precision float, `std::set` or `std::map`. Hand it to
    /// [`DatasetWriter::schema`](crate::DatasetWriter::schema) to write the
    /// same types back.
    ///
    /// # Errors
    ///
    /// A column a [`Batch`] cannot hold (an `RNTuple` variant, a tree branch of
    /// an unsupported class), or a read error.
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
                    Source::Ntuple(n) => ntuple_type(n.header(), name, shape),
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
    /// A column that does not exist, or one of a shape a [`Batch`] cannot
    /// hold (an `RNTuple` record or variant); read those through
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
                        let m = t.collection_members(n);
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
    let Some(layout) = t.branch_layout(name) else {
        let members = t.collection_members(name);
        if members.is_empty() {
            return Err(Error::NotFound {
                what: "column",
                name: name.to_string(),
            });
        }
        return tree_collection(t, name, &members, range, parallel);
    };
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
            // Only "no flat form" falls back: a read error is the caller's.
            Err(Error::Unsupported(_)) => array_of(branch_column(branch().values()?)?, len, name),
            Err(e) => Err(e),
        },
        BranchLayout::List | BranchLayout::CArray if !is_str => match branch().flat() {
            Ok(j) => Ok(Column::List {
                offsets: j.offsets,
                values: Box::new(branch_column(j.values)?),
            }),
            Err(Error::Unsupported(_)) => branch_column(branch().values()?),
            Err(e) => Err(e),
        },
        _ => branch_column(branch().values()?),
    }
}

/// A split `std::vector<Struct>` collection as a list of structs: each
/// member read flat, sharing the first member's offsets.
fn tree_collection(
    t: &TreeReader<'_>,
    name: &str,
    members: &[&str],
    range: Range<u64>,
    parallel: bool,
) -> Result<Column> {
    let mut offsets: Option<Vec<u64>> = None;
    let mut fields = Vec::with_capacity(members.len());
    for &member in members {
        #[cfg(feature = "rayon")]
        let read = t.branch(member).range(range.clone()).parallel(parallel);
        #[cfg(not(feature = "rayon"))]
        let read = {
            let _ = parallel;
            t.branch(member).range(range.clone())
        };
        let j = read.flat()?;
        match &offsets {
            None => offsets = Some(j.offsets),
            Some(o) if *o == j.offsets => {}
            Some(_) => {
                return Err(Error::Format(format!(
                    "column {name:?}: member {member:?} has other entry lengths than the first"
                )));
            }
        }
        let short = member
            .strip_prefix(name)
            .and_then(|m| m.strip_prefix('.'))
            .unwrap_or(member);
        fields.push((short.to_string(), branch_column(j.values)?));
    }
    Ok(Column::List {
        offsets: offsets.unwrap_or_else(|| vec![0]),
        values: Box::new(Column::Struct(fields)),
    })
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

/// Read one `RNTuple` field's window as a flat column.
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

/// An `RNTuple` field's values as a column.
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

/// A tree column's stored type, from its in-memory `shape`: a list counted by
/// another leaf is a C-style array.
fn tree_type(t: &TreeReader<'_>, name: &str, shape: DataType) -> DataType {
    match (t.branch_layout(name), shape) {
        (Some(BranchLayout::CArray), DataType::List(item)) => DataType::CArray(item),
        (_, shape) => shape,
    }
}

/// An `RNTuple` column's stored type, from its in-memory `shape` and the
/// header's description of the top-level field `name`.
fn ntuple_type(header: &Header, name: &str, shape: DataType) -> DataType {
    let top = header
        .fields
        .iter()
        .enumerate()
        .position(|(i, f)| f.name == name && f.parent_field_id as usize == i);
    match top {
        Some(id) => field_type(header, id, shape),
        None => shape,
    }
}

/// The fields directly inside field `id`, in order.
fn child_fields(header: &Header, id: usize) -> Vec<usize> {
    header
        .fields
        .iter()
        .enumerate()
        .filter(|&(i, f)| i != id && f.parent_field_id as usize == id)
        .map(|(i, _)| i)
        .collect()
}

/// Whether the C++ type `name` is the template `template` (`std::set`) or its
/// unordered or multi form.
fn is_template(name: &str, template: &str) -> bool {
    let base = name.split('<').next().unwrap_or(name);
    let base = base.strip_prefix("std::").unwrap_or(base);
    [
        template.to_string(),
        format!("unordered_{template}"),
        format!("multi{template}"),
        format!("unordered_multi{template}"),
    ]
    .iter()
    .any(|t| t == base)
}

/// Refine `shape` with how field `id` and the fields inside it are stored.
/// Where the header and the shape do not line up, the shape is kept as it is.
fn field_type(header: &Header, id: usize, shape: DataType) -> DataType {
    let field = &header.fields[id];
    let kids = child_fields(header, id);
    let type_name = field.type_name.as_str();
    let inner = |kid: Option<&usize>, shape: DataType| match kid {
        Some(&kid) => field_type(header, kid, shape),
        None => shape,
    };
    if is_template(type_name, "atomic") && kids.len() == 1 {
        return DataType::Atomic(Box::new(field_type(header, kids[0], shape)));
    }
    match shape {
        DataType::F32 => {
            let column = header.columns.iter().find(|c| c.field_id as usize == id);
            match column.map(|c| (c.column_type, c.bits_on_storage, c.value_range)) {
                Some((ColumnType::Real16 | ColumnType::SplitReal16, ..)) => DataType::Half,
                Some((ColumnType::Real32Trunc, bits, _)) => DataType::Truncated { bits },
                Some((ColumnType::Real32Quant, bits, Some((min, max)))) => {
                    DataType::Quantized { bits, min, max }
                }
                _ => DataType::F32,
            }
        }
        DataType::List(item) if is_template(type_name, "set") => {
            DataType::Set(Box::new(inner(kids.first(), *item)))
        }
        DataType::List(item) if is_template(type_name, "map") => {
            // A map is a collection of `std::pair<K, V>` records.
            let pair = kids.first().map(|&k| child_fields(header, k));
            match (*item, pair.as_deref()) {
                (DataType::Struct(members), Some([key, value])) if members.len() == 2 => {
                    let mut members = members.into_iter().map(|(_, t)| t);
                    let (k, v) = (members.next(), members.next());
                    match (k, v) {
                        (Some(k), Some(v)) => DataType::Map(
                            Box::new(field_type(header, *key, k)),
                            Box::new(field_type(header, *value, v)),
                        ),
                        _ => unreachable!("two members were counted"),
                    }
                }
                (item, _) => DataType::List(Box::new(item)),
            }
        }
        DataType::List(item) => DataType::List(Box::new(inner(kids.first(), *item))),
        DataType::Array(len, item)
            if type_name.starts_with("std::bitset<") && *item == DataType::Bool =>
        {
            DataType::Bitset(len)
        }
        DataType::Array(len, item) => DataType::Array(len, Box::new(inner(kids.first(), *item))),
        DataType::Optional(item) if type_name.starts_with("std::unique_ptr<") => {
            DataType::UniquePtr(Box::new(inner(kids.first(), *item)))
        }
        DataType::Optional(item) => DataType::Optional(Box::new(inner(kids.first(), *item))),
        DataType::Struct(members) if members.len() == kids.len() => DataType::Struct(
            members
                .into_iter()
                .zip(&kids)
                .map(|((name, t), &kid)| (name, field_type(header, kid, t)))
                .collect(),
        ),
        other => other,
    }
}
