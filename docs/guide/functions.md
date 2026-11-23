# Functions (`Func1D`/`Func2D`/`Func3D`)

A **function** is a formula, its parameter values, and a range. oxiroot's `Func1D`
(1-D), `Func2D` (2-D), and `Func3D` (3-D) are ROOT's parametric functions: they
evaluate in pure Rust — `eval`, `integral`, `derivative` — and read and write as
ordinary ROOT `TF1`/`TF2`/`TF3` keys (each embeds a `TFormula`), so ROOT C++ and
uproot read what oxiroot writes and vice versa.

The functions live in the `oxiroot-hist-func` crate and are re-exported as
`oxiroot::hist::{Func1D, Func2D, Func3D}` and in the prelude. The expression engine
behind them is the dependency-free
[`oxifit-formula`](../reference/crates.md) crate; the same engine powers
[`Model::from_formula`](fitting.md) so any formula is also fittable.

## Building and evaluating

Construct a `Func1D` from a name, a formula, and an `[xmin, xmax]` range, then set
the parameters:

```rust
use oxiroot::prelude::*;

let f = Func1D::new("[0]*sin([1]*x) + [2]", 0.0..=6.283)?.named("f")
    .with_params(&[2.0, 1.5, 0.5])?;

f.eval(1.0);                       // 2*sin(1.5) + 0.5  = 2.494990
f.integral(0.0, 3.14159);         // ∫ over [0, π]     = 2.904130
f.derivative(1.0);                // 3*cos(1.5)        = 0.212212
# Ok::<(), oxiroot::Error>(())
```

`integral` is an adaptive Gauss–Kronrod quadrature and `derivative` a
Richardson-extrapolated central difference, matching `Func1D::Integral` /
`Func1D::Derivative` to ~10 significant figures.

`Func2D`/`Func3D` add coordinates — the formula gains `y` (and `z`):

```rust
use oxiroot::prelude::*;
let f2 = Func2D::new("[0]*sin(x) + [1]*y*y", -3.0..=3.0, -2.0..=2.0)?.named("f2")
    .with_params(&[1.5, 0.7])?;
f2.eval(1.0, 1.0);                            // 1.5*sin(1) + 0.7

let f3 = Func3D::new("[0]*x + y*z", 0.0..=2.0, 0.0..=2.0, 0.0..=2.0)?.named("f3")
    .with_params(&[2.0])?;
f3.eval(1.0, 1.0, 1.0);                       // 3.0
# Ok::<(), oxiroot::Error>(())
```

## Formula syntax

Formulas are ROOT's `TFormula` syntax, and the engine follows ROOT closely:
the same parameters in the same order, the same shortcuts and functions, and
the same value — it is checked against ROOT 6.40 on more than a hundred
formulas.

- **Parameters** are `[0]`, `[1]`, … or names, `[mean]`. ROOT numbers them in
  its own order: `p<N>` first, by number, then the names in byte order, and
  `[N]` also defines every `[0]` … `[N-1]`. So `[b]*x+[a]` has `a` first.
- **Variables** are `x`, `y`, `z` (and `t`), or ROOT's `x[0]`, `x[1]`, ….
- **Operators** `+ - * /`, `^` and `**` (right-associative, and tighter than a
  minus: `-x^2` is `-(x^2)`), comparisons, `&&`, `||`, `!` and `cond ? a : b`,
  with C++ precedence; a comparison is `1` or `0`.
- **Functions**: the C library's and `TMath`'s, with or without a namespace
  (`sqrt`, `TMath::Sqrt`, `std::sqrt`), and `TMath::Gaus`, `TMath::Landau`,
  `TMath::BreitWigner`, `ROOT::Math::normal_pdf`, `breitwigner_pdf`,
  `crystalball_function`, `bigaussian_pdf` and `Chebyshev0`…`Chebyshev10`.
- **Constants**: `pi`, `e`, `sqrt2`, `ln10`, `c`, `h`, `k`, … and the
  `TMath::Pi()` forms, with ROOT's values.
- **ROOT shortcuts**, expanded as ROOT expands them: `gaus`, `gausn`, `expo`,
  `landau`, `landaun`, `crystalball`, `breitwigner`, `bigaus`, `cheb0`…`cheb10`
  and `pol0`…`polN`. A shortcut takes the index of its first parameter
  (`gaus(0)+pol1(3)`: the Gaussian is `[0..2]`, the line `[3..4]`), another
  variable (`ygaus`, `gaus[y]`, `ypol2`, `pol2(y, 0)`), or two (`xygaus`,
  `xyexpo`, `bigaus`); `pol2(x, [a], [b], [c])` names its parameters.

