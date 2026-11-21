//! [`FileReader`] — the high-level entry point for reading a ROOT file.
//!
//! Mirrors the spirit of `ROOT::Experimental::RFile`: a small Open/Get/List
//! surface over the `TFile` container. Typed objects are read through
//! [`get`](FileReader::get), which also reads any class as a
//! [`Value`](crate::Value); [`FileWriter`](crate::FileWriter) writes.

use crate::cast::SaturatingFrom;
use std::path::Path;

use bytes::Bytes;

use super::directory::Directory;
use super::free::{FreeSegment, read_free};
use super::header::FileHeader;
use super::key::Key;
use super::readahead::ReadAhead;
use super::source::{ByteSource, BytesSource, FileSource};
use crate::ClassKind;
use crate::buffer::RBuffer;
use crate::error::{Error, Result, decompress_payload};
use crate::object_io::{Object, ReadRoot};
use crate::streamer_info::{StoredInfo, StreamerRegistry, parse_stored_infos, parse_streamer_info};

/// Bytes fetched from the start of the file to parse its header. The `TFile`
/// header is ~100 bytes (a little more in the 64-bit form); 512 covers it with
/// room to spare and is a single small read for a remote source.
const HEADER_PROBE: u64 = 512;

/// An open ROOT file, read through a [`ByteSource`]. [`open`](Self::open) reads
/// a local file; (with the `http` or `xrootd` feature)
/// [`open_url`](Self::open_url) reads a remote one, fetching only the byte
/// ranges each object touches and never downloading the file whole.
///
/// The header, root directory, and key list are parsed at open (a few small
/// ranged reads); object, page, and basket bytes are fetched on demand.
///
/// [`FileWriter`](crate::FileWriter) writes files.
#[doc(alias = "RFile", alias = "TFile")]
pub struct FileReader {
    source: Box<dyn ByteSource>,
    size: u64,
    header: FileHeader,
    root_dir: Directory,
    /// The parsed streamer info, read once: every object read needs it, and
    /// decompressing and parsing it again each time dominated small reads.
    registry: std::sync::OnceLock<StreamerRegistry>,
    /// Subdirectories already read, by path: reading N objects from one
    /// subdirectory used to fetch and parse its key list N times.
    dirs: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Directory>>>,
    /// Batched read-ahead of announced ranges, for a source that batches reads
    /// (a remote file); unused otherwise.
    readahead: std::sync::Mutex<ReadAhead>,
}

impl std::fmt::Debug for FileReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileReader")
            .field("bytes", &self.size)
            .field("version", &self.header.version)
            .field("keys", &self.root_dir.keys.len())
            .finish_non_exhaustive()
    }
}

