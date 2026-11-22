//! [`NtupleWriter`]: an `RNTuple` written one cluster per batch, so the whole
//! dataset need not be in memory.

use oxiroot_io_core::cast::SaturatingFrom;
use std::io::{Seek, Write};
use std::path::Path;

use oxiroot_io_core::raw::{ContainerWriter, DirId, KSTART_BIG_FILE, Objects, compress_if_smaller};
use oxiroot_io_core::{Compression, Error, Result};

use super::classes::ntuple_streamer_info;
use super::envelopes::{PageRec, build_anchor, build_footer, build_header, build_page_list};
use super::fields::Field;
use super::lower::{ColumnPlan, FieldPlan, lower};
use super::offset;
use crate::anchor::ANCHOR_CLASS;

/// One batch lowered and its pages compressed, ready to place.
struct PreparedCluster {
    field_plans: Vec<FieldPlan>,
    cols: Vec<ColumnPlan>,
    n_entries: u32,
    disks: Vec<Vec<u8>>,
}

/// A batch's full lowered schema identity. Every batch must produce an equal
/// one, otherwise its pages would be appended under the first batch's header and
/// silently mis-described. Compares field identity (name, type, parent, role) as
/// well as physical columns — `(type, bits)` alone would let a field rename or
/// reorder slip through.
#[derive(PartialEq, Eq)]
struct SchemaSig {
    /// `(name, type_name, parent_id, role)` per field, in lowered order.
    fields: Vec<(String, String, u32, u16)>,
    /// `(column type, bit width, owning field id)` per physical column.
    columns: Vec<(u16, u16, u32)>,
}

fn schema_sig(field_plans: &[FieldPlan], cols: &[ColumnPlan]) -> SchemaSig {
    SchemaSig {
        fields: field_plans
            .iter()
            .map(|f| (f.name.clone(), f.type_name.clone(), f.parent_id, f.role))
            .collect(),
        columns: cols
            .iter()
            .map(|c| (c.column_type as u16, c.bits, c.field_id))
            .collect(),
    }
}

/// Schema + header bookkeeping, fixed once the first batch defines it.
struct HeaderState {
    seek: u64,
    len: usize,
    checksum: u64,
    /// The lowered schema the first batch committed — must match every batch.
    signature: SchemaSig,
    /// The file's streamer info, for the classes in the first batch's fields.
    streamer_info: Vec<u8>,
}

/// A streaming `RNTuple` writer: each [`write_batch`](NtupleWriter::write_batch)
/// flushes one *cluster* to the sink, so a large dataset can be written one
/// chunk at a time without ever holding it all in memory. Call
/// [`finish`](NtupleWriter::finish) to write the page list, footer, and anchor.
///
/// Handles the same field types as [`raw::write_ntuple`](super::raw::write_ntuple) — scalars,
/// `std::string`, and `std::vector<T>` — writing each batch's collection/string
/// index offsets relative to its own cluster, as the format requires.
#[doc(alias = "RNTupleWriter")]
#[must_use = "nothing is readable until `finish()` writes the file's directory"]
pub struct NtupleWriter<W: Write + Seek> {
    file: ContainerWriter<W>,
    ntuple_name: String,
    // Set when the first batch defines the schema and writes the header.
    header: Option<HeaderState>,
    element_base: Vec<u64>,
    // Accumulated per-cluster metadata.
    total_entries: u64,
    summaries: Vec<(u64, u64)>,
    cluster_pages: Vec<Vec<PageRec>>,
}

impl NtupleWriter<std::fs::File> {
    /// Create a streaming `RNTuple` file at `path` (32-bit container; supports up
    /// to 2 GiB — [`finish`](NtupleWriter::finish) errors if that is exceeded).
    ///
    /// # Errors
    ///
    /// If the file cannot be created, or its header cannot be written.
    pub fn create(
        path: impl AsRef<Path>,
        ntuple_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::create_fmt(path, ntuple_name, compression, false)
    }

    /// Like [`create`](NtupleWriter::create), but writes the 64-bit ("big")
    /// container form so the file may exceed 2 GiB. Use this when the streamed
    /// dataset is expected to be large; small files are still valid, just stored
    /// in the wider form.
    ///
    /// # Errors
    ///
    /// As [`create`](Self::create).
    pub fn create_large(
        path: impl AsRef<Path>,
        ntuple_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::create_fmt(path, ntuple_name, compression, true)
    }

