//! A template fit with systematic uncertainties — the `HistFactory` recipe of a
//! search — from ROOT histograms to the plots a search shows:
//!
//! 1. **Write the inputs** to a ROOT file, as a production step would: per
//!    region and sample the simulated template (its `fSumw2` the simulation's
//!    statistical errors), its jet-energy-scale up and down variations, and
//!    the data. **Read them back** to build the model.
//! 2. **Build the model**: a signal region (SR) and a control region (CR); a
//!    signal of strength `mu`, top pairs normalized in the CR (`mu_ttbar`),
//!    W+jets from simulation with a 10 % cross-section uncertainty; the jet
//!    energy scale as a shape uncertainty on every background, the templates'
//!    finite statistics per bin, and a 2 % luminosity uncertainty.
//! 3. **Fit**: `mu` with MINOS, the nuisance-parameter pulls, and their
//!    impact ranking on `mu`.
//! 4. **Test**: the observed and expected significance, and the 95 % CL upper
//!    limit with its expected band.
//! 5. **Plot** the post-fit stack with the data and the uncertainty band, the
//!    pulls and the impacts, and **write** the post-fit histograms to a ROOT
//!    file.
//!
//! The data are generated (fixed seed) with `mu = 0.8`. Run with:
//!
//! ```sh
//! cargo run --release -p oxiroot --example template_fit
//! ```

#[cfg(not(all(feature = "fit", feature = "plot")))]
fn main() {
    eprintln!("this example needs the `fit` and `plot` features (both on by default)");
}

#[cfg(all(feature = "fit", feature = "plot"))]
fn main() -> oxiroot::Result<()> {
    templates::run()
}

#[cfg(all(feature = "fit", feature = "plot"))]
mod templates {
    use oxiroot::fit::{Channel, Sample};
    use oxiroot::plot::{
        Axes, Color, CurveOpts, FillOpts, HistOpts, HistType, StackOpts, Style, ratio_subplots_with,
    };
    use oxiroot::prelude::*;