impl FileReader {
    /// Open a ROOT file on disk.
    ///
    /// A file up to [`WHOLE_FILE_LIMIT`](Self::WHOLE_FILE_LIMIT) is read into
    /// memory, which is the fastest way to read many of its objects. A larger
    /// one is read with positioned reads, fetching only the byte ranges each
    /// object touches, so opening it costs a few small reads whatever its size.
    ///
    /// # Errors
    ///
    /// If the file cannot be read or is not a ROOT file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if std::fs::metadata(path)?.len() <= Self::WHOLE_FILE_LIMIT {
            Self::from_bytes(std::fs::read(path)?)
        } else {
            Self::from_source(Box::new(FileSource::open(path)?))
        }
    }

    /// The largest file [`open`](Self::open) reads into memory whole: 64 MiB.
    pub const WHOLE_FILE_LIMIT: u64 = 64 << 20;

    /// Open a ROOT file from a remote URL, reading only the byte ranges each
    /// object touches — never downloading the file whole, the way ROOT and
    /// uproot read remote data. The scheme selects the transport:
    ///
    /// - `http://` / `https://` — HTTP(S) byte-range requests (`http` feature).
    ///   The server must honor `Range` requests (`Accept-Ranges: bytes`).
    /// - `roots://` — `XRootD` over TLS (`xrootd-tls` feature), which also adds
    ///   token authentication with a WLCG bearer token from the environment.
    /// - `root://` — the `XRootD` protocol (`xrootd` feature), with `unix` auth
    ///   for public/world-readable files (e.g. `root://eospublic.cern.ch`).
    ///
    /// Requires the matching feature for the URL's scheme.
    ///
    /// # Errors
    ///
    /// If the URL's scheme is not supported by this build, the server cannot be reached, or
    /// what it serves is not a ROOT file.
    #[cfg(any(feature = "http", feature = "xrootd"))]
    pub fn open_url(url: &str) -> Result<Self> {
        #[cfg(feature = "xrootd")]
        if url.starts_with("root://") || url.starts_with("roots://") {
            return Self::from_source(Box::new(super::xrootd::XrootdSource::open(url)?));
        }
        #[cfg(feature = "http")]
        if url.starts_with("http://") || url.starts_with("https://") {
            return Self::from_source(Box::new(super::http::HttpSource::open(url)?));
        }
        Err(Error::Unsupported(format!(
            "unsupported or unavailable URL scheme (need the http/xrootd feature): {url:?}"
        )))
    }

    /// Parse a ROOT file already held in memory.
    ///
    /// # Errors
    ///
    /// If the bytes are not a ROOT file: a bad magic, or a header or top directory that cannot
    /// be read.
    pub fn from_bytes(data: impl Into<Bytes>) -> Result<Self> {
        Self::from_source(Box::new(BytesSource::new(data)))
    }

    /// Parse a ROOT file from an arbitrary [`ByteSource`]. The header, root
    /// directory, and key list are read at open; everything else is on demand.
    ///
    /// # Errors
    ///
    /// If the source cannot be read, or what it holds is not a ROOT file.
    pub fn from_source(source: Box<dyn ByteSource>) -> Result<Self> {
        let size = source.len();
        let head = source.read_at(0, usize::saturating_from(HEADER_PROBE.min(size)))?;
        let header = {
            let mut r = RBuffer::new(&head);
            FileHeader::read(&mut r)?
        };
        let root_dir = Directory::read_root(&*source, &header)?;
        Ok(Self {
            source,
            size,
            header,
            root_dir,
            registry: std::sync::OnceLock::new(),
            dirs: std::sync::Mutex::default(),
            readahead: std::sync::Mutex::default(),
        })
    }

    /// The parsed file header.
    pub fn header(&self) -> &FileHeader {
        &self.header
    }

    /// The root (top-level) directory.
    pub fn root_directory(&self) -> &Directory {
        &self.root_dir
    }

    /// The keys in the root directory.
    pub fn keys(&self) -> &[Key] {
        &self.root_dir.keys
    }

    /// The key at `path`, which is `"dir/sub/name;cycle"`: the directories are
    /// optional, and so is the cycle (the highest is taken, as ROOT's `Get`
    /// does). Every reader in the workspace addresses an object this way.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if a directory on the way, or the key, does not exist.
    pub fn key(&self, path: &str) -> Result<Key> {
        // A top-level key wins, so a key whose own name holds a `/` is reachable.
        if let Some(key) = find_key(&self.root_dir.keys, path) {
            return Ok(key.clone());
        }
        let missing = || Error::NotFound {
            what: "key",
            name: path.to_string(),
        };
        let (dir, name) = path.rsplit_once('/').ok_or_else(missing)?;
        if dir.split('/').all(str::is_empty) {
            return find_key(&self.root_dir.keys, name)
                .cloned()
                .ok_or_else(missing);
        }
        let dir = self.subdir(dir)?;
        find_key(&dir.keys, name).cloned().ok_or_else(missing)
    }

    /// Read the object at `path` (`"dir/sub/name;cycle"`) as a `T`.
    ///
    /// `T` is any readable type: a histogram, a graph, a matrix, a parameter, a
    /// collection. [`Value`](crate::Value) reads an object of any class from the file's
    /// streamer info, with no typed model.
    ///
    /// ```ignore
    /// let h: Hist1D = file.get("regions/signal/pt")?;
    /// let v: Value = file.get("anything")?;
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if nothing is stored at `path`,
    /// [`Error::WrongClass`] if it is not a `T`, and any decoding error.
    pub fn get<T: ReadRoot>(&self, path: &str) -> Result<T> {
        let key = self.key(path)?;
        let payload = self.key_payload(&key)?;
        let bytes =
            decompress_payload(&payload, key.obj_len as usize, format_args!("key {path:?}"))?;
        let object = Object {
            name: &key.name,
            class: &key.class_name,
            bytes: &bytes,
            key_len: Some(key.key_len as usize),
            file: Some(self),
            title: &key.title,
            within: None,
        };
        T::from_object(&object).unwrap_or_else(|| {
            Err(Error::WrongClass {
                name: Some(path.to_string()),
                found: key.class_name.clone(),
                expected: T::CLASS.to_string(),
            })
        })
    }

    /// Navigate into a subdirectory, returning its parsed [`Directory`] (with the
    /// keys it directly contains). A `/`-separated `path` descends through nested
    /// subdirectories (`"cal/pedestals"`); a plain name selects a single level.
    /// Errors if any path component has no such subdirectory key.
    ///
    /// Accepts both `TDirectory` (the in-memory class oxiroot writes) and
    /// `TDirectoryFile` (the class official ROOT C++ records on disk), so
    /// subdirectories of ROOT-written files are navigable.
    ///
    /// # Errors
    ///
    /// If a component of `path` is not a subdirectory, or a directory record cannot be
    /// read.
    ///
    /// A directory is read once per path and then shared: the `Arc` is the
    /// reader's own copy, not a clone of its key list.
    pub fn subdir(&self, path: &str) -> Result<std::sync::Arc<Directory>> {
        let norm: String = path
            .split('/')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let cache = |d| {
            self.dirs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(d)
                .cloned()
        };
        if let Some(dir) = cache(&norm) {
            return Ok(dir);
        }
        let mut current: Option<Directory> = None;
        for part in norm.split('/').filter(|p| !p.is_empty()) {
            current = Some(self.read_subdir(current.as_ref(), part)?);
        }
        let dir = std::sync::Arc::new(
            current
                .ok_or_else(|| Error::InvalidInput(format!("empty subdirectory path {path:?}")))?,
        );
        self.dirs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(norm, std::sync::Arc::clone(&dir));
        Ok(dir)
    }

    /// Read the subdirectory `name` directly inside `parent` (or the root
    /// directory when `parent` is `None`).
    fn read_subdir(&self, parent: Option<&Directory>, name: &str) -> Result<Directory> {
        let keys = parent.map_or(self.root_dir.keys.as_slice(), |d| d.keys.as_slice());
        let key = keys
            .iter()
            .find(|k| k.name == name && ClassKind::of(&k.class_name) == ClassKind::Directory)
            .ok_or_else(|| Error::NotFound {
                what: "subdirectory",
                name: name.to_string(),
            })?;
        Directory::read(
            &*self.source,
            key.payload_start(usize::saturating_from(self.size))? as u64,
        )
    }

    /// The file's free-segment list (informational).
    ///
    /// # Errors
    ///
    /// If the free list cannot be read or is malformed.
    pub fn free_segments(&self) -> Result<Vec<FreeSegment>> {
        read_free(&*self.source, &self.header)
    }

    /// The file's `TStreamerInfo` records (at `fSeekInfo`), parsed into a
    /// registry describing every class stored in the file. They are read and
    /// parsed once, on the first call.
    ///
    /// # Errors
    ///
    /// If the record cannot be read or parsed.
    pub fn streamer_registry(&self) -> Result<&StreamerRegistry> {
        if let Some(reg) = self.registry.get() {
            return Ok(reg);
        }
        let reg = match self.streamer_info_bytes()? {
            Some((object, keylen)) => parse_streamer_info(&object, keylen)?,
            None => StreamerRegistry::default(),
        };
        Ok(self.registry.get_or_init(|| reg))
    }

    /// Each class's streamer info as the file stores it, element bodies included,
    /// so it can be copied into another file.
    pub(crate) fn stored_streamer_infos(&self) -> Result<Vec<StoredInfo>> {
        let Some((object, keylen)) = self.streamer_info_bytes()? else {
            return Ok(Vec::new());
        };
        parse_stored_infos(&object, keylen)
    }

    /// The file's streamer-info record as stored: the decompressed
    /// `TList<TStreamerInfo>` bytes at `fSeekInfo` and the key length they were
    /// written under, or `None` if the file has no record. Only that one record
    /// is read. [`StreamerInfoList::parse_keyed`](crate::streamer_gen::StreamerInfoList::parse_keyed)
    /// reads the bytes, and they carry a file's streamer info across a rewrite;
    /// [`streamer_registry`](Self::streamer_registry) is the way to the parsed
    /// descriptions. Reached as [`raw::streamer_info_bytes`](crate::raw::streamer_info_bytes).
    pub(crate) fn streamer_info_bytes(&self) -> Result<Option<(Vec<u8>, usize)>> {
        if self.header.seek_info == 0 || self.header.nbytes_info == 0 {
            return Ok(None);
        }
        let win = self.read_at(self.header.seek_info, self.header.nbytes_info as usize)?;
        let key = Key::read(&mut RBuffer::new(&win))?;
        let payload = payload_in_window(&win, &key)?;
        let object = decompress_payload(payload, key.obj_len as usize, "streamer info")?;
        Ok(Some((object.into_owned(), key.key_len as usize)))
    }

    /// Total size of the file in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Read exactly `len` bytes at absolute `offset` from the underlying source.
    /// For a resident (in-memory / mmap) file this is a zero-copy slice; for a
    /// ranged local or remote file it fetches just that range. Object, `RNTuple`
    /// page, and `TTree` basket readers go through this so they touch only the
    /// bytes they need.
    ///
    /// # Errors
    ///
    /// [`Error::UnexpectedEof`] if the range runs past the end of the file, and any error
    /// reading the source.
    pub fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        if self.source.batches_reads() {
            let mut ra = self
                .readahead
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(bytes) = ra.take(&*self.source, offset, len)? {
                return Ok(bytes);
            }
        }
        self.source.read_at(offset, len)
    }

    /// Announce the ranges about to be read, in order, so a remote file fetches
    /// them in batches (an `XRootD` vector read, concurrent HTTP requests) rather
    /// than a round trip each. A no-op for a local or in-memory file. The `TTree`
    /// and `RNTuple` readers call it before reading a branch's baskets or a
    /// column's pages.
    pub fn plan_reads(&self, ranges: &[(u64, usize)]) {
        if self.source.batches_reads() {
            self.readahead
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .plan(ranges);
        }
    }

    /// Fetch a key's (possibly compressed) object payload — the `fNbytes −
    /// fKeyLen` bytes after its header. Bounds-checked against the file size;
    /// errors (never panics) on a malformed key.
    ///
    /// # Errors
    ///
    /// If the key points outside the file, or the source cannot be read.
    pub fn key_payload(&self, key: &Key) -> Result<Bytes> {
        let start = key.payload_start(usize::saturating_from(self.size))? as u64;
        self.read_at(start, key.checked_payload_len()?)
    }
}

