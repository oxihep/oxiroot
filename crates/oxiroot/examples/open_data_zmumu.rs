//! Real data against simulation: Z → μμ in ATLAS open data.
//!
//! Reads two files from CERN's public EOS server over `root://`: 2012
//! proton-collision **data** recorded by ATLAS with a muon trigger, and the
//! **simulated** Z → μμ sample that models it. The same selection runs on
//! both, and the results are built in memory and written together:
//!
//! 1. **Read** the lepton branches of each file, one cluster at a time.
//! 2. **Select** two opposite-charge muons above a transverse-momentum
//!    threshold, counting what each cut keeps in a labelled cut-flow
//!    histogram.
//! 3. **Compare**: scale the simulation to the data in the Z peak, subtract
//!    it, and fit the peak in each to compare the muon momentum scale.
//! 4. **Write one file** holding everything: the histograms, the cut flows,
//!    the scale factor, and the selected candidates of each sample as a
//!    `TTree` and as an `RNTuple`, in a directory per sample.
//! 5. **Read it back** and **plot** data over simulation, with their ratio.
//!
//! ```sh
//! cargo run --release -p oxiroot --features xrootd --example open_data_zmumu
//! cargo run --release -p oxiroot --features xrootd --example open_data_zmumu -- 3000000 zmumu.root
//! ```
//!
//! The first argument is the number of events to read from each file (one
//! million by default), the second where to keep the output file.

#[cfg(not(all(feature = "xrootd", feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `xrootd` feature: --features xrootd");
}

#[cfg(all(feature = "xrootd", feature = "fit", feature = "plot"))]
mod analysis {
    use oxiroot::data::DataType;
    use oxiroot::file::Parameter;
    use oxiroot::fit::{FitExt, FitMethod, FitOptions, Model};
    use oxiroot::hist::oxihist::Weight;
    use oxiroot::particle::literals;
    use oxiroot::plot::{
        Binned1D, Color, CurveOpts, ErrorbarOpts, Hatch, HistOpts, HistType, Style, XyPoints,
        ratio_subplots_with,
    };
    use oxiroot::prelude::*;

    const BASE: &str = "root://eospublic.cern.ch//eos/opendata/atlas/OutreachDatasets/2016-07-29";
    const CUTS: [&str; 4] = [
        "all events",
        "two muons",
        "opposite charge",
        "both pT > 25 GeV",
    ];
    const BINS: usize = 60;
    const LO: f64 = 61.0;
    const HI: f64 = 121.0;

    /// What the selection leaves of one sample.
    struct Sample {
        /// The dimuon mass of the selected candidates.
        mass: Histogram<Weight, 1>,
        /// How many events pass each cut, in order.
        cutflow: Hist1D,
        /// One row per selected candidate.
        candidates: Batch,
    }

    /// Run the Z → μμ selection over the first `entries` events of `path`.
    fn select(path: &str, name: &str, entries: u64) -> oxiroot::Result<Sample> {
        let start = std::time::Instant::now();
        let file = FileReader::open_url(&format!("{BASE}/{path}"))?;
        let tree = Dataset::open(&file, "mini")?;
        let entries = entries.min(tree.entries());

        let mut mass = Hist::reg(BINS, LO, HI)
            .weight()
            .named(format!("m_mumu_{name}"));
        // The selection as a query: each cut is a bool column built on the
        // one before, so every event carries how far it got, and the counts
        // of the cut flow are the sums of those columns. The file stores a
        // lepton's kind as |PDG ID|: 13 for a muon. Momenta are in MeV; the
        // mass sums the two four-momenta (E, px, py, pz).
        let muon = literals::muon().pdgid().abspid().unsigned_abs();
        let events = tree
            .query()
            .range(0..entries)
            .define("event", "eventNumber")
            .define(
                "two_muons",
                format!("len(lep_type) == 2 and all(lep_type == {muon})"),
            )
            .define(
                "opposite",
                "(two_muons and lep_charge[0] != lep_charge[1]) ?? false",
            )
            .define("hard", "opposite and all(lep_pt > 25e3)")
            .define(
                "mass",
                "sqrt(max(sum(lep_E)^2 - sum(lep_pt * cos(lep_phi))^2 \
                      - sum(lep_pt * sin(lep_phi))^2 - sum(lep_pt * sinh(lep_eta))^2, 0)) / 1e3",
            )
            .define("pt_lead", "(max(lep_pt) ?? 0) / 1e3")
            .define("pt_sub", "(min(lep_pt) ?? 0) / 1e3")
            .read()?;
        let mut passed = [events.len() as u64, 0, 0, 0];
        for (count, cut) in passed[1..]
            .iter_mut()
            .zip(["two_muons", "opposite", "hard"])
        {
            *count = events.values::<bool>(cut)?.iter().filter(|&&p| p).count() as u64;
        }
        let selected = events
            .filter(events.values::<bool>("hard")?)?
            .select(["event", "mass", "pt_lead", "pt_sub"])?;
        for &m in selected.values::<f64>("mass")? {
            mass.fill(m);
        }
        // Stored in the file as the source has them: the event number in 32
        // bits, and the floats in 32 bits, which is all a mass in GeV needs.
        let candidates = selected
            .cast("event", DataType::I32)?
            .cast("mass", DataType::F32)?
            .cast("pt_lead", DataType::F32)?
            .cast("pt_sub", DataType::F32)?;

        // The cut flow is a histogram with a label on each bin: filled with
        // the counts as weights, then turned into the record ROOT stores,
        // whose axis takes the labels.
        let mut counts = Hist::reg(CUTS.len(), 0.0, CUTS.len() as f64).weight();
        for (bin, &count) in passed.iter().enumerate() {
            counts.fill_weight(bin as f64 + 0.5, count as f64);
        }
        let mut cutflow = counts
            .to_root()?
            .named(format!("cutflow_{name}"))
            .titled("events after each cut");
        for (bin, label) in CUTS.iter().enumerate() {
            cutflow.xaxis_mut().set_label(bin + 1, label)?;
        }
        cutflow.xaxis_mut().set_title("selection step");

        println!(
            "   {name:<4}  {:>4.0} MB file, {entries} events read in {:.1?}: {} candidates",
            file.size() as f64 / 1e6,
            start.elapsed(),
            passed[3],
        );
        Ok(Sample {
            mass,
            cutflow,
            candidates,
        })
    }

