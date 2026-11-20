//! Opening an RNTuple from a ROOT file: anchor → header/footer envelopes →
//! page-list envelopes → on-demand column decoding.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{decompress_payload, find_key};
use oxiroot_io_core::{Error, FileReader, Result};

use crate::anchor::{ANCHOR_CLASS, RNTupleAnchor};
use crate::envelope::{ENVELOPE_FOOTER, ENVELOPE_HEADER, ENVELOPE_PAGELIST, read_envelope};
use std::ops::Range;

use crate::field::{self, FieldValues, FromField, Jagged};
use crate::footer::Footer;
use crate::header::{Header, StructRole};
use crate::page::{ColumnValues, read_column};
use crate::pagelist::{ClusterPages, ClusterSummary, PageInfo, PageList};

/// Reads an RNTuple from a [`FileReader`]. Opening verifies the anchor and
/// parses the schema, cluster summaries, and per-cluster page locations; column
/// data is decoded on demand.
#[doc(alias = "RNTuple", alias = "RNTupleReader")]
#[derive(Clone)]
pub struct NtupleReader<'f> {
    /// The file the pages are read from.
    file: &'f FileReader,
    /// Decompress a column's pages in parallel (a per-read view's setting).
    parallel: bool,
    anchor: RNTupleAnchor,
    header: Header,
    footer: Footer,
    summaries: Vec<ClusterSummary>,
    page_clusters: Vec<ClusterPages>,
    header_bytes: Vec<u8>,
    footer_bytes: Vec<u8>,
}

