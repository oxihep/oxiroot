//! [`FileWriter`]: composing a ROOT file from several objects, optionally in
//! subdirectories, or appending them to an existing file.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use crate::ClassKind;
use crate::Compression;
use crate::error::{Error, Result, decompress_payload};
use crate::object_io::{KeyCheck, ObjectRecord, StreamerSet, WriteInto, WriteRoot};
use crate::streamer_gen::{ClassDef, StreamerInfoList};

use super::container::{ContainerWriter, DirId, KSTART_BIG_FILE};
use super::key::Key;
use super::reader::FileReader;
use super::reader::split_cycle;

/// One key in a directory being built: a streamed object, or a multi-record
/// object laid out when the file is written.
enum Entry {
    Object(ObjectRecord),
    Records(Box<dyn WriteInto>),
}

impl Entry {
    fn class(&self) -> String {
        match self {
            Self::Object(r) => r.class_name.clone(),
            Self::Records(o) => o.root_class().into_owned(),
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Object(r) => &r.name,
            Self::Records(o) => o.root_name(),
        }
    }

    fn place(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        match self {
            Self::Object(r) => place_record(file, dir, r),
            Self::Records(o) => o.write_into(file, dir),
        }
    }
}

/// Store `record` under its own key in `dir`.
fn place_record<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    record: &ObjectRecord,
) -> Result<()> {
    let (class, name, title) = (&record.class_name, &record.name, &record.title);
    let key_len = Key::header_len(class, name, title, file.is_big());
    record.key_check.check(class, name, key_len)?;
    file.place_key(dir, class, name, title, &record.object)
        .map(drop)
}

/// The objects a streaming writer stores next to its dataset: histograms,
/// graphs, parameters, each under a key of its own in the top directory.
///
/// A streaming writer owns its file from the first cluster on, so these are
/// held (they are small) and placed when the writer finishes.
#[derive(Default)]
pub struct Objects {
    records: Vec<ObjectRecord>,
    streamers: StreamerSet,
}

impl Objects {
    /// Take `object`, serialized now.
    pub fn add<T: WriteRoot + ?Sized>(&mut self, object: &T) {
        self.streamers.add_classes(object.streamer_classes());
        self.records.push(ObjectRecord {
            class_name: object.root_class().into_owned(),
            name: object.root_name().to_string(),
            title: object.root_title().to_string(),
            object: object.to_root_bytes(),
            key_check: object.key_check(),
        });
    }

    /// The `TStreamerInfo` entries the objects need.
    #[must_use]
    pub fn classes(&self) -> &[ClassDef<'static>] {
        self.streamers.classes()
    }

    /// Store every object in the top directory of `file`, which already
    /// holds a key named `dataset`.
    ///
    /// # Errors
    ///
    /// An unnamed object, two keys sharing a name, an object too large for
    /// its record, or a failed write.
    pub fn place<W: Write + Seek>(
        &self,
        file: &mut ContainerWriter<W>,
        dataset: &str,
    ) -> Result<()> {
        let mut seen = HashSet::from([dataset]);
        for record in &self.records {
            if record.name.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "cannot write an unnamed {} in the top directory; give it a key name with \
                     `.named(\"...\")`",
                    record.class_name
                )));
            }
            if !seen.insert(record.name.as_str()) {
                return Err(Error::DuplicateName {
                    name: record.name.clone(),
                    location: "the top directory".to_string(),
                });
            }
            place_record(file, DirId::TOP, record)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for Objects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.records.iter().map(|r| &r.name))
            .finish()
    }
}

/// The keys of one directory being built, and what they need described.
#[derive(Default)]
struct Entries {
    entries: Vec<Entry>,
    streamers: StreamerSet,
}

/// One object on its way into a file, with the streamer info it needs: what
/// [`Storable::into_stored`] hands to [`FileWriter::add`].
pub struct Stored {
    entry: Entry,
    streamers: StreamerSet,
}

impl Stored {
    /// An object stored as several records (a `TTree`, an `RNTuple`). It is laid
    /// out when the file is written, so the file takes it over.
    pub fn records(object: impl WriteInto + 'static) -> Self {
        let mut streamers = StreamerSet::default();
        streamers.add_records(&object);
        Self {
            entry: Entry::Records(Box::new(object)),
            streamers,
        }
    }
}

impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stored")
            .field("class", &self.entry.class())
            .field("name", &self.entry.name())
            .finish_non_exhaustive()
    }
}

/// Something [`FileWriter::add`] stores in a file.
///
/// A reference to any [`WriteRoot`] object is one: a histogram, a graph, a
/// matrix, a parameter. It is serialized when added, so it only needs to be
/// borrowed. A dataset (a `TTree` or an `RNTuple`) is one by value, because its
/// records are laid out when the file is written.
pub trait Storable {
    /// The object as the file stores it.
    fn into_stored(self) -> Stored;
}

impl<T: WriteRoot + ?Sized> Storable for &T {
    fn into_stored(self) -> Stored {
        let mut streamers = StreamerSet::default();
        streamers.add_classes(self.streamer_classes());
        Stored {
            entry: Entry::Object(ObjectRecord {
                class_name: self.root_class().into_owned(),
                name: self.root_name().to_string(),
                title: self.root_title().to_string(),
                object: self.to_root_bytes(),
                key_check: self.key_check(),
            }),
            streamers,
        }
    }
}

impl Entries {
    fn add(&mut self, object: impl Storable) {
        let stored = object.into_stored();
        self.streamers.extend(&stored.streamers);
        self.entries.push(stored.entry);
    }

    /// Reject keys that cannot be addressed: an empty name (it could never be
    /// looked up) or two keys sharing a name (the second would silently shadow
    /// the first on read). `subdirs` names the subdirectories created in the same
    /// directory, which share its namespace.
    fn check_names(&self, subdirs: &[&str], location: &str) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for entry in &self.entries {
            let name = entry.name();
            if name.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "cannot write an unnamed {} in {location}; give it a key name with `.named(\"...\")`",
                    entry.class()
                )));
            }
            if !seen.insert(name) {
                return Err(Error::DuplicateName {
                    name: name.to_string(),
                    location: location.to_string(),
                });
            }
        }
        for &name in subdirs {
            if name.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "cannot create a subdirectory with an empty name in {location}"
                )));
            }
            if !seen.insert(name) {
                return Err(Error::DuplicateName {
                    name: name.to_string(),
                    location: location.to_string(),
                });
            }
        }
        Ok(())
    }

    fn place(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        for entry in &self.entries {
            entry.place(file, dir)?;
        }
        Ok(())
    }
}

/// Composes a new ROOT file from several objects, optionally organised into
/// subdirectories, and writes it with [`write`](FileWriter::write).
/// [`FileReader`](crate::FileReader) reads files, and [`FileUpdater`] changes an
/// existing one.
///
/// Any mix of writable types goes in one file through [`add`](FileWriter::add):
///
/// ```no_run
/// use oxiroot_io_core::{FileWriter, ObjString, Parameter};
/// let lumi = Parameter::f64(137.5).named("lumi");
/// let label = ObjString::new("2024 run").named("label");
/// let cut = Parameter::f32(25.0).named("pt_min");
/// FileWriter::create("out.root")
///     .add(&lumi)
///     .add(&label)
///     .dir("cuts", |d| d.add(&cut)) // a TDirectory holding `pt_min`
///     .write()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
///
/// The file is compressed with Zstd level 5 unless
/// [`compression`](FileWriter::compression) says otherwise.
#[doc(alias = "RootFile", alias = "TFile")]
#[must_use = "a FileWriter does nothing until `.write(...)` is called"]
pub struct FileWriter {
    path: std::path::PathBuf,
    /// `Some` in append mode (the existing file bytes); `None` for a fresh file.
    existing: Option<bytes::Bytes>,
    top: Entries,
    dirs: Vec<DirNode>,
    /// Names to take out of the top directory, `"h"` or `"h;2"`, before the
    /// added objects are written (update mode only).
    deleted: Vec<String>,
    /// Keep only the highest cycle of each name in the top directory.
    purge: bool,
    /// Rewrite the file from what survives, instead of appending to it.
    compact: bool,
    compression: Compression,
    /// The size past which the 64-bit container form is written.
    threshold: u64,
}

