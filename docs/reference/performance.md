# Performance

oxiroot ships [criterion](https://crates.io/crates/criterion) benchmarks for
the hot paths — filling histograms, reading and writing datasets, and
(de)compressing ROOT blocks — in `crates/oxiroot/benches`:

```sh
cargo bench -p oxiroot --features rayon --bench fill --bench read --bench write --bench decompress
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

## Reading and writing

One million entries, written once with Zstd into a temporary directory: `pt`
(`double`), `n` (`int`) and `jets` (`std::vector<double>`, 0 to 4 values).
Read benchmarks open the dataset afresh each iteration and decompress every
basket or page they need; `parallel` uses rayon's pool (the `rayon` feature).

| Read | `TTree` | RNTuple |
| --- | ---: | ---: |
| `pt`, `Dataset::read` | 7.0 ms | 7.1 ms |
| `jets`, `Dataset::read` (flat) | 29 ms | 18 ms |
| `jets`, low-level `values()` (one `Vec` per entry) | 37 ms | 30 ms |
| all three, `Dataset::batches` | 39 ms | 28 ms |
| all three, `Dataset::batches`, parallel | 13 ms | 7.9 ms |

| Write (with `rayon`) | `TTree` | RNTuple |
| --- | ---: | ---: |
| `DatasetWriter`, one `Batch` | 14 ms | 4.9 ms |
| `TreeWriter` / `NtupleWriter`, from rows | 47 ms | 29 ms |

Most of a read is decompressing. Zstd goes through
[zrip](https://crates.io/crates/zrip), a pure, safe-Rust implementation within
about 1.25× of C zstd's speed and with its compression ratio. Reading 500
`TH1D` by name from one file takes 1.4 ms (about 350 k objects/s).

## Against uproot

`scripts/bench_uproot.py` reads the same files with oxiroot's `Dataset` and
with uproot 5.7: 10 million entries of `pt` (`double`), `n` (`int`) and
`jets` (`std::vector<double>`, 0 to 4 values), Zstd level 5. They are written
once by oxiroot and once by ROOT 6.40's RDataFrame (when `root` is on
`PATH`). Each read opens the file and reads every entry of the columns into
memory: a `Batch` on one side, NumPy and Awkward arrays on the other. Both
read from the page cache, after one untimed read. Medians of 5; 12 threads on
an Apple M4 Pro.

```sh
python3 scripts/bench_uproot.py            # --entries, --repeats
```

| File | Columns | oxiroot | uproot | oxiroot, parallel | uproot, parallel |
| --- | --- | ---: | ---: | ---: | ---: |
| oxiroot `TTree` (122 MiB) | `pt` | 74 ms | 78 ms | 22 ms | 31 ms |
| oxiroot `TTree` | all three | 390 ms | 492 ms | 142 ms | 107 ms |
| oxiroot RNTuple (100 MiB) | `pt` | 76 ms | 76 ms | 23 ms | 74 ms |
| oxiroot RNTuple | all three | 326 ms | 283 ms | 78 ms | 277 ms |
| ROOT `TTree` (121 MiB) | `pt` | 73 ms | 79 ms | 24 ms | 34 ms |
| ROOT `TTree` | all three | 408 ms | 566 ms | 136 ms | 360 ms |
| ROOT RNTuple (52 MiB) | `pt` | 44 ms | 45 ms | 35 ms | 42 ms |
| ROOT RNTuple | all three | 182 ms | 198 ms | 108 ms | 199 ms |

"Parallel" is `batches(..).parallel(true)` for oxiroot and a
`ThreadPoolExecutor` for uproot's decompression and interpretation.

On one thread, oxiroot is as fast as uproot or faster in seven of the eight
cases (up to 1.4×, on the ROOT `TTree`), and 0.9× in the last. In parallel it
is faster in seven of eight, up to 3.6×: it decompresses a column's baskets or
pages across threads, where uproot's executor helps a `TTree` but barely an
RNTuple.

Up to oxiroot 0.8, Zstd went through `ruzstd`, which decodes about 4× slower
than C zstd: uproot (C libzstd) was then 1.5–4× faster on one thread, and the
files oxiroot wrote were about a third larger, since `ruzstd` has a single
fast level. Of seven Rust Zstd crates timed on these files' blocks, zrip came
within 1.25× of C zstd both ways, with its compression ratio and no `unsafe`
code, so it took `ruzstd`'s place.

## Compression

4 MiB of big-endian doubles from a smooth series with noise in the low bits —
compressible the way physics columns are — through ROOT's block framing.

| Codec | Decompress | Compress |
| --- | ---: | ---: |
| LZ4 | 13.9 GiB/s | 6.8 GiB/s |
| Zstd | 1.2 GB/s | 1.4 GB/s |
| zlib (level 6) | 454 MiB/s | 55 MiB/s |
| LZMA (level 6) | 40 MiB/s | 10 MiB/s |

These doubles barely compress (about 5 % with Zstd), so most of a Zstd
block is entropy-coded literals: the slow case. The LZ4 encoder runs in its fast
mode whatever the level; Zstd levels above 4 use zrip's level 4. Zstd is ROOT's modern default and a good
balance; LZ4 is the fastest to read back; LZMA gives the smallest files at a
large CPU cost. See
[Compression](../guide/compression.md) for the settings.
