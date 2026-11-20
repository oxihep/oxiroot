//! A search for a narrow resonance on a smoothly falling background — the
//! shape of a diphoton bump hunt — from a `TTree` to the plots a search
//! publishes:
//!
//! 1. **Write** the diphoton candidates' invariant mass to a `TTree`, **read**
//!    it back and histogram it.
//! 2. **Choose the background function**: exponentials of polynomials of rising
//!    order, keeping the lowest order the next one does not improve (a
//!    likelihood-ratio test).
//! 3. **Scan the local significance** p₀ across the mass range, and **fit the
//!    signal** at the most significant mass with the mass free.
//! 4. **Correct for the look-elsewhere effect** (Gross–Vitells, upcrossings
//!    counted on background-only pseudo-experiments).
//! 5. **Set CLs upper limits** on the signal strength against mass, observed
//!    and expected with its ±1σ/±2σ band.
//! 6. **Plot** the fit with its residuals, p₀ against mass and the Brazil band,
//!    and **write** the histogram and the limit and p₀ curves (as `TGraph`s) to
//!    a ROOT file, read back to check.
//!
//! The events are generated here with a fixed seed, so the truth is known.
//! Run with (a few seconds in `--release`):
//!
//! ```sh
//! cargo run --release -p oxiroot --example resonance_search
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    search::run()
}

#[cfg(all(feature = "fit", feature = "plot"))]
mod search {
    use oxiroot::fit::{FitResult, UpperLimit, global_significance, upcrossings};
    use oxiroot::plot::{
        Axes, Color, CurveOpts, HAlign, HistOpts, HistType, Scale, Style, TextOpts, VAlign,
        ratio_subplots_with,
    };
    use oxiroot::prelude::*;
    use oxiroot::stat::{chi_square_prob, pvalue_from_significance};

