//! Writing an `RNTuple` into a ROOT file.
//!
//! [`NtupleWriter`] writes an `RNTuple` to its own file, one cluster per batch,
//! so a large dataset need not be held in memory at once; [`raw::write_ntuple`]
//! places a whole `RNTuple` (schema-extension fields included) into an open
//! container, next to other objects. The header/page/
//! page-list/footer envelopes are written as raw blobs at the offsets the anchor
//! (and the page locators) point to; only the anchor is a `Key`. Validated by
//! reading the result back and by official ROOT / uproot.

use oxiroot_io_core::Compression;
use oxiroot_io_core::cast::SaturatingFrom;
use std::io::{Seek, Write};

use oxiroot_io_core::raw::{ContainerWriter, DirId, compress_if_smaller};
use oxiroot_io_core::{Error, Result};

use crate::anchor::ANCHOR_CLASS;

mod check;
mod classes;
mod envelopes;
mod fields;
mod lower;
mod streaming;

use classes::ntuple_classes;
use envelopes::{
    build_anchor, build_footer, build_footer_ext, build_header, build_page_list,
    build_page_list_offsets,
};
pub use fields::{Column, Field};
use lower::{ColumnPlan, FieldPlan, lower};
pub use streaming::NtupleWriter;

const K_BYTE_COUNT_MASK: u32 = 0x4000_0000;

/// The stored bytes of one page: compressed when that makes it smaller (a reader
/// tells the two apart by comparing the stored size with the size the element
/// count implies, as ROOT does).
fn on_disk_page(page: &[u8], compression: Compression) -> Vec<u8> {
    compress_if_smaller(page, compression).into_owned()
}

const ROLE_LEAF: u16 = 0;
const ROLE_COLLECTION: u16 = 1;
const ROLE_RECORD: u16 = 2;
const ROLE_VARIANT: u16 = 3;

/// Field flag: the field is a fixed-size array (`std::array`/`std::bitset`); an
/// element count (`u64`) trails the field record.
const FIELD_FLAG_ARRAY: u16 = 0x01;
/// Field flag: a type checksum (`u32`) trails the field record (user classes).
const FIELD_FLAG_CHECKSUM: u16 = 0x04;
/// The on-disk type version a checksummed user-class field carries (`-1`).
const CHECKSUM_TYPE_VERSION: u32 = 0xFFFF_FFFF;

/// Column flag: the descriptor carries an `(f64, f64)` value range (e.g. for a
/// quantized real column).
const COLUMN_FLAG_RANGE: u16 = 0x02;

/// Column flag: a deferred column (a schema-extension field added late) — the
/// descriptor carries an `i64` first-element index trailer.
const COLUMN_FLAG_DEFERRED: u16 = 0x01;

/// One `RNTuple`'s fully-lowered, page-encoded payload, ready to place into a file
/// at any offset: the header envelope (and its checksum), each column's on-disk
/// page bytes, the column plans, and the entry count. Independent of where in the
/// file it lands, so the same prep serves both container-form passes.
struct NtuplePrep {
    name: String,
    header_env: Vec<u8>,
    header_checksum: u64,
    disk_pages: Vec<Vec<u8>>,
    disk_sizes: Vec<usize>,
    cols: Vec<ColumnPlan>,
    n_entries: u32,
    /// Set when this is a schema-extended `RNTuple`: the late field/column
    /// descriptors (placed in the footer's extension record) and the per-column
    /// element offsets. `cols`/`disk_pages` already include the late columns.
    ext: Option<ExtPrep>,
}

/// The schema-extension half of a [`NtuplePrep`]: the late field descriptors, the
/// boundary between header and late columns, and each column's first-element
/// offset (`0` for header columns, the late field's first entry for late ones).
struct ExtPrep {
    ext_fields: Vec<FieldPlan>,
    base_col_count: usize,
    first_entries: Vec<i64>,
    element_offsets: Vec<i64>,
}

/// Lower one `RNTuple`'s fields and encode its pages (the placement-independent
/// work), shared by every multi-RNTuple file pass.
fn prep_ntuple(name: &str, fields: &[Field], compression: Compression) -> Result<NtuplePrep> {
    let (field_plans, cols, n_entries) = lower(fields)?;
    let header_env = build_header(name, &field_plans, &cols);
    let header_checksum = crate::envelope::trailing_checksum(&header_env)
        .ok_or_else(|| Error::Format("the header envelope has no checksum".into()))?;
    let disk_pages: Vec<Vec<u8>> = cols
        .iter()
        .map(|c| on_disk_page(&c.page, compression))
        .collect();
    let disk_sizes: Vec<usize> = disk_pages.iter().map(std::vec::Vec::len).collect();
    Ok(NtuplePrep {
        name: name.to_string(),
        header_env,
        header_checksum,
        disk_pages,
        disk_sizes,
        cols,
        n_entries,
        ext: None,
    })
}