    const SR_BINS: usize = 8;
    const CR_BINS: usize = 6;
    /// The signal strength the data are generated with.
    const TRUE_MU: f64 = 0.8;

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
        /// A Poisson draw (inversion below 200, a normal approximation above).
        fn poisson(&mut self, mean: f64) -> f64 {
            if mean > 200.0 {
                let (u1, u2) = (self.uniform(), self.uniform());
                let g = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
                return (mean + mean.sqrt() * g).round().max(0.0);
            }
            let (mut k, mut p, limit) = (0.0, 1.0, (-mean).exp());
            loop {
                p *= self.uniform();
                if p <= limit {
                    return k;
                }
                k += 1.0;
            }
        }
    }

    /// Expected counts per bin of a discriminant in `[0, 1)`: `norm` events
    /// with a density `∝ exp(slope·x)`.
    fn shape(bins: usize, norm: f64, slope: f64) -> Vec<f64> {
        let total = (slope.exp() - 1.0) / slope;
        (0..bins)
            .map(|b| {
                let (a, c) = (b as f64 / bins as f64, (b + 1) as f64 / bins as f64);
                norm * ((slope * c).exp() - (slope * a).exp()) / slope / total
            })
            .collect()
    }

    /// The jet energy scale moved by `s` σ: a few per cent toward higher
    /// discriminant values.
    fn jes(nominal: &[f64], s: f64) -> Vec<f64> {
        let n = nominal.len() as f64;
        nominal
            .iter()
            .enumerate()
            .map(|(b, &v)| v * (1.0 + s * 0.06 * (b as f64 / (n - 1.0) - 0.5)))
            .collect()
    }

    /// A ROOT histogram on `[0, 1)` holding `values`, with per-bin errors²
    /// `errors2` (the simulation's statistics) when given.
    fn histogram(name: &str, values: &[f64], errors2: Option<&[f64]>) -> Hist1D {
        let mut h = Hist::reg(values.len(), 0.0, 1.0)
            .double()
            .to_root()
            .unwrap()
            .named(name)
            .titled(format!("{name};discriminant;Events"));
        h.contents_mut()[1..=values.len()].copy_from_slice(values);
        if let Some(e2) = errors2 {
            h.track_sumw2();
            h.sumw2_mut().expect("error tracking is on")[1..=values.len()].copy_from_slice(e2);
        }
        h.set_entries(values.iter().sum());
        h
    }

    /// The in-range contents of `h`.
    fn values(h: &Hist1D) -> Vec<f64> {
        let n = h.contents().len() - 2;
        h.contents()[1..=n].to_vec()
    }

    /// The in-range errors of `h`.
    fn errors(h: &Hist1D) -> Vec<f64> {
        (1..h.contents().len() - 1)
            .map(|i| h.bin_error(i))
            .collect()
    }

    pub(super) fn run() -> oxiroot::Result<()> {
        let start = std::time::Instant::now();
        let dir = std::env::temp_dir().join("oxiroot-template-fit");
        std::fs::create_dir_all(&dir)?;

        // ---- 1. The inputs, written as a production step would ---------------
        let inputs = dir.join("templates.root");
        {
            let signal = shape(SR_BINS, 30.0, 3.0);
            let mut templates = vec![("SR_signal".to_string(), signal.clone())];
            for (region, bins, tt, wj) in [
                ("SR", SR_BINS, (160.0, -1.5), (120.0, -2.5)),
                ("CR", CR_BINS, (900.0, -0.8), (150.0, -1.2)),
            ] {
                templates.push((format!("{region}_ttbar"), shape(bins, tt.0, tt.1)));
                templates.push((format!("{region}_wjets"), shape(bins, wj.0, wj.1)));
            }
            // The data: mu = 0.8, the top normalization 1.15, W+jets +0.5σ.
            let mut rng = Rng(42);
            let get = |n: &str| -> &Vec<f64> {
                &templates
                    .iter()
                    .find(|(m, _)| m == n)
                    .expect("a template")
                    .1
            };
            let data = |region: &str, signal: Option<&[f64]>, rng: &mut Rng| -> Vec<f64> {
                let (tt, wj) = (
                    get(&format!("{region}_ttbar")),
                    get(&format!("{region}_wjets")),
                );
                (0..tt.len())
                    .map(|b| {
                        let s = signal.map_or(0.0, |s| TRUE_MU * s[b]);
                        rng.poisson(s + 1.15 * tt[b] + 1.05 * wj[b])
                    })
                    .collect()
            };
            let sr_data = data("SR", Some(&signal), &mut rng);
            let cr_data = data("CR", None, &mut rng);
            let mut writer = FileWriter::create(&inputs);
            for (name, nominal) in &templates {
                // Each template from a simulated sample ~4× the data.
                let mc2: Vec<f64> = nominal.iter().map(|v| v / 4.0).collect();
                writer = writer.add(&histogram(name, nominal, Some(&mc2)));
                if !name.ends_with("signal") {
                    writer = writer
                        .add(&histogram(
                            &format!("{name}_jes_up"),
                            &jes(nominal, 1.0),
                            None,
                        ))
                        .add(&histogram(
                            &format!("{name}_jes_down"),
                            &jes(nominal, -1.0),
                            None,
                        ));
                }
            }
            writer
                .add(&histogram("SR_data", &sr_data, None))
                .add(&histogram("CR_data", &cr_data, None))
                .write()?;
        }
        let file = FileReader::open(&inputs)?;
        let read = |name: &str| file.get::<Hist1D>(name);
        println!(
            "1. Wrote and read back the templates and data — {}",
            inputs.display()
        );

        // ---- 2. The model, from the histograms -------------------------------
        let background = |region: &str, sample: &str| -> oxiroot::Result<Sample> {
            let name = format!("{region}_{sample}");
            let nominal = read(&name)?;
            let mut s = Sample::new(sample, &values(&nominal))
                .histo_sys(
                    "jes",
                    &values(&read(&format!("{name}_jes_down"))?),
                    &values(&read(&format!("{name}_jes_up"))?),
                )
                .lumi("lumi");
            s = if sample == "ttbar" {
                s.norm_factor("mu_ttbar")
            } else {
                s.norm_sys("wjets_xsec", 0.9, 1.1)
            };
            Ok(s)
        };
        // The MC-statistics uncertainty on each region's total background.
        let mc_stat = |region: &str| -> oxiroot::Result<Vec<f64>> {
            let (tt, wj) = (
                read(&format!("{region}_ttbar"))?,
                read(&format!("{region}_wjets"))?,
            );
            Ok(errors(&tt)
                .iter()
                .zip(errors(&wj))
                .map(|(a, b)| a.hypot(b))
                .collect())
        };
        let sr_data = values(&read("SR_data")?);
        let cr_data = values(&read("CR_data")?);
        let model = TemplateModel::new(vec![
            Channel::new("SR", &sr_data)
                .sample(
                    Sample::new("signal", &values(&read("SR_signal")?))
                        .norm_factor("mu")
                        .lumi("lumi"),
                )
                .sample(background("SR", "ttbar")?.stat_error("mc_SR", &mc_stat("SR")?))
                .sample(background("SR", "wjets")?),
            Channel::new("CR", &cr_data)
                .sample(background("CR", "ttbar")?.stat_error("mc_CR", &mc_stat("CR")?))
                .sample(background("CR", "wjets")?),
        ])
        .and_then(|m| m.lumi_uncertainty(0.02))
        .expect("a valid model");
        println!(
            "\n2. Model: {} channels, {} parameters",
            model.channels().len(),
            model.param_names().len()
        );

        // ---- 3. The fit, pulls and impacts -----------------------------------
        let fitter = model.fitter();
        let fit = fitter.fit(&FitOptions::new().minos(true));
        let names = model.param_names();
        let idx = |n: &str| names.iter().position(|m| m == n).expect("a parameter");
        let (i_mu, i_tt) = (idx("mu"), idx("mu_ttbar"));
        println!(
            "\n3. Fit: mu = {:.3} ± {:.3} (truth {TRUE_MU}), mu_ttbar = {:.3} ± {:.3} (truth 1.15)",
            fit.params[i_mu], fit.errors[i_mu], fit.params[i_tt], fit.errors[i_tt]
        );
        let pulls = model.pulls(&fit);
        let impacts = model.impacts(&fitter, &fit, "mu");
        println!("   largest impacts on mu (post-fit up / down):");
        for imp in impacts.iter().take(4) {
            println!(
                "     {:<12} {:>+7.3} {:>+7.3}",
                imp.name, imp.postfit.0, imp.postfit.1
            );
        }

        // ---- 4. Significance and limit ---------------------------------------
        let test = HypoTest::new(model.fitter(), "mu").expect("mu is free");
        let d = test.discovery(1.0).expect("discovery test");
        let limit = test.upper_limit(0.95, (0.0, 10.0)).expect("limit");
        println!(
            "\n4. Signal significance: observed Z = {:.2}, expected for mu = 1: {:.2}",
            d.z, d.z_expected
        );
        println!(
            "   95 % CL upper limit on mu: observed {:.3}, expected {:.3} [{:.3}, {:.3}]",
            limit.observed, limit.expected[2], limit.expected[1], limit.expected[3]
        );

        // ---- 5. Plots and the post-fit file ----------------------------------
        let yields = model.yields(&fit);
        let sr = &yields[0];
        let sample_hist = |name: &str| -> Hist1D {
            let v = &sr
                .samples
                .iter()
                .find(|(n, _)| n == name)
                .expect("a sample")
                .1;
            histogram(&format!("SR_postfit_{name}"), v, None)
        };
        let (h_wj, h_tt, h_sig) = (
            sample_hist("wjets"),
            sample_hist("ttbar"),
            sample_hist("signal"),
        );
        let data_hist = read("SR_data")?;
        let edges: Vec<f64> = (0..=SR_BINS).map(|b| b as f64 / SR_BINS as f64).collect();
        // The total's ±1σ band, drawn per bin as flat steps.
        let (mut bx, mut blo, mut bhi) = (Vec::new(), Vec::new(), Vec::new());
        for b in 0..SR_BINS {
            for x in [edges[b], edges[b + 1]] {
                bx.push(x);
                blo.push(sr.total[b] - sr.total_error[b]);
                bhi.push(sr.total[b] + sr.total_error[b]);
            }
        }
        let (fig, mut main, mut lower) = ratio_subplots_with(Style::mplhep());
        main.hist_stack_with(
            &[&h_wj, &h_tt, &h_sig],
            StackOpts::new()
                .histtype(HistType::Fill)
                .labels(["W+jets", "$t\\bar{t}$", "Signal"])
                .colors([
                    Color::hex("#94a4a2"),
                    Color::hex("#3f90da"),
                    Color::hex("#bd1f01"),
                ]),
        )?;
        main.fill_between_with(
            &bx,
            &blo,
            &bhi,
            FillOpts::new()
                .color(Color::BLACK)
                .alpha(0.25)
                .label("Post-fit unc."),
        )?;
        main.hist_with(
            &data_hist,
            HistOpts::new()
                .histtype(HistType::Errorbar)
                .color(Color::BLACK)
                .label("Toy data"),
        );
        main.ylabel("Events")
            .hep_label("Demo", "Simulation")
            .legend();
        let ratio_y: Vec<f64> = sr
            .observed
            .iter()
            .zip(&sr.total)
            .map(|(o, t)| o / t)
            .collect();
        let centres: Vec<f64> = (0..SR_BINS)
            .map(|b| (b as f64 + 0.5) / SR_BINS as f64)
            .collect();
        let ratio_e: Vec<f64> = sr
            .observed
            .iter()
            .zip(&sr.total)
            .map(|(o, t)| o.sqrt() / t)
            .collect();
        lower.fill_between_with(
            &bx,
            &blo.iter()
                .zip(&bhi)
                .map(|(l, h)| l / (0.5 * (l + h)))
                .collect::<Vec<_>>(),
            &bhi.iter()
                .zip(&blo)
                .map(|(h, l)| h / (0.5 * (l + h)))
                .collect::<Vec<_>>(),
            FillOpts::new().color(Color::BLACK).alpha(0.25),
        )?;
        lower.errorbar(&Graph::new(centres, ratio_y).and_then(|g| {
            g.with_errors(oxiroot::hist::GraphErrors::Symmetric {
                ex: vec![0.0; SR_BINS],
                ey: ratio_e,
            })
        })?);
        lower.axhline_with(1.0, CurveOpts::new().dashed(vec![4.0, 3.0]));
        lower
            .ylim(0.6..1.4)
            .ylabel("Data / fit")
            .xlabel("Discriminant");
        let stack_png = dir.join("postfit_sr.png");
        fig.ratio(main, lower).save(&stack_png)?;

        // Pulls of the named nuisance parameters, and the impact ranking.
        let shown: Vec<(String, f64, f64)> = pulls
            .iter()
            .filter(|(n, p, _)| !n.starts_with("mc_") || p.abs() > 0.5)
            .cloned()
            .collect();
        let mut ax = Axes::with_style(Style::mplhep());
        ax.fit_pulls(&shown).hep_label("Demo", "Simulation");
        let pulls_png = dir.join("pulls.png");
        ax.save(&pulls_png)?;
        let mut ax = Axes::with_style(Style::mplhep());
        ax.fit_impacts(&impacts, 8)
            .hep_label("Demo", "Simulation")
            .legend();
        let impacts_png = dir.join("impacts.png");
        ax.save(&impacts_png)?;

        let postfit = dir.join("postfit.root");
        FileWriter::create(&postfit)
            .add(&h_sig)
            .add(&h_tt)
            .add(&h_wj)
            .write()?;
        let back = FileReader::open(&postfit)?.get::<Hist1D>("SR_postfit_ttbar")?;
        println!(
            "\n5. Post-fit histograms in {} (ttbar: {:.1} events read back)",
            postfit.display(),
            back.integral()
        );
        for png in [&stack_png, &pulls_png, &impacts_png] {
            println!("   plot: {}", png.display());
        }
        println!("\n(total {:.1?})", start.elapsed());
        Ok(())
    }
}
