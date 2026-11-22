//! Decoding branch data: from basket payloads to [`Column`]s.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::K_BYTE_COUNT_MASK;
use oxiroot_io_core::{Column, Error, FileReader, RBuffer, Result, StreamerRegistry};

use super::members::{MemberVal, Members, walk_members};
use super::parse::skip_object;
use super::{Branch, ObjectMember};
use crate::basket::Basket;
use crate::value::LeafType;

/// One leaf of a leaflist branch: its bytes in each entry, which sit at a fixed
/// offset in a fixed stride. `None` for a branch that is not a leaflist leaf.
fn leaflist_regions<'a>(branch: &Branch, baskets: &'a [Basket]) -> Option<Vec<&'a [u8]>> {
    let (offset, stride) = branch.leaflist?;
    if stride == 0 {
        return Some(Vec::new());
    }
    let width = usize::saturating_from(branch.leaf_len.max(1)) * branch.leaf_type.size();
    let mut regions = Vec::new();
    for b in baskets {
        for chunk in b.entry_data().chunks_exact(stride) {
            let end = (offset + width).min(chunk.len());
            regions.push(chunk.get(offset..end).unwrap_or(&[]));
        }
    }
    Some(regions)
}

/// How [`read_baskets`] decompresses: in order on the calling thread, or, only
/// when a caller asks for it, across rayon's global pool. Without the `rayon`
/// feature there is only the serial way, whatever a caller asks.
#[derive(Clone, Copy)]
pub(super) enum Decode {
    Serial,
    #[cfg(feature = "rayon")]
    Parallel,
}

impl Decode {
    /// `Parallel` when asked for and the `rayon` feature provides a pool.
    pub(super) const fn from_parallel(parallel: bool) -> Self {
        #[cfg(feature = "rayon")]
        if parallel {
            return Self::Parallel;
        }
        let _ = parallel;
        Self::Serial
    }
}

/// Read the requested baskets of `branch` (by index) and decompress them,
/// returning them in index order either way.
pub(super) fn read_baskets(
    file: &FileReader,
    branch: &Branch,
    indices: impl Iterator<Item = usize>,
    decode: Decode,
) -> Result<Vec<Basket>> {
    let seek_of = |i: usize| -> Result<u64> {
        branch.basket_seek.get(i).copied().ok_or_else(|| {
            Error::Format(format!("branch {:?}: missing basket {i} seek", branch.name))
        })
    };
    // `fBasketBytes` gives the exact on-disk record size (one exact fetch, no
    // over-read); `None` when the file omits it (the reader probes the header).
    let bytes_of = |i: usize| -> Option<usize> {
        branch
            .basket_bytes
            .get(i)
            .and_then(|&b| (b > 0).then_some(usize::saturating_from(b)))
    };

    // Announce the baskets of known size, so a remote file fetches them in
    // batches instead of a round trip each. The ranges are exactly the ones
    // `Basket::read` asks for.
    let indices: Vec<usize> = indices.collect();
    let planned: Vec<(u64, usize)> = indices
        .iter()
        .filter_map(|&i| {
            let seek = seek_of(i).ok()?;
            let avail = file.size().saturating_sub(seek);
            bytes_of(i).map(|n| (seek, usize::saturating_from((n as u64).min(avail))))
        })
        .collect();
    file.plan_reads(&planned);
    let indices = indices.into_iter();

    match decode {
        // Stops at the first error.
        Decode::Serial => indices
            .map(|i| Basket::read(file, seek_of(i)?, bytes_of(i)))
            .collect(),
        #[cfg(feature = "rayon")]
        Decode::Parallel => {
            use rayon::prelude::*;
            let indices: Vec<usize> = indices.collect();
            // par_iter().collect() into a Result preserves order and short-circuits
            // on the first error; the file source and branch are read-only (Sync).
            indices
                .into_par_iter()
                .map(|i| Basket::read(file, seek_of(i)?, bytes_of(i)))
                .collect()
        }
    }
}

