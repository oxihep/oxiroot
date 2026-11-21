//! Collections of objects stored under one key: [`ObjList`] (a ROOT `TList` or
//! `TObjArray`) and [`ObjMap`] (a keyed object → object map). Build them from any
//! writable objects and, on read, pull the members back out by type with
//! [`items`](ObjList::items) / [`get`](ObjMap::get).
//!
//! Members are serialized through ROOT's object protocol (each with a fresh class
//! tag), so ROOT reads what oxiroot writes; reading uses [`TagReader`] so the
//! class back-references ROOT emits for repeated member types resolve. (uproot
//! reads `TList`/`TObjArray` but has no `ObjMap` model — see [`ObjMap`].)

use crate::cast::SaturatingFrom;
use std::borrow::Cow;
use std::ops::Range;

use crate::FileReader;
use crate::buffer::K_BYTE_COUNT_MASK;
use crate::buffer::{RBuffer, WBuffer};
use crate::error::{Error, Result};
use crate::object::TagReader;
use crate::object_io::{KeyCheck, Object, ReadRoot, StreamerSet, WriteRoot};
use crate::streamer::{read_object_base, write_object_any, write_object_base};
use crate::streamer_gen::{ClassDef, collect_stored};

use super::scalars::{ObjString, decode_tobjstring, member_classes};

/// Whether an [`ObjList`] serializes as a `TList` (ordered, with per-element
/// options) or a `TObjArray` (an indexed array).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListKind {
    /// ROOT's `TList`.
    List,
    /// ROOT's `TObjArray`.
    Array,
}

/// A `TList` or `TObjArray` of objects stored under a single key. Build with
/// [`ObjList::new`], name it with [`named`](ObjList::named),
/// and [`add`](ObjList::add) any writable objects; read one back with
/// [`FileReader::get`] and extract members by type with
/// [`items`](ObjList::items).
///
/// A list read from a file can be written again as it is, whatever its members,
/// and with members added to it: it keeps the bytes it was read from and the
/// streamer info that file stores for its members' classes, and is named after
/// its key. Those bytes refer to each other by their position after the key,
/// so such a list must be stored under the name it was read with; renaming it,
/// or adding it to another collection, is an error when the file is written.
/// To put its members somewhere else, decode them with
/// [`items`](ObjList::items) and build a new list.
#[derive(Debug, Clone)]
pub struct ObjList {
    kind: ListKind,
    name: String,
    /// Each member as `(class_name, streamed object body)`.
    members: Vec<Member>,
    /// The streamer info the members added with [`add`](ObjList::add) need.
    streamers: StreamerSet,
    /// Set for a list read from a file; see [`Origin`].
    origin: Option<Origin>,
    /// Why the list cannot be written, once it holds something it cannot.
    blocked: Option<String>,
}

/// Two lists are equal when they hold the same members; where the members'
/// streamer info came from does not matter.
impl PartialEq for ObjList {
    fn eq(&self, other: &Self) -> bool {
        (self.kind, &self.name, &self.members) == (other.kind, &other.name, &other.members)
    }
}

impl ObjList {
    /// An empty collection of the given kind: a `TList` or a `TObjArray`.
    #[must_use]
    pub fn new(kind: ListKind) -> Self {
        Self {
            kind,
            name: String::new(),
            members: Vec::new(),
            streamers: StreamerSet::default(),
            origin: None,
            blocked: None,
        }
    }

    /// Set the key name this collection is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Add an object to the collection.
    // `add` is the natural builder verb here; it is not the arithmetic `Add::add`.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(mut self, object: &dyn WriteRoot) -> Self {
        self.blocked = self.blocked.or_else(|| unappendable(object));
        self.streamers.add(object);
        self.members.push(Member::of(object));
        self
    }

    /// Whether this is a `TList` or a `TObjArray`.
    #[must_use]
    pub fn kind(&self) -> ListKind {
        self.kind
    }
    /// The key name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }
    /// Whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    /// The class name of every member, in order.
    pub fn class_names(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|m| m.class.as_str())
    }

    /// Decode every member that is a `T`, in order, skipping the rest. For
    /// example `list.items::<Hist1D>()?` returns the histograms in the collection.
    ///
    /// # Errors
    ///
    /// The first error decoding a member of class `T`.
    pub fn items<T: ReadRoot>(&self) -> Result<Vec<T>> {
        self.members
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                T::from_object(&member_object(self.origin.as_ref(), i, &m.class, &m.body))
            })
            .collect()
    }
}

