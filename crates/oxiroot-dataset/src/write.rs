//! [`DatasetWriter`]: one writer for a `TTree` or an `RNTuple`, taking
//! [`Batch`]es; and [`DatasetObject`], a dataset placed in a
//! [`FileWriter`](oxiroot_io_core::FileWriter) next to other objects.

use std::fs::File;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{ContainerWriter, DirId};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Compression, Error, Result, WriteInto};
use oxiroot_rntuple::ANCHOR_CLASS;
use oxiroot_rntuple::raw::{Column as RnColumn, Field, NtupleWriter};
use oxiroot_tree::raw::{Branch, SplitMember, TreeWriter};

use crate::batch::Batch;
use crate::column::Column;
use crate::read::Format;
use crate::schema::{DataType, Schema};

/// The uncompressed size at which a writer closes a cluster, unless set.
const DEFAULT_CLUSTER_BYTES: usize = 32 * 1024 * 1024;

/// About how many bytes of one column's values go in one basket or page.
const CHUNK_BYTES_PER_COLUMN: usize = 1024 * 1024;

/// Entry ranges cutting `batch` into chunks of about
/// [`CHUNK_BYTES_PER_COLUMN`] per column.
fn chunk_ranges(batch: &Batch) -> Vec<std::ops::Range<usize>> {
    let n = batch.len();
    let columns = batch.names().len().max(1);
    let per_entry = batch.byte_size().div_ceil(n.max(1)).max(1);
    let step = (CHUNK_BYTES_PER_COLUMN * columns / per_entry).max(1);
    (0..n).step_by(step).map(|a| a..(a + step).min(n)).collect()
}

/// Apply `f` to every chunk, in parallel with the `rayon` feature.
fn map_chunks<T: Send>(
    chunks: &[std::ops::Range<usize>],
    f: impl Fn(&std::ops::Range<usize>) -> Result<T> + Sync + Send,
) -> Result<Vec<T>> {
    #[cfg(feature = "rayon")]
    {
        use rayon::prelude::*;
        chunks.par_iter().map(f).collect()
    }
    #[cfg(not(feature = "rayon"))]
    {
        chunks.iter().map(f).collect()
    }
}

/// Writes [`Batch`]es as a `TTree` or an `RNTuple` — the format is the one
/// argument that differs.
///
/// Batches are buffered and written a cluster at a time, once about
/// [`cluster_bytes`](Self::cluster_bytes) of values (32 MiB by default) have
/// accumulated, however the caller sizes its batches. The first batch fixes
/// the schema: every later one must have the same columns, of the same types.
///
/// ```no_run
/// use oxiroot_dataset::{Batch, DatasetWriter, Format};
///
/// let batch = Batch::new()
///     .column("mass", vec![91.2_f64, 125.1])?
///     .column("jet_pt", vec![vec![40.0_f32, 31.5], vec![]])?;
/// let mut out = DatasetWriter::create("out.root", "events", Format::Tree)?;
/// out.write(batch)?;
/// out.finish()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
#[must_use = "a DatasetWriter writes its last entries, and the file's directory, in `finish`"]
pub struct DatasetWriter<W: Write + Seek = File> {
    sink: Sink<W>,
    name: String,
    format: Format,
    compression: Compression,
    large: bool,
    cluster_bytes: usize,
    /// How each column is stored: given, or inferred from the first batch.
    schema: Option<Schema>,
    /// The first batch's columns with no entries: what an empty dataset is
    /// written from.
    empty: Option<Batch>,
    buffer: Option<Batch>,
    entries: u64,
}

enum Sink<W: Write + Seek> {
    /// Not opened yet: options can still change.
    Pending {
        sink: W,
        file_name: String,
    },
    Tree(TreeWriter<W>),
    Ntuple(NtupleWriter<W>),
    /// Taken while being opened.
    Gone,
}

impl DatasetWriter<File> {
    /// Create the file `path` (truncating it) to hold the dataset `name` in
    /// `format`, compressed with Zstd level 5 unless
    /// [`compression`](Self::compression) says otherwise.
    ///
    /// # Errors
    ///
    /// The file cannot be created.
    pub fn create(path: impl AsRef<Path>, name: &str, format: Format) -> Result<Self> {
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.root")
            .to_string();
        Ok(Self::new(File::create(path)?, &file_name, name, format))
    }
}

