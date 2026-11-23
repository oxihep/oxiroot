//! Merging ROOT files — a pure-Rust `hadd`.
//!
//! A [`Merger`] combines several ROOT files the way ROOT's `hadd` command-line
//! tool does: histograms are summed bin-by-bin, and `TTree` / `RNTuple` entries
//! are concatenated. It returns a [`MergeReport`] describing exactly what each
//! key became, so nothing is silently dropped.
//!
//! ```no_run
//! use oxiroot::hadd::Merger;
//!
//! let report = Merger::new().inputs(["run1.root", "run2.root"]).merge("all.root")?;
//! println!("{report}");
//! # Ok::<(), oxiroot::Error>(())
//! ```
//!
//! # What a fileset may contain
//!
//! One invocation writes **one** output file. The merger does not yet combine
//! histograms with a `TTree` or `RNTuple` in one output (a
//! [`FileWriter`](oxiroot_io_core::FileWriter) can hold all three, but the merger
//! concatenates each tree or `RNTuple` on its own path). So a fileset must be one
//! of:
//!
//! * **all histogram-family objects** — everything ROOT's `hadd` merges is
//!   merged the same way (`Hist1D`/`Hist2D`/`Hist3D`, the profiles, `PolyHist` and
//!   `SparseHist` bin by bin, an `Efficiency`'s histograms, a `HistStack`'s
//!   histograms by name, a `Parameter`'s value, and the graphs by appending
//!   their points); what it does not merge is copied from the first file
//!   (`Func1D`/`2`/`3`, `Graph2D`, `MultiErrorGraph`, `GraphStack`, strings,
//!   maps, matrices); unknown classes are skipped and reported;
//! * **a single `TTree`** (and nothing else) — entries concatenated;
//! * **a single `RNTuple`** (and nothing else) — entries concatenated.
//!
//! Anything else — a `TTree` or `RNTuple` alongside histograms, or more than one
//! of them — is refused with an error that names the keys, rather than writing a
//! partial file.
//!
//! The inputs are read on demand rather than loaded whole. A tree or `RNTuple` is
//! copied one cluster at a time through [`Dataset`] and [`DatasetWriter`], so
//! memory holds a cluster, and every column type a
//! [`Batch`](oxiroot_dataset::Batch) can hold is merged, stored as the first
//! input stores it (an `RNTuple` `std::variant` is the one that cannot). A large
//! merge is written in ROOT's 64-bit container form. The output must not be
//! one of the inputs.

use std::fmt;
use std::path::{Path, PathBuf};

use oxiroot_dataset::{DataType, Dataset, DatasetWriter, Format, Schema};
use oxiroot_io_core::raw::{KSTART_BIG_FILE, PendingFile};
use oxiroot_io_core::{ClassKind, Compression, Error, FileReader, FileSource, Result};

mod histograms;
pub use histograms::HistMergeOutcome;
use histograms::merge_histogram_files;

/// Inputs larger than this in total are merged straight into the 64-bit
/// container form; smaller ones switch to it only if the output turns out not to
/// fit the 32-bit form.
const LARGE_INPUT_BYTES: u64 = KSTART_BIG_FILE / 2;

/// Which merge ran, and what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeKind {
    /// A histogram-family fileset: histograms summed, other objects copied.
    Histograms(HistMergeOutcome),
    /// A single `TTree`, its entries concatenated.
    Tree {
        /// The tree's name.
        name: String,
        /// Entries written.
        entries: u64,
    },
    /// A single `RNTuple`, its entries concatenated.
    Ntuple {
        /// The `RNTuple`'s name.
        name: String,
        /// Entries written.
        entries: u64,
    },
}

/// A summary of a [`Merger::merge`] run: what was written and how.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MergeReport {
    /// The output file path.
    pub output: PathBuf,
    /// Number of input files merged.
    pub inputs: usize,
    /// Which merge ran, and what it did.
    pub kind: MergeKind,
}

impl fmt::Display for MergeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "merged {} file(s) into {}",
            self.inputs,
            self.output.display()
        )?;
        match &self.kind {
            MergeKind::Histograms(outcome) => {
                write!(
                    f,
                    ": {} summed, {} copied, {} skipped",
                    outcome.summed.len(),
                    outcome.copied.len(),
                    outcome.skipped.len()
                )?;
                for (name, reason) in &outcome.skipped {
                    write!(f, "\n  skipped {name:?}: {reason}")?;
                }
            }
            MergeKind::Tree { name, entries } => {
                write!(f, ": TTree {name:?} with {entries} entries")?;
            }
            MergeKind::Ntuple { name, entries } => {
                write!(f, ": RNTuple {name:?} with {entries} entries")?;
            }
        }
        Ok(())
    }
}