impl WriteRoot for ObjList {
    fn root_class(&self) -> Cow<'static, str> {
        match self.kind {
            ListKind::List => Cow::Borrowed("TList"),
            ListKind::Array => Cow::Borrowed("TObjArray"),
        }
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        self.origin.as_ref().map_or("", |o| &o.key_title)
    }
    fn key_check(&self) -> KeyCheck {
        match &self.origin {
            Some(origin) => origin.key_check(&self.name, self.blocked.as_deref()),
            None => self
                .blocked
                .clone()
                .map_or(KeyCheck::Any, KeyCheck::Unwritable),
        }
    }
    fn write_body(&self, w: &mut WBuffer) {
        if let Some(origin) = &self.origin {
            // As read, then whatever was added since.
            let added = &self.members[origin.ranges.len().min(self.members.len())..];
            let mut tail = WBuffer::new();
            for m in added {
                write_object_any(&mut tail, &m.class, &m.body);
                if self.kind == ListKind::List {
                    tail.string(""); // the per-object option string
                }
            }
            w.bytes(&origin.with_appended(&tail.into_vec(), added.len()));
            return;
        }
        match self.kind {
            ListKind::List => {
                let obj = w.begin_object(5); // TList version 5
                write_object_base(w, 0);
                w.string(&self.name); // fName
                w.be_i32(i32::saturating_from(self.members.len())); // nobjects
                for m in &self.members {
                    write_object_any(w, &m.class, &m.body);
                    w.string(""); // the per-object option string
                }
                w.end_object(obj);
            }
            ListKind::Array => {
                let obj = w.begin_object(3); // TObjArray version 3
                write_object_base(w, 0);
                w.string(&self.name); // fName
                w.be_i32(i32::saturating_from(self.members.len())); // nobjects
                w.be_i32(0); // fLowerBound
                for m in &self.members {
                    write_object_any(w, &m.class, &m.body);
                }
                w.end_object(obj);
            }
        }
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        member_streamer_classes(&self.streamers, self.members.iter().map(|m| &m.class))
    }
}

/// What a collection's members need described: the classes gathered when they
/// were added, plus those known by name (a collection read from a file keeps
/// only its members' class names).
fn member_streamer_classes<'a>(
    added: &StreamerSet,
    names: impl Iterator<Item = &'a String>,
) -> Vec<ClassDef<'static>> {
    let mut set = added.clone();
    for class in names {
        set.add_classes(member_classes(class));
    }
    set.into_classes()
}

/// The streamer info `file` stores for the classes of `members` (each a class
/// name and streamed body) and every class they depend on, dependencies first.
/// A collection read from `file` keeps them, so wherever it is written its
/// members are described, whatever their class. A file whose streamer info
/// cannot be read contributes nothing.
fn source_classes<'a>(
    file: &FileReader,
    members: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Vec<ClassDef<'static>> {
    let Ok(infos) = file.stored_streamer_infos() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (class, body) in members {
        collect_stored(&infos, class, object_version(body), &mut seen, &mut out);
    }
    out
}

/// One object in a collection: its class and its streamed body.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Member {
    class: String,
    body: Vec<u8>,
}

impl Member {
    fn of(object: &dyn WriteRoot) -> Self {
        Self {
            class: object.root_class().into_owned(),
            body: object.to_root_bytes(),
        }
    }
}

/// What a collection read from a file keeps of where it came from.
///
/// ROOT names a class in full once per buffer and by its position afterwards,
/// so the members of a stored collection refer into each other: the second
/// histogram of a list names `TList` (its function list) by pointing into the
/// first. A member cut out of the buffer no longer resolves those references,
/// and moved to another buffer it points at whatever lies there. So the
/// collection keeps the buffer whole: members are decoded in place, and it is
/// written back as it was read, with anything added since appended to it.
#[derive(Debug, Clone)]
struct Origin {
    /// The streamed collection, as read.
    object: Vec<u8>,
    /// The header length of the key it was read from: the references count
    /// from the key, so it must be stored under a header of the same length.
    key_len: usize,
    /// That key's name and title, which fix its header length.
    key_name: String,
    key_title: String,
    /// Where the member count is in `object`, and what it says. It can exceed
    /// the entries read: a null slot is counted and skipped.
    count_pos: usize,
    count: i32,
    /// The body of each entry read, in order, as a range of `object`.
    ranges: Vec<Range<usize>>,
}

