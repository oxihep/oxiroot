//! Real data, read over the network: the Z boson in ATLAS open data.
//!
//! Streams a 966 MB ATLAS open-data simulation sample (Z → ee) from CERN's
//! public EOS server over `root://`, without downloading it: only the
//! branches read, for the entries read, cross the network (batched with
//! `XRootD` vector reads). Selects events with exactly two opposite-charge
//! electrons, builds their invariant mass from the four-momenta, fits the Z
//! peak, and plots it. Needs the network and the
//! `xrootd` feature:
//!
//! ```sh
//! cargo run --release -p oxiroot --features xrootd --example open_data_zee
//! ```

#[cfg(not(all(feature = "xrootd", feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `xrootd` feature: --features xrootd");
}

#[cfg(all(feature = "xrootd", feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    use oxiroot::fit::FitExt;
    use oxiroot::fit::{FitMethod, FitOptions, Minimizer, Model};
    use oxiroot::particle::literals;
    use oxiroot::plot::{Axes, Color, CurveOpts, HistOpts, HistType, Style};
    use oxiroot::prelude::*;

    const URL: &str = "root://eospublic.cern.ch//eos/opendata/atlas/OutreachDatasets/\
                       2016-07-29/MC/mc_147770.Zee.root";
    let entries = 200_000;

    let start = std::time::Instant::now();
    let file = FileReader::open_url(URL)?;
    let tree = Dataset::open(&file, "mini")?;
    println!(
        "opened {:.0} MB file, tree `mini` with {} entries, in {:.1?}",
        file.size() as f64 / 1e6,
        tree.entries(),
        start.elapsed()
    );

    // Exactly two electrons, opposite charge, pT > 25 GeV. The file stores a
    // lepton's kind as the absolute value of its PDG ID: 11 for an electron.
    let electron = literals::electron().pdgid().abspid().unsigned_abs();
    let mut mass = Hist::reg(50, 66.0, 116.0)
        .weight()
        .named("m_ee")
        .titled("dielectron mass");

    // Lepton four-momenta (MeV), charge and type, for the first entries, one
    // cluster at a time: only those baskets are fetched over the network.
    let columns = [
        "lep_pt",
        "lep_eta",
        "lep_phi",
        "lep_E",
        "lep_charge",
        "lep_type",
    ];
    for batch in tree.batches(columns).range(0..entries) {
        let batch = batch?;
        let pt = batch.jagged::<f32>("lep_pt")?;
        let eta = batch.jagged::<f32>("lep_eta")?;
        let phi = batch.jagged::<f32>("lep_phi")?;
        let energy = batch.jagged::<f32>("lep_E")?;
        let charge = batch.jagged::<f32>("lep_charge")?;
        let kind = batch.jagged::<u32>("lep_type")?;
        for i in 0..batch.len() {
            let (kind, q, pt) = (kind.row(i), charge.row(i), pt.row(i));
            if kind.len() != 2 || kind.iter().any(|&t| t != electron) || q[0] == q[1] {
                continue;
            }
            if pt.iter().any(|&p| p < 25e3) {
                continue;
            }
            // Sum the four-momenta: (E, px, py, pz) from (E, pT, η, φ).
            let (mut e, mut px, mut py, mut pz) = (0.0, 0.0, 0.0, 0.0);
            let lepton = pt.iter().zip(energy.row(i)).zip(eta.row(i)).zip(phi.row(i));
            for (((&pt, &en), &eta), &phi) in lepton {
                let p = f64::from(pt);
                e += f64::from(en);
                px += p * f64::from(phi).cos();
                py += p * f64::from(phi).sin();
                pz += p * f64::from(eta).sinh();
            }
            mass.fill((e * e - px * px - py * py - pz * pz).max(0.0).sqrt() / 1e3);
        }
    }
    println!(
        "read 6 lepton branches for {entries} entries in {:.1?}",
        start.elapsed()
    );
    println!("{:.0} selected Z → ee candidates", mass.integral(false));

    // Electrons radiate (bremsstrahlung, final-state photons), so the peak
    // has a long low-mass tail that a simple lineshape does not follow. The
    // usual quick look fits a Gaussian to the core of the peak only, in a
    // window around its maximum, which measures the peak position and the
    // core resolution.
    let peak_bin = (0..mass.values(false).len())
        .max_by(|&a, &b| mass.values(false)[a].total_cmp(&mass.values(false)[b]))
        .expect("bins");
    let peak = 66.5 + peak_bin as f64;
    let model = Model::gaussian("core").with_params(vec![mass.values(false)[peak_bin], peak, 2.5]);
    let opts = FitOptions::new()
        .method(FitMethod::Likelihood)
        .range(peak - 3.5, peak + 3.5);
    let fit = mass.fit_opts(&model, &opts);
    println!(
        "core fit in [{:.1}, {:.1}] GeV: peak {:.2} ± {:.2} GeV, resolution {:.2} ± {:.2} GeV, χ²/ndf = {:.1}/{}",
        peak - 3.5,
        peak + 3.5,
        fit.params[1],
        fit.errors[1],
        fit.params[2],
        fit.errors[2],
        fit.chi2,
        fit.ndf
    );
    // The whole lineshape over [70, 110] GeV: a double-sided Crystal Ball — the
    // Gaussian core with a power-law tail on each side, the low one taking the
    // radiation — on a falling exponential continuum, seeded from the core
    // fit. MIGRAD alone stalls from this seed; the combined minimizer
    // (MIGRAD with a SIMPLEX fallback) converges. The tail powers are capped
    // at 50: beyond that a power law is an exponential, and the likelihood is
    // flat in them.
    let full = Model::double_crystal_ball("Z")
        .with_params(vec![
            fit.params[0],
            fit.params[1],
            fit.params[2],
            1.0,
            3.0,
            1.5,
            5.0,
        ])
        .limit("sigma", 0.5, 10.0)
        .limit("alpha_lo", 0.1, 5.0)
        .limit("n_lo", 1.01, 50.0)
        .limit("alpha_hi", 0.1, 5.0)
        .limit("n_hi", 1.01, 50.0)
        + Model::exponential("continuum").with_params(vec![3.0, -0.05]);
    let wide = FitOptions::new()
        .method(FitMethod::Likelihood)
        .range(70.0, 110.0)
        .minimizer(Minimizer::Minuit2Combined);
    let lineshape = mass.fit_opts(&full, &wide);
    println!(
        "full fit in [70, 110] GeV (double-sided Crystal Ball + exponential): peak {:.2} ± {:.2} GeV, \
         resolution {:.2} ± {:.2} GeV, χ²/ndf = {:.1}/{}{}",
        lineshape.params[1],
        lineshape.errors[1],
        lineshape.params[2],
        lineshape.errors[2],
        lineshape.chi2,
        lineshape.ndf,
        if lineshape.valid {
            ""
        } else {
            " (not converged)"
        }
    );
    let tail: f64 = mass.values(false)[..(peak_bin - 8)].iter().sum();
    println!(
        "{:.0} % of the candidates sit more than 8 GeV below the peak: the radiative tail.",
        100.0 * tail / mass.integral(false)
    );
    println!(
        "(The PDG Z mass is {:.2} GeV; comparing this peak in data and simulation is how",
        literals::z().mass().map_or(f64::NAN, |mev| mev / 1e3)
    );
    println!(" the electron energy scale is calibrated.)");

    let fitted = model.with_params(fit.params.clone());
    let mut ax = Axes::with_style(Style::mplhep());
    ax.hist_with(
        &mass,
        HistOpts::new()
            .histtype(HistType::Errorbar)
            .color(Color::BLACK)
            .label("ATLAS open data (sim.)"),
    );
    ax.model_with(
        &full.with_params(lineshape.params.clone()),
        70.0..110.0,
        CurveOpts::new()
            .color(Color::hex("#3f90da"))
            .samples(400)
            .label("Crystal Ball + continuum"),
    );
    ax.model_with(
        &fitted,
        (peak - 3.5)..(peak + 3.5),
        CurveOpts::new()
            .color(Color::hex("#bd1f01"))
            .label("Gaussian core fit"),
    );
    ax.xlabel("$m_{ee}$ [GeV]")
        .ylabel("Events / GeV")
        .hep_label("Demo", "ATLAS open data")
        .hep_rhs("$\\sqrt{s} = 8$ TeV")
        .legend();
    let path = std::env::temp_dir().join("open_data_zee.png");
    ax.save(&path)?;
    println!("plot: {}  (total {:.1?})", path.display(), start.elapsed());
    Ok(())
}
