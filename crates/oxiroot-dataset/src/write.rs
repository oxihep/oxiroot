//! Writing a `TTree` or an `RNTuple` from [`Batch`]es: [`NamedBatch`], a dataset
//! held in memory and placed in a [`FileWriter`](oxiroot_io_core::FileWriter)
//! like any other object; and [`DatasetWriter`], which streams one too large
//! to hold.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{ContainerWriter, DirId, Objects};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Compression, Error, Result, WriteInto, WriteRoot};
use oxiroot_rntuple::ANCHOR_CLASS;
use oxiroot_rntuple::raw::{Field, NtupleWriter};
use oxiroot_tree::raw::{Branch, TreeWriter};

use crate::Column;
use crate::DataType;
use crate::batch::Batch;
use crate::read::Format;
use crate::schema::Schema;

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
/// out.append(batch)?;
/// out.finish()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
#[must_use = "a DatasetWriter writes its last entries, and the file's directory, in `finish`"]
pub struct DatasetWriter<W: Write + Seek = File> {
    /// The sink and the file name, until the first cluster is written.
    pending: Option<(W, String)>,
    /// For a writer [`create`](DatasetWriter::create)d at a path: the
    /// temporary file the sink is, which takes the path's place at `finish`.
    in_place_of: Option<oxiroot_io_core::raw::PendingFile>,
    /// The file's writer, from the first cluster on.
    opened: Option<Opened<W>>,
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
    /// What [`add`](Self::add) took, stored when the file is finished.
    objects: Objects,
}

/// The file once its first cluster is written: the options are fixed.
enum Opened<W: Write + Seek> {
    Tree(TreeWriter<W>),
    Ntuple(NtupleWriter<W>),
}

impl DatasetWriter<File> {
    /// Create the file `path` to hold the dataset `name` in `format`,
    /// compressed with Zstd level 5 unless [`compression`](Self::compression)
    /// says otherwise. It is written beside `path` and takes its place when
    /// [`finish`](Self::finish) succeeds, so a write that fails or is dropped
    /// leaves `path` as it was.
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
        let pending = oxiroot_io_core::raw::PendingFile::create(path)?;
        let mut writer = Self::new(pending.file()?, &file_name, name, format);
        writer.in_place_of = Some(pending);
        Ok(writer)
    }
}

impl<W: Write + Seek> DatasetWriter<W> {
    /// Write the dataset `name` in `format` to `sink`; `file_name` is the name
    /// recorded in the file's header.
    pub fn new(sink: W, file_name: &str, name: &str, format: Format) -> Self {
        Self {
            pending: Some((sink, file_name.to_string())),
            in_place_of: None,
            opened: None,
            name: name.to_string(),
            format,
            compression: Compression::default(),
            large: false,
            cluster_bytes: DEFAULT_CLUSTER_BYTES,
            schema: None,
            empty: None,
            buffer: None,
            entries: 0,
            objects: Objects::default(),
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
    ///     .with_schema(batch.schema().with("pt", DataType::Half));
    /// out.append(batch)?;
    /// out.finish()?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    pub fn with_schema(mut self, schema: Schema) -> Self {
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

    /// Store `object` (a histogram, a graph, a parameter) in the same file,
    /// under a key of its own in the top directory. It is serialized now and
    /// written by [`finish`](Self::finish), after the dataset.
    ///
    /// An unnamed object, or one named as the dataset or as another object,
    /// is an error when the file is finished.
    pub fn add<T: WriteRoot + ?Sized>(&mut self, object: &T) {
        self.objects.add(object);
    }

    /// Add `batch`'s entries.
    ///
    /// # Errors
    ///
    /// On the first batch, a column the format cannot hold or one that does
    /// not fit the [`with_schema`](Self::with_schema) (nothing is written then); later,
    /// a batch whose columns differ from the first's; or a write error when a
    /// cluster is flushed.
    pub fn append(&mut self, batch: Batch) -> Result<()> {
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
        let chunks = chunk_ranges(batch);
        let schema = self.schema.clone().unwrap_or_else(|| batch.schema());
        let convert_tree =
            |r: &std::ops::Range<usize>| Ok(tree_branches(batch.slice(r.clone()), &schema));
        let convert_ntuple =
            |r: &std::ops::Range<usize>| Ok(ntuple_fields(batch.slice(r.clone()), &schema));
        match self.open()? {
            Opened::Tree(w) => w.write_batches(&map_chunks(&chunks, convert_tree)?),
            Opened::Ntuple(w) => w.write_batches(&map_chunks(&chunks, convert_ntuple)?),
        }
    }

    /// The file's writer, started on first use.
    ///
    /// # Errors
    ///
    /// If the file's header cannot be written. The sink is gone with the
    /// attempt, so every later call fails too.
    fn open(&mut self) -> Result<&mut Opened<W>> {
        match &mut self.opened {
            Some(opened) => Ok(opened),
            slot @ None => {
                let (sink, file_name) = self.pending.take().ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "dataset {:?}: the file could not be started, so nothing more can be \
                         written to it",
                        self.name
                    ))
                })?;
                let (c, name) = (self.compression, self.name.as_str());
                let opened = match (self.format, self.large) {
                    (Format::Tree, false) => {
                        Opened::Tree(TreeWriter::new(sink, &file_name, name, c)?)
                    }
                    (Format::Tree, true) => {
                        Opened::Tree(TreeWriter::new_large(sink, &file_name, name, c)?)
                    }
                    (Format::Ntuple, false) => {
                        Opened::Ntuple(NtupleWriter::new(sink, &file_name, name, c)?)
                    }
                    (Format::Ntuple, true) => {
                        Opened::Ntuple(NtupleWriter::new_large(sink, &file_name, name, c)?)
                    }
                };
                Ok(slot.insert(opened))
            }
        }
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
        if self.opened.is_none()
            && let Some(empty) = self.empty.take()
        {
            let schema = self.schema.clone().unwrap_or_else(|| empty.schema());
            match self.open()? {
                Opened::Tree(w) => w.write_batch(&tree_branches(empty, &schema))?,
                Opened::Ntuple(w) => w.write_batch(&ntuple_fields(empty, &schema))?,
            }
        }
        let sink = match self.opened {
            Some(Opened::Tree(w)) => w.finish_with(&self.objects),
            Some(Opened::Ntuple(w)) => w.finish_with(&self.objects),
            None => Err(Error::InvalidInput(format!(
                "dataset {:?}: no entries were written",
                self.name
            ))),
        }?;
        if let Some(pending) = self.in_place_of.take() {
            pending.commit()?;
        }
        Ok(sink)
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

