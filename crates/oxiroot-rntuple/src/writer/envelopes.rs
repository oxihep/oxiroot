//! The header, page-list and footer envelopes, and the anchor.
//!
//! Each envelope is written into one buffer: a frame's size goes in a
//! reserved slot that is filled when the frame ends, so nothing is built
//! apart and copied in.

use oxiroot_io_core::Compression;
use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result, WBuffer};

use super::lower::{ColumnPlan, FieldPlan};
use super::{CHECKSUM_TYPE_VERSION, COLUMN_FLAG_DEFERRED, COLUMN_FLAG_RANGE, K_BYTE_COUNT_MASK};

/// An envelope being written.
struct Out(WBuffer);

impl Out {
    /// Start an envelope: the slot for its type and length comes first.
    fn new() -> (Self, oxiroot_io_core::Patch<8>) {
        let mut w = WBuffer::new();
        let word = w.reserve::<8>();
        (Self(w), word)
    }

    /// End the envelope: fill in its type and length, and append the
    /// checksum of everything before it.
    fn finish(mut self, type_id: u16, word: oxiroot_io_core::Patch<8>) -> Vec<u8> {
        let length = (self.0.len() + 8) as u64;
        self.0
            .patch_le_u64(word, u64::from(type_id) | (length << 16));
        let checksum = xxhash_rust::xxh3::xxh3_64(self.0.as_slice());
        self.0.le_u64(checksum);
        self.0.into_vec()
    }

    /// A string: its length as a `u32`, then its bytes.
    fn str(&mut self, s: &str) {
        self.0.le_u32(u32::saturating_from(s.len()));
        self.0.bytes(s.as_bytes());
    }

    /// A record frame around what `body` writes: its size, counting the size
    /// field itself, then the body.
    fn record<R>(&mut self, body: impl FnOnce(&mut Self) -> R) -> R {
        let start = self.0.len();
        let size = self.0.reserve::<8>();
        let out = body(self);
        self.0.patch_le_u64(size, (self.0.len() - start) as u64);
        out
    }

    /// A list frame of `count` items written by `body`: the size negated,
    /// which is what marks a list, then the count, then the items.
    fn list<R>(&mut self, count: usize, body: impl FnOnce(&mut Self) -> R) -> R {
        let start = self.0.len();
        let size = self.0.reserve::<8>();
        self.0.le_u32(u32::saturating_from(count));
        let out = body(self);
        let len = i64::saturating_from(self.0.len() - start);
        self.0.patch_le_u64(size, (-len).cast_unsigned());
        out
    }

    /// A list with nothing in it.
    fn empty_list(&mut self) {
        self.list(0, |_| ());
    }
}

/// One field descriptor record (shared by the header and the footer's
/// schema-extension record).
fn field_record(o: &mut Out, f: &FieldPlan) {
    o.record(|o| {
        o.0.le_u32(0); // field version
        // A checksummed (user-class) field records type version -1.
        o.0.le_u32(if f.type_checksum.is_some() {
            CHECKSUM_TYPE_VERSION
        } else {
            0
        });
        o.0.le_u32(f.parent_id);
        o.0.le_u16(f.role); // struct role
        o.0.le_u16(f.flags);
        o.str(&f.name);
        o.str(&f.type_name);
        o.str(""); // type alias
        o.str(""); // description
        // Flag-gated trailers (reader order: array, projection source, checksum).
        if let Some(size) = f.array_size {
            o.0.le_u64(size);
        }
        if let Some(checksum) = f.type_checksum {
            o.0.le_u32(checksum);
        }
    });
}

/// One column descriptor record. `first_element_index` is `Some` for a deferred
/// (schema-extension) column — its data starts at that global element index — and
/// sets the [`COLUMN_FLAG_DEFERRED`] flag plus the index trailer (before the value
/// range, matching the reader's order).
fn column_record(o: &mut Out, c: &ColumnPlan, first_element_index: Option<i64>) {
    o.record(|o| {
        o.0.le_u16(c.column_type as u16);
        o.0.le_u16(c.bits);
        o.0.le_u32(c.field_id);
        let mut flags = 0u16;
        if first_element_index.is_some() {
            flags |= COLUMN_FLAG_DEFERRED;
        }
        if c.value_range.is_some() {
            flags |= COLUMN_FLAG_RANGE;
        }
        o.0.le_u16(flags);
        o.0.le_u16(0); // representation index
        if let Some(first) = first_element_index {
            o.0.le_i64(first);
        }
        if let Some((min, max)) = c.value_range {
            o.0.le_f64(min);
            o.0.le_f64(max);
        }
    });
}

/// The four lists a schema is: fields, columns, then alias columns and extra
/// type information, which this writer leaves empty. The header holds one,
/// and so does the footer's extension record.
fn schema(o: &mut Out, fields: &[FieldPlan], cols: &[ColumnPlan], first_entries: Option<&[i64]>) {
    o.list(fields.len(), |o| {
        for f in fields {
            field_record(o, f);
        }
    });
    o.list(cols.len(), |o| {
        for (i, c) in cols.iter().enumerate() {
            column_record(o, c, first_entries.map(|first| first[i]));
        }
    });
    o.empty_list(); // alias columns
    o.empty_list(); // extra type info
}

pub(super) fn build_header(name: &str, fields: &[FieldPlan], cols: &[ColumnPlan]) -> Vec<u8> {
    let (mut o, word) = Out::new();
    o.0.le_i64(0); // feature flags
    o.str(name);
    o.str(""); // description
    o.str("oxiroot"); // writer
    schema(&mut o, fields, cols, None);
    o.finish(0x01, word)
}