impl FileWriter {
    /// Start a fresh ROOT file at `path` (any existing file is overwritten on
    /// [`write`](FileWriter::write)).
    pub fn create(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            existing: None,
            top: Entries::default(),
            dirs: Vec::new(),
            deleted: Vec::new(),
            purge: false,
            compact: false,
            compression: Compression::default(),
            threshold: KSTART_BIG_FILE,
        }
    }

    /// Add an object to the file's top directory: `.add(&histogram)` for an
    /// object stored under one key, `.add(dataset)` for a `TTree` or an
    /// `RNTuple`. See [`Storable`].
    // `add` is the natural builder verb here; it is not the arithmetic `Add::add`.
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, object: impl Storable) -> Self {
        self.top.add(object);
        self
    }

    /// Add a `TDirectory` named `name` holding the objects added inside `build`
    /// (e.g. one directory per analysis region).
    pub fn dir(
        mut self,
        name: impl Into<String>,
        build: impl FnOnce(SubdirWriter) -> SubdirWriter,
    ) -> Self {
        self.dirs.push(SubdirWriter::build(name, build));
        self
    }

    /// Compress the file's records with `compression`. The default is Zstd
    /// level 5.
    pub const fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Switch to the 64-bit container form once the file would exceed
    /// `threshold` bytes rather than ROOT's ~2 GiB ([`KSTART_BIG_FILE`]). A
    /// threshold of 0 always writes the 64-bit form, which is how tests check
    /// readers against it; the form is otherwise chosen from the file's size.
    #[doc(hidden)]
    pub const fn large_above(mut self, threshold: u64) -> Self {
        self.threshold = threshold;
        self
    }

    /// Build the file and write it to the path, all or nothing. A file that
    /// grows past ~2 GiB is written in ROOT's 64-bit ("big") container form
    /// automatically.
    ///
    /// # Errors
    ///
    /// An unnamed object, two objects sharing a name in one directory, an
    /// object too large for its record, or the file not being writable.
    pub fn write(self) -> Result<()> {
        let bytes = self.to_bytes()?;
        write_atomic(&self.path, &bytes)?;
        Ok(())
    }

    /// The bytes [`write`](FileWriter::write) would write, without writing them.
    ///
    /// # Errors
    ///
    /// As [`write`](FileWriter::write), short of touching the file system.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.build(self.compression, self.threshold)
    }

    /// Which of `keys` survive this builder's deletions and purge, in their
    /// original order. Errors on a name the file does not hold, so a typo is not
    /// silently a no-op.
    fn surviving<'k>(&self, keys: &'k [Key]) -> Result<Vec<&'k Key>> {
        let mut kept: Vec<&Key> = keys.iter().filter(|k| !k.is_deleted()).collect();
        for spec in &self.deleted {
            let (name, cycle) = split_cycle(spec);
            let before = kept.len();
            kept.retain(|key| key.name != name || cycle.is_some_and(|c| key.cycle != c));
            if kept.len() == before {
                return Err(Error::NotFound {
                    what: "key to delete",
                    name: spec.clone(),
                });
            }
        }
        if self.purge {
            let current = kept.iter().fold(HashMap::new(), |mut top, key| {
                let cycle: &mut u16 = top.entry(key.name.as_str()).or_default();
                *cycle = (*cycle).max(key.cycle);
                top
            });
            kept.retain(|key| current.get(key.name.as_str()) == Some(&key.cycle));
        }
        Ok(kept)
    }

    /// Take the deleted names, and the superseded cycles when purging, out of
    /// the top directory's key list.
    fn apply_deletions(&self, c: &mut ContainerWriter<Cursor<Vec<u8>>>) -> Result<()> {
        if self.deleted.is_empty() && !self.purge {
            return Ok(());
        }
        // The keys to keep, by position: nothing is cloned to decide.
        let keys = c.keys(DirId::TOP)?;
        let kept: HashSet<(&str, u16)> = self
            .surviving(keys)?
            .iter()
            .map(|key| (key.name.as_str(), key.cycle))
            .collect();
        let keep: Vec<bool> = keys
            .iter()
            .map(|key| kept.contains(&(key.name.as_str(), key.cycle)))
            .collect();
        let mut keep = keep.into_iter();
        c.drop_keys(DirId::TOP, |_| keep.next().unwrap_or(true))?;
        Ok(())
    }

    fn build(&self, compression: Compression, threshold: u64) -> Result<Vec<u8>> {
        // Reject unnamed / clashing keys before writing — loudly, instead of
        // ROOT's silent shadow-on-read.
        let subdir_names: Vec<&str> = self.dirs.iter().map(|d| d.name.as_str()).collect();
        self.top.check_names(&subdir_names, "the top directory")?;
        let mut streamers = self.top.streamers.clone();
        for dir in &self.dirs {
            dir.check_names(&dir.name)?;
            dir.collect_streamers(&mut streamers);
        }
        let file_name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.root");
        let layout = |c: &mut ContainerWriter<Cursor<Vec<u8>>>| {
            // Deletions come first, so an object added in the same pass takes
            // the next cycle after what is left.
            self.apply_deletions(c)?;
            self.top.place(c, DirId::TOP)?;
            // When appending to a file that has streamer info, only the generated
            // classes it lacks are added (readers know the histogram family).
            c.place_streamer_info(&[], streamers.classes())?;
            for dir in &self.dirs {
                dir.place(c, DirId::TOP)?;
            }
            Ok(())
        };
        if !self.deleted.is_empty() || self.purge {
            if self.existing.is_none() {
                return Err(Error::InvalidInput(
                    "delete and purge need a file to work on".to_string(),
                ));
            }
            for name in &self.deleted {
                if name.trim().is_empty() {
                    return Err(Error::InvalidInput(
                        "cannot delete a key with an empty name".to_string(),
                    ));
                }
            }
        }
        match (&self.existing, self.compact) {
            (Some(existing), true) => {
                self.rebuild(existing, file_name, compression, threshold, &streamers)
            }
            (Some(existing), false) => {
                if !self.dirs.is_empty() {
                    return Err(Error::Unsupported(
                        "adding new subdirectories while appending is not supported \
                         (append adds objects to the top directory; existing \
                         subdirectories are preserved)"
                            .to_string(),
                    ));
                }
                ContainerWriter::build_append(existing, file_name, compression, threshold, layout)
            }
            (None, true) => Err(Error::InvalidInput(
                "compact needs a file to rewrite".to_string(),
            )),
            (None, false) => ContainerWriter::build(file_name, compression, threshold, layout),
        }
    }

    /// Write the file afresh, holding what survives the deletions and the purge
    /// plus whatever this builder adds, so the space the rest held is given up.
    fn rebuild(
        &self,
        existing: &bytes::Bytes,
        file_name: &str,
        compression: Compression,
        threshold: u64,
        streamers: &StreamerSet,
    ) -> Result<Vec<u8>> {
        let file = FileReader::from_bytes(existing.clone())?;
        let top = CopiedDir::read(&file, &self.surviving(file.keys())?, "")?;
        // The file describes what it still holds: the classes of the copied
        // objects, with what those depend on, plus the added objects' own.
        let described = match file.streamer_info_bytes()? {
            Some((list, key_len)) => {
                let classes = top.class_names();
                let names: Vec<&str> = classes.iter().map(String::as_str).collect();
                StreamerInfoList::parse_keyed(&list, key_len)
                    .map_or_default(|list| list.classes_for(&names))
            }
            None => Vec::new(),
        };
        let mut classes = described;
        classes.extend(streamers.classes().iter().cloned());

        ContainerWriter::build(file_name, compression, threshold, |c| {
            top.place(c, DirId::TOP)?;
            self.top.place(c, DirId::TOP)?;
            c.place_streamer_info(&[], &classes)?;
            for dir in &self.dirs {
                dir.place(c, DirId::TOP)?;
            }
            Ok(())
        })
    }
}

