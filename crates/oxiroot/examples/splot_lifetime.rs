//! A B⁰ lifetime from sWeights, from a `TTree` to the plots: separate the
//! signal from the background in the invariant mass, then measure the
//! signal's decay-time distribution alone, with no model of the background's
//! decay times.
//!
//! 1. **Write** the candidates (mass, decay time) to a `TTree` and **read**
//!    them back.
//! 2. **Fit the mass** (extended unbinned: a Gaussian peak on a linear
//!    background) for the yields.
//! 3. **Compute sWeights** (sPlot) and **write** them as a second tree, one
//!    entry per candidate — a friend of the first.
//! 4. **Fit the lifetime** to the sWeighted decay times with the signal model
//!    alone, with three error estimates: naive, `SumW2`, and the asymptotically
//!    correct one (Langenbruch).
//! 5. **Check the errors with toys** of the whole procedure: only the
//!    asymptotic ones give pulls of unit width.
//! 6. **Plot** the mass fit, the sWeighted decay times with the fit, the
//!    sWeights against mass, and the toys' pulls.
//!
//! Run with (a few seconds in `--release`):
//!
//! ```sh
//! cargo run --release -p oxiroot --example splot_lifetime
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    splot::run()
}

#[cfg(all(feature = "fit", feature = "plot"))]
mod splot {
    use oxiroot::fit::Events;
    use oxiroot::fit::FitExt;
    use oxiroot::fit::FitResult;
    use oxiroot::fit::{FitMethod, FitOptions, Model, SWeights};
    use oxiroot::hist::oxihist::Weight;
    use oxiroot::plot::{Axes, Color, CurveOpts, HistOpts, HistType, Scale, Style};
    use oxiroot::prelude::*;

    const M_LO: f64 = 5.18;
    const M_HI: f64 = 5.38;
    const M_B: f64 = 5.2797;
    const RESOLUTION: f64 = 0.018;
    /// Decay times (ps) in [0, 12): the B⁰ lifetime and the background's.
    const T_HI: f64 = 12.0;
    const TAU_B: f64 = 1.519;
    const TAU_BKG: f64 = 0.6;
    const N_SIG: usize = 1_500;
    const N_BKG: usize = 4_000;
    /// Mass and decay-time histogram binnings.
    const M_BINS: usize = 40;
    const T_BINS: usize = 48;

    /// A small reproducible generator.
    struct Rng(u64);

    impl Rng {
        fn uniform(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        }
        fn gauss(&mut self) -> f64 {
            let (u1, u2) = (self.uniform(), self.uniform());
            (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
        }
        fn decay(&mut self, tau: f64) -> f64 {
            loop {
                let t = -tau * self.uniform().ln();
                if t < T_HI {
                    return t;
                }
            }
        }
        /// A falling linear background in mass, by accept–reject.
        fn background_mass(&mut self) -> f64 {
            loop {
                let m = M_LO + (M_HI - M_LO) * self.uniform();
                if self.uniform() < 1.0 - 1.5 * (m - M_LO) {
                    return m;
                }
            }
        }
    }

    /// A sample: `(mass, decay time)` per candidate, signal first.
    fn sample(seed: u64) -> (Vec<f64>, Vec<f64>) {
        let mut rng = Rng(seed);
        let (mut m, mut t) = (Vec::new(), Vec::new());
        while m.len() < N_SIG {
            let x = M_B + RESOLUTION * rng.gauss();
            if (M_LO..M_HI).contains(&x) {
                m.push(x);
                t.push(rng.decay(TAU_B));
            }
        }
        for _ in 0..N_BKG {
            m.push(rng.background_mass());
            t.push(rng.decay(TAU_BKG));
        }
        (m, t)
    }

    fn gauss(m: f64, mean: f64, sigma: f64) -> f64 {
        (-0.5 * ((m - mean) / sigma).powi(2)).exp() / (sigma * std::f64::consts::TAU.sqrt())
    }