/// A page-list entry stores a page's element count and on-disk size as signed
/// 32-bit fields — the element count's sign bit flags a trailing per-page
/// checksum — so a single page holds at most `i32::MAX` elements and `i32::MAX`
/// on-disk bytes. Reject anything larger rather than letting the `as i32` cast
/// wrap into a negative value that mislabels the page (a corrupt file). A genuine
/// page that big would need to be split across more clusters by the caller.
pub(super) fn check_page_limits(n_elements: u32, disk_size: usize) -> Result<()> {
    if n_elements > i32::MAX as u32 {
        return Err(Error::InvalidInput(format!(
            "RNTuple page has {n_elements} elements, over the per-page limit of {} \
             (write fewer entries per cluster)",
            i32::MAX
        )));
    }
    if disk_size > i32::MAX as usize {
        return Err(Error::InvalidInput(format!(
            "RNTuple page on-disk size {disk_size} exceeds the per-page limit of {} bytes",
            i32::MAX
        )));
    }
    Ok(())
}

/// Where one column's page of one cluster is, for the page list.
pub(super) struct PageRec {
    pub(super) offset: u64,
    pub(super) disk_size: usize,
    pub(super) n_elements: u32,
    /// The index of the page's first element in the whole column: past the
    /// earlier clusters' elements, and for a column added late, past the
    /// entries it has no data for.
    pub(super) element_offset: i64,
}

/// The page-list envelope: one summary `(first entry, entry count)` per
/// cluster, then the page locations nested clusters → columns → one page.
pub(super) fn build_page_list(
    summaries: &[(u64, u64)],
    cluster_pages: &[Vec<PageRec>],
    compression: Compression,
    header_checksum: u64,
) -> Result<Vec<u8>> {
    let (mut o, word) = Out::new();
    o.0.le_u64(header_checksum);
    o.list(summaries.len(), |o| {
        for &(first, n) in summaries {
            o.record(|o| {
                o.0.le_u64(first);
                o.0.le_u64(n); // the high byte is the flags, 0
            });
        }
    });
    o.list(cluster_pages.len(), |o| {
        for cols in cluster_pages {
            o.list(cols.len(), |o| {
                for page in cols {
                    check_page_limits(page.n_elements, page.disk_size)?;
                    // A list of one page, with the column's element offset
                    // and compression after it in the same frame.
                    o.list(1, |o| {
                        o.0.le_i32(page.n_elements.cast_signed()); // positive: no checksum
                        o.0.le_i32(i32::saturating_from(page.disk_size)); // locator size
                        o.0.le_u64(page.offset); // locator offset
                        o.0.le_i64(page.element_offset);
                        o.0.le_u32(compression.setting());
                    });
                }
                Ok::<(), Error>(())
            })?;
        }
        Ok::<(), Error>(())
    })?;
    Ok(o.finish(0x03, word))
}

/// The fields and columns added after the header was written, for the
/// footer's schema-extension record.
pub(super) struct Extension<'a> {
    pub(super) fields: &'a [FieldPlan],
    pub(super) columns: &'a [ColumnPlan],
    /// Each column's first element index, in the order of `columns`.
    pub(super) first_entries: &'a [i64],
}

/// The footer envelope. With an `extension`, readers merge its fields and
/// columns into the schema and fill the entries before each one's first.
pub(super) fn build_footer(
    n_entries: u32,
    num_clusters: u32,
    page_list_offset: usize,
    page_list_len: usize,
    header_checksum: u64,
    extension: Option<&Extension<'_>>,
) -> Vec<u8> {
    let (mut o, word) = Out::new();
    o.0.le_i64(0); // feature flags
    o.0.le_u64(header_checksum);
    o.record(|o| match extension {
        Some(ext) => schema(o, ext.fields, ext.columns, Some(ext.first_entries)),
        None => schema(o, &[], &[], None),
    });
    // One cluster group spanning every cluster; it links to the single page-list
    // envelope that details all clusters' pages.
    o.list(1, |o| {
        o.record(|o| {
            o.0.le_u64(0); // min entry
            o.0.le_u64(u64::from(n_entries)); // entry span
            o.0.le_u32(num_clusters);
            o.0.le_u64(page_list_len as u64); // envelope link: uncompressed len
            o.0.le_i32(i32::saturating_from(page_list_len)); // locator size
            o.0.le_u64(page_list_offset as u64); // locator offset
        });
    });
    // Linked attribute sets (RNTuple format >= 1.0.1.0); empty here.
    o.empty_list();
    o.finish(0x02, word)
}

pub(super) fn build_anchor(
    seek_header: usize,
    len_header: usize,
    seek_footer: usize,
    len_footer: usize,
) -> Vec<u8> {
    let mut w = WBuffer::with_capacity(70);
    w.be_u32(66 | K_BYTE_COUNT_MASK);
    w.be_u16(2); // class version
    let fields = w.len();
    w.be_u16(1); // epoch
    w.be_u16(0); // major
    w.be_u16(1); // minor
    w.be_u16(1); // patch
    w.be_u64(seek_header as u64);
    w.be_u64(len_header as u64);
    w.be_u64(len_header as u64);
    w.be_u64(seek_footer as u64);
    w.be_u64(len_footer as u64);
    w.be_u64(len_footer as u64);
    w.be_u64(0x4000_0000); // max key size
    let checksum = xxhash_rust::xxh3::xxh3_64(&w.as_slice()[fields..]);
    w.be_u64(checksum);
    w.into_vec()
}
