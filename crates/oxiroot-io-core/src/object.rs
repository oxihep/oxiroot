//! ROOT's generic object protocol (`ReadObjectAny`).
//!
//! Embedded objects are prefixed with a byte count and a class tag that is
//! either a new class name (`kNewClassTag`), a back-reference to a
//! previously-named class (`kClassMask`), or a null/parent marker. Class
//! references are keyed by the object's buffer offset **plus the key length**
//! (ROOT reads objects from a key with `origin = -fKeylen`). This machinery is
//! shared by the streamer-info parser, `TObjArray` reading, and `TTree`.

use crate::cast::SaturatingFrom;
use std::collections::HashMap;

use crate::buffer::{K_BYTE_COUNT_MASK, RBuffer};
use crate::error::{Error, Result};

/// The class tag that introduces a class by name (ROOT's `kNewClassTag`): a
/// NUL-terminated class name follows it.
pub const K_NEW_CLASS_TAG: u32 = 0xFFFF_FFFF;
/// Set in a class tag that refers back to a class already named in the buffer
/// (ROOT's `kClassMask`); the other bits say where.
pub const K_CLASS_MASK: u32 = 0x8000_0000;
const K_MAP_OFFSET: i64 = 2;

/// The outcome of reading an object's `{byte-count, class-tag}` header.
#[derive(Debug)]
#[non_exhaustive]
pub struct RecordHeader {
    /// The resolved class name, or `None` for a null/parent slot.
    pub class_name: Option<String>,
    /// Absolute buffer offset one past the object, when a byte count was present.
    pub end: Option<usize>,
    /// Where the object this slot points back at was written, when the slot held
    /// a reference to an object already streamed in this buffer. The slot itself
    /// has no body: a reader that wants the object reads it again from there.
    pub back_ref: Option<ObjectRef>,
}

/// An object already streamed in this buffer, named by a later reference to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectRef {
    /// The object's class.
    pub class_name: String,
    /// The buffer offset its body starts at (just past its own header).
    pub pos: usize,
}

/// The header of an object held by pointer, read without a class-tag map.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PointerHeader {
    /// The object's class, when the header names it. `None` when the header
    /// refers back to a class named earlier in the buffer, which only a
    /// [`TagReader`] can resolve; the caller then knows the class from context.
    pub class_name: Option<String>,
    /// The buffer offset one past the object.
    pub end: usize,
}

/// Read the `{byte count}{class tag}` header of an object held by pointer,
/// leaving the cursor at the object's body. `None` for a null pointer.
///
/// This is for a member whose class the reader already knows (an axis's label
/// list, an efficiency's two histograms): it does not follow class
/// back-references. Where the class has to be found out, use a [`TagReader`].
///
/// # Errors
///
/// [`Error::Format`] if the byte count runs past the buffer, and the buffer's
/// errors if the header is cut short.
pub fn read_pointer_header(r: &mut RBuffer<'_>) -> Result<Option<PointerHeader>> {
    let count = r.be_u32()?;
    if count == 0 {
        return Ok(None);
    }
    let len = usize::saturating_from(count & !(K_BYTE_COUNT_MASK | K_CLASS_MASK));
    let end = r
        .pos()
        .checked_add(len)
        .filter(|&end| end <= r.len())
        .ok_or_else(|| Error::Format(format!("object byte count {len} runs past the buffer")))?;
    let class_name = if r.be_u32()? == K_NEW_CLASS_TAG {
        Some(read_cstring(r)?)
    } else {
        None
    };
    Ok(Some(PointerHeader { class_name, end }))
}

/// Read a NUL-terminated class name (the `kNewClassTag` path).
fn read_cstring(r: &mut RBuffer<'_>) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        let b = r.u8()?;
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)
}

/// Resolves the generic object protocol's class tags, tracking back-references.
/// Construct one per top-level object with that object's key length, then call
/// [`read_header`](TagReader::read_header) for each embedded object.
pub struct TagReader {
    refs: HashMap<i64, String>,
    objects: HashMap<i64, ObjectRef>,
    seq: i64,
    keylen: i64,
}

impl TagReader {
    /// Begin resolving tags for an object read from a key whose header is
    /// `keylen` bytes (ROOT keys objects relative to `-keylen`).
    #[must_use]
    pub fn new(keylen: usize) -> Self {
        Self {
            refs: HashMap::new(),
            objects: HashMap::new(),
            seq: 0,
            keylen: i64::saturating_from(keylen),
        }
    }

    /// Read a `ReadObjectAny`-style header, resolving the class name and the
    /// object's end offset. Leaves the cursor at the object body.
    ///
    /// The end offset comes from the file's byte count, so it is checked here,
    /// once, for every caller: it must lie between the cursor and the end of
    /// the buffer.
    ///
    /// # Errors
    ///
    /// [`Error::Format`] if the byte count points outside that range, and the
    /// buffer's errors if the header is truncated.
    pub fn read_header(&mut self, r: &mut RBuffer<'_>) -> Result<RecordHeader> {
        let header = self.read_header_unchecked(r)?;
        if let Some(end) = header.end
            && (end > r.len() || end < r.pos())
        {
            return Err(Error::Format(format!(
                "object byte count ends at {end}, outside {}..={}",
                r.pos(),
                r.len()
            )));
        }
        Ok(header)
    }