    /// The linear background density on the mass range, slope `s` (unit area).
    fn linear(m: f64, s: f64) -> f64 {
        (1.0 + s * (m - 0.5 * (M_LO + M_HI))) / (M_HI - M_LO)
    }

    /// The extended mass model: yields, mean, resolution, slope.
    fn mass_model() -> Model {
        Model::new(
            "mass",
            &["n_sig", "n_bkg", "mean", "sigma", "slope"],
            vec![1000.0, 4000.0, 5.28, 0.02, -3.0],
            |m, p| p[0] * gauss(m, p[2], p[3]) + p[1] * linear(m, p[4]),
        )
        .limit("sigma", 0.005, 0.05)
        .limit("slope", -9.9, 9.9)
    }

    fn mass_fit(masses: &[f64]) -> FitResult {
        Events::new(masses).fit_opts(
            &mass_model(),
            &FitOptions::new()
                .method(FitMethod::ExtendedUnbinnedLikelihood)
                .range(M_LO, M_HI),
        )
    }

    fn sweights(masses: &[f64], fit: &FitResult) -> SWeights {
        let (mean, sigma, slope) = (fit.params[2], fit.params[3], fit.params[4]);
        let signal = Model::new("sig", &[], vec![], move |m, _| gauss(m, mean, sigma));
        let background = Model::new("bkg", &[], vec![], move |m, _| linear(m, slope));
        SWeights::compute(
            &Events::new(masses),
            &[&signal, &background],
            &fit.params[..2],
            &FitOptions::new().range(M_LO, M_HI),
        )
        .expect("sWeights")
    }

    /// The weighted lifetime fit, `exp(−t/τ)` normalized on [0, 12).
    fn lifetime_fit(times: &[f64], weights: &[f64], opts: FitOptions) -> FitResult {
        let model = Model::new("decay", &["tau"], vec![1.3], |t, p| (-t / p[0]).exp())
            .limit("tau", 0.2, 6.0);
        Events::weighted(times, weights)
            .expect("same lengths")
            .fit_opts(
                &model,
                &opts.method(FitMethod::UnbinnedLikelihood).range(0.0, T_HI),
            )
    }

    /// A ROOT histogram of `values` (weighted when `weights` is given).
    fn histogram(
        name: &str,
        title: &str,
        (n, lo, hi): (usize, f64, f64),
        values: &[f64],
        weights: Option<&[f64]>,
    ) -> Histogram<Weight, 1> {
        let mut h = Hist::reg(n, lo, hi).weight().named(name).titled(title);
        match weights {
            Some(w) => values
                .iter()
                .zip(w)
                .for_each(|(&v, &w)| h.fill_weight(v, w)),
            None => values.iter().for_each(|&v| h.fill(v)),
        }
        h
    }

