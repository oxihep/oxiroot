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
use std::ops::Range;

use crate::FileReader;
use crate::buffer::K_BYTE_COUNT_MASK;
use crate::buffer::{RBuffer, WBuffer};
use crate::error::{Error, Result};
use crate::object::TagReader;
use crate::object_io::{Object, ReadRoot, StreamerSet, WriteRoot};
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
/// A list read from a file can be written again as it is, whatever its members:
/// it keeps the streamer info that file stores for their classes (and the
/// classes those depend on), and a list stored without a name (as ROOT writes
/// them) takes its key's name.
#[derive(Debug, Clone)]
pub struct ObjList {
    kind: ListKind,
    name: String,
    /// Each member as `(class_name, streamed object body)`.
    members: Vec<(String, Vec<u8>)>,
    /// The streamer info the members added with [`add`](ObjList::add) need.
    streamers: StreamerSet,
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
        self.streamers.add(object);
        self.members
            .push((object.root_class(), object.to_root_bytes()));
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
        self.members.iter().map(|(c, _)| c.as_str())
    }

    /// Decode every member that is a `T`, in order, skipping the rest. For
    /// example `list.items::<Hist1D>()?` returns the histograms in the collection.
    pub fn items<T: ReadRoot>(&self) -> Result<Vec<T>> {
        self.members
            .iter()
            .filter_map(|(class, bytes)| T::from_object(&Object::member(class, bytes)))
            .collect()
    }
}

impl WriteRoot for ObjList {
    fn root_class(&self) -> String {
        match self.kind {
            ListKind::List => "TList".to_string(),
            ListKind::Array => "TObjArray".to_string(),
        }
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        ""
    }
    fn to_root_bytes(&self) -> Vec<u8> {
        let mut w = WBuffer::new();
        match self.kind {
            ListKind::List => {
                let obj = w.begin_object(5); // TList version 5
                write_object_base(&mut w, 0);
                w.string(&self.name); // fName
                w.be_i32(i32::saturating_from(self.members.len())); // nobjects
                for (class, body) in &self.members {
                    write_object_any(&mut w, class, body);
                    w.string(""); // the per-object option string
                }
                w.end_object(obj);
            }
            ListKind::Array => {
                let obj = w.begin_object(3); // TObjArray version 3
                write_object_base(&mut w, 0);
                w.string(&self.name); // fName
                w.be_i32(i32::saturating_from(self.members.len())); // nobjects
                w.be_i32(0); // fLowerBound
                for (class, body) in &self.members {
                    write_object_any(&mut w, class, body);
                }
                w.end_object(obj);
            }
        }
        w.into_vec()
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        member_streamer_classes(&self.streamers, self.members.iter().map(|(c, _)| c))
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
    set.classes().to_vec()
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
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for (class, body) in members {
        if !class.is_empty() {
            collect_stored(&infos, class, object_version(body), &mut seen, &mut out);
        }
    }
    out
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

/// Read a top-level `TList`/`TObjArray` object body into `(name, members)`, each
/// member as its class name and the byte range of its streamed body.
fn read_members(class: &str, object: &[u8], keylen: usize) -> Result<(String, Vec<MemberRange>)> {
    let kind = match class {
        "TList" => ListKind::List,
        "TObjArray" => ListKind::Array,
        other => {
            return Err(Error::WrongClass {
                name: String::new(),
                found: other.to_string(),
                expected: "TList or TObjArray".to_string(),
            });
        }
    };
    let mut r = RBuffer::new(object);
    r.read_version()?; // TList v5 / TObjArray v3
    read_object_base(&mut r)?;
    let name = r.string()?; // fName
    let n = r.be_i32()?.max(0);
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
                out.push((member_class, r.pos()..end));
                r.seek(end)?;
            }
            (_, Some(end)) => r.seek(end)?, // a null/parent slot
            _ => {}
        }
        if kind == ListKind::List {
            r.string()?; // the per-object option string (TList only)
        }
    }
    Ok((name, out))
}

fn decode_objlist(class: &str, object: &[u8], keylen: usize) -> Result<ObjList> {
    let kind = if class == "TObjArray" {
        ListKind::Array
    } else {
        ListKind::List
    };
    let (name, ranges) = read_members(class, object, keylen)?;
    let members = ranges
        .into_iter()
        .map(|(c, range)| (c, object[range].to_vec()))
        .collect();
    Ok(ObjList {
        kind,
        name,
        members,
        streamers: StreamerSet::default(),
    })
}

/// `list`, read from `file` under the key `key`, ready to be written again: named
/// by its key when the list itself has no name (ROOT writes lists that way), and
/// carrying the streamer info `file` stores for its members.
fn read_back(file: Option<&FileReader>, key: &str, mut list: ObjList) -> ObjList {
    if list.name.is_empty() {
        list.name = key.to_string();
    }
    let members = list.members.iter().map(|(c, b)| (c.as_str(), b.as_slice()));
    if let Some(file) = file {
        let classes = source_classes(file, members);
        list.streamers.add_classes(classes);
    }
    list
}