impl<'f> NtupleReader<'f> {
    /// Open the RNTuple named `name` from the file's top directory.
    pub fn open(file: &'f FileReader, name: &str) -> Result<NtupleReader<'f>> {
        let key = file.key(name).ok_or_else(|| Error::NotFound {
            what: "key",
            name: name.to_string(),
        })?;
        Self::open_from_key(file, key)
    }

    /// Open the RNTuple named `name` from the subdirectory `subdir`. The anchor's
    /// blob offsets are absolute file positions, so reading is identical to a
    /// top-directory RNTuple once the anchor key is located.
    pub fn open_in(file: &'f FileReader, subdir: &str, name: &str) -> Result<NtupleReader<'f>> {
        let dir = file.subdir(subdir)?;
        // The cycle `name` asks for, or the highest — `FileReader::key`'s rule.
        let key = find_key(&dir.keys, name).ok_or_else(|| Error::NotFound {
            what: "key",
            name: format!("{}/{name}", subdir.trim_end_matches('/')),
        })?;
        // Clone out of the borrowed `dir` so the returned RNTuple owns nothing
        // tied to it.
        let key = key.clone();
        Self::open_from_key(file, &key)
    }

    /// Read an RNTuple given its already-located anchor `key`.
    fn open_from_key(
        file: &'f FileReader,
        key: &oxiroot_io_core::raw::Key,
    ) -> Result<NtupleReader<'f>> {
        if key.class_name != ANCHOR_CLASS {
            return Err(Error::WrongClass {
                name: key.name.clone(),
                found: key.class_name.clone(),
                expected: ANCHOR_CLASS.to_string(),
            });
        }

        let anchor_payload = file.key_payload(key)?;
        let anchor_object =
            decompress_payload(&anchor_payload, key.obj_len as usize, "RNTuple anchor")?;
        let anchor = RNTupleAnchor::read(&anchor_object)?;

        let header_bytes = read_blob(
            file,
            anchor.seek_header,
            anchor.nbytes_header,
            anchor.len_header,
            "header",
        )?;
        let footer_bytes = read_blob(
            file,
            anchor.seek_footer,
            anchor.nbytes_footer,
            anchor.len_footer,
            "footer",
        )?;

        let h = read_envelope(&header_bytes)?;
        if h.type_id != ENVELOPE_HEADER {
            return Err(Error::Format(format!(
                "bad header envelope type {:#x}",
                h.type_id
            )));
        }
        let mut header = Header::parse(h.payload)?;

        let f = read_envelope(&footer_bytes)?;
        if f.type_id != ENVELOPE_FOOTER {
            return Err(Error::Format(format!(
                "bad footer envelope type {:#x}",
                f.type_id
            )));
        }
        let footer = Footer::parse(f.payload)?;

        // Merge any schema-extension fields/columns (late-added fields of a
        // schema-extended RNTuple) into the schema, continuing the header's IDs.
        header.fields.extend(footer.ext_fields.iter().cloned());
        header.columns.extend(footer.ext_columns.iter().cloned());

        // Read each cluster group's page-list envelope.
        let mut summaries = Vec::new();
        let mut page_clusters = Vec::new();
        for group in &footer.cluster_groups {
            let blob = read_blob(
                file,
                group.page_list.offset,
                u64::from(group.page_list.size),
                group.page_list_len,
                "page list",
            )?;
            let env = read_envelope(&blob)?;
            if env.type_id != ENVELOPE_PAGELIST {
                return Err(Error::Format(format!(
                    "bad page-list envelope type {:#x}",
                    env.type_id
                )));
            }
            let page_list = PageList::parse(env.payload)?;
            summaries.extend(page_list.summaries);
            page_clusters.extend(page_list.clusters);
        }

        Ok(NtupleReader {
            file,
            parallel: false,
            anchor,
            header,
            footer,
            summaries,
            page_clusters,
            header_bytes,
            footer_bytes,
        })
    }

    /// The verified anchor.
    pub fn anchor(&self) -> &RNTupleAnchor {
        &self.anchor
    }

    /// The parsed schema (fields and columns).
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The parsed footer (cluster groups).
    pub fn footer(&self) -> &Footer {
        &self.footer
    }

    /// The decompressed header envelope bytes.
    pub fn header_envelope(&self) -> &[u8] {
        &self.header_bytes
    }

    /// The decompressed footer envelope bytes.
    pub fn footer_envelope(&self) -> &[u8] {
        &self.footer_bytes
    }

    /// Total number of entries across all clusters.
    pub fn entries(&self) -> u64 {
        self.summaries.iter().map(|s| s.num_entries).sum()
    }

    /// Decode physical column `column_index` across all clusters.
    pub fn read_column(&self, column_index: usize) -> Result<ColumnValues> {
        self.column_values(self.file, column_index)
    }

    fn column_values(&self, file: &FileReader, column_index: usize) -> Result<ColumnValues> {
        let descriptor = self
            .header
            .columns
            .get(column_index)
            .ok_or_else(|| Error::NotFound {
                what: "column",
                name: column_index.to_string(),
            })?;

        let mut pages: Vec<PageInfo> = Vec::new();
        for cluster in &self.page_clusters {
            // A deferred (schema-extension) column is absent from the clusters
            // written before it existed; those contribute no pages and are
            // back-filled with defaults below.
            if let Some(column) = cluster.columns.get(column_index) {
                pages.extend_from_slice(&column.pages);
            }
        }

        let values = read_column(
            file,
            descriptor.column_type,
            descriptor.bits_on_storage,
            &pages,
            descriptor.value_range,
            self.parallel,
        )?;

        // A deferred (schema-extension) column carries a non-zero
        // `first_element_index`: its pages cover only the elements from that
        // index on, and ROOT defaults the earlier ones. Prepend that many
        // defaults so the column lines up. (A normal collection child column has
        // no such index and is legitimately shorter than the entry count.)
        match descriptor.first_element_index {
            Some(first) if first > 0 => {
                // `first` comes from the header. A deferred column cannot start
                // past every element its field could hold: cap it at a generous
                // number of elements per entry before allocating that many
                // defaults (and allocate fallibly, below).
                let limit = self.entries().max(1).saturating_mul(MAX_ELEMENTS_PER_ENTRY);
                let first = u64::try_from(first)
                    .ok()
                    .filter(|&f| f <= limit)
                    .and_then(|f| usize::try_from(f).ok())
                    .ok_or_else(|| {
                        Error::Format(format!(
                            "deferred column {column_index} starts at element {first}, \
                             beyond {limit} for {} entries",
                            self.entries()
                        ))
                    })?;
                backfill_leading(values, first)
            }
            _ => Ok(values),
        }
    }

    /// Names of the top-level fields, in schema order.
    pub fn field_names(&self) -> Vec<&str> {
        self.header
            .fields
            .iter()
            .enumerate()
            .filter(|(i, f)| f.parent_field_id as usize == *i)
            .map(|(_, f)| f.name.as_str())
            .collect()
    }

    /// Read the top-level field `name`: set a [`range`](FieldRead::range) of
    /// entries on the returned [`FieldRead`], then take the values typed
    /// ([`read`](FieldRead::read)) or as the [`FieldValues`] enum
    /// ([`values`](FieldRead::values)).
    ///
    /// ```no_run
    /// # use oxiroot_io_core::FileReader;
    /// # use oxiroot_rntuple::NtupleReader;
    /// let file = FileReader::open("events.root")?;
    /// let ntpl = NtupleReader::open(&file, "events")?;
    /// let mass: Vec<f64> = ntpl.field("mass").range(0..1000).read()?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    pub fn field<'a>(&'a self, name: &'a str) -> FieldRead<'a, 'f> {
        FieldRead {
            ntuple: self,
            name,
            range: None,
            parallel: false,
        }
    }

