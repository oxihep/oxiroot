//! The start-here RNTuple example: write a flat event dataset, reopen it, and
//! read the columns back per entry — then put two RNTuples in one file with
//! `FileWriter` and read both. RNTuple is ROOT's modern columnar event-data
//! format; the files here are readable by official ROOT and uproot. The same
//! code writes a `TTree` with `Format::Tree`. (For records and other
//! RNTuple-only shapes, see `rntuple_nested.rs`.)
//!
//! Run with:
//!
//! ```sh
//! cargo run -p oxiroot --example rntuple
//! ```

use oxiroot::prelude::*;

fn main() -> oxiroot::Result<()> {
    let dir = std::env::temp_dir();

    // --- Build one flat event dataset, column by column. -----------------------
    // Every column of a batch has the same number of entries — here, four events.
    // Types are mixed freely: an `f64`, an `i32`, a string, and a
    // `std::vector<f64>` (a variable-length list per event).
    let events = Batch::new()
        .column("mass", vec![91.19, 125.10, 4.18, 173.1])? // GeV
        .column("charge", vec![0_i32, 0, -1, 1])?
        .column("label", vec!["Z", "H", "b", "top"])?
        .column(
            "jet_pt",
            vec![
                vec![55.0, 31.2],       // two jets
                vec![120.0],            // one jet
                vec![],                 // none
                vec![88.0, 40.0, 12.0], // three jets
            ],
        )?;

    // --- Write it as an RNTuple. Pages are Zstd-compressed on disk. -------------
    let path = dir.join("oxiroot_ex_rntuple.root");
    let mut out = DatasetWriter::create(&path, "events", Format::Ntuple)?;
    out.write(&events)?;
    out.finish()?;
    println!("wrote RNTuple `events` -> {}", path.display());

    // --- Reopen and inspect the schema before touching any data. ---------------
    // Opening only parses the anchor, header, and footer — pages are read lazily,
    // per column, when you ask for them.
    let f = FileReader::open(&path)?;
    let ntpl = Dataset::open(&f, "events")?;
    println!(
        "reopened {:?}: {} entries, columns = {:?}",
        ntpl.format(),
        ntpl.entries(),
        ntpl.columns(),
    );

    // --- Read the columns back: typed views over flat buffers. -----------------
    let batch = ntpl.read(["mass", "charge", "label", "jet_pt"])?;
    let masses = batch.values::<f64>("mass")?;
    let charges = batch.values::<i32>("charge")?;
    let labels = batch.strings("label")?;
    let jets = batch.jagged::<f64>("jet_pt")?; // one slice per entry

    // --- Print it as an event loop would see it: one row per entry. ------------
    println!("per-entry values:");
    for i in 0..batch.len() {
        println!(
            "  {:>3}: label = {:>3}  mass = {:>7.2} GeV  charge = {:+}  jets = {:?}",
            i,
            labels.get(i),
            masses[i],
            charges[i],
            jets.row(i),
        );
    }
    let total_jets = jets.values().len();
    println!("  ({total_jets} jets across {} events)", jets.len());

    // --- Several datasets in one file, via `FileWriter`. ------------------------
    // `FileWriter::put` puts more than one in the same file, next to histograms
    // and in `TDirectory`s: here the per-event `events` alongside a small per-run
    // bookkeeping RNTuple.
    let multi_path = dir.join("oxiroot_ex_rntuple_multi.root");
    let runs = Batch::new()
        .column("run", vec![101_i32, 102, 103])?
        .column("n_events", vec![12_000_i64, 8_400, 15_250])?;
    FileWriter::create(&multi_path)
        .put(DatasetObject::new(
            "events",
            Format::Ntuple,
            &events.slice(0..2),
        )?)
        .put(DatasetObject::new("runs", Format::Ntuple, &runs)?)
        .write(Compression::Zstd(5))?;
    println!("wrote 2 RNTuples -> {}", multi_path.display());

    // --- Read both back out of the one file. -----------------------------------
    let g = FileReader::open(&multi_path)?;
    let ev = Dataset::open(&g, "events")?;
    let runs = Dataset::open(&g, "runs")?;
    println!("  `events`: {} entries {:?}", ev.entries(), ev.columns());
    let n = runs.read(["n_events"])?;
    let total: i64 = n.values::<i64>("n_events")?.iter().sum();
    println!(
        "  `runs`:   {} entries, {total} events recorded across all runs",
        runs.entries(),
    );

    // --- Clean up the temp files (never litter). -------------------------------
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&multi_path);

    Ok(())
}