    pub(super) fn run() -> oxiroot::Result<()> {
        let start = std::time::Instant::now();
        let dir = std::env::temp_dir().join("oxiroot-splot");
        std::fs::create_dir_all(&dir)?;

        // ---- 1. The candidates ---------------------------------------------
        let (m, t) = sample(2026);
        let tree_path = dir.join("b0_candidates.root");
        let mut out = DatasetWriter::create(&tree_path, "b0", Format::Tree)?;
        out.write(Batch::new().column("mass", m)?.column("time", t)?)?;
        out.finish()?;
        let file = FileReader::open(&tree_path)?;
        let tree = Dataset::open(&file, "b0")?;
        let data = tree.read(["mass", "time"])?;
        let (m, t) = (
            data.values::<f64>("mass")?.to_vec(),
            data.values::<f64>("time")?.to_vec(),
        );
        println!(
            "1. Wrote and read back {} B⁰ candidates — {}",
            tree.entries(),
            tree_path.display()
        );

        // ---- 2. The mass fit -------------------------------------------------
        let fit = mass_fit(&m);
        println!(
            "\n2. Mass fit: signal {:.0} ± {:.0} (truth {N_SIG}), background {:.0} ± {:.0} \
             (truth {N_BKG}), resolution {:.4} GeV",
            fit.params[0], fit.errors[0], fit.params[1], fit.errors[1], fit.params[3]
        );

        // ---- 3. sWeights, as a friend tree ------------------------------------
        let sw = sweights(&m, &fit);
        let (w_sig, w_bkg) = (sw.weights(0).to_vec(), sw.weights(1).to_vec());
        let sw_path = dir.join("b0_sweights.root");
        let mut out = DatasetWriter::create(&sw_path, "sweights", Format::Tree)?;
        out.write(
            Batch::new()
                .column("n_sig_sw", w_sig.clone())?
                .column("n_bkg_sw", w_bkg.clone())?,
        )?;
        out.finish()?;
        println!(
            "\n3. sWeights: signal weights sum to {:.1}, {} negative — written to {}",
            w_sig.iter().sum::<f64>(),
            w_sig.iter().filter(|&&x| x < 0.0).count(),
            sw_path.display()
        );

        // ---- 4. The lifetime ---------------------------------------------------
        let variants = [
            ("naive", FitOptions::new()),
            ("SumW2", FitOptions::new().sumw2(true)),
            ("asymptotic", FitOptions::new().asymptotic_errors(true)),
        ];
        println!("\n4. Signal lifetime from the sWeighted decay times (truth {TAU_B} ps)");
        let fits: Vec<FitResult> = variants
            .iter()
            .map(|(_, opts)| lifetime_fit(&t, &w_sig, *opts))
            .collect();
        for ((label, _), f) in variants.iter().zip(&fits) {
            println!(
                "   τ = {:.4} ± {:.4} ps   [{label}]",
                f.params[0], f.errors[0]
            );
        }

        // ---- 5. Toys of the whole procedure ----------------------------------
        let ntoys = 150;
        let mut pulls = [Vec::new(), Vec::new(), Vec::new()];
        for toy in 0..ntoys {
            let (m, t) = sample(10_000 + toy);
            let fit = mass_fit(&m);
            if !fit.valid {
                continue;
            }
            let sw = sweights(&m, &fit);
            for (k, (_, opts)) in variants.iter().enumerate() {
                let f = lifetime_fit(&t, sw.weights(0), *opts);
                if f.valid && f.errors[0] > 0.0 {
                    pulls[k].push((f.params[0] - TAU_B) / f.errors[0]);
                }
            }
        }
        println!("\n5. {ntoys} toys: pull widths (1 means the errors are right)");
        let width = |p: &[f64]| {
            let n = p.len() as f64;
            let mean = p.iter().sum::<f64>() / n;
            (p.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        };
        for ((label, _), p) in variants.iter().zip(&pulls) {
            println!("   {label:<11} {:.3}", width(p));
        }

        // ---- 6. Plots and the results file -------------------------------------
        let red = Color::hex("#bd1f01");
        let blue = Color::hex("#3f90da");
        let gray = Color::hex("#94a4a2");

        // The mass fit, its components scaled to events per bin.
        let m_hist = histogram(
            "mass",
            "B^{0} candidates;m [GeV];Candidates",
            (M_BINS, M_LO, M_HI),
            &m,
            None,
        );
        let bin = (M_HI - M_LO) / M_BINS as f64;
        let p = fit.params.clone();
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &m_hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy data"),
        );
        ax.function_with(
            |x| bin * (p[0] * gauss(x, p[2], p[3]) + p[1] * linear(x, p[4])),
            M_LO..M_HI,
            CurveOpts::new().color(red).samples(400).label("Fit"),
        );
        ax.function_with(
            |x| bin * p[1] * linear(x, p[4]),
            M_LO..M_HI,
            CurveOpts::new()
                .color(gray)
                .dashed(vec![6.0, 4.0])
                .label("Background"),
        );
        ax.xlabel("$m(K^+\\pi^-\\mu^+\\mu^-)$ [GeV]")
            .ylabel("Candidates / 5 MeV")
            .hep_label("Demo", "Simulation")
            .legend();
        let mass_png = dir.join("mass_fit.png");
        ax.save(&mass_png)?;

        // The sWeighted decay times with the fitted lifetime.
        let t_hist = histogram(
            "time_sweighted",
            "sWeighted decay time;t [ps];Signal candidates",
            (T_BINS, 0.0, T_HI),
            &t,
            Some(&w_sig),
        );
        let tau = fits[2].params[0];
        let norm = w_sig.iter().sum::<f64>() * (T_HI / T_BINS as f64)
            / (tau * (1.0 - (-T_HI / tau).exp()));
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &t_hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("sWeighted toy data"),
        );
        ax.function_with(
            |x| norm * (-x / tau).exp(),
            0.0..T_HI,
            CurveOpts::new()
                .color(red)
                .label(format!("$\\tau$ = {tau:.3} ps")),
        );
        ax.yscale(Scale::Log)
            .ylim(0.5..600.0)
            .xlabel("Decay time [ps]")
            .ylabel("Signal candidates / 0.25 ps")
            .hep_label("Demo", "Simulation")
            .legend();
        let time_png = dir.join("lifetime_fit.png");
        ax.save(&time_png)?;