impl<W: Write + Seek> DatasetWriter<W> {
    /// Write the dataset `name` in `format` to `sink`; `file_name` is the name
    /// recorded in the file's header.
    pub fn new(sink: W, file_name: &str, name: &str, format: Format) -> Self {
        Self {
            sink: Sink::Pending {
                sink,
                file_name: file_name.to_string(),
            },
            name: name.to_string(),
            format,
            compression: Compression::default(),
            large: false,
            cluster_bytes: DEFAULT_CLUSTER_BYTES,
            schema: None,
            empty: None,
            buffer: None,
            entries: 0,
        }
    }

    /// Compress with `compression`. Set it before the first write: the file is
    /// opened with the first cluster.
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Close a cluster once about `bytes` of uncompressed values are buffered.
    pub fn cluster_bytes(mut self, bytes: usize) -> Self {
        self.cluster_bytes = bytes.max(1);
        self
    }

    /// Store the columns as `schema` says, where the data alone does not: an
    /// `f32` column in half precision, a list as a `std::set` or a C-style
    /// array. Without it each column is stored as its
    /// [`data_type`](Column::data_type). Set it before the first write; the
    /// first batch is checked against it.
    ///
    /// ```no_run
    /// use oxiroot_dataset::{Batch, DataType, DatasetWriter, Format};
    ///
    /// let batch = Batch::new().column("pt", vec![41.5_f32, 37.2])?;
    /// let mut out = DatasetWriter::create("out.root", "events", Format::Ntuple)?
    ///     .schema(batch.schema().with("pt", DataType::Half));
    /// out.write(batch)?;
    /// out.finish()?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    pub fn schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Use ROOT's 64-bit file layout, needed past 2 GiB. Set it before the
    /// first write.
    pub const fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// The entries written so far, buffered ones included.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Add `batch`'s entries.
    ///
    /// # Errors
    ///
    /// On the first batch, a column the format cannot hold or one that does
    /// not fit the [`schema`](Self::schema) (nothing is written then); later,
    /// a batch whose columns differ from the first's; or a write error when a
    /// cluster is flushed.
    pub fn write(&mut self, batch: Batch) -> Result<()> {
        if self.empty.is_none() {
            let schema = self.schema.take().unwrap_or_else(|| batch.schema());
            check_batch(&schema, &batch)?;
            for (name, data_type) in schema.iter() {
                check_type(self.format, name, data_type)?;
            }
            self.schema = Some(schema);
            self.empty = Some(batch.slice(0..0));
        } else if let Some(schema) = &self.schema {
            check_batch(schema, &batch)?;
        }
        if let (Format::Tree, Some(schema)) = (self.format, &self.schema) {
            check_counts(&batch, schema)?;
        }
        if batch.is_empty() {
            return Ok(());
        }
        self.entries += batch.len() as u64;
        match &mut self.buffer {
            None if batch.byte_size() >= self.cluster_bytes => return self.flush_batch(&batch),
            None => self.buffer = Some(batch),
            Some(buffer) => buffer.append(batch)?,
        }
        if self
            .buffer
            .as_ref()
            .is_some_and(|b| b.byte_size() >= self.cluster_bytes)
        {
            self.flush()?;
        }
        Ok(())
    }

    /// Write the buffered entries as a cluster.
    fn flush(&mut self) -> Result<()> {
        if let Some(buffer) = self.buffer.take() {
            self.flush_batch(&buffer)?;
        }
        Ok(())
    }

    /// Write `batch` as one cluster, cut into chunks of about
    /// [`CHUNK_BYTES_PER_COLUMN`] per column: a `TTree` basket per branch per
    /// chunk, or an `RNTuple` cluster per chunk. Small chunks keep window reads
    /// cheap and let reads and writes run on every core.
    fn flush_batch(&mut self, batch: &Batch) -> Result<()> {
        self.open()?;
        let chunks = chunk_ranges(batch);
        let schema = self.schema.clone().unwrap_or_else(|| batch.schema());
        let convert_tree =
            |r: &std::ops::Range<usize>| tree_branches(&batch.slice(r.clone()), &schema);
        let convert_ntuple =
            |r: &std::ops::Range<usize>| Ok(ntuple_fields(batch.slice(r.clone()), &schema));
        match &mut self.sink {
            Sink::Tree(w) => w.write_batches(&map_chunks(&chunks, convert_tree)?),
            Sink::Ntuple(w) => w.write_batches(&map_chunks(&chunks, convert_ntuple)?),
            Sink::Pending { .. } | Sink::Gone => unreachable!("opened above"),
        }
    }