    fn create_fmt(
        path: impl AsRef<Path>,
        ntuple_name: &str,
        compression: Compression,
        big: bool,
    ) -> Result<Self> {
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.root")
            .to_string();
        let file = std::fs::File::create(path)?;
        Self::new_fmt(file, &file_name, ntuple_name, compression, big)
    }
}

impl<W: Write + Seek> NtupleWriter<W> {
    /// Begin writing into an arbitrary seekable sink (the `TFile` header and root
    /// directory are written immediately, with pointers to patch at the end).
    /// Small (32-bit) container — see [`new_large`](NtupleWriter::new_large) for
    /// the >2 GiB form.
    ///
    /// # Errors
    ///
    /// If the file's header cannot be written to `sink`.
    pub fn new(
        sink: W,
        file_name: &str,
        ntuple_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::new_fmt(sink, file_name, ntuple_name, compression, false)
    }

    /// Like [`new`](NtupleWriter::new), but writes the 64-bit ("big") container
    /// form so the streamed file may exceed 2 GiB.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_large(
        sink: W,
        file_name: &str,
        ntuple_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::new_fmt(sink, file_name, ntuple_name, compression, true)
    }

    fn new_fmt(
        sink: W,
        file_name: &str,
        ntuple_name: &str,
        compression: Compression,
        big: bool,
    ) -> Result<Self> {
        Ok(Self {
            file: ContainerWriter::new(
                sink,
                file_name,
                compression,
                oxiroot_io_core::raw::Form::big_if(big),
            )?,
            ntuple_name: ntuple_name.to_string(),
            header: None,
            element_base: Vec::new(),
            total_entries: 0,
            summaries: Vec::new(),
            cluster_pages: Vec::new(),
        })
    }

    /// Append one cluster holding the entries in `fields`. All batches must share
    /// the same field schema; the first batch fixes it and writes the header.
    ///
    /// # Errors
    ///
    /// [`Error::SchemaChanged`] if the fields differ from the first batch's,
    /// [`Error::InvalidInput`] for a batch the format cannot hold as declared, and any
    /// error writing to the sink.
    pub fn write_batch(&mut self, fields: &[Field]) -> Result<()> {
        self.write_chunks(&[fields])
    }

    /// Append several clusters at once, one per batch, in order. With the
    /// `rayon` feature every batch is encoded and its pages compressed in
    /// parallel before they are written.
    ///
    /// # Errors
    ///
    /// As [`write_batch`](Self::write_batch), for the first batch that fails.
    pub fn write_batches(&mut self, batches: &[Vec<Field>]) -> Result<()> {
        let slices: Vec<&[Field]> = batches.iter().map(Vec::as_slice).collect();
        self.write_chunks(&slices)
    }

    fn write_chunks(&mut self, batches: &[&[Field]]) -> Result<()> {
        let compression = self.file.compression();
        let prepare = |fields: &&[Field]| -> Result<Option<PreparedCluster>> {
            if fields.is_empty() {
                return Ok(None);
            }
            let (field_plans, cols, n_entries) = lower(fields)?;
            let compress = |c: &ColumnPlan| compress_if_smaller(&c.page, compression).into_owned();
            #[cfg(feature = "rayon")]
            let disks: Vec<Vec<u8>> = {
                use rayon::prelude::*;
                cols.par_iter().map(compress).collect()
            };
            #[cfg(not(feature = "rayon"))]
            let disks: Vec<Vec<u8>> = cols.iter().map(compress).collect();
            Ok(Some(PreparedCluster {
                field_plans,
                cols,
                n_entries,
                disks,
            }))
        };
        #[cfg(feature = "rayon")]
        let prepared: Vec<Option<PreparedCluster>> = {
            use rayon::prelude::*;
            batches.par_iter().map(prepare).collect::<Result<_>>()?
        };
        #[cfg(not(feature = "rayon"))]
        let prepared: Vec<Option<PreparedCluster>> =
            batches.iter().map(prepare).collect::<Result<_>>()?;
        for (p, fields) in prepared.into_iter().zip(batches) {
            if let Some(p) = p {
                // An empty batch writes no cluster, but the first one still
                // fixes the schema, so an RNTuple with no entries can be written.
                if p.n_entries > 0 || self.header.is_none() {
                    self.place(p, fields)?;
                }
            }
        }
        Ok(())
    }

    /// Write one prepared cluster: the header first if this is the first one,
    /// then its pages.
    fn place(&mut self, p: PreparedCluster, fields: &[Field]) -> Result<()> {
        let PreparedCluster {
            field_plans,
            cols,
            n_entries,
            disks,
        } = p;
        let signature = schema_sig(&field_plans, &cols);

        match self.header.as_ref().map(|h| h.signature == signature) {
            Some(true) => {} // schema matches; header already written
            Some(false) => {
                return Err(Error::SchemaChanged {
                    detail: "this batch's field schema differs from the first batch's".into(),
                });
            }
            None => {
                // First batch fixes the schema and writes the header.
                let header_env = build_header(&self.ntuple_name, &field_plans, &cols);
                let checksum = crate::envelope::trailing_checksum(&header_env)
                    .ok_or_else(|| Error::Format("the header envelope has no checksum".into()))?;
                let seek = self.file.place_blob(&header_env)?;
                self.element_base = vec![0u64; cols.len()];
                self.header = Some(HeaderState {
                    seek,
                    len: header_env.len(),
                    checksum,
                    signature,
                    streamer_info: ntuple_streamer_info(fields),
                });
            }
        }

        if n_entries == 0 {
            return Ok(());
        }
        let first_entry = self.total_entries;
        let mut recs = Vec::with_capacity(cols.len());
        for (i, (c, disk)) in cols.iter().zip(&disks).enumerate() {
            let element_offset = self.element_base[i].cast_signed();
            let offset = self.file.place_blob(disk)?;
            recs.push(PageRec {
                offset,
                disk_size: disk.len(),
                n_elements: c.n,
                element_offset,
            });
            self.element_base[i] += u64::from(c.n);
        }
        self.cluster_pages.push(recs);
        self.summaries.push((first_entry, u64::from(n_entries)));
        self.total_entries += u64::from(n_entries);
        Ok(())
    }

    /// Total entries appended so far.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.total_entries
    }

    /// Finish the file: write the page list (all clusters), footer, anchor key,
    /// and key list, then patch the header pointers. Returns the sink.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if no batch was written, [`Error::FileTooLarge`] if a small
    /// file grew past what it can address, and any error writing to the sink.
    pub fn finish(self) -> Result<W> {
        self.finish_with(&Objects::default())
    }

    /// [`finish`](Self::finish), storing `objects` (histograms, parameters)
    /// in the top directory next to the ntuple.
    ///
    /// # Errors
    ///
    /// As [`finish`](Self::finish), and an object that cannot be stored: an
    /// unnamed one, or one named as the ntuple or as another.
    pub fn finish_with(mut self, objects: &Objects) -> Result<W> {
        let header = self.header.take().ok_or_else(|| {
            Error::InvalidInput("NtupleWriter finished with no batches written".into())
        })?;
        let num_clusters = u32::saturating_from(self.summaries.len());

        let compression = self.file.compression();
        let page_list_env = build_page_list(
            &self.summaries,
            &self.cluster_pages,
            compression,
            header.checksum,
        )?;
        let page_list_offset = self.file.place_blob(&page_list_env)?;

        let footer_env = build_footer(
            u32::saturating_from(self.total_entries),
            num_clusters,
            offset(page_list_offset)?,
            page_list_env.len(),
            header.checksum,
            None,
        );
        let seek_footer = self.file.place_blob(&footer_env)?;

        // A small (32-bit) container cannot address past 2 GiB. Fail loudly
        // rather than truncating the anchor / key-list seek pointers into a
        // corrupt file; the caller can re-run with `create_large`/`new_large`.
        let pos = self.file.position();
        if !self.file.is_big() && pos > KSTART_BIG_FILE {
            return Err(Error::FileTooLarge { size: pos });
        }

        let anchor = build_anchor(
            offset(header.seek)?,
            header.len,
            offset(seek_footer)?,
            footer_env.len(),
        );
        self.file.place_key_uncompressed(
            DirId::TOP,
            ANCHOR_CLASS,
            &self.ntuple_name,
            "",
            &anchor,
        )?;
        objects.place(&mut self.file, &self.ntuple_name)?;
        self.file
            .place_streamer_info(&header.streamer_info, objects.classes())?;
        self.file.finish()
    }
}

impl<W: Write + Seek> std::fmt::Debug for NtupleWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NtupleWriter")
            .field("name", &self.ntuple_name)
            .field("entries", &self.total_entries)
            .finish_non_exhaustive()
    }
}
