# Installation

oxiroot is pure Rust. It needs a **Rust 1.100+** toolchain and nothing else — no
libROOT, no Python, no system libraries. All compression codecs are pure-Rust
crates pulled in automatically.

## Add the dependency

oxiroot is not on crates.io yet: depend on it from GitHub. It is pre-1.0, so
the API may still change between versions. Pull in everything through the **facade**, or just the
one crate you need — the histogram, tree, and RNTuple crates are independent, so
a histogram-only project never compiles the others.

```toml
[dependencies]
# Everything — histograms, graphs, TTree, RNTuple — through the facade:
oxiroot = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }

# …or depend on just one crate (Cargo finds it in the repository):
oxiroot-hist      = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }  # histograms + graphs
oxiroot-hist-func = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }  # Func1D/Func2D/Func3D
oxiroot-tree      = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }  # TTree
oxiroot-rntuple   = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }  # RNTuple
```

Then bring the common types into scope with the prelude:

```rust
use oxiroot::prelude::*;
```

## Optional features

The `oxiroot` facade is **batteries-included**: fitting and plotting
are on by default, so there is usually nothing extra to enable. The parallel
helpers (`rayon`), remote reads (`http`, `xrootd`) and the memory-mapped read
path (`mmap`) are opt-in. Opt
out with `default-features = false` (then re-enable à la carte) when you want
just the lean, pure-Rust format core.

| Feature | Default | Effect |
|---------|:---:|--------|
| `mmap` | — | Memory-mapped read path (`unsafe FileReader::open_mmap`) for large files; adds `memmap2`. Opt-in: the file must not change while mapped. `FileReader::open_ranged` reads lazily with no such contract. |
| `rayon` | — | Adds the data-parallel histogram fill (`hist::fill_par`) and the parallel TTree reads (`TreeReader::read_branch_par` and friends); adds `rayon`. Opt-in, so nothing spawns threads unless you ask. |
| `fit` | ✅ | Curve fitting (`oxiroot::fit`, `Hist1D::fit`) via the pure-Rust Minuit2 port; adds `minuit2`. |
| `argmin` | ✅ | Gradient-free Nelder–Mead minimizer backend (`Minimizer::NelderMead`); implies `fit`, adds `argmin`. |
| `plot` | ✅ | Plotting to SVG/PNG/PDF (`oxiroot::plot`, from [oxiplot](https://github.com/oxihep/oxiplot)); adds `tiny-skia`/`skrifa` and the ReX TeX engine. |
| `http` | — | Remote reads over HTTP(S) range requests (`FileReader::open_url` with `http://` or `https://`); adds `ureq`. |
| `xrootd` | — | Remote reads over the XRootD protocol (`FileReader::open_url` with `root://`), pure Rust, `unix` auth for public files. |

```toml
[dependencies]
# Batteries-included — fitting, plotting and argmin all on:
oxiroot = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0" }

# …or the lean format core only (drops the extra dependencies):
# oxiroot = { git = "https://github.com/oxihep/oxiroot", tag = "v0.6.0", default-features = false }
```

!!! tip "Leaner builds"
    The extras pull real dependencies (the Minuit2 port, the ReX TeX engine, …).
    If you only read and write ROOT files, `default-features = false` keeps the
    build minimal; re-enable individual features as you need them.

## Build & test

```sh
cargo build  --workspace
cargo test   --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --all --check
```

The committed tests are pure Rust — they check self-round-trips, byte-level
agreement against committed reference files, and malformed-input hardening, with
no ROOT or Python required.

For a full cross-language interop check against official ROOT (C++) **and**
uproot in both directions, see the [interop guide](../guide/interop.md).

Criterion benchmarks for filling, reading and (de)compression live in
`crates/oxiroot/benches`; see [Performance](../reference/performance.md):

```sh
cargo bench -p oxiroot --bench fill --bench read --bench decompress
```


## Next

→ **[Quick start](quickstart.md)** — write and read a histogram, a `TTree`, and
an RNTuple in a few lines.
