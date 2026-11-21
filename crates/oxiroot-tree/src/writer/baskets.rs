//! Writing branch data into `TBasket` records, whole or in chunks.

use oxiroot_io_core::Compression;
use oxiroot_io_core::cast::SaturatingFrom;
use std::io::{Seek, Write};

use oxiroot_io_core::raw::{ContainerWriter, DATIME, DirId, Key, compress_if_smaller};
use oxiroot_io_core::{Error, Result, WBuffer};

use super::branch::{Branch, BranchKind, vec_row_lengths};
use super::layout::Kind;
use crate::value::BranchValues;

/// One basket's recorded location, for the branch metadata.
#[derive(Clone, Copy)]
pub(super) struct BasketRec {
    pub(super) seek: u64,
    pub(super) nbytes: u32,
    /// Number of entries this basket holds (for the cumulative `fBasketEntry`).
    pub(super) n_entries: u32,
}

/// Serialize and compress one basket of `branch`, then write it at the end of
/// `file` under directory `dir`.
pub(super) fn write_basket<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    branch: &Branch,
    tree_name: &str,
) -> Result<BasketRec> {
    let prepared = prepare_basket(branch, tree_name, file.compression())?;
    place_basket(file, dir, &prepared, tree_name)
}

/// One basket serialized and compressed, not yet placed: everything but its
/// file offset, which only the writer knows. Independent of other baskets, so
/// a batch's baskets can be prepared in parallel.
pub(super) struct PreparedBasket {
    name: String,
    n_entries: u32,
    nev_buf_size: i32,
    klen: u32,
    obj_len: u32,
    border: u32,
    payload: Vec<u8>,
}

/// The most bytes a basket's key header and uncompressed buffer may take
/// together: `fLast`, `fNbytes` and the entry offsets are signed 32-bit.
const MAX_BASKET_BYTES: usize = i32::MAX as usize;

/// Serialize `branch`'s entries into a basket buffer and compress it.
///
/// # Errors
///
/// [`Error::InvalidInput`] if the basket would pass the format's 2 GiB limit:
/// its sizes and offsets would not fit their fields, and the file would be
/// corrupt. Writing fewer entries per basket avoids it.
pub(super) fn prepare_basket(
    branch: &Branch,
    tree_name: &str,
    compression: Compression,
) -> Result<PreparedBasket> {
    let (data, offsets) = branch.basket_content();
    let n_entries = branch.n_entries();
    let leaf = branch.leaf();
    // `fNevBufSize` is the per-entry buffer size: `flen * elem_size` for a
    // fixed/scalar branch; ROOT writes a default (1000) for variable baskets.
    let nev_buf_size = match branch.kind() {
        Kind::Str | Kind::Jagged | Kind::StlVector => 1000,
        _ => branch.flen() * leaf.size,
    };

    let klen = u32::saturating_from(Key::header_len("TBasket", &branch.name, tree_name, true)) + 19;
    // One 4-byte offset per entry plus the count, when the branch has them.
    let offsets_len = offsets.as_ref().map_or(0, |o| 4 * (o.len() + 1));
    let total = (klen as usize)
        .saturating_add(data.len())
        .saturating_add(offsets_len);
    if total > MAX_BASKET_BYTES {
        return Err(Error::InvalidInput(format!(
            "branch {:?}: a basket of {n_entries} entries takes {total} bytes, more than the \
             2 GiB a TBasket can hold; write fewer entries per basket",
            branch.name
        )));
    }
    let border = u32::saturating_from(data.len());

    // The uncompressed buffer is the entry data, then (for a variable branch)
    // the `fEntryOffset` array: `int32 count(=n_entries+1)` followed by
    // basket-relative offsets (the data-relative offsets plus `fKeyLen`).
    let mut buffer = data;
    if let Some(offs) = &offsets {
        buffer.extend_from_slice(&i32::saturating_from(offs.len()).to_be_bytes());
        for &o in offs {
            buffer.extend_from_slice(&(o + klen).cast_signed().to_be_bytes());
        }
    }
    let obj_len = u32::saturating_from(buffer.len());
    let payload = compress_if_smaller(&buffer, compression).into_owned();
    Ok(PreparedBasket {
        name: branch.name.clone(),
        n_entries,
        nev_buf_size,
        klen,
        obj_len,
        border,
        payload,
    })
}

/// Write a prepared basket at the end of `file`: its key header, with the
/// offset it lands at, then the compressed payload.
pub(super) fn place_basket<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    p: &PreparedBasket,
    tree_name: &str,
) -> Result<BasketRec> {
    let seek = file.position();
    let seek_pdir = file.dir_offset(dir)?;
    let nbytes = p.klen + u32::saturating_from(p.payload.len());
    let f_last = p.klen + p.border; // entry data ends at the border

    let mut w = WBuffer::with_capacity(p.klen as usize);
    // Big-format Key header.
    w.be_i32(nbytes.cast_signed());
    w.be_u16(1004); // big-format key version
    w.be_u32(p.obj_len);
    w.be_u32(DATIME);
    w.be_u16(u16::saturating_from(p.klen));
    w.be_u16(0); // cycle
    w.be_u64(seek);
    w.be_u64(seek_pdir); // fSeekPdir
    w.string("TBasket");
    w.string(&p.name);
    w.string(tree_name);
    // TBasket extension (the tail of fKeyLen).
    w.be_u16(3); // basket version
    w.be_i32(32000); // fBufferSize
    w.be_i32(p.nev_buf_size); // fNevBufSize
    w.be_i32(p.n_entries.cast_signed()); // fNevBuf
    w.be_i32(f_last.cast_signed()); // fLast
    w.u8(0); // flag
    // The header, then the payload after it: two writes, no copy of the
    // payload into the header's buffer.
    file.place_blob(w.as_slice())?;
    file.place_blob(&p.payload)?;
    Ok(BasketRec {
        seek,
        nbytes,
        n_entries: p.n_entries,
    })
}