    /// The peak position and width of `mass`, from a Gaussian on its core.
    fn peak(mass: &Histogram<Weight, 1>) -> (f64, f64, f64) {
        let top = mass.values(false).iter().copied().fold(0.0, f64::max);
        let model = Model::gaussian("core").with_params(vec![top, 91.0, 2.5]);
        let opts = FitOptions::new()
            .method(FitMethod::Likelihood)
            .range(87.0, 95.0);
        let fit = mass.fit_opts(&model, &opts);
        (fit.params[1], fit.errors[1], fit.params[2].abs())
    }

    pub fn run() -> oxiroot::Result<()> {
        let mut args = std::env::args().skip(1);
        let entries: u64 = args
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1_000_000);
        let out_path = args.next().map_or_else(
            || std::env::temp_dir().join("open_data_zmumu.root"),
            std::path::PathBuf::from,
        );
        let start = std::time::Instant::now();

        // ---- 1 and 2. The same selection on data and on simulation ---------
        println!("1. Reading and selecting:");
        let data = select("Data/DataMuons.root", "data", entries)?;
        let mut sim = select("MC/mc_147771.Zmumu.root", "sim", entries)?;

        println!("2. Cut flow (events, and the fraction of the step before):");
        println!("   {:<18} {:>20} {:>20}", "", "data", "simulation");
        for (bin, label) in data.cutflow.xaxis().labels().iter().enumerate() {
            let cell = |h: &Hist1D| {
                let (now, before) = (h.contents()[bin + 1], h.contents()[bin.max(1)]);
                format!("{now:>10.0} ({:>5.1} %)", 100.0 * now / before)
            };
            println!(
                "   {label:<18} {:>20} {:>20}",
                cell(&data.cutflow),
                cell(&sim.cutflow)
            );
        }

        // ---- 3. Compare ------------------------------------------------------
        // The simulation holds only Z → μμ, at an arbitrary luminosity: scale
        // it to the data in the peak.
        let window = |h: &Histogram<Weight, 1>| -> f64 {
            h.values(false)[20..40].iter().sum() // 81 to 101 GeV
        };
        let scale = window(&data.mass) / window(&sim.mass);
        // The peak of each, fitted on the events as they were counted.
        let (peak_data, err_data, width_data) = peak(&data.mass);
        let (peak_sim, err_sim, width_sim) = peak(&sim.mass);
        sim.mass.scale(scale);

        // What the Z → μμ simulation does not describe: data minus simulation,
        // bin by bin, as a ROOT histogram.
        let data_hist = data.mass.to_root()?;
        let sim_hist = sim.mass.to_root()?;
        let mut excess = data_hist.clone().named("data_minus_sim");
        excess.add_scaled(&sim_hist, -1.0)?;
        let outside: f64 = excess.contents()[1..=10].iter().sum();
        println!("3. Simulation scaled by {scale:.4} to the data between 81 and 101 GeV.");
        let z = literals::z();
        println!(
            "   PDG:     {} mass {:.3} GeV, natural width {:.3} GeV",
            z.name(),
            z.mass().map_or(f64::NAN, |mev| mev / 1e3),
            z.width().map_or(f64::NAN, |mev| mev / 1e3),
        );
        println!(
            "   Z peak:  data {peak_data:.3} ± {err_data:.3} GeV (width {width_data:.2}),  \
             simulation {peak_sim:.3} ± {err_sim:.3} GeV (width {width_sim:.2})"
        );
        println!(
            "   momentum scale, data / simulation: {:.4} ± {:.4}",
            peak_data / peak_sim,
            (err_data.hypot(err_sim)) / peak_sim
        );
        println!(
            "   below 71 GeV the data hold {outside:.0} events more than the Z simulation: \
             other processes."
        );