    fn open(&mut self) -> Result<()> {
        if !matches!(self.sink, Sink::Pending { .. }) {
            return Ok(());
        }
        let Sink::Pending { sink, file_name } = std::mem::replace(&mut self.sink, Sink::Gone)
        else {
            unreachable!("checked above")
        };
        let (c, name) = (self.compression, self.name.as_str());
        self.sink = match (self.format, self.large) {
            (Format::Tree, false) => Sink::Tree(TreeWriter::new(sink, &file_name, name, c)?),
            (Format::Tree, true) => Sink::Tree(TreeWriter::new_large(sink, &file_name, name, c)?),
            (Format::Ntuple, false) => Sink::Ntuple(NtupleWriter::new(sink, &file_name, name, c)?),
            (Format::Ntuple, true) => {
                Sink::Ntuple(NtupleWriter::new_large(sink, &file_name, name, c)?)
            }
        };
        Ok(())
    }

    /// Write what is buffered and close the file. Returns the sink. A writer
    /// that was only given empty batches writes a dataset with their columns
    /// and no entries.
    ///
    /// # Errors
    ///
    /// No batch at all was written (there are no columns), or a write error.
    pub fn finish(mut self) -> Result<W> {
        self.flush()?;
        // Columns but no entries: write the schema alone.
        if let (Sink::Pending { .. }, Some(empty)) = (&self.sink, self.empty.take()) {
            self.open()?;
            let schema = self.schema.clone().unwrap_or_else(|| empty.schema());
            match &mut self.sink {
                Sink::Tree(w) => w.write_batch(&tree_branches(&empty, &schema)?)?,
                Sink::Ntuple(w) => w.write_batch(&ntuple_fields(empty, &schema))?,
                Sink::Pending { .. } | Sink::Gone => unreachable!("opened above"),
            }
        }
        match self.sink {
            Sink::Tree(w) => w.finish(),
            Sink::Ntuple(w) => w.finish(),
            Sink::Pending { .. } | Sink::Gone => Err(Error::InvalidInput(format!(
                "dataset {:?}: no entries were written",
                self.name
            ))),
        }
    }
}

impl<W: Write + Seek> std::fmt::Debug for DatasetWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatasetWriter")
            .field("name", &self.name)
            .field("format", &self.format)
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

/// A dataset to place in a [`FileWriter`](oxiroot_io_core::FileWriter) with
/// `add`, next to histograms or other datasets, in one cluster.
///
/// ```no_run
/// use oxiroot_dataset::{Batch, DatasetObject, Format};
/// use oxiroot_io_core::FileWriter;
///
/// let batch = Batch::new().column("energy", vec![10.5_f64, 20.1])?;
/// FileWriter::create("run.root")
///     .add(DatasetObject::new("events", Format::Tree, batch.clone()))
///     .add(DatasetObject::new("events_rn", Format::Ntuple, batch))
///     .write()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
///
/// A column the format cannot hold, or one that does not fit the
/// [`schema`](Self::schema), is reported when the file is written.
#[derive(Debug)]
#[must_use = "a DatasetObject does nothing until added to a FileWriter"]
pub struct DatasetObject {
    name: String,
    format: Format,
    batch: Batch,
    schema: Option<Schema>,
}

impl DatasetObject {
    /// `batch` as the dataset `name` in `format`.
    pub fn new(name: &str, format: Format, batch: Batch) -> Self {
        Self {
            name: name.to_string(),
            format,
            batch,
            schema: None,
        }
    }

    /// Store the columns as `schema` says; see [`DatasetWriter::schema`].
    pub fn schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// The batch as the format's own records.
    fn content(self) -> Result<Content> {
        let schema = self.schema.unwrap_or_else(|| self.batch.schema());
        check_batch(&schema, &self.batch)?;
        for (column, data_type) in schema.iter() {
            check_type(self.format, column, data_type)?;
        }
        Ok(match self.format {
            Format::Tree => {
                check_counts(&self.batch, &schema)?;
                Content::Tree(tree_branches(&self.batch, &schema)?)
            }
            Format::Ntuple => Content::Ntuple(ntuple_fields(self.batch, &schema)),
        })
    }
}

