//! Real collision data, start to finish: the dimuon spectrum of CMS.
//!
//! Streams events of the CMS 2012 `DoubleMuParked` open-data sample (29 million
//! proton collisions in a 3.5 GB `NanoAOD`-style `TTree`) from CERN's public
//! EOS server over `root://`, and does what an analysis does with them:
//!
//! 1. **Read** only the muon branches, one cluster at a time, as flat columns.
//! 2. **Select** events with exactly two muons of opposite charge and build
//!    their invariant mass.
//! 3. **Skim**: write the selected pairs to a new file as an `RNTuple`, one
//!    cluster at a time, with the angles in half precision. The spectrum and
//!    the event counts are stored *in the same file*, next to the dataset.
//! 4. **Read the skim back** and measure the J/ψ, the Υ(1S) and the Z from it.
//! 5. **Plot** the spectrum, from the ρ/ω up to the Z.
//!
//! Nothing is held in memory but one cluster: the input is larger than this
//! example ever reads at once, and so may the skim be.
//!
//! ```sh
//! cargo run --release -p oxiroot --features xrootd --example open_data_dimuon
//! cargo run --release -p oxiroot --features xrootd --example open_data_dimuon -- 10000000 skim.root
//! ```
//!
//! The first argument is the number of events to read (2 million by default,
//! about half a minute on a home connection), the second where to keep the
//! skim.

#[cfg(not(all(feature = "xrootd", feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `xrootd` feature: --features xrootd");
}