/// Decode the given (contiguous, in-order) baskets of `branch` into a
/// [`Column`] — the body of [`BranchRead::values`](super::BranchRead::values),
/// over every basket or a window of them. A scalar branch is a scalar
/// column, a fixed array an array, a counted array or `std::vector` a list.
pub(super) fn decode_baskets(branch: &Branch, baskets: &[Basket]) -> Result<Column> {
    let column = decode_entries(branch, baskets)?;
    // Baskets say how their entries are cut; with none to read, or none that
    // holds an entry, the branch's own metadata says it is a list.
    let empty_list = branch.variable
        && column.is_empty()
        && !matches!(column, Column::List { .. } | Column::String { .. });
    Ok(if empty_list {
        Column::List {
            offsets: vec![0],
            values: Box::new(column),
        }
    } else {
        column
    })
}

/// The entries of `baskets`, shaped as the baskets themselves say.
fn decode_entries(branch: &Branch, baskets: &[Basket]) -> Result<Column> {
    let width = usize::saturating_from(branch.leaf_len);
    // A synthesized `TBranchObject` member column: each entry is a whole object,
    // from which this member is extracted.
    if let Some(om) = &branch.object_member {
        return decode_object_members(branch.leaf_type, om, &entry_regions_variable(baskets)?);
    }

    // A leaflist leaf: take this leaf's bytes out of each entry's fixed stride
    // at its offset, then decode like a scalar / fixed array.
    if let Some(regions) = leaflist_regions(branch, baskets) {
        if branch.leaflist.is_some_and(|(_, stride)| stride == 0) {
            return decode_scalar(branch.leaf_type, &[]);
        }
        if branch.leaf_len > 1 {
            return decode_array(branch.leaf_type, width, &regions);
        }
        let bytes: Vec<u8> = regions.concat();
        return decode_scalar(branch.leaf_type, &bytes);
    }

    // A `std::vector<std::vector<T>>`: each entry's region is the outer vector's
    // 10-byte streamer header (whose last 4 bytes are the outer count) followed
    // by the inner vectors, each a `{count, elements}` block.
    if let Some(elem) = branch.nested_elem {
        return decode_nested_vec(&entry_regions_variable(baskets)?, elem);
    }

    let variable = baskets.iter().any(|b| b.entry_offsets.is_some());
    if branch.leaf_type == LeafType::Str {
        // A `std::vector<std::string>` branch carries a 10-byte streamer header
        // (with the element count); a plain `TLeafC` is one string per entry.
        if branch.elem_header > 0 {
            return decode_vec_strings(&entry_regions_variable(baskets)?);
        }
        return decode_strings(baskets);
    }
    if variable {
        let mut regions = entry_regions_variable(baskets)?;
        // A `std::vector` `TBranchElement` prefixes each entry with a streamer
        // header; strip it so only the element bytes remain.
        if branch.elem_header > 0 {
            for r in &mut regions {
                *r = &r[branch.elem_header.min(r.len())..];
            }
        }
        return decode_list(branch.leaf_type, &regions);
    }
    if branch.leaf_len > 1 {
        let stride = width * branch.leaf_type.size();
        return decode_array(branch.leaf_type, width, &chunk_regions(baskets, stride));
    }
    // Scalar: concatenate every basket's entry data, decode once.
    let mut bytes = Vec::with_capacity(baskets.iter().map(|b| b.entry_data().len()).sum());
    let mut total = 0u64;
    for b in baskets {
        bytes.extend_from_slice(b.entry_data());
        total += u64::from(b.n_entries);
    }
    if bytes.len() != usize::saturating_from(total) * branch.leaf_type.size() {
        return Err(Error::Format(format!(
            "branch {:?}: {} basket bytes for {total} {:?} entries",
            branch.name,
            bytes.len(),
            branch.leaf_type
        )));
    }
    decode_scalar(branch.leaf_type, &bytes)
}

