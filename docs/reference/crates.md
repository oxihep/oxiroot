# Crate layout

oxiroot is a Cargo workspace of small, focused crates for the ROOT file
format. The `oxiroot` facade re-exports them and provides the `prelude`; depend
on it for the full surface, or pull in a single leaf crate to compile only what
you use.

The pieces that are not about ROOT live in their own repositories in the
[oxihep](https://github.com/oxihep) organization, the way Python's Scikit-HEP
splits uproot, hist, mplhep and particle. The facade re-exports them too, so
`oxiroot::stat`, `oxiroot::particle`, `oxiroot::fit` and `oxiroot::plot` work as
before.

| Crate | Purpose |
|-------|---------|
| [`oxiroot`](../api/oxiroot/index.html) | Facade: `prelude` + re-exports of the crates below and of the oxihep crates |
| [`oxiroot-io-core`](../api/oxiroot_io_core/index.html) | `TFile` container, buffer primitives, streamer + object-reference engine, the `WriteRoot`/`ReadRoot` object framework, `Error` |
| [`oxiroot-compress`](../api/oxiroot_compress/index.html) | ROOT 9-byte block framing + Zstd/zlib/LZ4/LZMA codecs |
| [`oxiroot-rntuple`](../api/oxiroot_rntuple/index.html) | RNTuple reader/writer (spec v1.0.0.0) |
| [`oxiroot-hist`](../api/oxiroot_hist/index.html) | Histograms, profiles, `Efficiency`/`SparseHist`/`PolyHist`, and the `Graph` family |
| [`oxiroot-hist-func`](../api/oxiroot_hist_func/index.html) | `Func1D`/`Func2D`/`Func3D` parametric functions on `oxifit-formula`, with ROOT read/write |
| [`oxiroot-linalg`](../api/oxiroot_linalg/index.html) | ROOT linear-algebra objects — `Vector`/`Matrix`/`SymMatrix` (`TVectorD`/`TMatrixD`/`TMatrixDSym`), with byte-exact ROOT read/write |
| [`oxiroot-tree`](../api/oxiroot_tree/index.html) | Classic `TTree` read/write |
| `oxiroot-cli` | `oxroot`: a command-line inspector (`ls`/`show`/`dump`/`stat`) |

### The oxihep crates

| Crate | Repository | Purpose | In oxiroot |
|-------|------------|---------|------------|
| `oxistats` | [oxihep/oxistats](https://github.com/oxihep/oxistats) | Dependency-free statistics: special functions, distributions, tests, HEP lineshapes (verified vs `scipy.stats`) | `oxiroot::stat` |
| `oxiparticle` | [oxihep/oxiparticle](https://github.com/oxihep/oxiparticle) | PDG particle data: the numbering-scheme decoder and a bundled particle table | `oxiroot::particle` |
| `oxifit` | [oxihep/oxifit](https://github.com/oxihep/oxifit) | Minuit2 curve fitting for any 1-D data (`FitData`/`Model`) | `oxiroot::fit` (`fit` feature) |
| `oxifit-formula` | [oxihep/oxifit](https://github.com/oxihep/oxifit) | Dependency-free `TFormula` expression engine: parse, evaluate, integrate, differentiate | behind `Func1D` and `Model::from_formula` |
| `oxiplot` | [oxihep/oxiplot](https://github.com/oxihep/oxiplot) | Matplotlib-style SVG/PNG/PDF plotting; draws anything implementing its data traits, which `Hist1D`/`Hist2D`/`Profile1D`/`Graph` do | `oxiroot::plot` (`plot` feature) |
| `oxiplot-tex` | [oxihep/oxiplot](https://github.com/oxihep/oxiplot) | Internal: the vendored ReX TeX math layout engine used by `oxiplot` | — |

## Dependency graph

The leaf crates layer cleanly: `io-core` and `compress` underpin the format
crates (`rntuple`, `hist`, `tree`, `linalg`); the `WriteRoot`/`ReadRoot` object
framework lives in `io-core`, so each format crate registers its own objects.
`oxistats` and `oxifit-formula` are dependency-free leaves: `oxistats` is shared
by `hist` (compatibility tests) and `oxifit` (goodness-of-fit), and
`oxifit-formula` by `hist-func` (the `Func1D`/`Func2D`/`Func3D` functions) and
`oxifit` (formula models). Keeping the functions in `hist-func` means a
histogram-only build never compiles the formula engine. `oxifit` and `oxiplot`
are optional, behind the `fit` and `plot` features. `oxiplot` knows nothing
about ROOT: `hist` implements its data traits, as Python's histogram libraries
implement UHI for mplhep.

```text
oxiroot    -> io-core, compress, rntuple, hist, hist-func, linalg, tree,
              oxistats, oxiparticle, [oxifit], [oxiplot]
hist-func  -> hist, io-core, oxifit-formula, [oxifit]
hist       -> io-core, oxistats, [oxifit], [oxiplot]
tree       -> io-core
rntuple    -> io-core
linalg     -> io-core
io-core    -> compress, [oxistats], [oxifit-formula], [oxiplot]
compress: no oxiroot dependencies
oxiroot-cli (oxroot) -> oxiroot

oxifit     -> oxifit-formula, oxistats        (github.com/oxihep/oxifit)
oxiplot    -> [oxifit], [oxiplot-tex]         (github.com/oxihep/oxiplot)
oxistats, oxiparticle, oxifit-formula: no dependencies

[x] = optional, behind a feature. Only oxiroot and oxihep crates are listed.
```

## Third-party dependencies

All pure Rust — the no-libROOT promise holds end to end:

- [`ruzstd`](https://crates.io/crates/ruzstd) — Zstd
- [`miniz_oxide`](https://crates.io/crates/miniz_oxide) — zlib
- [`lz4_flex`](https://crates.io/crates/lz4_flex) — LZ4
- [`lzma-rust2`](https://crates.io/crates/lzma-rust2) — LZMA / XZ (encode + decode)
- [`xxhash-rust`](https://crates.io/crates/xxhash-rust) — RNTuple XXH3 + LZ4 XXH64
- [`minuit2`](https://crates.io/crates/minuit2) — Minuit2 MIGRAD (with the `fit` feature)
- [`argmin`](https://crates.io/crates/argmin) — Nelder–Mead backend (with the `argmin` feature)
- [`rayon`](https://crates.io/crates/rayon) — data-parallel fill / basket decode (with the `rayon` feature)
- [`memmap2`](https://crates.io/crates/memmap2) — memory-mapped reads (with the `mmap` feature)
- [`bytes`](https://crates.io/crates/bytes) — zero-copy byte ranges behind the reader
- [`ureq`](https://crates.io/crates/ureq) — HTTP(S) client (rustls) for remote range reads (with the `http` feature)
- [ReX](https://github.com/KenyC/ReX) — TeX math layout (with the `plot` feature); not on crates.io, so a trimmed copy is vendored as `oxiplot-tex` (MIT, see its `LICENSE-3rdparty`)
- [`ttf-parser`](https://crates.io/crates/ttf-parser), [`ab_glyph`](https://crates.io/crates/ab_glyph), [`tiny-skia`](https://crates.io/crates/tiny-skia) — font parsing, text outlines and PNG rasterization (with the `plot` feature)

## API reference

The complete, type-level documentation is the rustdoc, browsable here under
**[API reference](../api/oxiroot/index.html)**. Locally you can regenerate and
open it with:

```sh
cargo doc --no-deps --all-features --workspace --open
```