enum Content {
    Tree(Vec<Branch>),
    Ntuple(Vec<Field>),
}

/// A [`DatasetObject`] converted for its format, or why it could not be.
struct Records {
    name: String,
    format: Format,
    content: Result<Content>,
}

impl oxiroot_io_core::Storable for DatasetObject {
    fn into_stored(self) -> oxiroot_io_core::Stored {
        oxiroot_io_core::Stored::records(Records {
            name: self.name.clone(),
            format: self.format,
            content: self.content(),
        })
    }
}

impl WriteInto for Records {
    fn root_class(&self) -> String {
        match self.format {
            Format::Tree => "TTree".to_string(),
            Format::Ntuple => ANCHOR_CLASS.to_string(),
        }
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        match &self.content {
            Ok(Content::Tree(b)) => oxiroot_tree::raw::write_tree(file, dir, &self.name, b, 0),
            Ok(Content::Ntuple(f)) => {
                oxiroot_rntuple::raw::write_ntuple(file, dir, &self.name, f, &[])
            }
            Err(e) => Err(e.clone()),
        }
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        match &self.content {
            Ok(Content::Tree(b)) => oxiroot_tree::raw::tree_streamer_classes(b),
            Ok(Content::Ntuple(f)) => oxiroot_rntuple::raw::ntuple_streamer_classes(f),
            Err(_) => Vec::new(),
        }
    }
}

/// Check that `batch` holds exactly `schema`'s columns, each of the shape its
/// stored type takes in memory.
fn check_batch(schema: &Schema, batch: &Batch) -> Result<()> {
    let mismatch = |detail: String| Err(Error::SchemaChanged { detail });
    if !schema.iter().map(|(n, _)| n).eq(batch.names()) {
        return mismatch(format!(
            "the batch's columns {:?} are not the schema's {:?}",
            batch.names().collect::<Vec<_>>(),
            schema.iter().map(|(n, _)| n).collect::<Vec<_>>()
        ));
    }
    for ((name, data_type), (_, column)) in schema.iter().zip(batch.iter()) {
        let held = column.data_type();
        if data_type.storage() != held {
            return mismatch(format!(
                "column {name:?} holds {held}, which is not what {data_type} is stored from \
                 ({})",
                data_type.storage()
            ));
        }
    }
    Ok(())
}

/// Refuse a type `format` cannot hold, naming the column and saying why.
fn check_type(format: Format, name: &str, data_type: &DataType) -> Result<()> {
    let refusal = match format {
        Format::Tree => tree_refusal(data_type),
        Format::Ntuple => ntuple_refusal(data_type),
    };
    match refusal {
        None => Ok(()),
        Some(why) => {
            let what = match format {
                Format::Tree => "a TTree",
                Format::Ntuple => "an RNTuple",
            };
            Err(Error::Unsupported(format!(
                "column {name:?} is {data_type}, which {what} written by oxiroot cannot hold{why}"
            )))
        }
    }
}

/// Why a tree cannot hold `t` (the text after "cannot hold"), or `None` if it
/// can.
fn tree_refusal(t: &DataType) -> Option<&'static str> {
    let plain = DataType::is_plain_scalar;
    let ok = match t {
        DataType::String => true,
        DataType::List(item) => match &**item {
            DataType::String => true,
            // A split `std::vector<Struct>` of number members.
            DataType::Struct(members) => {
                !members.is_empty() && members.iter().all(|(_, m)| plain(m))
            }
            DataType::List(inner) => plain(inner),
            other => plain(other),
        },
        DataType::CArray(item) | DataType::Array(_, item) => plain(item),
        other => plain(other),
    };
    if ok {
        None
    } else if matches!(t, DataType::Struct(_)) {
        Some("; a TTree holds structs in a list, as a split std::vector<Struct>")
    } else {
        Some("")
    }
}

