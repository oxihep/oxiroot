//! Merging histogram files — the histogram half of a [`Merger`](super::Merger).
//!
//! [`merge_histogram_files`] combines several ROOT files whose keys are all
//! histogram-family objects. Everything ROOT's `hadd` merges is merged the same
//! way: `Hist1D`/`Hist2D`/`Hist3D`, the `Profile1D`s, `PolyHist` and `SparseHist` bin by
//! bin, an `Efficiency`'s passed and total histograms, a `HistStack`'s histograms
//! by name, a `Parameter`'s value, and the graphs by appending their points.
//! What ROOT's `hadd` does not merge is copied from the first file that holds
//! it: `Func1D`/`Func2D`/`Func3D`, `Graph2D`, `MultiErrorGraph`, `GraphStack`,
//! strings, maps and matrices. (ROOT writes one key per input for those, which
//! oxiroot cannot do: it rejects two objects of the same name in one directory.)
//! Objects of a class oxiroot cannot read *and* write are skipped and listed in
//! the returned report rather than silently dropped.
//!
//! Each object is written as soon as it is merged, so one is in memory at a
//! time, and the output's `TStreamerInfo` is built from the objects written
//! (no reliance on the inputs' streamer versions). The output takes its name
//! only once complete. [`Merger`](super::Merger)
//! uses it when a fileset contains no `TTree` or `RNTuple`.

use std::collections::HashSet;
use std::path::Path;

use oxiroot_io_core::raw::{ContainerWriter, DirId, Form, Key, record_of};
use oxiroot_io_core::{Compression, FileReader, ReadRoot, Result, StreamerSet};

use oxiroot_linalg::{Matrix, SymMatrix, Vector};

use oxiroot_hist::{
    Efficiency, Graph, Graph2D, GraphStack, Hist1D, Hist2D, Hist3D, HistStack, Mergeable,
    MultiErrorGraph, PolyHist, Profile1D, Profile2D, Profile3D, SparseHist,
};
use oxiroot_hist_func::{Func1D, Func2D, Func3D};
use oxiroot_io_core::{ObjMap, ObjString, Parameter, WriteRoot};

/// What a histogram merge did with each key: the names that were summed
/// bin-by-bin, the names copied from the first file, and the `(name, reason)`
/// pairs skipped because oxiroot cannot merge or reproduce that class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HistMergeOutcome {
    /// Keys summed across all inputs (`Hist1D`/`Hist2D`/`Hist3D` and the 1-, 2- and 3-D
    /// profiles).
    pub summed: Vec<String>,
    /// Keys copied from the first file that holds them (not summed).
    pub copied: Vec<String>,
    /// Keys skipped, with the reason (an unmergeable / unreadable class).
    pub skipped: Vec<(String, String)>,
}

/// The dimension of a summable histogram class (`TH1D` → 1, `TH2F` → 2,
/// `TH3I` → 3), or `None` for anything else (including `PolyHist`, `SparseHist`,
/// and `HistStack`, which are not summed here).
fn summable_hist_dim(class: &str) -> Option<u8> {
    match oxiroot_io_core::ClassKind::of(class) {
        oxiroot_io_core::ClassKind::Hist(dim) => Some(dim),
        _ => None,
    }
}

/// Merge histogram-family files into `output`.
///
/// The union of the inputs' top-level keys (in first-seen order) is merged:
/// each summable histogram is added across every input that holds it (keeping
/// the first file's name, title, and binning), and every other supported object
/// is taken from the first file that holds it. `inputs` are already-opened files
/// (the caller opens them); it must be non-empty.
///
/// # Errors
///
/// Returns an error if two histograms with the same key have incompatible
/// binning (as ROOT's `hadd` also refuses; the error names the key), or if
/// writing the output fails. Classes oxiroot cannot merge, and objects it cannot
/// read from any input, are recorded in the returned
/// [`HistMergeOutcome::skipped`] rather than erroring; a key is never written as
/// a partial sum.
pub(crate) fn merge_histogram_files(
    output: &Path,
    inputs: &[FileReader],
    compression: Compression,
) -> Result<HistMergeOutcome> {
    // Union of top-level key names, in first-seen order, with the class of the
    // first file that holds each.
    let mut order: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for file in inputs {
        for key in file.keys() {
            if key.is_deleted() {
                continue;
            }
            if seen.insert(key.name.clone()) {
                order.push((key.name.clone(), key.class_name.clone()));
            }
        }
    }

    // One object is in memory at a time: each is merged, written, and
    // dropped before the next. Only the classes they need are kept, for the
    // streamer info that closes the file.
    let mut outcome = HistMergeOutcome::default();
    super::write_atomically(output, false, |sink, file_name, big| {
        let form = if big { Form::Big } else { Form::Small };
        let mut file = ContainerWriter::new(sink, file_name, compression, form)?;
        let mut streamers = StreamerSet::default();
        outcome = HistMergeOutcome::default();
        for (name, class) in &order {
            // Inputs that hold this key with a matching class, in file order.
            let contributors: Vec<&FileReader> = inputs
                .iter()
                .filter(|f| f.key(name).is_ok_and(|k| k.class_name == *class))
                .collect();
            if contributors.is_empty() {
                continue;
            }
            let object = match build_object(class, name, &contributors)? {
                Built::Summed(object) => {
                    outcome.summed.push(name.clone());
                    object
                }
                Built::Copied(object) => {
                    outcome.copied.push(name.clone());
                    object
                }
                Built::Skipped(reason) => {
                    outcome.skipped.push((name.clone(), reason));
                    continue;
                }
            };
            let record = record_of(&*object);
            let key_len = Key::header_len(
                &record.class_name,
                &record.name,
                &record.title,
                file.is_big(),
            );
            (record.key_check).check(&record.class_name, &record.name, key_len)?;
            file.place_key(
                DirId::TOP,
                &record.class_name,
                &record.name,
                &record.title,
                &record.object,
            )?;
            streamers.add(&*object);
        }
        file.place_streamer_info(&[], streamers.classes())?;
        file.finish().map(drop)
    })?;
    Ok(outcome)
}