impl Origin {
    /// `object` with `added` (already streamed entries) appended and counted.
    fn with_appended(&self, added: &[u8], n_added: usize) -> Vec<u8> {
        let mut out = self.object.clone();
        if n_added == 0 {
            return out;
        }
        out.extend_from_slice(added);
        let count = self.count.saturating_add(i32::saturating_from(n_added));
        if let Some(slot) = out.get_mut(self.count_pos..self.count_pos + 4) {
            slot.copy_from_slice(&count.to_be_bytes());
        }
        // The collection's own byte count covers what follows its first word.
        let head = u32::from_be_bytes([out[0], out[1], out[2], out[3]]);
        if head & K_BYTE_COUNT_MASK != 0 {
            let len = u32::saturating_from(out.len() - 4) | K_BYTE_COUNT_MASK;
            out[..4].copy_from_slice(&len.to_be_bytes());
        }
        out
    }

    /// The key a collection named `name` must be stored under, given whether
    /// anything it holds cannot be appended (`blocked`).
    fn key_check(&self, name: &str, blocked: Option<&str>) -> KeyCheck {
        if let Some(why) = blocked {
            return KeyCheck::Unwritable(why.to_string());
        }
        if name != self.key_name {
            return KeyCheck::Unwritable(format!(
                "it was read from the key {:?} and cannot be renamed: the class references \
                 inside it count from that key. Build a new collection from its decoded members",
                self.key_name
            ));
        }
        KeyCheck::HeaderLen(self.key_len)
    }
}

/// Why `object` cannot become a member of a collection, if it cannot: its bytes
/// are tied to a key of their own.
fn unappendable(object: &dyn WriteRoot) -> Option<String> {
    (object.key_check() != KeyCheck::Any).then(|| {
        format!(
            "it holds {} {:?}, which was read from a file as it is and can only be stored \
             under its own key",
            object.root_class(),
            object.root_name()
        )
    })
}

/// The class version at the head of a streamed object body, if it has one.
fn object_version(body: &[u8]) -> Option<i32> {
    let head: [u8; 6] = body.get(..6)?.try_into().ok()?;
    let count = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
    let version = if count & K_BYTE_COUNT_MASK != 0 {
        u16::from_be_bytes([head[4], head[5]])
    } else {
        u16::from_be_bytes([head[0], head[1]])
    };
    Some(i32::from(version))
}

/// A member's class name and the byte range of its streamed body within the
/// collection's object buffer.
type MemberRange = (String, Range<usize>);

/// Entry `i` of a collection as an [`Object`] to decode: in place in the
/// buffer it was read from when the collection has one and the entry came
/// from it, on its own otherwise.
fn member_object<'a>(
    origin: Option<&'a Origin>,
    i: usize,
    class: &'a str,
    bytes: &'a [u8],
) -> Object<'a> {
    match origin.and_then(|o| Some((o, o.ranges.get(i)?))) {
        Some((o, range)) if !range.is_empty() => {
            Object::member_in(class, &o.object, range.clone(), o.key_len)
        }
        _ => Object::member(class, bytes),
    }
}

/// The members of a stored collection, and where its member count is.
struct Members {
    name: String,
    count_pos: usize,
    count: i32,
    ranges: Vec<MemberRange>,
}

/// Read a top-level `TList`/`TObjArray` object body into `(name, members)`, each
/// member as its class name and the byte range of its streamed body.
fn read_members(class: &str, object: &[u8], keylen: usize) -> Result<Members> {
    let kind = match class {
        "TList" => ListKind::List,
        "TObjArray" => ListKind::Array,
        other => {
            return Err(Error::WrongClass {
                name: None,
                found: other.to_string(),
                expected: "TList or TObjArray".to_string(),
            });
        }
    };
    let mut r = RBuffer::new(object);
    r.read_version()?; // TList v5 / TObjArray v3
    read_object_base(&mut r)?;
    let name = r.string()?; // fName
    let count_pos = r.pos();
    let count = r.be_i32()?;
    let n = count.max(0);
    if kind == ListKind::Array {
        r.be_i32()?; // fLowerBound
    }

    let mut tags = TagReader::new(keylen);
    // `n` comes from the file; every entry takes at least a 4-byte header.
    let mut out = Vec::with_capacity(usize::saturating_from(n).min(r.remaining() / 4));
    for _ in 0..n {
        let header = tags.read_header(&mut r)?;
        match (header.class_name, header.end) {
            (Some(member_class), Some(end)) => {
                out.push((member_class.to_string(), r.pos()..end));
                r.seek(end)?;
            }
            (_, Some(end)) => r.seek(end)?, // a null/parent slot
            _ => {}
        }
        if kind == ListKind::List {
            r.skip_string()?; // the per-object option string (TList only)
        }
    }
    Ok(Members {
        name,
        count_pos,
        count,
        ranges: out,
    })
}