/// A dataset held in memory: [`Batch`]es under a name, to store as a `TTree`
/// or an `RNTuple`. It goes in a [`FileWriter`](oxiroot_io_core::FileWriter)
/// with `add`, like a histogram or a graph, and is written with the file.
///
/// ```no_run
/// use oxiroot_dataset::{Batch, Format, NamedBatch};
/// use oxiroot_io_core::{FileWriter, Parameter};
///
/// let batch = Batch::new().column("energy", vec![10.5_f64, 20.1])?;
/// let mut events = NamedBatch::new("events", Format::Tree, batch);
/// events.append(Batch::new().column("energy", vec![7.25_f64])?)?;
///
/// FileWriter::create("run.root")
///     .add(&Parameter::f64(137.5).named("lumi"))
///     .add(&events)
///     .write()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
///
/// The first batch fixes the columns: a later one must have the same, of the
/// same types. A column the format cannot hold, or one that does not fit the
/// [`with_schema`](Self::with_schema), is reported when the file is written.
///
/// Adding it by reference copies its values into the file being built;
/// adding it by value hands them over. For a dataset too large to hold, write it with a
/// [`DatasetWriter`], which stores histograms beside it too.
#[derive(Debug, Clone)]
#[must_use = "a NamedBatch does nothing until added to a FileWriter"]
pub struct NamedBatch {
    name: String,
    format: Format,
    batch: Batch,
    schema: Option<Schema>,
}

impl NamedBatch {
    /// `batch` as the dataset `name` in `format`.
    pub fn new(name: &str, format: Format, batch: Batch) -> Self {
        Self {
            name: name.to_string(),
            format,
            batch,
            schema: None,
        }
    }

    /// Store the columns as `schema` says; see [`DatasetWriter::with_schema`].
    pub fn with_schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Add the entries of `batch` after those held.
    ///
    /// # Errors
    ///
    /// [`Error::SchemaChanged`] if its columns are not those of the batches
    /// before it.
    pub fn append(&mut self, batch: Batch) -> Result<()> {
        self.batch.append(batch)
    }

    /// The dataset's name: the key it is stored under.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The format it is stored in.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// The number of entries held.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.batch.len() as u64
    }

    /// The entries held, as one batch.
    #[must_use]
    pub fn batch(&self) -> &Batch {
        &self.batch
    }

    /// The batch as the format's own records, cut into chunks of about
    /// [`CHUNK_BYTES_PER_COLUMN`] per column as a [`DatasetWriter`] cuts a
    /// cluster: a basket per branch per chunk, or an `RNTuple` cluster per
    /// chunk, so a read can take a window or run in parallel.
    fn content(self) -> Result<Content> {
        let schema = self.schema.unwrap_or_else(|| self.batch.schema());
        check_batch(&schema, &self.batch)?;
        for (column, data_type) in schema.iter() {
            check_type(self.format, column, data_type)?;
        }
        let chunks = chunk_ranges(&self.batch);
        Ok(match self.format {
            Format::Tree => {
                check_counts(&self.batch, &schema)?;
                let per_basket = chunks.first().map_or(0, ExactSizeIterator::len);
                Content::Tree(tree_branches(self.batch, &schema), per_basket)
            }
            Format::Ntuple if chunks.len() > 1 => Content::Ntuple(
                chunks
                    .into_iter()
                    .map(|r| ntuple_fields(self.batch.slice(r), &schema))
                    .collect(),
            ),
            Format::Ntuple => Content::Ntuple(vec![ntuple_fields(self.batch, &schema)]),
        })
    }
}

