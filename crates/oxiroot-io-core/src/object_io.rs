//! The ROOT object read/write framework shared by every persistable oxiroot
//! type: the [`WriteRoot`] and [`ReadRoot`] traits, plus the small helpers they
//! build on (fetching a key's class + decompressed bytes, and the single-object
//! write path).
//!
//! Concrete objects — histograms, graphs, matrices, parameters, … — live in the
//! higher-level crates and `impl` these traits; this crate owns only the
//! framework so a leaf crate (e.g. `oxiroot-linalg`) can persist its own types
//! without depending on `oxiroot-hist`.

use std::borrow::Cow;
use std::fmt;

use std::io::Cursor;

use crate::FileReader;
use crate::buffer::RBuffer;
use crate::error::{Result, decompress_payload};
use crate::file::{ContainerWriter, DirId};
use crate::object::TagReader;
use crate::streamer_gen::ClassDef;

/// A stored object, located and decompressed: what a [`ReadRoot`] type decodes.
///
/// [`FileReader::get`] builds one from a key; a collection builds one for each
/// of its members.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Object<'a> {
    /// The key name, without directory or cycle. Empty for a collection member.
    pub name: &'a str,
    /// The ROOT class name (e.g. `"TH1D"`).
    pub class: &'a str,
    /// The streamed object bytes.
    pub bytes: &'a [u8],
    /// The header length of the key the bytes came from (`fKeyLen`), which
    /// resolves class back-references inside the object. `None` when `bytes` is
    /// a slice of a larger object, as a collection member is.
    pub key_len: Option<usize>,
    /// The file the object was read from, when it came from a key.
    pub file: Option<&'a FileReader>,
    /// The title of the key the object was read from. Empty for a collection
    /// member.
    pub title: &'a str,
    /// Where a collection member sits in the buffer of the key that holds the
    /// collection. A member's class back-references point into that buffer, so
    /// a decoder that follows them reads the member there, not from `bytes`.
    pub within: Option<Within<'a>>,
}

/// A collection member's place in the streamed object it was read from.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Within<'a> {
    /// The whole streamed object the key holds.
    pub buffer: &'a [u8],
    /// The offset of the member's body in `buffer`.
    pub start: usize,
    /// The header length of the key (`fKeyLen`), for a
    /// [`TagReader`](crate::raw::TagReader) over `buffer`.
    pub key_len: usize,
}

impl<'a> Object<'a> {
    /// A member of a collection: a class and its streamed body.
    #[must_use]
    pub const fn member(class: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            name: "",
            class,
            bytes,
            key_len: None,
            file: None,
            title: "",
            within: None,
        }
    }

    /// A member of a collection read from a key: its class, and the range of
    /// its streamed body in the key's `buffer`, whose header is `key_len`
    /// bytes. Unlike [`member`](Self::member), the member can be decoded with
    /// its class back-references resolved.
    #[must_use]
    pub fn member_in(
        class: &'a str,
        buffer: &'a [u8],
        range: std::ops::Range<usize>,
        key_len: usize,
    ) -> Self {
        Self {
            within: Some(Within {
                buffer,
                start: range.start,
                key_len,
            }),
            ..Self::member(class, buffer.get(range).unwrap_or(&[]))
        }
    }

    /// A reader at the object's body, with the class-tag map of the buffer
    /// it reads: the way every decoder starts.
    ///
    /// An object names a class in full once and by its position afterwards,
    /// counted from the start of the key. So a collection member is read
    /// where it lies in the key's buffer, and an object from a key with the
    /// key's header length. A member held on its own has neither: its
    /// positions count from its own first byte.
    ///
    /// # Errors
    ///
    /// If a collection member's place lies outside its buffer.
    pub fn reader(&self) -> Result<(RBuffer<'a>, TagReader)> {
        match self.within {
            Some(within) => {
                let mut r = RBuffer::new(within.buffer);
                r.seek(within.start)?;
                Ok((r, TagReader::new(within.key_len)))
            }
            None => Ok((
                RBuffer::new(self.bytes),
                TagReader::new(self.key_len.unwrap_or(0)),
            )),
        }
    }
}