/// The result of merging one key.
enum Built {
    Summed(Box<dyn WriteRoot>),
    Copied(Box<dyn WriteRoot>),
    Skipped(String),
}

/// Sum a summable histogram type across `contributors` (first as the
/// accumulator), or copy a supported non-summable type from the first
/// contributor. Unknown classes become [`Built::Skipped`].
fn build_object(class: &str, name: &str, contributors: &[&FileReader]) -> Result<Built> {
    if let Some(dim) = summable_hist_dim(class) {
        return Ok(match dim {
            1 => summed(class, name, contributors, Hist1D::merge)?,
            2 => summed(class, name, contributors, Hist2D::merge)?,
            _ => summed(class, name, contributors, Hist3D::merge)?,
        });
    }

    let built = match class {
        "TProfile" => summed(class, name, contributors, Profile1D::merge)?,
        "TProfile2D" => summed(class, name, contributors, Profile2D::merge)?,
        "TProfile3D" => summed(class, name, contributors, Profile3D::merge)?,
        // Summed, as ROOT's hadd does: the efficiency's two histograms, the
        // poly's bins, the sparse histogram's filled bins, the graphs' points
        // (appended), the stack's histograms by name, and the parameter's value.
        "TEfficiency" => summed(class, name, contributors, Efficiency::merge)?,
        "TH2Poly" => summed(class, name, contributors, PolyHist::merge)?,
        "TGraph" | "TGraphErrors" | "TGraphAsymmErrors" => {
            summed(class, name, contributors, Graph::merge)?
        }
        "THStack" => summed(class, name, contributors, HistStack::merge)?,
        c if c.starts_with("THnSparse") => summed(class, name, contributors, SparseHist::merge)?,
        // A parameter is not a histogram, so it is not `Mergeable`: add values.
        c if c.starts_with("TParameter") => summed(class, name, contributors, Parameter::merge)?,
        // Copied from the first file: ROOT's hadd does not merge these either.
        // It writes one key per input instead, which oxiroot cannot do, since
        // it rejects two objects of the same name in one directory.
        "TF1" => copied::<Func1D>(class, name, contributors),
        "TF2" => copied::<Func2D>(class, name, contributors),
        "TF3" => copied::<Func3D>(class, name, contributors),
        "TGraph2D" => copied::<Graph2D>(class, name, contributors),
        // ROOT 6.40's hadd crashes merging this one.
        "TGraphMultiErrors" => copied::<MultiErrorGraph>(class, name, contributors),
        "TObjString" => copied::<ObjString>(class, name, contributors),
        "TMultiGraph" => copied::<GraphStack>(class, name, contributors),
        "TMap" => copied::<ObjMap>(class, name, contributors),
        c if c.starts_with("TVectorT") => copied::<Vector>(class, name, contributors),
        c if c.starts_with("TMatrixTSym") => copied::<SymMatrix>(class, name, contributors),
        c if c.starts_with("TMatrixT") => copied::<Matrix>(class, name, contributors),
        other => Built::Skipped(format!("class {other:?} is not mergeable by oxiroot")),
    };
    Ok(built)
}

/// Add every contributor, read as `T`, onto the first with `merge`. If any of
/// them cannot be read, the whole key is skipped with the reason, as
/// [`copied`] does, so one unreadable object neither aborts the merge nor
/// leaves a partial sum.
fn summed<T: ReadRoot + WriteRoot + 'static>(
    class: &str,
    name: &str,
    contributors: &[&FileReader],
    merge: fn(&mut T, &T) -> Result<()>,
) -> Result<Built> {
    let mut sum: Option<T> = None;
    for (i, file) in contributors.iter().enumerate() {
        let object = match file.get::<T>(name) {
            Ok(object) => object,
            Err(e) => {
                return Ok(Built::Skipped(format!(
                    "cannot read {class} from input {} of {}: {e}",
                    i + 1,
                    contributors.len()
                )));
            }
        };
        match sum.as_mut() {
            None => sum = Some(object),
            Some(sum) => merge(sum, &object).map_err(|e| e.context(format_args!("{name:?}")))?,
        }
    }
    Ok(match sum {
        Some(sum) => Built::Summed(Box::new(sum)),
        None => Built::Skipped("no input holds it".into()),
    })
}

/// Copy `T` from the first contributor; a read failure becomes a skip, so one
/// odd object never aborts the whole merge.
fn copied<T: ReadRoot + WriteRoot + 'static>(
    class: &str,
    name: &str,
    contributors: &[&FileReader],
) -> Built {
    match contributors[0].get::<T>(name) {
        Ok(object) => Built::Copied(Box::new(object)),
        Err(e) => Built::Skipped(format!("cannot read {class}: {e}")),
    }
}
