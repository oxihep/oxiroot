//! How a [`Branch`] lays out on disk: its leaf type, entry count, count
//! branch and the bytes of its baskets.

use super::NESTED_NOT_WRITABLE;
use super::branch::{Branch, BranchKind, vec_row_lengths};
use crate::value::BranchValues;
use oxiroot_io_core::cast::SaturatingFrom;

/// Whether a branch is a scalar, a fixed-size array, a variable-length (jagged)
/// array, a `std::vector<T>` (`TBranchElement`), or a string.
pub(super) enum Kind {
    Scalar,
    FixedArray(usize),
    Jagged,
    StlVector,
    Str,
}

/// The on-disk description of one leaf type.
pub(super) struct LeafInfo {
    /// `TLeafI`/`TLeafD`/`TLeafC`/… class name.
    pub(super) class: &'static str,
    /// Leaflist type code (`I`/`D`/`C`/…) used in the branch title.
    pub(super) code: char,
    /// Element byte width (the data stride; 1 for a `TLeafC` char).
    pub(super) size: i32,
    /// `fLenType` (the element width for numerics, 0 for `TLeafC`).
    pub(super) len_type: i32,
    /// `fIsUnsigned`.
    pub(super) unsigned: bool,
}

impl Branch {
    /// Number of entries (rows for arrays/strings).
    pub(super) fn n_entries(&self) -> u32 {
        if let Some(spec) = self.split() {
            return u32::saturating_from(
                spec.members
                    .first()
                    .map_or(0, |m| vec_row_lengths(&m.values).len()),
            );
        }
        u32::saturating_from(self.values.len())
    }

    /// The element leaf type (the inner type for arrays). A `std::vector<T>`
    /// branch keeps the element width/sign but its leaf class is `TLeafElement`.
    pub(super) fn leaf(&self) -> LeafInfo {
        use BranchValues::*;
        let (mut class, code, size, unsigned) = leaf_of(&self.values);
        let len_type = if matches!(self.values, Str(_)) || self.stl_vector() {
            0
        } else {
            size
        };
        if self.stl_vector() {
            class = "TLeafElement";
        }
        LeafInfo {
            class,
            code,
            size,
            len_type,
            unsigned,
        }
    }
}

/// The `TLeaf` class, type code, element width and signedness of `values`'
/// element type (a nested collection's innermost type).
fn leaf_of(values: &BranchValues) -> (&'static str, char, i32, bool) {
    use BranchValues::*;
    match values {
        Bool(_) | VecBool(_) => ("TLeafO", 'O', 1, false),
        I8(_) | VecI8(_) => ("TLeafB", 'B', 1, false),
        U8(_) | VecU8(_) => ("TLeafB", 'b', 1, true),
        I16(_) | VecI16(_) => ("TLeafS", 'S', 2, false),
        U16(_) | VecU16(_) => ("TLeafS", 's', 2, true),
        I32(_) | VecI32(_) => ("TLeafI", 'I', 4, false),
        U32(_) | VecU32(_) => ("TLeafI", 'i', 4, true),
        I64(_) | VecI64(_) => ("TLeafL", 'L', 8, false),
        U64(_) | VecU64(_) => ("TLeafL", 'l', 8, true),
        F32(_) | VecF32(_) => ("TLeafF", 'F', 4, false),
        F64(_) | VecF64(_) => ("TLeafD", 'D', 8, false),
        Str(_) | VecStr(_) => ("TLeafC", 'C', 1, false),
        Nested { items, .. } => leaf_of(items),
    }
}

