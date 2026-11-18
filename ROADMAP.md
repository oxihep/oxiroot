# Roadmap

Where the oxihep crates go next. They are pre-1.0: the API may still change
between minor versions, and they are used from GitHub, pinned to release tags,
rather than from crates.io. Every IO item targets the bar of what already
ships: byte-level round-trips checked against both ROOT C++ and uproot.

The items are grouped by milestone, then by crate. Each says what is missing
today and what done looks like, so it can be picked up on its own.

## Where things stand

A full review of the workspace (code, architecture, performance, tests, docs,
CI) found 165 issues. All its critical and high-priority fixes have landed:

- bounded decompression and recursion;
- crafted-input panics turned into errors;
- protected releases, with tag lockstep checked in CI;
- one shared `ThreadedHist`;
- no quadratic paths left;
- a fallible oxihist → ROOT bridge;
- compiled guide snippets;
- CI on macOS, a pinned ROOT version, and weekly fuzzing.

Two follow-ups have since shipped:

- Criterion benchmarks (see [Performance](https://oxihep.github.io/oxiroot/reference/performance/)).
- A usage and layout review of oxiplot, released as 0.3. It added log axes,
  legend placement, figure layout around labels, and PDF pages sized in points.

What remains from that review is medium- and low-priority. The parts worth
doing are listed below.

## Next

Small, independent fixes, each a PR in one repo.

### Correctness and validation

- **oxihist: deserialisation skips the constructors' checks.** With the
  `serde` feature, `Axis`, `Histogram` and the storages derive `Deserialize`
  directly, so a hand-edited or hostile file can build an axis with
  descending edges or a storage of the wrong length.
  - *Done when:* deserialisation goes through the constructors
    (`#[serde(try_from = …)]`) and malformed input is an error, with a test
    per invariant.
- **oxihist: `Axis::integer` over a very wide range.** The bin count is
  `hi - lo`; near the ends of `i64` that overflows.
  - *Done when:* the span is checked, with a clear error or panic message,
    and the extremes are tested.
- **oxifit: the Nelder–Mead fallback reports `valid` from a finite χ²
  alone.** A fit stopped by the 10 000-iteration cap still says it is valid.
  - *Done when:* `valid` also requires convergence (the simplex tolerance was
    met before the cap), as Minuit's `is_valid()` does, with a test that
    caps the iterations.

### Plotting (oxiplot)

- **Stacked histograms.**
  - *Today:* a stack is drawn by hand, as cumulative fills, tallest first.
  - *Done when:* `Axes::hist_stack(&[&h1, &h2, …], opts)` sums the layers,
    draws them in order, and gives each a legend entry, as mplhep's
    `histplot(stack=True)` does.
- **Categorical axes.**
  - *Today:* the data traits describe numeric bins only, so a string-category
    oxihist histogram plots against made-up numeric edges.
  - *Done when:* there is a category-aware trait (or trait method), with
    labelled ticks at the bin centres.
- **The legend's `Best` placement ignores the fit-statistics box.** The box
  stacks under the legend when they would overlap, but `Best` may still put
  the legend over the box's corner.
  - *Done when:* `Best` counts the box as data to avoid.
- **SVG sized in points.**
  - *Today:* the SVG's `width`/`height` are pixels at the figure's DPI, while
    PDF pages are now in points.
  - *Done when:* the SVG states its size in `pt` (as matplotlib does), so a
    6.4 × 4.8 in figure is that size wherever it is opened.
- **Heatmaps as one image.**
  - *Today:* `hist2d` emits one rectangle per bin, so a 500 × 500 map is a
    250 000-element SVG or PDF.
  - *Done when:* the mesh is embedded as a single raster image (PNG in SVG,
    an image XObject in PDF), keeping the per-bin path for small maps.
- **Golden-image tests.**
  - *Today:* the render tests read geometry back from the SVG, but nothing
    compares a whole picture.
  - *Done when:* a few reference renders are checked in, compared with a
    tolerance, and re-recorded with an explicit flag (as oxiplot-tex already
    does for its formula renders).

### Documentation

- **Hosted API docs for every repo.**
  - *Today:* only oxiroot publishes a docs site; oxihist, oxiplot, oxifit,
    oxistats and oxiparticle have rustdoc but no hosted copy.
  - *Done when:* each repo deploys `cargo doc` to its GitHub Pages on every
    push to `main`, linked from its README.
- **Examples for oxistats.**
  - *Done when:* the distributions, lineshapes and intervals have runnable
    examples, and oxiroot's guide links to them.

## Then

Larger pieces, each worth a design note before code.

### Remote reads

- **Fewer round trips.**
  - *Today:* every basket or page is one blocking request, over both XRootD
    and HTTP.
  - *Done when:* reading a cluster asks for all its baskets or pages at once
    (XRootD `kXR_readv`, HTTP multi-range requests) and caches them, with
    tests against the mock servers. This is where remote reads lose most of
    their time.
- **XRootD authentication beyond `unix`.**
  - *Done when:* GSI / X.509 and token (ZTN) authentication open
    access-controlled data. `root://` with `unix` auth, and HTTP(S) range
    reads, already ship.
- **Hostile-server tests.** The XRootD client is fuzzed for parsing but not
  driven by a misbehaving server (short reads, wrong lengths, stalls).
  - *Done when:* the mock server can play each of these, and every case ends
    in an error, not a hang or a panic.

### RNTuple

- **A ROOT-C++-confirmed `std::map` write.**
  - *Today:* oxiroot writes a spec-compliant `std::map` field that uproot reads
    and oxiroot round-trips (`Field::map`).
  - *Blocked on:* ROOT 6.40's `std::map` collection proxy, which can neither
    create nor read such a field in the test build.
  - *Done when:* a ROOT install with the `std::map` dictionary reads it back in
    the interop job.

### Code health

- **One error contract per crate.** Some errors drop their cause; make
  `source()` chain through every wrapped error, and give every public error
  `#[non_exhaustive]` and `Debug`, consistently across the repos.
- **Less parallel code.**
  - The ROOT histogram types repeat their 1-D/2-D/3-D logic (about 2000 lines).
    Generate it or share it, behind the same public API.
  - The inverse-CDF sampling in `oxiroot-hist` and `oxiroot-hist-func` should
    become one function.
- **Lints.** Turn on the cast and idiom lints (`cast_possible_truncation`,
  `cast_sign_loss`, …) workspace by workspace, fixing or justifying each hit.
  Settle the `i32`/`usize` mixing they surface.

## Toward 1.0

- **API review.** Walk each crate's public surface once more:
  - names that collide across crates;
  - public fields that should be private with accessors;
  - options builders that disagree.
  Then freeze it.
- **Publishing.** The crates stay on GitHub until it is decided to publish.
  - *When it is:* publish in dependency order (oxistats → oxiparticle →
    oxifit → oxiplot → oxihist → oxiroot), drop the `git`/`tag` from each
    manifest, and turn the `package (dry run)` job into a real gate.
  - [`RELEASING.md`](RELEASING.md) describes the tag lockstep that would carry
    over.

## Far future

- **`RDataFrame`-style analysis.** A lazy, columnar front-end built on the
  existing readers:
  - `Define`, `Filter`, `Histo1D` and `Sum`, run in one pass over a `TTree` or
    RNTuple;
  - parallelised across clusters.
  This is a compute engine rather than IO, and the natural cap on the stack
  once the IO layer is complete. ROOT's `RNTupleProcessor` has the same shape
  and would share the machinery.

## Out of scope

- **ROOT 7 `RHist`.** It has no persistable on-disk format: its `Streamer`
  throws.
- **ROOT's graphics objects** (`TCanvas`, `TPad`, …), read or written. The
  `plot` feature (oxiplot) renders the data; it does not (de)serialise ROOT
  graphics.