fn decode_objlist(class: &str, object: &[u8], keylen: usize) -> Result<ObjList> {
    let kind = if class == "TObjArray" {
        ListKind::Array
    } else {
        ListKind::List
    };
    let read = read_members(class, object, keylen)?;
    let members = read
        .ranges
        .iter()
        .map(|(class, range)| Member {
            class: class.clone(),
            body: object[range.clone()].to_vec(),
        })
        .collect();
    Ok(ObjList {
        kind,
        name: read.name,
        members,
        streamers: StreamerSet::default(),
        origin: Some(Origin {
            object: object.to_vec(),
            key_len: keylen,
            key_name: String::new(),
            key_title: String::new(),
            count_pos: read.count_pos,
            count: read.count,
            ranges: read.ranges.into_iter().map(|(_, range)| range).collect(),
        }),
        blocked: None,
    })
}

/// `list`, read from `file` under the key `key`, ready to be written again: named
/// by its key when the list itself has no name (ROOT writes lists that way), and
/// carrying the streamer info `file` stores for its members.
fn read_back(object: &Object<'_>, mut list: ObjList) -> ObjList {
    let (file, key) = (object.file, object.name);
    // The key is what the list is stored under, and what it must be stored
    // under again; ROOT usually leaves the list's own `fName` empty.
    list.name = key.to_string();
    if let Some(origin) = &mut list.origin {
        origin.key_name = key.to_string();
        origin.key_title = object.title.to_string();
    }
    let members = list
        .members
        .iter()
        .map(|m| (m.class.as_str(), m.body.as_slice()));
    if let Some(file) = file {
        let classes = source_classes(file, members);
        list.streamers.add_classes(classes);
    }
    list
}

// --- ObjMap -----------------------------------------------------------------

/// One side of a [`ObjMap`] pair: an object, or `None` for a null pointer.
type MapEntry = Option<Member>;

/// An `ObjMap` — ROOT's keyed map of object → object, stored under one key (the way
/// ROOT keeps string-keyed metadata). Build it with [`ObjMap::insert`]; read one back with
/// [`FileReader::get`] and look values up by string key with
/// [`get`](ObjMap::get).
///
/// Like an [`ObjList`], a map read from a file keeps the streamer info that file
/// stores for its keys' and values' classes, and takes its key's name if it has
/// none of its own.
///
/// Note: uproot has no `TMap` model, so an `ObjMap` is unreadable there (ROOT's own
/// `TMap`s share this). ROOT C++ reads what oxiroot writes, and oxiroot reads
/// ROOT's `TMap`s.
#[derive(Debug, Clone, Default)]
#[doc(alias = "TMap")]
pub struct ObjMap {
    name: String,
    pairs: Vec<(MapEntry, MapEntry)>,
    /// The streamer info the inserted entries need.
    streamers: StreamerSet,
    /// Set for a map read from a file; see [`Origin`]. Its ranges are each
    /// pair's key, then its value.
    origin: Option<Origin>,
    /// Why the map cannot be written, once it holds something it cannot.
    blocked: Option<String>,
}

/// Two maps are equal when they hold the same entries.
impl PartialEq for ObjMap {
    fn eq(&self, other: &Self) -> bool {
        (&self.name, &self.pairs) == (&other.name, &other.pairs)
    }
}

