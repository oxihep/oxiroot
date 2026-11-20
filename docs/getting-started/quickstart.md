# Quick start

This page writes and reads each of the four main object families — a histogram,
a multi-object file, a `TTree`, and an RNTuple — in a handful of lines. Every
file it produces opens in official ROOT and uproot.

Bring the common types into scope first:

```rust
use oxiroot::prelude::*;
```

## One histogram

A histogram is just data. Construct it with `Hist::reg(nbins, lo, hi).double().to_root()`, fill it,
and name it only when you persist it. The `WriteRoot` / `ReadRoot` traits give
every writable object a `write_root` and every readable object a `read_root`.

```rust
let mut h = Hist::reg(50, 0.0, 100.0).double().to_root().named("pt").titled("p_{T}");
h.track_sumw2();                                  // per-bin (weighted) errors
h.fill_weight(42.0, 1.5);
h.write_root("hist.root", Compression::Zstd(5))?;        // any single writable object

let same = Hist1D::read_root(&FileReader::open("hist.root")?, "pt")?;  // any readable object
```

## Several objects, subdirectories, appending

For more than one object, a `TDirectory`, or appending to an existing file, use
`FileWriter` — the single entry point for file composition.

```rust
let prof = Hist::reg(5, 0.0, 5.0).mean().to_root().named("prof").titled("<pt> per region");
FileWriter::create("out.root")
    .add(&h)                              // any &dyn WriteRoot: hist, profile, graph…
    .dir("by_region", |d| d.add(&prof))   // a TDirectory
    .write(Compression::Zstd(5))?;

let g = FileReader::open("out.root")?;
let p = Profile1D::read_root_in(&g, "by_region", "prof")?;   // read from a subdirectory
```

## A TTree or an RNTuple

Columns go in a `Batch`; the format is one argument when writing and detected
when reading (see [Datasets](../guide/datasets.md)).

```rust
let batch = Batch::new()
    .column("n", vec![1_i32, 2, 3])?
    .column("pt", vec![10.5_f64, 20.1, 33.7])?
    .column("jets", vec![vec![30.0_f32], vec![], vec![55.0, 41.2]])?;
let mut out = DatasetWriter::create("tree.root", "Events", Format::Tree)?; // or Format::Ntuple
out.write(&batch)?;
out.finish()?;

let f = FileReader::open("tree.root")?;
let events = Dataset::open(&f, "Events")?;
let back = events.read(["pt", "jets"])?;
let pt: &[f64] = back.values("pt")?;
let jets = back.jagged::<f32>("jets")?;
for batch in events.batches(["pt"]) {          // cluster by cluster, for big files
    let total: f64 = batch?.values::<f64>("pt")?.iter().sum();
}
```

## Run the worked example

The [`analysis` example](https://github.com/oxihep/oxiroot/blob/main/crates/oxiroot/examples/analysis.rs)
is an end-to-end mini analysis — weighted/variable-bin histograms →
scale/merge/normalize → per-region subdirectories → a columnar event dataset →
read-back:

```sh
cargo run -p oxiroot --example analysis
```

## Next

Dive into any area:

- **[Histograms](../guide/histograms.md)** — the full `Hist1D`/`Hist2D`/`Hist3D` family,
  fill semantics, arithmetic, statistics, and derived histograms.
- **[Graphs](../guide/graphs.md)** — `Graph` and its error variants.
- **[TTree](../guide/ttree.md)** and **[RNTuple](../guide/rntuple.md)** — the two
  event-data formats.
- **[Fitting](../guide/fitting.md)** — fit any 1-D data.
- **[Statistics](../guide/statistics.md)** — `scipy.stats`-verified special
  functions, distributions, and tests.
- **[Multithreaded fill](../guide/multithreading.md)**,
  **[Compression](../guide/compression.md)**, and
  **[interop](../guide/interop.md)**.
