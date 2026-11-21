//! The ROOT file container for `oxiroot`: reading and writing `TFile`s and the
//! objects in them. The other oxiroot crates build on it.
//!
//! # Files
//!
//! [`FileReader`] opens a file from disk, from memory, through a memory map (the
//! `mmap` feature), or remotely over HTTP(S) (`http`) or `XRootD` (`xrootd`), and
//! reads its keys, directories and streamer info on demand through a
//! [`ByteSource`]. [`FileWriter`] writes a new file, or appends to an existing
//! one, with subdirectories ([`SubdirWriter`]). [`ContainerWriter`](raw::ContainerWriter) is the layer
//! beneath it, which places keys and raw records: formats such as `TTree` and
//! `RNTuple` are more than one keyed object.
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
//! read and write ROOT's encoding and its byte-count framing; [`read_object_base`](raw::read_object_base),
//! [`read_named`](raw::read_named), [`write_object_base`](raw::write_object_base) and [`write_named`](raw::write_named) stream the common base
//! classes; and [`TagReader`](raw::TagReader) resolves class tags and back-references. The
//! [`streamer_gen`] module describes a class's members for the `TStreamerInfo`
//! record a written file carries. ROOT's classic on-disk integers are big-endian;
//! the buffers name their endianness, so the same types serve `RNTuple`'s
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
#[doc(hidden)]
pub mod streamer_gen;
mod streamer_info;
mod value;

pub use buffer::{CountToken, Patch, RBuffer, WBuffer};
pub use compression::Compression;

pub use error::{Error, External, Result};
#[cfg(feature = "mmap")]
pub use file::MmapSource;
#[cfg(feature = "xrootd-tls")]
pub use file::XrootdOptions;
#[cfg(feature = "xrootd")]
pub use file::XrootdSource;
pub use file::{
    ByteSource, BytesSource, FileReader, FileSource, FileUpdater, FileWriter, Storable, Stored,
    SubdirWriter,
};

pub use object_io::{Object, ReadRoot, StreamerSet, WriteInto, WriteRoot};
pub use objects::{ListKind, ObjList, ObjMap, ObjString, ParamValue, Parameter};
pub use read_object::read_object;

pub use streamer_info::StreamerRegistry;
pub use value::Value;

/// The ROOT file format itself: on-disk records (file header, keys,
/// directories, free segments, streamer info, object headers), their
/// constants, and the helpers that read and write them. For tools that work
/// at the level of the format; reading and writing objects needs none of it.
pub mod raw {
    pub use crate::buffer::{K_BYTE_COUNT_MASK, VersionHeader};
    pub use crate::error::{check_len, decompress_payload};
    pub use crate::file::{
        BIG_FILE_VERSION, ContainerWriter, DATIME, Datime, DirId, Directory, FILE_VERSION,
        FileHeader, Form, FreeSegment, KSTART_BIG_FILE, Key, MAGIC, Uuid, compress_if_smaller,
        find_key, read_free, split_cycle,
    };
    pub use crate::object::{
        K_CLASS_MASK, K_NEW_CLASS_TAG, ObjectRef, PointerHeader, RecordHeader, TagReader,
        read_pointer_header,
    };
    pub use crate::object_io::{ObjectRecord, object_bytes, record_of};
    pub use crate::streamer::{
        Named, ObjectBase, begin_object_any, end_object_any, read_named, read_object_base,
        skip_versioned, write_named, write_object_any, write_object_base, write_object_ptr,
    };
    pub use crate::streamer_info::{StreamerElement, StreamerInfo, parse_streamer_info};

    /// A file's streamer-info record as stored: the decompressed
    /// `TList<TStreamerInfo>` bytes and the key length they were written under,
    /// or `None` if the file has no record.
    ///
    /// # Errors
    ///
    /// If the record cannot be read or decompressed.
    pub fn streamer_info_bytes(
        file: &crate::FileReader,
    ) -> crate::Result<Option<(Vec<u8>, usize)>> {
        file.streamer_info_bytes()
    }

    /// The deepest nesting of streamed objects and base classes a reader follows.
    /// Streamer info comes from the file, so a class that lists itself as its own
    /// base (or a long chain) must end in an error, not a stack overflow. Every
    /// streamer walker in the workspace uses this limit.
    pub const MAX_STREAMER_DEPTH: usize = 64;
}