/// Changes an existing ROOT file: adds objects to its top directory, takes
/// some out, and optionally rewrites it to give back their space.
///
/// ```no_run
/// # use oxiroot_io_core::{FileUpdater, Parameter};
/// # let extra = Parameter::i32(3).named("extra");
/// FileUpdater::open("out.root")?
///     .delete("scratch")
///     .add(&extra)
///     .write()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
///
/// Objects are appended in place, so what the file already holds never moves,
/// and an added object whose name matches an existing one lands at a higher
/// cycle, as in ROOT. Subdirectories and `RNTuple`s already in the file are
/// kept. The file is replaced in one step: a failure part-way leaves it as it
/// was.
#[doc(alias = "UPDATE", alias = "TFile")]
#[must_use = "a FileUpdater does nothing until `.write()` is called"]
#[derive(Debug)]
pub struct FileUpdater(FileWriter);

impl FileUpdater {
    /// Open the ROOT file at `path` to change it.
    ///
    /// # Errors
    ///
    /// If the file cannot be read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let existing = std::fs::read(path.as_ref())?;
        let mut writer = FileWriter::create(path);
        writer.existing = Some(existing.into());
        Ok(Self(writer))
    }

    /// Add an object to the file's top directory; see [`FileWriter::add`].
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, object: impl Storable) -> Self {
        self.0.top.add(object);
        self
    }

    /// Take `name` out of the file's top directory: every cycle of it, or the
    /// one an explicit `"name;2"` asks for. Writing fails if the file holds no
    /// such key, rather than quietly doing nothing.
    ///
    /// The record stays where it is and the file does not shrink; what changes
    /// is that the directory no longer lists it, so nothing reads it. This is
    /// ROOT's `TFile::Delete`. [`compact`](Self::compact) takes the space back.
    pub fn delete(mut self, name: impl Into<String>) -> Self {
        self.0.deleted.push(name.into());
        self
    }

    /// Keep only the current (highest) cycle of each name in the file's top
    /// directory, dropping the older ones: ROOT's `TFile::Purge`. Like
    /// [`delete`](Self::delete), the file does not shrink.
    pub const fn purge(mut self) -> Self {
        self.0.purge = true;
        self
    }

    /// Write the file afresh from what survives, rather than appending to it, so
    /// that deleted and superseded objects stop taking up space. The objects
    /// keep their names, titles, cycle order and subdirectories; the file
    /// describes the classes it still holds.
    ///
    /// A `TTree` or an `RNTuple` is more than its key (its baskets and pages
    /// sit elsewhere in the file), so compacting a file that holds one is
    /// refused rather than half-done. Merge those with `oxiroot::hadd` instead.
    ///
    /// ```no_run
    /// # use oxiroot_io_core::FileUpdater;
    /// # fn main() -> oxiroot_io_core::Result<()> {
    /// FileUpdater::open("out.root")?
    ///     .purge()      // drop the superseded cycles …
    ///     .compact()    // … and take back the space they held
    ///     .write()?;
    /// # Ok(()) }
    /// ```
    pub const fn compact(mut self) -> Self {
        self.0.compact = true;
        self
    }

    /// Add a `TDirectory` named `name`; see [`FileWriter::dir`]. A new
    /// subdirectory needs the file rewritten, so this goes with
    /// [`compact`](Self::compact): without it, writing fails.
    pub fn dir(
        mut self,
        name: impl Into<String>,
        build: impl FnOnce(SubdirWriter) -> SubdirWriter,
    ) -> Self {
        self.0.dirs.push(SubdirWriter::build(name, build));
        self
    }

    /// Compress the records this update writes with `compression`. The default
    /// is Zstd level 5.
    pub const fn compression(mut self, compression: Compression) -> Self {
        self.0.compression = compression;
        self
    }

    /// See [`FileWriter::large_above`].
    #[doc(hidden)]
    pub const fn large_above(mut self, threshold: u64) -> Self {
        self.0.threshold = threshold;
        self
    }

    /// Apply the changes and replace the file, all or nothing.
    ///
    /// # Errors
    ///
    /// As [`FileWriter::write`]; also a name to delete that the file does not
    /// hold, a new subdirectory without [`compact`](Self::compact), or
    /// compacting a file that holds a `TTree` or an `RNTuple`.
    pub fn write(self) -> Result<()> {
        self.0.write()
    }

    /// The bytes [`write`](Self::write) would write, without writing them.
    ///
    /// # Errors
    ///
    /// As [`write`](Self::write), short of touching the file system.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.0.to_bytes()
    }
}