/// A file merge: collect inputs, choose compression, then
/// [`merge`](Merger::merge).
///
/// ```no_run
/// use oxiroot::hadd::Merger;
/// use oxiroot::Compression;
///
/// let report = Merger::new()
///     .input("run1.root")
///     .input("run2.root")
///     .compression(Compression::Zstd(5))
///     .merge("all.root")?;
/// # Ok::<(), oxiroot::Error>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct Merger {
    inputs: Vec<PathBuf>,
    compression: Option<Compression>,
}

impl Merger {
    /// A new, empty merger (default compression: Zstd level 5).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one input file.
    #[must_use]
    pub fn input(mut self, path: impl AsRef<Path>) -> Self {
        self.inputs.push(path.as_ref().to_path_buf());
        self
    }

    /// Add several input files.
    #[must_use]
    pub fn inputs<P: AsRef<Path>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.inputs
            .extend(paths.into_iter().map(|p| p.as_ref().to_path_buf()));
        self
    }

    /// Set the output compression (default: Zstd level 5).
    #[must_use]
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = Some(compression);
        self
    }

    /// Merge the collected inputs into `output`, dispatching on what the
    /// fileset contains (see the [module docs](self)).
    ///
    /// # Errors
    ///
    /// If there are no inputs, a file cannot be read, the output is also an
    /// input, the fileset mixes histograms with a `TTree`/`RNTuple` (or holds
    /// more than one of them), or the merge itself fails (incompatible
    /// histogram binnings, a branch or field type oxiroot cannot write back).
    pub fn merge(self, output: impl AsRef<Path>) -> crate::Result<MergeReport> {
        Ok(merge(
            output.as_ref(),
            &self.inputs,
            self.compression.unwrap_or_default(),
        )?)
    }
}

fn merge(output: &Path, inputs: &[PathBuf], compression: Compression) -> Result<MergeReport> {
    if inputs.is_empty() {
        return Err(Error::InvalidInput("merge: no input files".into()));
    }

    // The inputs are read on demand, so the output must not be one of them.
    let out_path = std::fs::canonicalize(output).ok();
    for input in inputs {
        if out_path.is_some() && std::fs::canonicalize(input).ok() == out_path {
            return Err(Error::InvalidInput(format!(
                "merge: the output {} is also an input",
                output.display()
            )));
        }
    }
    // Positioned reads: only the objects and data a merge touches are read, one
    // input's worth of tree or RNTuple entries at a time.
    let files: Vec<FileReader> = inputs
        .iter()
        .map(|path| FileReader::from_source(Box::new(FileSource::open(path)?)))
        .collect::<Result<Vec<_>>>()?;
    let large = files.iter().map(FileReader::size).sum::<u64>() > LARGE_INPUT_BYTES;

    // Union of top-level key names (first-seen order) with each key's class.
    let mut seen = std::collections::HashSet::new();
    let mut trees = Vec::new();
    let mut rntuples = Vec::new();
    let mut others = 0usize;
    for file in &files {
        for key in file.keys() {
            if key.is_deleted() || !seen.insert(key.name.clone()) {
                continue;
            }
            match ClassKind::of(&key.class_name) {
                ClassKind::Tree => trees.push(key.name.clone()),
                ClassKind::Ntuple => rntuples.push(key.name.clone()),
                _ => others += 1,
            }
        }
    }

    match (trees.len(), rntuples.len(), others) {
        // No trees or RNTuples: a histogram-family fileset.
        (0, 0, _) => merge_histograms(output, &files, inputs.len(), compression),
        // Exactly one TTree and nothing else.
        (1, 0, 0) => merge_dataset(output, &files, &trees[0], inputs.len(), compression, large),
        // Exactly one RNTuple and nothing else.
        (0, 1, 0) => merge_dataset(
            output,
            &files,
            &rntuples[0],
            inputs.len(),
            compression,
            large,
        ),
        // Anything mixed or plural: refuse loudly rather than write a partial file.
        _ => Err(Error::Unsupported(format!(
            "merge: this fileset mixes objects oxiroot cannot combine into one file yet \
             ({} TTree(s): {trees:?}; {} RNTuple(s): {rntuples:?}; {others} other object(s)). \
             oxiroot merges either an all-histogram fileset, a single TTree, or a single RNTuple \
             per call; merge the pieces separately.",
            trees.len(),
            rntuples.len(),
        ))),
    }
}

fn merge_histograms(
    output: &Path,
    files: &[FileReader],
    inputs: usize,
    compression: Compression,
) -> Result<MergeReport> {
    let outcome = merge_histogram_files(output, files, compression)?;
    Ok(MergeReport {
        output: output.to_path_buf(),
        inputs,
        kind: MergeKind::Histograms(outcome),
    })
}

/// Run `write` in the 32-bit container form unless `large`, and once more in the
/// 64-bit form if the 32-bit file turned out too large.
fn with_form_fallback(large: bool, mut write: impl FnMut(bool) -> Result<()>) -> Result<()> {
    match write(large) {
        Err(Error::FileTooLarge { .. }) if !large => write(true),
        other => other,
    }
}