enum Content {
    /// The branches, and the entries in each basket (`0`: one basket).
    Tree(Vec<Branch>, usize),
    /// The fields of each cluster.
    Ntuple(Vec<Vec<Field>>),
}

/// A [`NamedBatch`] converted for its format, or why it could not be.
struct Records {
    name: String,
    format: Format,
    content: std::result::Result<Content, Deferred>,
}

/// An error found when the object was built, reported when the file is
/// written. An error is handed over once, whole; the writer lays a file out
/// at most twice, and a second request gets the same message.
struct Deferred {
    error: std::sync::Mutex<Option<Error>>,
    message: String,
}

impl Deferred {
    fn new(error: Error) -> Self {
        Self {
            message: error.to_string(),
            error: std::sync::Mutex::new(Some(error)),
        }
    }

    fn take(&self) -> Error {
        self.error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Error::InvalidInput(self.message.clone()))
    }
}

impl oxiroot_io_core::Storable for &NamedBatch {
    fn into_stored(self) -> oxiroot_io_core::Stored {
        self.clone().into_stored()
    }
}

impl oxiroot_io_core::Storable for NamedBatch {
    fn into_stored(self) -> oxiroot_io_core::Stored {
        oxiroot_io_core::Stored::records(Records {
            name: self.name.clone(),
            format: self.format,
            content: self.content().map_err(Deferred::new),
        })
    }
}

impl WriteInto for Records {
    fn root_class(&self) -> Cow<'static, str> {
        match self.format {
            Format::Tree => Cow::Borrowed("TTree"),
            Format::Ntuple => Cow::Borrowed(ANCHOR_CLASS),
        }
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        match &self.content {
            Ok(Content::Tree(b, per_basket)) => {
                oxiroot_tree::raw::write_tree(file, dir, &self.name, b, *per_basket)
            }
            Ok(Content::Ntuple(clusters)) => {
                oxiroot_rntuple::raw::write_ntuple_clusters(file, dir, &self.name, clusters)
            }
            Err(deferred) => Err(deferred.take()),
        }
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        match &self.content {
            Ok(Content::Tree(b, _)) => oxiroot_tree::raw::tree_streamer_classes(b),
            Ok(Content::Ntuple(clusters)) => oxiroot_rntuple::raw::ntuple_streamer_classes(
                clusters.first().into_iter().flatten(),
            ),
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

/// A batch's columns as tree branches, stored as `schema` says: a branch
/// takes its column as it is, and a counted array says so. A column that is
/// the count of a counted array is written by the array, not a second time
/// (see [`check_counts`]).
fn tree_branches(batch: Batch, schema: &Schema) -> Vec<Branch> {
    let counts = count_branches(&batch, schema);
    batch
        .into_columns()
        .zip(schema.iter())
        .filter(|((name, _), _)| !counts.iter().any(|(count, _)| count == name))
        .map(|((name, column), (_, data_type))| {
            let branch = Branch::new(name, column);
            match data_type {
                DataType::CArray(_) => branch.counted(),
                _ => branch,
            }
        })
        .collect()
}

/// A batch's columns as `RNTuple` fields, stored as `schema` says.
fn ntuple_fields(batch: Batch, schema: &Schema) -> Vec<Field> {
    batch
        .into_columns()
        .zip(schema.iter())
        .map(|((name, column), (_, data_type))| {
            Field::new(name, column).stored_as(data_type.clone())
        })
        .collect()
}

#[cfg(test)]
mod failed_start_tests {
    use std::io::{Seek, SeekFrom, Write};

    use super::{Batch, DatasetWriter, Format};

    /// A sink that refuses every write.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk on fire"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for Broken {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    /// When the file cannot be started, the write fails, and so does every
    /// later one: an error each time, never a panic.
    #[test]
    fn a_file_that_cannot_be_started_keeps_failing() {
        for format in [Format::Tree, Format::Ntuple] {
            let batch = || Batch::new().column("x", vec![1.0_f64, 2.0]).unwrap();
            let mut out = DatasetWriter::new(Broken, "f.root", "events", format).cluster_bytes(1);
            assert!(out.append(batch()).is_err());
            let err = out.append(batch()).unwrap_err();
            assert!(err.to_string().contains("could not be started"), "{err}");
            assert!(out.finish().is_err());
        }
    }
}