/// The temporary file beside `path` that is written in its place, and the
/// file `path` names (the one a symbolic link points at, not the link).
fn temporary_beside(path: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target.file_name().unwrap_or_default();
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    (target.with_file_name(tmp_name), target)
}

/// Make a finished temporary file `tmp` the file `target`: its bytes reach
/// the disk, it takes the permissions of the file it replaces, then its name.
fn put_in_place(file: &std::fs::File, tmp: &Path, target: &Path) -> std::io::Result<()> {
    file.sync_all()?;
    if let Ok(existing) = std::fs::metadata(target) {
        std::fs::set_permissions(tmp, existing.permissions())?;
    }
    std::fs::rename(tmp, target)
}

/// Write `bytes` as the file `path`, all or nothing: they go to a temporary
/// file beside it, which reaches the disk and then takes its name. An existing
/// file — the one an update is rewriting — is replaced in one step, so a
/// failure part-way leaves it as it was instead of cut short.
///
/// # Errors
///
/// If the temporary file cannot be written or renamed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if path.file_name().is_none() {
        return std::fs::write(path, bytes);
    }
    let (tmp, target) = temporary_beside(path);
    let written = std::fs::File::create(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        put_in_place(&file, &tmp, &target)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// A file being written in place of `path`, for a writer that streams to
/// disk: the writer fills [`file`](Self::file), a temporary file beside
/// `path`, and [`commit`](Self::commit) makes it `path` once the file is
/// complete. Dropped without a commit — the writer failed, or was dropped part
/// way — it deletes the temporary file, so `path` keeps what it held.
#[derive(Debug)]
pub struct PendingFile {
    tmp: std::path::PathBuf,
    target: std::path::PathBuf,
    file: Option<std::fs::File>,
}

impl PendingFile {
    /// Start a file to take the place of `path`.
    ///
    /// # Errors
    ///
    /// If the temporary file cannot be created.
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let (tmp, target) = temporary_beside(path.as_ref());
        let file = std::fs::File::create(&tmp)?;
        Ok(Self {
            tmp,
            target,
            file: Some(file),
        })
    }

    /// The temporary file to write, as a handle of its own.
    ///
    /// # Errors
    ///
    /// If the handle cannot be duplicated.
    pub fn file(&self) -> std::io::Result<std::fs::File> {
        self.file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the pending file is gone"))?
            .try_clone()
    }

    /// Make the written file `path`: its bytes reach the disk, then it takes
    /// the name (and the permissions of the file it replaces).
    ///
    /// # Errors
    ///
    /// If it cannot be synced or renamed; the temporary file is deleted.
    pub fn commit(mut self) -> std::io::Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| std::io::Error::other("the pending file is gone"))?;
        let done = put_in_place(&file, &self.tmp, &self.target);
        if done.is_ok() {
            // Nothing left for `drop` to delete.
            self.tmp = std::path::PathBuf::new();
        }
        done
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.tmp.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// A directory being copied from one file into another: the objects it holds,
/// read out whole, and the directories inside it.
struct CopiedDir {
    name: String,
    objects: Vec<ObjectRecord>,
    dirs: Vec<Self>,
}