    /// The entry numbers at which the clusters start, followed by the entry
    /// count: cluster `i` holds entries `[b[i], b[i + 1])`. Reading a window
    /// that starts and ends on these decompresses no page twice.
    #[must_use]
    pub fn cluster_boundaries(&self) -> Vec<u64> {
        let mut bounds: Vec<u64> = self.summaries.iter().map(|s| s.first_entry).collect();
        bounds.push(self.entries());
        bounds.dedup();
        bounds
    }

    fn read_top(&self, file: &FileReader, name: &str) -> Result<FieldValues> {
        let idx = self
            .header
            .fields
            .iter()
            .enumerate()
            .position(|(i, f)| f.name == name && f.parent_field_id as usize == i)
            .ok_or_else(|| Error::NotFound {
                what: "top-level field",
                name: name.to_string(),
            })?;

        let values = self.read_field_tree(file, idx)?;
        // A top-level field carries exactly one element per entry; a mismatch
        // means a truncated or corrupt index/leaf column.
        if values.len() as u64 != self.entries() {
            return Err(Error::Format(format!(
                "field {name:?}: decoded {} entries, expected {}",
                values.len(),
                self.entries()
            )));
        }
        Ok(values)
    }

    /// The top-level field `name` if it is a `std::vector` of a scalar type:
    /// its offsets and its element column, decoded straight from the two
    /// columns with no per-entry allocation. `Ok(None)` for any other shape.
    fn read_flat_top(&self, file: &FileReader, name: &str) -> Result<Option<Jagged>> {
        let Some(idx) = self.top_field(name) else {
            return Err(Error::NotFound {
                what: "top-level field",
                name: name.to_string(),
            });
        };
        let fld = &self.header.fields[idx];
        if fld.struct_role != StructRole::Collection
            || fld.array_size.is_some()
            || is_late_field(&fld.type_name)
        {
            return Ok(None);
        }
        let children = self.child_fields(idx);
        let [child] = children[..] else {
            return Ok(None);
        };
        let cfld = &self.header.fields[child];
        let child_cols = self.field_columns(child);
        if cfld.struct_role != StructRole::Leaf
            || cfld.type_name == "std::string"
            || cfld.array_size.is_some()
            || child_cols.len() != 1
        {
            return Ok(None);
        }
        let Some(index_ci) = self.index_column(&self.field_columns(idx)) else {
            return Ok(None);
        };
        let mut offsets = self.read_offsets(file, index_ci)?;
        offsets.insert(0, 0);
        let values = field::scalar(self.column_values(file, child_cols[0])?)?;
        let monotonic = offsets.windows(2).all(|w| w[0] <= w[1]);
        if !monotonic || offsets.last().copied() != Some(values.len() as u64) {
            return Err(Error::Format(format!(
                "field {name:?}: collection offsets do not partition its {} elements",
                values.len()
            )));
        }
        if offsets.len() as u64 != self.entries() + 1 {
            return Err(Error::Format(format!(
                "field {name:?}: decoded {} entries, expected {}",
                offsets.len() - 1,
                self.entries()
            )));
        }
        Ok(Some(Jagged { offsets, values }))
    }

