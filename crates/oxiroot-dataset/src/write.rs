//! [`DatasetWriter`]: one writer for a `TTree` or an RNTuple, taking
//! [`Batch`]es; and [`DatasetObject`], a dataset placed in a
//! [`FileWriter`](oxiroot_io_core::FileWriter) next to other objects.

use std::fs::File;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{ContainerWriter, DirId};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Compression, Error, Result, WriteInto};
use oxiroot_rntuple::{ANCHOR_CLASS, Field, NtupleWriter};
use oxiroot_tree::{Branch, TreeWriter};

use crate::batch::Batch;
use crate::column::Column;
use crate::read::Format;

/// The uncompressed size at which a writer closes a cluster, unless set.
const DEFAULT_CLUSTER_BYTES: usize = 32 * 1024 * 1024;

/// Writes [`Batch`]es as a `TTree` or an RNTuple — the format is the one
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
/// out.write(&batch)?;
/// out.finish()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
pub struct DatasetWriter<W: Write + Seek = File> {
    sink: Sink<W>,
    name: String,
    format: Format,
    compression: Compression,
    large: bool,
    cluster_bytes: usize,
    schema: Option<Vec<(String, String)>>,
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
        DatasetWriter {
            sink: Sink::Pending {
                sink,
                file_name: file_name.to_string(),
            },
            name: name.to_string(),
            format,
            compression: Compression::Zstd(5),
            large: false,
            cluster_bytes: DEFAULT_CLUSTER_BYTES,
            schema: None,
            buffer: None,
            entries: 0,
        }
    }

    /// Compress with `compression`. Set it before the first write: the file is
    /// opened with the first cluster.
    #[must_use]
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Close a cluster once about `bytes` of uncompressed values are buffered.
    #[must_use]
    pub fn cluster_bytes(mut self, bytes: usize) -> Self {
        self.cluster_bytes = bytes.max(1);
        self
    }

    /// Use ROOT's 64-bit file layout, needed past 2 GiB. Set it before the
    /// first write.
    #[must_use]
    pub fn large(mut self, large: bool) -> Self {
        self.large = large;
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
    /// On the first batch, a column the format cannot hold (nothing is written
    /// then); later, a batch whose columns differ from the first's; or a write
    /// error when a cluster is flushed.
    pub fn write(&mut self, batch: &Batch) -> Result<()> {
        let schema = batch.schema();
        match &self.schema {
            None => {
                for (name, column) in batch.iter() {
                    check_column(self.format, name, column)?;
                }
                self.schema = Some(schema);
            }
            Some(s) if *s != schema => {
                return Err(Error::SchemaChanged {
                    detail: format!(
                        "this batch's columns {schema:?} differ from the first's {s:?}"
                    ),
                });
            }
            Some(_) => {}
        }
        if batch.is_empty() {
            return Ok(());
        }
        self.entries += batch.len() as u64;
        match &mut self.buffer {
            None if batch.byte_size() >= self.cluster_bytes => return self.flush_batch(batch),
            None => self.buffer = Some(batch.clone()),
            Some(buffer) => buffer.append(batch.clone())?,
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

    fn flush_batch(&mut self, batch: &Batch) -> Result<()> {
        self.open()?;
        match &mut self.sink {
            Sink::Tree(w) => {
                let branches = batch
                    .iter()
                    .map(|(n, c)| tree_branch(n, c))
                    .collect::<Result<Vec<_>>>()?;
                w.write_batch(&branches)
            }
            Sink::Ntuple(w) => {
                let fields = batch
                    .iter()
                    .map(|(n, c)| ntuple_field(n, c))
                    .collect::<Result<Vec<_>>>()?;
                w.write_batch(&fields)
            }
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

    /// Write what is buffered and close the file. Returns the sink.
    ///
    /// # Errors
    ///
    /// Nothing was written (a dataset needs at least one entry), or a write
    /// error.
    pub fn finish(mut self) -> Result<W> {
        self.flush()?;
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
/// `put`, next to histograms or other datasets, in one cluster.
///
/// ```no_run
/// use oxiroot_dataset::{Batch, DatasetObject, Format};
/// use oxiroot_io_core::{Compression, FileWriter};
///
/// let batch = Batch::new().column("energy", vec![10.5_f64, 20.1])?;
/// FileWriter::create("run.root")
///     .put(DatasetObject::new("events", Format::Tree, &batch)?)
///     .put(DatasetObject::new("events_rn", Format::Ntuple, &batch)?)
///     .write(Compression::Zstd(5))?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
pub struct DatasetObject {
    name: String,
    content: Content,
}

enum Content {
    Tree(Vec<Branch>),
    Ntuple(Vec<Field>),
}

impl DatasetObject {
    /// `batch` as the dataset `name` in `format`.
    ///
    /// # Errors
    ///
    /// A column the format cannot hold.
    pub fn new(name: &str, format: Format, batch: &Batch) -> Result<DatasetObject> {
        for (n, c) in batch.iter() {
            check_column(format, n, c)?;
        }
        let content = match format {
            Format::Tree => Content::Tree(
                batch
                    .iter()
                    .map(|(n, c)| tree_branch(n, c))
                    .collect::<Result<_>>()?,
            ),
            Format::Ntuple => Content::Ntuple(
                batch
                    .iter()
                    .map(|(n, c)| ntuple_field(n, c))
                    .collect::<Result<_>>()?,
            ),
        };
        Ok(DatasetObject {
            name: name.to_string(),
            content,
        })
    }
}

impl std::fmt::Debug for DatasetObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatasetObject")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl WriteInto for DatasetObject {
    fn root_class(&self) -> String {
        match self.content {
            Content::Tree(_) => "TTree".to_string(),
            Content::Ntuple(_) => ANCHOR_CLASS.to_string(),
        }
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        match &self.content {
            Content::Tree(b) => oxiroot_tree::raw::write_tree(file, dir, &self.name, b, 0),
            Content::Ntuple(f) => oxiroot_rntuple::raw::write_ntuple(file, dir, &self.name, f, &[]),
        }
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        match &self.content {
            Content::Tree(b) => oxiroot_tree::raw::tree_streamer_classes(b),
            Content::Ntuple(f) => oxiroot_rntuple::raw::ntuple_streamer_classes(f),
        }
    }
}

/// Refuse a column `format` cannot hold, naming it and saying why.
fn check_column(format: Format, name: &str, column: &Column) -> Result<()> {
    let ok = match format {
        Format::Tree => tree_branch_ok(column),
        Format::Ntuple => ntuple_field_ok(column),
    };
    if ok {
        Ok(())
    } else {
        let what = match format {
            Format::Tree => "a TTree",
            Format::Ntuple => "an RNTuple",
        };
        Err(Error::Unsupported(format!(
            "column {name:?} is {}, which {what} written by oxiroot cannot hold",
            column.type_name()
        )))
    }
}

fn is_scalar(c: &Column) -> bool {
    matches!(
        c,
        Column::Bool(_)
            | Column::I8(_)
            | Column::U8(_)
            | Column::I16(_)
            | Column::U16(_)
            | Column::I32(_)
            | Column::U32(_)
            | Column::I64(_)
            | Column::U64(_)
            | Column::F32(_)
            | Column::F64(_)
    )
}

fn tree_branch_ok(c: &Column) -> bool {
    match c {
        Column::String { .. } => true,
        Column::List { values, .. } => is_scalar(values) && !matches!(**values, Column::Bool(_)),
        Column::Array { values, .. } => is_scalar(values),
        other => is_scalar(other),
    }
}

fn ntuple_field_ok(c: &Column) -> bool {
    match c {
        Column::String { .. } => true,
        Column::List { values, .. } => match &**values {
            Column::U32(_) | Column::U64(_) => false,
            Column::String { .. } => true,
            Column::List { values, .. } => matches!(
                **values,
                Column::Bool(_)
                    | Column::I32(_)
                    | Column::I64(_)
                    | Column::F32(_)
                    | Column::F64(_)
                    | Column::String { .. }
            ),
            v => is_scalar(v),
        },
        Column::Array { values, .. } => matches!(
            **values,
            Column::I32(_)
                | Column::I64(_)
                | Column::U32(_)
                | Column::U64(_)
                | Column::F32(_)
                | Column::F64(_)
        ),
        Column::Optional { values, .. } => matches!(
            **values,
            Column::Bool(_)
                | Column::I32(_)
                | Column::I64(_)
                | Column::U32(_)
                | Column::U64(_)
                | Column::F32(_)
                | Column::F64(_)
        ),
        other => is_scalar(other),
    }
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

/// A column as a tree branch: `Vec<T>` as `std::vector<T>`, `[T; N]` as a
/// fixed array.
fn tree_branch(name: &str, c: &Column) -> Result<Branch> {
    const WHAT: &str = "a TTree";
    match c {
        Column::String { offsets, bytes } => Ok(Branch::string(name, strings(offsets, bytes))),
        Column::List { offsets, values } => {
            by_type!(Branch, name, c, WHAT, &**values, v => rows(offsets, v);
            I8 => vector_i8, U8 => vector_u8, I16 => vector_i16, U16 => vector_u16,
            I32 => vector_i32, U32 => vector_u32, I64 => vector_i64, U64 => vector_u64,
            F32 => vector_f32, F64 => vector_f64)
        }
        Column::Array { len, values } => by_type!(Branch, name, c, WHAT, &**values,
            v => v.chunks(*len).map(<[_]>::to_vec).collect();
            Bool => array_bool, I8 => array_i8, U8 => array_u8, I16 => array_i16,
            U16 => array_u16, I32 => array_i32, U32 => array_u32, I64 => array_i64,
            U64 => array_u64, F32 => array_f32, F64 => array_f64),
        scalar => by_type!(Branch, name, c, WHAT, scalar, v => v.clone();
            Bool => bool, I8 => i8, U8 => u8, I16 => i16, U16 => u16, I32 => i32,
            U32 => u32, I64 => i64, U64 => u64, F32 => f32, F64 => f64),
    }
}

/// A column as an RNTuple field.
fn ntuple_field(name: &str, c: &Column) -> Result<Field> {
    const WHAT: &str = "an RNTuple";
    match c {
        Column::String { offsets, bytes } => Ok(Field::string(name, strings(offsets, bytes))),
        Column::List { offsets, values } => match &**values {
            Column::String { offsets: o2, bytes } => Ok(Field::vector_string(
                name,
                rows(offsets, &strings(o2, bytes)),
            )),
            Column::List {
                offsets: o2,
                values: inner,
            } => match &**inner {
                Column::String { offsets: o3, bytes } => Ok(Field::vector_vector_string(
                    name,
                    rows(offsets, &rows(o2, &strings(o3, bytes))),
                )),
                v => by_type!(Field, name, c, WHAT, v, x => rows(offsets, &rows(o2, x));
                    Bool => vector_vector_bool, I32 => vector_vector_i32, I64 => vector_vector_i64,
                    F32 => vector_vector_f32, F64 => vector_vector_f64),
            },
            v => by_type!(Field, name, c, WHAT, v, x => rows(offsets, x);
                Bool => vector_bool, I8 => vector_i8, U8 => vector_u8, I16 => vector_i16,
                U16 => vector_u16, I32 => vector_i32, I64 => vector_i64, F32 => vector_f32,
                F64 => vector_f64),
        },
        Column::Array { len, values } => by_type!(Field, name, c, WHAT, &**values,
            v => v.chunks(*len).map(<[_]>::to_vec).collect();
            I32 => array_i32, I64 => array_i64, U32 => array_u32, U64 => array_u64,
            F32 => array_f32, F64 => array_f64)?,
        Column::Optional { valid, values } => by_type!(Field, name, c, WHAT, &**values,
            v => {
                let mut it = v.iter().copied();
                valid.iter().map(|&p| if p { it.next() } else { None }).collect()
            };
            Bool => optional_bool, I32 => optional_i32, I64 => optional_i64, U32 => optional_u32,
            U64 => optional_u64, F32 => optional_f32, F64 => optional_f64),
        scalar => by_type!(Field, name, c, WHAT, scalar, v => v.clone();
            Bool => bool, I8 => i8, U8 => u8, I16 => i16, U16 => u16, I32 => i32,
            U32 => u32, I64 => i64, U64 => u64, F32 => f32, F64 => f64),
    }
}
