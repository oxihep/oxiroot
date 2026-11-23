//! [`TreeWriter`]: a tree written batch by batch, one basket per branch per
//! batch, so the whole dataset need not be in memory.

use oxiroot_io_core::cast::SaturatingFrom;
use std::io::{Seek, Write};
use std::path::Path;

use oxiroot_io_core::raw::{ContainerWriter, DirId, Objects};
use oxiroot_io_core::{Compression, Error, Result};

use super::baskets::{
    BasketRec, PreparedBasket, chunk_values, place_basket, prepare_basket, split_pieces,
};
use super::branch::{Branch, BranchKind, LeafMeta, SplitMember, SplitSpec, split_class};
use super::check_entry_counts;
use super::layout::Kind;
use super::serialize::build_tree_object;
use oxiroot_io_core::{Column, DataType};

/// A column's identity, used to check that every batch shares the first batch's
/// schema: name, value-variant, and the array/`std::vector` flags. Fixed-array
/// width is folded into the variant via `flen` so a shape change is caught too.
#[derive(PartialEq)]
struct ColSig {
    name: String,
    /// The type of the values: element type and shape.
    data_type: DataType,
    jagged: bool,
    stl_vector: bool,
    flen: i32,
    /// The element type code (`F`, `I`, …), so nested element types differ.
    elem: char,
    /// A split `std::vector<Struct>`'s class name and its members' names and
    /// types.
    split: Option<(String, Vec<MemberSig>)>,
}

/// A split member's name and value type.
type MemberSig = (String, DataType);

fn col_sig(b: &Branch) -> ColSig {
    ColSig {
        name: b.name.clone(),
        data_type: b.values.data_type(),
        jagged: b.jagged(),
        stl_vector: b.stl_vector(),
        flen: b.flen(),
        elem: b.leaf().code,
        split: b.split().map(|spec| {
            (
                spec.class_name.clone(),
                spec.members
                    .iter()
                    .map(|m| (m.name.clone(), m.values.data_type()))
                    .collect(),
            )
        }),
    }
}

/// A split branch's representative: its class and members, with no values.
/// What the header says of them is in its [`LeafMeta`].
fn split_rep(b: &Branch) -> Branch {
    let spec = b.split().expect("a split branch");
    let members = spec
        .members
        .iter()
        .map(|m| SplitMember {
            name: m.name.clone(),
            values: chunk_values(&m.values, 0, 0),
        })
        .collect();
    let mut rep = Branch::raw(
        b.name.clone(),
        Column::I32(Vec::new()),
        BranchKind::Split(SplitSpec {
            class_name: spec.class_name.clone(),
            members,
        }),
    );
    rep.meta = Some(LeafMeta::default());
    rep
}

/// The largest element count in a split branch's count piece (one `i32` per
/// entry).
fn max_count(count_piece: &Branch) -> usize {
    match &count_piece.values {
        Column::List { values, .. } => match &**values {
            Column::I32(counts) => counts
                .iter()
                .map(|&n| usize::saturating_from(n))
                .max()
                .unwrap_or(0),
            _ => 0,
        },
        _ => 0,
    }
}

/// The running aggregate a streamed column must keep so its leaf metadata is
/// correct once every batch has been seen.
enum ColAgg {
    /// A plain data column (scalar / fixed array / jagged data / `std::vector`):
    /// its leaf needs no value-derived aggregate.
    Data,
    /// A synthetic `n<name>` count column: track the maximum multiplicity, which
    /// becomes the count leaf's `fMaximum` (ROOT sizes the read buffer from it).
    Count(i64),
    /// A `TLeafC` string column: track the longest string length + 1, which is
    /// the leaf's `fLen` (the buffer ROOT allocates for the string).
    Str(i32),
    /// A split `std::vector<Struct>` column: track the largest element count,
    /// the parent's `fMaximum`.
    Split(usize),
}