/// Write `output` through a file beside it, renamed over it when complete: a
/// merge that fails part-way (an input with other columns, a full disk)
/// leaves no output, and an existing one as it was.
///
/// `write` gets the file to fill, the output's file name, and whether to use
/// the 64-bit container form; it runs again in that form if the 32-bit file
/// turned out too large.
fn write_atomically(
    output: &Path,
    large: bool,
    mut write: impl FnMut(std::fs::File, &str, bool) -> Result<()>,
) -> Result<()> {
    let file_name = output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file.root");
    // Each attempt fills a fresh temporary file; one that fails is deleted
    // when its `PendingFile` drops.
    let mut pending = None;
    with_form_fallback(large, |big| {
        let attempt = PendingFile::create(output)?;
        write(attempt.file()?, file_name, big)?;
        pending = Some(attempt);
        Ok(())
    })?;
    match pending {
        Some(p) => Ok(p.commit()?),
        None => Ok(()),
    }
}

/// `schema` with every counted array whose counter is not one of `columns`
/// stored as a `std::vector` instead.
///
/// A tree writes a counted array `x[n]` next to the branch that counts it,
/// which the writer names `n` followed by the array's name and adds when it
/// is missing. A member of a split object is counted by the object, and a C
/// array can be counted by a leaf of any name: for those the writer would add
/// a counter branch the inputs do not have. A `std::vector` holds the same
/// values and needs none.
fn without_invented_counters(schema: &Schema, columns: &[String]) -> Schema {
    Schema::new(schema.iter().map(|(name, data_type)| {
        let stored = match data_type {
            DataType::CArray(item) if !columns.contains(&format!("n{name}")) => {
                DataType::List(item.clone())
            }
            other => other.clone(),
        };
        (name, stored)
    }))
}

/// Concatenate the dataset `name` of every file into `output`: a `TTree` into
/// a `TTree`, an `RNTuple` into an `RNTuple`, each column stored as the first
/// input stores it.
///
/// One cluster of one input is in memory at a time. Every input must have the
/// first input's columns, of the same types.
fn merge_dataset(
    output: &Path,
    files: &[FileReader],
    name: &str,
    inputs: usize,
    compression: Compression,
    large: bool,
) -> Result<MergeReport> {
    let parts: Vec<Dataset<'_>> = files
        .iter()
        .map(|f| Dataset::open(f, name))
        .collect::<Result<Vec<_>>>()?;
    let first = &parts[0];
    let format = first.format();
    let entries = parts.iter().map(Dataset::entries).sum();
    let columns: Vec<String> = first.columns().into_iter().map(str::to_string).collect();
    let schema = without_invented_counters(&first.schema()?, &columns);
    // The key in the output: the object's own name, without its directory.
    let key = name.rsplit('/').next().unwrap_or(name);

    write_atomically(output, large, |sink, file_name, big| {
        let mut writer = DatasetWriter::new(sink, file_name, key, format)
            .compression(compression)
            .with_schema(schema.clone());
        if big {
            writer = writer.large();
        }
        for (i, part) in parts.iter().enumerate() {
            for batch in part.batches(&columns) {
                let batch = batch.map_err(|e| e.context(format_args!("input #{i}")))?;
                writer.append(batch)?;
            }
        }
        if entries == 0 {
            // No cluster to copy: the columns alone, so the output has them.
            writer.append(first.read(&columns)?)?;
        }
        writer.finish().map(drop)
    })?;

    Ok(MergeReport {
        output: output.to_path_buf(),
        inputs,
        kind: match format {
            Format::Tree => MergeKind::Tree {
                name: name.to_string(),
                entries,
            },
            // `Format` can grow; today everything that is not a tree is an RNTuple.
            _ => MergeKind::Ntuple {
                name: name.to_string(),
                entries,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::with_form_fallback;
    use oxiroot_io_core::Error;

    #[test]
    fn a_too_large_small_file_is_rewritten_in_the_large_form() {
        let mut forms = Vec::new();
        let result = with_form_fallback(false, |big| {
            forms.push(big);
            if big {
                Ok(())
            } else {
                Err(Error::FileTooLarge { size: 3 << 30 })
            }
        });
        assert!(result.is_ok());
        assert_eq!(forms, [false, true]);
    }

    #[test]
    fn other_errors_and_large_writes_are_not_retried() {
        let mut calls = 0;
        let result = with_form_fallback(false, |_| {
            calls += 1;
            Err(Error::Format("bad branch".into()))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);

        let mut forms = Vec::new();
        let result = with_form_fallback(true, |big| {
            forms.push(big);
            Err(Error::FileTooLarge { size: 1 })
        });
        assert!(matches!(result, Err(Error::FileTooLarge { .. })));
        assert_eq!(forms, [true]);
    }
}