impl CopiedDir {
    /// Read `keys` — the surviving keys of the directory at `path` — and every
    /// directory below it, out of `file`.
    fn read(file: &FileReader, keys: &[&Key], path: &str) -> Result<Self> {
        let mut objects = Vec::new();
        let mut dirs = Vec::new();
        for key in keys {
            if ClassKind::of(&key.class_name) == ClassKind::Directory {
                let below = if path.is_empty() {
                    key.name.clone()
                } else {
                    format!("{path}/{}", key.name)
                };
                let sub = file.subdir(&below)?;
                let live: Vec<&Key> = sub.keys.iter().filter(|k| !k.is_deleted()).collect();
                dirs.push(Self::read(file, &live, &below)?);
                continue;
            }
            // A TTree or an RNTuple is more than its key: its baskets and pages
            // sit elsewhere in the file, and copying the key alone would leave
            // them behind.
            if ClassKind::holds_data_outside_its_key(&key.class_name) {
                return Err(Error::Unsupported(format!(
                    "cannot compact a file that holds {} {:?}: its records live outside its \
                     key. Merge the file with oxiroot::hadd instead",
                    key.class_name, key.name
                )));
            }
            let payload = file.key_payload(key)?;
            let object = decompress_payload(
                &payload,
                key.obj_len as usize,
                format_args!("key {:?}", key.name),
            )?;
            objects.push(ObjectRecord {
                class_name: key.class_name.clone(),
                name: key.name.clone(),
                title: key.title.clone(),
                object: object.into_owned(),
                // Copied as it is: valid under a key of the same header length.
                key_check: KeyCheck::HeaderLen(key.key_len as usize),
            });
        }
        Ok(Self {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            objects,
            dirs,
        })
    }

