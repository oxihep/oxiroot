//! Plotting with oxiroot (the `plot` feature) — render ROOT histograms and
//! graphs to SVG/PNG with a matplotlib-like API and an mplhep histogram style.
//! Run with:
//!
//! ```sh
//! cargo run -p oxiroot --example plot --features plot
//! ```
//!
//! It renders these figures into `oxiroot-plots` in the temporary directory,
//! most as PNG, SVG, and PDF:
//!
//! 1. `mass` — a filled MC template with "data" points overlaid, a legend, and a
//!    LaTeX axis label (the default matplotlib look). Also saved at 220 DPI.
//! 2. `mplhep` — the same histogram as a step staircase with error bars in the
//!    mplhep style (in-pointing ticks, minors, all four sides), with a bold
//!    `Demo Simulation` experiment-style label and luminosity/energy above the frame,
//!    a fitted Gaussian, and a ROOT-style fit stat box (with the `fit` feature).
//! 3. `heatmap` — a 2-D `Hist2D` as a viridis color mesh with a colorbar.
//! 4. `ratio` — a main panel over a data/MC ratio panel sharing the x-axis.
//! 5. `grid` — a 2×2 grid of panels sharing their axes, with a figure title.
//! 6. `fit` — a fitted Gaussian with a ROOT-style stat box (the `fit` feature).
//! 7. `font-stix` and `font-dejavu` — the MC template in the default STIX Two
//!    fonts and in the matplotlib `DejaVu` Sans look.
//!
//! The plotting guide's images are these files, prefixed with `plot-`.

#[cfg(not(feature = "plot"))]
fn main() {
    eprintln!("re-run with `--features plot` to render the figures");
}