    /// The mass window, its binning (GeV) and the detector resolution.
    const LO: f64 = 100.0;
    const HI: f64 = 160.0;
    const BINS: usize = 120;
    const WIDTH: f64 = (HI - LO) / BINS as f64;
    const RESOLUTION: f64 = 1.7;
    /// The signal yield the strength `μ = 1` stands for.
    const REF_SIGNAL: f64 = 200.0;
    /// The truth the events are drawn from.
    const TRUE_MASS: f64 = 125.0;
    const N_SIGNAL: usize = 260;
    const N_BACKGROUND: usize = 40_000;
    /// The background shape `exp(a·x + b·x²)`, `x = (m − 100)/60`.
    const TRUE_SHAPE: (f64, f64) = (-3.5, 0.9);

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
        /// A draw from `exp(a·x + b·x²)` on `[0, 1)`: an exponential by
        /// inversion, accepted with probability `exp(b·(x² − 1))`.
        fn background(&mut self, (a, b): (f64, f64)) -> f64 {
            loop {
                let x = (1.0 - self.uniform() * (1.0 - a.exp())).ln() / a;
                if self.uniform() < (b * (x * x - 1.0)).exp() {
                    return x;
                }
            }
        }
    }

    /// `exp(c₀ + c₁x + …)` at mass `m`, `x = (m − LO)/(HI − LO)`: counts per bin.
    fn poly_exp(m: f64, c: &[f64]) -> f64 {
        let x = (m - LO) / (HI - LO);
        c.iter().rev().fold(0.0, |acc, &ck| acc * x + ck).exp()
    }

    /// The background model of polynomial order `order`.
    fn background(order: usize) -> Model {
        let names: Vec<String> = (0..=order).map(|k| format!("c{k}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut seed = vec![0.0; order + 1];
        seed[0] = 6.0;
        if order >= 1 {
            seed[1] = -3.0;
        }
        Model::new("background", &refs, seed, poly_exp)
    }

    /// The signal shape, counts per bin for `mu = 1`: a Gaussian of the
    /// detector resolution at `mass`.
    fn peak(m: f64, mass: f64) -> f64 {
        REF_SIGNAL * WIDTH * (-0.5 * ((m - mass) / RESOLUTION).powi(2)).exp()
            / (RESOLUTION * std::f64::consts::TAU.sqrt())
    }

    /// Signal + background: `mu·peak(mass)` on the background of `order`.
    fn signal_plus_background(order: usize) -> Model {
        let bkg = background(order);
        let mut names = vec!["mu".to_string(), "mass".to_string()];
        names.extend(bkg.param_names().iter().cloned());
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut seed = vec![1.0, TRUE_MASS];
        seed.extend(bkg.params().iter());
        Model::new("s+b", &refs, seed, |m, p| {
            p[0] * peak(m, p[1]) + poly_exp(m, &p[2..])
        })
        .limit("mu", 0.0, 20.0)
        .limit("mass", LO + 5.0, HI - 5.0)
    }

    fn likelihood() -> FitOptions {
        FitOptions::new().method(FitMethod::Likelihood)
    }

    /// `fitter` with the mass held at `m`.
    fn at_mass<'c>(fitter: &Fitter<'c>, m: f64) -> Fitter<'c> {
        fitter.clone().set("mass", m).fix("mass")
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "generated values stored in an f32 branch"
    )]
    pub(super) fn run() -> oxiroot::Result<()> {
        let start = std::time::Instant::now();
        let dir = std::env::temp_dir().join("oxiroot-resonance-search");
        std::fs::create_dir_all(&dir)?;

        // ---- 1. The candidates: a TTree, written and read back ------------
        let mut rng = Rng(20_261_119);
        let mut masses = Vec::with_capacity(N_BACKGROUND + N_SIGNAL);
        for _ in 0..N_BACKGROUND {
            masses.push((LO + rng.background(TRUE_SHAPE) * (HI - LO)) as f32);
        }
        for _ in 0..N_SIGNAL {
            masses.push((TRUE_MASS + RESOLUTION * rng.gauss()) as f32);
        }
        let tree_path = dir.join("diphotons.root");
        Tree::new("diphotons", vec![Branch::f32("m_gg", masses)])
            .write_root(&tree_path, Compression::Zstd(5))?;
        let file = FileReader::open(&tree_path)?;
        let tree = TreeReader::open(&file, "diphotons")?;
        let BranchValues::F32(m_gg) = tree.read_branch(&file, "m_gg")? else {
            unreachable!("m_gg is written as f32")
        };
        let mut hist = Hist::reg(BINS, LO, HI)
            .double()
            .to_root()
            .named("m_gg")
            .titled("Diphoton mass;m_{#gamma#gamma} [GeV];Events / 0.5 GeV");
        for &m in &m_gg {
            hist.fill(f64::from(m));
        }
        println!(
            "1. Wrote and read back {} diphoton candidates ({N_BACKGROUND} background, \
             {N_SIGNAL} signal at {TRUE_MASS} GeV) — {}",
            tree.entries(),
            tree_path.display()
        );

        // ---- 2. The background function ------------------------------------
        println!("\n2. Background: exp(polynomial of order k), binned likelihood");
        println!("   k   −2 ln L     Δ vs k−1   p(Δ)");
        let mut chosen = None;
        let mut previous: Option<FitResult> = None;
        for k in 1..=4 {
            let fit = hist.fit_opts(&background(k), &likelihood());
            let (delta, p) = previous.as_ref().map_or((f64::NAN, f64::NAN), |prev| {
                let d = prev.fval - fit.fval;
                (d, chi_square_prob(d.max(0.0), 1))
            });
            println!("   {k}   {:>9.2}   {delta:>8.2}   {p:>6.4}", fit.fval);
            if chosen.is_none() && p > 0.05 {
                chosen = Some(k - 1);
            }
            previous = Some(fit);
        }
        let order = chosen.unwrap_or(4);
        println!("   → order {order}");

        // ---- 3. Local significance and the signal fit -----------------------
        let model = signal_plus_background(order);
        let bkg_fit = hist.fit_opts(&background(order), &likelihood());
        let mut seeds = vec![0.5, TRUE_MASS];
        seeds.extend(&bkg_fit.params);
        let cost = ModelCost::new(&hist, &model, &likelihood());
        let fitter = cost.fitter().with_params(seeds);
        let scan: Vec<f64> = (0..=80).map(|k| 110.0 + 0.5 * f64::from(k)).collect();
        let q0_scan = |f: &Fitter<'_>| -> Vec<f64> {
            scan.iter()
                .map(|&m| {
                    HypoTest::new(at_mass(f, m), "mu")
                        .ok()
                        .and_then(|t| t.discovery(1.0).ok())
                        .map_or(f64::NAN, |d| d.q0)
                })
                .collect()
        };
        let q0 = q0_scan(&fitter);
        let (best, q_max) = scan
            .iter()
            .zip(&q0)
            .filter(|(_, q)| q.is_finite())
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(&m, &q)| (m, q))
            .expect("a scan point");
        let fitter = fitter.set("mass", best);
        let fit = fitter.fit(&FitOptions::new().minos(true));
        let (lo, hi) = fit.minos.as_ref().map_or((f64::NAN, f64::NAN), |m| m[0]);
        println!(
            "\n3. Largest local excess at {best:.1} GeV: Z_local = {:.2}",
            q_max.sqrt()
        );
        println!(
            "   signal fit (mass free): μ = {:.3} {hi:+.3}/{lo:+.3} → {:.0} ± {:.0} events \
             (truth {N_SIGNAL}); mass {:.2} ± {:.2} GeV (truth {TRUE_MASS})",
            fit.params[0],
            fit.params[0] * REF_SIGNAL,
            fit.errors[0] * REF_SIGNAL,
            fit.params[1],
            fit.errors[1]
        );

        // ---- 4. Look-elsewhere effect -----------------------------------------
        let c0 = 0.5;
        let bkg_only = fitter
            .clone()
            .set("mu", 0.0)
            .fix("mu")
            .fit(&FitOptions::new());
        let ntoys = 20_u32;
        let mut crossings = 0;
        for t in 0..ntoys {
            let toy = cost
                .toy(&bkg_only.params, 1000 + u64::from(t))
                .expect("binned toys");
            let toy_fitter = Fitter::new(&*toy, fit.params.clone())
                .limit("mu", 0.0, 20.0)
                .limit("mass", LO + 5.0, HI - 5.0);
            crossings += upcrossings(&q0_scan(&toy_fitter), c0);
        }
        let mean_n = crossings as f64 / f64::from(ntoys);
        let (p_global, z_global) = global_significance(q_max, c0, mean_n);
        println!(
            "\n4. Look-elsewhere effect ({ntoys} background-only toy scans, ⟨N({c0})⟩ = {mean_n:.2}): \
             p_local {:.2e} → p_global {p_global:.2e} (Z_global = {z_global:.2})",
            pvalue_from_significance(q_max.sqrt())
        );

        // ---- 5. Upper limits against mass ------------------------------------
        let limit_masses: Vec<f64> = (0..=20).map(|k| 110.0 + 2.0 * f64::from(k)).collect();
        let limits: Vec<UpperLimit> = limit_masses
            .iter()
            .map(|&m| {
                HypoTest::new(at_mass(&fitter, m), "mu")
                    .and_then(|t| t.upper_limit(0.95, (0.0, 20.0)))
                    .expect("a limit")
            })
            .collect();
        println!("\n5. 95 % CL upper limits on μ (CLs, asymptotic)");
        println!("   mass    observed   expected  [−1σ, +1σ]");
        for (m, l) in limit_masses.iter().zip(&limits).step_by(4) {
            println!(
                "   {m:>5.1}   {:>7.3}    {:>7.3}   [{:.3}, {:.3}]",
                l.observed, l.expected[2], l.expected[1], l.expected[3]
            );
        }

        // ---- 6. Plots and the results file -----------------------------------
        let red = Color::hex("#bd1f01");
        let blue = Color::hex("#3f90da");
        let fitted = model.clone().with_params(fit.params.clone());
        let mut bkg_params = fit.params.clone();
        bkg_params[0] = 0.0;
        let bkg_curve = model.clone().with_params(bkg_params);

        // The fit, and the data minus the background beneath it.
        let (fig, mut main, mut lower) = ratio_subplots_with(Style::mplhep());
        main.hist_with(
            &hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy data"),
        );
        main.model_with(
            &fitted,
            LO..HI,
            CurveOpts::new().color(red).samples(600).label("S + B fit"),
        );
        main.model_with(
            &bkg_curve,
            LO..HI,
            CurveOpts::new()
                .color(blue)
                .dashed(vec![6.0, 4.0])
                .label("Background"),
        );
        main.ylabel("Events / 0.5 GeV")
            .hep_label("Demo", "Simulation")
            .legend();
        let centres: Vec<f64> = (1..=BINS).map(|i| hist.bin_center(i)).collect();
        let residual: Vec<f64> = centres
            .iter()
            .enumerate()
            .map(|(i, &m)| hist.contents()[i + 1] - bkg_curve.eval(m))
            .collect();
        let errors: Vec<f64> = (1..=BINS).map(|i| hist.bin_error(i)).collect();
        let residuals = Graph::with_errors(centres.clone(), residual, vec![0.0; BINS], errors)?;
        lower.errorbar(&residuals);
        lower.function_with(
            |m| fit.params[0] * peak(m, fit.params[1]),
            LO..HI,
            CurveOpts::new().color(red).samples(600),
        );
        lower.axhline(0.0);
        lower
            .ylabel("Data − bkg")
            .xlabel("$m_{\\gamma\\gamma}$ [GeV]");
        let fit_png = dir.join("mass_fit.png");
        fig.ratio(main, lower).save(&fit_png)?;

        // Local p₀ against mass, with the 1σ–3σ levels.
        let p0: Vec<f64> = q0
            .iter()
            .map(|q| pvalue_from_significance(q.max(0.0).sqrt()))
            .collect();
        let mut ax = Axes::with_style(Style::mplhep());
        ax.plot_with(
            &scan,
            &p0,
            CurveOpts::new().color(Color::BLACK).label("Observed"),
        )?;
        for z in 1..=3 {
            let p = pvalue_from_significance(f64::from(z));
            ax.axhline_with(p, CurveOpts::new().color(red).dashed(vec![4.0, 3.0]));
            ax.text_with(
                151.5,
                p,
                format!("{z}σ"),
                TextOpts::new()
                    .color(red)
                    .align(HAlign::Right, VAlign::Bottom),
            );
        }
        ax.yscale(Scale::Log)
            .ylim(5e-4..1.0)
            .xlabel("$m_X$ [GeV]")
            .ylabel("Local $p_0$")
            .hep_label("Demo", "Simulation")
            .legend();
        let p0_png = dir.join("local_p0.png");
        ax.save(&p0_png)?;

        // The Brazil band.
        let mut ax = Axes::with_style(Style::mplhep());
        ax.fit_limits(&limit_masses, &limits)?
            .axhline_with(1.0, CurveOpts::new().color(red))
            .xlabel("$m_X$ [GeV]")
            .ylabel("95 % CL limit on $\\mu$")
            .hep_label("Demo", "Simulation")
            .legend();
        ax.text_with(
            0.04,
            0.06,
            "CLs, asymptotic",
            TextOpts::new().axes_coords(true),
        );
        let limits_png = dir.join("limits.png");
        ax.save(&limits_png)?;

        // The results file: the histogram and the curves as TGraphs.
        let graph = |name: &str, y: Vec<f64>| -> oxiroot::Result<Graph> {
            Ok(Graph::new(limit_masses.clone(), y)?.named(name))
        };
        let band = |k: (usize, usize), name: &str| -> oxiroot::Result<Graph> {
            let median: Vec<f64> = limits.iter().map(|l| l.expected[2]).collect();
            let low: Vec<f64> = limits
                .iter()
                .map(|l| l.expected[2] - l.expected[k.0])
                .collect();
            let high: Vec<f64> = limits
                .iter()
                .map(|l| l.expected[k.1] - l.expected[2])
                .collect();
            let zeros = vec![0.0; limit_masses.len()];
            Ok(Graph::with_asymm_errors(
                limit_masses.clone(),
                median,
                zeros.clone(),
                zeros,
                low,
                high,
            )?
            .named(name))
        };
        let results = dir.join("search_results.root");
        FileWriter::create(&results)
            .add(&hist)
            .add(&graph(
                "limit_observed",
                limits.iter().map(|l| l.observed).collect(),
            )?)
            .add(&band((1, 3), "limit_expected_1sigma")?)
            .add(&band((0, 4), "limit_expected_2sigma")?)
            .add(&Graph::new(scan.clone(), p0)?.named("local_p0"))
            .write(Compression::Zstd(5))?;
        let check = FileReader::open(&results)?;
        let back = Graph::read_root(&check, "limit_observed")?;
        println!(
            "\n6. Results file {} — m_gg and {} limit points read back",
            results.display(),
            back.x().len()
        );
        for png in [&fit_png, &p0_png, &limits_png] {
            println!("   plot: {}", png.display());
        }
        println!("\n(total {:.1?})", start.elapsed());
        Ok(())
    }
}