impl ObjMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the key name this map is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Insert a `value` under a string `key` (stored as an `ObjString`, the usual
    /// map-key type).
    #[must_use]
    pub fn insert(mut self, key: &str, value: &dyn WriteRoot) -> Self {
        self.blocked = self.blocked.or_else(|| unappendable(value));
        let key: &dyn WriteRoot = &ObjString::new(key);
        self.streamers.add(key);
        self.streamers.add(value);
        self.pairs
            .push((Some(Member::of(key)), Some(Member::of(value))));
        self
    }

    /// The key name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }
    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The string (`ObjString`) keys, in insertion order; keys of other types
    /// are skipped.
    #[must_use]
    pub fn string_keys(&self) -> Vec<String> {
        self.pairs
            .iter()
            .filter_map(|(key, _)| {
                let key = key.as_ref().filter(|k| k.class == "TObjString")?;
                decode_tobjstring("", &key.class, &key.body)
                    .ok()
                    .map(|s| s.value().to_string())
            })
            .collect()
    }

    /// The value stored under the string `key`, decoded as `T` — `None` if no
    /// entry has that `ObjString` key or its value is not a `T`.
    #[must_use]
    pub fn get<T: ReadRoot>(&self, key: &str) -> Option<Result<T>> {
        self.pairs.iter().enumerate().find_map(|(i, (k, v))| {
            let k = k.as_ref().filter(|k| k.class == "TObjString")?;
            match decode_tobjstring("", &k.class, &k.body) {
                Ok(name) if name.value() == key => {
                    let v = v.as_ref()?;
                    T::from_object(&member_object(
                        self.origin.as_ref(),
                        2 * i + 1,
                        &v.class,
                        &v.body,
                    ))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Every value that is a `T`, in insertion order.
    ///
    /// # Errors
    ///
    /// The first error decoding a value of class `T`.
    pub fn values<T: ReadRoot>(&self) -> Result<Vec<T>> {
        self.pairs
            .iter()
            .enumerate()
            .filter_map(|(i, (_, v))| {
                let v = v.as_ref()?;
                T::from_object(&member_object(
                    self.origin.as_ref(),
                    2 * i + 1,
                    &v.class,
                    &v.body,
                ))
            })
            .collect()
    }
}

impl WriteRoot for ObjMap {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Borrowed("TMap")
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        self.origin.as_ref().map_or("", |o| &o.key_title)
    }
    fn key_check(&self) -> KeyCheck {
        match &self.origin {
            Some(origin) => origin.key_check(&self.name, self.blocked.as_deref()),
            None => self
                .blocked
                .clone()
                .map_or(KeyCheck::Any, KeyCheck::Unwritable),
        }
    }
    fn write_body(&self, w: &mut WBuffer) {
        if let Some(origin) = &self.origin {
            // As read, then whatever was inserted since.
            let added = &self.pairs[(origin.ranges.len() / 2).min(self.pairs.len())..];
            let mut tail = WBuffer::new();
            for (key, value) in added {
                write_entry(&mut tail, key.as_ref());
                write_entry(&mut tail, value.as_ref());
            }
            w.bytes(&origin.with_appended(&tail.into_vec(), added.len()));
            return;
        }
        let obj = w.begin_object(3); // TMap version 3
        write_object_base(w, 0);
        w.string(&self.name); // fName
        w.be_i32(i32::saturating_from(self.pairs.len())); // number of pairs
        for (key, value) in &self.pairs {
            write_entry(w, key.as_ref()); // key object
            write_entry(w, value.as_ref()); // value object
        }
        w.end_object(obj);
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        member_streamer_classes(
            &self.streamers,
            self.pairs
                .iter()
                .flat_map(|(key, value)| [key, value])
                .flatten()
                .map(|m| &m.class),
        )
    }
}

/// Write one side of a pair: the object, or a null pointer where there is
/// none.
fn write_entry(w: &mut WBuffer, entry: Option<&Member>) {
    match entry {
        Some(m) => write_object_any(w, &m.class, &m.body),
        None => w.be_u32(0),
    }
}

/// Read one map entry (key or value): its class name and body byte range, or an
/// empty entry for a null slot. Advances the cursor past the object.
fn read_entry(
    r: &mut RBuffer<'_>,
    tags: &mut TagReader,
    object: &[u8],
) -> Result<(MapEntry, Range<usize>)> {
    let header = tags.read_header(r)?;
    let entry = match (header.class_name, header.end) {
        (Some(class), Some(end)) => {
            let range = r.pos()..end;
            let body = object[range.clone()].to_vec();
            r.seek(end)?;
            (
                Some(Member {
                    class: class.to_string(),
                    body,
                }),
                range,
            )
        }
        (_, Some(end)) => {
            r.seek(end)?;
            (None, 0..0)
        }
        _ => (None, 0..0),
    };
    Ok(entry)
}

fn decode_tmap(class: &str, object: &[u8], keylen: usize) -> Result<ObjMap> {
    if class != "TMap" {
        return Err(Error::WrongClass {
            name: None,
            found: class.to_string(),
            expected: "TMap".to_string(),
        });
    }
    let mut r = RBuffer::new(object);
    r.read_version()?; // TMap version
    read_object_base(&mut r)?;
    let name = r.string()?; // fName
    let count_pos = r.pos();
    let count = r.be_i32()?;
    let n = count.max(0);

    let mut tags = TagReader::new(keylen);
    // `n` comes from the file; every pair takes at least two 4-byte headers.
    let capacity = usize::saturating_from(n).min(r.remaining() / 8);
    let mut pairs = Vec::with_capacity(capacity);
    let mut ranges = Vec::with_capacity(2 * capacity);
    for _ in 0..n {
        let (key, key_range) = read_entry(&mut r, &mut tags, object)?;
        let (value, value_range) = read_entry(&mut r, &mut tags, object)?;
        pairs.push((key, value));
        ranges.extend([key_range, value_range]);
    }
    Ok(ObjMap {
        name,
        pairs,
        streamers: StreamerSet::default(),
        origin: Some(Origin {
            object: object.to_vec(),
            key_len: keylen,
            key_name: String::new(),
            key_title: String::new(),
            count_pos,
            count,
            ranges,
        }),
        blocked: None,
    })
}

/// `map`, read from `file` under the key `key`, ready to be written again; see
/// [`read_back`].
fn map_read_back(object: &Object<'_>, mut map: ObjMap) -> ObjMap {
    let (file, key) = (object.file, object.name);
    map.name = key.to_string();
    if let Some(origin) = &mut map.origin {
        origin.key_name = key.to_string();
        origin.key_title = object.title.to_string();
    }
    if let Some(file) = file {
        let entries = map
            .pairs
            .iter()
            .flat_map(|(key, value)| [key, value])
            .flatten()
            .map(|m| (m.class.as_str(), m.body.as_slice()));
        let classes = source_classes(file, entries);
        map.streamers.add_classes(classes);
    }
    map
}

/// The key length a collection needs to resolve its members' class
/// back-references; a collection nested in another has none of its own.
fn collection_key_len(object: &Object<'_>) -> Result<usize> {
    object.key_len.ok_or_else(|| {
        Error::Unsupported(format!(
            "a {} nested inside another collection cannot be read on its own",
            object.class
        ))
    })
}

impl ReadRoot for ObjList {
    const CLASS: &'static str = "TList or TObjArray";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        matches!(object.class, "TList" | "TObjArray").then(|| {
            let list = decode_objlist(object.class, object.bytes, collection_key_len(object)?)?;
            Ok(read_back(object, list))
        })
    }
}

impl ReadRoot for ObjMap {
    const CLASS: &'static str = "TMap";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == "TMap").then(|| {
            let map = decode_tmap(object.class, object.bytes, collection_key_len(object)?)?;
            Ok(map_read_back(object, map))
        })
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    use crate::buffer::K_BYTE_COUNT_MASK;

    /// A collection claiming `i32::MAX` entries in a few bytes is an error
    /// once the buffer runs out, not an attempt to reserve gigabytes first.
    #[test]
    fn a_forged_entry_count_does_not_reserve_memory() {
        let mut body = Vec::new();
        body.extend_from_slice(&(K_BYTE_COUNT_MASK | 30).to_be_bytes()); // TList version header
        body.extend_from_slice(&5u16.to_be_bytes());
        body.extend_from_slice(&1u16.to_be_bytes()); // TObject version
        body.extend_from_slice(&0u32.to_be_bytes()); // fUniqueID
        body.extend_from_slice(&0x0300_0000u32.to_be_bytes()); // fBits
        body.push(0); // fName ""
        body.extend_from_slice(&i32::MAX.to_be_bytes()); // fSize
        body.resize(34, 0);
        assert!(read_members("TList", &body, 0).is_err());
    }
}
