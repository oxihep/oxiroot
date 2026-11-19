//! A complete charmonium analysis, from a `TTree` to published-style
//! numbers, plots and a results file — the shape of a J/ψ and ψ(2S)
//! production measurement:
//!
//! 1. **Write** a dimuon `TTree` (mass and pseudo-proper decay length per
//!    candidate) and **read** it back.
//! 2. **Fit the mass spectrum**: a Crystal Ball for each resonance, with a
//!    shared radiative tail and a resolution that scales with the mass, on an
//!    exponential background — binned likelihood, MINOS on the ψ(2S)/J/ψ
//!    yield ratio.
//! 3. **Look past the parabolic errors**: the profile likelihood of the ratio
//!    and the 68 % / 95 % contours of the J/ψ mass and resolution.
//! 4. **Validate the fit with toys**: refit pseudo-experiments drawn from the
//!    fitted model and check the pulls are unbiased with unit width.
//! 5. **Separate prompt from non-prompt J/ψ** (from b-hadron decays) with a
//!    two-dimensional unbinned extended likelihood fit in mass and decay
//!    length: the non-prompt fraction and the b-hadron lifetime.
//! 6. **Plot** each step and **write** the histograms and the toy pulls to a
//!    ROOT file, read back to check.
//!
//! The events are generated here with a fixed seed, so every result can be
//! compared with its truth. It takes about half a minute in `--release`,
//! most of it the two-dimensional fit, whose normalization integral is
//! numerical. Run with:
//!
//! ```sh
//! cargo run --release -p oxiroot --example charmonium_analysis
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    analysis::run()
}

#[cfg(all(feature = "fit", feature = "plot"))]
mod analysis {
    use oxiroot::plot::{Axes, Color, CurveOpts, HistOpts, HistType, Scale, Style};
    use oxiroot::prelude::*;
    use oxiroot::stat::{crystal_ball, erf, erfc};

    // ---- The truth --------------------------------------------------------
    /// PDG masses (GeV) and the ψ(2S) − J/ψ mass difference the fit keeps.
    const M_JPSI: f64 = 3.096_900;
    const M_PSI2S: f64 = 3.686_097;
    const DELTA_M: f64 = M_PSI2S - M_JPSI;
    /// Mass resolution at the J/ψ (scales with the mass), and the Crystal
    /// Ball tail the muons' radiation gives the peak.
    const RESOLUTION: f64 = 0.028;
    const ALPHA: f64 = 1.8;
    const N_TAIL: f64 = 3.0;
    const N_JPSI: usize = 30_000;
    const RATIO: f64 = 0.032;
    const N_BKG: usize = 30_000;
    const BKG_SLOPE: f64 = 1.5;
    /// Decay length (mm): resolution, the non-prompt fraction and the
    /// b-hadron cτ; the background's long-lived fraction and its cτ.
    const CT_RESOLUTION: f64 = 0.025;
    const NONPROMPT: f64 = 0.20;
    const CT_B: f64 = 0.455;
    const BKG_LONG: f64 = 0.45;
    const CT_BKG: f64 = 0.25;

