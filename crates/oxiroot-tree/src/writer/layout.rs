//! How a [`Branch`] lays out on disk: its leaf type, entry count, count
//! branch and the bytes of its baskets.

use std::ops::Range;

use super::branch::{Branch, BranchKind, vec_row_lengths};
use oxiroot_io_core::Column;
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

/// The `TLeaf` class, type code, element width and signedness of the values
/// `column` holds, whatever lists or arrays they are in. Everything about a
/// value type the writer needs is read from this one table.
fn leaf_of(column: &Column) -> (&'static str, char, i32, bool) {
    match column {
        Column::Bool(_) => ("TLeafO", 'O', 1, false),
        Column::I8(_) => ("TLeafB", 'B', 1, false),
        Column::U8(_) => ("TLeafB", 'b', 1, true),
        Column::I16(_) => ("TLeafS", 'S', 2, false),
        Column::U16(_) => ("TLeafS", 's', 2, true),
        Column::I32(_) => ("TLeafI", 'I', 4, false),
        Column::U32(_) => ("TLeafI", 'i', 4, true),
        Column::I64(_) => ("TLeafL", 'L', 8, false),
        Column::U64(_) => ("TLeafL", 'l', 8, true),
        Column::F32(_) => ("TLeafF", 'F', 4, false),
        Column::F64(_) => ("TLeafD", 'D', 8, false),
        Column::String { .. } => ("TLeafC", 'C', 1, false),
        Column::List { values, .. } | Column::Array { values, .. } => leaf_of(values),
        // `Branch::check` refuses every other column before anything is laid
        // out. A made-up leaf would write a file ROOT misreads.
        other => unreachable!("a branch holding a {}", other.type_name()),
    }
}

/// Append values `range` of a scalar column, big-endian. The one place a
/// value becomes bytes.
fn put_be(column: &Column, range: Range<usize>, out: &mut Vec<u8>) {
    macro_rules! be {
        ($v:expr) => {
            for x in $v.get(range).unwrap_or(&[]) {
                out.extend_from_slice(&x.to_be_bytes());
            }
        };
    }
    match column {
        Column::Bool(v) => out.extend(v.get(range).unwrap_or(&[]).iter().map(|&b| u8::from(b))),
        Column::I8(v) => be!(v),
        Column::U8(v) => out.extend_from_slice(v.get(range).unwrap_or(&[])),
        Column::I16(v) => be!(v),
        Column::U16(v) => be!(v),
        Column::I32(v) => be!(v),
        Column::U32(v) => be!(v),
        Column::I64(v) => be!(v),
        Column::U64(v) => be!(v),
        Column::F32(v) => be!(v),
        Column::F64(v) => be!(v),
        other => unreachable!("values of a {} written as numbers", other.type_name()),
    }
}

