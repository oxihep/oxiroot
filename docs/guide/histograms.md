# Histograms

A histogram in oxiroot is an [oxihist](https://github.com/oxihep/oxihist)
`Histogram`: you build it with `Hist`, fill it, and analyse it (arithmetic,
statistics, projections, tests, fits, plots). ROOT's classes
(`TH1`/`TH2`/`TH3`, `TProfile*`) are *records*: `Hist1D`/`Hist2D`/`Hist3D` and
`Profile1D`/`Profile2D`/`Profile3D` hold exactly what a ROOT file stores, and
exist to be written and read. One call goes each way:

```rust
use oxiroot::prelude::*;

// Build and fill the histogram…
let mut h = Hist::reg(50, 0.0, 100.0).weight().named("pt");
h.fill_weight(42.0, 1.5);

// …turn it into the ROOT record to store it…
FileWriter::create("pt.root").add(&h.to_root()?).write()?;

// …and read the record back as a histogram.
let back = FileReader::open("pt.root")?.get::<Hist1D>("pt")?.to_hist()?;
assert_eq!(back.values(false), h.values(false));
```

This page covers construction, filling, operations, statistics, derived
histograms, compatibility tests, and what the records add for ROOT (class
names, axis labels, time axes, persistence). `Efficiency`, `SparseHist` and
`PolyHist` have no oxihist counterpart and are filled directly. (ROOT 7 `RHist`
has no on-disk format and is intentionally out of scope.)

!!! info "What survives the round trip"
    `to_root()` writes the bins, their variances, the entry count and the
    statistics of the fills (`fTsumw`, `fTsumwx`, …); `to_hist()` restores them.
    The [conversion table](../api/oxiroot_hist/index.html) lists how each axis
    and storage maps onto ROOT's classes.

    Some oxihist features have no ROOT counterpart: an `Integer` axis reads back
    as a `Regular` one, an `IntCategory` or `Boolean` axis as a `StrCategory`
    one, and an axis's growth flag and name are dropped (its label is kept).
    `to_root()` returns an error for the rare histogram ROOT cannot hold at all
    (an axis of more than `i32::MAX` bins).

## Construction without names

A histogram is just data: a binning and its contents. Unlike ROOT, oxiroot never
forces a name at construction and keeps no global directory, so any number of
anonymous (or same-named) histograms coexist in memory. A name is only needed
when you *persist* the object — it becomes the file key.

```rust
use oxiroot::prelude::*;

// Anonymous histograms — no gROOT, no gDirectory.
let mut h = Hist::reg(100, 0.0, 100.0).double();

// Name and title are fluent, chainable, and set whenever you like.
let mut pt = Hist::reg(100, 0.0, 100.0).double()
    .named("pt")
    .titled("p_{T} spectrum");
```

`named` sets the on-disk key (`fName`); `titled` sets the title (`fTitle`). Both
return `Self`, so they chain off the constructor.

### Uniform vs variable (irregular) bins

`Hist::reg(nbins, xmin, xmax)` declares uniform bins over `[xmin, xmax)`. For
**irregular** bins of arbitrary width, `Hist::var(edges)` takes the `nbins + 1`
bin boundaries directly (they must be strictly ascending):

```rust
use oxiroot::prelude::*;

let uniform = Hist::reg(100, 0.0, 100.0).double();

// Arbitrary edges → 5 bins of widths 1, 1, 3, 5, 90.
let variable = Hist::var(&[0.0, 1.0, 2.0, 5.0, 10.0, 100.0]).double();
```

Chain another `reg`/`var` per axis for higher dimensions:
`Hist::reg(nx, xlo, xhi).reg(ny, ylo, yhi)` is 2-D, and a third
`.reg(nz, zlo, zhi)` makes it 3-D. `reg` and `var` mix freely on any axis.

### Storage

The call that ends the builder picks what each bin holds:

| Finalizer | A bin holds | Record from `to_root()` |
| --- | --- | --- |
| `.double()` | a sum of weights | `TH1D`/`TH2D`/`TH3D`, no `fSumw2` |
| `.int64()` | an integer count | `TH1L`/`TH2L`/`TH3L` |
| `.weight()` | a sum of weights and its variance (Σw²) | `TH1D`/`TH2D`/`TH3D` with `fSumw2` |
| `.mean()` | the mean of a sampled value | `TProfile`/`TProfile2D`/`TProfile3D` |
| `.weighted_mean()` | the weighted mean of a sampled value | `TProfile`/`TProfile2D`/`TProfile3D` |

## Filling

`fill` adds an entry with unit weight and `fill_weight` one with weight `w`,
each taking one value per axis. A profile is filled with `fill_sample`
(`fill_sample_weight` for a weighted one), which takes the sampled value too.

```rust
use oxiroot::prelude::*;

let mut pt = Hist::var(&[0.0, 10.0, 20.0, 40.0, 80.0, 160.0]).weight().named("pt");
for &(x, w) in &[(5.0, 1.2), (15.0, 0.8), (35.0, 1.5)] {
    pt.fill_weight(x, w);
}

let mut map = Hist::reg(10, 0.0, 1.0).reg(10, 0.0, 1.0).double();
map.fill(0.25, 0.75);

let mut prof = Hist::reg(5, 0.0, 5.0).mean().named("pt_prof").titled("<pt> per region");
prof.fill_sample(0.5, 91.2);
prof.fill_sample(1.5, 125.1);
let profiled = prof.values(false); // the mean per in-range bin
```

Every fill counts as an entry (`entries()`), and the statistics of the fills
(`fill_stats()`: Σw, Σw², Σwx, Σwx², …) accumulate for in-range fills only —
ROOT's exact `Fill` semantics, so a stored histogram carries the same
`fEntries` and `fTsumw*` ROOT would have computed.

### Per-bin errors

Per-bin errors come from the storage, not from a switch: a `.weight()`
histogram keeps Σw² for every bin (ROOT's `fSumw2`), a `.double()` one does not.
Pick `.weight()` whenever the fills are weighted or the histogram will be
scaled, and read the variances with `variances(flow)`:

```rust
use oxiroot::prelude::*;

let mut h = Hist::reg(4, 0.0, 4.0).weight();
h.fill_weight(0.5, 2.0);
h.fill_weight(0.5, 2.0);

assert_eq!(h.value([0]), Some(4.0));      // Σw
assert_eq!(h.variance([0]), Some(8.0));   // Σw²
let errors: Vec<f64> = h.variances(false).iter().map(|v| v.sqrt()).collect();
```

!!! note
    A histogram's bins are indexed from `0`, with the flow bins at `-1`
    (underflow) and `n` (overflow); `values(flow)`, `variances(flow)`,
    `integral(flow)` and `sum(flow)` say whether to include them. A *record*
    is laid out as ROOT's arrays: cell `0` is underflow, `1..=nbins` are in
    range and `nbins + 1` is overflow; `contents()` covers every cell and
    `values()` the in-range ones.

## Operations

| Operation | Call | Notes |
| --- | --- | --- |
| scale | `h.scale(c)` | variances scale by `c²` |
| add | `h.add(&other)?` | bin by bin, entries and fill statistics included |
| multiply | `h.multiply(&other)?` | 1-D |
| divide | `h.divide(&other)?` | 1-D; `0` where the denominator is `0` |
| integral | `h.integral(flow)` | sum of the bins, with or without the flow bins |
| reset | `h.reset()` | clear the contents, keep the binning |

`add`, `multiply` and `divide` return an error and make no change if the
binnings differ.

```rust
use oxiroot::prelude::*;

let mut signal = Hist::reg(10, 0.0, 10.0).weight();
let mut background = signal.clone();
signal.fill(2.5);
background.fill(7.5);

background.scale(0.1);                              // scale background down
signal.add(&background)?;                           // stack it onto signal
signal.scale(1.0 / signal.integral(false).max(1.0)); // normalize to unit area
```

Single bins are read with `value([i])` / `variance([i])` and written through
`bin_mut([i])`, which is how a histogram is built from numbers that were not
filled.

## Statistics and shape

| Accessor | Meaning |
| --- | --- |
| `entries()` | number of fills |
| `effective_entries()` | `(Σw)² / Σw²` |
| `mean()` / `std_dev()` | mean and standard deviation of the in-range fills (2-D and 3-D take the axis: `mean(0)`, `std_dev(1)`) |
| `maximum()` / `minimum()` | the extreme in-range bin, as `Some((index, content))` |
| `interpolate(x)` | linear interpolation between adjacent bin centers; `None` outside the axis |
| `quantiles(&probs)` | x values where the cumulative reaches each probability |
| `fill_stats()` | the sums the fills accumulated, `None` once they no longer describe the bins |

```rust
use oxiroot::prelude::*;

let mut mass = Hist::reg(100, 0.0, 100.0).double();
for x in [48.0, 50.0, 50.0, 52.0] {
    mass.fill(x);
}

println!("mean {:.3}, std {:.3}", mass.mean(), mass.std_dev());
if let Some((bin, content)) = mass.maximum() {
    println!("max bin {} = {content}", bin[0]);
}

let quartiles = mass.quantiles(&[0.25, 0.5, 0.75]);
let mid = mass.interpolate(50.0); // content at x = 50
```

`mean` and `std_dev` use the sums of the fills when the histogram has them, as
ROOT does, and fall back to an estimate from the bin centers when it does not
(a histogram built bin by bin, or read from a file written without them).

The axes are `h.axis(i)`, with `edges()` (`None` for a categorical axis) and
`centers()`.

## Derived histograms

Each derived operation returns a new histogram and leaves the source untouched.

| Operation | Description |
| --- | --- |
| `rebin(axis, factor)?` | merge `factor` adjacent bins along `axis` |
| `slice(lo..hi)?` | keep the bins inside a range of a 1-D histogram |
| `cumulative(forward)` | running sum of a 1-D histogram, forward or reverse |
| `project([axes])?` | keep the listed axes and sum over the others |
| `profile(axis)?` | profile a 2-D histogram along `axis` |

```rust
use oxiroot::prelude::*;

let mut fine = Hist::reg(100, 0.0, 100.0).double();
fine.fill(42.0);
let coarse = fine.rebin(0, 4)?;      // group every 4 bins
let cdf = fine.cumulative(true);     // forward running sum
let window = fine.slice(40.0..60.0)?;

let mut corr = Hist::reg(20, 0.0, 1.0).reg(20, 0.0, 1.0).double();
corr.fill(0.3, 0.6);
let px = corr.project([0])?;         // 2-D → 1-D along x
let prof = corr.profile(0)?;         // mean of y in each x bin
```

`slice` keeps the bins that lie inside the range: a bin that starts at the
upper bound is left out.

## Sampling and smoothing

Draw random values from a histogram's distribution (ROOT's `GetRandom` /
`FillRandom`) — the bin contents are the density, a bin is picked in proportion
to its content, and the value is interpolated within it. Sampling needs a uniform
source; oxiroot has no `gRandom`, so a small seedable [`Random`](../api/oxiroot/index.html)
is provided (no `rand` dependency, reproducible draws).

```rust
use oxiroot::prelude::*;
use oxiroot::hist::Random;

let mut source = Hist::reg(100, -5.0, 5.0).double();
for x in [-1.0, 0.0, 0.0, 1.0] {
    source.fill(x);
}

let mut rng = Random::seed(12345);
let x = source.sample(&mut rng);            // one draw; None for an empty histogram

// Fill a new histogram with 100k draws from `source`'s shape (efficient — the
// cumulative is built once):
let mut resampled = Hist::reg(100, -5.0, 5.0).double();
resampled.fill_random(&source, 100_000, &mut rng);

// …or draw from a function over the histogram's range:
resampled.fill_random_fn(|x| (-0.5 * x * x).exp(), 100_000, &mut rng);
let g = Func1D::gaus(-5.0..=5.0).named("g").with_params(&[1.0, 0.0, 1.0])?;
let y = g.get_random(&mut rng);             // TF1::GetRandom
```

`smooth(times)` applies ROOT's `353QH, twice` smoother to the in-range bins
(matching `TH1::Smooth`):

```rust
use oxiroot::prelude::*;
let mut h = Hist::reg(100, 0.0, 100.0).double();
# for x in [1.0, 2.0, 3.0] { h.fill(x); }
h.smooth(2);
```

## Compatibility tests

A 1-D histogram supports ROOT's `Chi2Test` and `KolmogorovTest`, returning
ROOT-matched p-values. Both require identical binning and return an error
otherwise.

```rust
use oxiroot::prelude::*;
use oxiroot::hist::oxihist::Chi2TestKind;

let mut data = Hist::reg(10, 0.0, 10.0).double();
let mut model = data.clone();
for x in [1.5, 2.5, 2.5, 3.5, 6.5] {
    data.fill(x);
    model.fill(x);
}
model.fill(4.5);

let chi2 = data.chi2_test(&model, Chi2TestKind::UnweightedUnweighted)?;
println!("χ²/ndf = {:.2}, p = {:.3}", chi2.chi2 / chi2.ndf as f64, chi2.p_value);

let ks = data.kolmogorov_test(&model)?;
println!("KS distance {:.4}, prob {:.3}", ks.distance, ks.prob);
```

`Chi2TestKind` selects the weighting scheme: `UnweightedUnweighted` (ROOT's
`"UU"`), `UnweightedWeighted` (`"UW"`), or `WeightedWeighted` (`"WW"`).
`Chi2TestResult` carries `p_value`, `chi2`, and `ndf`; `KsTestResult`
carries `prob` and `distance`.

## The ROOT records

`to_root()` gives the record of a histogram, and `file.get::<Hist1D>(path)?`
the record a file holds. A record is what ROOT stores, field for field, so it
reads and writes byte-for-byte:

| Record | Class(es) | From |
| --- | --- | --- |
| `Hist1D` | `TH1D/F/I/S/C/L` | a 1-D count histogram |
| `Hist2D` | `TH2D/F/I/S/C/L` | a 2-D count histogram |
| `Hist3D` | `TH3D/F/I/S/C/L` | a 3-D count histogram |
| `Profile1D` | `TProfile` | a 1-D `.mean()` / `.weighted_mean()` histogram |
| `Profile2D` | `TProfile2D` | a 2-D one |
| `Profile3D` | `TProfile3D` | a 3-D one |
| `Efficiency` | `TEfficiency` | `Efficiency::new(nbins, xlo, xhi)`, then `fill(passed, x)` |
| `SparseHist` | `THnSparseT<TArrayD>` | `SparseHist::new(&[(nbins, lo, hi), …])`, then `fill(&coords)` |
| `PolyHist` | `TH2Poly` | `PolyHist::new(xlow, xup, ylow, yup)`, then `add_bin`/`add_bin_rect` |

A record is not a second histogram to compute with. It offers what the file
format has and oxihist does not:

- the stored fields, read-only: `contents()` (every cell, flow included),
  `values()`, `sumw2()`, `entries()`, `tsumw()`, `tsumwx()`…, and the axes
  (`xaxis()`/`yaxis()`/`zaxis()`, an [`Axis`](../api/oxiroot/index.html) with
  `edges()`, `find_bin(x)` and the 1-based `bin_center`/`bin_width`, `None`
  for a bin the axis does not have);
- the on-disk bin type and class name (below);
- axis metadata: [bin labels](#labelled-alphanumeric-axes) and
  [time formats](#time-axes), through `xaxis_mut()`;
- `add_scaled(&other, c)`, the bin-by-bin merge `hadd` performs on records without
  converting them;
- on a `Hist1D`, the functions stored with it (`functions()`, `with_function`):
  the curve ROOT's `TH1::Fit` leaves with a histogram, kept through a read and
  a write and drawn over the histogram by ROOT;
- `Display` on `Hist1D`/`Hist2D`/`Hist3D`, a one-line summary such as
  `TH1D "pt": 100 bins [0, 100), entries=4096`.

Everything else — filling, statistics, arithmetic, projections, fits, plots —
is on the histogram: call `to_hist()?` on a record you read.

A profile's per-bin error follows its typed
[`ErrorMode`](../api/oxiroot/index.html) (`Mean`, `Spread`, `SpreadI`,
`SpreadG`, mapping ROOT's `fErrorMode`). `Efficiency` wraps a passed/total pair
of `TH1D`s; `PolyHist` supports arbitrary-shape polygon bins
(`add_bin(&xs, &ys)`) and axis-aligned rectangles
(`add_bin_rect(xmin, ymin, xmax, ymax)`), both returning the new bin number.

### Bin content type and class names

A `Hist1D`/`Hist2D`/`Hist3D` keeps its bin contents as `f64` in memory regardless of
their on-disk type. The class suffix (`D`/`F`/`I`/`S`/`C`/`L`) is a typed
[`BinContentType`](../api/oxiroot/index.html) value. `to_root()` picks it from
the storage (`double()` and `weight()` → `TH1D`, `int64()` → `TH1L`), and
`with_bin_content_type` picks any other (`F32` → `TH1F`, `I32` → `TH1I`,
`I16` → `TH1S`, `I8` → `TH1C`), on a record you just converted or one you read
back; the contents are narrowed only at write time.

```rust
use oxiroot::prelude::*;
use oxiroot::hist::BinContentType;

let h = Hist::reg(100, 0.0, 1.0).double().named("h")
    .to_root()?
    .with_bin_content_type(BinContentType::F32);
assert_eq!(h.class_name(), "TH1F");

// Retype an existing record (e.g. store a filled TH1D compactly):
let hc = h.clone().with_bin_content_type(BinContentType::I8);
assert_eq!(hc.class_name(), "TH1C");
```

| Method | Returns |
| --- | --- |
| `bin_content_type()` | the typed `BinContentType` (the class suffix) |
| `with_bin_content_type(t)` | `Self` with the on-disk bin content type set |
| `class_name()` | the exact ROOT class, e.g. `"TH1D"` / `"TH2F"` |

`BinContentType` covers `F64` (`D`, the default), `F32` (`F`), `I32` (`I`),
`I16` (`S`), `I8` (`C`), and `I64` (`L`).

## Labelled (alphanumeric) axes

A record's `Axis` can carry alphanumeric bin labels (`fLabels`), which round-trip
through ROOT files in both directions.

```rust
use oxiroot::prelude::*;

let mut h = Hist::reg(3, 0.0, 3.0).double().named("by_region").to_root()?;
h.xaxis_mut().set_label(1, "barrel")?;
h.xaxis_mut().set_label(2, "endcap")?;
h.xaxis_mut().set_label(3, "forward")?;

assert_eq!(h.xaxis().bin_label(2), Some("endcap"));
assert_eq!(h.xaxis().find_label("forward"), Some(3));
```

`set_label(bin, text)` and `bin_label(bin)` use 1-based bin numbers; `labels` is
the raw `Vec<String>`, `find_label` maps a label back to its bin, and
`is_labelled` reports whether any label is set.

## Time axes

An axis whose values are times says so with `set_time_format` on the record,
ROOT's `fTimeDisplay` / `fTimeFormat`. The format is a `strftime` format, and `%F`
after it names the epoch the values count from — exactly what ROOT writes:

```rust
use oxiroot::prelude::*;

// One bin per hour of 2024-01-01, x in seconds from the start of that day.
let mut rate = Hist::reg(24, 0.0, 86_400.0).double().named("rate");
rate.fill(3_600.0);

let mut h = rate.to_root()?;
h.xaxis_mut().set_time_format("%H:%M%F2024-01-01 00:00:00");

assert!(h.xaxis().is_time());
```

ROOT and uproot read the axis back as a time axis, and so does oxiroot.
`clear_time_format` turns it back into a numeric axis.

A graph's axes live on its display frame (`graph.histogram()`), which is where
ROOT keeps them too, so a time axis on a graph is set on the frame before it
is attached with `with_histogram`.

When [plotting](plotting.md), the axis is picked up on its own: the tick labels
are drawn as dates and clock times, and the ticks step in seconds, minutes,
hours or days rather than in decimals. `Axes::x_time_format` sets the same thing
for data that carries no axis of its own.

## Writing and reading

`FileWriter` composes records (and subdirectories) into one file, and
`FileReader::get` reads one back by path. Written files embed a `TStreamerInfo` list, so they are
self-describing for ROOT and uproot.

### One object

```rust
use oxiroot::prelude::*;

let h = Hist::reg(100, 0.0, 1.0).double().named("h").to_root()?;
FileWriter::create("out.root").add(&h).write()?; // works for any writable type

let bytes = h.to_root_bytes(); // just the streamed object payload
```

A stored object needs a non-empty name; writing an unnamed one is an error.
`to_root_bytes` returns the streamed payload (no file/key framing).

### Several objects with `FileWriter`

```rust
use oxiroot::prelude::*;

let pt = Hist::reg(10, 0.0, 1.0).double().named("pt").to_root()?;
let prof = Hist::reg(10, 0.0, 1.0).mean().named("prof").to_root()?;
let signal = Hist::reg(10, 0.0, 1.0).double().named("sig").to_root()?;

FileWriter::create("out.root")
    .add(&pt)                            // any record: hist, profile, graph…
    .add(&prof)
    .dir("by_region", |d| d.add(&signal)) // a TDirectory holding `sig`
    .write()?;
```

Append to an existing file with `FileUpdater::open`. The append is *in place* —
existing objects never move — so files that already contain subdirectories or an
RNTuple are preserved (added objects land in the top directory; only *creating*
new subdirectories while appending is unsupported):

<!-- fragment -->
```rust
use oxiroot::prelude::*;

FileUpdater::open("out.root")?
    .add(&extra)
    .write()?;
```

!!! warning "Loud collisions"
    Writing an object with no name, or two objects sharing a name in one
    directory, is a loud error — `Error::DuplicateName` for the collision case —
    not ROOT's silent shadow-on-read. Give every persisted object a unique key
    with `.named("...")`.

### Reading back

```rust
use oxiroot::prelude::*;

let f = FileReader::open("out.root")?;
let pt = f.get::<Hist1D>("pt")?.to_hist()?; // any of TH1D/F/I/S/C/L
let sig = f.get::<Hist1D>("by_region/sig")?; // the record, from a subdirectory
println!("{sig}: mean of pt {:.3}", pt.mean());
```

`get` auto-detects the on-disk precision (every `TH1D/F/I/S/C/L` reads into
a `Hist1D`, with the exact class preserved in `class_name()`); contents are widened
to `f64`, and `to_hist()` gives a `Histogram<Weight, 1>` to work with. A path with `/` reads from a subdirectory written via `FileWriter::dir`.

See [Compression](compression.md) for the codec options and
[ROOT / uproot interop](interop.md) for the cross-language read/write guarantee.

## Fitting and multithreading

Fitting and multithreaded fills have dedicated pages:

- Fit a parametric model (`gaussian`/`exponential`/`polynomial`, or a closure) to
  any 1-D data — histograms and `Graph` implement `FitData` — via `.fit(&model)`.
  Requires the `fit` feature. See [Fitting](fitting.md).
- `ThreadedHist` is the pure-Rust analog of ROOT's `TThreadedObject<TH1D>`: share
  `&hist`, call `hist.fill(x)` from any thread, then `hist.merge()`. See
  [Multithreading](multithreading.md).

## See also

- [Fitting](fitting.md)
- [Multithreading](multithreading.md)
- [ROOT / uproot interop](interop.md)
- [Quickstart](../getting-started/quickstart.md)