    // ---- The analysis ranges ---------------------------------------------
    const LO: f64 = 2.8;
    const HI: f64 = 4.0;
    const BINS: usize = 120;
    const WIDTH: f64 = (HI - LO) / BINS as f64;
    /// The two-dimensional fit's windows: around the J/ψ, and in decay length.
    const WIN: (f64, f64) = (3.0, 3.19);
    const CT: (f64, f64) = (-0.3, 3.0);

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
        fn exponential(&mut self, mean: f64) -> f64 {
            -mean * self.uniform().ln()
        }
        /// `exp(−slope·(x − lo))` truncated to `[lo, hi)`, by inversion.
        fn falling(&mut self, slope: f64, lo: f64, hi: f64) -> f64 {
            let span = 1.0 - (-slope * (hi - lo)).exp();
            lo - (1.0 - self.uniform() * span).ln() / slope
        }
        /// A resonance's reconstructed mass: a Crystal Ball (the detector
        /// resolution with the radiative tail) inside the analysis range, by
        /// accept–reject under its unit peak.
        fn resonance(&mut self, m: f64) -> f64 {
            let sigma = RESOLUTION * m / M_JPSI;
            loop {
                let x = LO + (HI - LO) * self.uniform();
                if self.uniform() < crystal_ball(x, m, sigma, ALPHA, N_TAIL) {
                    return x;
                }
            }
        }
        /// A decay length: prompt, or exponential of `ct`, then the detector.
        fn decay_length(&mut self, long_lived: f64, ct: f64) -> f64 {
            let true_ct = if self.uniform() < long_lived {
                self.exponential(ct)
            } else {
                0.0
            };
            true_ct + CT_RESOLUTION * self.gauss()
        }
    }

    /// The Crystal Ball normalized to unit area (its tail on the low side).
    fn cb_density(m: f64, mean: f64, sigma: f64, alpha: f64, n: f64) -> f64 {
        let a = alpha.abs();
        let area = sigma
            * (n / a / (n - 1.0) * (-0.5 * a * a).exp()
                + (std::f64::consts::PI / 2.0).sqrt() * (1.0 + erf(a / std::f64::consts::SQRT_2)));
        crystal_ball(m, mean, sigma, alpha, n) / area
    }

    /// A decay-length resolution Gaussian, unit area.
    fn prompt(t: f64, s: f64) -> f64 {
        (-0.5 * (t / s).powi(2)).exp() / (s * std::f64::consts::TAU.sqrt())
    }

    /// An exponential of `ct` convolved with the resolution Gaussian, unit area.
    fn long_lived(t: f64, ct: f64, s: f64) -> f64 {
        let arg = (s / ct - t / s) / std::f64::consts::SQRT_2;
        0.5 / ct * (0.5 * (s / ct).powi(2) - t / ct).exp() * erfc(arg)
    }

    pub(super) fn run() -> oxiroot::Result<()> {
        let start = std::time::Instant::now();
        let dir = std::env::temp_dir().join("oxiroot-charmonium");
        std::fs::create_dir_all(&dir)?;

        // ---- 1. Write the dimuon candidates, read them back ---------------
        let mut rng = Rng(3_096_900);
        let (mut mass, mut ct) = (Vec::new(), Vec::new());
        let n_psi2s = (RATIO * N_JPSI as f64).round() as usize;
        for (m0, n) in [(M_JPSI, N_JPSI), (M_PSI2S, n_psi2s)] {
            for _ in 0..n {
                mass.push(rng.resonance(m0) as f32);
                ct.push(rng.decay_length(NONPROMPT, CT_B) as f32);
            }
        }
        for _ in 0..N_BKG {
            mass.push(rng.falling(BKG_SLOPE, LO, HI) as f32);
            ct.push(rng.decay_length(BKG_LONG, CT_BKG) as f32);
        }
        let tree_path = dir.join("dimuons.root");
        Tree::new(
            "dimuons",
            vec![Branch::f32("mass", mass), Branch::f32("ctau", ct)],
        )
        .write_root(&tree_path, Compression::Zstd(5))?;

        let file = FileReader::open(&tree_path)?;
        let tree = TreeReader::open(&file, "dimuons")?;
        let column = |name: &str| -> oxiroot::Result<Vec<f64>> {
            match tree.read_branch(&file, name)? {
                BranchValues::F32(v) => Ok(v.into_iter().map(f64::from).collect()),
                other => panic!("{name} is {:?}, not a float branch", other.leaf_type()),
            }
        };
        let (mass, ct) = (column("mass")?, column("ctau")?);
        println!(
            "1. Wrote and read back {} dimuon candidates ({} J/ψ, {} ψ(2S), {} background) — {}",
            tree.num_entries(),
            N_JPSI,
            n_psi2s,
            N_BKG,
            tree_path.display()
        );

        // ---- 2. The mass fit ----------------------------------------------
        let mut m_hist = Hist::reg(BINS, LO, HI)
            .double()
            .to_root()
            .named("m_mumu")
            .titled("dimuon mass");
        m_hist.sumw2();
        for &m in &mass {
            m_hist.fill(m);
        }
        let model = Model::new(
            "J/ψ + ψ(2S) + background",
            &[
                "N_jpsi", "ratio", "m_jpsi", "sigma", "alpha", "n", "N_bkg", "slope",
            ],
            vec![25_000.0, 0.05, 3.09, 0.03, 1.5, 5.0, 25_000.0, 1.0],
            |m, p| {
                let (n1, r, m1, s1, alpha, n) = (p[0], p[1], p[2], p[3], p[4], p[5]);
                let m2 = m1 + DELTA_M;
                let jpsi = n1 * cb_density(m, m1, s1, alpha, n);
                let psi2s = r * n1 * cb_density(m, m2, s1 * m2 / m1, alpha, n);
                let bkg =
                    p[6] * p[7] * (-p[7] * (m - LO)).exp() / (1.0 - (-p[7] * (HI - LO)).exp());
                WIDTH * (jpsi + psi2s + bkg)
            },
        )
        .limit("ratio", 0.0, 0.5)
        .limit("m_jpsi", 3.0, 3.2)
        .limit("sigma", 0.005, 0.2)
        .limit("alpha", 0.3, 5.0)
        .limit("n", 1.05, 50.0)
        .limit("slope", 0.01, 10.0);
        // MIGRAD with a SIMPLEX fallback: from a rough seed, eight parameters
        // of very different scales.
        let opts = FitOptions::new()
            .method(FitMethod::Likelihood)
            .minimizer(Minimizer::Minuit2Combined);
        let fit = m_hist.fit_opts(&model, &opts.with_minos(true));
        let minos = fit.minos.clone().unwrap_or_default();
        println!(
            "\n2. Mass fit (binned likelihood), χ²/ndf = {:.1}/{}",
            fit.chi2, fit.ndf
        );
        println!("   parameter   fitted                     truth");
        println!(
            "   N(J/ψ)      {:>8.0} ± {:<5.0}            {N_JPSI}",
            fit.params[0], fit.errors[0]
        );
        let (r_lo, r_hi) = minos.get(1).copied().unwrap_or((f64::NAN, f64::NAN));
        println!(
            "   ψ(2S)/J/ψ   {:.4} {:+.4}/{:+.4} (MINOS)   {RATIO}",
            fit.params[1], r_hi, r_lo
        );
        println!(
            "   m(J/ψ)      {:.5} ± {:.5} GeV      {M_JPSI}",
            fit.params[2], fit.errors[2]
        );
        println!(
            "   σ(J/ψ)      {:.5} ± {:.5} GeV      {RESOLUTION}",
            fit.params[3], fit.errors[3]
        );
        println!(
            "   tail α, n   {:.2} ± {:.2}, {:.2} ± {:.2}       {ALPHA}, {N_TAIL}",
            fit.params[4], fit.errors[4], fit.params[5], fit.errors[5]
        );

        // ---- 3. Profile and contours --------------------------------------
        let seeded = model.clone().with_params(fit.params.clone());
        let r = fit.params[1];
        let dr = fit.errors[1];
        let profile = m_hist
            .profile(&seeded, &opts, "ratio", (r - 3.5 * dr, r + 3.5 * dr), 41)
            .expect("ratio profile");
        let contour_68 = m_hist
            .contour(&seeded, &opts, "m_jpsi", "sigma", 2.30, 40)
            .expect("68 % contour");
        let contour_95 = m_hist
            .contour(&seeded, &opts, "m_jpsi", "sigma", 5.99, 40)
            .expect("95 % contour");
        println!("\n3. Beyond the parabolic errors");
        if let (Some((a, b)), Some((a2, b2))) = (profile.interval(1.0), profile.interval(4.0)) {
            println!(
                "   ratio, profile likelihood: 68 % [{a:.4}, {b:.4}], 95 % [{a2:.4}, {b2:.4}]"
            );
        }
        println!(
            "   (m, σ) contours traced with {} and {} points",
            contour_68.points.len(),
            contour_95.points.len()
        );

        // ---- 4. Toy validation --------------------------------------------
        let centers: Vec<f64> = (0..BINS).map(|i| LO + WIDTH * (i as f64 + 0.5)).collect();
        let ntoys = 500;
        let study = Toys::binned(&seeded, &centers)
            .seed(1)
            .run(&seeded, &opts, ntoys);
        println!(
            "\n4. {ntoys} pseudo-experiments from the fitted model ({:.0} % valid fits)",
            100.0 * study.valid_fraction()
        );
        println!("   parameter   pull mean           pull width");
        for name in ["ratio", "m_jpsi", "sigma", "N_jpsi"] {
            if let Some(s) = study.pull_summary(name) {
                println!(
                    "   {name:<10}  {:+.3} ± {:.3}      {:.3} ± {:.3}",
                    s.mean, s.mean_err, s.width, s.width_err
                );
            }
        }
        let ratio_pulls = study.pulls("ratio").unwrap_or_default();
        let mass_pulls = study.pulls("m_jpsi").unwrap_or_default();

        // ---- 5. Prompt and non-prompt J/ψ ---------------------------------
        // Unbinned in (mass, decay length) around the J/ψ: the mass shapes are
        // fixed from step 2, the decay-length ones fitted.
        let (sel_m, sel_t): (Vec<f64>, Vec<f64>) = mass
            .iter()
            .zip(&ct)
            .filter(|&(&m, &t)| (WIN.0..WIN.1).contains(&m) && (CT.0..CT.1).contains(&t))
            .map(|(&m, &t)| (m, t))
            .unzip();
        let (m1, s1, alpha, n_cb, slope) = (
            fit.params[2],
            fit.params[3],
            fit.params[4],
            fit.params[5],
            fit.params[7],
        );
        // The mass shapes normalized inside the window, once.
        let steps = 2000;
        let integral = |f: &dyn Fn(f64) -> f64| -> f64 {
            let h = (WIN.1 - WIN.0) / steps as f64;
            (0..steps)
                .map(|k| f(WIN.0 + h * (k as f64 + 0.5)) * h)
                .sum()
        };
        let sig_m = move |m: f64| cb_density(m, m1, s1, alpha, n_cb);
        let bkg_m = move |m: f64| (-slope * (m - LO)).exp();
        let (sig_norm, bkg_norm) = (integral(&sig_m), integral(&bkg_m));
        let lifetime = Model::new2(
            "mass × decay length",
            &["N_sig", "N_bkg", "f_B", "ct_B", "res", "f_long", "ct_bkg"],
            vec![20_000.0, 5_000.0, 0.3, 0.4, 0.03, 0.4, 0.3],
            move |m, t, p| {
                let s = p[4];
                let sig = sig_m(m) / sig_norm
                    * (p[2] * long_lived(t, p[3], s) + (1.0 - p[2]) * prompt(t, s));
                let bkg = bkg_m(m) / bkg_norm
                    * (p[5] * long_lived(t, p[6], s) + (1.0 - p[5]) * prompt(t, s));
                p[0] * sig + p[1] * bkg
            },
        )
        .limit("f_B", 0.0, 1.0)
        .limit("ct_B", 0.05, 2.0)
        .limit("res", 0.005, 0.1)
        .limit("f_long", 0.0, 1.0)
        .limit("ct_bkg", 0.02, 2.0)
        .lower_limit("N_sig", 0.0)
        .lower_limit("N_bkg", 0.0);
        let events = Events2::new(&sel_m, &sel_t).expect("same lengths");
        let t2 = std::time::Instant::now();
        let fit2 = events.fit_opts(
            &lifetime,
            &FitOptions::new()
                .method(FitMethod::ExtendedUnbinnedLikelihood)
                .range(WIN.0, WIN.1)
                .range_y(CT.0, CT.1),
        );
        println!(
            "\n5. Unbinned extended fit in (mass, decay length): {} candidates, {:.1?} ({} evaluations){}",
            sel_m.len(),
            t2.elapsed(),
            fit2.nfcn,
            if fit2.valid { "" } else { " — NOT VALID" }
        );
        println!("   parameter          fitted               truth");
        let rows = [
            ("non-prompt f_B", 2, NONPROMPT),
            ("b-hadron cτ (mm)", 3, CT_B),
            ("resolution (mm)", 4, CT_RESOLUTION),
            ("bkg long-lived", 5, BKG_LONG),
            ("bkg cτ (mm)", 6, CT_BKG),
        ];
        for (label, k, truth) in rows {
            println!(
                "   {label:<18} {:.4} ± {:.4}      {truth}",
                fit2.params[k], fit2.errors[k]
            );
        }
        println!(
            "   signal / background in the window: {:.0} ± {:.0} / {:.0} ± {:.0}",
            fit2.params[0], fit2.errors[0], fit2.params[1], fit2.errors[1]
        );

        // ---- 6. Plots and the results file --------------------------------
        let blue = Color::hex("#3f90da");
        let red = Color::hex("#bd1f01");
        let gray = Color::hex("#717581");
        let fitted = model.clone().with_params(fit.params.clone());
        let bkg_only = {
            let mut p = fit.params.clone();
            p[0] = 0.0;
            model.clone().with_params(p)
        };
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &m_hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy data"),
        );
        ax.model_with(
            &fitted,
            LO..HI,
            CurveOpts::new().color(red).samples(600).label("Fit"),
        );
        ax.model_with(
            &bkg_only,
            LO..HI,
            CurveOpts::new()
                .color(gray)
                .dashed(vec![6.0, 4.0])
                .label("Background"),
        );
        ax.yscale(Scale::Log)
            .xlabel("$m_{\\mu\\mu}$ [GeV]")
            .ylabel("Candidates / 10 MeV")
            .hep_label("Demo", "Simulation")
            .legend();
        let mass_png = dir.join("mass_fit.png");
        ax.save(&mass_png)?;

        // Decay length in the J/ψ window, with the fit's projection.
        let ct_bins = 66;
        let ct_width = (CT.1 - CT.0) / ct_bins as f64;
        let mut t_hist = Hist::reg(ct_bins, CT.0, CT.1)
            .double()
            .to_root()
            .named("ctau")
            .titled("decay length, J/#psi window");
        t_hist.sumw2();
        for &t in &sel_t {
            t_hist.fill(t);
        }
        let p2 = fit2.params.clone();
        let projection = |part: usize| {
            let p = p2.clone();
            Model::new("projection", &[], vec![], move |t, _| {
                let s = p[4];
                let sig_p = p[0] * (1.0 - p[2]) * prompt(t, s);
                let sig_np = p[0] * p[2] * long_lived(t, p[3], s);
                let bkg = p[1] * (p[5] * long_lived(t, p[6], s) + (1.0 - p[5]) * prompt(t, s));
                ct_width
                    * match part {
                        0 => sig_p + sig_np + bkg,
                        1 => sig_np,
                        _ => bkg,
                    }
            })
        };
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &t_hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy data"),
        );
        ax.model_with(
            &projection(0),
            CT.0..CT.1,
            CurveOpts::new().color(red).samples(800).label("Fit"),
        );
        ax.model_with(
            &projection(1),
            CT.0..CT.1,
            CurveOpts::new()
                .color(blue)
                .samples(800)
                .label("Non-prompt J/ψ"),
        );
        ax.model_with(
            &projection(2),
            CT.0..CT.1,
            CurveOpts::new()
                .color(gray)
                .dashed(vec![6.0, 4.0])
                .samples(800)
                .label("Background"),
        );
        ax.yscale(Scale::Log)
            .ylim(0.5..2.0e4)
            .xlabel("$\\ell_{J/\\psi}$ [mm]")
            .ylabel("Candidates / 50 μm")
            .hep_label("Demo", "Simulation")
            .legend();
        let ct_png = dir.join("decay_length_fit.png");
        ax.save(&ct_png)?;

        let mut ax = Axes::with_style(Style::mplhep());
        ax.fit_contour_with(&contour_95, CurveOpts::new().color(blue).label("95 % CL"));
        ax.fit_contour_with(&contour_68, CurveOpts::new().color(red).label("68 % CL"));
        ax.xlabel("$m_{J/\\psi}$ [GeV]")
            .ylabel("$\\sigma$ [GeV]")
            .hep_label("Demo", "Simulation")
            .legend();
        let contour_png = dir.join("mass_resolution_contour.png");
        ax.save(&contour_png)?;

        let mut ax = Axes::with_style(Style::mplhep());
        ax.fit_profile_with(&profile, CurveOpts::new().color(red));
        ax.xlabel("$N_{\\psi(2S)} / N_{J/\\psi}$")
            .ylabel("$-2\\,\\Delta\\ln L$")
            .hep_label("Demo", "Simulation");
        let profile_png = dir.join("ratio_profile.png");
        ax.save(&profile_png)?;

        let mut pulls = Hist::reg(24, -4.0, 4.0)
            .double()
            .to_root()
            .named("ratio_pull")
            .titled("ratio pulls");
        for &p in &ratio_pulls {
            pulls.fill(p);
        }
        let n_pulls = ratio_pulls.len() as f64;
        let unit_normal = Model::new("N(0,1)", &[], vec![], move |x, _| {
            n_pulls * (8.0 / 24.0) * (-0.5 * x * x).exp() / std::f64::consts::TAU.sqrt()
        });
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &pulls,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy fits"),
        );
        ax.model_with(
            &unit_normal,
            -4.0..4.0,
            CurveOpts::new().color(red).label("Unit Gaussian"),
        );
        ax.xlabel("pull of $N_{\\psi(2S)} / N_{J/\\psi}$")
            .ylabel("Toys")
            .hep_label("Demo", "Simulation")
            .legend();
        let pulls_png = dir.join("ratio_pulls.png");
        ax.save(&pulls_png)?;

        let results = dir.join("charmonium_results.root");
        FileWriter::create(&results)
            .add(&m_hist)
            .add(&t_hist)
            .add(&pulls)
            .put(Tree::new(
                "toys",
                vec![
                    Branch::f64("ratio_pull", ratio_pulls.clone()),
                    Branch::f64("mass_pull", mass_pulls.clone()),
                ],
            ))
            .write(Compression::Zstd(5))?;
        let check = FileReader::open(&results)?;
        let toys = TreeReader::open(&check, "toys")?;
        let back = Hist1D::read_root(&check, "m_mumu")?;
        println!(
            "\n6. Results file {} — m_mumu ({:.0} entries) and {} toy pulls read back",
            results.display(),
            back.integral(),
            toys.num_entries()
        );
        for png in [&mass_png, &ct_png, &contour_png, &profile_png, &pulls_png] {
            println!("   plot: {}", png.display());
        }
        println!("\n(total {:.1?})", start.elapsed());
        Ok(())
    }
}
