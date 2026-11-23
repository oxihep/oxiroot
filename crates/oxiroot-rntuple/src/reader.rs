//! Opening an `RNTuple` from a ROOT file: anchor → header/footer envelopes →
//! page-list envelopes → on-demand column decoding.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::decompress_payload;
use oxiroot_io_core::{Column, Error, FileReader, Result};

use crate::anchor::{ANCHOR_CLASS, RNTupleAnchor};
use crate::envelope::{ENVELOPE_FOOTER, ENVELOPE_HEADER, ENVELOPE_PAGELIST, read_envelope};
use std::ops::Range;
use std::sync::Arc;

use crate::field;
use crate::footer::Footer;
use crate::header::{Header, StructRole};
use crate::page::{ColumnValues, read_column};
use crate::pagelist::{ClusterPages, ClusterSummary, PageInfo, PageList};
use oxiroot_io_core::column::{FromColumn, typed};

/// Reads an `RNTuple` from a [`FileReader`]. Opening verifies the anchor and
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
    /// The schema and the page lists, shared: a view of some of the clusters,
    /// or one that reads in parallel, costs a few reference counts.
    header: Arc<Header>,
    footer: Arc<Footer>,
    all_summaries: Arc<[ClusterSummary]>,
    all_page_clusters: Arc<[ClusterPages]>,
    header_bytes: Arc<[u8]>,
    footer_bytes: Arc<[u8]>,
    /// The clusters this reader covers: all of them, or a window of entries.
    clusters: Range<usize>,
    /// For each field, the columns it owns and the fields inside it, in
    /// order: looked up for every field read, so built once.
    columns_of: Arc<[Vec<usize>]>,
    children_of: Arc<[Vec<usize>]>,
}

impl<'f> NtupleReader<'f> {
    /// Open the `RNTuple` at `path` in `file`: `"events"`, or
    /// `"dir/sub/events;cycle"` for one in a subdirectory or at a given cycle.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if `path` names no key, [`Error::WrongClass`] if the key is not an
    /// `RNTuple`, and any error reading its header and footer.
    pub fn open(file: &'f FileReader, path: &str) -> Result<Self> {
        Self::open_from_key(file, &file.key(path)?)
    }