        // ---- 4. One file -----------------------------------------------------
        // Everything is built; now it is written, each kind of object the same
        // way. The candidates go in as a `TTree` for the data and an `RNTuple`
        // for the simulation, to show that only one argument differs.
        let data_table = NamedBatch::new("candidates", Format::Tree, data.candidates);
        let sim_table = NamedBatch::new("candidates", Format::Ntuple, sim.candidates);
        FileWriter::create(&out_path)
            .add(&excess)
            .add(&Parameter::f64(scale).named("sim_scale"))
            .add(&Parameter::i64(i64::try_from(entries).unwrap_or(i64::MAX)).named("events_read"))
            .dir("data", |d| {
                d.add(&data_hist).add(&data.cutflow).add(&data_table)
            })
            .dir("sim", |d| {
                d.add(&sim_hist).add(&sim.cutflow).add(&sim_table)
            })
            .write()?;
        println!(
            "4. Wrote {} ({:.0} kB): 5 histograms, 2 parameters and 2 datasets in 2 directories.",
            out_path.display(),
            std::fs::metadata(&out_path)?.len() as f64 / 1e3
        );

        // ---- 5. Read it back -------------------------------------------------
        let file = FileReader::open(&out_path)?;
        for sample in ["data", "sim"] {
            let table = Dataset::open(&file, &format!("{sample}/candidates"))?;
            let hist = file.get::<Hist1D>(&format!("{sample}/m_mumu_{sample}"))?;
            let flow = file.get::<Hist1D>(&format!("{sample}/cutflow_{sample}"))?;
            let masses = table.read(["mass"])?;
            let mean = masses
                .values::<f32>("mass")?
                .iter()
                .map(|&m| f64::from(m))
                .sum::<f64>()
                / masses.len() as f64;
            println!(
                "5. {sample:<4}: a {:?} of {} candidates (mean mass {mean:.2} GeV), a histogram \
                 of {} bins, and a cut flow ending at \"{}\".",
                table.format(),
                table.entries(),
                hist.xaxis().nbins(),
                flow.xaxis().bin_label(CUTS.len()).unwrap_or("?"),
            );
        }
        assert_eq!(file.get::<Parameter>("sim_scale")?.value().as_f64(), scale);

        // ---- The plot: data over simulation, and their ratio -----------------
        // The ATLAS look: the label inside the frame, and room made for it.
        let (fig, mut main, mut lower) = ratio_subplots_with(Style::atlas());
        main.hist_with(
            &sim.mass,
            HistOpts::new()
                .histtype(HistType::Fill)
                .color(Color::hex("#3f90da"))
                .label("$Z \\rightarrow \\mu\\mu$ simulation"),
        );
        main.hist_with(
            &data.mass,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                // Counts carry Poisson (Garwood) intervals, not ±√n.
                .poisson_errors(true)
                .color(Color::BLACK)
                .label("ATLAS data, 2012"),
        );
        main.ylabel("Events / GeV")
            .hep_label("ATLAS", "Open Data")
            .hep_rhs("$\\sqrt{s} = 8$ TeV")
            .legend();

        // The ratio carries the data's uncertainty; the simulation's own is
        // the hatched band around one.
        lower.hist_with(
            &Binned1D::relative_uncertainty(&sim.mass),
            HistOpts::new()
                .histtype(HistType::Band)
                .hatch(Hatch::Diagonal)
                .color(Color::hex("#555555")),
        );
        lower.errorbar_with(
            &XyPoints::ratio(&data.mass, &sim.mass)?,
            ErrorbarOpts::new().color(Color::BLACK),
        );
        lower.axhline_with(1.0, CurveOpts::new().dashed(vec![4.0, 3.0]));
        lower
            .ylim(0.6..1.6)
            .yticks(&[0.8, 1.0, 1.2, 1.4])
            .ylabel("Data / sim.")
            .xlabel("$m_{\\mu\\mu}$ [GeV]");
        let png = std::env::temp_dir().join("open_data_zmumu.png");
        fig.ratio(main, lower).save(&png)?;
        println!(
            "   Plot: {}  (total {:.1?})",
            png.display(),
            start.elapsed()
        );
        Ok(())
    }
}

#[cfg(all(feature = "xrootd", feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    analysis::run()
}