/// Per-entry byte regions of a variable-length branch, from each basket's
/// `fEntryOffset` array.
fn entry_regions_variable(baskets: &[Basket]) -> Result<Vec<&[u8]>> {
    let mut regions = Vec::new();
    for b in baskets {
        let offs = b
            .entry_offsets
            .as_ref()
            .ok_or_else(|| Error::Format("variable branch basket missing fEntryOffset".into()))?;
        for i in 0..b.n_entries as usize {
            let (a, c) = (
                *offs.get(i).unwrap_or(&b.border),
                *offs.get(i + 1).unwrap_or(&b.border),
            );
            regions.push(b.data.get(a..c).unwrap_or(&[]));
        }
    }
    Ok(regions)
}

/// Decode a synthesized `TBranchObject` member column: each `region` is one
/// entry's whole object (`[className][version][members]`); walk the object class
/// and collect `om.member`, typed by `leaf_type`. Decoding is registry-free —
/// the object class's elements are carried on `om`, and base classes other than
/// `TObject`/`Named` degrade to a byte-count skip.
fn decode_object_members(
    leaf_type: LeafType,
    om: &ObjectMember,
    regions: &[&[u8]],
) -> Result<Column> {
    let reg = StreamerRegistry::default();
    let mut values: Vec<MemberVal> = Vec::with_capacity(regions.len());
    for region in regions {
        let mut r = RBuffer::new(region);
        locate_object(&mut r);
        r.read_version()?; // the object's own version header
        let mut members = Members::new();
        let mut on_object = |_: &str, rb: &mut RBuffer<'_>| -> Result<()> { skip_object(rb) };
        walk_members(
            &mut r,
            &reg,
            &om.class_elements,
            &mut members,
            &mut on_object,
            // The walk ends at the member this column holds: the ones after it
            // are not decoded for a value nobody reads.
            &om.member,
        )?;
        values.push(members.remove(&om.member).unwrap_or(MemberVal::Int(0)));
    }
    Ok(build_member_column(leaf_type, &values))
}

/// Skip an entry's leading class-name string (when present, for a "virtual"
/// `TLeafObject`) and any padding so the cursor sits at the object's version
/// header — recognised by the byte count's mask bit in its leading word.
fn locate_object(r: &mut RBuffer<'_>) {
    if r.remaining() < 4 {
        return;
    }
    let start = r.pos();
    let first = r.be_u32().unwrap_or(0);
    let _ = r.seek(start);
    // A small leading byte (a class-name length) rather than a masked byte count
    // means the class name is present; consume it.
    if first & K_BYTE_COUNT_MASK == 0 {
        let _ = r.string();
    }
    // Step over up to a few padding bytes to the object's byte-count word.
    for _ in 0..4 {
        if r.remaining() < 4 {
            return;
        }
        let p = r.pos();
        let word = r.be_u32().unwrap_or(0);
        let _ = r.seek(p);
        if word & K_BYTE_COUNT_MASK != 0 {
            return;
        }
        let _ = r.seek(p + 1);
    }
}

/// Build a column of `leaf_type` from per-entry member values
/// (integers, floats, or strings extracted from each entry's object).
#[allow(
    clippy::cast_possible_truncation,
    reason = "f32 leaves read as f32 and widened: narrowing back is exact"
)]
fn build_member_column(leaf_type: LeafType, vals: &[MemberVal]) -> Column {
    let floats = |v: &[MemberVal]| -> Vec<f64> {
        v.iter()
            .map(|m| match m {
                MemberVal::Float(f) => *f,
                other => other.int() as f64,
            })
            .collect()
    };
    let ints: Vec<i64> = vals.iter().map(MemberVal::int).collect();
    match leaf_type {
        LeafType::Str => vals
            .iter()
            .map(MemberVal::str)
            .collect::<Vec<&str>>()
            .into(),
        LeafType::Bool => Column::Bool(ints.iter().map(|&i| i != 0).collect()),
        LeafType::I8 => Column::I8(ints.iter().map(|&i| i8::saturating_from(i)).collect()),
        LeafType::U8 => Column::U8(ints.iter().map(|&i| u8::saturating_from(i)).collect()),
        LeafType::I16 => Column::I16(ints.iter().map(|&i| i16::saturating_from(i)).collect()),
        LeafType::U16 => Column::U16(ints.iter().map(|&i| u16::saturating_from(i)).collect()),
        LeafType::I32 => Column::I32(ints.iter().map(|&i| i32::saturating_from(i)).collect()),
        LeafType::U32 => Column::U32(ints.iter().map(|&i| u32::saturating_from(i)).collect()),
        LeafType::I64 => Column::I64(ints),
        LeafType::U64 => Column::U64(ints.iter().map(|&i| i.cast_unsigned()).collect()),
        LeafType::F32 => Column::F32(floats(vals).iter().map(|&f| f as f32).collect()),
        LeafType::F64 => Column::F64(floats(vals)),
    }
}

