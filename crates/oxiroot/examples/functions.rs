//! Standalone `Func1D`/`Func2D`/`Func3D` functions: build them from ROOT's
//! built-in shapes (`gaus`, `expo`, `pol2`) and from a hand-written formula
//! with named parameters, evaluate them in pure Rust (`eval`), do calculus
//! (`integral`/`derivative`), scan a grid to find a peak and a crossing, see
//! what a formula with a mistake reports, then write them to a ROOT file and
//! read one back — a round-trip that ROOT C++ and uproot also read.
//!
//! ```sh
//! cargo run -p oxiroot --example functions
//! ```

use oxiroot::prelude::*;

fn main() -> oxiroot::Result<()> {
    // --- Four 1-D functions. ----------------------------------------------------
    // A hand-written formula: a decaying sine. A parameter is written in
    // brackets, by number (`[0]`) or by name (`[A]`); the variable is `x`.
    // `with_param` sets one by its name.
    let damped = Func1D::new("[A]*exp(-[gamma]*x)*sin([omega]*x)", 0.0..=10.0)?
        .named("damped")
        .with_param("A", 5.0)?
        .with_param("gamma", 0.3)?
        .with_param("omega", 2.0)?;

    // ROOT's built-in shapes, with ROOT's parameter names: `gaus` is
    // `Constant·exp(−½((x − Mean)/Sigma)²)`, `expo` is `exp(Constant + Slope·x)`,
    // `pol2` is `p0 + p1·x + p2·x²`. `with_params` sets them all, in order.
    let gauss = Func1D::gaus(-5.0..=5.0)
        .named("gauss")
        .with_params(&[2.0, 0.5, 1.2])?;
    let decay = Func1D::expo(0.0..=5.0)
        .named("decay")
        .with_params(&[1.0, -0.7])?;
    let parab = Func1D::pol(2, -3.0..=3.0)?
        .named("parab")
        .with_params(&[1.0, -2.0, 1.0])?;

    println!("Four TF1s (each formula as ROOT stores it, and its parameters):");
    for f in [&damped, &gauss, &decay, &parab] {
        // `title()` is the formula as written; `formula()` is ROOT's spelling.
        println!(
            "  {:<7} {:<42} ({})  eval(1.0) = {:+.6}",
            f.name(),
            f.formula(),
            f.param_names().join(", "),
            f.eval(1.0),
        );
    }

    // --- Calculus. --------------------------------------------------------------
    // integral(a,b) is the signed area under the curve (adaptive Gauss–Kronrod);
    // derivative(x) is the slope, exact where the formula has a closed-form
    // derivative (as all of these do), numerical where it has not.
    println!("\nCalculus on `damped` = 5·exp(-0.3x)·sin(2x):");
    println!(
        "  integral(0,10) = {:+.6}   (net signed area over the range)",
        damped.integral(0.0, 10.0)
    );
    println!(
        "  derivative(0)  = {:+.6}   (slope at x=0; here 5·2 = 10)",
        damped.derivative(0.0)
    );
    // A parabola integrates and differentiates to values you can check by hand:
    // ∫_{-1}^{1} (1 - 2x + x²) dx = 2 + 0 + 2/3 = 2.6667, and d/dx = -2 + 2x → 0 at x=1.
    println!("Check on `parab` = 1 - 2x + x²:");
    println!(
        "  integral(-1,1) = {:.6}   (analytic 8/3 = 2.666667)",
        parab.integral(-1.0, 1.0)
    );
    println!(
        "  derivative(1)  = {:.6}   (analytic -2 + 2·1 = 0)",
        parab.derivative(1.0)
    );

    // --- Numeric use: scan the grid to locate features of a callable function. -
    // Find the maximum of the damped sine by evaluating on a fine grid, and the
    // Gaussian's half-maximum crossing on its right flank (where it drops to 1.0,
    // half of its peak constant 2.0). Both are ordinary `eval` loops.
    let n = 2_000;
    let (mut x_max, mut y_max) = (0.0, f64::NEG_INFINITY);
    for i in 0..=n {
        let x = 10.0 * f64::from(i) / f64::from(n);
        let y = damped.eval(x);
        if y > y_max {
            (x_max, y_max) = (x, y);
        }
    }
    println!("\nGrid scan of `damped` over [0,10]:");
    println!("  maximum ≈ {:.4} at x ≈ {:.4}", y_max, x_max);

    let param = |name| gauss.param_named(name).expect("a `gaus` parameter");
    let (height, mean, sigma) = (param("Constant"), param("Mean"), param("Sigma"));
    let half = height / 2.0; // half of the Gaussian's peak height
    let mut crossing = f64::NAN;
    for i in 0..=n {
        let x = mean + 5.0 * f64::from(i) / f64::from(n); // walk right from the mean
        if gauss.eval(x) <= half {
            crossing = x;
            break;
        }
    }
    // For a Gaussian the half-max is sigma·sqrt(2·ln2) ≈ 1.1774·sigma to the right.
    let expected = mean + sigma * (2.0_f64 * 2.0_f64.ln()).sqrt();
    println!(
        "  `gauss` right half-max crossing ≈ {:.4}  (analytic mean + σ·√(2ln2) = {:.4})",
        crossing, expected,
    );

    // --- 2-D and 3-D functions add the y (and z) variables. --------------------
    let f2 = Func2D::new("[0]*sin(x) + [1]*y*y", -3.0..=3.0, -2.0..=2.0)?
        .named("f2")
        .with_params(&[1.5, 0.7])?;
    let f3 = Func3D::new("[0]*x + y*z", 0.0..=2.0, 0.0..=2.0, 0.0..=2.0)?
        .named("f3")
        .with_params(&[2.0])?;
    println!("\nHigher-dimensional functions:");
    println!(
        "  f2(1,1)   = {:.6}   (1.5·sin1 + 0.7·1)",
        f2.eval(1.0, 1.0)
    );
    println!("  f3(1,1,1) = {:.6}   (2·1 + 1·1)", f3.eval(1.0, 1.0, 1.0));

    // --- A formula with a mistake says what and where. --------------------------
    if let Err(e) = Func1D::new("[A]*exp(-[gamma]*x", 0.0..=10.0) {
        println!("\nA formula with a missing parenthesis:\n{e}");
    }

    // --- Write all six to a ROOT file (ROOT C++ and uproot read these keys). ----
    // The file lives in the temp dir and is removed before we return — no litter.
    let out = std::env::temp_dir().join("oxiroot_ex_functions.root");
    FileWriter::create(&out)
        .add(&damped)
        .add(&gauss)
        .add(&decay)
        .add(&parab)
        .add(&f2)
        .add(&f3)
        .write()?;
    println!("\nwrote {}", out.display());

    // --- Round-trip: read one Func1D back and confirm it evaluates identically. ---
    // `file.get::<Func1D>` re-parses the stored TFormula, with its parameters and
    // their names, so the decoded function reproduces `eval` at sampled points.
    let g = FileReader::open(&out)?.get::<Func1D>("gauss")?;
    for &x in &[-2.0, -0.5, 0.5, 2.0] {
        assert!(
            (g.eval(x) - gauss.eval(x)).abs() < 1e-12,
            "round-trip mismatch at x = {x}",
        );
    }
    println!(
        "read back `gauss`: parameters {}, eval(0.5) = {:.6} (matches original {:.6})",
        g.param_names().join(", "),
        g.eval(0.5),
        gauss.eval(0.5),
    );

    let _ = std::fs::remove_file(&out);
    Ok(())
}