/// A type that can be read from a ROOT file. Read one with
/// [`FileReader::get`]:
///
/// ```ignore
/// let f = FileReader::open("in.root")?;
/// let h: Hist1D = f.get("h")?;                 // any of TH1D/F/I/S/C/L
/// let s: Hist1D = f.get("by_region/sig")?;     // from a subdirectory
/// ```
pub trait ReadRoot: Sized {
    /// The ROOT class this type reads, as named in a
    /// [`WrongClass`](crate::Error::WrongClass) error (e.g. `"TH1"`, `"TGraph"`).
    const CLASS: &'static str;

    /// Decode from a located object, or `None` if its class is not this type.
    fn from_object(object: &Object<'_>) -> Option<Result<Self>>;
}

/// A ROOT object this workspace can serialize. Implementors provide the class
/// name, key name/title, and streamed payload; [`FileWriter::add`](crate::FileWriter::add)
/// stores it in a file.
///
/// The result reads in ROOT, uproot, and this crate. A type whose class a reader
/// may not know describes it with
/// [`streamer_classes`](WriteRoot::streamer_classes); every file that stores the
/// object embeds that description, so uproot can model it.
pub trait WriteRoot {
    /// The ROOT class name written for this object (e.g. `"TH1D"`, `"Profile1D"`).
    fn root_class(&self) -> Cow<'static, str>;
    /// The object's key name (`fName`).
    fn root_name(&self) -> &str;
    /// The object's title (`fTitle`).
    fn root_title(&self) -> &str;
    /// Write the streamed object payload (no file or key framing) into `w`:
    /// the bytes stored under the object's key. `w` is empty when this is
    /// called, so positions in it are positions in the object.
    fn write_body(&self, w: &mut crate::WBuffer);

    /// The streamed object payload, as [`write_body`](Self::write_body)
    /// writes it.
    #[must_use]
    fn to_root_bytes(&self) -> Vec<u8> {
        let mut w = crate::WBuffer::new();
        self.write_body(&mut w);
        w.into_vec()
    }

    /// The `TStreamerInfo` entries describing this object's class, and the
    /// classes of any objects it contains, for readers that do not know them.
    /// The default is none. Files that store the object embed these, once per
    /// class name.
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        Vec::new()
    }

    /// What [`to_root_bytes`](Self::to_root_bytes) requires of the key it is
    /// stored under. The default is nothing, which is right for every object
    /// that is serialized from its fields.
    fn key_check(&self) -> KeyCheck {
        KeyCheck::Any
    }
}

/// What an object's bytes require of the key they are stored under.
///
/// ROOT refers to a class it has already named in a buffer by position, and
/// counts positions from the start of the key. Bytes copied as they were read
/// keep those references, so they are only valid under a key whose header has
/// the length of the one they came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum KeyCheck {
    /// Any key will do.
    #[default]
    Any,
    /// The key's header must be this many bytes (`fKeyLen`).
    HeaderLen(usize),
    /// The object cannot be stored; the message says why and what to do.
    Unwritable(String),
}

impl KeyCheck {
    /// Check a key of class `class`, named `name`, whose header is `key_len`
    /// bytes.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`](crate::Error::InvalidInput) if the key does not
    /// meet the requirement.
    pub fn check(&self, class: &str, name: &str, key_len: usize) -> crate::Result<()> {
        match self {
            Self::Any => Ok(()),
            Self::HeaderLen(n) if *n == key_len => Ok(()),
            Self::HeaderLen(n) => Err(crate::Error::InvalidInput(format!(
                "{class} {name:?} was read from a key with a {n}-byte header and cannot be \
                 stored under one of {key_len} bytes: the class references inside it count \
                 from the key. Keep the name and title it was read with, or build a new \
                 object from its decoded members"
            ))),
            Self::Unwritable(why) => Err(crate::Error::InvalidInput(format!(
                "{class} {name:?} cannot be stored: {why}"
            ))),
        }
    }
}