/// Per-entry byte regions of a fixed-size array branch: each basket's entry data
/// split into `stride`-byte chunks.
fn chunk_regions(baskets: &[Basket], stride: usize) -> Vec<&[u8]> {
    let mut regions = Vec::new();
    for b in baskets {
        if stride == 0 {
            continue;
        }
        for chunk in b.entry_data().chunks_exact(stride) {
            regions.push(chunk);
        }
    }
    regions
}

/// Decode `bytes` as a contiguous big-endian array of `leaf`-typed scalars.
pub(super) fn decode_scalar(leaf: LeafType, bytes: &[u8]) -> Result<Column> {
    macro_rules! be {
        ($variant:ident, $ty:ty, $w:expr) => {{
            let chunks = bytes.as_chunks::<{ $w }>().0;
            Column::$variant(chunks.iter().map(|c| <$ty>::from_be_bytes(*c)).collect())
        }};
    }
    Ok(match leaf {
        LeafType::Bool => Column::Bool(bytes.iter().map(|&b| b != 0).collect()),
        LeafType::I8 => Column::I8(bytes.iter().map(|&b| b.cast_signed()).collect()),
        LeafType::U8 => Column::U8(bytes.to_vec()),
        LeafType::I16 => be!(I16, i16, 2),
        LeafType::U16 => be!(U16, u16, 2),
        LeafType::I32 => be!(I32, i32, 4),
        LeafType::U32 => be!(U32, u32, 4),
        LeafType::I64 => be!(I64, i64, 8),
        LeafType::U64 => be!(U64, u64, 8),
        LeafType::F32 => be!(F32, f32, 4),
        LeafType::F64 => be!(F64, f64, 8),
        LeafType::Str => return Err(Error::Format("string branch decoded as numbers".into())),
    })
}

/// The values of every region back to back, and where each region ends among
/// them: one decode for the whole branch, no allocation per entry.
fn decode_regions(leaf: LeafType, regions: &[&[u8]]) -> Result<(Vec<u64>, Column)> {
    let size = leaf.size().max(1);
    let mut offsets = Vec::with_capacity(regions.len() + 1);
    offsets.push(0u64);
    let mut bytes = Vec::with_capacity(regions.iter().map(|r| r.len()).sum());
    let mut acc = 0u64;
    for r in regions {
        // Whole elements only: a region cut mid-element loses its tail, as
        // decoding it alone would.
        let whole = r.len() / size * size;
        bytes.extend_from_slice(&r[..whole]);
        acc += (whole / size) as u64;
        offsets.push(acc);
    }
    Ok((offsets, decode_scalar(leaf, &bytes)?))
}

/// One list of `leaf`-typed values per region.
fn decode_list(leaf: LeafType, regions: &[&[u8]]) -> Result<Column> {
    let (offsets, values) = decode_regions(leaf, regions)?;
    Ok(Column::List {
        offsets,
        values: Box::new(values),
    })
}

