//! Histogram fitting (`fit` feature). Run: `cargo test -p oxiroot-hist --features fit`.
#![cfg(feature = "fit")]

use std::path::PathBuf;

use oxiroot_hist::oxihist::{Histogram, Weight};
use oxiroot_hist::{FitExt, Hist1D, Model};
use oxiroot_io_core::FileReader;

fn read(name: &str) -> Histogram<Weight, 1> {
    let f = FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/analysis.root"),
    )
    .expect("open");
    f.get::<Hist1D>(name)
        .expect("read")
        .to_hist()
        .expect("convert")
}

fn rel_close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

#[test]
fn tf1_eval_uses_current_params() {
    let g = Model::gaussian("g").with_params(vec![10.0, 0.0, 1.0]);
    assert!((g.eval(0.0) - 10.0).abs() < 1e-12); // peak
    assert!((g.eval(1.0) - 10.0 * (-0.5_f64).exp()).abs() < 1e-12);
}

#[test]
fn ergonomics_estimate_seed_and_fit_into() {
    use oxiroot_hist::FitOptions;

    // Model is Send + Sync + Clone (cross threads, share the model, rebuild seeds).
    fn assert_traits<T: Send + Sync + Clone>() {}
    assert_traits::<Model>();

    // A gaussian set bin by bin: the file stores no moment sums, so the
    // histogram has no fill statistics and its mean is estimated from the bins.
    let hg = read("hg");
    assert!(hg.fill_stats().is_none());
    assert!(rel_close(hg.mean(), 0.5, 5e-3), "mean = {}", hg.mean());

    // estimate_from seeds (constant, mean, sigma) straight from the bins, so the
    // call site needs no manual content-weighted moment loop.
    let mut model = Model::gaussian("g").estimate_from(&hg);
    // A clone shares the closure and fits identically.
    assert_eq!(model.clone().eval(0.5), model.eval(0.5));
    assert!(model.params()[2] > 0.0, "sigma seed must be positive");
    assert!(format!("{model:?}").contains("Model")); // Debug prints (closure hidden)

    // fit_into writes the best-fit parameters back into the model.
    let r = hg.fit_into(&mut model, &FitOptions::new());
    assert!(r.valid);
    assert!(
        rel_close(model.params()[1], 0.5, 5e-3),
        "mean = {}",
        model.params()[1]
    );
    assert_eq!(
        model.params(),
        r.params,
        "model holds the fitted parameters"
    );
    // The model now evaluates the fitted curve: its peak == the fitted constant.
    assert!(rel_close(model.eval(model.params()[1]), r.params[0], 1e-9));
}

#[test]
fn fit_a_tgraph_with_errors() {
    use oxiroot_hist::Graph;

    // Points exactly on y = 2x + 3 with small symmetric y-errors. The same
    // `.fit(...)` API as a histogram works because `Graph` is `FitData` too.
    let x = vec![0.0, 1.0, 2.0, 3.0, 4.0];
    let y: Vec<f64> = x.iter().map(|&x| 2.0 * x + 3.0).collect();
    let ex = vec![0.0; x.len()];
    let ey = vec![0.1; x.len()];
    let g = Graph::new(x, y)
        .and_then(|g| g.with_errors(oxiroot_hist::GraphErrors::Symmetric { ex, ey }))
        .unwrap()
        .named("g")
        .titled("line");

    let fit = g.fit(&Model::polynomial("line", 1).with_params(vec![0.0, 0.0]));
    assert!(fit.valid);
    assert!(
        rel_close(fit.params[0], 3.0, 1e-6),
        "intercept = {}",
        fit.params[0]
    );
    assert!(
        rel_close(fit.params[1], 2.0, 1e-6),
        "slope = {}",
        fit.params[1]
    );
    assert!(fit.chi2 < 1e-6, "perfect line → chi2 ≈ 0, got {}", fit.chi2);
    // 5 points − 2 free params = 3 d.o.f.
    assert_eq!(fit.ndf, 3);
}
