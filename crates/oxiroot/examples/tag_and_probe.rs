//! A trigger efficiency by tag and probe, from a `TTree` to an efficiency map
//! — the way a collider experiment measures one in data: Z → μμ events where
//! one muon (the tag) fired the trigger and the other (the probe) did or did
//! not.
//!
//! 1. **Write** the probes (pT, |η|, dimuon mass, passed) to a `TTree` and
//!    **read** them back.
//! 2. **Per (pT, |η|) bin, fit** the dimuon mass of passing and failing probes
//!    simultaneously: one Z yield and one efficiency `ε` shared by the two
//!    categories, each with its own background — the background is
//!    subtracted by the fit, not by hand.
//! 3. **The efficiency map** (a `TH2D`) and **the turn-on** in each |η| region:
//!    an error-function step fitted to the bin-averaged efficiencies.
//! 4. **A matched, background-free sample** (as in simulation) as a ROOT
//!    `TEfficiency`, fitted with the binomial likelihood — and toys showing
//!    why a χ² with binomial errors is the wrong tool where the efficiency
//!    nears 0 or 1.
//! 5. **Plot** one bin's pass/fail fits, the map, the turn-ons and the toys'
//!    pulls, and **write** the map, the turn-ons and the `TEfficiency` to a
//!    ROOT file.
//!
//! Run with (a few seconds in `--release`):
//!
//! ```sh
//! cargo run --release -p oxiroot --example tag_and_probe
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    probes::run()
}

#[cfg(all(feature = "fit", feature = "plot"))]
mod probes {
    use std::fmt::Write as _;

    use oxiroot::fit::BinomialData;
    use oxiroot::hist::oxihist::{Double, WeightedSum};
    use oxiroot::plot::{
        Axes, Color, Colormap, CurveOpts, HAlign, Hist2dOpts, HistOpts, HistType, Style, TextOpts,
        VAlign,
    };
    use oxiroot::prelude::*;
    use oxiroot::stat::erf;

    const M_LO: f64 = 70.0;
    const M_HI: f64 = 110.0;
    const M_BINS: usize = 40;
    const Z_MASS: f64 = 91.19;
    const Z_WIDTH: f64 = 3.0;
    /// Probe-pT and |η| bins.
    const PT_EDGES: [f64; 10] = [14.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 32.0, 40.0, 60.0];
    const ETA_EDGES: [f64; 3] = [0.0, 1.2, 2.4];
    /// The true turn-on per |η| region: threshold, resolution (`GeV`), plateau.
    const TRUE: [[f64; 3]; 2] = [[24.0, 2.0, 0.94], [24.5, 2.6, 0.88]];
    /// Z probes and background pairs per (pT, |η|) bin.
    const N_Z: usize = 3000;
    const N_BKG: usize = 2500;

    /// An in-range bin index, as oxihist counts them.
    fn bin(i: usize) -> isize {
        isize::try_from(i).expect("a small index")
    }

