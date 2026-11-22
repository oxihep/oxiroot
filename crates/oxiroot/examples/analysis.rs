//! A miniature analysis end-to-end: fill weighted histograms (with variable
//! bins), combine and normalize them (`*=`, `Index`, the `Histogram` trait),
//! write them — into subdirectories and a flat heterogeneous file via the
//! `FileWriter` (with a float-precision `TH1F`) — write a columnar event
//! dataset, and read it back (`FileReader::get`) — all readable by official ROOT
//! and uproot.
//!
//! Run with: `cargo run -p oxiroot --example analysis`

use oxiroot::hist::{BinContentType, ThreadedHist};
use oxiroot::prelude::*;

fn main() -> oxiroot::Result<()> {
    let dir = std::env::temp_dir();

    // --- Fill histograms, as in an event loop. ---------------------------------
    // A momentum spectrum with physics-style variable bins and weighted entries.
    let edges = [0.0, 10.0, 20.0, 40.0, 80.0, 160.0];
    let mut pt = Hist::var(&edges)
        .weight()
        .named("pt")
        .titled("p_{T} spectrum");

    // A 2-D correlation.
    let mut eta_phi = Hist::reg(5, -2.5, 2.5)
        .reg(4, -3.2, 3.2)
        .double()
        .named("eta_phi")
        .titled("#eta vs #phi");

    // Pretend events: (pt, weight, eta, phi).
    let events = [
        (5.0, 1.2, 0.3, 1.0),
        (15.0, 0.8, -1.1, -2.0),
        (35.0, 1.5, 2.0, 0.5),
        (90.0, 1.0, 0.1, 3.0),
        (12.0, 0.9, -0.4, -0.2),
    ];
    for &(p, w, eta, phi) in &events {
        pt.fill_weight(p, w);
        eta_phi.fill(eta, phi);
    }
    println!(
        "pt: {} entries, integral {:.2}, all-cell sum {:.2}, bin-2 = {:.2} ± {:.2}",
        pt.entries(),
        pt.integral(false),           // in-range bins only
        pt.sum(true).value,           // every bin, flow included
        pt.value([1]).unwrap_or(0.0), // a bin's content, by index
        pt.variance([1]).unwrap_or(0.0).sqrt(),
    );

    // --- Multithreaded fill, ROOT's TThreadedObject pattern. -------------------
    // Share `&acc` across threads and call `fill` from any of them — each thread
    // transparently gets its own copy; `merge` combines them exactly (identical
    // to a serial fill).
    let samples: Vec<f64> = (0..100_000)
        .map(|i| (f64::from(i) * 0.618) % 100.0)
        .collect();
    let acc = ThreadedHist::new(
        Hist::reg(100, 0.0, 100.0)
            .double()
            .named("mass")
            .titled("toy mass"),
    );
    std::thread::scope(|s| {
        for chunk in samples.chunks(samples.len().div_ceil(4)) {
            let acc = &acc;
            s.spawn(move || {
                for &x in chunk {
                    acc.fill(x); // routes to this thread's copy, no manual setup
                }
            });
        }
    });
    let mass = acc.merge()?;
    let (peak_bin, peak) = mass.maximum().expect("a histogram with bins");
    println!(
        "mass (parallel fill): {} entries, mean {:.3}, std {:.3}, max bin {} = {}",
        mass.entries(),
        mass.mean(),
        mass.std_dev(),
        peak_bin[0],
        peak,
    );

    // --- Combine and normalize, as when merging samples. -----------------------
    let mut signal = pt.clone();
    let mut background = pt.clone();
    background.scale(0.1); // scale background down
    signal.add(&background)?; // stack background onto signal (a merge)
    signal.scale(1.0 / signal.integral(false).max(1.0)); // normalize to unit area
    println!("normalized signal integral = {:.6}", signal.integral(false));

    // --- Compose a file with `FileWriter`: top-level objects plus -----------------
    // per-region subdirectories. The one way to write more than a single object.
    let hist_path = dir.join("analysis_hists.root");
    let (signal_record, background_record) = (signal.to_root()?, background.to_root()?);
    FileWriter::create(&hist_path)
        .add(&pt.to_root()?) // top level
        .add(&eta_phi.to_root()?)
        .dir("signal", |d| d.add(&signal_record)) // a TDirectory per region
        .dir("background", |d| d.add(&background_record))
        .write()?;
    println!("wrote histograms -> {}", hist_path.display());

    // --- A flat file mixing any writable types — histograms, profiles, graphs. -
    let mut prof = Hist::reg(5, 0.0, 5.0)
        .mean()
        .named("pt_prof")
        .titled("<pt> per region");
    for (region, &(p, ..)) in events.iter().enumerate() {
        prof.fill_sample(region as f64 + 0.5, p);
    }
    // Write `pt` as a float-precision TH1F just by setting its on-disk bin content type.
    let pt_f32 = pt.to_root()?.with_bin_content_type(BinContentType::F32);
    let multi_path = dir.join("analysis_multi.root");
    FileWriter::create(&multi_path)
        .add(&pt_f32)
        .add(&eta_phi.to_root()?)
        .add(&prof.to_root()?)
        .write()?;
    println!(
        "wrote {} as {} + TH2D + TProfile",
        multi_path.display(),
        pt_f32.class_name(),
    );

    // --- Append one more object to that existing file (FileUpdater::open). -------
    // The file keeps its existing keys; the normalized signal is added alongside.
    let sig = signal.clone().named("signal");
    oxiroot::FileUpdater::open(&multi_path)?
        .add(&sig.to_root()?)
        .write()?;
    println!("appended `{}` to {}", sig.name, multi_path.display());

    // --- Write a columnar event dataset: a `Batch`, written as an RNTuple. ----
    let events_batch = Batch::new()
        .column("mass", vec![91.2, 125.1, 173.0])?
        .column("charge", vec![0_i32, -1, 1])?
        .column("label", vec!["Z", "H", "top"])?
        .column("jet_pt", vec![vec![30.0, 25.0], vec![], vec![120.0]])?;
    let ntuple_path = dir.join("analysis_events.root");
    FileWriter::create(&ntuple_path)
        .add(NamedBatch::new(
            "events",
            Format::Ntuple,
            events_batch.clone(),
        ))
        .write()?;
    println!("wrote RNTuple -> {}", ntuple_path.display());

    // --- The same batch as a classic TTree: only the format changes. -----------
    let tree_path = dir.join("analysis_tree.root");
    FileWriter::create(&tree_path)
        .add(NamedBatch::new("Events", Format::Tree, events_batch))
        .write()?;
    println!("wrote TTree   -> {}", tree_path.display());

    // --- Read it all back (`file.get::<Hist1D>(path)`; a subdirectory is part of the path).
    let f = FileReader::open(&hist_path)?;
    let pt_back = f.get::<Hist1D>("pt")?;
    let sig_back = f.get::<Hist1D>("signal/pt")?;
    println!(
        "read back: pt has {} bins, signal/pt integral = {:.6}",
        pt_back.values().len(),
        sig_back.values().iter().sum::<f64>(),
    );

    // Both read back the same way.
    for (path, name) in [(&ntuple_path, "events"), (&tree_path, "Events")] {
        let file = FileReader::open(path)?;
        let events = Dataset::open(&file, name)?;
        let batch = events.read(["mass", "jet_pt"])?;
        let jets: Vec<&[f64]> = batch.jagged::<f64>("jet_pt")?.rows().collect();
        println!(
            "{:?} `{name}`: {} entries, mass {:?}, jet_pt {jets:?}",
            events.format(),
            events.entries(),
            batch.values::<f64>("mass")?
        );
    }

    Ok(())
}
