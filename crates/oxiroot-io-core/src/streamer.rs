//! Generic ROOT object-streaming helpers shared by class readers.
//!
//! ROOT serializes most objects with a `{byte-count, version}` header (see
//! [`RBuffer::read_version`]); the byte count lets a reader consume the members
//! it understands and then seek to the object's end, ignoring members added by
//! later schema versions. `TObject` is the notable exception — it is written
//! with a 2-byte version and no byte count.

use crate::buffer::{Patch, RBuffer, WBuffer};
use crate::cast::SaturatingFrom;
use crate::error::Result;

/// `TObject::kIsReferenced` — when set in `fBits`, a 2-byte process-id follows.
const K_IS_REFERENCED: u32 = 0x10;

/// A streamed `TObject` base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(alias = "TObject")]
#[non_exhaustive]
pub struct ObjectBase {
    /// `TObject` streamer version.
    pub version: u16,
    /// Unique id (usually 0).
    pub unique_id: u32,
    /// Status bits.
    pub bits: u32,
}

/// Read a `TObject` base: a 2-byte version, `fUniqueID`, `fBits`, and a 2-byte
/// process-id reference iff `kIsReferenced` is set. Leaves the cursor just past
/// the `TObject` data.
pub fn read_object_base(r: &mut RBuffer<'_>) -> Result<ObjectBase> {
    let vh = r.read_version()?; // version only, no byte count
    let unique_id = r.be_u32()?;
    let bits = r.be_u32()?;
    if bits & K_IS_REFERENCED != 0 {
        let _pidf = r.be_u16()?;
    }
    Ok(ObjectBase {
        version: vh.version,
        unique_id,
        bits,
    })
}

/// A streamed `Named` (name + title).
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(alias = "TNamed")]
#[non_exhaustive]
pub struct Named {
    /// Object name (`fName`).
    pub name: String,
    /// Object title (`fTitle`).
    pub title: String,
}

/// Read a `Named` base (`TObject` + `fName` + `fTitle`), then seek to the end
/// of the `Named` record as given by its byte count.
pub fn read_named(r: &mut RBuffer<'_>) -> Result<Named> {
    let vh = r.read_version()?;
    let _obj = read_object_base(r)?;
    let name = r.string()?;
    let title = r.string()?;
    if let Some(end) = vh.end {
        r.seek(end)?;
    }
    Ok(Named { name, title })
}

/// Read a versioned object's header and seek straight to its end, skipping the
/// payload entirely. Returns the version. Errors if the object had no byte
/// count (and therefore no known end to skip to).
pub fn skip_versioned(r: &mut RBuffer<'_>) -> Result<u16> {
    let vh = r.read_version()?;
    match vh.end {
        Some(end) => {
            r.seek(end)?;
            Ok(vh.version)
        }
        None => Err(crate::error::Error::Unsupported(
            "cannot skip a versioned object that carries no byte count".into(),
        )),
    }
}

/// Write a `TObject` base: a 2-byte version, `fUniqueID = 0`, and `fBits`.
/// (No byte count, matching ROOT's `TObject::Streamer`.)
pub fn write_object_base(w: &mut WBuffer, bits: u32) {
    w.be_u16(1); // TObject version
    w.be_u32(0); // fUniqueID
    w.be_u32(bits); // fBits
}

/// Write an embedded object the way ROOT's `WriteObjectAny` does: a byte count,
/// `kNewClassTag`, the NUL-terminated class name, then `body` (the object's own
/// streamed bytes, as produced by [`WriteRoot`](crate::WriteRoot)).
pub fn write_object_any(w: &mut WBuffer, class: &str, body: &[u8]) {
    write_object_ptr(w, class, |w| w.bytes(body));
}

/// Write an object held by pointer: a byte count, `kNewClassTag`, the
/// NUL-terminated class name, then the body `write` produces. Every object gets
/// a fresh class tag, never a back-reference, so what is written does not
/// depend on where in the buffer it lands.
pub fn write_object_ptr(w: &mut WBuffer, class: &str, write: impl FnOnce(&mut WBuffer)) {
    let count = begin_object_any(w, class);
    write(w);
    end_object_any(w, count);
}

/// The first half of [`write_object_ptr`], for a body that cannot be written
/// from one closure: the byte-count placeholder (returned, for
/// [`end_object_any`]), `kNewClassTag` and the class name.
pub fn begin_object_any(w: &mut WBuffer, class: &str) -> Patch<4> {
    let count = w.reserve::<4>();
    w.be_u32(crate::object::K_NEW_CLASS_TAG);
    w.bytes(class.as_bytes());
    w.u8(0);
    count
}

/// Finish an object begun with [`begin_object_any`]: fill in its byte count,
/// which covers everything after the count word.
pub fn end_object_any(w: &mut WBuffer, count: Patch<4>) {
    let inner = u32::saturating_from(w.len() - count.offset() - 4);
    w.patch_be_u32(count, inner | crate::buffer::K_BYTE_COUNT_MASK);
}

/// Write a `Named` base (a byte-counted `TObject` + `fName` + `fTitle`).
pub fn write_named(w: &mut WBuffer, bits: u32, name: &str, title: &str) {
    let tok = w.begin_object(1); // Named version 1
    write_object_base(w, bits);
    w.string(name);
    w.string(title);
    w.end_object(tok);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::WBuffer;

    #[test]
    fn reads_tnamed_with_tobject() {
        // Build: Named{ TObject(v1, uid=0, bits=0), fName="hi", fTitle="" }.
        let mut inner = WBuffer::new();
        // TObject: version (no byte count) + uniqueID + bits.
        inner.be_u16(1);
        inner.be_u32(0);
        inner.be_u32(0x0300_0000);
        inner.string("hi");
        inner.string("");
        let inner = inner.into_vec();

        let mut w = WBuffer::new();
        let tok = w.begin_object(1); // Named version 1 + byte count
        w.bytes(&inner);
        w.end_object(tok);
        let bytes = w.into_vec();

        let mut r = RBuffer::new(&bytes);
        let named = read_named(&mut r).unwrap();
        assert_eq!(named.name, "hi");
        assert_eq!(named.title, "");
        assert_eq!(r.pos(), bytes.len());
    }
}