    fn top_field(&self, name: &str) -> Option<usize> {
        self.header
            .fields
            .iter()
            .enumerate()
            .position(|(i, f)| f.name == name && f.parent_field_id as usize == i)
    }

    /// Entries `[start, stop)` (clamped) of `name` in flat form, reading only
    /// the clusters the window overlaps.
    fn read_flat_range(&self, name: &str, start: u64, stop: u64) -> Result<Option<Jagged>> {
        let stop = stop.min(self.entries());
        let start = start.min(stop);
        if start == 0 && stop == self.entries() {
            return self.read_flat_top(self.file, name);
        }
        let deferred = self
            .header
            .columns
            .iter()
            .any(|c| c.first_element_index.is_some_and(|f| f > 0));
        let first = self
            .summaries
            .iter()
            .position(|s| s.first_entry + s.num_entries > start);
        let last = self.summaries.iter().rposition(|s| s.first_entry < stop);
        let (jagged, from) = match (deferred, first, last) {
            (false, Some(k0), Some(k1)) if k0 <= k1 => (
                self.cluster_window(k0, k1 + 1)
                    .read_flat_top(self.file, name)?,
                self.summaries[k0].first_entry,
            ),
            _ => (self.read_flat_top(self.file, name)?, 0),
        };
        let Some(Jagged {
            offsets,
            mut values,
        }) = jagged
        else {
            return Ok(None);
        };
        let a = usize::saturating_from(start - from);
        let b = usize::saturating_from(stop - from).min(offsets.len() - 1);
        let lo = offsets[a];
        let window: Vec<u64> = offsets[a..=b].iter().map(|&o| o - lo).collect();
        values.skip(usize::saturating_from(lo));
        values.truncate(usize::saturating_from(offsets[b] - lo));
        Ok(Some(Jagged {
            offsets: window,
            values,
        }))
    }

    /// Entries `[start, stop)` (clamped) of field `name`, reading only the
    /// clusters the window overlaps.
    fn read_range(&self, name: &str, start: u64, stop: u64) -> Result<FieldValues> {
        let stop = stop.min(self.entries());
        let start = start.min(stop);
        if start == 0 && stop == self.entries() {
            return self.read_top(self.file, name);
        }
        // A deferred (schema-extension) column back-fills from its first
        // element index, which a window not starting at the first cluster
        // would misplace: read whole, then cut.
        let deferred = self
            .header
            .columns
            .iter()
            .any(|c| c.first_element_index.is_some_and(|f| f > 0));
        let first = self
            .summaries
            .iter()
            .position(|s| s.first_entry + s.num_entries > start);
        let last = self.summaries.iter().rposition(|s| s.first_entry < stop);
        let (mut values, from) = match (deferred, first, last) {
            (false, Some(k0), Some(k1)) if k0 <= k1 => (
                self.cluster_window(k0, k1 + 1).read_top(self.file, name)?,
                self.summaries[k0].first_entry,
            ),
            _ => (self.read_top(self.file, name)?, 0),
        };
        values.skip(usize::saturating_from(start - from));
        values.truncate(usize::saturating_from(stop - start));
        Ok(values)
    }