impl Branch {
    /// The largest value of an integer scalar branch — the `fMaximum` of a count
    /// leaf, which ROOT uses to size the read buffer of the array it counts —
    /// or, for a string branch, the longest length + 1. 0 for anything else.
    pub(super) fn leaf_max(&self) -> i64 {
        use BranchValues::*;
        match &self.values {
            Bool(v) => i64::from(v.iter().any(|&b| b)),
            I8(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            U8(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            I16(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            U16(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            I32(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            U32(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
            I64(v) => v.iter().copied().max().unwrap_or(0),
            U64(v) => v.iter().copied().max().unwrap_or(0).cast_signed(),
            // A TLeafC's fMaximum is the longest string length + 1 (the buffer
            // size); ROOT uses it to size fValue and reallocates if it is 0.
            Str(v) => {
                v.iter()
                    .map(|s| i64::saturating_from(s.len()))
                    .max()
                    .unwrap_or(0)
                    + 1
            }
            _ => 0,
        }
    }

    /// The `TLeafC` `fLen` (longest string length + 1) for a string branch.
    pub(super) fn str_len(&self) -> i32 {
        match &self.values {
            BranchValues::Str(v) => {
                i32::saturating_from(v.iter().map(String::len).max().unwrap_or(0)) + 1
            }
            _ => 1,
        }
    }

    /// Elements per entry: `N` for a fixed array (from row 0), else 1 (scalar,
    /// string, and the jagged leaf — whose per-entry length is dynamic).
    pub(super) fn flen(&self) -> i32 {
        use BranchValues::*;
        if self.jagged() || self.stl_vector() {
            return 1;
        }
        if let BranchKind::Array(n) = self.kind {
            return i32::saturating_from(n);
        }
        let n = match &self.values {
            VecBool(r) => r.first().map_or(0, Vec::len),
            VecI8(r) => r.first().map_or(0, Vec::len),
            VecU8(r) => r.first().map_or(0, Vec::len),
            VecI16(r) => r.first().map_or(0, Vec::len),
            VecU16(r) => r.first().map_or(0, Vec::len),
            VecI32(r) => r.first().map_or(0, Vec::len),
            VecU32(r) => r.first().map_or(0, Vec::len),
            VecI64(r) => r.first().map_or(0, Vec::len),
            VecU64(r) => r.first().map_or(0, Vec::len),
            VecF32(r) => r.first().map_or(0, Vec::len),
            VecF64(r) => r.first().map_or(0, Vec::len),
            _ => 1,
        };
        i32::saturating_from(n)
    }

    /// Whether this is an array branch whose rows differ in length (not yet
    /// writable — variable-length numeric arrays need a separate count branch).
    pub(super) fn is_jagged(&self) -> bool {
        use BranchValues::*;
        // A fixed array's rows must all be its width; others, as long as the
        // first.
        let width = match self.kind {
            BranchKind::Array(n) => Some(n),
            _ => None,
        };
        macro_rules! jag {
            ($r:expr) => {{
                let n = width.unwrap_or_else(|| $r.first().map_or(0, Vec::len));
                $r.iter().any(|x| x.len() != n)
            }};
        }
        match &self.values {
            VecBool(r) => jag!(r),
            VecI8(r) => jag!(r),
            VecU8(r) => jag!(r),
            VecI16(r) => jag!(r),
            VecU16(r) => jag!(r),
            VecI32(r) => jag!(r),
            VecU32(r) => jag!(r),
            VecI64(r) => jag!(r),
            VecU64(r) => jag!(r),
            VecF32(r) => jag!(r),
            VecF64(r) => jag!(r),
            _ => false,
        }
    }

    pub(super) fn kind(&self) -> Kind {
        use BranchValues::*;
        if self.stl_vector() {
            return Kind::StlVector;
        }
        if self.jagged() {
            return Kind::Jagged;
        }
        match &self.values {
            Str(_) => Kind::Str,
            VecBool(_) | VecI8(_) | VecU8(_) | VecI16(_) | VecU16(_) | VecI32(_) | VecU32(_)
            | VecI64(_) | VecU64(_) | VecF32(_) | VecF64(_) => {
                Kind::FixedArray(usize::saturating_from(self.flen()))
            }
            _ => Kind::Scalar,
        }
    }

    /// The name of the auto-generated count branch for a jagged branch (`y` →
    /// `ny`), matching uproot's convention.
    pub(super) fn count_name(&self) -> String {
        format!("n{}", self.name)
    }

    /// Per-row element counts (for a jagged branch's count branch); empty for
    /// non-array branches.
    fn row_lengths(&self) -> Vec<i32> {
        vec_row_lengths(&self.values)
    }

    /// The paired count branch (`n<name>`, a scalar `i32` of row lengths) for a
    /// jagged branch; `None` for any other branch.
    pub(super) fn count_branch(&self) -> Option<Self> {
        self.jagged().then(|| Self {
            name: self.count_name(),
            values: BranchValues::I32(self.row_lengths()),
            kind: BranchKind::Count,
        })
    }

    /// The `std::vector<T>` class name and `fCheckSum` ROOT writes for the
    /// element type (used when this is a `TBranchElement`). Checksums are the
    /// fixed values ROOT computes for each `vector<T>` specialization.
    pub(super) fn stl_info(&self) -> (&'static str, u32) {
        use BranchValues::*;
        match &self.values {
            VecI8(_) => ("vector<char>", 2107423027),
            VecU8(_) => ("vector<unsigned char>", 3193843768),
            VecI16(_) => ("vector<short>", 2609783071),
            VecU16(_) => ("vector<unsigned short>", 2240785856),
            VecI32(_) => ("vector<int>", 1796663354),
            VecU32(_) => ("vector<unsigned int>", 2269658365),
            VecI64(_) => ("vector<Long64_t>", 1788137638),
            VecU64(_) => ("vector<ULong64_t>", 3999597035),
            VecF32(_) => ("vector<float>", 1727547419),
            VecF64(_) => ("vector<double>", 3894200540),
            VecBool(_) => ("vector<bool>", 3155025381),
            VecStr(_) => ("vector<string>", 2038122176),
            Nested { items, .. } => match **items {
                VecBool(_) => ("vector<vector<bool> >", 723642738),
                VecI8(_) => ("vector<vector<char> >", 723640794),
                VecU8(_) => ("vector<vector<unsigned char> >", 1634559415),
                VecI16(_) => ("vector<vector<short> >", 2170962476),
                VecU16(_) => ("vector<vector<unsigned short> >", 608751043),
                VecI32(_) => ("vector<vector<int> >", 302296425),
                VecU32(_) => ("vector<vector<unsigned int> >", 4220325614),
                VecI64(_) => ("vector<vector<Long64_t> >", 2589173317),
                VecU64(_) => ("vector<vector<ULong64_t> >", 4042251808),
                VecF32(_) => ("vector<vector<float> >", 2219069760),
                VecF64(_) => ("vector<vector<double> >", 3084746367),
                // `Branch`'s vector constructors build the variants above and
                // no other. A made-up class name and checksum would write a
                // file ROOT misreads.
                ref other => unreachable!("a nested vector branch of {other:?}"),
            },
            other => unreachable!("a std::vector branch holding {other:?}"),
        }
    }

    /// Entry data + `fEntryOffset` for a `std::vector<T>` branch: each row is a
    /// streamed collection — `[byte count | mask](4) [0x000a](2) [size n](4)`
    /// then `n` big-endian elements.
    fn stl_basket_content(&self) -> (Vec<u8>, Vec<u32>) {
        use BranchValues::*;
        let mut data = Vec::new();
        let mut offsets = vec![0u32];
        macro_rules! emit {
            ($rows:expr, $w:expr, $conv:expr) => {{
                for row in $rows {
                    let n = u32::saturating_from(row.len());
                    let bc = (6 + n * $w) | 0x4000_0000;
                    data.extend_from_slice(&bc.to_be_bytes());
                    data.extend_from_slice(&0x000a_u16.to_be_bytes());
                    data.extend_from_slice(&n.to_be_bytes());
                    for x in row {
                        data.extend_from_slice(&$conv(x));
                    }
                    offsets.push(u32::saturating_from(data.len()));
                }
            }};
        }
        match &self.values {
            VecBool(r) => emit!(r, 1, |x: &bool| [u8::from(*x)]),
            VecStr(rows) => {
                for row in rows {
                    let start = data.len();
                    data.extend_from_slice(&[0; 4]); // byte count, patched below
                    data.extend_from_slice(&0x000a_u16.to_be_bytes());
                    data.extend_from_slice(&u32::saturating_from(row.len()).to_be_bytes());
                    for s in row {
                        put_string(&mut data, s);
                    }
                    let bc = u32::saturating_from(data.len() - start - 4) | 0x4000_0000;
                    data[start..start + 4].copy_from_slice(&bc.to_be_bytes());
                    offsets.push(u32::saturating_from(data.len()));
                }
            }
            Nested {
                offsets: outer,
                items,
            } => nested_content(outer, items, &mut data, &mut offsets),
            VecI8(r) => emit!(r, 1, |x: &i8| [(*x).cast_unsigned()]),
            VecU8(r) => emit!(r, 1, |x: &u8| [*x]),
            VecI16(r) => emit!(r, 2, |x: &i16| x.to_be_bytes()),
            VecU16(r) => emit!(r, 2, |x: &u16| x.to_be_bytes()),
            VecI32(r) => emit!(r, 4, |x: &i32| x.to_be_bytes()),
            VecU32(r) => emit!(r, 4, |x: &u32| x.to_be_bytes()),
            VecI64(r) => emit!(r, 8, |x: &i64| x.to_be_bytes()),
            VecU64(r) => emit!(r, 8, |x: &u64| x.to_be_bytes()),
            VecF32(r) => emit!(r, 4, |x: &f32| x.to_be_bytes()),
            VecF64(r) => emit!(r, 8, |x: &f64| x.to_be_bytes()),
            _ => {}
        }
        (data, offsets)
    }

    /// The basket's uncompressed entry data, plus (for variable branches) the
    /// data-relative `fEntryOffset` array (`n_entries + 1` offsets).
    pub(super) fn basket_content(&self) -> (Vec<u8>, Option<Vec<u32>>) {
        use BranchValues::*;
        if self.stl_vector() {
            let (data, offsets) = self.stl_basket_content();
            return (data, Some(offsets));
        }
        macro_rules! be {
            ($v:expr, $w:expr) => {{
                let mut out = Vec::with_capacity($v.len() * $w);
                for x in $v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
                out
            }};
        }
        macro_rules! be_rows {
            ($r:expr, $w:expr) => {{
                let mut out = Vec::new();
                for row in $r {
                    for x in row {
                        out.extend_from_slice(&x.to_be_bytes());
                    }
                }
                out
            }};
        }
        let data = match &self.values {
            Bool(v) => v.iter().map(|&b| u8::from(b)).collect(),
            I8(v) => v.iter().map(|&x| x.cast_unsigned()).collect(),
            U8(v) => v.clone(),
            I16(v) => be!(v, 2),
            U16(v) => be!(v, 2),
            I32(v) => be!(v, 4),
            U32(v) => be!(v, 4),
            I64(v) => be!(v, 8),
            U64(v) => be!(v, 8),
            F32(v) => be!(v, 4),
            F64(v) => be!(v, 8),
            VecBool(r) => r.iter().flatten().map(|&b| u8::from(b)).collect(),
            VecI8(r) => r.iter().flatten().map(|&x| x.cast_unsigned()).collect(),
            VecU8(r) => r.concat(),
            VecI16(r) => be_rows!(r, 2),
            VecU16(r) => be_rows!(r, 2),
            VecI32(r) => be_rows!(r, 4),
            VecU32(r) => be_rows!(r, 4),
            VecI64(r) => be_rows!(r, 8),
            VecU64(r) => be_rows!(r, 8),
            VecF32(r) => be_rows!(r, 4),
            VecF64(r) => be_rows!(r, 8),
            Str(strings) => {
                let mut data = Vec::new();
                let mut offsets = vec![0u32];
                for s in strings {
                    put_string(&mut data, s);
                    offsets.push(u32::saturating_from(data.len()));
                }
                return (data, Some(offsets));
            }
            // A vector<string> or vector<vector<T>> branch is always a
            // TBranchElement, handled above.
            VecStr(_) | Nested { .. } => unreachable!("{NESTED_NOT_WRITABLE}"),
        };
        // A jagged numeric branch is variable-length too: emit the byte offset
        // after each row (element count × element width).
        if self.jagged() {
            let elem = self.leaf().size.cast_unsigned();
            let mut offsets = Vec::with_capacity(self.n_entries() as usize + 1);
            let mut acc = 0u32;
            offsets.push(0);
            for len in self.row_lengths() {
                // Saturating: a basket this large is refused when it is prepared.
                acc = acc.saturating_add(len.cast_unsigned().saturating_mul(elem));
                offsets.push(acc);
            }
            return (data, Some(offsets));
        }
        (data, None)
    }
}

/// Append a ROOT string: a length byte (or `255` then a 4-byte length) and the
/// bytes.
fn put_string(data: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    if b.len() < 255 {
        data.push(u8::saturating_from(b.len()));
    } else {
        data.push(255);
        data.extend_from_slice(&u32::saturating_from(b.len()).to_be_bytes());
    }
    data.extend_from_slice(b);
}

/// Entry data for a `std::vector<std::vector<T>>` branch: per entry the
/// collection header with the outer count, then per inner vector its count
/// and big-endian elements.
fn nested_content(outer: &[u64], items: &BranchValues, data: &mut Vec<u8>, offsets: &mut Vec<u32>) {
    use BranchValues::*;
    macro_rules! rows {
        ($r:expr, $conv:expr) => {{
            for w in outer.windows(2) {
                let start = data.len();
                data.extend_from_slice(&[0; 4]); // byte count, patched below
                data.extend_from_slice(&0x000a_u16.to_be_bytes());
                let rows = &$r[usize::saturating_from(w[0])..usize::saturating_from(w[1])];
                data.extend_from_slice(&u32::saturating_from(rows.len()).to_be_bytes());
                for row in rows {
                    data.extend_from_slice(&u32::saturating_from(row.len()).to_be_bytes());
                    for x in row {
                        data.extend_from_slice(&$conv(x));
                    }
                }
                let bc = u32::saturating_from(data.len() - start - 4) | 0x4000_0000;
                data[start..start + 4].copy_from_slice(&bc.to_be_bytes());
                offsets.push(u32::saturating_from(data.len()));
            }
        }};
    }
    match items {
        VecBool(r) => rows!(r, |x: &bool| [u8::from(*x)]),
        VecI8(r) => rows!(r, |x: &i8| [(*x).cast_unsigned()]),
        VecU8(r) => rows!(r, |x: &u8| [*x]),
        VecI16(r) => rows!(r, |x: &i16| x.to_be_bytes()),
        VecU16(r) => rows!(r, |x: &u16| x.to_be_bytes()),
        VecI32(r) => rows!(r, |x: &i32| x.to_be_bytes()),
        VecU32(r) => rows!(r, |x: &u32| x.to_be_bytes()),
        VecI64(r) => rows!(r, |x: &i64| x.to_be_bytes()),
        VecU64(r) => rows!(r, |x: &u64| x.to_be_bytes()),
        VecF32(r) => rows!(r, |x: &f32| x.to_be_bytes()),
        VecF64(r) => rows!(r, |x: &f64| x.to_be_bytes()),
        _ => {}
    }
}
