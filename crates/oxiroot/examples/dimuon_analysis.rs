//! An end-to-end dimuon analysis, the shape of a real one on CMS `NanoAOD`:
//! write events with per-event muon collections to a `TTree`, read them
//! back, select opposite-sign muon pairs, histogram their invariant mass, fit
//! the Z peak over the Drell–Yan continuum, plot it, and save the histogram.
//!
//! The events are toys (Z → μμ, a Drell–Yan continuum, and fake muons), but
//! the file layout is `NanoAOD`'s: jagged branches `Muon_pt`, `Muon_eta`,
//! `Muon_phi`, `Muon_charge`, one entry per muon. Run with:
//!
//! ```sh
//! cargo run -p oxiroot --example dimuon_analysis
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
#[allow(
    clippy::cast_possible_truncation,
    reason = "generated values stored in f32 branches"
)]
fn main() -> oxiroot::Result<()> {
    use oxiroot::fit::{FitMethod, FitOptions, Model};
    use oxiroot::plot::{Axes, Color, CurveOpts, HistOpts, HistType, Style};
    use oxiroot::prelude::*;

    let dir = std::env::temp_dir().join("oxiroot-dimuon");
    std::fs::create_dir_all(&dir)?;

    // --- 1. Generate and write the events. --------------------------------
    let mut toy = Toy(2026);
    let n_events = 100_000;
    let mut pt: Vec<Vec<f32>> = Vec::new();
    let mut eta: Vec<Vec<f32>> = Vec::new();
    let mut phi: Vec<Vec<f32>> = Vec::new();
    let mut charge: Vec<Vec<i32>> = Vec::new();
    for _ in 0..n_events {
        let mut muons: Vec<(f64, f64, f64, i32)> = Vec::new();
        let r = toy.uniform();
        if r < 0.6 {
            // Z → μμ, or a Drell–Yan pair from the falling continuum.
            let m = if r < 0.35 {
                91.1876 + 1.2476 * (std::f64::consts::PI * (toy.uniform() - 0.5)).tan()
            } else {
                40.0 - 30.0 * toy.uniform().ln()
            };
            let y = toy.gauss(0.0, 1.5);
            let [a, b] = decay(&mut toy, m, y);
            muons.push((a.0, a.1, a.2, -1));
            muons.push((b.0, b.1, b.2, 1));
        }
        // Fake or non-prompt muons (from heavy-flavour decays), soft and
        // uncorrelated.
        while toy.uniform() < 0.3 {
            let sign = if toy.uniform() < 0.5 { -1 } else { 1 };
            muons.push((
                5.0 - 8.0 * toy.uniform().ln(),
                toy.gauss(0.0, 1.5),
                toy.phi(),
                sign,
            ));
        }
        // Detector: 2 % pT resolution, |η| < 2.4 acceptance, pT-ordered.
        let mut kept: Vec<_> = muons
            .into_iter()
            .map(|(p, e, f, q)| (p * toy.gauss(1.0, 0.02), e, f, q))
            .filter(|m| m.1.abs() < 2.4 && m.0 > 3.0)
            .collect();
        kept.sort_by(|a, b| b.0.total_cmp(&a.0));
        pt.push(kept.iter().map(|m| m.0 as f32).collect());
        eta.push(kept.iter().map(|m| m.1 as f32).collect());
        phi.push(kept.iter().map(|m| m.2 as f32).collect());
        charge.push(kept.iter().map(|m| m.3).collect());
    }
    let tree_path = dir.join("nano.root");
    let muons = Batch::new()
        .column("Muon_pt", pt)?
        .column("Muon_eta", eta)?
        .column("Muon_phi", phi)?
        .column("Muon_charge", charge)?;
    let mut out = DatasetWriter::create(&tree_path, "Events", Format::Tree)?;
    out.write(muons)?;
    out.finish()?;
    println!("wrote {n_events} events to {}", tree_path.display());

    // --- 2. Read them back: flat columns, one view per entry. -----------
    let file = FileReader::open(&tree_path)?;
    let events = Dataset::open(&file, "Events")?;
    let data = events.read(["Muon_pt", "Muon_eta", "Muon_phi", "Muon_charge"])?;
    let pt = data.jagged::<f32>("Muon_pt")?;
    let eta = data.jagged::<f32>("Muon_eta")?;
    let phi = data.jagged::<f32>("Muon_phi")?;
    let charge = data.jagged::<i32>("Muon_charge")?;

    // --- 3. Select: the two leading muons, pT > 25 and 15 GeV, opposite sign.
    let mut mass = Hist::reg(60, 60.0, 120.0)
        .double()
        .to_root()?
        .named("m_mumu")
        .titled("dimuon mass");
    mass.track_sumw2();
    let mut selected = 0;
    for i in 0..pt.len() {
        let (pt, eta, phi, q) = (pt.row(i), eta.row(i), phi.row(i), charge.row(i));
        if pt.len() < 2 || pt[0] < 25.0 || pt[1] < 15.0 || q[0] == q[1] {
            continue;
        }
        let m = (2.0
            * f64::from(pt[0])
            * f64::from(pt[1])
            * (f64::from(eta[0] - eta[1]).cosh() - f64::from(phi[0] - phi[1]).cos()))
        .sqrt();
        mass.fill(m);
        selected += 1;
    }
    println!("{selected} events with an opposite-sign pair passing the pT cuts");

    // --- 4. Fit: background from the low sideband, then Voigt + exponential.
    let sideband = mass.fit_opts(
        &Model::exponential("continuum").with_params(vec![5.0, -0.05]),
        &FitOptions::new()
            .method(FitMethod::Likelihood)
            .range(60.0, 78.0),
    );
    let (b0, b1) = (sideband.params[0], sideband.params[1]);
    let bin_width = 1.0;
    // Seed the Z yield from the data: events in 81–101 GeV minus the
    // sideband background extrapolated under the peak.
    let in_window: f64 = mass.values()[21..41].iter().sum();
    let background: f64 = (21..41)
        .map(|b| (b0 + b1 * (60.5 + f64::from(b))).exp())
        .sum();
    let model = Model::new(
        "Z → μμ + Drell–Yan",
        &["N", "mass", "resolution", "half-width", "b0", "b1"],
        vec![
            (in_window - background).max(1.0) * bin_width,
            91.0,
            2.0,
            1.2476,
            b0,
            b1,
        ],
        |x, p| p[0] * oxiroot::stat::voigtian(x, p[1], p[2], p[3]) + (p[4] + p[5] * x).exp(),
    )
    .fix("half-width") // Γ_Z / 2: the natural width is known
    .lower_limit("resolution", 0.1);
    let fit = mass.fit_opts(&model, &FitOptions::new().method(FitMethod::Likelihood));
    println!(
        "sideband continuum exp({b0:.2} {b1:+.4}·m); seed {:.0} Z events",
        in_window - background
    );
    println!(
        "Z mass {:.3} ± {:.3} GeV, resolution {:.2} ± {:.2} GeV, {:.0} ± {:.0} Z events; χ²/ndf = {:.1}/{}",
        fit.params[1],
        fit.errors[1],
        fit.params[2],
        fit.errors[2],
        fit.params[0] / bin_width,
        fit.errors[0] / bin_width,
        fit.chi2,
        fit.ndf
    );

    // --- 5. Plot data and fit; save the histogram for later. --------------
    let fitted = model.with_params(fit.params.clone());
    let mut ax = Axes::with_style(Style::mplhep());
    ax.hist_with(
        &mass,
        HistOpts::new()
            .histtype(HistType::Errorbar)
            .color(Color::BLACK)
            .label("Toy data"),
    );
    ax.model_with(
        &fitted,
        60.0..120.0,
        CurveOpts::new()
            .color(Color::hex("#bd1f01"))
            .label("Voigt + exponential"),
    );
    ax.xlabel("$m_{\\mu\\mu}$ [GeV]")
        .ylabel("Events / GeV")
        .hep_label("Demo", "Simulation")
        .legend();
    let plot_path = dir.join("m_mumu.png");
    ax.save(&plot_path)?;
    let hist_path = dir.join("m_mumu.root");
    oxiroot::FileWriter::create(&hist_path).add(&mass).write()?;
    println!(
        "plot: {}\nhistogram: {}",
        plot_path.display(),
        hist_path.display()
    );
    Ok(())
}

/// A small reproducible generator.
#[cfg(all(feature = "fit", feature = "plot"))]
struct Toy(u64);

#[cfg(all(feature = "fit", feature = "plot"))]
impl Toy {
    fn uniform(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn gauss(&mut self, mean: f64, sigma: f64) -> f64 {
        let (u1, u2) = (self.uniform(), self.uniform());
        mean + sigma * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    fn phi(&mut self) -> f64 {
        std::f64::consts::PI * (2.0 * self.uniform() - 1.0)
    }
}

/// Two massless muons from a resonance of mass `m` at rapidity `y` (no
/// transverse boost), as (pT, η, φ).
#[cfg(all(feature = "fit", feature = "plot"))]
fn decay(toy: &mut Toy, m: f64, y: f64) -> [(f64, f64, f64); 2] {
    let cos_theta = 2.0 * toy.uniform() - 1.0;
    let phi = toy.phi();
    let pt = m / 2.0 * (1.0 - cos_theta * cos_theta).sqrt();
    let eta = cos_theta.atanh();
    let back = if phi > 0.0 {
        phi - std::f64::consts::PI
    } else {
        phi + std::f64::consts::PI
    };
    [(pt, eta + y, phi), (pt, -eta + y, back)]
}