/// Why an `RNTuple` cannot hold `t`, or `None` if it can.
fn ntuple_refusal(t: &DataType) -> Option<&'static str> {
    // A set or map names its element type; a user class has no name here.
    let named =
        |t: &DataType| matches!(t, DataType::Struct(m) if !positional(m.iter().map(|(n, _)| n)));
    match t {
        DataType::CArray(_) => Some("; a counted C-style array is a TTree type"),
        DataType::Truncated { bits } if !(10..=31).contains(bits) => {
            Some("; a truncated float takes 10 to 31 bits")
        }
        DataType::Quantized { bits, .. } if !(1..=32).contains(bits) => {
            Some("; a quantized float takes 1 to 32 bits")
        }
        // Not `min >= max`: a NaN bound is refused too.
        DataType::Quantized { min, max, .. }
            if min.partial_cmp(max) != Some(std::cmp::Ordering::Less) =>
        {
            Some("; a quantized float needs a range with min < max")
        }
        DataType::Set(item) if named(item) => Some("; a set of a user class is not supported"),
        DataType::Map(k, v) if named(k) || named(v) => {
            Some("; a map of a user class is not supported")
        }
        DataType::Map(k, v) => ntuple_refusal(k).or_else(|| ntuple_refusal(v)),
        DataType::List(item)
        | DataType::Array(_, item)
        | DataType::Optional(item)
        | DataType::UniquePtr(item)
        | DataType::Atomic(item)
        | DataType::Set(item) => ntuple_refusal(item),
        DataType::Struct(members) => members.iter().find_map(|(_, m)| ntuple_refusal(m)),
        _ => None,
    }
}

/// Whether member names are `_0`, `_1`, …: a `std::pair` or `std::tuple`.
fn positional<'a>(names: impl Iterator<Item = &'a String>) -> bool {
    names.enumerate().all(|(i, n)| *n == format!("_{i}"))
}

/// The count branch of each counted array in `batch`: its name, `n<array>`,
/// and the array's length in each entry.
fn count_branches(batch: &Batch, schema: &Schema) -> Vec<(String, Vec<i32>)> {
    batch
        .iter()
        .zip(schema.iter())
        .filter_map(
            |((name, column), (_, data_type))| match (column, data_type) {
                (Column::List { offsets, .. }, DataType::CArray(_)) => Some((
                    format!("n{name}"),
                    offsets
                        .windows(2)
                        .map(|w| i32::saturating_from(w[1] - w[0]))
                        .collect(),
                )),
                _ => None,
            },
        )
        .collect()
}