    /// A lightweight view of this RNTuple limited to clusters `[k0, k1)`: the
    /// same schema, but `entries` and every column read cover only those
    /// clusters. Cloning the schema is cheap next to decoding pages.
    fn cluster_window(&self, k0: usize, k1: usize) -> NtupleReader<'f> {
        NtupleReader {
            file: self.file,
            parallel: self.parallel,
            anchor: self.anchor.clone(),
            header: self.header.clone(),
            footer: self.footer.clone(),
            summaries: self.summaries[k0..k1].to_vec(),
            page_clusters: self.page_clusters[k0..k1].to_vec(),
            header_bytes: self.header_bytes.clone(),
            footer_bytes: self.footer_bytes.clone(),
        }
    }

    /// Recursively reconstruct the values of field `field_idx` (a leaf, string,
    /// collection, or record) as flattened-at-this-level [`FieldValues`].
    fn read_field_tree(&self, file: &FileReader, field_idx: usize) -> Result<FieldValues> {
        let fld = self
            .header
            .fields
            .get(field_idx)
            .ok_or_else(|| Error::Format(format!("no field {field_idx}")))?;
        let columns = self.field_columns(field_idx);

        // A fixed-size array field (`std::array<T, n>` / `std::bitset<n>`) carries
        // `n` elements per entry. The values come from its own column (e.g. a
        // bitset's Bit column) or from its single element child (e.g. an array's
        // `_0`); group them into one chunk of `n` per entry.
        if let Some(rep) = fld.array_size {
            let flat = if let Some(&ci) = columns.first() {
                field::scalar(self.column_values(file, ci)?)?
            } else {
                let child = *self.child_fields(field_idx).first().ok_or_else(|| {
                    Error::Format(format!(
                        "array field {:?} has no element column or child",
                        fld.name
                    ))
                })?;
                self.read_field_tree(file, child)?
            };
            return field::chunk(flat, usize::saturating_from(rep));
        }

        match fld.struct_role {
            StructRole::Leaf if fld.type_name == "std::string" => {
                let index_ci = self.index_column(&columns).ok_or_else(|| {
                    Error::Format(format!("string field {:?} has no index column", fld.name))
                })?;
                let char_ci = *columns
                    .iter()
                    .find(|&&ci| !self.header.columns[ci].column_type.is_index())
                    .ok_or_else(|| {
                        Error::Format(format!("string field {:?} has no char column", fld.name))
                    })?;
                let offsets = self.read_offsets(file, index_ci)?;
                let bytes = match self.column_values(file, char_ci)? {
                    ColumnValues::Bytes(v) => v,
                    other => {
                        return Err(Error::Format(format!("string chars decoded as {other:?}")));
                    }
                };
                field::strings(&offsets, &bytes)
            }
            StructRole::Leaf => {
                // A `std::atomic<T>` (and similar transparent wrappers) is a Leaf
                // with no column of its own that delegates to a single child leaf.
                if columns.is_empty()
                    && let Some(&child) = self.child_fields(field_idx).first()
                {
                    return self.read_field_tree(file, child);
                }
                let ci = *columns
                    .first()
                    .ok_or_else(|| Error::Format(format!("field {:?} has no column", fld.name)))?;
                field::scalar(self.column_values(file, ci)?)
            }
            StructRole::Collection => {
                let index_ci = self.index_column(&columns).ok_or_else(|| {
                    Error::Format(format!(
                        "collection field {:?} has no index column",
                        fld.name
                    ))
                })?;
                let offsets = self.read_offsets(file, index_ci)?;
                let child = *self.child_fields(field_idx).first().ok_or_else(|| {
                    Error::Format(format!(
                        "collection field {:?} has no element field",
                        fld.name
                    ))
                })?;
                let items = self.read_field_tree(file, child)?;
                // `std::optional<T>` / `std::unique_ptr<T>` use the collection
                // shape (0 or 1 element per entry) but read back as a nullable.
                if is_late_field(&fld.type_name) {
                    return field::optional(&offsets, items);
                }
                field::collect(offsets, items)
            }
            StructRole::Record => {
                let children = self.child_fields(field_idx);
                if children.is_empty() {
                    return Err(Error::Format(format!(
                        "record field {:?} has no sub-fields",
                        fld.name
                    )));
                }
                let mut out = Vec::with_capacity(children.len());
                for c in children {
                    let sub_name = self.header.fields[c].name.clone();
                    out.push((sub_name, self.read_field_tree(file, c)?));
                }
                Ok(FieldValues::Record(out))
            }
            StructRole::Streamer => {
                // An unsplit class blob: a per-entry Index column over a single
                // Byte column. Interpret each blob with the class TStreamerInfo.
                let index_ci = self.index_column(&columns).ok_or_else(|| {
                    Error::Format(format!("streamer field {:?} has no index column", fld.name))
                })?;
                let byte_ci = *columns
                    .iter()
                    .find(|&&ci| !self.header.columns[ci].column_type.is_index())
                    .ok_or_else(|| {
                        Error::Format(format!("streamer field {:?} has no byte column", fld.name))
                    })?;
                let offsets = self.read_offsets(file, index_ci)?;
                let bytes = match self.column_values(file, byte_ci)? {
                    ColumnValues::Bytes(v) => v,
                    other => {
                        return Err(Error::Format(format!(
                            "streamer field bytes decoded as {other:?}"
                        )));
                    }
                };
                let registry = file.streamer_registry()?;
                // The field's type version is the class version it was written at.
                let version = i32::try_from(fld.type_version).unwrap_or(i32::MAX);
                let info = registry.get_at(&fld.type_name, version).ok_or_else(|| {
                    Error::MissingStreamerInfo {
                        class: fld.type_name.clone(),
                    }
                })?;
                crate::streamer::decode(info, &offsets, &bytes)
            }
            StructRole::Variant => {
                // The variant field carries one Switch column of (index, tag);
                // each sub-field is one densely-packed alternative.
                let switch_ci = *columns.first().ok_or_else(|| {
                    Error::Format(format!("variant field {:?} has no switch column", fld.name))
                })?;
                let (tags, indices) = match self.column_values(file, switch_ci)? {
                    ColumnValues::Switch(v) => (
                        v.iter().map(|&(_, t)| t).collect::<Vec<_>>(),
                        v.iter().map(|&(i, _)| i).collect::<Vec<_>>(),
                    ),
                    other => {
                        return Err(Error::Format(format!(
                            "variant switch column decoded as {other:?}"
                        )));
                    }
                };
                let children = self.child_fields(field_idx);
                if children.is_empty() {
                    return Err(Error::Format(format!(
                        "variant field {:?} has no alternatives",
                        fld.name
                    )));
                }
                let mut alternatives = Vec::with_capacity(children.len());
                for c in children {
                    let sub_name = self.header.fields[c].name.clone();
                    alternatives.push((sub_name, self.read_field_tree(file, c)?));
                }
                Ok(FieldValues::Variant {
                    alternatives,
                    tags,
                    indices,
                })
            }
            other => Err(Error::Unsupported(format!(
                "field role {other:?} is not supported"
            ))),
        }
    }

    /// Column indices belonging to field `field_idx`, in column order.
    fn field_columns(&self, field_idx: usize) -> Vec<usize> {
        self.header
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.field_id as usize == field_idx)
            .map(|(ci, _)| ci)
            .collect()
    }

    /// Sub-field ids of `field_idx` (children whose parent is `field_idx`),
    /// excluding the field itself (a top-level field is its own parent), in
    /// declaration order.
    fn child_fields(&self, field_idx: usize) -> Vec<usize> {
        self.header
            .fields
            .iter()
            .enumerate()
            .filter(|(ci, f)| *ci != field_idx && f.parent_field_id as usize == field_idx)
            .map(|(ci, _)| ci)
            .collect()
    }

    fn index_column(&self, columns: &[usize]) -> Option<usize> {
        columns
            .iter()
            .copied()
            .find(|&ci| self.header.columns[ci].column_type.is_index())
    }

    /// Read a collection/string index column as globally cumulative element
    /// offsets. Index offsets are stored relative to each cluster, so we decode
    /// the column one cluster at a time and shift each cluster's values by the
    /// number of elements in all preceding clusters. (This is a no-op for a
    /// single cluster, and matches a flat decode for delta-encoded indices.)
    ///
    /// The decoded count is one offset per element of the *enclosing* level —
    /// the entry count for a top-level collection, but the parent-element count
    /// for a nested one — so it is validated by the caller, not here.
    fn read_offsets(&self, file: &FileReader, column_index: usize) -> Result<Vec<u64>> {
        let descriptor = self
            .header
            .columns
            .get(column_index)
            .ok_or_else(|| Error::Format(format!("no column {column_index}")))?;

        // Each cluster's offsets decode on their own (an index column's delta
        // encoding restarts per cluster), concurrently when reading in
        // parallel; then each is rebased onto the running total. Offsets are
        // decoded from the file; use wrapping arithmetic (as delta_decode
        // does) so a corrupt index column cannot panic in debug.
        let decode = |cluster: &ClusterPages| -> Result<Vec<u64>> {
            let column = cluster
                .columns
                .get(column_index)
                .ok_or_else(|| Error::Format(format!("cluster missing column {column_index}")))?;
            match read_column(
                file,
                descriptor.column_type,
                descriptor.bits_on_storage,
                &column.pages,
                descriptor.value_range,
                false,
            )? {
                ColumnValues::U64(v) => Ok(v),
                other => Err(Error::Format(format!(
                    "expected index offsets, got {other:?}"
                ))),
            }
        };
        #[cfg(feature = "rayon")]
        let clusters: Vec<Vec<u64>> = if self.parallel {
            use rayon::prelude::*;
            self.page_clusters
                .par_iter()
                .map(decode)
                .collect::<Result<_>>()?
        } else {
            self.page_clusters
                .iter()
                .map(decode)
                .collect::<Result<_>>()?
        };
        #[cfg(not(feature = "rayon"))]
        let clusters: Vec<Vec<u64>> = self
            .page_clusters
            .iter()
            .map(decode)
            .collect::<Result<_>>()?;
        let mut out = Vec::with_capacity(clusters.iter().map(Vec::len).sum());
        let mut base = 0u64;
        for local in clusters {
            let cluster_total = local.last().copied().unwrap_or(0);
            out.extend(local.into_iter().map(|v| v.wrapping_add(base)));
            base = base.wrapping_add(cluster_total);
        }
        Ok(out)
    }
}