    /// Read an `RNTuple` given its already-located anchor `key`.
    fn open_from_key(file: &'f FileReader, key: &oxiroot_io_core::raw::Key) -> Result<Self> {
        if key.class_name != ANCHOR_CLASS {
            return Err(Error::WrongClass {
                name: Some(key.name.clone()),
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

        let mut columns_of = vec![Vec::new(); header.fields.len()];
        for (ci, column) in header.columns.iter().enumerate() {
            if let Some(owned) = columns_of.get_mut(column.field()) {
                owned.push(ci);
            }
        }
        let mut children_of = vec![Vec::new(); header.fields.len()];
        for (fi, field) in header.fields.iter().enumerate() {
            // A top-level field is its own parent, and not its own child.
            let parent = field.parent();
            if parent != fi
                && let Some(children) = children_of.get_mut(parent)
            {
                children.push(fi);
            }
        }
        Ok(NtupleReader {
            file,
            parallel: false,
            anchor,
            header: Arc::new(header),
            footer: Arc::new(footer),
            clusters: 0..summaries.len(),
            all_summaries: summaries.into(),
            all_page_clusters: page_clusters.into(),
            header_bytes: header_bytes.into(),
            footer_bytes: footer_bytes.into(),
            columns_of: columns_of.into(),
            children_of: children_of.into(),
        })
    }

    /// The verified anchor.
    #[must_use]
    pub fn anchor(&self) -> &RNTupleAnchor {
        &self.anchor
    }

    /// The parsed schema (fields and columns).
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The parsed footer (cluster groups).
    #[must_use]
    pub fn footer(&self) -> &Footer {
        &self.footer
    }

    /// The decompressed header envelope bytes.
    #[must_use]
    pub fn header_envelope(&self) -> &[u8] {
        &self.header_bytes
    }

    /// The decompressed footer envelope bytes.
    #[must_use]
    pub fn footer_envelope(&self) -> &[u8] {
        &self.footer_bytes
    }

    /// Total number of entries across all clusters.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.summaries().iter().map(|s| s.num_entries).sum()
    }

    /// Decode physical column `column_index` across all clusters.
    ///
    /// # Errors
    ///
    /// If `column_index` names no column, or its pages cannot be read or decoded.
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
        for cluster in self.page_clusters() {
            // A deferred (schema-extension) column is absent from the clusters
            // written before it existed; those contribute no pages and are
            // back-filled with defaults below.
            if let Some(column) = cluster.columns.get(column_index) {
                pages.extend_from_slice(&column.pages);
            }
        }

        let values = read_column(file, descriptor, &pages, self.parallel)?;

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
    pub fn field_names(&self) -> impl Iterator<Item = &str> + Clone {
        self.header.top_level_fields().map(|(_, f)| f.name.as_str())
    }

    /// Read the top-level field `name`: set a [`range`](FieldRead::range) of
    /// entries on the returned [`FieldRead`], then take the values typed
    /// ([`read`](FieldRead::read)) or as a [`Column`]
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
        let mut bounds: Vec<u64> = self.summaries().iter().map(|s| s.first_entry).collect();
        bounds.push(self.entries());
        bounds.dedup();
        bounds
    }

    fn read_top(&self, file: &FileReader, name: &str) -> Result<Column> {
        let idx = self
            .header
            .top_level_field(name)
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

    /// Entries `[start, stop)` (clamped) of field `name`, reading only the
    /// clusters the window overlaps.
    fn read_range(&self, name: &str, start: u64, stop: u64) -> Result<Column> {
        let stop = stop.min(self.entries());
        let start = start.min(stop);
        if start == 0 && stop == self.entries() {
            return self.read_top(self.file, name);
        }
        let (values, from) = match self.entry_window(start, stop) {
            Some((view, from)) => (view.read_top(self.file, name)?, from),
            None => (self.read_top(self.file, name)?, 0),
        };
        Ok(values.slice(
            usize::saturating_from(start - from),
            usize::saturating_from(stop - from),
        ))
    }

    /// A lightweight view of this `RNTuple` limited to clusters `[k0, k1)`: the
    /// same schema, but `entries` and every column read cover only those
    /// clusters. Cloning the schema is cheap next to decoding pages.
    fn cluster_window(&self, k0: usize, k1: usize) -> Self {
        let base = self.clusters.start;
        Self {
            clusters: base + k0..(base + k1).min(self.clusters.end),
            ..self.clone()
        }
    }

    /// The summaries of the clusters this reader covers.
    fn summaries(&self) -> &[ClusterSummary] {
        self.all_summaries.get(self.clusters.clone()).unwrap_or(&[])
    }

    /// The page lists of the clusters this reader covers.
    fn page_clusters(&self) -> &[ClusterPages] {
        self.all_page_clusters
            .get(self.clusters.clone())
            .unwrap_or(&[])
    }

    /// The clusters that hold entries `[start, stop)`, as a view of this
    /// reader, and the first entry that view holds; or `None` when the window
    /// has to be cut from a read of everything.
    ///
    /// A deferred (schema-extension) column back-fills from its first element
    /// index, which a view not starting at the first cluster would misplace.
    fn entry_window(&self, start: u64, stop: u64) -> Option<(Self, u64)> {
        let deferred = self
            .header
            .columns
            .iter()
            .any(|c| c.first_element_index.is_some_and(|f| f > 0));
        if deferred {
            return None;
        }
        let summaries = self.summaries();
        let k0 = summaries
            .iter()
            .position(|s| s.first_entry + s.num_entries > start)?;
        let k1 = summaries.iter().rposition(|s| s.first_entry < stop)?;
        (k0 <= k1).then(|| (self.cluster_window(k0, k1 + 1), summaries[k0].first_entry))
    }

    /// Recursively reconstruct the values of field `field_idx` (a leaf, string,
    /// collection, or record) as a [`Column`], one value per element at this
    /// level.
    fn read_field_tree(&self, file: &FileReader, field_idx: usize) -> Result<Column> {
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
                let index_ci = self.index_column(columns).ok_or_else(|| {
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
                field::strings(offsets, bytes)
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
                let index_ci = self.index_column(columns).ok_or_else(|| {
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
                    return field::optional(&offsets[1..], items);
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
                for &c in children {
                    let sub_name = self.header.fields[c].name.clone();
                    out.push((sub_name, self.read_field_tree(file, c)?));
                }
                Ok(Column::Struct(out))
            }
            StructRole::Streamer => {
                // An unsplit class blob: a per-entry Index column over a single
                // Byte column. Interpret each blob with the class TStreamerInfo.
                let index_ci = self.index_column(columns).ok_or_else(|| {
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
                crate::streamer::decode(info, &offsets[1..], &bytes)
            }
            StructRole::Variant => {
                // The variant field carries one Switch column of (index, tag);
                // each sub-field is one densely-packed alternative.
                let switch_ci = *columns.first().ok_or_else(|| {
                    Error::Format(format!("variant field {:?} has no switch column", fld.name))
                })?;
                let (mut indices, tags): (Vec<_>, Vec<_>) =
                    match self.column_values(file, switch_ci)? {
                        ColumnValues::Switch(v) => v.into_iter().unzip(),
                        other => {
                            return Err(Error::Format(format!(
                                "variant switch column decoded as {other:?}"
                            )));
                        }
                    };
                let children = self.child_fields(field_idx);
                self.rebase_switch(switch_ci, children, &tags, &mut indices);
                if children.is_empty() {
                    return Err(Error::Format(format!(
                        "variant field {:?} has no alternatives",
                        fld.name
                    )));
                }
                let mut alternatives = Vec::with_capacity(children.len());
                for &c in children {
                    let sub_name = self.header.fields[c].name.clone();
                    alternatives.push((sub_name, self.read_field_tree(file, c)?));
                }
                Ok(Column::Union {
                    tags,
                    indices,
                    alternatives,
                })
            }
            other => Err(Error::Unsupported(format!(
                "field role {other:?} is not supported"
            ))),
        }
    }

    /// Make a variant's switch indices count from the start of the read, not
    /// of each cluster. A switch entry's index counts the values of its
    /// alternative within its cluster, and the alternatives are read whole, so
    /// an index in a later cluster moves past the values that alternative holds
    /// in the earlier ones. Those come from the alternative's own pages: ROOT
    /// stores one value per entry that chose it, and files oxiroot 0.10 and
    /// 0.11 wrote could store more.
    fn rebase_switch(
        &self,
        switch_ci: usize,
        children: &[usize],
        tags: &[u32],
        indices: &mut [u64],
    ) {
        let held_per_cluster: Vec<Option<Vec<u64>>> = children
            .iter()
            .map(|&c| self.elements_per_cluster(c))
            .collect();
        let mut held = vec![0u64; children.len()];
        let mut at = 0usize;
        for (cluster_no, cluster) in self.page_clusters().iter().enumerate() {
            let n: usize = cluster
                .columns
                .get(switch_ci)
                .map_or(0, |c| c.pages.iter().map(|p| p.num_elements as usize).sum());
            let end = at.saturating_add(n).min(tags.len());
            let mut chosen = vec![0u64; children.len()];
            for (&tag, index) in tags[at..end].iter().zip(&mut indices[at..end]) {
                if let Some(k) = (tag as usize)
                    .checked_sub(1)
                    .filter(|&k| k < children.len())
                {
                    *index = index.saturating_add(held[k]);
                    chosen[k] += 1;
                }
            }
            for ((h, counts), chosen) in held.iter_mut().zip(&held_per_cluster).zip(chosen) {
                *h += counts
                    .as_ref()
                    .and_then(|c| c.get(cluster_no).copied())
                    .unwrap_or(chosen);
            }
            at = end;
        }
    }

    /// How many values field `field_idx` holds in each cluster this reader
    /// covers: the elements of its first (principal) column, or for a record or
    /// fixed array without columns, its first sub-field's, per array element.
    fn elements_per_cluster(&self, field_idx: usize) -> Option<Vec<u64>> {
        if let Some(&ci) = self.field_columns(field_idx).first() {
            return Some(
                self.page_clusters()
                    .iter()
                    .map(|cluster| {
                        cluster.columns.get(ci).map_or(0, |c| {
                            c.pages.iter().map(|p| u64::from(p.num_elements)).sum()
                        })
                    })
                    .collect(),
            );
        }
        let child = *self.child_fields(field_idx).first()?;
        let counts = self.elements_per_cluster(child)?;
        Some(match self.header.fields.get(field_idx)?.array_size {
            Some(n) if n > 0 => counts.into_iter().map(|c| c / n).collect(),
            _ => counts,
        })
    }

    /// Column indices belonging to field `field_idx`, in column order.
    fn field_columns(&self, field_idx: usize) -> &[usize] {
        self.columns_of.get(field_idx).map_or(&[], Vec::as_slice)
    }

    /// Sub-field ids of `field_idx` (children whose parent is `field_idx`),
    /// excluding the field itself (a top-level field is its own parent), in
    /// declaration order.
    fn child_fields(&self, field_idx: usize) -> &[usize] {
        self.children_of.get(field_idx).map_or(&[], Vec::as_slice)
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
    ///
    /// The result leads with a `0`, as a [`Column`]'s offsets do: one value
    /// more than the level has elements.
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
            match read_column(file, descriptor, &column.pages, false)? {
                ColumnValues::U64(v) => Ok(v),
                other => Err(Error::Format(format!(
                    "expected index offsets, got {other:?}"
                ))),
            }
        };
        #[cfg(feature = "rayon")]
        let clusters: Vec<Vec<u64>> = if self.parallel {
            use rayon::prelude::*;
            self.page_clusters()
                .par_iter()
                .map(decode)
                .collect::<Result<_>>()?
        } else {
            self.page_clusters()
                .iter()
                .map(decode)
                .collect::<Result<_>>()?
        };
        #[cfg(not(feature = "rayon"))]
        let clusters: Vec<Vec<u64>> = self
            .page_clusters()
            .iter()
            .map(decode)
            .collect::<Result<_>>()?;
        let total: usize = clusters.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(total + 1);
        out.push(0);
        let mut base = 0u64;
        for local in clusters {
            let cluster_total = local.last().copied().unwrap_or(0);
            out.extend(local.into_iter().map(|v| v.wrapping_add(base)));
            base = base.wrapping_add(cluster_total);
        }
        Ok(out)
    }
}

/// Whether `type_name` is a nullable / "late" field (`std::optional<T>` or
/// `std::unique_ptr<T>`) — read back as an optional column, not a list.
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
        ($variant:ident, $v:ident, $default:expr) => {{
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

/// Read and decompress an `RBlob` (header/footer/page list) at `seek`.
fn read_blob(file: &FileReader, seek: u64, nbytes: u64, len: u64, what: &str) -> Result<Vec<u8>> {
    let win = file
        .read_at(seek, usize::saturating_from(nbytes))
        .map_err(|e| match e {
            // The descriptor points outside the file: a format error. Anything
            // else (a failed read, a dropped connection) is reported as it is.
            Error::UnexpectedEof { .. } => {
                Error::Format(format!("{what} blob at {seek} runs past end of file"))
            }
            other => other,
        })?;
    decompress_payload(&win, usize::saturating_from(len), what).map(std::borrow::Cow::into_owned)
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
    /// pool. Without the `rayon` feature there is no pool, and the read
    /// stays on the calling thread whatever is asked.
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// The reader to read through: this one, or a parallel view of it.
    fn reader(&self) -> std::borrow::Cow<'a, NtupleReader<'f>> {
        if self.parallel {
            let mut view = self.ntuple.cluster_window(0, self.ntuple.summaries().len());
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
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if there is no such field, [`Error::TypeMismatch`] if it does not
    /// hold `T`, and any error reading its pages.
    pub fn read<T: FromColumn>(self) -> Result<Vec<T>> {
        let name = self.name;
        typed(name, self.values()?)
    }

    /// The values as a [`Column`], stored flat as the pages are: a
    /// `std::vector<T>` is offsets over one buffer, a record a struct of
    /// columns, an optional a validity mask over the values present, a
    /// `std::variant` a union.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if there is no such field, and any error reading its pages.
    pub fn values(self) -> Result<Column> {
        let reader = self.reader();
        match self.range {
            None => reader.read_top(reader.file, self.name),
            Some(r) => reader.read_range(self.name, r.start, r.end),
        }
    }

    pub(crate) fn name(&self) -> &'a str {
        self.name
    }

    /// The schema of the `RNTuple` the field is read from.
    pub(crate) fn header(&self) -> Arc<Header> {
        Arc::clone(&self.ntuple.header)
    }
}

impl std::fmt::Debug for NtupleReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NtupleReader")
            .field("clusters", &self.summaries().len())
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