/// An object stored as several records rather than under a single key: data
/// blocks placed anywhere in the file, then the key that locates them. A `TTree`
/// (its baskets, then the tree) and an `RNTuple` (its envelopes and pages, then
/// the anchor) are written this way. [`FileWriter::add`](crate::FileWriter::add)
/// stores one in a file, next to any other objects.
pub trait WriteInto {
    /// The class of the key that locates the object (e.g. `"TTree"`).
    fn root_class(&self) -> Cow<'static, str>;
    /// The name of that key.
    fn root_name(&self) -> &str;
    /// Write the object's records at the end of `file`, with its key in `dir`.
    /// This may run more than once for one file: the builder lays a file out
    /// again when it has to switch to the 64-bit form.
    ///
    /// # Errors
    ///
    /// If the object cannot be stored: a name that does not fit a key, or a failed write.
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()>;
    /// The `TStreamerInfo` entries the object's classes need; see
    /// [`WriteRoot::streamer_classes`].
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        Vec::new()
    }
}

/// The streamer info a set of objects needs: each class their
/// [`streamer_classes`](WriteRoot::streamer_classes) describe, once.
#[derive(Clone, Default)]
pub struct StreamerSet {
    classes: Vec<ClassDef<'static>>,
}

impl StreamerSet {
    /// Add what `object` needs.
    pub fn add(&mut self, object: &dyn WriteRoot) {
        self.add_classes(object.streamer_classes());
    }

    /// Add what a multi-record `object` needs.
    pub fn add_records(&mut self, object: &dyn WriteInto) {
        self.add_classes(object.streamer_classes());
    }

    /// Add each class not already present at the same version.
    pub fn add_classes(&mut self, classes: impl IntoIterator<Item = ClassDef<'static>>) {
        for class in classes {
            if !self
                .classes
                .iter()
                .any(|c| c.name == class.name && c.version == class.version)
            {
                self.classes.push(class);
            }
        }
    }

    /// The classes, in the order they were added.
    #[must_use]
    pub fn into_classes(self) -> Vec<ClassDef<'static>> {
        self.classes
    }

    /// Add everything `other` holds.
    pub fn extend(&mut self, other: &Self) {
        self.add_classes(other.classes.iter().cloned());
    }

    /// The classes, in the order they were first added.
    #[must_use]
    pub fn classes(&self) -> &[ClassDef<'static>] {
        &self.classes
    }
}

impl fmt::Debug for StreamerSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamerSet")
            .field(
                "classes",
                &self.classes.iter().map(|c| &*c.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// One object to store in a file: its class, name, title, and streamed bytes
/// (including the object's own byte count and version).
#[non_exhaustive]
pub struct ObjectRecord {
    /// ROOT class name (e.g. `"TH1D"`).
    pub class_name: String,
    /// Object name (the key name).
    pub name: String,
    /// Object title.
    pub title: String,
    /// Streamed object bytes.
    pub object: Vec<u8>,
    /// What `object` requires of its key; see [`WriteRoot::key_check`].
    pub key_check: KeyCheck,
}

/// The on-disk record for any writable object — its class, name, title, and
/// streamed payload. Shared by the [`WriteRoot`] single-object path and the
/// multi-object file builder.
pub fn record_of(object: &dyn WriteRoot) -> ObjectRecord {
    ObjectRecord {
        class_name: object.root_class().into_owned(),
        name: object.root_name().to_string(),
        title: object.root_title().to_string(),
        object: object.to_root_bytes(),
        key_check: object.key_check(),
    }
}

/// A key's class name, its decompressed object bytes, and its header length.
/// `path` is `"dir/sub/name;cycle"`, as for [`FileReader::get`].
///
/// The header length is what the object-reference map
/// ([`crate::object::TagReader`]) needs to resolve the class back-references
/// inside a collection.
///
/// # Errors
///
/// [`Error::NotFound`](crate::Error::NotFound) if `path` names no key, and any error reading or decompressing its
/// payload.
pub fn object_bytes(file: &FileReader, path: &str) -> Result<(String, Vec<u8>, usize)> {
    let key = file.key(path)?;
    let payload = file.key_payload(&key)?;
    let object = decompress_payload(&payload, key.obj_len as usize, format_args!("key {path:?}"))?;
    let key_len = key.key_len as usize;
    Ok((key.class_name, object.into_owned(), key_len))
}

impl std::fmt::Debug for ObjectRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectRecord")
            .field("class_name", &self.class_name)
            .field("name", &self.name)
            .field("title", &self.title)
            .field("bytes", &self.object.len())
            .finish_non_exhaustive()
    }
}
