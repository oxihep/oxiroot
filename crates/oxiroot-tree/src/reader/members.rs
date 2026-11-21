//! Reading an object member by member, following the file's `TStreamerInfo`.
use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::ElementKind;
use oxiroot_io_core::streamer_types::*;

use oxiroot_io_core::raw::{StreamerElement, read_named, read_object_base};
use oxiroot_io_core::{Error, RBuffer, Result, StreamerRegistry};

/// A scalar or array member captured while walking a class's streamer elements.
/// Integers (any width) are widened to `i64`; arrays keep their element values.
pub(super) enum MemberVal {
    Int(i64),
    /// A floating member (e.g. `fWeight`). Captured for completeness so the walk
    /// stays generic; no member the tree reader consumes is a float yet.
    Float(#[allow(dead_code)] f64),
    IntArray(Vec<i64>),
    Str(String),
}

impl MemberVal {
    pub(super) fn int(&self) -> i64 {
        match self {
            Self::Int(v) => *v,
            _ => 0,
        }
    }
    pub(super) fn ints(&self) -> &[i64] {
        match self {
            Self::IntArray(v) => v,
            _ => &[],
        }
    }
    pub(super) fn str(&self) -> &str {
        match self {
            Self::Str(s) => s,
            _ => "",
        }
    }
}

/// Members captured from one object, keyed by streamer-element name.
pub(super) type Members = std::collections::HashMap<String, MemberVal>;

pub(super) fn member_int(m: &Members, name: &str) -> i64 {
    m.get(name).map_or(0, MemberVal::int)
}
/// The integer member `name` of a `class` object, which every such object
/// has. Its absence means the file's streamer info does not describe the
/// class as this reader knows it, and a default would be a wrong answer: a
/// tree without `fEntries` is not a tree of no entries.
pub(super) fn required_int(m: &Members, name: &str, class: &str) -> Result<i64> {
    match m.get(name) {
        Some(MemberVal::Int(v)) => Ok(*v),
        _ => Err(Error::Format(format!(
            "the file's description of {class} has no integer member {name}"
        ))),
    }
}
pub(super) fn member_str(m: &Members, name: &str) -> String {
    m.get(name).map_or(String::new(), |v| v.str().to_string())
}

/// The on-disk width (bytes) and float-ness of a basic streamer type code
/// (`fType` < 20). `None` for codes whose on-disk encoding we don't handle
/// (e.g. `Double32`/`Float16`), so the walker errors rather than misparsing.
fn basic_kind(t: i32) -> Option<(usize, bool)> {
    Some(match t {
        K_CHAR | K_UCHAR | K_BOOL => (1, false),
        K_SHORT | K_USHORT => (2, false),
        K_INT | K_UINT | K_COUNTER | K_BITS => (4, false),
        K_LONG | K_ULONG | K_LONG64 | K_ULONG64 => (8, false),
        K_FLOAT => (4, true),
        K_DOUBLE => (8, true),
        _ => return None,
    })
}

/// Read one basic value of the given width, widening integers to `i64`.
fn read_basic(r: &mut RBuffer<'_>, width: usize, is_float: bool) -> Result<MemberVal> {
    Ok(match (width, is_float) {
        (1, false) => MemberVal::Int(i64::from(r.u8()?.cast_signed())),
        (2, false) => MemberVal::Int(i64::from(r.be_i16()?)),
        (4, false) => MemberVal::Int(i64::from(r.be_i32()?)),
        (8, false) => MemberVal::Int(r.be_i64()?),
        (4, true) => MemberVal::Float(f64::from(r.be_f32()?)),
        (_, true) => MemberVal::Float(r.be_f64()?),
        _ => MemberVal::Int(0),
    })
}

fn unsupported_element(el: &StreamerElement) -> Error {
    Error::Unsupported(format!(
        "streamer element {:?} has unsupported type code {} ({})",
        el.name, el.el_type, el.type_name
    ))
}

/// Walk a class's streamer `elements`, reading each member from `r`. Scalar and
/// array members are captured into `out` by name (counted pointer arrays use the
/// already-read counter named by `fCountName`); base classes are read in place
/// (recursing through their own streamer info); object members are handed to
/// `on_object`. Reading stops after the element named `stop_after` (the caller
/// then seeks to the object end), so a long trailing tail of unread members is
/// skipped via the enclosing byte count. This is the adaptive replacement for
/// fixed-offset parsing: layout comes from the file's `TStreamerInfo`, not pins.
pub(super) fn walk_members(
    r: &mut RBuffer<'_>,
    reg: &StreamerRegistry,
    elements: &[StreamerElement],
    out: &mut Members,
    on_object: &mut dyn FnMut(&str, &mut RBuffer<'_>) -> Result<()>,
    stop_after: &str,
) -> Result<()> {
    walk_members_at(r, reg, elements, out, on_object, stop_after, 0)
}

/// [`walk_members`] at base-class nesting `depth`, which is limited by
/// [`MAX_STREAMER_DEPTH`](oxiroot_io_core::raw::MAX_STREAMER_DEPTH): the file's own
/// streamer info drives the recursion, and a class listing itself as its base
/// would otherwise recurse until the stack overflows.
fn walk_members_at(
    r: &mut RBuffer<'_>,
    reg: &StreamerRegistry,
    elements: &[StreamerElement],
    out: &mut Members,
    on_object: &mut dyn FnMut(&str, &mut RBuffer<'_>) -> Result<()>,
    stop_after: &str,
    depth: usize,
) -> Result<()> {
    if depth > oxiroot_io_core::raw::MAX_STREAMER_DEPTH {
        return Err(Error::Format(format!(
            "base classes nested deeper than {} levels",
            oxiroot_io_core::raw::MAX_STREAMER_DEPTH
        )));
    }
    for el in elements {
        let t = el.el_type;
        if el.kind() == ElementKind::Base {
            read_base(r, reg, &el.name, out, on_object, stop_after, depth)?;
        } else if t == K_TSTRING {
            out.insert(el.name.clone(), MemberVal::Str(r.string()?));
        } else if (K_OBJECT..=71).contains(&t)
            || (K_STL_FIRST..=K_STL_STRING).contains(&t)
            || t == K_STL
            || t == K_STREAM_LOOP
        {
            // Object / object-pointer / STL / streamer member: the caller reads
            // the ones it needs (e.g. fBranches/fLeaves) and skips the rest.
            on_object(&el.name, r)?;
        } else if (K_OFFSET_P..K_OBJECT).contains(&t) {
            // kOffsetP + basic: a `T* //[fCount]` variable-length array.
            let (width, is_float) =
                basic_kind(t - K_OFFSET_P).ok_or_else(|| unsupported_element(el))?;
            let count = el
                .count_name
                .as_deref()
                .map_or(0, |c| usize::saturating_from(member_int(out, c)));
            r.u8()?; // is-array marker
            let mut vals = Vec::with_capacity(count.min(r.remaining()));
            for _ in 0..count {
                vals.push(read_basic(r, width, is_float)?.int());
            }
            out.insert(el.name.clone(), MemberVal::IntArray(vals));
        } else if (K_OFFSET_L..K_OFFSET_P).contains(&t) {
            // kOffsetL + basic: a fixed `T[fArrayLength]` member (read, not kept).
            let (width, is_float) =
                basic_kind(t - K_OFFSET_L).ok_or_else(|| unsupported_element(el))?;
            for _ in 0..el.array_length.max(0) {
                read_basic(r, width, is_float)?;
            }
        } else {
            let (width, is_float) = basic_kind(t).ok_or_else(|| unsupported_element(el))?;
            out.insert(el.name.clone(), read_basic(r, width, is_float)?);
        }
        if el.name == stop_after {
            break;
        }
    }
    Ok(())
}

/// Read a base-class slot named `class`. `TObject`/`Named` are read with their
/// dedicated readers (the latter captures `fName`/`fTitle`); any other base is
/// walked through its own streamer info when present, else skipped via its
/// version byte count.
fn read_base(
    r: &mut RBuffer<'_>,
    reg: &StreamerRegistry,
    class: &str,
    out: &mut Members,
    on_object: &mut dyn FnMut(&str, &mut RBuffer<'_>) -> Result<()>,
    stop_after: &str,
    depth: usize,
) -> Result<()> {
    match class {
        "TObject" => {
            read_object_base(r)?;
        }
        "TNamed" => {
            let named = read_named(r)?;
            out.insert("fName".to_string(), MemberVal::Str(named.name));
            out.insert("fTitle".to_string(), MemberVal::Str(named.title));
        }
        _ => {
            // Read the version first: the base is walked with the description
            // of its own class version.
            let vh = r.read_version()?;
            match reg.get_at(class, i32::from(vh.version)) {
                Some(info) => {
                    walk_members_at(
                        r,
                        reg,
                        &info.elements,
                        out,
                        on_object,
                        stop_after,
                        depth + 1,
                    )?;
                    if let Some(end) = vh.end {
                        r.seek(end)?;
                    }
                }
                None => {
                    let end = vh.end.ok_or_else(|| {
                        Error::Unsupported(format!(
                            "cannot skip a {class} that carries no byte count"
                        ))
                    })?;
                    r.seek(end)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MemberVal, Members, member_int, walk_members};
    use oxiroot_io_core::RBuffer;
    use oxiroot_io_core::Result;
    use oxiroot_io_core::StreamerRegistry;
    use oxiroot_io_core::raw::StreamerElement;

    fn elem(class: &str, name: &str, el_type: i32, count: Option<&str>) -> StreamerElement {
        let mut e = StreamerElement::default();
        e.element_class = class.to_string();
        e.name = name.to_string();
        e.el_type = el_type;
        e.count_name = count.map(str::to_string);
        e
    }

    /// The walker reads members by the streamer element list, so it adapts to a
    /// member order this reader was never compiled against, picks up an extra
    /// member ROOT might add in a future version, and sizes a `//[fCount]` array
    /// from the named counter — none of which a fixed-offset reader could do.
    #[test]
    fn walker_reads_by_element_list_not_fixed_offsets() {
        // Layout: a:int, b:Long64, c:int (a hypothetical *new* member), n:counter,
        // arr:Long64*[n]. A pinned reader keyed to "a,b" would misread c and arr.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&7i32.to_be_bytes()); // a
        bytes.extend_from_slice(&100i64.to_be_bytes()); // b
        bytes.extend_from_slice(&42i32.to_be_bytes()); // c (the evolved member)
        bytes.extend_from_slice(&2i32.to_be_bytes()); // n = 2
        bytes.push(1); // is-array marker
        bytes.extend_from_slice(&555i64.to_be_bytes()); // arr[0]
        bytes.extend_from_slice(&666i64.to_be_bytes()); // arr[1]

        let elements = vec![
            elem("TStreamerBasicType", "a", 3, None),            // kInt
            elem("TStreamerBasicType", "b", 16, None),           // kLong64
            elem("TStreamerBasicType", "c", 3, None),            // kInt
            elem("TStreamerBasicType", "n", 6, None),            // kCounter
            elem("TStreamerBasicPointer", "arr", 56, Some("n")), // kOffsetP + kLong64
        ];

        let reg = StreamerRegistry::default();
        let mut out = Members::new();
        let mut r = RBuffer::new(&bytes);
        let mut on_object = |_: &str, _: &mut RBuffer<'_>| -> Result<()> { Ok(()) };
        walk_members(&mut r, &reg, &elements, &mut out, &mut on_object, "").unwrap();

        assert_eq!(member_int(&out, "a"), 7);
        assert_eq!(member_int(&out, "b"), 100);
        assert_eq!(member_int(&out, "c"), 42); // read purely by its element name
        assert_eq!(member_int(&out, "n"), 2);
        match out.get("arr") {
            Some(MemberVal::IntArray(v)) => assert_eq!(v, &[555, 666]),
            other => panic!("arr not a counted array: {:?}", other.map(|_| ())),
        }
        assert_eq!(r.remaining(), 0, "the whole record was consumed");
    }

    /// `stop_after` ends the walk early (the caller then seeks past the rest via
    /// the object byte count), the way the tree reader stops after `fBranches`.
    #[test]
    fn walker_stops_after_named_member() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1i32.to_be_bytes()); // a
        bytes.extend_from_slice(&2i32.to_be_bytes()); // b
        bytes.extend_from_slice(&3i32.to_be_bytes()); // c (must remain unread)
        let elements = vec![
            elem("TStreamerBasicType", "a", 3, None),
            elem("TStreamerBasicType", "b", 3, None),
            elem("TStreamerBasicType", "c", 3, None),
        ];
        let reg = StreamerRegistry::default();
        let mut out = Members::new();
        let mut r = RBuffer::new(&bytes);
        let mut on_object = |_: &str, _: &mut RBuffer<'_>| -> Result<()> { Ok(()) };
        walk_members(&mut r, &reg, &elements, &mut out, &mut on_object, "b").unwrap();
        assert_eq!(member_int(&out, "a"), 1);
        assert_eq!(member_int(&out, "b"), 2);
        assert!(!out.contains_key("c"), "stopped before reading c");
        assert_eq!(
            r.remaining(),
            4,
            "c's 4 bytes are left for the caller to skip"
        );
    }

    /// A class that lists itself as its own base must end in an error after
    /// `MAX_STREAMER_DEPTH` levels, not recurse until the stack overflows.
    #[test]
    fn a_self_referential_base_class_is_an_error() {
        use oxiroot_io_core::raw::StreamerInfo;
        let mut looped = StreamerInfo::default();
        looped.class_name = "Loop".into();
        looped.class_version = 1;
        looped.elements = vec![elem("TStreamerBase", "Loop", 0, None)];
        let reg = StreamerRegistry::from_infos(vec![looped.clone()]);
        // Each level reads a version header: byte count (with the
        // kByteCountMask bit) then version 1. Far more levels than allowed.
        let mut bytes = Vec::new();
        for _ in 0..100_000 {
            bytes.extend_from_slice(&(0x4000_0000u32 | 6).to_be_bytes());
            bytes.extend_from_slice(&1u16.to_be_bytes());
        }
        let mut r = RBuffer::new(&bytes);
        let mut out = Members::new();
        let mut on_object = |_: &str, _: &mut RBuffer<'_>| Ok(());
        let err =
            walk_members(&mut r, &reg, &looped.elements, &mut out, &mut on_object, "").unwrap_err();
        assert!(err.to_string().contains("nested deeper"), "{err}");
    }
}

#[cfg(test)]
mod required_tests {
    use super::{MemberVal, Members, required_int};

    /// A member every object of a class has is an error when missing, or of
    /// another kind, never a silent zero.
    #[test]
    fn a_required_member_is_not_defaulted() {
        let mut m = Members::new();
        m.insert("fEntries".to_string(), MemberVal::Int(12));
        m.insert("fName".to_string(), MemberVal::Str("t".to_string()));
        assert_eq!(required_int(&m, "fEntries", "TTree").unwrap(), 12);
        let err = required_int(&m, "fNope", "TTree").unwrap_err();
        assert!(err.to_string().contains("no integer member fNope"), "{err}");
        assert!(required_int(&m, "fName", "TTree").is_err());
    }
}
