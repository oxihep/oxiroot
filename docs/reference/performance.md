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
| `pt`, `Dataset::read` | 0.99 ms (1.0 G entries/s) | 1.1 ms (900 M entries/s) |
| `jets`, `Dataset::read` (flat) | 59 ms | 60 ms |
| `jets`, low-level `values()` (one `Vec` per entry) | 65 ms | 72 ms |
| all three, `Dataset::batches` | 62 ms | 65 ms |
| all three, `Dataset::batches`, parallel | 18 ms | 13 ms |

| Write (with `rayon`) | `TTree` | RNTuple |
| --- | ---: | ---: |
| `DatasetWriter`, one `Batch` | 42 ms | 27 ms |
| `TreeWriter` / `NtupleWriter`, from rows | 166 ms | 104 ms |

The `pt` column barely compresses, so its read is close to a memory copy;
`jets` holds compressible values, and decompressing them is most of its
time (see below). Reading 500 `TH1D` by name from one file takes 2.8 ms
(about 180 k objects/s).

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
| oxiroot `TTree` | `pt` | 17 ms | 21 ms | 19 ms | 23 ms |
| oxiroot `TTree` | all three | 552 ms | 409 ms | 150 ms | 103 ms |
| oxiroot RNTuple | `pt` | 20 ms | 14 ms | 19 ms | 14 ms |
| oxiroot RNTuple | all three | 680 ms | 239 ms | 115 ms | 235 ms |
| ROOT `TTree` | `pt` | 309 ms | 77 ms | 47 ms | 33 ms |
| ROOT `TTree` | all three | 991 ms | 559 ms | 199 ms | 353 ms |
| ROOT RNTuple | `pt` | 71 ms | 44 ms | 38 ms | 41 ms |
| ROOT RNTuple | all three | 452 ms | 198 ms | 187 ms | 194 ms |

"Parallel" is `batches(..).parallel(true)` for oxiroot and a
`ThreadPoolExecutor` for uproot's decompression and interpretation.

On one thread, uproot is faster: 1.5–4× on data that compresses (the `jets`
values, everything ROOT writes). Profiles put 85 % of oxiroot's time in
decompression. uproot decompresses with the C libzstd; oxiroot with `ruzstd`,
the pure-Rust decoder, which is about 4× slower on compressed blocks. The
`pt` column oxiroot writes barely compresses (its encoder has only the fast
level), so it reads at memory speed on both sides.

With C libzstd swapped in as an experiment, oxiroot's single-thread reads match
or beat uproot's on every row: 334 ms against 559 ms for all three columns of
the ROOT `TTree`, 204 ms against 198 ms for the ROOT RNTuple. In parallel,
oxiroot is already faster in five of the eight rows, up to 2× on the heavy
RNTuple and ROOT `TTree` reads: it decompresses a column's baskets or pages
across threads, where uproot's executor helps a `TTree` but barely an RNTuple.

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