/// Lower a schema-extended `RNTuple`: `base_fields` go in the header, and each
/// `(first_entry, late_field)` becomes a deferred field in the footer's extension
/// record whose data covers entries `first_entry..n_entries`. The late field's
/// IDs continue the base's. Late fields must be scalar leaves (one column each)
/// and supply exactly `n_entries - first_entry` values.
fn prep_ntuple_extended(
    name: &str,
    base_fields: &[Field],
    late: &[(u64, Field)],
    compression: Compression,
) -> Result<NtuplePrep> {
    let (base_fields_plan, base_cols, n_entries) = lower(base_fields)?;
    let header_env = build_header(name, &base_fields_plan, &base_cols);
    let header_checksum = crate::envelope::trailing_checksum(&header_env)
        .ok_or_else(|| Error::Format("the header envelope has no checksum".into()))?;

    let base_col_count = base_cols.len();
    let mut cols = base_cols;
    let mut ext_fields = Vec::new();
    let mut first_entries = Vec::new();
    let mut element_offsets = vec![0i64; base_col_count];

    for (first_entry, field) in late {
        let (mut late_fp, mut late_cols, late_n) = lower(std::slice::from_ref(field))?;
        if late_cols.len() != 1 || late_fp.len() != 1 {
            return Err(Error::Unsupported(format!(
                "late RNTuple field {:?} must be a scalar leaf",
                field.name
            )));
        }
        if *first_entry + u64::from(late_n) != u64::from(n_entries) {
            return Err(Error::LengthMismatch {
                what: format!(
                    "late RNTuple field {:?}, from entry {first_entry}",
                    field.name
                ),
                expected: usize::saturating_from(u64::from(n_entries).saturating_sub(*first_entry)),
                found: late_n as usize,
            });
        }
        // Continue the schema's field/column IDs past everything added so far.
        let id_base = u32::saturating_from(base_fields_plan.len() + ext_fields.len());
        for f in &mut late_fp {
            f.parent_id += id_base;
        }
        for c in &mut late_cols {
            c.field_id += id_base;
        }
        ext_fields.append(&mut late_fp);
        first_entries.push((*first_entry).cast_signed());
        element_offsets.push((*first_entry).cast_signed());
        cols.append(&mut late_cols);
    }

    let disk_pages: Vec<Vec<u8>> = cols
        .iter()
        .map(|c| on_disk_page(&c.page, compression))
        .collect();
    let disk_sizes: Vec<usize> = disk_pages.iter().map(std::vec::Vec::len).collect();

    Ok(NtuplePrep {
        name: name.to_string(),
        header_env,
        header_checksum,
        disk_pages,
        disk_sizes,
        cols,
        n_entries,
        ext: Some(ExtPrep {
            ext_fields,
            base_col_count,
            first_entries,
            element_offsets,
        }),
    })
}

/// Write one `RNTuple`'s blobs (header, pages, page list, footer) at the end of
/// `file`, then its anchor key in directory `dir`.
fn write_one_rntuple<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    p: &NtuplePrep,
) -> Result<()> {
    let compression = file.compression();
    let seek_header = file.place_blob(&p.header_env)?;
    let mut page_offsets = Vec::with_capacity(p.cols.len());
    for dp in &p.disk_pages {
        page_offsets.push(offset(file.place_blob(dp)?)?);
    }
    let page_list_offset = offset(file.position())?;
    let footer_env = if let Some(ext) = &p.ext {
        // Schema-extended: the late columns' pages start at their first element
        // index (the page list records the offset), and the late field/column
        // descriptors go in the footer's schema-extension record.
        let page_list_env = build_page_list_offsets(
            p.n_entries,
            &page_offsets,
            &p.disk_sizes,
            &p.cols,
            &ext.element_offsets,
            compression,
            p.header_checksum,
        )?;
        file.place_blob(&page_list_env)?;
        build_footer_ext(
            p.n_entries,
            1,
            page_list_offset,
            page_list_env.len(),
            p.header_checksum,
            &ext.ext_fields,
            &p.cols[ext.base_col_count..],
            &ext.first_entries,
        )
    } else {
        let page_list_env = build_page_list(
            p.n_entries,
            &page_offsets,
            &p.disk_sizes,
            &p.cols,
            compression,
            p.header_checksum,
        )?;
        file.place_blob(&page_list_env)?;
        build_footer(
            p.n_entries,
            1,
            page_list_offset,
            page_list_env.len(),
            p.header_checksum,
        )
    };
    let seek_footer = file.place_blob(&footer_env)?;

    let anchor = build_anchor(
        offset(seek_header)?,
        p.header_env.len(),
        offset(seek_footer)?,
        footer_env.len(),
    );
    file.place_key_uncompressed(dir, ANCHOR_CLASS, &p.name, "", &anchor)?;
    Ok(())
}

/// A file offset as the `usize` the envelope builders take.
fn offset(seek: u64) -> Result<usize> {
    usize::try_from(seek)
        .map_err(|_| Error::Unsupported(format!("file offset {seek} does not fit this platform")))
}

/// Low-level writing of a whole `RNTuple` into an open ROOT container — what a
/// [`WriteInto`](oxiroot_io_core::WriteInto) object (such as
/// `oxiroot::data::Dataset`'s) calls to place an `RNTuple` next to other
/// objects. For an `RNTuple` written to its own file, cluster by cluster, use
/// [`NtupleWriter`].
pub mod raw {
    use std::io::{Seek, Write};