// --- ObjMap -----------------------------------------------------------------

/// One side of a [`ObjMap`] pair: a member's `(class_name, streamed body)`.
type MapEntry = (String, Vec<u8>);

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
        let key: &dyn WriteRoot = &ObjString::new(key);
        self.streamers.add(key);
        self.streamers.add(value);
        self.pairs.push((
            (key.root_class(), key.to_root_bytes()),
            (value.root_class(), value.to_root_bytes()),
        ));
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
            .filter_map(|((kc, kb), _)| {
                (kc == "TObjString")
                    .then(|| {
                        decode_tobjstring("", kc, kb)
                            .ok()
                            .map(|s| s.value().to_string())
                    })
                    .flatten()
            })
            .collect()
    }

    /// The value stored under the string `key`, decoded as `T` — `None` if no
    /// entry has that `ObjString` key or its value is not a `T`.
    #[must_use]
    pub fn get<T: ReadRoot>(&self, key: &str) -> Option<Result<T>> {
        self.pairs.iter().find_map(|((kc, kb), (vc, vb))| {
            if kc != "TObjString" {
                return None;
            }
            match decode_tobjstring("", kc, kb) {
                Ok(k) if k.value() == key => T::from_object(&Object::member(vc, vb)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Every value that is a `T`, in insertion order.
    pub fn values<T: ReadRoot>(&self) -> Result<Vec<T>> {
        self.pairs
            .iter()
            .filter_map(|(_, (vc, vb))| T::from_object(&Object::member(vc, vb)))
            .collect()
    }
}

impl WriteRoot for ObjMap {
    fn root_class(&self) -> String {
        "TMap".to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        ""
    }
    fn to_root_bytes(&self) -> Vec<u8> {
        let mut w = WBuffer::new();
        let obj = w.begin_object(3); // TMap version 3
        write_object_base(&mut w, 0);
        w.string(&self.name); // fName
        w.be_i32(i32::saturating_from(self.pairs.len())); // number of pairs
        for ((kc, kb), (vc, vb)) in &self.pairs {
            write_object_any(&mut w, kc, kb); // key object
            write_object_any(&mut w, vc, vb); // value object
        }
        w.end_object(obj);
        w.into_vec()
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        member_streamer_classes(
            &self.streamers,
            self.pairs.iter().flat_map(|((kc, _), (vc, _))| [kc, vc]),
        )
    }
}

/// Read one map entry (key or value): its class name and body byte range, or an
/// empty entry for a null slot. Advances the cursor past the object.
fn read_entry(r: &mut RBuffer<'_>, tags: &mut TagReader, object: &[u8]) -> Result<MapEntry> {
    let header = tags.read_header(r)?;
    let entry = match (header.class_name, header.end) {
        (Some(class), Some(end)) => {
            let body = object[r.pos()..end].to_vec();
            r.seek(end)?;
            (class, body)
        }
        (_, Some(end)) => {
            r.seek(end)?;
            (String::new(), Vec::new())
        }
        _ => (String::new(), Vec::new()),
    };
    Ok(entry)
}

fn decode_tmap(class: &str, object: &[u8], keylen: usize) -> Result<ObjMap> {
    if class != "TMap" {
        return Err(Error::WrongClass {
            name: String::new(),
            found: class.to_string(),
            expected: "TMap".to_string(),
        });
    }
    let mut r = RBuffer::new(object);
    r.read_version()?; // TMap version
    read_object_base(&mut r)?;
    let name = r.string()?; // fName
    let n = r.be_i32()?.max(0);

    let mut tags = TagReader::new(keylen);
    // `n` comes from the file; every pair takes at least two 4-byte headers.
    let mut pairs = Vec::with_capacity(usize::saturating_from(n).min(r.remaining() / 8));
    for _ in 0..n {
        let key = read_entry(&mut r, &mut tags, object)?;
        let value = read_entry(&mut r, &mut tags, object)?;
        pairs.push((key, value));
    }
    Ok(ObjMap {
        name,
        pairs,
        streamers: StreamerSet::default(),
    })
}

/// `map`, read from `file` under the key `key`, ready to be written again; see
/// [`read_back`].
fn map_read_back(file: Option<&FileReader>, key: &str, mut map: ObjMap) -> ObjMap {
    if map.name.is_empty() {
        map.name = key.to_string();
    }
    if let Some(file) = file {
        let entries = map
            .pairs
            .iter()
            .flat_map(|(key, value)| [key, value])
            .map(|(c, b)| (c.as_str(), b.as_slice()));
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
            Ok(read_back(object.file, object.name, list))
        })
    }
}

impl ReadRoot for ObjMap {
    const CLASS: &'static str = "TMap";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == "TMap").then(|| {
            let map = decode_tmap(object.class, object.bytes, collection_key_len(object)?)?;
            Ok(map_read_back(object.file, object.name, map))
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