A formula that is one shortcut alone gets ROOT's names for its parameters:

```rust
use oxiroot::prelude::*;
let g = Func1D::new("gaus", -5.0..=5.0)?
    .named("g")
    .with_param("Constant", 2.0)?
    .with_param("Sigma", 1.0)?;
assert_eq!(g.param_names(), ["Constant", "Mean", "Sigma"]);
assert_eq!(g.eval(0.0), 2.0);
# Ok::<(), oxiroot::Error>(())
```

A mistake is reported where it is, with a hint for the common ones:

```text
bad formula: unexpected character `&`
  x > 0 & x < 1
        ^
  help: ROOT writes logical AND as `&&`
```

A few spellings ROOT itself does not accept are taken too (`ln`, a
one-argument `sign`, `min`), and a function is always stored as ROOT spells it
(`log`, `TMath::Sign(1.,x)`, `TMath::Min`), so ROOT compiles every function
oxiroot writes.

## Parameters

`npar()` is the number of parameters and `param_names()` their names, by
index: `p0`, `p1`, … for numbered parameters. Read them with `params()`,
`param(i)` or `param_named("Mean")`; set them all with `with_params(&[…])`,
which checks their number, one by name with `with_param` (or `set_param` in
place), or change them through `params_mut()`. A file stores the names with
the function, and ROOT reads them back.

`title()` is the formula as written (ROOT's `fTitle`); `formula()` is the
formula as the file stores it (`fFormula`): ROOT's spelling, each parameter by
its name.

## Read and write

Functions read and write through the same `WriteRoot`/`ReadRoot` traits as every
other object — [Reading & writing](reading-writing.md):

```rust
use oxiroot::prelude::*;
let f = Func1D::new("[0]*exp(-[1]*x)", 0.0..=5.0)?.named("resp").with_params(&[10.0, 0.5])?;
FileWriter::create("func.root").add(&f).compression(Compression::Stored).write()?;             // a standalone Func1D key
let back = FileReader::open("func.root")?.get::<Func1D>("resp")?;
assert!((back.eval(2.0) - f.eval(2.0)).abs() < 1e-12);
# Ok::<(), oxiroot::Error>(())
```

They also go into a multi-object file or a subdirectory via the `FileWriter`
builder, and a graph's attached fitted functions (`fFunctions`) use the same
`Func1D`/`TFormula` serialization — see [Graphs](graphs.md).

A `Func1D` and a graph's `GraphFunction` are the same ROOT record, so they convert
both ways: `to_graph_function()` attaches a function to a graph, and
`Func1D::from_graph_function` turns an attached function back into something you
can evaluate:

```rust
use oxiroot::prelude::*;
let f = Func1D::new("[0]+[1]*x", 0.0..=2.0)?.named("line").with_params(&[1.0, 2.0])?;
let g = Graph::new(vec![0.0, 1.0, 2.0], vec![1.1, 2.9, 5.2])?
    .named("g")
    .with_function(f.to_graph_function());
let attached = Func1D::from_graph_function(g.functions()[0].clone())?;
assert_eq!(attached.eval(1.5), f.eval(1.5));
# Ok::<(), oxiroot::Error>(())
```

ROOT C++ and uproot both read oxiroot's `Func1D`, `Func2D`, and `Func3D` and re-evaluate
them: oxiroot embeds the `Func1D`/`Func2D`/`Func3D`/`TFormula` `TStreamerInfo` (including
the `std::vector<double>`/`std::map` members) so uproot builds a model for each.

## Fitting to a function's shape

To fit data to a formula, build a [`Model`](fitting.md) from it — either directly
with `Model::from_formula`, or from an existing `Func1D` with `to_model()` (the
`fit` feature of `oxiroot` or `oxiroot-hist-func`) — and fit any histogram,
graph, or point set:

```rust
use oxiroot::prelude::*;
use oxiroot::fit::{FitExt, Model};
let mut h = Hist::reg(60, 80.0, 100.0).double().named("mass");
// … fill h …
let model = Model::from_formula("peak", "gaus(0) + pol1(3)")?
    .with_params(vec![100.0, 91.0, 2.5, 10.0, 0.0]);
let result = h.fit(&model);
println!("chi2/ndf = {:.2}", result.chi2_per_ndf());
# Ok::<(), oxiroot::Error>(())
```