/// Accumulated state for one effective output column across batches.
struct StreamCol {
    /// A representative branch carrying the column's type/flags (and, for fixed
    /// arrays, its width) plus minimal values, so [`build_tree_object`] can emit
    /// the branch/leaf metadata. Its aggregate-bearing values (count `fMaximum`,
    /// string `fLen`) are kept at the running maximum via [`StreamCol::agg`].
    rep: Branch,
    /// One [`BasketRec`] per batch written so far.
    baskets: Vec<BasketRec>,
    agg: ColAgg,
}

/// A streaming, bounded-memory `TTree` writer. Append entries in batches with
/// [`write_batch`](TreeWriter::write_batch); each call emits one basket per
/// branch straight to the sink, so only the current batch's data is held in
/// memory (the way ROOT's `TTree::Fill` flushes baskets as they fill).
/// [`finish`](TreeWriter::finish) writes the small `TTree` metadata, the
/// streamer info, and the key list, then patches the file header.
///
/// Every batch must share the first batch's schema: branch names, element
/// types, the jagged / `std::vector` flags, fixed-array widths, and a split
/// `std::vector<Struct>` branch's class and members.
///
/// ```no_run
/// use oxiroot_io_core::Compression;
/// use oxiroot_tree::raw::{Branch, TreeWriter};
///
/// let mut w = TreeWriter::create("big.root", "T", Compression::Stored)?;
/// for batch in 0..1_000 {
///     let x: Vec<f64> = (0..10_000).map(|i| (batch * 10_000 + i) as f64).collect();
///     w.write_batch(&[Branch::new("x", x)])?; // one basket, flushed now
/// }
/// w.finish()?;
/// # Ok::<(), oxiroot_io_core::Error>(())
/// ```
#[doc(alias = "TTreeWriter")]
#[must_use = "nothing is readable until `finish()` writes the file's directory"]
pub struct TreeWriter<W: Write + Seek> {
    file: ContainerWriter<W>,
    tree_name: String,
    /// Effective columns (count branches expanded inline); set by the first batch.
    columns: Vec<StreamCol>,
    /// The first batch's schema; `None` until the first batch is written.
    schema: Option<Vec<ColSig>>,
    total_entries: i64,
    /// For a writer [`create`](Self::create)d at a path: the temporary file
    /// it writes, which takes the path's place when the writer finishes.
    pending: Option<oxiroot_io_core::raw::PendingFile>,
}

impl TreeWriter<std::fs::File> {
    /// Create a streaming tree file at `path`. It is written beside `path`
    /// and takes its place when [`finish`](TreeWriter::finish) succeeds, so a
    /// write that fails or is dropped leaves `path` as it was. The tree is named `tree_name`;
    /// the file is the small (32-bit) container, so the total must stay under
    /// 2 GiB ([`finish`](TreeWriter::finish) errors otherwise). For a file that
    /// may exceed 2 GiB, use [`create_large`](TreeWriter::create_large).
    ///
    /// # Errors
    ///
    /// If the file cannot be created, or its header cannot be written.
    pub fn create(
        path: impl AsRef<Path>,
        tree_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::create_fmt(path, tree_name, compression, false)
    }

    /// Like [`create`](TreeWriter::create), but writes the 64-bit ("big")
    /// container form so the tree may exceed 2 GiB. Use this when the streamed
    /// dataset is expected to be large; small files are still valid, just stored
    /// in the wider form (as ROOT does past `kStartBigFile`).
    ///
    /// # Errors
    ///
    /// As [`create`](Self::create).
    pub fn create_large(
        path: impl AsRef<Path>,
        tree_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::create_fmt(path, tree_name, compression, true)
    }