    use oxiroot_io_core::Result;
    use oxiroot_io_core::raw::{ContainerWriter, DirId};
    use oxiroot_io_core::streamer_gen::ClassDef;

    use super::Field;

    /// Write the `RNTuple` `name` with `fields` into `file` under directory
    /// `dir`: header, pages, page list, footer, then its anchor key. `late`
    /// adds schema-extension fields, each `(first entry, field)`: the field
    /// holds values from that entry on, and ROOT reads defaults before it.
    ///
    /// # Errors
    ///
    /// Fields of different lengths, a type the writer cannot encode, or a write
    /// error.
    pub fn write_ntuple<W: Write + Seek>(
        file: &mut ContainerWriter<W>,
        dir: DirId,
        name: &str,
        fields: &[Field],
        late: &[(u64, Field)],
    ) -> Result<()> {
        let setting = file.compression();
        let prep = if late.is_empty() {
            super::prep_ntuple(name, fields, setting)?
        } else {
            super::prep_ntuple_extended(name, fields, late, setting)?
        };
        super::write_one_rntuple(file, dir, &prep)
    }

    /// The `TStreamerInfo` classes an `RNTuple` with these fields needs in its
    /// file.
    #[must_use]
    pub fn ntuple_streamer_classes<'a>(
        fields: impl IntoIterator<Item = &'a Field>,
    ) -> Vec<ClassDef<'static>> {
        super::ntuple_classes(fields)
            .into_iter()
            .map(ClassDef::into_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::envelopes::check_page_limits;
    use super::*;
    use crate::NtupleReader;
    use crate::raw::FieldValues;
    use oxiroot_io_core::raw::KSTART_BIG_FILE;
    use oxiroot_io_core::{Compression, FileReader};

    fn rntuple_file_bytes_threshold(
        file_name: &str,
        name: &str,
        fields: &[Field],
        compression: Compression,
        threshold: u64,
    ) -> Result<Vec<u8>> {
        let classes = raw::ntuple_streamer_classes(fields);
        ContainerWriter::build(file_name, compression, threshold, |file| {
            raw::write_ntuple(file, DirId::TOP, name, fields, &[])?;
            file.place_streamer_info(&[], &classes)
        })
    }

    fn rntuple_file_bytes(
        file_name: &str,
        name: &str,
        fields: &[Field],
        compression: Compression,
    ) -> Result<Vec<u8>> {
        rntuple_file_bytes_threshold(file_name, name, fields, compression, KSTART_BIG_FILE)
    }

    #[test]
    fn one_shot_writes_and_reads_big_format() {
        let fields = vec![
            Field::i32("x", vec![1, 2, 3, 4]),
            Field::f64("y", vec![1.5, 2.5, 3.5, 4.5]),
        ];

        // Tiny file, but forced into the 64-bit container form via a low
        // threshold — it must still parse and yield identical values.
        let bytes =
            rntuple_file_bytes_threshold("t.root", "ntpl", &fields, Compression::Stored, 64)
                .unwrap();
        // Also drop it to a temp file so an external reader (uproot) can be run
        // against the one-shot big-format output out of band.
        let _ = std::fs::write("/tmp/rootrs_oneshot_big.root", &bytes);
        let f = FileReader::from_bytes(bytes).unwrap();
        assert!(f.header().is_big(), "forced into big-format container");
        let ntpl = NtupleReader::open(&f, "ntpl").unwrap();
        assert_eq!(ntpl.entries(), 4);
        assert_eq!(
            ntpl.field("x").values().unwrap(),
            FieldValues::I32(vec![1, 2, 3, 4])
        );
        assert_eq!(
            ntpl.field("y").values().unwrap(),
            FieldValues::F64(vec![1.5, 2.5, 3.5, 4.5])
        );

        // The same data under the real threshold stays in small (32-bit) form.
        let small = rntuple_file_bytes("t.root", "ntpl", &fields, Compression::Stored).unwrap();
        let fs = FileReader::from_bytes(small).unwrap();
        assert!(!fs.header().is_big());
        let ntpl = NtupleReader::open(&fs, "ntpl").unwrap();
        assert_eq!(
            ntpl.field("x").values().unwrap(),
            FieldValues::I32(vec![1, 2, 3, 4])
        );
    }

    #[test]
    fn page_limits_reject_oversized_counts_and_sizes() {
        // In range, including exactly at the boundary, is accepted.
        assert!(check_page_limits(1_000_000, 1_000_000).is_ok());
        assert!(check_page_limits(i32::MAX as u32, i32::MAX as usize).is_ok());
        // One element past the limit would flip the count's i32 sign bit, which
        // the format reads as "this page has a trailing checksum" — rejected.
        assert!(check_page_limits(i32::MAX as u32 + 1, 0).is_err());
        // One byte past the on-disk-size limit would flip the locator size sign.
        assert!(check_page_limits(0, i32::MAX as usize + 1).is_err());
    }
}
