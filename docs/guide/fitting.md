# Fitting

Parametric curve fitting for **any 1-D data** — not just histograms — backed by
a pure-Rust port of [Minuit2](https://crates.io/crates/minuit2), the minimizer
ROOT uses. This page covers the standalone `oxifit` crate, the `FitData` /
`FitExt` abstraction, building models, choosing a cost and minimizer, and
reading back results.

Everything here comes from the **`fit`** feature, which is **on by default** — a
plain `oxiroot` dependency already has it. (Turn it off, along with the other
extras, with `default-features = false`.)

```toml
# Cargo.toml
oxiroot = { git = "https://github.com/oxihep/oxiroot", tag = "v0.10.0" } # the `fit` feature is on by default
```

## The fitting abstraction

A fit needs only, per data point, an independent value `x`, a measured value
`y`, and a Gaussian uncertainty `sigma` on `y`. Anything that can produce those
implements the `FitData` trait, and a blanket `FitExt` impl gives every such
dataset the `.fit(...)` methods for free.

| Item | Role |
| --- | --- |
| `FitData` | Trait with one method, `fn points(&self) -> Vec<Point>` |
| `FitExt` | Blanket trait adding `fit` / `fit_opts` / `fit_into` to any `FitData` |
| `Point` | One `(x, y, sigma)` triple |
| `Points` | A standalone collection of `Point`s |
| `Model` | A named parametric fit function `f(x, params)` |
| `FitResult` | Best-fit parameters, errors, covariance, chi-square |

Histograms (oxihist's, 1-D to 3-D) and `Graph` implement `FitData`, so they fit
directly; a histogram read from a file is fitted after `to_hist()?`. `Points`, `&[Point]`,
`[Point; N]`, and `Vec<Point>` also implement it, and you can implement it on
your own type to fit anything.

!!! note
    `FitExt` is in scope via `oxiroot::prelude::*`. If you import individual
    items instead, bring it in with `use oxiroot::fit::FitExt;` (or
    `use oxifit::FitExt;`).

## A first fit

Fitting a Gaussian peak to a histogram. `estimate_from` seeds the
`(constant, mean, sigma)` parameters straight from the data, so no manual
moment loop is needed.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, Model};

// the `fit` feature is on by default
let mut peak = Hist::reg(60, 80.0, 100.0).weight().named("mass");
for _ in 0..10_000 {
    peak.fill(/* sample */ 91.2);
}

let model = Model::gaussian("z").estimate_from(&peak);
let fit = peak.fit(&model); // chi-square (ROOT's default)

println!(
    "mean = {:.3} ± {:.3}, sigma = {:.3} ± {:.3}, chi2/ndf = {:.2}",
    fit.params[1], fit.errors[1],
    fit.params[2], fit.errors[2],
    fit.chi2_per_ndf(),
);
```

The same `data.fit(&model)` call works on a `Graph` or on raw points:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, Model, Points};

// the `fit` feature is on by default
// Raw (x, y, sigma) measurements.
let data = Points::new(&[0.0, 1.0, 2.0, 3.0], &[1.0, 3.0, 5.0, 7.0], &[0.1; 4])?;
let line = Model::polynomial("line", 1).with_params(vec![0.0, 1.0]);
let fit = data.fit(&line);
assert!(fit.valid);
assert!((fit.params[0] - 1.0).abs() < 1e-6); // intercept ≈ 1
assert!((fit.params[1] - 2.0).abs() < 1e-6); // slope ≈ 2
```

## Building a model

`Model` is a named parametric fit function `f(x, params)` with named parameters
and optional per-parameter constraints. (The evaluable, persistable ROOT
function object is [`Func1D`](functions.md); `Func1D::to_model()` converts one to a
`Model` for fitting. It needs the `fit` feature of `oxiroot-hist-func`, which
the facade's `fit` feature turns on.)

### Built-in shapes

| Constructor | Function | Parameters |
| --- | --- | --- |
| `Model::gaussian(name)` | `[0]·exp(-½·((x-[1])/[2])²)` (ROOT `"gaus"`) | `constant`, `mean`, `sigma` |
| `Model::exponential(name)` | `exp([0] + [1]·x)` (ROOT `"expo"`) | `constant`, `slope` |
| `Model::polynomial(name, degree)` | `Σ p[k]·x^k` (ROOT `"polN"`) | `p0` … `p<degree>` |

Plus the HEP lineshapes (`constant ·` the
[lineshapes of `oxiroot::stat`](statistics.md#hep-lineshapes)) — a Gaussian peak
with tails, resonances, and a phase-space background:

| Constructor | Shape | Parameters |
| --- | --- | --- |
| `Model::crystal_ball(name)` | Crystal Ball (Gaussian core + power-law tail) | `constant`, `mean`, `sigma`, `alpha`, `n` |
| `Model::double_crystal_ball(name)` | tails on both sides | `constant`, `mean`, `sigma`, `alpha_lo`, `n_lo`, `alpha_hi`, `n_hi` |
| `Model::breit_wigner(name)` | Lorentzian resonance | `constant`, `mean`, `gamma` |
| `Model::voigtian(name)` | Gaussian ⊗ Lorentzian | `constant`, `mean`, `sigma`, `gamma` |
| `Model::novosibirsk(name)` | asymmetric peak | `constant`, `peak`, `sigma`, `tail` |
| `Model::argus(name)` | endpoint background | `constant`, `m0`, `c`, `power` |

`estimate_from` seeds the `constant`/`mean`/`sigma` core of the Gaussian, Crystal
Ball, double Crystal Ball, and Voigt models; seed the rest with `with_params`.

### Arbitrary formulas

`Model::from_formula(name, formula)` builds a model from any ROOT
[`TFormula`](functions.md) string — `"[0]*exp(-[1]*x)+[2]"`, `"gaus(0)+pol1(3)"`,
… — so you are not limited to the built-in shapes. The parameter count is
inferred from the formula; seed the values with `with_params`.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;
let model = Model::from_formula("decay", "[0]*exp(-[1]*x)")?
    .with_params(vec![100.0, 0.5])
    .lower_limit("p1", 0.0);
# Ok::<(), oxiroot::Error>(())
```

### Custom closures

For anything else, pass a closure `f(x, params)` to `Model::new`. The example
below is a Gaussian signal on a flat background:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;

// the `fit` feature is on by default
// params: [norm, mean, sigma, background-per-bin].
let sig_bkg = Model::new(
    "sig+bkg",
    &["norm", "mean", "sigma", "bkg"],
    vec![100.0, 91.0, 2.0, 10.0], // initial values
    |x, q| q[0] * (-0.5 * ((x - q[1]) / q[2]).powi(2)).exp() + q[3],
);
```

The closure must be `Fn(f64, &[f64]) -> f64 + Send + Sync + 'static`, so a model
can be shared across threads.

### Sums of models

Built-in shapes add up: `sig + bkg` (or `Model::sum("name", &[&sig, &bkg])`)
evaluates to the sum of its parts, with each parameter named after its part.
Each part keeps its seeds, limits and fixed parameters:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;

// the `fit` feature is on by default
let sig = Model::gaussian("sig")
    .with_params(vec![100.0, 91.0, 2.0])
    .lower_limit("sigma", 0.0);
let bkg = Model::polynomial("bkg", 0).with_params(vec![10.0]);
let sum = sig + bkg; // parameters: sig.constant, sig.mean, sig.sigma, bkg.p0
let sum = sum.limit("sig.mean", 85.0, 97.0);
```

A part name that repeats is numbered (`g`, `g_2`), and `a + b + c` adds a third
part rather than nesting the first sum.

!!! tip "Typos in parameter names"
    `limit`, `fix`, `constrain` and the other per-parameter setters given a
    name the model does not have would otherwise do nothing. A model that
    received such a name fits as **invalid**, and `model.check()` names it
    (`FitError::UnknownParameter`).

### Convolutions

`Model::convolve("m", &signal, &resolution, window)` smears a model by a
resolution function: `∫ signal(x − t)·resolution(t) dt` over `window`, the range
of `t` the resolution covers. Use a unit-area resolution such as
`Model::normal`, so the signal keeps its area:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;

// the `fit` feature is on by default
// A decay that starts at t = 0, smeared by a Gaussian vertex resolution.
let decay = Model::new("decay", &["n", "tau"], vec![500.0, 1.5], |t, p| {
    if t < 0.0 { 0.0 } else { p[0] / p[1] * (-t / p[1]).exp() }
});
let resolution = Model::normal("res").with_params(vec![0.0, 0.1]).fix("mean");
let measured = Model::convolve("measured", &decay, &resolution, (-1.0, 1.0));
// parameters: decay.n, decay.tau, res.mean, res.sigma
```

### Seeding and constraints

These chainable methods return the `Model`, so they compose:

| Method | Effect |
| --- | --- |
| `with_params(values)` | Replace the initial parameter values |
| `estimate_from(&data)` | Data-driven seed (Gaussian shape only; no-op otherwise) |
| `limit(name, lo, hi)` | Constrain a parameter to `[lo, hi]` |
| `lower_limit(name, lo)` | Constrain a parameter to `>= lo` (e.g. a positive width) |
| `upper_limit(name, hi)` | Constrain a parameter to `<= hi` |
| `fix(name)` | Hold a parameter fixed at its current value |
| `step(name, step)` | Set the initial Minuit2 step (default: 10 % of the value) |
| `constrain(name, mean, sigma)` | Add a Gaussian constraint `((p − mean)/sigma)²` to the cost — an external measurement or a nuisance prior; counts as one more measurement in `ndf` |

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;

// the `fit` feature is on by default
let model = Model::gaussian("g")
    .estimate_from(&data)        // seed (constant, mean, sigma) from the data
    .lower_limit("sigma", 0.0);  // keep the width non-negative
```

!!! tip
    `estimate_from` seeds the Gaussian-core shapes (the Gaussian, Crystal Ball,
    double Crystal Ball, and Voigt peaks): it sets the leading `(constant, mean,
    sigma)` to the peak height and the `y`-weighted mean and standard deviation of
    the data, leaving any tail parameters at their defaults. For other shapes,
    supply initial values with `with_params` (or in `Model::new`). It is
    especially useful for histograms set bin by bin (`bin_mut`) rather than
    filled, which have no fill statistics.

`Model::eval(x)` evaluates the function at `x` with its current parameters —
handy after `fit_into` (below) to draw the fitted curve.

## Costs and the fit methods

`FitExt` gives a dataset three entry points; they layer on top of one another:

| Method | Signature | Behaviour |
| --- | --- | --- |
| `fit` | `fit(&model) -> FitResult` | Chi-square over the full range (ROOT's default) |
| `fit_opts` | `fit_opts(&model, &opts) -> FitResult` | Full control via `FitOptions` (cost, range, robust loss, MINOS) |
| `fit_into` | `fit_into(&mut model, &opts) -> FitResult` | Like `fit_opts`, but writes the best-fit parameters back into the model |

The cost being minimized is a `FitMethod`:

| `FitMethod` | Cost | Notes |
| --- | --- | --- |
| `Chi2` (default) | Neyman χ², `Σ (y − f)² / σ²` | Uses the *observed* per-point error; empty (`σ ≤ 0`) points are dropped |
| `PearsonChi2` | Pearson χ², `Σ (y − f)² / f` | Uses the *expected* (model) variance; less biased at low counts |
| `Likelihood` | Binned Poisson, `2·Σ [f − y + y·ln(y/f)]` | Maximum-likelihood for binned counts (histograms) |
| `UnbinnedLikelihood` | `−2·Σ w·ln(f/∫f)` | Individual events (`Events`); the model's shape, normalized over the range (see [Unbinned fits](#unbinned-fits)) |
| `ExtendedUnbinnedLikelihood` | `−2·[Σ w·ln f − ∫f]` | Individual events; the model is the expected event density, so the yield is fitted too |

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitMethod, FitOptions};

// the `fit` feature is on by default
let chi2 = peak.fit(&model);                             // χ², the default
let opts = FitOptions::new().method(FitMethod::Likelihood);
let like = peak.fit_opts(&model, &opts);                 // Poisson likelihood
```

!!! warning
    `Likelihood` treats each `y` as a count and assumes a non-negative model; it
    is meaningful for binned histograms, not arbitrary `y`. A model that dips
    below zero is clamped to a tiny positive value (heavily penalised, not
    rejected).

### Options

`FitOptions` carries the cost, an optional fit range, the MINOS toggle, and the
minimizer backend. Construct it with `FitOptions::new()` and chain setters:

| Setter | Effect |
| --- | --- |
| `method(FitMethod)` | The cost to minimize (default `Chi2`) |
| `range(lo, hi)` | Fit only points whose `x` lies in `[lo, hi]` |
| `with_minos(bool)` | Also compute asymmetric MINOS errors |
| `with_hesse(bool)` | Run HESSE after MIGRAD (default `true`, as ROOT does) |
| `minimizer(Minimizer)` | Choose the optimizer backend (see [below](#minimizer-backends)) |
| `strategy(u32)` | Minuit2 strategy `0`/`1`/`2` (default `1`) |
| `tolerance(f64)` | Minuit2 convergence tolerance (default `0.1`) |
| `max_fcn(usize)` | Cap on cost evaluations per Minuit2 minimization |
| `loss(Loss)` | Robust loss to down-weight outliers (see [below](#robust-fitting-outliers)) |
| `f_scale(f64)` | Residual scale for the robust loss (default `1.0`) |
| `range_y(lo, hi)`, `range_z(lo, hi)` | For 2-D/3-D data, fit only points inside these `y`/`z` ranges |
| `with_gradient(bool)` | Use the model's analytic gradient when it has one (default `true`) |
| `with_sumw2(bool)` | Correct a weighted unbinned fit's covariance for the weights (RooFit's `SumW2Error`) |

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitMethod, FitOptions};

// the `fit` feature is on by default
let opts = FitOptions::new()
    .method(FitMethod::Chi2)
    .range(85.0, 97.0)
    .minos(true);
let fit = peak.fit_opts(&model, &opts);
```

`fit_into` writes the fitted parameters back into the model so it evaluates the
fitted curve afterwards:

<!-- fragment -->
```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions};

// the `fit` feature is on by default
let mut model = sig_bkg;
let fit = withbkg.fit_into(&mut model, &FitOptions::new());
// model.eval(x) now draws the *fitted* curve:
let height_at_peak = model.eval(fit.params[1]);
```

## Robust fitting (outliers)

A least-squares fit (`Chi2`/`PearsonChi2`) can be dragged off by a few outliers,
because each point's cost grows with the *square* of its residual. A **robust
loss** `ρ` — the `loss` of
[`scipy.optimize.least_squares`](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.least_squares.html)
— grows sub-quadratically instead, so a far-out point pulls on the fit far less:

| `Loss` | `ρ(z)` | Character |
| --- | --- | --- |
| `Linear` (default) | `z` | Ordinary least squares |
| `SoftL1` | `2(√(1+z) − 1)` | Gentle, everywhere smooth |
| `Huber` | `z` if `z ≤ 1` else `2√z − 1` | Quadratic core, linear tails |
| `Cauchy` | `ln(1 + z)` | Strong suppression |
| `Arctan` | `arctan(z)` | Most aggressive (bounded influence) |

`z` is the squared residual; `f_scale` sets the residual at which a point starts
to count as an outlier (`ρ` is applied as `C²·ρ(z / C²)` with `C = f_scale`).

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, Loss};

let opts = FitOptions::new().loss(Loss::Huber).f_scale(1.5);
let fit = peak.fit_opts(&model, &opts);
```

!!! warning "Seed robust fits well"
    A robust loss has a **small basin of attraction** — points already far from
    the model barely pull, so a gradient minimizer can stall far from the answer.
    Fit with `Linear` first and use those parameters to seed the robust fit
    (`SoftL1`/`Huber` are forgiving; `Cauchy`/`Arctan` need a close start). A
    robust loss also weakens the χ² / `p_value` interpretation of the result.

## Reading the result

`FitResult` holds the outcome. Fields are in the model's parameter order.

| Field / method | Meaning |
| --- | --- |
| `params: Vec<f64>` | Best-fit parameter values |
| `errors: Vec<f64>` | Parabolic uncertainties (from HESSE) |
| `minos: Option<Vec<(f64, f64)>>` | Asymmetric `(lower, upper)` MINOS errors, when requested |
| `covariance: Option<Vec<Vec<f64>>>` | Covariance of the *free* parameters (row-major) |
| `chi2: f64` | Goodness-of-fit statistic at the minimum (`NaN` under a robust loss) |
| `fval: f64` | The minimized cost itself |
| `edm: f64` | Minuit2's estimated distance to the minimum |
| `nfcn: usize` | Cost evaluations the fit took |
| `ndf: usize` | Degrees of freedom: fitted points − free parameters |
| `valid: bool` | Whether the minimizer reported a valid minimum |
| `chi2_per_ndf()` | Reduced chi-square `chi2 / ndf` (`NaN` if `ndf == 0`) |
| `p_value()` | Goodness-of-fit p-value (near 1 for a good fit, near 0 for a poor one) |

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions};

// the `fit` feature is on by default
let fit = peak.fit_opts(&model, &FitOptions::new().minos(true));
if fit.valid {
    println!("mean   = {:.3} ± {:.3}", fit.params[1], fit.errors[1]);
    if let Some(minos) = &fit.minos {
        let (lo, hi) = minos[1];
        println!("  MINOS: {lo:+.3} / {hi:+.3}");
    }
    println!("chi2/ndf = {:.2}, p = {:.3}", fit.chi2_per_ndf(), fit.p_value());
}
```

!!! note
    The χ²-survival function behind `p_value` lives in the dependency-free
    `oxistats` leaf crate, shared with the histogram comparison tests.
    A `Chi2` fit drops empty points and a fixed parameter does not count toward
    `ndf`, so reduced-χ² and p-values reflect only what actually entered the fit.

!!! tip "Show the result on a plot"
    With the `plot` feature, `ax.fit_stats(&fitted, &fit)` draws a ROOT-style
    statistics box — the function name, χ²/ndf, and every parameter with its
    error — next to the fitted curve. See
    [Plotting → Fit statistics box](plotting.md#fit-statistics-box).

## Unbinned fits

A histogram throws away where each event fell inside its bin. With few events,
or a narrow feature, fit the events themselves: `Events::new(&values)` holds
them (`Events::weighted(&values, &weights)` with weights), and the two unbinned
likelihoods fit a model to them. Their normalization is numerical, over the fit
`range` (default: the events' span), so any model works.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{Events, FitExt, FitMethod, FitOptions, Model};

// the `fit` feature is on by default
// Decay times in ps (in practice, a branch of a TTree).
let times = [0.42, 1.73, 0.95, 2.61, 0.31, 1.12, 3.40, 0.77, 1.58, 0.66];
let events = Events::new(&times);
// UnbinnedLikelihood fits the *shape*: its overall scale is free, so fix it.
let decay = Model::exponential("decay").with_params(vec![0.0, -0.7]).fix("constant");
let opts = FitOptions::new().method(FitMethod::UnbinnedLikelihood).range(0.0, 10.0);
let fit = events.fit_opts(&decay, &opts);
println!("τ = {:.2} ps", -1.0 / fit.params[1]);

// ExtendedUnbinnedLikelihood: the model is events per unit x, so its
// integral — the yield — is fitted as well.
let density = Model::new("yield·decay", &["n", "tau"], vec![8.0, 1.5], |t, p| {
    p[0] / p[1] * (-t / p[1]).exp()
});
let ext = events.fit_opts(&density, &opts.method(FitMethod::ExtendedUnbinnedLikelihood));
println!("{:.1} ± {:.1} events", ext.params[0], ext.errors[0]);
```

The unbinned likelihoods have no goodness of fit: `chi2` and `p_value()` are
`NaN`. A weighted fit's errors are those of `Σw` unit-weight events unless
`with_sumw2(true)` corrects them for the weights.

## 2-D and 3-D fits

Models of two or three variables fit 2-D and 3-D histograms (each bin a point
at its center) and 2-D/3-D events. `Model::new2(|x, y, p| …)` and
`Model::new3(…)` take a closure; `Model::product` multiplies 1-D models, one per
variable, and its integral factorizes, so unbinned fits normalize it with 1-D
integrals:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{Events2, FitExt, FitMethod, FitOptions, Model};

// the `fit` feature is on by default
let mut map = Hist::reg(40, -4.0, 4.0).reg(40, -4.0, 4.0).double().named("map");
// … fill map with (x, y) pairs …
let blob = Model::product(
    "blob",
    &[&Model::gaussian("x").with_params(vec![50.0, 0.0, 1.0]), &Model::normal("y")],
);
let fit = map.fit_opts(&blob, &FitOptions::new().method(FitMethod::Likelihood));

// A ROOT TF2 converts too:
let tf2 = Func2D::new("[0]+[1]*x+[2]*y", -4.0..=4.0, -4.0..=4.0)?.named("plane");
let plane_fit = map.fit(&tf2.to_model());

// Unbinned: events of two variables, normalized over x × y ranges.
let events = Events2::new(&[0.1, -0.4, 0.8], &[0.3, 0.0, -0.6])?;
let opts = FitOptions::new()
    .method(FitMethod::UnbinnedLikelihood)
    .range(-4.0, 4.0)
    .range_y(-4.0, 4.0);
let shape = Model::product("xy", &[&Model::normal("x"), &Model::normal("y")]);
let unbinned = events.fit_opts(&shape, &opts);
```

`Points2`/`Points3` hold other 2-D/3-D data, `Events3` 3-D events. A model of
another dimension than the data fits as invalid (`FitError::DimensionMismatch`).

## Analytic gradients

`model.with_gradient(|x, p, g| …)` gives a model its derivatives `∂f/∂pₖ`; the
Gaussian, exponential, polynomial and normal shapes have theirs, as do sums,
products and convolutions of models that do, and `check_gradient` compares one
with finite differences. Every cost then has an exact gradient: MIGRAD,
Levenberg–Marquardt and `Minimizer::LBfgsB` use it, and the error matrix comes
from it in `2n` gradient calls instead of HESSE's `~2n²` cost calls — the gain
grows with the number of parameters.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::Model;

// the `fit` feature is on by default
let line = Model::new("line", &["a", "b"], vec![0.0, 1.0], |x, p| p[0] + p[1] * x)
    .with_gradient(|x, _p, g| {
        g[0] = 1.0;
        g[1] = x;
    });
assert!(line.check_gradient(&[0.0, 1.0, 2.0]) < 1e-8);
```

With the `rayon` feature, large costs (more than 4096 points) are summed on all
cores, and toys, profile points and contour rays fit in parallel — with results
that do not depend on the number of threads.

## Simultaneous fits

Several datasets fitted together share the parameters with the same name — a
mass seen in two decay modes, a signal strength across categories. Each channel
keeps its own model and cost (binned or unbinned):

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitMethod, FitOptions, Model, Simultaneous};

// the `fit` feature is on by default
// Two peaks of the same particle, in two samples: the mean is shared, the
// heights and widths are each sample's own.
let peak_in = |tag: &str| {
    let names = [format!("{tag}.height"), "mean".to_string(), format!("{tag}.width")];
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    Model::new(tag, &refs, vec![50.0, 91.0, 2.5], |x, p| {
        p[0] * (-0.5 * ((x - p[1]) / p[2]).powi(2)).exp()
    })
};
let opts = FitOptions::new().method(FitMethod::Likelihood);
let sim = Simultaneous::new()
    .channel("electrons", &peak, &peak_in("ee"), &opts)
    .channel("muons", &peak, &peak_in("mumu"), &opts);
let fit = sim.fit(&FitOptions::new());
let names = sim.param_names(); // ee.height, mean, ee.width, mumu.height, mumu.width
for (name, (v, e)) in names.iter().zip(fit.params.iter().zip(&fit.errors)) {
    println!("{name} = {v:.3} ± {e:.3}");
}
for (channel, cost, n) in sim.channel_costs(&fit) {
    println!("{channel}: {cost:.1} over {n} bins");
}
```

## Toy studies

Is the fit unbiased, and are its errors right? Generate pseudo-data from the
model at known parameters, fit each sample, and look at the pulls
`(fitted − true)/error` — standard normal when all is well (RooFit's
`RooMCStudy`):

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitMethod, FitOptions, Model, Toys};

// the `fit` feature is on by default
let truth = Model::gaussian("z").with_params(vec![40.0, 91.0, 2.5]);
let centres: Vec<f64> = (0..40).map(|i| 81.0 + 0.5 * f64::from(i)).collect();
let opts = FitOptions::new().method(FitMethod::Likelihood);
let study = Toys::binned(&truth, &centres).seed(1).run(&truth, &opts, 200);
for name in ["constant", "mean", "sigma"] {
    if let Some(p) = study.pull_summary(name) {
        println!("{name}: pull mean {:+.2} ± {:.2}, width {:.2} ± {:.2}",
                 p.mean, p.mean_err, p.width, p.width_err);
    }
}
```

`Toys::unbinned(&truth, range, n)` draws events instead (`.extended(true)` for a
Poisson number of them).

## Profiles and contours

The parabolic errors (and MINOS) describe one parameter at a time. Two scans
go further, both refitting the other parameters at every point:

- `profile` steps one parameter across a range and returns `Δcost` against it;
  `interval(1.0)` reads off the 1σ interval where the curve crosses `Δ = 1`
  (`4.0` for 2σ).
- `contour` traces the joint region of two parameters where the cost is `Δ`
  above its minimum: `2.30` for 68 %, `5.99` for 95 %.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, Model};

// the `fit` feature is on by default
let opts = FitOptions::new();
let model = Model::gaussian("z").estimate_from(&peak).lower_limit("sigma", 0.0);
let fit = peak.fit_opts(&model, &opts);
let (m, e) = (fit.params[1], fit.errors[1]);
let profile = peak.profile(&model, &opts, "mean", (m - 3.0 * e, m + 3.0 * e), 41)?;
if let Some((lo, hi)) = profile.interval(1.0) {
    println!("mean ∈ [{lo:.3}, {hi:.3}] (1σ)");
}
let region = peak.contour(&model, &opts, "mean", "sigma", 2.30, 60)?;
println!("{} points around ({:.3}, {:.3})", region.points.len(), region.best.0, region.best.1);
```

Both return a `FitError` for an unknown or fixed parameter, or a fit that
failed. With the `plot` feature, `ax.fit_contour(&region)` and
`ax.fit_profile(&profile)` draw them (see
[Plotting → Fit contours and profiles](plotting.md#fit-contours-and-profiles)).

## Your own cost

Any `−2 ln L` or χ² can be minimized with every backend below, HESSE, MINOS,
profiles and contours, as iminuit's `Minuit(fcn)` does. A cost is anything
that implements `Cost` (parameter names and a value, optionally a gradient):

- `FnCost::new(&names, closure)` wraps a closure;
- `ModelCost::new(&data, &model, &opts)` turns a data fit into a cost
  (`ModelCost::new(&d, &m, &o).fitter().fit(&o)` is exactly `d.fit_opts(&m, &o)`);
- `CostSum::new().with(a).with(b)` adds costs, merging parameters by name.

`Fitter` holds the seeds and the per-parameter settings (`set`, `limit`, `fix`,
`release`, `step`, `constrain`) and runs `fit`, `profile` and `contour`.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{CostSum, FitExt, FitOptions, Fitter, FnCost, Model, ModelCost};

// The peak's width also measured elsewhere: 0.62 ± 0.03.
let model = Model::gaussian("z").estimate_from(&peak);
let data_fit = ModelCost::new(&peak, &model, &FitOptions::new());
let external = FnCost::new(&["sigma"], |p| ((p[0] - 0.62) / 0.03).powi(2));
let total = CostSum::new().with(data_fit).with(external);
let fit = Fitter::new(&total, model.params().to_vec())
    .limit("sigma", 0.1, 2.0)
    .fit(&FitOptions::new().minos(true));
```

## Limits and significance

`HypoTest::new(fitter, "mu")` tests a parameter of interest, typically a
signal strength, on any cost with the profile likelihood ratio (the pyhf and
RooStats recipe):

- `discovery(1.0)`: the observed `q₀`, its p-value and significance, and the
  median significance expected for a signal of strength 1;
- `cls(mu)`: CLs+b, CLb and CLs at `mu`, with the expected −2σ…+2σ band;
- `upper_limit(0.95, (lo, hi))`: the observed 95 % CL upper limit and its
  expected band (the "Brazil band");
- `cls_toys` and `discovery_toys`: the same from pseudo-experiments, for small
  counts (in parallel with the `rayon` feature).

The asymptotic formulas (Cowan, Cranmer, Gross, Vitells) use the cost's Asimov
dataset, which `ModelCost` builds for binned and unbinned data. The `Fitter`'s
Gaussian constraints act as auxiliary measurements. CLs, the limits and the
discovery p-value match pyhf. For a mass scan, `upcrossings` and
`global_significance` give the look-elsewhere correction (Gross–Vitells).

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitMethod, FitOptions, HypoTest, Model, ModelCost};

// Signal strength μ times a known peak, on a flat background.
let s_plus_b = Model::new("sb", &["mu", "bkg"], vec![1.0, 20.0], |x, p| {
    p[0] * 50.0 * (-0.5 * ((x - 91.0) / 2.5_f64).powi(2)).exp() + p[1]
});
let cost = ModelCost::new(&peak, &s_plus_b, &FitOptions::new().method(FitMethod::Likelihood));
let test = HypoTest::new(cost.fitter().limit("mu", 0.0, 10.0), "mu")?;
let limit = test.upper_limit(0.95, (0.0, 5.0))?;
println!("μ < {:.2} (expected {:.2})", limit.observed, limit.expected[2]);
let found = test.discovery(1.0)?;
println!("Z = {:.1}σ (expected {:.1}σ)", found.z, found.z_expected);
```

## Template fits

`TemplateModel` is the HistFactory model of ROOT and pyhf: channels of observed
counts, each with samples (simulated templates) scaled and deformed by
modifiers. `norm_factor` is a free normalization; `norm_sys` and `histo_sys`
are normalization and shape uncertainties, interpolated as pyhf does;
`stat_error` covers the templates' finite statistics (Barlow–Beeston lite);
`shape_sys`, `lumi` and `shape_factor` complete the set. A `TemplateModel` is a
`Cost`, so `model.fitter()` fits it and `HypoTest` sets limits on it. `pulls`,
`impacts` (the ranking on the parameter of interest) and `yields` (pre- and
post-fit, with errors) summarize the fit. With the `pyhf` feature of `oxifit`,
`oxiroot::fit::Workspace::from_json` reads a pyhf JSON workspace. Best fits,
CLs and limits match pyhf on two reference workspaces.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, HypoTest, TemplateModel};
use oxiroot::fit::{Channel, Sample};

// Bin contents of the signal-region histogram and of the simulated templates.
let (observed, sig, bkg, bkg_err) = (
    vec![52.0, 61.0, 49.0],
    vec![2.0, 9.0, 3.0],
    vec![50.0, 52.0, 47.0],
    vec![3.0, 3.0, 3.0],
);
let model = TemplateModel::new(vec![Channel::new("SR", &observed)
    .sample(Sample::new("signal", &sig).norm_factor("mu"))
    .sample(
        Sample::new("background", &bkg)
            .norm_sys("xsec", 0.9, 1.1)
            .stat_error("mc", &bkg_err),
    )])?;
let fitter = model.fitter();
let fit = fitter.fit(&FitOptions::new());
for (name, pull, error) in model.pulls(&fit) {
    println!("{name}: {pull:+.2} ± {error:.2}");
}
let limit = HypoTest::new(model.fitter(), "mu")?.upper_limit(0.95, (0.0, 10.0))?;
```

## sWeights

`SWeights::compute(&events, &[&signal, &background], &yields, &opts)` turns an
extended fit in a discriminating variable (a mass) into per-event weights for
each species (sPlot, Pivk and Le Diberder). A weighted unbinned fit of another
variable with the signal weights then measures the signal alone. Use
`FitOptions::asymptotic_errors(true)` for that fit: it gives the
asymptotically correct covariance (Langenbruch) for weights of any sign and
size, where the naive and SumW2 errors are too small.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{Events, FitExt, FitMethod, FitOptions, Model, SWeights};

// Mass and decay time of the same events (in practice, two TTree branches).
let mass = [5.27, 5.29, 5.31, 5.12, 5.28, 5.44, 5.26, 5.20, 5.30, 5.37];
let time = [1.2, 0.4, 2.3, 0.1, 0.9, 0.2, 3.1, 0.3, 1.7, 0.1];
let signal = Model::gaussian("sig").with_params(vec![1.0, 5.28, 0.02]);
let background = Model::exponential("bkg").with_params(vec![1.0, -2.0]);
// The yields from an extended fit of the mass (fit separately).
let yields = [5.8, 4.2];
let opts = FitOptions::new().range(5.1, 5.5);
let sw = SWeights::compute(&Events::new(&mass), &[&signal, &background], &yields, &opts)?;
let weighted = Events::weighted(&time, sw.weights(0))?;
let decay = Model::exponential("decay").with_params(vec![0.0, -0.7]).fix("constant");
let fit = weighted.fit_opts(
    &decay,
    &FitOptions::new()
        .method(FitMethod::UnbinnedLikelihood)
        .range(0.0, 10.0)
        .asymptotic_errors(true),
);
```

## Bin-integrated fits

By default a binned fit compares each bin with the model at the bin centre.
`FitOptions::bin_integration(true)` compares it with the model averaged
over the bin instead (ROOT's option `"I"`, iminuit's `BinnedNLL`). That matters
when the model curves within a bin: a narrow peak in wide bins, or a steep
slope. A `Hist1D` passes its real bin edges, variable-width ones included.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, Model};

let model = Model::gaussian("z").estimate_from(&peak);
let fit = peak.fit_opts(&model, &FitOptions::new().bin_integration(true));
```

## Efficiency fits

A χ² fit to `passed/total` with binomial errors fails where the efficiency is
near 0 or 1 or a bin has few trials. `oxiroot::fit::BinomialData` fits an
efficiency curve with the **binomial likelihood** instead (ROOT's
`TEfficiency::Fit`, RooFit's `RooEfficiency`), from pass/total counts per bin
or from single pass/fail events. It matches ROOT. (`oxiroot::fit::BinomialData`
is the fit; the prelude's `Efficiency` is ROOT's `TEfficiency` object.)

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, Model};
use oxiroot::fit::BinomialData;

// Per-bin trigger counts: `passed` of `trials` at each pT (GeV).
let pt = [12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 30.0];
let passed = [1.0, 4.0, 12.0, 20.0, 29.0, 37.0, 40.0];
let trials = [40.0; 7];
let data = BinomialData::new(&pt, &passed, &trials)?;
// Or one event at a time: BinomialData::events(&pt, &fired)?
let turn_on = Model::new("eff", &["threshold", "width"], vec![18.0, 2.0], |x, p| {
    0.5 * (1.0 + oxiroot::stat::erf((x - p[0]) / (2f64.sqrt() * p[1])))
})
.lower_limit("width", 0.1);
let fit = data.fit(&turn_on, &FitOptions::new());
```

## Bayesian sampling

`Sampler::new(fitter).run(burn_in, steps)` samples a `Fitter`'s posterior: its
cost as `−2 ln L`, its limits as flat priors, its Gaussian constraints as
Gaussian priors. It uses the affine-invariant ensemble sampler of emcee
(Goodman–Weare stretch moves), which needs no tuning and copes with
correlations; it is reproducible from `seed` and runs walkers in parallel with
`rayon`. The `Chain` gives means, quantiles, central credible intervals, the
covariance, the acceptance and each parameter's autocorrelation time.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitMethod, FitOptions, Model, ModelCost, Sampler};

let model = Model::gaussian("z").estimate_from(&peak).limit("sigma", 0.1, 5.0);
let cost = ModelCost::new(&peak, &model, &FitOptions::new().method(FitMethod::Likelihood));
let chain = Sampler::new(cost.fitter()).seed(1).run(500, 3000)?;
let (lo, hi) = chain.interval("mean", 0.6827).expect("a parameter");
println!(
    "mean: median {:.3}, 68 % in [{lo:.3}, {hi:.3}], τ = {:.0} steps",
    chain.quantile("mean", 0.5).expect("a parameter"),
    chain.autocorrelation_time("mean").expect("a parameter"),
);
```

## Minimizer backends

`FitOptions::minimizer(...)` selects the optimizer. Every Minuit-based backend
ends with MIGRAD and HESSE, so the values, errors, covariance and `valid` mean
the same thing whichever one found the minimum.

| `Minimizer` | What it does | When |
| --- | --- | --- |
| `Minuit2Combined` (default) | MIGRAD, falling back to SIMPLEX when it fails (ROOT's `"Minimize"`) | Almost always |
| `Minuit2` | MIGRAD alone, from the model's parameters | A seed near the answer, and no fallback wanted |
| `Simplex` | Minuit2's SIMPLEX | A rough, noisy cost |
| `MultiStart(MultiStart::new(n))` | `n` local fits from points spread over the parameter limits; the lowest wins | A few competing minima |
| `DifferentialEvolution(…)` | A population-based global search over the limits, then MIGRAD | The seed may sit in the wrong basin |
| `CmaEs(CmaEs::new())` | CMA-ES, an evolution strategy that learns the valley's shape, then MIGRAD | As above, usually in a third of the evaluations |
| `LevenbergMarquardt` | Damped Gauss–Newton on the residuals, then MIGRAD | A good seed: fewer evaluations than MIGRAD |
| `LBfgsB` | Limited-memory quasi-Newton within the limits, then MIGRAD | Many parameters, with an [analytic gradient](#analytic-gradients) |
| `NelderMead` (`argmin` feature) | An independent simplex, errors from a numerical Hessian, no MINOS | Cross-checking Minuit2 |

A local minimizer only finds the minimum nearest its seed. When the seed may
be in the wrong basin, a peak whose position is not known for instance, a
global search looks over each free parameter's limits first. Set a `limit` on
the parameters to search:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{DifferentialEvolution, FitExt, FitOptions, Minimizer, Model, MultiStart};

// Search the whole mass window for the peak, wherever the seed puts it.
let search = Model::gaussian("z")
    .with_params(vec![10.0, 82.0, 1.0])
    .limit("constant", 0.0, 1e4)
    .limit("mean", 80.0, 100.0)
    .limit("sigma", 0.2, 10.0);
let de = Minimizer::DifferentialEvolution(DifferentialEvolution::new());
let fit = peak.fit_opts(&search, &FitOptions::new().minimizer(de));
println!("mean = {:.3} ± {:.3} after {} evaluations", fit.params[1], fit.errors[1], fit.nfcn);

// Or a handful of local fits from spread-out starts:
let ms = Minimizer::MultiStart(MultiStart::new(20).seed(1));
let fit = peak.fit_opts(&search, &FitOptions::new().minimizer(ms));
```

Both searches are reproducible from their `seed` and cost thousands of
evaluations instead of a few hundred.

The gradient-free `NelderMead` simplex (from the
[`argmin`](https://crates.io/crates/argmin) crate) optimizes the free
parameters only, holds limits exactly through Minuit's internal coordinates,
and derives errors from a
central-difference Hessian. The `Minimizer::NelderMead` variant only exists
when the `argmin` feature is on; `with_minos` is ignored by it.

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, FitOptions, Minimizer, Model};

// the `fit` + `argmin` features are both on by default
let fit = peak.fit_opts(
    &Model::gaussian("z").estimate_from(&peak),
    &FitOptions::new().minimizer(Minimizer::NelderMead),
);
```

## See also

- [Histograms](histograms.md)
- [Graphs](graphs.md)
- [Compression](compression.md)
- [Quickstart](../getting-started/quickstart.md)
- [API reference](../api/oxiroot/index.html)