/// Read and decompress an RBlob (header/footer/page list) at `seek`.
/// Whether `type_name` is a nullable / "late" field (`std::optional<T>` or
/// `std::unique_ptr<T>`) — read back as [`FieldValues::Opt`], not a collection.
fn is_late_field(type_name: &str) -> bool {
    type_name.starts_with("std::optional<") || type_name.starts_with("std::unique_ptr<")
}

/// Prepend `k` default values to a decoded column — the leading entries a
/// late-added (schema-extension) column does not cover. Numerics default to `0`
/// and bools to `false` (an `Index*` offset of `0` is an empty collection),
/// matching how ROOT defaults entries written before the field was added.
fn backfill_leading(values: ColumnValues, k: usize) -> Result<ColumnValues> {
    use ColumnValues::*;
    macro_rules! pre {
        ($variant:ident, $v:ident, $default:expr_2021) => {{
            let mut out = Vec::new();
            out.try_reserve_exact(k.saturating_add($v.len()))
                .map_err(|_| {
                    Error::Format(format!(
                        "cannot allocate {k} leading defaults for a deferred column"
                    ))
                })?;
            out.resize(k, $default);
            out.extend($v);
            $variant(out)
        }};
    }
    Ok(match values {
        Bits(v) => pre!(Bits, v, false),
        Bytes(v) => pre!(Bytes, v, 0),
        I8(v) => pre!(I8, v, 0),
        U8(v) => pre!(U8, v, 0),
        I16(v) => pre!(I16, v, 0),
        U16(v) => pre!(U16, v, 0),
        I32(v) => pre!(I32, v, 0),
        I64(v) => pre!(I64, v, 0),
        U32(v) => pre!(U32, v, 0),
        U64(v) => pre!(U64, v, 0),
        F32(v) => pre!(F32, v, 0.0),
        F64(v) => pre!(F64, v, 0.0),
        Switch(v) => pre!(Switch, v, (0u64, 0u32)),
    })
}

