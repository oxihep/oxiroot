# Performance

oxiroot ships [criterion](https://crates.io/crates/criterion) benchmarks for
the three hot paths — filling histograms, reading data back, and
(de)compressing ROOT blocks — in `crates/oxiroot/benches`:

```sh
cargo bench -p oxiroot --bench fill --bench read --bench decompress
# one group, shorter: cargo bench -p oxiroot --bench fill -- 'fill/oxihist'
```

CI builds them on every pull request and runs them weekly, keeping criterion's
report as a build artifact. Shared CI runners are too noisy to gate on the
numbers, so they inform rather than fail a build.

The figures below are medians from one run on an Apple Silicon laptop
(`--warm-up-time 1 --measurement-time 3`). Treat them as orders of magnitude:
absolute numbers depend on the machine, and differences of ~10% between runs
are noise.

## Filling histograms

One million values per fill benchmark.

| Benchmark | Throughput |
| --- | ---: |
| `Hist1D`, 100 regular bins | 756 M fills/s |
| `Hist1D`, 100 variable bins | 248 M fills/s |
| `Hist2D`, 100 × 100 | 276 M fills/s |
| `Profile1D`, 100 bins | 275 M fills/s |
| oxihist, 100 regular bins, `Double` storage | 770 M fills/s |
| oxihist, 100 regular bins, `Weight` storage | 607 M fills/s |
| oxihist, string category axis, 20 categories | 128 M fills/s |
| `ThreadedHist<Hist1D>`, 1 thread | 233 M fills/s |
| `ThreadedHist<Hist1D>`, 4 threads | 735 M fills/s |

The ROOT histograms (`Hist1D`, …) also keep ROOT's entry count and moment sums
on every fill; oxihist histograms (what [`Hist`](../guide/histograms.md)
builds before `.to_root()`) keep only the bins, and fill a number on a regular
or variable axis, an integer, a known category or a boolean without going
through their general (growing, converting) code path.

`ThreadedHist` gives every thread its own copy, found without a shared lock,
so fills scale with the threads until the merge: 4 threads fill about 3× faster
than one here. See [Multithreaded fill](../guide/multithreading.md).

## Reading

Files written once with Zstd, then read back from a fresh
`FileReader` each iteration, so the numbers include opening the file and
decompressing every basket or page.

| Benchmark | Throughput |
| --- | ---: |
| `TTree`, `f64` branch, 1 M entries | 700 M entries/s |
| `TTree`, `i32` branch, 1 M entries | 786 M entries/s |
| `TTree`, jagged `f64` branch, 100 k entries | 19 M entries/s |
| RNTuple, `f64` + `i32` fields, 1 M entries | 304 M entries/s |
| 500 `TH1D` read by name from one file | 2.8 ms (≈ 180 k objects/s) |

## Compression

4 MiB of big-endian doubles from a smooth series with noise in the low bits —
compressible the way physics columns are — through ROOT's block framing.

| Codec | Decompress | Compress |
| --- | ---: | ---: |
| LZ4 | 13.9 GiB/s | 6.8 GiB/s |
| Zstd | 6.8 GiB/s | 99 MiB/s |
| zlib (level 6) | 454 MiB/s | 55 MiB/s |
| LZMA (level 6) | 40 MiB/s | 10 MiB/s |

The pure-Rust Zstd and LZ4 encoders run in their fast mode whatever the level
(the level tunes zlib and LZMA). Zstd is ROOT's modern default and a good
balance; LZ4 is the fastest to read back; LZMA gives the smallest files at a
large CPU cost. See
[Compression](../guide/compression.md) for the settings.