    fn efficiency(pt: f64, p: &[f64]) -> f64 {
        p[2] * 0.5 * (1.0 + erf((pt - p[0]) / (2f64.sqrt() * p[1])))
    }

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
    }

    /// The probes: per (pT, |η|) bin, Z decays and a falling background of
    /// pairs whose probes pass half as often.
    fn generate(rng: &mut Rng) -> [Vec<f64>; 4] {
        let [mut pt, mut eta, mut mass, mut passed] =
            [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (r, e) in ETA_EDGES.windows(2).enumerate() {
            for p in PT_EDGES.windows(2) {
                for k in 0..N_Z + N_BKG {
                    let x = p[0] + (p[1] - p[0]) * rng.uniform();
                    let z = k < N_Z;
                    let m = if z {
                        Z_MASS + Z_WIDTH * rng.gauss()
                    } else {
                        let span = 1.0 - (-(M_HI - M_LO) / 25.0f64).exp();
                        M_LO - 25.0 * (1.0 - rng.uniform() * span).ln()
                    };
                    let eps = efficiency(x, &TRUE[r]) * if z { 1.0 } else { 0.5 };
                    pt.push(x);
                    eta.push(e[0] + (e[1] - e[0]) * rng.uniform());
                    mass.push(m);
                    passed.push(f64::from(u8::from(rng.uniform() < eps)));
                }
            }
        }
        [pt, eta, mass, passed]
    }

    /// Fit one bin's passing and failing mass histograms together: the Z
    /// yield, the efficiency (with its larger MINOS error) and the fit.
    fn pass_fail_fit(
        pass: &Histogram<Double, 1>,
        fail: &Histogram<Double, 1>,
    ) -> (f64, f64, Model, Model, FitResult) {
        let width = (M_HI - M_LO) / M_BINS as f64;
        let gauss = move |m: f64, mean: f64, sigma: f64| {
            width * (-0.5 * ((m - mean) / sigma).powi(2)).exp()
                / (sigma * std::f64::consts::TAU.sqrt())
        };
        let pass_model = Model::new(
            "pass",
            &["N", "eff", "mass", "sigma", "bp0", "bp1"],
            vec![3000.0, 0.5, 91.0, 3.0, 3.0, -0.04],
            move |m, p| p[1] * p[0] * gauss(m, p[2], p[3]) + (p[4] + p[5] * (m - M_LO)).exp(),
        )
        .limit("eff", 0.0, 1.0)
        .limit("sigma", 1.0, 6.0);
        let fail_model = Model::new(
            "fail",
            &["N", "eff", "mass", "sigma", "bf0", "bf1"],
            vec![3000.0, 0.5, 91.0, 3.0, 3.0, -0.04],
            move |m, p| {
                (1.0 - p[1]) * p[0] * gauss(m, p[2], p[3]) + (p[4] + p[5] * (m - M_LO)).exp()
            },
        );
        let opts = FitOptions::new().method(FitMethod::Likelihood);
        let sim = Simultaneous::new()
            .channel("pass", pass, &pass_model, &opts)
            .channel("fail", fail, &fail_model, &opts);
        let fit = sim.fit(&opts.minos(true));
        // An efficiency near 0 or 1 has a one-sided interval: take the larger side.
        let (lo, hi) = fit.minos.as_ref().map_or((0.0, 0.0), |m| m[1]);
        let err = fit.errors[1].max(lo.abs()).max(hi.abs());
        // The fitted curves, per category, in their own parameter order.
        let names = sim.param_names();
        let pick = |model: &Model| -> Vec<f64> {
            model
                .param_names()
                .iter()
                .map(|n| {
                    let i = names.iter().position(|m| m == n).expect("a parameter");
                    fit.params[i]
                })
                .collect()
        };
        let (pf, ff) = (
            pass_model.clone().with_params(pick(&pass_model)),
            fail_model.clone().with_params(pick(&fail_model)),
        );
        (fit.params[1], err, pf, ff, fit)
    }

    /// The turn-on model.
    fn turn_on() -> Model {
        Model::new(
            "turn-on",
            &["threshold", "resolution", "plateau"],
            vec![22.0, 3.0, 0.9],
            efficiency,
        )
        .limit("plateau", 0.0, 1.0)
        .lower_limit("resolution", 0.1)
    }

    pub(super) fn run() -> oxiroot::Result<()> {
        let start = std::time::Instant::now();
        let dir = std::env::temp_dir().join("oxiroot-tag-and-probe");
        std::fs::create_dir_all(&dir)?;

        // ---- 1. The probes --------------------------------------------------
        let mut rng = Rng(7);
        let [pt, eta, mass, passed] = generate(&mut rng);
        let tree_path = dir.join("probes.root");
        let probes = Batch::new()
            .column("pt", pt)?
            .column("abs_eta", eta)?
            .column("m_mumu", mass)?
            .column("passed", passed)?;
        let mut out = DatasetWriter::create(&tree_path, "probes", Format::Tree)?;
        out.write(probes)?;
        out.finish()?;
        let file = FileReader::open(&tree_path)?;
        let tree = Dataset::open(&file, "probes")?;
        let data = tree.read(["pt", "abs_eta", "m_mumu", "passed"])?;
        let column =
            |name: &str| -> oxiroot::Result<Vec<f64>> { Ok(data.values::<f64>(name)?.to_vec()) };
        let (pt, eta, mass, passed) = (
            column("pt")?,
            column("abs_eta")?,
            column("m_mumu")?,
            column("passed")?,
        );
        println!(
            "1. Wrote and read back {} probes — {}",
            tree.entries(),
            tree_path.display()
        );

        // ---- 2. Pass/fail fits per (pT, |η|) bin ------------------------------
        let mut map = Hist::var(&PT_EDGES)
            .var(&ETA_EDGES)
            .weight()
            .named("efficiency_map")
            .titled("Trigger efficiency;probe p_{T} [GeV];probe |#eta|");
        let mut per_region: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); 2];
        let mut shown = None;
        println!("\n2. Simultaneous pass/fail fits: ε per (pT, |η|) bin");
        println!("   pT bin (GeV)    barrel ε          endcap ε");
        for (ip, p) in PT_EDGES.windows(2).enumerate() {
            let mut row = String::new();
            for (r, e) in ETA_EDGES.windows(2).enumerate() {
                let new = |name: &str| Hist::reg(M_BINS, M_LO, M_HI).double().named(name);
                let (mut h_pass, mut h_fail) = (new("pass"), new("fail"));
                for i in 0..pt.len() {
                    if (p[0]..p[1]).contains(&pt[i]) && (e[0]..e[1]).contains(&eta[i]) {
                        if passed[i] > 0.5 {
                            h_pass.fill(mass[i]);
                        } else {
                            h_fail.fill(mass[i]);
                        }
                    }
                }
                let (eps, err, pf, ff, _) = pass_fail_fit(&h_pass, &h_fail);
                // The cell holds the fitted efficiency and its variance.
                *map.bin_mut([bin(ip), bin(r)]).expect("in range") = WeightedSum {
                    value: eps,
                    variance: err * err,
                };
                per_region[r].0.push(eps);
                per_region[r].1.push(err);
                let _ = write!(row, "   {eps:.4} ± {err:.4}"); // a String never fails
                if p[0] == 24.0 && r == 0 {
                    shown = Some((h_pass, h_fail, pf, ff));
                }
            }
            println!("   [{:>4.0}, {:>4.0}) {row}", p[0], p[1]);
        }

        // ---- 3. The turn-ons ---------------------------------------------------
        let centres: Vec<f64> = PT_EDGES.windows(2).map(|p| 0.5 * (p[0] + p[1])).collect();
        let mut turn_ons = Vec::new();
        println!(
            "\n3. Turn-on fits (χ², bin-averaged)   threshold        resolution       plateau"
        );
        for (r, (eff, err)) in per_region.iter().enumerate() {
            let points = Points::new(&centres, eff, err)
                .expect("lengths")
                .with_bin_edges(&PT_EDGES)
                .expect("edges");
            let fit = points.fit_opts(&turn_on(), &FitOptions::new().bin_integration(true));
            println!(
                "   {:<33} {:.2} ± {:.2} ({})   {:.2} ± {:.2} ({})   {:.3} ± {:.3} ({})",
                ["barrel |η| < 1.2", "endcap 1.2 < |η| < 2.4"][r],
                fit.params[0],
                fit.errors[0],
                TRUE[r][0],
                fit.params[1],
                fit.errors[1],
                TRUE[r][1],
                fit.params[2],
                fit.errors[2],
                TRUE[r][2]
            );
            turn_ons.push(fit);
        }

        // ---- 4. A matched sample: TEfficiency, binomial vs χ² ------------------
        let mut matched = Efficiency::new(40, 15.0, 35.0)
            .named("matched_barrel")
            .titled("Matched probes, barrel;p_{T} [GeV];efficiency");
        let x: Vec<f64> = (0..40).map(|i| 15.25 + 0.5 * f64::from(i)).collect();
        for &c in &x {
            for _ in 0..25 {
                matched.fill(rng.uniform() < efficiency(c, &TRUE[0]), c);
            }
        }
        let (passed_m, total_m): (Vec<f64>, Vec<f64>) = (1..=40)
            .map(|b| {
                (
                    matched.passed().contents()[b],
                    matched.total().contents()[b],
                )
            })
            .unzip();
        let binomial = BinomialData::new(&x, &passed_m, &total_m)
            .expect("lengths")
            .fit(&turn_on(), &FitOptions::new());
        println!(
            "\n4. Matched sample (25 probes per 0.5 GeV), binomial likelihood: threshold {:.2} ± {:.2}",
            binomial.params[0], binomial.errors[0]
        );
        let ntoys = 200;
        let (mut pulls_binomial, mut pulls_chi2) = (Vec::new(), Vec::new());
        for _ in 0..ntoys {
            let k: Vec<f64> = x
                .iter()
                .map(|&c| {
                    (0..25)
                        .filter(|_| rng.uniform() < efficiency(c, &TRUE[0]))
                        .count() as f64
                })
                .collect();
            let b = BinomialData::new(&x, &k, &vec![25.0; x.len()])
                .expect("lengths")
                .fit(&turn_on(), &FitOptions::new());
            // χ² on k/n with the binomial error, which vanishes at k = 0 or n:
            // those bins are dropped, as a χ² must.
            let keep: Vec<usize> = (0..x.len())
                .filter(|&i| k[i] > 0.0 && k[i] < 25.0)
                .collect();
            let (cx, cy, ce): (Vec<f64>, Vec<f64>, Vec<f64>) = (
                keep.iter().map(|&i| x[i]).collect(),
                keep.iter().map(|&i| k[i] / 25.0).collect(),
                keep.iter()
                    .map(|&i| (k[i] / 25.0 * (1.0 - k[i] / 25.0) / 25.0).sqrt())
                    .collect(),
            );
            let c = Points::new(&cx, &cy, &ce).expect("lengths").fit(&turn_on());
            for (f, pulls) in [(&b, &mut pulls_binomial), (&c, &mut pulls_chi2)] {
                if f.valid && f.params[2] < 1.0 - 1e-4 {
                    pulls.push((f.params[0] - TRUE[0][0]) / f.errors[0]);
                }
            }
        }
        let stats = |p: &[f64]| {
            let n = p.len() as f64;
            let mean = p.iter().sum::<f64>() / n;
            (
                mean,
                (p.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt(),
            )
        };
        let ((bm, bw), (cm, cw)) = (stats(&pulls_binomial), stats(&pulls_chi2));
        println!(
            "   {ntoys} toys, threshold pulls: binomial mean {bm:+.2} width {bw:.2}; \
             χ² mean {cm:+.2} width {cw:.2}"
        );

        // ---- 5. Plots and the results file -------------------------------------
        let red = Color::hex("#bd1f01");
        let blue = Color::hex("#3f90da");

        // One bin's pass and fail fits.
        let (h_pass, h_fail, pf, ff) = shown.expect("the 24–26 GeV barrel bin");
        let mut ax = Axes::with_style(Style::mplhep());
        for (h, model, color, label) in [
            (&h_pass, &pf, blue, "passing"),
            (&h_fail, &ff, red, "failing"),
        ] {
            ax.hist_with(
                h,
                HistOpts::new()
                    .histtype(HistType::Errorbar)
                    .color(color)
                    .label(format!("{label} probes")),
            );
            ax.model_with(
                model,
                M_LO..M_HI,
                CurveOpts::new().color(color).samples(300),
            );
        }
        ax.xlabel("$m_{\\mu\\mu}$ [GeV]")
            .ylabel("Pairs / GeV")
            .hep_label("Demo", "Simulation")
            .legend();
        ax.text_with(
            0.04,
            0.92,
            "$24 < p_T < 26$ GeV, $|\\eta| < 1.2$",
            TextOpts::new()
                .axes_coords(true)
                .align(HAlign::Left, VAlign::Top),
        );
        let fits_png = dir.join("pass_fail_fit.png");
        ax.save(&fits_png)?;

        // The efficiency map, each cell labelled.
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist2d_with(
            &map,
            Hist2dOpts::new()
                .cmap(Colormap::Viridis)
                .vrange(0.0..1.0)
                .label("Efficiency"),
        );
        for (ip, p) in PT_EDGES.windows(2).enumerate() {
            for (ie, e) in ETA_EDGES.windows(2).enumerate() {
                let (cx, cy) = (0.5 * (p[0] + p[1]), 0.5 * (e[0] + e[1]));
                let v = map.value([bin(ip), bin(ie)]).expect("in range");
                ax.text_with(
                    cx,
                    cy,
                    format!("{:.0}", 100.0 * v),
                    TextOpts::new()
                        .size(7.0)
                        .color(if v > 0.6 { Color::BLACK } else { Color::WHITE })
                        .align(HAlign::Center, VAlign::Middle),
                );
            }
        }
        ax.xlabel("Probe $p_T$ [GeV]")
            .ylabel("Probe $|\\eta|$")
            .hep_label("Demo", "Simulation")
            .hep_rhs("cell labels in %");
        let map_png = dir.join("efficiency_map.png");
        ax.save(&map_png)?;

        // The turn-ons.
        let half: Vec<f64> = PT_EDGES.windows(2).map(|p| 0.5 * (p[1] - p[0])).collect();
        let mut ax = Axes::with_style(Style::mplhep());
        let mut graphs = Vec::new();
        for (r, ((eff, err), fit)) in per_region.iter().zip(&turn_ons).enumerate() {
            let color = [blue, red][r];
            let label = ["$|\\eta| < 1.2$", "$1.2 < |\\eta| < 2.4$"][r];
            let g = Graph::new(centres.clone(), eff.clone())
                .and_then(|g| {
                    g.with_errors(oxiroot::hist::GraphErrors::Symmetric {
                        ex: half.clone(),
                        ey: err.clone(),
                    })
                })?
                .named(["turn_on_barrel", "turn_on_endcap"][r]);
            ax.errorbar_with(
                &g,
                oxiroot::plot::ErrorbarOpts::new().color(color).label(label),
            );
            let curve = turn_on().with_params(fit.params.clone());
            ax.model_with(
                &curve,
                14.0..60.0,
                CurveOpts::new().color(color).samples(300),
            );
            graphs.push(g);
        }
        ax.ylim(0.0..1.1)
            .xlabel("Probe $p_T$ [GeV]")
            .ylabel("Trigger efficiency")
            .hep_label("Demo", "Simulation")
            .legend();
        let turn_on_png = dir.join("turn_on.png");
        ax.save(&turn_on_png)?;

        // The toys' threshold pulls.
        let pull_hist = |name: &str, p: &[f64]| {
            let mut h = Hist::reg(24, -4.0, 4.0).double().named(name);
            p.iter().for_each(|&v| h.fill(v));
            h
        };
        let mut ax = Axes::with_style(Style::mplhep());
        ax.hist_with(
            &pull_hist("binomial", &pulls_binomial),
            HistOpts::new()
                .color(blue)
                .label(format!("binomial (mean {bm:+.2}, width {bw:.2})")),
        );
        ax.hist_with(
            &pull_hist("chi2", &pulls_chi2),
            HistOpts::new()
                .color(red)
                .label(format!("χ² (mean {cm:+.2}, width {cw:.2})")),
        );
        ax.xlabel("Threshold pull")
            .ylabel("Toys")
            .hep_label("Demo", "Simulation")
            .legend();
        let pulls_png = dir.join("binomial_vs_chi2.png");
        ax.save(&pulls_png)?;

        let results = dir.join("efficiency_results.root");
        FileWriter::create(&results)
            .add(&map.to_root()?)
            .add(&graphs[0])
            .add(&graphs[1])
            .add(&matched)
            .write()?;
        let back = FileReader::open(&results)?.get::<Hist2D>("efficiency_map")?;
        println!(
            "\n5. Results file {} (efficiency map: {} × {} bins read back)",
            results.display(),
            back.nx(),
            back.ny()
        );
        for png in [&fits_png, &map_png, &turn_on_png, &pulls_png] {
            println!("   plot: {}", png.display());
        }
        println!("\n(total {:.1?})", start.elapsed());
        Ok(())
    }
}