/// A tree writes a counted array `x` with the branch `nx` that counts it. A
/// column of that name, as a tree read back gives, has to be that branch.
fn check_counts(batch: &Batch, schema: &Schema) -> Result<()> {
    for (count, lengths) in count_branches(batch, schema) {
        match batch.get(&count) {
            Some(column) if *column != Column::I32(lengths) => {
                return Err(Error::InvalidInput(format!(
                    "column {count:?} is the count of the array {:?}, but does not hold its lengths",
                    &count[1..]
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A batch's columns as tree branches, stored as `schema` says. A column that
/// is the count of a counted array is written by the array, not a second time
/// (see [`check_counts`]).
fn tree_branches(batch: &Batch, schema: &Schema) -> Result<Vec<Branch>> {
    let counts = count_branches(batch, schema);
    batch
        .iter()
        .zip(schema.iter())
        .filter(|((name, _), _)| !counts.iter().any(|(count, _)| count == name))
        .map(|((name, column), (_, data_type))| tree_branch(name, column, data_type))
        .collect()
}

/// A batch's columns as `RNTuple` fields, stored as `schema` says.
fn ntuple_fields(batch: Batch, schema: &Schema) -> Vec<Field> {
    batch
        .into_columns()
        .zip(schema.iter())
        .map(|((name, column), (_, data_type))| {
            let column = ntuple_column(column, &name, data_type);
            Field::new(name, column)
        })
        .collect()
}

/// Split flat `values` into one `Vec` per entry at the cumulative `offsets`.
fn rows<T: Clone>(offsets: &[u64], values: &[T]) -> Vec<Vec<T>> {
    offsets
        .windows(2)
        .map(|w| values[usize::saturating_from(w[0])..usize::saturating_from(w[1])].to_vec())
        .collect()
}

fn strings(offsets: &[u64], bytes: &[u8]) -> Vec<String> {
    offsets
        .windows(2)
        .map(|w| {
            String::from_utf8_lossy(
                &bytes[usize::saturating_from(w[0])..usize::saturating_from(w[1])],
            )
            .into_owned()
        })
        .collect()
}

fn unsupported(name: &str, c: &Column, what: &str) -> Error {
    Error::Unsupported(format!(
        "column {name:?} is {}, which {what} written by oxiroot cannot hold",
        c.type_name()
    ))
}

/// Match a column's scalar variants, binding the values as `$v` and building
/// `$ctor(name, $conv)` for each listed one; any other is unsupported.
macro_rules! by_type {
    ($ty:ident, $name:expr, $c:expr, $what:expr, $col:expr, $v:ident => $conv:expr;
     $($variant:ident => $ctor:ident),* $(,)?) => {
        match $col {
            $(Column::$variant($v) => Ok($ty::$ctor($name, $conv)),)*
            _ => Err(unsupported($name, $c, $what)),
        }
    };
}

/// A column as a tree branch: `Vec<T>` as `std::vector<T>` (or a counted
/// `x[n]` when `data_type` says so), `[T; N]` as a fixed array.
fn tree_branch(name: &str, c: &Column, data_type: &DataType) -> Result<Branch> {
    const WHAT: &str = "a TTree";
    match c {
        Column::String { offsets, bytes } => Ok(Branch::string(name, strings(offsets, bytes))),
        Column::List { offsets, values } if matches!(data_type, DataType::CArray(_)) => {
            by_type!(Branch, name, c, WHAT, &**values, x => rows(offsets, x);
                Bool => jagged_bool, I8 => jagged_i8, U8 => jagged_u8, I16 => jagged_i16,
                U16 => jagged_u16, I32 => jagged_i32, U32 => jagged_u32, I64 => jagged_i64,
                U64 => jagged_u64, F32 => jagged_f32, F64 => jagged_f64)
        }
        Column::List { offsets, values } => match &**values {
            // A list of structs is a split `std::vector<Struct>`: one jagged
            // sub-branch per member, the struct a class named after the column.
            Column::Struct(members) => {
                let members = members
                    .iter()
                    .map(|(m, mc)| {
                        by_type!(SplitMember, m.as_str(), c, WHAT, mc, x => rows(offsets, x);
                            Bool => bool, I8 => i8, U8 => u8, I16 => i16, U16 => u16,
                            I32 => i32, U32 => u32, I64 => i64, U64 => u64, F32 => f32,
                            F64 => f64)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Branch::split_vector(name, class_name(name), members))
            }
            Column::String { offsets: o2, bytes } => Ok(Branch::vector_string(
                name,
                rows(offsets, &strings(o2, bytes)),
            )),
            Column::List {
                offsets: o2,
                values: inner,
            } => {
                by_type!(Branch, name, c, WHAT, &**inner, x => rows(offsets, &rows(o2, x));
                Bool => vector_vector_bool, I8 => vector_vector_i8, U8 => vector_vector_u8,
                I16 => vector_vector_i16, U16 => vector_vector_u16, I32 => vector_vector_i32,
                U32 => vector_vector_u32, I64 => vector_vector_i64, U64 => vector_vector_u64,
                F32 => vector_vector_f32, F64 => vector_vector_f64)
            }
            v => by_type!(Branch, name, c, WHAT, v, x => rows(offsets, x);
                Bool => vector_bool, I8 => vector_i8, U8 => vector_u8, I16 => vector_i16,
                U16 => vector_u16, I32 => vector_i32, U32 => vector_u32, I64 => vector_i64,
                U64 => vector_u64, F32 => vector_f32, F64 => vector_f64),
        },
        // The width goes with the branch, so an empty batch keeps it too.
        Column::Array { len, values } => by_type!(Branch, name, c, WHAT, &**values,
            v => if *len == 0 { Vec::new() } else { v.chunks(*len).map(<[_]>::to_vec).collect() };
            Bool => array_bool, I8 => array_i8, U8 => array_u8, I16 => array_i16,
            U16 => array_u16, I32 => array_i32, U32 => array_u32, I64 => array_i64,
            U64 => array_u64, F32 => array_f32, F64 => array_f64)
        .map(|b| b.with_width(*len)),
        scalar => by_type!(Branch, name, c, WHAT, scalar, v => v.clone();
            Bool => bool, I8 => i8, U8 => u8, I16 => i16, U16 => u16, I32 => i32,
            U32 => u32, I64 => i64, U64 => u64, F32 => f32, F64 => f64),
    }
}

/// A column as an `RNTuple` column, moved rather than copied: lists become
/// collections over their flat values, so a `std::vector<float>` is written
/// straight from its offsets and values. `data_type` chooses the storage
/// where the column's shape allows more than one. `class` names a struct:
/// members `_0`, `_1`, … make a `std::pair` / `std::tuple`, named members a
/// user class called `class` (ROOT reads it with that class's dictionary, or
/// by type emulation without one).
fn ntuple_column(c: Column, class: &str, data_type: &DataType) -> RnColumn {
    // The type of what is inside a list, array or optional. A type that does
    // not have the column's shape was refused by `check_batch`; the column's
    // own type stands in so this cannot fail.
    let inner = |t: &DataType| -> Option<DataType> {
        match t {
            DataType::List(i)
            | DataType::Set(i)
            | DataType::Array(_, i)
            | DataType::Optional(i)
            | DataType::UniquePtr(i) => Some((**i).clone()),
            _ => None,
        }
    };
    if let DataType::Atomic(of) = data_type {
        return RnColumn::Atomic(Box::new(ntuple_column(c, class, of)));
    }
    match c {
        Column::Bool(v) => RnColumn::Bool(v),
        Column::I8(v) => RnColumn::I8(v),
        Column::U8(v) => RnColumn::U8(v),
        Column::I16(v) => RnColumn::I16(v),
        Column::U16(v) => RnColumn::U16(v),
        Column::I32(v) => RnColumn::I32(v),
        Column::U32(v) => RnColumn::U32(v),
        Column::I64(v) => RnColumn::I64(v),
        Column::U64(v) => RnColumn::U64(v),
        Column::F32(values) => match *data_type {
            DataType::Half => RnColumn::HalfF32(values),
            DataType::Truncated { bits } => RnColumn::TruncF32 { values, bits },
            DataType::Quantized { bits, min, max } => RnColumn::QuantF32 {
                values,
                range: (min, max),
                bits,
            },
            _ => RnColumn::F32(values),
        },
        Column::F64(v) => RnColumn::F64(v),
        Column::String { offsets, bytes } => RnColumn::Str(strings(&offsets, &bytes)),
        Column::List { offsets, values } => {
            let item = match (data_type, &*values) {
                // A map's `{_0, _1}` entries take the key and value types.
                (DataType::Map(k, v), Column::Struct(_)) => DataType::Struct(vec![
                    ("_0".into(), (**k).clone()),
                    ("_1".into(), (**v).clone()),
                ]),
                _ => inner(data_type).unwrap_or_else(|| values.data_type()),
            };
            let items = Box::new(ntuple_column(*values, class, &item));
            match data_type {
                DataType::Set(_) | DataType::Map(..) => RnColumn::Assoc {
                    type_name: data_type.cpp_name(),
                    offsets,
                    items,
                },
                _ => RnColumn::Nested { offsets, items },
            }
        }
        Column::Array { len, values } => match (data_type, *values) {
            (DataType::Bitset(_), Column::Bool(bits)) => RnColumn::Bitset { len, bits },
            (_, values) => {
                let item = inner(data_type).unwrap_or_else(|| values.data_type());
                RnColumn::Array {
                    len,
                    items: Box::new(ntuple_column(values, class, &item)),
                }
            }
        },
        Column::Optional { valid, values } => {
            let item = inner(data_type).unwrap_or_else(|| values.data_type());
            RnColumn::Optional {
                unique: matches!(data_type, DataType::UniquePtr(_)),
                present: valid,
                values: Box::new(ntuple_column(*values, class, &item)),
            }
        }
        Column::Struct(members) => {
            let types: Vec<DataType> = match data_type {
                DataType::Struct(types) if types.len() == members.len() => {
                    types.iter().map(|(_, t)| t.clone()).collect()
                }
                _ => members.iter().map(|(_, c)| c.data_type()).collect(),
            };
            let positional = positional(members.iter().map(|(n, _)| n));
            let members: Vec<(String, RnColumn)> = members
                .into_iter()
                .zip(&types)
                .map(|((n, c), t)| {
                    let inner = format!("{class}_{n}");
                    (n, ntuple_column(c, &inner, t))
                })
                .collect();
            if positional {
                RnColumn::Record(members)
            } else {
                RnColumn::Object {
                    type_name: class_name(class),
                    members,
                }
            }
        }
    }
}

/// `name` as a C++ identifier: other characters become `_`, and a leading
/// digit gets a `_` in front.
fn class_name(name: &str) -> String {
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