/// The most elements per entry a deferred (schema-extension) column may claim
/// to start after. Generous for nested collections; it exists to reject a
/// forged `first_element_index` before allocating defaults for it.
const MAX_ELEMENTS_PER_ENTRY: u64 = 1 << 16;

fn read_blob(file: &FileReader, seek: u64, nbytes: u64, len: u64, what: &str) -> Result<Vec<u8>> {
    let win = file
        .read_at(seek, usize::saturating_from(nbytes))
        .map_err(|_| Error::Format(format!("{what} blob at {seek} runs past end of file")))?;
    decompress_payload(&win, usize::saturating_from(len), what)
}

/// One field read, set up by [`NtupleReader::field`]: the entries to read,
/// then the form to read them in.
#[must_use = "a FieldRead does nothing until read or values is called"]
pub struct FieldRead<'a, 'f> {
    ntuple: &'a NtupleReader<'f>,
    name: &'a str,
    range: Option<Range<u64>>,
    parallel: bool,
}

impl<'a, 'f> FieldRead<'a, 'f> {
    /// Read only entries `range` (clamped to the entry count: a window past
    /// the end yields fewer entries, not an error). Only the clusters the window
    /// touches are fetched and decompressed.
    pub fn range(mut self, range: Range<u64>) -> Self {
        self.range = Some(range);
        self
    }