/// A [`FileReader`] is itself a byte source (delegating to its backing), so page and
/// basket decoders can take a `&dyn ByteSource` and be unit-tested against a
/// bare in-memory buffer without a full file.
impl ByteSource for FileReader {
    fn len(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        Self::read_at(self, offset, len)
    }

    fn batches_reads(&self) -> bool {
        self.source.batches_reads()
    }

    fn plan_reads(&self, ranges: &[(u64, usize)]) {
        Self::plan_reads(self, ranges);
    }
}

/// Slice a wrapping key's payload out of a window fetched at the key's own
/// offset: the payload begins `fKeyLen` bytes into the record and runs for
/// `fNbytes − fKeyLen` bytes. Bounds-checked against the window.
fn payload_in_window<'a>(win: &'a [u8], key: &Key) -> Result<&'a [u8]> {
    let start = key.key_len as usize;
    let len = key.checked_payload_len()?;
    win.get(start..start + len)
        .ok_or_else(|| Error::Format(format!("key {:?}: payload runs past record", key.name)))
}

/// Split a ROOT object name into the name and the cycle it asks for, ROOT's
/// `"name;cycle"` form: `"h;2"` names cycle 2 of `h`, and a plain `"h"` names
/// whichever cycle is current (the highest). The separator is the last `;`, and
/// only when a cycle number follows it, so a name that carries a `;` of its own
/// stays whole.
///
/// ```
/// # use oxiroot_io_core::raw::split_cycle;
/// assert_eq!(split_cycle("h"), ("h", None));
/// assert_eq!(split_cycle("h;2"), ("h", Some(2)));
/// assert_eq!(split_cycle("a;b"), ("a;b", None));
/// ```
#[must_use]
pub fn split_cycle(name: &str) -> (&str, Option<u16>) {
    match name.rsplit_once(';') {
        Some((stem, cycle)) => match cycle.parse::<u16>() {
            Ok(cycle) => (stem, Some(cycle)),
            Err(_) => (name, None),
        },
        None => (name, None),
    }
}

/// The key `name` names among `keys`: the cycle it asks for, or the highest one.
/// A deleted key is never returned, whether or not its cycle was asked for.
#[must_use]
pub fn find_key<'a>(keys: &'a [Key], name: &str) -> Option<&'a Key> {
    let (stem, cycle) = split_cycle(name);
    let mut named = keys.iter().filter(|k| k.name == stem && !k.is_deleted());
    match cycle {
        Some(cycle) => named.find(|k| k.cycle == cycle),
        None => named.max_by_key(|k| k.cycle),
    }
}