/// The largest value of a scalar integer column, 0 for an empty one; `None`
/// for a column that is not one.
fn integer_max(column: &Column) -> Option<i64> {
    Some(match column {
        Column::Bool(v) => i64::from(v.iter().any(|&b| b)),
        Column::I8(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::U8(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::I16(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::U16(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::I32(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::U32(v) => i64::from(v.iter().copied().max().unwrap_or(0)),
        Column::I64(v) => v.iter().copied().max().unwrap_or(0),
        Column::U64(v) => v.iter().copied().max().unwrap_or(0).cast_signed(),
        _ => return None,
    })
}

/// The `[start, end)` each entry of `offsets` covers in its child.
fn entries(offsets: &[u64]) -> impl Iterator<Item = Range<usize>> + '_ {
    offsets
        .windows(2)
        .map(|w| usize::saturating_from(w[0])..usize::saturating_from(w[1]))
}

/// The longest string of a string column, in bytes.
fn longest(offsets: &[u64]) -> usize {
    entries(offsets).map(|r| r.len()).max().unwrap_or(0)
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

/// One streamed `std::vector`: the byte count, the version, the element
/// count, then what `elements` writes.
fn put_vector(data: &mut Vec<u8>, count: usize, elements: impl FnOnce(&mut Vec<u8>)) {
    let start = data.len();
    data.extend_from_slice(&[0; 4]); // byte count, filled in below
    data.extend_from_slice(&0x000a_u16.to_be_bytes());
    data.extend_from_slice(&u32::saturating_from(count).to_be_bytes());
    elements(data);
    let bc = u32::saturating_from(data.len() - start - 4) | 0x4000_0000;
    data[start..start + 4].copy_from_slice(&bc.to_be_bytes());
}

impl Branch {
    /// Number of entries.
    pub(super) fn n_entries(&self) -> u32 {
        if let Some(spec) = self.split() {
            return u32::saturating_from(spec.members.first().map_or(0, |m| m.values.len()));
        }
        u32::saturating_from(self.values.len())
    }

    /// The element leaf type (the inner type for arrays). A `std::vector<T>`
    /// branch keeps the element width/sign but its leaf class is `TLeafElement`.
    pub(super) fn leaf(&self) -> LeafInfo {
        let (mut class, code, size, unsigned) = leaf_of(&self.values);
        let len_type = if matches!(self.values, Column::String { .. }) || self.stl_vector() {
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

    /// The largest value of an integer scalar branch — the `fMaximum` of a count
    /// leaf, which ROOT uses to size the read buffer of the array it counts —
    /// or, for a string branch, the longest length + 1. 0 for anything else.
    pub(super) fn leaf_max(&self) -> i64 {
        if let Some(meta) = self.meta {
            return meta.max;
        }
        match &self.values {
            // A TLeafC's fMaximum is the longest string length + 1 (the buffer
            // size); ROOT uses it to size fValue and reallocates if it is 0.
            Column::String { offsets, .. } => i64::saturating_from(longest(offsets)) + 1,
            other => integer_max(other).unwrap_or(0),
        }
    }

    /// The largest element count of a split `std::vector<Struct>` branch: its
    /// parent's `fMaximum`.
    pub(super) fn split_max_count(&self) -> i32 {
        if let Some(meta) = self.meta {
            return meta.split_max;
        }
        self.split()
            .and_then(|spec| spec.members.first())
            .and_then(|m| vec_row_lengths(&m.values).into_iter().max())
            .unwrap_or(0)
    }

    /// The `TLeafC` `fLen` (longest string length + 1) for a string branch.
    pub(super) fn str_len(&self) -> i32 {
        if let Some(meta) = self.meta {
            return meta.str_len;
        }
        match &self.values {
            Column::String { offsets, .. } => i32::saturating_from(longest(offsets)) + 1,
            _ => 1,
        }
    }

    /// Elements per entry: `N` for a fixed array, else 1 (scalar, string, and
    /// the jagged leaf — whose per-entry length is dynamic).
    pub(super) fn flen(&self) -> i32 {
        match self.kind {
            BranchKind::Array(n) => i32::saturating_from(n),
            _ => 1,
        }
    }

    pub(super) fn kind(&self) -> Kind {
        match (&self.kind, &self.values) {
            (BranchKind::StlVector, _) => Kind::StlVector,
            (BranchKind::Jagged, _) => Kind::Jagged,
            (BranchKind::Array(n), _) => Kind::FixedArray(*n),
            (_, Column::String { .. }) => Kind::Str,
            _ => Kind::Scalar,
        }
    }

    /// The name of the auto-generated count branch for a jagged branch (`y` →
    /// `ny`), matching uproot's convention.
    pub(super) fn count_name(&self) -> String {
        format!("n{}", self.name)
    }

    /// The paired count branch (`n<name>`, a scalar `i32` of row lengths) for a
    /// jagged branch; `None` for any other branch.
    pub(super) fn count_branch(&self) -> Option<Self> {
        self.jagged().then(|| {
            Self::raw(
                self.count_name(),
                Column::I32(vec_row_lengths(&self.values)),
                BranchKind::Count,
            )
        })
    }

    /// The `std::vector<T>` class name and `fCheckSum` ROOT writes for the
    /// element type (used when this is a `TBranchElement`). Checksums are the
    /// fixed values ROOT computes for each `vector<T>` specialization.
    pub(super) fn stl_info(&self) -> (&'static str, u32) {
        let Column::List { values, .. } = &self.values else {
            unreachable!("a std::vector branch holding a {}", self.values.type_name())
        };
        match &**values {
            Column::I8(_) => ("vector<char>", 2107423027),
            Column::U8(_) => ("vector<unsigned char>", 3193843768),
            Column::I16(_) => ("vector<short>", 2609783071),
            Column::U16(_) => ("vector<unsigned short>", 2240785856),
            Column::I32(_) => ("vector<int>", 1796663354),
            Column::U32(_) => ("vector<unsigned int>", 2269658365),
            Column::I64(_) => ("vector<Long64_t>", 1788137638),
            Column::U64(_) => ("vector<ULong64_t>", 3999597035),
            Column::F32(_) => ("vector<float>", 1727547419),
            Column::F64(_) => ("vector<double>", 3894200540),
            Column::Bool(_) => ("vector<bool>", 3155025381),
            Column::String { .. } => ("vector<string>", 2038122176),
            Column::List { values, .. } => match &**values {
                Column::Bool(_) => ("vector<vector<bool> >", 723642738),
                Column::I8(_) => ("vector<vector<char> >", 723640794),
                Column::U8(_) => ("vector<vector<unsigned char> >", 1634559415),
                Column::I16(_) => ("vector<vector<short> >", 2170962476),
                Column::U16(_) => ("vector<vector<unsigned short> >", 608751043),
                Column::I32(_) => ("vector<vector<int> >", 302296425),
                Column::U32(_) => ("vector<vector<unsigned int> >", 4220325614),
                Column::I64(_) => ("vector<vector<Long64_t> >", 2589173317),
                Column::U64(_) => ("vector<vector<ULong64_t> >", 4042251808),
                Column::F32(_) => ("vector<vector<float> >", 2219069760),
                Column::F64(_) => ("vector<vector<double> >", 3084746367),
                // `Branch::new` makes a std::vector branch of the shapes
                // above and no other. A made-up class name and checksum
                // would write a file ROOT misreads.
                other => unreachable!("a nested vector branch of {}", other.type_name()),
            },
            other => unreachable!("a std::vector branch of {}", other.type_name()),
        }
    }

    /// Entry data + `fEntryOffset` for a `std::vector<T>` branch: each row is a
    /// streamed collection — `[byte count | mask](4) [0x000a](2) [size n](4)`
    /// then its elements.
    fn stl_basket_content(&self) -> (Vec<u8>, Vec<u32>) {
        let mut data = Vec::new();
        let mut ends = vec![0u32];
        let Column::List { offsets, values } = &self.values else {
            return (data, ends);
        };
        for row in entries(offsets) {
            let count = row.len();
            match &**values {
                Column::String {
                    offsets: bounds,
                    text,
                } => put_vector(&mut data, count, |data| {
                    for s in entries(bounds.get(row.start..=row.end).unwrap_or(&[])) {
                        put_string(data, text.get(s).unwrap_or_default());
                    }
                }),
                // Each inner vector is its count, then its elements.
                Column::List {
                    offsets: bounds,
                    values: inner,
                } => put_vector(&mut data, count, |data| {
                    for inner_row in entries(bounds.get(row.start..=row.end).unwrap_or(&[])) {
                        data.extend_from_slice(
                            &u32::saturating_from(inner_row.len()).to_be_bytes(),
                        );
                        put_be(inner, inner_row, data);
                    }
                }),
                scalars => put_vector(&mut data, count, |data| put_be(scalars, row, data)),
            }
            ends.push(u32::saturating_from(data.len()));
        }
        (data, ends)
    }

    /// The basket's uncompressed entry data, plus (for variable branches) the
    /// data-relative `fEntryOffset` array (`n_entries + 1` offsets).
    pub(super) fn basket_content(&self) -> (Vec<u8>, Option<Vec<u32>>) {
        if self.stl_vector() {
            let (data, offsets) = self.stl_basket_content();
            return (data, Some(offsets));
        }
        let mut data = Vec::new();
        match &self.values {
            Column::String { offsets, text } => {
                let mut ends = vec![0u32];
                for s in entries(offsets) {
                    put_string(&mut data, text.get(s).unwrap_or_default());
                    ends.push(u32::saturating_from(data.len()));
                }
                (data, Some(ends))
            }
            Column::Array { values, .. } => {
                put_be(values, 0..values.len(), &mut data);
                (data, None)
            }
            // A jagged numeric branch is variable-length: the byte offset after
            // each row (element count × element width) goes with the data.
            Column::List { offsets, values } => {
                put_be(values, 0..values.len(), &mut data);
                let elem = u64::from(self.leaf().size.cast_unsigned());
                // Saturating: a basket this large is refused when it is prepared.
                let ends = offsets
                    .iter()
                    .map(|&o| u32::saturating_from(o.saturating_mul(elem)))
                    .collect();
                (data, Some(ends))
            }
            scalars => {
                put_be(scalars, 0..scalars.len(), &mut data);
                (data, None)
            }
        }
    }
}