        // The sWeights against mass.
        let mut order: Vec<usize> = (0..m.len()).collect();
        order.sort_by(|&a, &b| m[a].total_cmp(&m[b]));
        let sorted = |w: &[f64]| order.iter().map(|&i| w[i]).collect::<Vec<_>>();
        let ms: Vec<f64> = order.iter().map(|&i| m[i]).collect();
        let mut ax = Axes::with_style(Style::mplhep());
        ax.plot_with(
            &ms,
            &sorted(&w_sig),
            CurveOpts::new().color(red).label("signal"),
        )?;
        ax.plot_with(
            &ms,
            &sorted(&w_bkg),
            CurveOpts::new().color(blue).label("background"),
        )?;
        ax.axhline(0.0);
        ax.xlabel("$m$ [GeV]")
            .ylabel("sWeight")
            .hep_label("Demo", "Simulation")
            .legend();
        let weights_png = dir.join("sweights.png");
        ax.save(&weights_png)?;

        // The toys' pulls: only the asymptotic errors give a unit width.
        let pull_hist = |name: &str, p: &[f64]| {
            histogram(name, &format!("{name};pull;Toys"), (24, -4.0, 4.0), p, None)
        };
        let (naive, asym) = (
            pull_hist("pull_naive", &pulls[0]),
            pull_hist("pull_asymptotic", &pulls[2]),
        );
        let scale = pulls[2].len() as f64 * 8.0 / 24.0 / std::f64::consts::TAU.sqrt();
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &naive,
            HistOpts::new()
                .color(gray)
                .label(format!("naive errors (width {:.2})", width(&pulls[0]))),
        );
        ax.hist_with(
            &asym,
            HistOpts::new()
                .color(red)
                .label(format!("asymptotic (width {:.2})", width(&pulls[2]))),
        );
        ax.function_with(
            |x| scale * (-0.5 * x * x).exp(),
            -4.0..4.0,
            CurveOpts::new()
                .color(Color::BLACK)
                .dashed(vec![4.0, 3.0])
                .label("N(0, 1)"),
        );
        ax.xlabel("$(\\hat\\tau - \\tau)/\\sigma_{\\hat\\tau}$")
            .ylabel("Toys")
            .hep_label("Demo", "Simulation")
            .legend();
        let pulls_png = dir.join("toy_pulls.png");
        ax.save(&pulls_png)?;

        let results = dir.join("splot_results.root");
        FileWriter::create(&results)
            .add(&m_hist.to_root()?)
            .add(&t_hist.to_root()?)
            .add(&naive.to_root()?)
            .add(&asym.to_root()?)
            .write()?;
        let back = FileReader::open(&results)?.get::<Hist1D>("time_sweighted")?;
        println!(
            "\n6. Results file {} (sWeighted time histogram: {:.1} signal read back)",
            results.display(),
            back.values().iter().sum::<f64>()
        );
        for png in [&mass_png, &time_png, &weights_png, &pulls_png] {
            println!("   plot: {}", png.display());
        }
        println!("\n(total {:.1?})", start.elapsed());
        Ok(())
    }
}