    /// The class of every object copied here and below.
    fn class_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.objects.iter().map(|o| o.class_name.clone()).collect();
        for dir in &self.dirs {
            names.extend(dir.class_names());
        }
        names.sort();
        names.dedup();
        names
    }

    /// Write these objects into directory `dir`, then the directories below it.
    fn place(&self, c: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        for object in &self.objects {
            let key_len =
                Key::header_len(&object.class_name, &object.name, &object.title, c.is_big());
            object
                .key_check
                .check(&object.class_name, &object.name, key_len)?;
            c.place_key(
                dir,
                &object.class_name,
                &object.name,
                &object.title,
                &object.object,
            )?;
        }
        for below in &self.dirs {
            let id = c.mkdir(dir, &below.name)?;
            below.place(c, id)?;
            c.close_dir(id)?;
        }
        Ok(())
    }
}

/// One subdirectory in a file being composed: what it holds, and the
/// subdirectories inside it.
struct DirNode {
    name: String,
    entries: Entries,
    dirs: Vec<Self>,
}

impl DirNode {
    /// Check this directory's names and those of every directory below it,
    /// naming each by the path a reader would use.
    fn check_names(&self, path: &str) -> Result<()> {
        let subdirs: Vec<&str> = self.dirs.iter().map(|d| d.name.as_str()).collect();
        self.entries
            .check_names(&subdirs, &format!("subdirectory {path:?}"))?;
        for dir in &self.dirs {
            dir.check_names(&format!("{path}/{}", dir.name))?;
        }
        Ok(())
    }

    /// Every streamer set in this directory and below it.
    fn collect_streamers(&self, into: &mut StreamerSet) {
        into.extend(&self.entries.streamers);
        for dir in &self.dirs {
            dir.collect_streamers(into);
        }
    }

    /// Write this directory inside `parent`: its own objects, then the
    /// directories inside it, which must exist before it closes.
    fn place(&self, c: &mut ContainerWriter<Cursor<Vec<u8>>>, parent: DirId) -> Result<()> {
        let id = c.mkdir(parent, &self.name)?;
        self.entries.place(c, id)?;
        for dir in &self.dirs {
            dir.place(c, id)?;
        }
        c.close_dir(id)?;
        Ok(())
    }
}

/// A subdirectory (a `TDirectory`) being composed inside a [`FileWriter`]; see
/// [`FileWriter::dir`]. The methods take and return `self`, so return the
/// `SubdirWriter` from the `dir` closure.
#[doc(
    alias = "SubdirBuilder",
    alias = "Dir",
    alias = "TDirectory",
    alias = "mkdir"
)]
#[must_use = "SubdirWriter methods consume self; return it from the closure"]
pub struct SubdirWriter {
    entries: Entries,
    dirs: Vec<DirNode>,
}

impl SubdirWriter {
    /// Add an object to this subdirectory; see [`FileWriter::add`].
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, object: impl Storable) -> Self {
        self.entries.add(object);
        self
    }

    /// Add a `TDirectory` named `name` inside this one, holding what `build`
    /// adds — the same call as [`FileWriter::dir`], so directories nest as deep
    /// as you write them.
    pub fn dir(mut self, name: impl Into<String>, build: impl FnOnce(Self) -> Self) -> Self {
        self.dirs.push(Self::build(name, build));
        self
    }

    /// Run `build` over a fresh subdirectory and take what it composed.
    fn build(name: impl Into<String>, build: impl FnOnce(Self) -> Self) -> DirNode {
        let dir = build(Self {
            entries: Entries::default(),
            dirs: Vec::new(),
        });
        DirNode {
            name: name.into(),
            entries: dir.entries,
            dirs: dir.dirs,
        }
    }
}

impl std::fmt::Debug for FileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWriter")
            .field("path", &self.path)
            .field("purge", &self.purge)
            .field("compact", &self.compact)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SubdirWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubdirWriter")
            .field("subdirs", &self.dirs.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod atomic_tests {
    use super::write_atomic;

    #[test]
    fn a_file_is_replaced_whole_and_no_temporary_is_left() {
        let dir = std::env::temp_dir().join(format!("oxiroot-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.root");

        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second, longer");

        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["out.root"], "only the file itself remains");

        // A directory that does not exist is an error, and leaves nothing.
        assert!(write_atomic(&dir.join("missing/out.root"), b"x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