#[cfg(feature = "plot")]
fn main() -> oxiroot::Result<()> {
    #[cfg(feature = "fit")]
    use oxiroot::fit::{FitExt, Model};
    use oxiroot::plot::{
        Axes, Color, ErrorbarOpts, FontSet, Hist2dOpts, HistOpts, HistType, SaveOpts, Style,
        ratio_subplots,
    };
    use oxiroot::prelude::*;

    let out = std::env::temp_dir().join("oxiroot-plots");
    std::fs::create_dir_all(&out)?;

    // A deterministic Gaussian-filled di-muon mass histogram (Sumw2 on).
    let mut mc = Hist::reg(40, 50.0, 130.0)
        .weight()
        .named("mass")
        .titled("di-muon mass");
    let mut rng = Lcg::new(0x1234_5678_9abc_def0);
    for _ in 0..40_000 {
        mc.fill(91.0 + 7.5 * rng.gauss());
    }

    // The simulation has ten times the data's events: scale it to the data.
    mc.scale(0.1);

    // "Data": 4000 events of the same shape with their own fluctuations, in
    // the simulation's binning, so the two compare bin by bin.
    let mut data = Hist::reg(40, 50.0, 130.0).weight().named("data");
    let mut rng = Lcg::new(0x0fed_cba9_8765_4321);
    for _ in 0..4_000 {
        data.fill(91.0 + 7.5 * rng.gauss());
    }

    // --- 1. Filled MC + data overlay, default matplotlib look. ---
    let mut ax = Axes::new();
    ax.hist_with(
        &mc,
        HistOpts::new()
            .histtype(HistType::Fill)
            .fill_color(Color::hex("#1f77b4").with_alpha(0.4))
            .label("MC"),
    );
    ax.hist_with(
        &data,
        HistOpts::new()
            .histtype(HistType::Errorbar)
            .color(Color::BLACK)
            .label("data"),
    );
    ax.xlabel("$m_{\\mu\\mu}$ [GeV]");
    ax.ylabel("Events / 2 GeV");
    ax.title("$Z \\rightarrow \\mu\\mu$ candidates");
    ax.legend();
    save_both(&ax, &out, "mass")?;

    // Also save the first figure at a higher DPI for a sharper raster.
    ax.save_with(out.join("mass_hi.png"), SaveOpts::new().dpi(220.0))?;
    println!("wrote {} (220 dpi)", out.join("mass_hi.png").display());

    // --- 2. The same histogram as a step + error bars, mplhep style, with a
    //        an experiment-style label and luminosity/energy above the frame.
    //        A collaboration's name belongs only on its own results, so a
    //        demo uses a neutral one. ---
    let mut hep = Axes::with_style(Style::mplhep());
    hep.hist_with(&mc, HistOpts::new().yerr(true).label("MC"));
    hep.hep_label("Demo", "Simulation")
        .hep_rhs("138 fb$^{-1}$ (13 TeV)");
    hep.xlabel("$m_{\\mu\\mu}$ [GeV]");
    hep.ylabel("Events / 2 GeV");
    // Fit the peak and add a ROOT-style stat box in the HEP style, too. The box
    // adopts the mplhep frame colour/weight and stacks under the legend.
    #[cfg(feature = "fit")]
    {
        let model = Model::gaussian("gaus").estimate_from(&mc);
        let r = mc.fit(&model);
        let fitted = model.with_params(r.params.clone());
        hep.model_with(
            &fitted,
            50.0..130.0,
            oxiroot::plot::CurveOpts::new()
                .color(Color::hex("#d62728"))
                .linewidth(2.0)
                .label("fit"),
        );
        hep.fit_stats(&fitted, &r);
    }
    hep.legend();
    save_both(&hep, &out, "mplhep")?;

    // --- 3. A 2-D Hist2D as a viridis heatmap with a colorbar. ---
    let mut h2 = Hist::reg(40, -4.0, 4.0)
        .reg(40, -4.0, 4.0)
        .weight()
        .named("h2");
    for ix in 0..40 {
        for iy in 0..40 {
            let x = -4.0 + (f64::from(ix) + 0.5) * 0.2;
            let y = -4.0 + (f64::from(iy) + 0.5) * 0.2;
            let z = (-(x * x + y * y) / 2.0).exp() * 100.0
                + 50.0 * (-((x - 1.5).powi(2) + (y + 1.2).powi(2)) / 0.5).exp();
            h2.fill_weight(x, y, z);
        }
    }
    let mut ax2 = Axes::new();
    ax2.hist2d_with(&h2, Hist2dOpts::new().label("entries"));
    ax2.xlabel("$x$");
    ax2.ylabel("$y$");
    ax2.title("two Gaussians");
    save_both(&ax2, &out, "heatmap")?;

    // --- 4. A ratio plot: filled MC + data over a data/MC ratio panel. ---
    let (fig, mut main, mut ratio) = ratio_subplots();
    main.hist_with(
        &mc,
        HistOpts::new()
            .histtype(HistType::Fill)
            .fill_color(Color::hex("#1f77b4").with_alpha(0.4))
            .label("MC"),
    );
    main.hist_with(
        &data,
        HistOpts::new()
            .histtype(HistType::Errorbar)
            // Counts carry Poisson (Garwood) intervals, not ±√n.
            .poisson_errors(true)
            .color(Color::BLACK)
            .label("data"),
    );
    main.ylabel("Events / 2 GeV");
    main.legend();
    // The frame stops where the simulation runs out of events: beyond it a
    // ratio says nothing.
    main.xlim(70.0..112.0);
    ratio.xlim(70.0..112.0);
    // The ratio in every bin both fill, with the data's uncertainty; the
    // simulation's own is the hatched band around one.
    ratio.hist_with(
        &oxiroot::plot::Binned1D::relative_uncertainty(&mc),
        HistOpts::new()
            .histtype(HistType::Band)
            .hatch(oxiroot::plot::Hatch::Diagonal)
            .color(Color::hex("#555555")),
    );
    ratio.errorbar_with(
        &oxiroot::plot::XyPoints::ratio(&data, &mc)?,
        ErrorbarOpts::new().color(Color::BLACK),
    );
    ratio.axhline_with(
        1.0,
        oxiroot::plot::CurveOpts::new()
            .color(Color::hex("#888888"))
            .dashed(vec![4.0, 3.0]),
    );
    ratio.ylim(0.4..1.6);
    ratio.yticks(&[0.5, 1.0, 1.5]);
    ratio.ylabel("data/MC");
    ratio.xlabel("$m_{\\mu\\mu}$ [GeV]");
    let figr = fig.ratio(main, ratio);
    for ext in ["png", "svg", "pdf"] {
        let path = out.join(format!("ratio.{ext}"));
        figr.save(&path)?;
        println!(
            "wrote {} ({} bytes)",
            path.display(),
            std::fs::metadata(&path)?.len()
        );
    }

    // --- 5. A shared-axis 2x2 subplot grid with a figure title. ---
    use oxiroot::plot::subplots_grid;
    let (gfig, mut axs) = subplots_grid(2, 2);
    axs[0].hist(&mc);
    axs[0].ylabel("Events");
    axs[1].hist_with(&mc, HistOpts::new().histtype(HistType::Fill));
    axs[2].hist_with(&data, HistOpts::new().histtype(HistType::Errorbar));
    axs[2].ylabel("Events");
    let edges = data.axis(0).edges().expect("a numeric axis");
    let centres: Vec<f64> = edges.windows(2).map(|e| 0.5 * (e[0] + e[1])).collect();
    axs[3].plot(&centres, &data.values(false))?;
    let gfig = gfig
        .sharex(true)
        .sharey(true)
        .suptitle("$Z \\rightarrow \\mu\\mu$ — subplot grid")
        .with_axes(axs);
    gfig.save(out.join("grid.png"))?;
    println!("wrote {}", out.join("grid.png").display());

    // --- 6. Fit a Gaussian to the MC and overlay the fitted curve. ---
    // (Needs `--features plot,fit`; skipped otherwise.)
    #[cfg(feature = "fit")]
    {
        let model = Model::gaussian("gaus").estimate_from(&mc);
        let r = mc.fit(&model);
        let fitted = model.with_params(r.params.clone());
        let mut ax = Axes::new();
        ax.hist_with(
            &mc,
            HistOpts::new()
                .histtype(HistType::Fill)
                .fill_color(Color::hex("#1f77b4").with_alpha(0.4))
                .label("data"),
        );
        ax.model_with(
            &fitted,
            50.0..130.0,
            oxiroot::plot::CurveOpts::new()
                .color(Color::hex("#d62728"))
                .linewidth(2.0)
                .label("Gaussian fit"),
        );
        // A ROOT-style stat box: the function name, chi2/ndf, and each fitted
        // parameter with its error. Anchored top-right; because a legend is also
        // shown there, the box is stacked directly beneath it.
        ax.fit_stats(&fitted, &r);
        ax.xlabel("$m_{\\mu\\mu}$ [GeV]");
        ax.ylabel("Events / 2 GeV");
        ax.legend();
        save_both(&ax, &out, "fit")?;
    }

    // --- 7. The MC template in each font set, for the guide's Fonts section. ---
    for (stem, title, fonts) in [
        ("font-stix", "STIX Two (default)", FontSet::stix()),
        ("font-dejavu", "DejaVu Sans", FontSet::dejavu()),
    ] {
        let mut ax = Axes::new();
        ax.fonts(fonts);
        ax.hist_with(
            &mc,
            HistOpts::new()
                .histtype(HistType::Fill)
                .fill_color(Color::hex("#1f77b4").with_alpha(0.4))
                .label("MC"),
        );
        ax.xlabel("$m_{\\mu\\mu}$ [GeV]");
        ax.ylabel("Events / 2 GeV");
        ax.title(title);
        ax.legend();
        save_both(&ax, &out, stem)?;
    }

    Ok(())
}

#[cfg(feature = "plot")]
fn save_both(ax: &oxiroot::plot::Axes, dir: &std::path::Path, stem: &str) -> oxiroot::Result<()> {
    for ext in ["png", "svg", "pdf"] {
        let path = dir.join(format!("{stem}.{ext}"));
        ax.save(&path)?;
        let len = std::fs::metadata(&path)?.len();
        assert!(len > 0, "{} is empty", path.display());
        println!("wrote {} ({len} bytes)", path.display());
    }
    Ok(())
}

/// A tiny linear-congruential generator with a 12-uniform Gaussian, so the
/// figures are byte-for-byte reproducible without an RNG dependency.
#[cfg(feature = "plot")]
struct Lcg(u64);

#[cfg(feature = "plot")]
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as f64 / (1u64 << 31) as f64
    }
    fn gauss(&mut self) -> f64 {
        (0..12).map(|_| self.unit()).sum::<f64>() - 6.0
    }
}
