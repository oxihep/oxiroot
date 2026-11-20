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

### Done since this roadmap was written

- **oxihist** ([#18](https://github.com/oxihep/oxihist/pull/18)):
  deserialisation goes through the constructors' checks, and
  `Axis::try_integer` refuses spans that overflow.
- **oxifit** ([#11](https://github.com/oxihep/oxifit/pull/11)): a
  Nelder–Mead fit is `valid` only when the simplex converged.
- **oxistats** ([#6](https://github.com/oxihep/oxistats/pull/6)): runnable
  examples.
- **oxiplot** ([#12](https://github.com/oxihep/oxiplot/pull/12),
  [#13](https://github.com/oxihep/oxiplot/pull/13)):
  - stacked histograms and category axes;
  - the legend keeps off the fit-statistics box;
  - SVGs are sized in points;
  - large heatmaps are drawn as one image;
  - golden-image tests;
  - a narrow axis gets at least two ticks.

- **Hosted API docs** for oxihist, oxiplot, oxifit, oxistats and
  oxiparticle, deployed to each repo's GitHub Pages on every push to
  `main` (e.g. <https://oxihep.github.io/oxiplot/>).

- **oxihist category axes in plots** (oxihist 0.6.0): a histogram over a
  string, integer-category or boolean axis names its bins in plots.

All of these are released: oxifit 0.1.2, oxiplot 0.4.0, oxihist 0.6.1, and
oxiroot pins them.

## Then

Larger pieces, each worth a design note before code.

### Done

- **Remote reads take fewer round trips**
  ([oxiroot#70](https://github.com/oxihep/oxiroot/pull/70)). Readers
  announce a branch's baskets or a column's pages, and the file fetches them
  in batches of up to 1024 ranges or 32 MiB: one XRootD `kXR_readv` per
  batch, or up to eight concurrent HTTP range requests. About 30 % faster
  on 3 million entries from EOS here; more on higher-latency links.
- **Hostile-server tests** (same PR). The XRootD mock plays short reads, a
  connection dropped mid-reply, an error status, and vector-read replies at
  the wrong offset, cut short or with extra bytes; each ends in an error.
  (A server that stalls is cut off by the 30 s socket timeout; it is not
  tested, to keep the suite fast.)
- **Shared inverse-CDF sampling.** oxihist's `BinnedSampler` (0.6.2) is the
  one implementation; oxiroot's `Hist1D` and `Func1D` use it.
- **Error causes.** oxiplot's `Error::Io` now reports its cause through
  `source()` (0.4.2); oxiroot's facade error already chained every wrapped
  error. `oxiroot_io_core::Error::Io` keeps the I/O error's kind and
  message rather than the error itself, by design: the type is `Clone` and
  `Eq`, which a `std::io::Error` is not.
- **`#[non_exhaustive]` and `Debug` across the repos.** Every public error
  and option type has both (oxifit 0.8.0, oxiplot 0.10.0, oxihist 0.12.0,
  oxiroot); oxiplot's public enums that may grow are non-exhaustive.
- **Less parallel code.** The plain histograms' shared methods (arithmetic,
  extrema, statistics, bookkeeping) come from one macro (`family.rs`). The
  profiles share `add` and their per-cell code — filling, value, error,
  effective entries — the same way
  ([oxiroot#92](https://github.com/oxihep/oxiroot/pull/92)). The readers and
  writers stay per class, as ROOT's on-disk layouts differ.
- **The legend keeps off text annotations**
  ([oxiplot#30](https://github.com/oxihep/oxiplot/pull/30)). Automatic
  placement now avoids `text` labels as well as the fit-statistics box.
- **A ROOT-C++-confirmed RNTuple `std::map` write**
  ([oxiroot#93](https://github.com/oxihep/oxiroot/pull/93)). ROOT 6.40 writes
  and reads a `std::map` field once the map's dictionary is compiled in; it
  was only the dictionary-less (and interpreted) path that crashed. The
  interop job's ROOT oracle now reads oxiroot's map field, and ROOT-written
  map fixtures are read in the tests.
- **XRootD over TLS, with token authentication**
  ([oxiroot#94](https://github.com/oxihep/oxiroot/pull/94), the `xrootd-tls`
  feature). `roots://` and servers that require TLS switch to TLS before the
  login, and the client authenticates with a WLCG bearer token (`ztn`). Tested
  against a mock server in CI and a real `xrootd` locally
  (`scripts/xrootd_tls_local.sh`).
- **API review** (oxiroot 0.6.0, with oxistats 0.2.0, oxifit 0.9.0, oxiplot
  0.11.0 and oxihist 0.13.0). Names that collided across crates are renamed,
  public fields are private behind accessors, and options builders take one
  form: toggles named after their option, taking a `bool`.
- **Cast lints in every workspace.** `cast_possible_truncation`,
  `cast_sign_loss`, `cast_possible_wrap` and `cast_lossless` are on; each hit
  is fixed or justified where it stands. In oxiroot the `i32`/`usize` mixing
  of ROOT's on-disk types goes through `SaturatingFrom` (a value out of range
  clamps instead of wrapping), same-width reinterpretations are explicit
  (`cast_signed`/`cast_unsigned`), and the bit-level encodings say why they
  truncate.

### Still open

- **XRootD GSI (X.509 proxy) authentication.**
  - *Today:* `unix` and token (`ztn`, over TLS) authentication ship, which
    covers open data and token-based access. WLCG is moving from X.509 to
    tokens.
  - *Done when:* a GSI-only server opens with the user's X.509 proxy.
  - *Design first:* GSI is XRootD's own multi-step handshake (a Diffie-Hellman
    key agreement, then the signed proxy chain), and needs a test server with
    a grid CA and a proxy issued from it.

## Toward 1.0

The API review is done (see above): the surface is what 1.0 would freeze.

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