    /// Decompress each column's pages in parallel, on rayon's global thread
    /// pool.
    #[cfg(feature = "rayon")]
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// The reader to read through: this one, or a parallel view of it.
    fn reader(&self) -> std::borrow::Cow<'a, NtupleReader<'f>> {
        if self.parallel {
            let mut view = self.ntuple.cluster_window(0, self.ntuple.summaries.len());
            view.parallel = true;
            std::borrow::Cow::Owned(view)
        } else {
            std::borrow::Cow::Borrowed(self.ntuple)
        }
    }

    /// The values as `Vec<T>`: `f64` for a `double` field, `Vec<f32>` for a
    /// `std::vector<float>`, `String` for a `std::string`, `Option<T>` for a
    /// `std::optional<T>`. Asking for another type than the field holds is an
    /// [`Error::TypeMismatch`]; nothing is converted.
    pub fn read<T: FromField>(self) -> Result<Vec<T>> {
        let name = self.name;
        T::from_values(name, self.values()?)
    }

    /// The values as the [`FieldValues`] enum, for code that does not know the
    /// field's type in advance.
    pub fn values(self) -> Result<FieldValues> {
        let reader = self.reader();
        match self.range {
            None => reader.read_top(reader.file, self.name),
            Some(r) => reader.read_range(self.name, r.start, r.end),
        }
    }

    /// A `std::vector<T>` field flat: one buffer of every element plus
    /// per-entry offsets, decoded straight from the field's two columns with
    /// no allocation per entry. Only for `std::vector` of a scalar type; other
    /// fields are an [`Error::Unsupported`].
    pub fn flat(self) -> Result<Jagged> {
        let r = self.range.clone().unwrap_or(0..u64::MAX);
        self.reader()
            .read_flat_range(self.name, r.start, r.end)?
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "field {:?} is not a std::vector of a scalar type; use values()",
                    self.name
                ))
            })
    }

    pub(crate) fn name(&self) -> &'a str {
        self.name
    }
}

impl std::fmt::Debug for NtupleReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NtupleReader")
            .field("clusters", &self.summaries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leading defaults for a deferred column are allocated fallibly: an
    /// impossible count is an error, not an abort.
    #[test]
    fn backfill_of_an_impossible_length_is_an_error() {
        let huge = usize::MAX / 2;
        assert!(backfill_leading(ColumnValues::U64(vec![1]), huge).is_err());
        let ok = backfill_leading(ColumnValues::U64(vec![7]), 2).unwrap();
        assert!(matches!(ok, ColumnValues::U64(v) if v == vec![0, 0, 7]));
    }
}
