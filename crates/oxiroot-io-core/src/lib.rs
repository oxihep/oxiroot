//! The ROOT file container for `oxiroot`: reading and writing `TFile`s and the
//! objects in them. The other oxiroot crates build on it.
//!
//! # Files
//!
//! [`FileReader`] opens a file from disk, from memory, through a memory map (the
//! `mmap` feature), or remotely over HTTP(S) (`http`) or XRootD (`xrootd`), and
//! reads its keys, directories and streamer info on demand through a
//! [`ByteSource`]. [`FileWriter`] writes a new file, or appends to an existing
//! one, with subdirectories ([`SubdirWriter`]). [`ContainerWriter`] is the layer
//! beneath it, which places keys and raw records: formats such as `TTree` and
//! RNTuple are more than one keyed object.
//!
//! # Objects
//!
//! Every persistable type implements [`WriteRoot`] and [`ReadRoot`], or
//! [`WriteInto`] when it is written as several records. The concrete classes
//! (histograms, graphs, matrices, …) live in the crates that model them. This
//! crate owns only the framework, so a crate can persist its own types without
//! depending on `oxiroot-hist`. The generic objects that belong to no format live
//! here: [`ObjString`], [`Parameter`], [`ObjList`] (`TList` and `TObjArray`)
//! and [`ObjMap`].
//!
//! # Any class, through its streamer info
//!
//! [`read_object`] decodes an object of any class into a [`Value`] tree, by the
//! member layout the file's own `TStreamerInfo` declares ([`StreamerRegistry`]),
//! with no compiled-in knowledge of the class. `rootls`- and `rootprint`-style
//! inspection runs on it.
//!
//! # Building blocks
//!
//! To implement the traits for a class of your own: [`RBuffer`] and [`WBuffer`]
//! read and write ROOT's encoding and its byte-count framing; [`read_object_base`],
//! [`read_named`], [`write_object_base`] and [`write_named`] stream the common base
//! classes; and [`TagReader`] resolves class tags and back-references. The
//! [`streamer_gen`] module describes a class's members for the `TStreamerInfo`
//! record a written file carries. ROOT's classic on-disk integers are big-endian;
//! the buffers name their endianness, so the same types serve RNTuple's
//! little-endian payloads.
//!
//! Every item is exported at the crate root, except the [`streamer_gen`] helpers,
//! whose short names (`basic`, `base`, `stl`, …) read best qualified.

mod buffer;
#[doc(hidden)]
pub mod cast;
mod compression;
mod error;
mod file;
mod object;
mod object_io;
mod objects;
mod read_object;
mod streamer;
pub mod streamer_gen;
mod streamer_info;
mod value;

pub use buffer::{CountToken, K_BYTE_COUNT_MASK, Patch, RBuffer, VersionHeader, WBuffer};
pub use compression::Compression;

/// The deepest nesting of streamed objects and base classes a reader follows.
/// Streamer info comes from the file, so a class that lists itself as its own
/// base (or a long chain) must end in an error, not a stack overflow. Every
/// streamer walker in the workspace uses this limit.
pub const MAX_STREAMER_DEPTH: usize = 64;
pub use error::{Error, External, Result, decompress_payload};
#[cfg(feature = "mmap")]
pub use file::MmapSource;
#[cfg(feature = "xrootd")]
pub use file::XrootdSource;
pub use file::{
    BIG_FILE_VERSION, ByteSource, BytesSource, ContainerWriter, DATIME, Datime, DirId, Directory,
    FILE_VERSION, FileHeader, FileReader, FileSource, FileWriter, FreeSegment, KSTART_BIG_FILE,
    Key, MAGIC, SubdirWriter, Uuid, compress_if_smaller, find_key, read_free, split_cycle,
};
pub use object::{ObjectRef, RecordHeader, TagReader};
pub use object_io::{
    ObjectRecord, ReadRoot, StreamerSet, WriteInto, WriteRoot, object_bytes_any,
    object_bytes_any_keyed, record_of,
};
pub use objects::{FromMember, ListKind, ObjList, ObjMap, ObjString, ParamValue, Parameter};
pub use read_object::read_object;
pub use streamer::{
    Named, ObjectBase, read_named, read_object_base, skip_versioned, write_named, write_object_any,
    write_object_base,
};
pub use streamer_info::{StreamerElement, StreamerInfo, StreamerRegistry, parse_streamer_info};
pub use value::Value;