    fn read_header_unchecked(&mut self, r: &mut RBuffer<'_>) -> Result<RecordHeader> {
        let beg = r.pos();
        let bcnt_raw = r.be_u32()?;

        let (versioned, tag, bcnt) =
            if (bcnt_raw & K_BYTE_COUNT_MASK) == 0 || bcnt_raw == K_NEW_CLASS_TAG {
                (false, bcnt_raw, 0u32)
            } else {
                let tag = r.be_u32()?;
                (true, tag, bcnt_raw & !K_BYTE_COUNT_MASK)
            };
        let end = if versioned {
            Some(beg + 4 + bcnt as usize)
        } else {
            None
        };

        if tag & K_CLASS_MASK == 0 {
            // Null (0), parent (1), or an object back-reference: a pointer to an
            // object already streamed in this buffer (a split TBranchElement's
            // fLeaves references its sub-branches' leaves; a PolyHist's fBins
            // references the bins its fCells grid wrote in full). The slot holds
            // no body, so it reads as "no object"; `back_ref` says where the
            // object it points at was written, for a reader that wants it.
            let back_ref = if tag > 1 {
                self.objects.get(&i64::from(tag)).cloned()
            } else {
                None
            };
            Ok(RecordHeader {
                class_name: None,
                end,
                back_ref,
            })
        } else if tag == K_NEW_CLASS_TAG {
            let classname = read_cstring(r)?;
            // Register the class at the tag's displacement (+ keylen + offset).
            if versioned {
                let start_disp = i64::saturating_from(beg + 4) + self.keylen;
                self.refs
                    .insert(start_disp + K_MAP_OFFSET, classname.clone());
            } else {
                self.seq += 1;
                self.refs.insert(self.seq, classname.clone());
            }
            self.map_object(beg, &classname, r.pos());
            Ok(RecordHeader {
                class_name: Some(classname),
                end,
                back_ref: None,
            })
        } else {
            let refpos = i64::from(tag & !K_CLASS_MASK);
            let classname = match self.refs.get(&refpos) {
                Some(name) => name.clone(),
                // Not seen by this reader: the class was named inside an object
                // it skipped over. The reference says where; read it there.
                None => {
                    let name = self.class_declared_at(r, refpos).ok_or_else(|| {
                        Error::Format(format!("unknown class-tag reference {refpos}"))
                    })?;
                    self.refs.insert(refpos, name.clone());
                    name
                }
            };
            self.map_object(beg, &classname, r.pos());
            Ok(RecordHeader {
                class_name: Some(classname),
                end,
                back_ref: None,
            })
        }
    }

    /// The class a `kNewClassTag` declares at the buffer position the
    /// reference `refpos` names, if there is one there. Leaves the cursor where
    /// it was.
    fn class_declared_at(&self, r: &mut RBuffer<'_>, refpos: i64) -> Option<String> {
        let tag_at = usize::try_from(refpos - K_MAP_OFFSET - self.keylen).ok()?;
        let here = r.pos();
        let name = (|| {
            r.seek(tag_at).ok()?;
            (r.be_u32().ok()? == K_NEW_CLASS_TAG).then_some(())?;
            let name = read_cstring(r).ok()?;
            // A class name, not whatever bytes happened to follow a matching word.
            (!name.is_empty() && name.bytes().all(|b| b.is_ascii_graphic() || b == b' '))
                .then_some(name)
        })();
        r.seek(here).ok()?;
        name
    }

    /// Record an object that starts at `beg` (the offset of its header) and whose
    /// body starts at `pos`, so that a later reference to it resolves. ROOT keys
    /// an object by the same displacement it keys a class by, taken at the
    /// header's first word rather than at the class tag.
    fn map_object(&mut self, beg: usize, class_name: &str, pos: usize) {
        self.objects.insert(
            i64::saturating_from(beg) + self.keylen + K_MAP_OFFSET,
            ObjectRef {
                class_name: class_name.to_string(),
                pos,
            },
        );
    }
}

impl std::fmt::Debug for TagReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagReader")
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod byte_count_tests {
    use super::*;

    /// A record header whose byte count runs past the buffer, or ends before
    /// the header itself, is a format error (callers used to slice with it).
    #[test]
    fn read_header_rejects_a_byte_count_outside_the_buffer() {
        let mut past = Vec::new();
        past.extend_from_slice(&(K_BYTE_COUNT_MASK | 1000).to_be_bytes());
        past.extend_from_slice(&K_NEW_CLASS_TAG.to_be_bytes());
        past.extend_from_slice(b"TObject\0");
        let mut r = RBuffer::new(&past);
        assert!(matches!(
            TagReader::new(0).read_header(&mut r),
            Err(Error::Format(_))
        ));

        let mut short = Vec::new();
        short.extend_from_slice(&(K_BYTE_COUNT_MASK | 4).to_be_bytes()); // ends before the class name
        short.extend_from_slice(&K_NEW_CLASS_TAG.to_be_bytes());
        short.extend_from_slice(b"TObject\0");
        let mut r = RBuffer::new(&short);
        assert!(matches!(
            TagReader::new(0).read_header(&mut r),
            Err(Error::Format(_))
        ));
    }

    #[test]
    fn read_version_rejects_a_byte_count_outside_the_buffer() {
        let mut past = Vec::new();
        past.extend_from_slice(&(K_BYTE_COUNT_MASK | 16).to_be_bytes());
        past.extend_from_slice(&1u16.to_be_bytes());
        assert!(matches!(
            RBuffer::new(&past).read_version(),
            Err(Error::Format(_))
        ));
        let mut fits = past.clone();
        fits.extend_from_slice(&[0u8; 14]);
        assert!(RBuffer::new(&fits).read_version().is_ok());
    }
}