/// A sub-range `[start, start+len)` of a non-split branch's entries, as a fresh
/// `Branch` — used to split a branch into multiple baskets.
fn chunk_branch(branch: &Branch, start: usize, len: usize) -> Branch {
    Branch {
        name: branch.name.clone(),
        values: chunk_values(&branch.values, start, len),
        kind: branch.chunk_kind(),
        meta: None,
    }
}

/// Slice a [`BranchValues`] to `[start, start+len)` (clamped), preserving variant.
pub(super) fn chunk_values(bv: &BranchValues, start: usize, len: usize) -> BranchValues {
    use BranchValues::*;
    macro_rules! sl {
        ($variant:ident, $v:expr) => {{
            let end = (start + len).min($v.len());
            let s = start.min(end);
            $variant($v[s..end].to_vec())
        }};
    }
    match bv {
        Bool(v) => sl!(Bool, v),
        I8(v) => sl!(I8, v),
        U8(v) => sl!(U8, v),
        I16(v) => sl!(I16, v),
        U16(v) => sl!(U16, v),
        I32(v) => sl!(I32, v),
        U32(v) => sl!(U32, v),
        I64(v) => sl!(I64, v),
        U64(v) => sl!(U64, v),
        F32(v) => sl!(F32, v),
        F64(v) => sl!(F64, v),
        VecBool(v) => sl!(VecBool, v),
        VecI8(v) => sl!(VecI8, v),
        VecU8(v) => sl!(VecU8, v),
        VecI16(v) => sl!(VecI16, v),
        VecU16(v) => sl!(VecU16, v),
        VecI32(v) => sl!(VecI32, v),
        VecU32(v) => sl!(VecU32, v),
        VecI64(v) => sl!(VecI64, v),
        VecU64(v) => sl!(VecU64, v),
        VecF32(v) => sl!(VecF32, v),
        VecF64(v) => sl!(VecF64, v),
        Str(v) => sl!(Str, v),
        VecStr(v) => sl!(VecStr, v),
        Nested { offsets, items } => {
            let n = offsets.len().saturating_sub(1);
            let end = (start + len).min(n);
            let s = start.min(end);
            let (lo, hi) = (offsets[s], offsets[end]);
            Nested {
                offsets: offsets[s..=end].iter().map(|&o| o - lo).collect(),
                items: Box::new(chunk_values(
                    items,
                    usize::saturating_from(lo),
                    usize::saturating_from(hi - lo),
                )),
            }
        }
    }
}

/// The branches a split `std::vector<MyStruct>` branch's entries
/// `[start, start+len)` are written as: a *count* branch (the parent's
/// per-entry element counts, as single-element jagged `i32` rows, so it carries
/// the `fEntryOffset` ROOT writes for the parent), then one jagged branch per
/// member (`parent.member`). One basket of each makes one chunk.
pub(super) fn split_pieces(branch: &Branch, start: usize, len: usize) -> Vec<Branch> {
    let spec = branch.split().expect("a split branch");
    let first = chunk_values(&spec.members[0].values, start, len);
    let counts = vec_row_lengths(&first);
    let mut out = Vec::with_capacity(spec.members.len() + 1);
    out.push(Branch {
        name: branch.name.clone(),
        values: BranchValues::VecI32(counts.into_iter().map(|n| vec![n]).collect()),
        kind: BranchKind::Jagged,
        meta: None,
    });
    for (i, m) in spec.members.iter().enumerate() {
        out.push(Branch {
            name: format!("{}.{}", branch.name, m.name),
            values: if i == 0 {
                first.clone()
            } else {
                chunk_values(&m.values, start, len)
            },
            kind: BranchKind::Jagged,
            meta: None,
        });
    }
    out
}

/// Write the basket(s) backing one branch, in chunks of `entries_per_basket`
/// entries (`0`: one chunk). A leaf branch has one basket per chunk. A split
/// `std::vector<MyStruct>` branch has, per chunk, a count basket then one
/// basket per member sub-branch ([`split_pieces`]): its group is
/// `[count, member 0, member 1, …]` repeated, the order
/// `serialize::write_split_parent` reads them back in. An empty branch still
/// gets one (empty) chunk.
pub(super) fn write_branch_baskets<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    branch: &Branch,
    tree_name: &str,
    entries_per_basket: usize,
) -> Result<Vec<BasketRec>> {
    let n = branch.n_entries() as usize;
    let epb = if entries_per_basket == 0 {
        n.max(1)
    } else {
        entries_per_basket
    };
    let mut recs = Vec::new();
    let mut start = 0;
    loop {
        let len = epb.min(n - start);
        if branch.split().is_some() {
            for piece in split_pieces(branch, start, len) {
                recs.push(write_basket(file, dir, &piece, tree_name)?);
            }
        } else {
            let chunk = if start == 0 && len == n {
                write_basket(file, dir, branch, tree_name)?
            } else {
                write_basket(file, dir, &chunk_branch(branch, start, len), tree_name)?
            };
            recs.push(chunk);
        }
        start += len;
        if start >= n {
            break;
        }
    }
    Ok(recs)
}