    fn create_fmt(
        path: impl AsRef<Path>,
        tree_name: &str,
        compression: Compression,
        big: bool,
    ) -> Result<Self> {
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.root")
            .to_string();
        // Written beside `path` and renamed over it when finished, so a
        // write that fails or is dropped leaves `path` as it was.
        let pending = oxiroot_io_core::raw::PendingFile::create(path)?;
        let mut writer = Self::new_fmt(pending.file()?, &file_name, tree_name, compression, big)?;
        writer.pending = Some(pending);
        Ok(writer)
    }
}

impl<W: Write + Seek> TreeWriter<W> {
    /// Begin writing into an arbitrary seekable sink (small 32-bit container).
    /// The file header and root directory record are written immediately (with
    /// pointers patched at the end); `file_name` is the name stored in the
    /// directory record. See [`new_large`](TreeWriter::new_large) for the
    /// >2 GiB form.
    ///
    /// # Errors
    ///
    /// If the file's header cannot be written to `sink`.
    pub fn new(
        sink: W,
        file_name: &str,
        tree_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::new_fmt(sink, file_name, tree_name, compression, false)
    }

    /// Like [`new`](TreeWriter::new), but writes the 64-bit ("big") container
    /// form so the streamed file may exceed 2 GiB.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_large(
        sink: W,
        file_name: &str,
        tree_name: &str,
        compression: Compression,
    ) -> Result<Self> {
        Self::new_fmt(sink, file_name, tree_name, compression, true)
    }

    fn new_fmt(
        sink: W,
        file_name: &str,
        tree_name: &str,
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
            tree_name: tree_name.to_string(),
            columns: Vec::new(),
            schema: None,
            total_entries: 0,
            pending: None,
        })
    }

    /// Total entries appended so far.
    #[must_use]
    pub fn entries(&self) -> u64 {
        u64::try_from(self.total_entries).unwrap_or(0)
    }

    /// Append one batch of entries (one basket per branch). The first batch fixes
    /// the schema; later batches must match it. An empty batch is a no-op.
    ///
    /// # Errors
    ///
    /// [`Error::SchemaChanged`] if the branches differ from the first batch's,
    /// [`Error::InvalidInput`] for a batch the format cannot hold as declared, and any
    /// error writing to the sink.
    pub fn write_batch(&mut self, branches: &[Branch]) -> Result<()> {
        self.write_chunks(&[branches])
    }

    /// Append several batches at once, in order: one basket per branch per
    /// batch. With the `rayon` feature every basket of every batch is
    /// serialized and compressed in parallel before they are written, so a
    /// cluster cut into many baskets costs about one basket's time per core.
    ///
    /// # Errors
    ///
    /// As [`write_batch`](Self::write_batch), for the first batch that fails.
    pub fn write_batches(&mut self, batches: &[Vec<Branch>]) -> Result<()> {
        let slices: Vec<&[Branch]> = batches.iter().map(Vec::as_slice).collect();
        self.write_chunks(&slices)
    }

    fn write_chunks(&mut self, batches: &[&[Branch]]) -> Result<()> {
        let mut entries = Vec::with_capacity(batches.len());
        for branches in batches {
            entries.push(self.check_batch(branches)?);
        }
        // The branches each batch's baskets are made from: a jagged branch's
        // synthetic count branch, then the branch itself (two columns); a split
        // branch's count and member pieces (one column, its baskets
        // interleaved); any other branch as it is (one column).
        let pieces: Vec<Vec<Vec<Branch>>> = batches
            .iter()
            .map(|bs| {
                bs.iter()
                    .map(|b| {
                        if b.jagged() {
                            b.count_branch().into_iter().collect()
                        } else if b.split().is_some() {
                            split_pieces(b, 0, b.n_entries() as usize)
                        } else {
                            Vec::new()
                        }
                    })
                    .collect()
            })
            .collect();
        // Each basket to write, and the column it belongs to.
        let mut jobs: Vec<(usize, &Branch)> = Vec::new();
        for ((branches, pieces), &n) in batches.iter().zip(&pieces).zip(&entries) {
            if n == 0 {
                continue;
            }
            let mut col = 0;
            for (b, extra) in branches.iter().zip(pieces) {
                if b.split().is_some() {
                    jobs.extend(extra.iter().map(|p| (col, p)));
                    col += 1;
                    continue;
                }
                if let Some(c) = extra.first() {
                    jobs.push((col, c));
                    col += 1;
                }
                jobs.push((col, b));
                col += 1;
            }
        }
        let compression = self.file.compression();
        let tree_name = self.tree_name.clone();
        let prepare = |(_, b): &(usize, &Branch)| prepare_basket(b, &tree_name, compression);
        #[cfg(feature = "rayon")]
        let prepared: Vec<PreparedBasket> = {
            use rayon::prelude::*;
            jobs.par_iter().map(prepare).collect::<Result<_>>()?
        };
        #[cfg(not(feature = "rayon"))]
        let prepared: Vec<PreparedBasket> = jobs.iter().map(prepare).collect::<Result<_>>()?;

        for (p, &(col, branch)) in prepared.iter().zip(&jobs) {
            let rec = place_basket(&mut self.file, DirId::TOP, p, &tree_name)?;
            self.record(col, branch, rec);
        }
        self.total_entries += entries.iter().map(|&n| i64::from(n)).sum::<i64>();
        Ok(())
    }

    /// Check one batch and fix the schema on the first non-empty one. Returns
    /// its entry count.
    fn check_batch(&mut self, branches: &[Branch]) -> Result<u32> {
        for b in branches {
            b.check()?;
        }

        // All branches in a batch must carry the same number of entries.
        check_entry_counts(branches)?;
        let batch_entries = branches.first().map_or(0, Branch::n_entries);
        // An empty batch writes nothing, but the first one still fixes the
        // schema, so a tree with no entries can be written.
        if branches.is_empty() || (batch_entries == 0 && self.schema.is_some()) {
            return Ok(0);
        }

        let sig: Vec<ColSig> = branches.iter().map(col_sig).collect();
        match &self.schema {
            Some(prev) if *prev != sig => {
                return Err(Error::SchemaChanged {
                    detail: "this batch's branch schema differs from the first batch's".into(),
                });
            }
            Some(_) => {}
            None => {
                self.init_columns(branches);
                self.schema = Some(sig);
            }
        }
        Ok(batch_entries)
    }

    /// Record one placed basket for column `col`, and grow that column's leaf
    /// aggregate (count `fMaximum` / string `fLen`).
    fn record(&mut self, col: usize, branch: &Branch, rec: BasketRec) {
        let c = &mut self.columns[col];
        c.baskets.push(rec);
        match &mut c.agg {
            ColAgg::Count(m) => {
                let batch_max = branch.leaf_max();
                if batch_max > *m {
                    *m = batch_max;
                    c.rep.meta = Some(LeafMeta {
                        max: *m,
                        ..LeafMeta::default()
                    });
                }
            }
            ColAgg::Str(len) => {
                let batch_len = branch.str_len();
                if batch_len > *len {
                    *len = batch_len;
                    c.rep.meta = Some(LeafMeta {
                        max: i64::from(*len),
                        str_len: *len,
                        ..LeafMeta::default()
                    });
                }
            }
            // The count piece (named after the branch) carries the counts;
            // the member pieces add nothing.
            ColAgg::Split(m) if branch.name == c.rep.name => {
                let batch_max = max_count(branch);
                if batch_max > *m {
                    *m = batch_max;
                    c.rep.meta = Some(LeafMeta {
                        split_max: i32::saturating_from(*m),
                        ..LeafMeta::default()
                    });
                }
            }
            ColAgg::Split(_) | ColAgg::Data => {}
        }
    }

    /// Build the effective-column list from the first batch (jagged branches
    /// expanded into a synthetic count column followed by the data column).
    fn init_columns(&mut self, branches: &[Branch]) {
        let mut cols = Vec::new();
        for b in branches {
            if b.split().is_some() {
                cols.push(StreamCol {
                    rep: split_rep(b),
                    baskets: Vec::new(),
                    agg: ColAgg::Split(0),
                });
                continue;
            }
            if b.jagged() {
                let count = b
                    .count_branch()
                    .expect("a jagged branch has a count branch");
                let m = count.leaf_max();
                let mut rep = Branch::raw(
                    count.name.clone(),
                    Column::I32(Vec::new()),
                    BranchKind::Count,
                );
                rep.meta = Some(LeafMeta {
                    max: m,
                    ..LeafMeta::default()
                });
                cols.push(StreamCol {
                    rep,
                    baskets: Vec::new(),
                    agg: ColAgg::Count(m),
                });
            }
            let (agg, meta) = if matches!(b.kind(), Kind::Str) {
                let len = b.str_len();
                let meta = LeafMeta {
                    max: i64::from(len),
                    str_len: len,
                    ..LeafMeta::default()
                };
                (ColAgg::Str(len), Some(meta))
            } else {
                (ColAgg::Data, None)
            };
            // No rows are needed: the column says its type and, for a fixed
            // array, its width.
            let mut rep = Branch::raw(
                b.name.clone(),
                chunk_values(&b.values, 0, 0),
                b.chunk_kind(),
            );
            rep.meta = meta;
            cols.push(StreamCol {
                rep,
                baskets: Vec::new(),
                agg,
            });
        }
        self.columns = cols;
    }

    /// Finish the file: write the `TTree` object, streamer info, and key list,
    /// then patch the header pointers. Returns the sink. Errors if no batch was
    /// written or the file exceeds the 2 GiB small-format limit.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if no batch was written, [`Error::FileTooLarge`] if a small
    /// file grew past what it can address, and any error writing to the sink.
    pub fn finish(self) -> Result<W> {
        self.finish_with(&Objects::default())
    }

    /// [`finish`](Self::finish), storing `objects` (histograms, parameters)
    /// in the top directory next to the tree.
    ///
    /// # Errors
    ///
    /// As [`finish`](Self::finish), and an object that cannot be stored: an
    /// unnamed one, or one named as the tree or as another.
    pub fn finish_with(mut self, objects: &Objects) -> Result<W> {
        if self.schema.is_none() {
            return Err(Error::InvalidInput(
                "TreeWriter finished with no batches written".into(),
            ));
        }
        let tot_bytes: i64 = self
            .columns
            .iter()
            .flat_map(|c| &c.baskets)
            .map(|r| i64::from(r.nbytes))
            .sum();
        let eff: Vec<&Branch> = self.columns.iter().map(|c| &c.rep).collect();
        let groups: Vec<Vec<BasketRec>> = self.columns.iter().map(|c| c.baskets.clone()).collect();

        let tree_obj = build_tree_object(
            &self.tree_name,
            &eff,
            &groups,
            self.total_entries,
            tot_bytes,
            self.file.is_big(),
        );
        self.file
            .place_key(DirId::TOP, "TTree", &self.tree_name, "", &tree_obj)?;
        objects.place(&mut self.file, &self.tree_name)?;
        // The tree classes, each split branch's struct, and the objects' own.
        let structs: Vec<_> = self
            .columns
            .iter()
            .filter_map(|c| c.rep.split())
            .map(split_class)
            .chain(objects.classes().iter().cloned())
            .collect();
        self.file
            .place_streamer_info(&crate::streamer_gen::tree_streamer_info(), &structs)?;
        let sink = self.file.finish()?;
        if let Some(pending) = self.pending.take() {
            pending.commit()?;
        }
        Ok(sink)
    }
}

impl<W: Write + Seek> std::fmt::Debug for TreeWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeWriter")
            .field("name", &self.tree_name)
            .field("entries", &self.total_entries)
            .finish_non_exhaustive()
    }
}