#[cfg(all(feature = "xrootd", feature = "fit", feature = "plot"))]
#[allow(
    clippy::cast_possible_truncation,
    reason = "a mass of a few hundred GeV at most, stored in an f32 column"
)]
fn main() -> oxiroot::Result<()> {
    use oxiroot::data::DataType;
    use oxiroot::file::Parameter;
    use oxiroot::fit::{FitExt, FitMethod, FitOptions, Model};
    use oxiroot::particle::literals;
    use oxiroot::plot::{Axes, Color, HistOpts, HistType, Scale, Style};
    use oxiroot::prelude::*;

    const URL: &str = "root://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod/\
                       Run2012B_DoubleMuParked.root";

    let mut args = std::env::args().skip(1);
    let wanted: u64 = args
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(2_000_000);
    let skim_path = args.next().map_or_else(
        || std::env::temp_dir().join("open_data_dimuon_skim.root"),
        std::path::PathBuf::from,
    );

    // ---- 1. Open: the header and the tree's metadata, nothing else -------
    let start = std::time::Instant::now();
    let file = FileReader::open_url(URL)?;
    let events = Dataset::open(&file, "Events")?;
    let wanted = wanted.min(events.entries());
    println!(
        "1. Opened a {:.1} GB file: `Events` holds {} collisions in {} columns ({:.1?}).",
        file.size() as f64 / 1e9,
        events.entries(),
        events.columns().len(),
        start.elapsed()
    );

    // The spectrum spans three decades, so its bins are logarithmic: 60 per
    // decade from 0.25 to 300 GeV.
    let edges: Vec<f64> = (0..=185)
        .map(|i| 0.25 * 10f64.powf(f64::from(i) / 60.0))
        .collect();
    let mut spectrum = Hist::var(&edges)
        .label("$m_{\\mu\\mu}$ [GeV]")
        .weight()
        .named("m_mumu")
        .titled("opposite-charge dimuon mass");

    // ---- 2 and 3. Select, and skim as we go ------------------------------
    // The skim's columns, and how they are stored: the angles do not need 32
    // bits, so they go in half precision.
    let pairs = |run: Vec<i32>, event: Vec<u64>, kin: [Vec<f32>; 7]| -> oxiroot::Result<Batch> {
        let [mass, pt_1, pt_2, eta_1, eta_2, phi_1, phi_2] = kin;
        Ok(Batch::new()
            .column("run", run)?
            .column("event", event)?
            .column("mass", mass)?
            .column("pt_1", pt_1)?
            .column("pt_2", pt_2)?
            .column("eta_1", eta_1)?
            .column("eta_2", eta_2)?
            .column("phi_1", phi_1)?
            .column("phi_2", phi_2)?)
    };
    let mut schema = pairs(Vec::new(), Vec::new(), Default::default())?.schema();
    for angle in ["eta_1", "eta_2", "phi_1", "phi_2"] {
        schema = schema.with(angle, DataType::Half);
    }
    let mut skim =
        DatasetWriter::create(&skim_path, "dimuons", Format::Ntuple)?.with_schema(schema);

    let columns = [
        "run",
        "event",
        "Muon_pt",
        "Muon_eta",
        "Muon_phi",
        "Muon_mass",
        "Muon_charge",
    ];
    let (mut read, mut clusters) = (0_u64, 0_u32);
    for batch in events.batches(columns).range(0..wanted) {
        let batch = batch?;
        let run = batch.values::<i32>("run")?;
        let event = batch.values::<u64>("event")?;
        let pt = batch.jagged::<f32>("Muon_pt")?;
        let eta = batch.jagged::<f32>("Muon_eta")?;
        let phi = batch.jagged::<f32>("Muon_phi")?;
        let m = batch.jagged::<f32>("Muon_mass")?;
        let charge = batch.jagged::<i32>("Muon_charge")?;

        let (mut runs, mut ids) = (Vec::new(), Vec::new());
        let mut kin: [Vec<f32>; 7] = Default::default();
        for i in 0..batch.len() {
            let q = charge.row(i);
            if q.len() != 2 || q[0] == q[1] {
                continue;
            }
            // The pair's four-momentum from each muon's (pT, η, φ, m).
            let (mut e, mut px, mut py, mut pz) = (0.0_f64, 0.0, 0.0, 0.0);
            for k in 0..2 {
                let (pt, eta, phi) = (
                    f64::from(pt.row(i)[k]),
                    f64::from(eta.row(i)[k]),
                    f64::from(phi.row(i)[k]),
                );
                let (x, y, z) = (pt * phi.cos(), pt * phi.sin(), pt * eta.sinh());
                let mass = f64::from(m.row(i)[k]);
                e += (x * x + y * y + z * z + mass * mass).sqrt();
                px += x;
                py += y;
                pz += z;
            }
            let mass = (e * e - px * px - py * py - pz * pz).max(0.0).sqrt();
            spectrum.fill(mass);

            runs.push(run[i]);
            ids.push(event[i]);
            let row = [
                mass as f32,
                pt.row(i)[0],
                pt.row(i)[1],
                eta.row(i)[0],
                eta.row(i)[1],
                phi.row(i)[0],
                phi.row(i)[1],
            ];
            for (column, value) in kin.iter_mut().zip(row) {
                column.push(value);
            }
        }
        read += batch.len() as u64;
        clusters += 1;
        skim.append(pairs(runs, ids, kin)?)?;
    }
    let selected = skim.entries();
    println!(
        "2. Read 7 branches of {read} events in {clusters} clusters: {selected} opposite-charge \
         pairs ({:.0} %), in {:.1?}.",
        100.0 * selected as f64 / read as f64,
        start.elapsed()
    );

    // The spectrum and the bookkeeping go in the skim's own file: a
    // histogram and two parameters, next to the dataset, written in the same
    // pass.
    let spectrum_root = spectrum.to_root()?;
    skim.add(&spectrum_root);
    skim.add(&Parameter::i64(i64::try_from(read).unwrap_or(i64::MAX)).named("events_read"));
    skim.add(&Parameter::i64(i64::try_from(selected).unwrap_or(i64::MAX)).named("pairs_selected"));
    skim.finish()?;
    println!(
        "3. Skim written: {} ({:.1} MB for {selected} pairs, {:.1} bytes each).",
        skim_path.display(),
        std::fs::metadata(&skim_path)?.len() as f64 / 1e6,
        std::fs::metadata(&skim_path)?.len() as f64 / selected.max(1) as f64
    );

    // ---- 4. Back from the skim: three resonances -------------------------
    let skim_file = FileReader::open(&skim_path)?;
    let dimuons = Dataset::open(&skim_file, "dimuons")?;
    let stored = skim_file.get::<Hist1D>("m_mumu")?;
    let counted = skim_file.get::<Parameter>("pairs_selected")?;
    assert_eq!(dimuons.entries(), selected);
    assert_eq!(stored.entries(), selected as f64);
    assert_eq!(counted.value().as_f64(), selected as f64);
    println!(
        "4. Read back: an {:?} of {} pairs, with the spectrum and the counts beside it. \
         `eta_1` is stored as {}.",
        dimuons.format(),
        dimuons.entries(),
        dimuons.schema()?.get("eta_1").expect("a skim column"),
    );

    // One pass over the skim's mass column fills a fine histogram around each
    // peak; a Gaussian on a straight line then measures it.
    let peaks = [
        (literals::jpsi(), "J/ψ", 2.9, 3.3, 0.03),
        (literals::upsilon_1s(), "Υ(1S)", 9.1, 9.8, 0.09),
        (literals::z(), "Z", 81.0, 101.0, 2.5),
    ];
    let mut windows: Vec<_> = peaks
        .iter()
        .map(|&(_, _, lo, hi, _)| Hist::reg(40, lo, hi).weight())
        .collect();
    for batch in dimuons.batches(["mass"]) {
        for &mass in batch?.values::<f32>("mass")? {
            for window in &mut windows {
                window.fill(f64::from(mass));
            }
        }
    }
    println!("   resonance   candidates   fitted mass [GeV]      PDG [GeV]   width [GeV]");
    for (&(particle, label, lo, hi, width), window) in peaks.iter().zip(&windows) {
        let height = window.values(false).iter().copied().fold(0.0, f64::max);
        let floor = window.values(false)[0];
        let model =
            Model::gaussian("peak").with_params(vec![height - floor, 0.5 * (lo + hi), width])
                + Model::polynomial("continuum", 1).with_params(vec![floor, 0.0]);
        let fit = window.fit_opts(&model, &FitOptions::new().method(FitMethod::Likelihood));
        // The world average, from the bundled PDG table (in MeV).
        let pdg = particle.mass().map_or(f64::NAN, |mev| mev / 1e3);
        println!(
            "   {label:<9}   {:>10.0}   {:>8.4} ± {:.4}   {pdg:>12.4}   {:>11.4}",
            window.integral(false),
            fit.params[1],
            fit.errors[1],
            fit.params[2].abs(),
        );
    }

    // ---- 5. The spectrum ---------------------------------------------------
    // The bins are logarithmic, each 3.9 % wide, so a flat line is a
    // spectrum falling as 1/m.
    let mut ax = Axes::with_style(Style::mplhep());
    ax.hist_with(
        &spectrum,
        HistOpts::new()
            .histtype(HistType::Step)
            .color(Color::BLACK)
            .label("CMS open data, 2012"),
    );
    ax.xscale(Scale::Log)
        .yscale(Scale::Log)
        .xlabel("$m_{\\mu\\mu}$ [GeV]")
        .ylabel("Pairs / bin (60 bins per decade)")
        .hep_label("CMS", "Open Data")
        .hep_rhs("$\\sqrt{s} = 8$ TeV")
        .legend();
    let png = std::env::temp_dir().join("open_data_dimuon.png");
    ax.save(&png)?;
    println!(
        "5. Plot: {}  (total {:.1?})",
        png.display(),
        start.elapsed()
    );
    Ok(())
}