/// One array of `len` `leaf`-typed values per region.
fn decode_array(leaf: LeafType, len: usize, regions: &[&[u8]]) -> Result<Column> {
    let (offsets, values) = decode_regions(leaf, regions)?;
    if offsets.windows(2).any(|w| w[1] - w[0] != len as u64) {
        return Err(Error::Format(format!(
            "a fixed array of {len} has an entry of another length"
        )));
    }
    Ok(Column::Array {
        len,
        values: Box::new(values),
    })
}

/// Decode `std::vector<std::vector<T>>` entries into a list of lists.
/// Each region is the outer vector's 10-byte streamer header (byte count +
/// version + outer count) followed by that many inner vectors, each a `u32`
/// element count then the contiguous big-endian elements.
fn decode_nested_vec(regions: &[&[u8]], elem: LeafType) -> Result<Column> {
    let size = elem.size();
    let mut offsets = Vec::with_capacity(regions.len() + 1);
    offsets.push(0u64);
    let mut total = 0u64;
    let mut inner: Vec<&[u8]> = Vec::new();
    for region in regions {
        if region.len() >= 10 {
            // The outer count is the last 4 bytes of the 10-byte header.
            let outer = be_u32_at(region, 6).ok_or_else(short_entry)? as usize;
            let mut pos = 10;
            for _ in 0..outer {
                let count = be_u32_at(region, pos).ok_or_else(short_entry)? as usize;
                pos += 4;
                let nbytes = count * size;
                inner.push(region.get(pos..pos + nbytes).ok_or_else(short_entry)?);
                pos += nbytes;
            }
            total += outer as u64;
        }
        offsets.push(total);
    }
    Ok(Column::List {
        offsets,
        values: Box::new(decode_list(elem, &inner)?),
    })
}

/// The big-endian `u32` at byte `at` of `bytes`, if it is all there.
fn be_u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let word = bytes.get(at..)?.first_chunk::<4>()?;
    Some(u32::from_be_bytes(*word))
}

/// Error for a `std::vector<std::vector<T>>` entry that ends mid-element.
fn short_entry() -> Error {
    Error::Format("nested vector entry truncated".into())
}

/// Strings read one after another into a string column.
struct StringsBuilder {
    offsets: Vec<u64>,
    text: String,
}

impl StringsBuilder {
    fn new() -> Self {
        Self {
            offsets: vec![0],
            text: String::new(),
        }
    }

    /// Read the next ROOT-encoded string from `r`.
    fn read(&mut self, r: &mut RBuffer<'_>) -> Result<()> {
        self.text.push_str(r.str()?);
        self.offsets.push(self.text.len() as u64);
        Ok(())
    }

    /// The number of strings read.
    fn len(&self) -> u64 {
        self.offsets.len() as u64 - 1
    }

    fn finish(self) -> Column {
        Column::String {
            offsets: self.offsets,
            text: self.text,
        }
    }
}

/// Decode `std::vector<std::string>` entries: each region is a 10-byte streamer
/// header (byte count + version + `u32` element count) followed by that many
/// ROOT-encoded strings.
fn decode_vec_strings(regions: &[&[u8]]) -> Result<Column> {
    let mut offsets = Vec::with_capacity(regions.len() + 1);
    offsets.push(0u64);
    let mut strings = StringsBuilder::new();
    for r in regions {
        let mut buf = RBuffer::new(r);
        if buf.remaining() >= 10 {
            buf.be_u32()?; // byte count
            buf.be_u16()?; // version
            let count = buf.be_u32()? as usize;
            for _ in 0..count {
                strings.read(&mut buf)?;
            }
        }
        offsets.push(strings.len());
    }
    Ok(Column::List {
        offsets,
        values: Box::new(strings.finish()),
    })
}

/// Decode `TLeafC` baskets: each entry is one ROOT-encoded (length-prefixed)
/// string, read sequentially from the entry data.
fn decode_strings(baskets: &[Basket]) -> Result<Column> {
    let mut strings = StringsBuilder::new();
    for b in baskets {
        let mut r = RBuffer::new(b.entry_data());
        for _ in 0..b.n_entries {
            strings.read(&mut r)?;
        }
    }
    Ok(strings.finish())
}
