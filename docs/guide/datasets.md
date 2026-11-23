# Datasets: TTree and RNTuple

ROOT stores event data in two columnar formats: the classic `TTree` and the
newer RNTuple. `oxiroot::data` reads and writes both through one API. A
[`Batch`](https://oxihep.github.io/oxiroot/api/oxiroot/data/struct.Batch.html)
of named, flat columns is what reading returns and writing takes; the format
is detected when reading and is one argument when writing.

```rust,no_run
use oxiroot::prelude::*;

// Write: build a batch, pick a format, add it to a file.
let batch = Batch::new()
    .column("mass", vec![91.2_f64, 125.1, 173.0])?
    .column("charge", vec![0_i32, -1, 1])?
    .column("label", vec!["Z", "H", "top"])?
    .column("jet_pt", vec![vec![30.0_f32, 25.0], vec![], vec![120.0]])?;
FileWriter::create("events.root")
    .add(NamedBatch::new("events", Format::Ntuple, batch))
    .write()?;

// Read: the format is detected.
let file = FileReader::open("events.root")?;
let events = Dataset::open(&file, "events")?;
let back = events.read(["mass", "jet_pt"])?;
let mass: &[f64] = back.values("mass")?;          // a slice, no copy
let jets = back.jagged::<f32>("jet_pt")?;          // one slice per entry
assert_eq!(jets.row(0), [30.0, 25.0]);
# Ok::<(), oxiroot::Error>(())
```

Swap `Format::Ntuple` for `Format::Tree` and nothing else changes.

## Batches

A `Batch` holds columns of equal length. Each column is stored flat, the way
Arrow stores it: scalars in one `Vec`, strings as offsets into one byte
buffer, variable-length lists as offsets into one child column. Reading a
`std::vector<float>` column allocates two buffers, not one `Vec` per entry.

Build a batch with `column(name, data)`, where `data` is a `Vec` of one of:

| Rust | `TTree` | RNTuple |
|---|---|---|
| `bool`, `i8` … `u64`, `f32`, `f64` | scalar leaf | scalar field |
| `String`, `&str` | `TLeafC` string | `std::string` |
| `Vec<T>` | `std::vector<T>` (reads `x[n]` too) | `std::vector<T>` |
| `[T; N]` | fixed array `x[N]` | `std::array<T, N>` |
| `Vec<Vec<T>>`, `Vec<String>`, `Vec<bool>` | `std::vector<…>` | `std::vector<…>` |
| `Option<T>` | — | `std::optional<T>` |
| struct (`Column::Struct`) | in a list only: split `std::vector<Struct>` | record / user class |

Or give a list flat with `list(name, offsets, values)`, or any shape as a
`Column`, structs included: `Column::Struct(vec![("x", Column::F32(…)), …])`,
and a `std::vector<Struct>` is a `Column::List` of one. A struct column's
members are reached with `Column::member(name)`. In an RNTuple, a struct whose
members are `_0`, `_1`, … is a `std::pair` / `std::tuple`; one with named
members is a user class named after the column (`hits`). ROOT reads it with a
dictionary of that class, or without one through type emulation
(`RCreateModelOptions::SetEmulateUnknownTypes`); uproot reads it as is.

A `TTree` holds structs in a list, as a split `std::vector<hits>` whose
members are numbers: one sub-branch per member (`hits.x`, `hits.id`), the way
ROOT splits it. Reading gives back the list of structs, from a split
collection ROOT wrote too; a member also reads on its own
(`read(["hits.x"])`).

A batch checks every
column when it is added: same entry count, offsets that partition their
values, UTF-8 strings.

Read a column back typed:

| Method | Gives | For |
|---|---|---|
| `values::<T>(name)` | `&[T]` | scalars |
| `jagged::<T>(name)` | `Jagged<T>`: `row(i)`, `rows()`, `values()`, `offsets()` | lists |
| `arrays::<T>(name)` | `(&[T], N)` | fixed arrays |
| `strings(name)` | `Strings`: `get(i)`, `iter()` | strings |
| `get(name)` | the `Column` enum | anything, by matching |

Reads are strict: `values::<f64>` on a `float` column is an
`Error::TypeMismatch` naming the column, what it holds and what was asked
for. Nothing is converted silently.

`filter(&mask)`, `slice(range)`, `select(names)` and `append(other)` work on
whole batches, every column at once. A skim needs no per-type code:

```rust,no_run
use oxiroot::prelude::*;

let file = FileReader::open("events.root")?;
let events = Dataset::open(&file, "events")?;
let mut out = DatasetWriter::create("skim.root", "events", Format::Tree)?;
for batch in events.batches(["mass", "jet_pt"]) {
    let batch = batch?;
    let keep: Vec<bool> = batch.values::<f64>("mass")?.iter().map(|&m| m > 100.0).collect();
    out.append(batch.filter(&keep)?)?;
}
out.finish()?;
# Ok::<(), oxiroot::Error>(())
```

## Reading

`Dataset::open(&file, path)` (`"events"`, or `"dir/events"` in a subdirectory) opens a
`TTree` or an RNTuple. `entries()`, `columns()` and `format()` describe it.

- `read(columns)` reads every entry of the columns into one batch.
- `batches(columns)` iterates in order. By default each batch is one cluster:
  an RNTuple cluster, or the entries where every selected branch of a tree
  starts a basket. So no basket or page is decompressed twice, and a loop
  holds one cluster at a time. `.step(n)` aims for about `n` entries per batch,
  still cut at a cluster boundary unless that is more than twice as far.
  `.range(a..b)` reads only those entries. With the `rayon` feature,
  `.parallel(true)` reads a batch's columns concurrently.

Remote files work the same: over `root://` or HTTP a batch's baskets are
fetched together (see [Reading and writing](reading-writing.md)). Two examples
run a whole analysis this way on real collision data from CERN's open-data
server: [`open_data_dimuon`](https://github.com/oxihep/oxiroot/blob/main/crates/oxiroot/examples/open_data_dimuon.rs)
streams CMS events and writes a skim with its histograms beside it, and
[`open_data_zmumu`](https://github.com/oxihep/oxiroot/blob/main/crates/oxiroot/examples/open_data_zmumu.rs)
compares ATLAS data with simulation and writes the result to one file.

RNTuple records read as `Column::Struct` and a `std::variant` as
`Column::Union`. `as_ntuple()` reaches the `NtupleReader` underneath, for the
schema and schema extension, and `as_tree()` the `TreeReader`, for friends,
indices and split objects.

## Queries: derived columns and cuts

`dataset.query()` computes new columns and keeps the entries that pass cuts,
reading only the columns it uses, batch by batch. It is checked against the
dataset's columns **before anything is read**: a missing column, a type that
does not fit, or a cut that is not one bool per entry is an error up front,
with the expression underlined and a hint.

```rust
use oxiroot::prelude::*;
use oxiroot::data::query::col;

let f = FileReader::open("nano.root")?;
let events = Dataset::open(&f, "Events")?;
let out = events
    .query()
    .select(["run", "event"])                          // kept as they are
    .filter(col("nMuon").ge(2))                        // built in Rust…
    .filter("any(Muon.pt > 20 and abs(Muon.eta) < 2.4)") // …or written as text
    .define("lead_pt", col("Muon_pt").max())
    .define("lead_eta", "Muon_eta[0] ?? -99")
    .define("pt", (col("px").pow(2) + col("py").pow(2)).sqrt())
    .read()?;                                          // or .batches() to stream
# Ok::<(), oxiroot::Error>(())
```

An expression is built in Rust from `col`, `lit` and `when` with the `Expr`
methods (`gt`, `abs`, `any`, `count`, `at`, `keep`, `unwrap_or`, …), or
written as text. The two are the same thing: an expression prints as its text,
and `Expr::parse` reads it back. The text form has columns (`pt`, `Muon.pt`,
`` `odd name` ``), numbers, `true`/`false`, `+ - * / ^`, comparisons (which do
not chain), `and or not` (or `&& || !`), `x ?? default`, `if c then a else b`,
the usual functions (`sqrt`, `abs`, `exp`, `log`, `sin`, …, `atan2`, `hypot`,
`min`, `max`), reductions (`any`, `all`, `count`, `sum`, `max`, `min`,
`mean`, `len`), `x[i]` and `x[mask]`.

**Per entry and per object.** `nMuon` has one value per entry, `Muon_pt` one
per muon. An operation between them applies to each muon. `any`, `all`,
`count`, `sum`, `max`, `min`, `mean`, `len` and `x[i]` bring the muons back to
one value per entry, and `x[mask]` keeps the muons a per-muon bool selects. A
cut has to say which it means:

```text
a filter needs one bool per entry, but this is one bool per object of `Muon_pt`
  nMuon >= 2 and Muon_pt > 20
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^
  help: say which: any(…) or all(…)
```

**Missing values.** `Muon_pt[0]` for an event without muons, `max` of no
objects, or a value of an optional column, is missing. It stays missing
through arithmetic, a cut drops an entry where its condition is missing (but
`false and missing` is false, and `true or missing` true), and `?? default`
fills it. A defined column that can be missing is an optional column.

**Records.** `Muon.pt` is the member `pt` of a record column `Muon`, or, where
there is none, the NanoAOD column `Muon_pt`.

**Types.** Integers stay integers (an event number past 2⁵³ compares
exactly), `/` and `^` give floats, and a bool is not a number: `if c then 1
else 0` makes one.

For what an expression cannot say, `define_with` and `filter_with` take a
Rust closure. Its argument types are checked against the columns before
reading: a number or `bool` per entry, an `Option` for a column with missing
values, a `Vec` for one value per object.

```rust
# use oxiroot::prelude::*;
# let f = FileReader::open("nano.root")?;
# let events = Dataset::open(&f, "Events")?;
let out = events
    .query()
    .define_with("mt", ["pt", "met", "dphi"], |pt: f32, met: f32, dphi: f32| {
        (2.0 * pt * met * (1.0 - dphi.cos())).sqrt()
    })
    .filter_with(["Jet_pt"], |pts: Vec<f32>| pts.iter().any(|&p| p > 30.0))
    .read()?;
# Ok::<(), oxiroot::Error>(())
```

A closure runs entry by entry; an expression runs a whole column at a time,
and is the faster of the two. On the same file, the expression query above
gives the same selection and the same values as ROOT's `RDataFrame` with the
equivalent `Filter`/`Define` (a `float` column's sum differs in its last
digits, as `RDataFrame` sums floats in single precision).

## Writing

A `NamedBatch` is a dataset held in memory: batches under a name and a format. It
goes in a `FileWriter` with `add`, like a histogram or a graph, so any mix of
objects is built first and written at the end:

```rust,no_run
use oxiroot::prelude::*;

let mut events = NamedBatch::new(
    "events",
    Format::Tree,
    Batch::new().column("energy", vec![10.5_f64, 20.1])?,
);
events.append(Batch::new().column("energy", vec![7.25_f64])?)?; // more entries
let runs = NamedBatch::new("runs", Format::Ntuple, Batch::new().column("run", vec![101_i32])?);

let mut spectrum = Hist::reg(50, 0.0, 100.0).double().named("spectrum");
spectrum.fill(10.5);

FileWriter::create("run.root")
    .add(&spectrum.to_root()?)
    .add(&events)
    .add(&runs)
    .write()?;
# Ok::<(), oxiroot::Error>(())
```

The first batch fixes the columns; `append` takes more entries of the same
columns. `.with_schema(…)` chooses the stored types, as below. `.add(&events)`
copies the values into the file being built, and `.add(events)` hands them
over. The entries are cut into chunks of about 1 MiB per column, as a
`DatasetWriter` cuts a cluster (below), so a read of a window or in parallel
need not decompress everything.

### A dataset too large to hold

`DatasetWriter` streams batches to the file as they come, so only one
cluster is in memory. `DatasetWriter::create(path, name, format)` writes to a
file; `new(sink, …)` writes to any `Write + Seek`. `add` stores a histogram,
a graph or a parameter in the same file:

```rust,no_run
use oxiroot::prelude::*;

let mut spectrum = Hist::reg(50, 0.0, 100.0).double().named("spectrum");
let mut out = DatasetWriter::create("big.root", "events", Format::Tree)?;
for chunk in 0..100 {
    let energy: Vec<f64> = (0..10_000).map(|i| f64::from(chunk * 10_000 + i) * 1e-4).collect();
    energy.iter().for_each(|&e| spectrum.fill(e));
    out.append(Batch::new().column("energy", energy)?)?;
}
out.add(&spectrum.to_root()?); // written with the file, next to the tree
out.finish()?;
# Ok::<(), oxiroot::Error>(())
```

- `.compression(c)` sets the compression (Zstd level 5 by default);
- `.cluster_bytes(n)` sets the cluster size (32 MiB of values by default);
- `.large()` selects ROOT's 64-bit layout, for files past 2 GiB.

`append(batch)` takes the batch and buffers its entries and writes a cluster whenever enough have
accumulated, however you size the batches. `finish()` writes the rest and
closes the file. Each cluster is cut into chunks of about 1 MiB per column (a
`TTree` basket per branch, or an RNTuple cluster, per chunk), so later reads of
a window decompress little and parallel reads have work to share.

With the `rayon` feature, every chunk is converted and compressed in
parallel. In the `write` benchmark (1M entries, two scalars and a
`std::vector<double>`), writes are 5–7× faster than serial; reads with
`.parallel(true)` are 2–4× faster.

The first batch fixes the schema; every later batch must have the same
columns and types. A writer given only empty batches writes a dataset with
their columns and no entries, fixed arrays keeping their width. A column the format cannot hold, such as `Option<T>` in a
`TTree`, is refused on that first write, before anything reaches the file.

## Stored types: the schema

A column's shape does not say everything about how it is stored. A list can be
a `std::vector`, a `std::set`, or a C-style `x[n]`; an `f32` can be kept in
half precision. A `Schema` holds one `DataType` per column and says which.

`Dataset::schema()` reports what a file holds. A writer stores each column as
its own shape (`Vec<T>` as `std::vector<T>`, `Option<T>` as `std::optional<T>`)
unless `.with_schema(…)` says otherwise:

```rust,no_run
use oxiroot::prelude::*;
use oxiroot::data::{DataType, Schema};

let batch = Batch::new()
    .column("pt", vec![41.5_f32, 37.2])?
    .column("hits", vec![vec![3_i32, 7], vec![]])?;
let schema = batch
    .schema()                                                   // the defaults
    .with("pt", DataType::Half)                                 // 16 bits per value
    .with("hits", DataType::Set(Box::new(DataType::I32)));      // a std::set<int>
FileWriter::create("out.root")
    .add(NamedBatch::new("events", Format::Ntuple, batch).with_schema(schema))
    .write()?;
# Ok::<(), oxiroot::Error>(())
```

| `DataType` | Held in a batch as | Format |
|---|---|---|
| `Half`, `Truncated { bits }`, `Quantized { bits, min, max }` | `f32` | RNTuple |
| `CArray(T)`, a C-style `x[n]` | list | `TTree` |
| `Set(T)` | list | RNTuple |
| `Map(K, V)` | list of `{_0: K, _1: V}` structs | RNTuple |
| `Bitset(N)` | array of `N` `bool` | RNTuple |
| `UniquePtr(T)` | optional | RNTuple |
| `Atomic(T)` | the bare `T` | RNTuple |

A skim that keeps every stored type passes the source's schema on:
`.with_schema(events.schema()?)`, on a `NamedBatch` or a `DatasetWriter`. A type
the format does not have (half precision in a `TTree`) or a batch that does
not fit the schema is refused when it is written. A tree writes a counted array `x` together with
the branch `nx` that counts it; a column of that name read back from a tree is
recognised as that branch and not written twice.

## Several files as one: chains

`Dataset::chain` reads datasets with the same columns one after the other, the
way ROOT's `TChain` spans a dataset split over many files. The parts can be
trees, RNTuples, or both:

```rust,no_run
use oxiroot::prelude::*;

let (a, b) = (FileReader::open("part1.root")?, FileReader::open("part2.root")?);
let events = Dataset::chain([Dataset::open(&a, "events")?, Dataset::open(&b, "events")?])?;
for batch in events.batches(["pt"]) {
    let batch = batch?; // one cluster at a time, across both files
}
# Ok::<(), oxiroot::Error>(())
```

## Arrow and Polars

With the `arrow` feature, a batch converts to an Arrow `RecordBatch` and back
(arrow-rs); with the `polars` feature, to a Polars `DataFrame` and back:

```rust,no_run
# use oxiroot::prelude::*;
# let file = FileReader::open("events.root")?;
# let events = Dataset::open(&file, "events")?;
let batch = events.read(["mass", "jet_pt"])?;
let record_batch = batch.to_arrow()?;   // or into_arrow(), which moves the buffers
let frame = batch.into_polars()?;       // a DataFrame: mass f64, jet_pt list[f32]
let back = Batch::from_polars(&frame)?;
# Ok::<(), oxiroot::Error>(())
```

Every column shape converts: lists to `LargeList` / `List`, fixed arrays to
`FixedSizeList` / `Array`, structs to `Struct`, and an `Optional` to its
value's type with nulls; coming back, a column with nulls becomes an
`Optional`. Arrow and Polars have one level of nulls, so an `Optional` of an
`Optional` folds into it: `Some(None)` and `None` both become null. `oxiroot::data::arrow_rs` and `polars_rs` re-export the crates at
the matching versions.

## Format-specific features

The low-level readers and writers cover what only one format has:

- [TTree](ttree.md): leaflists, split single objects, friends and indices.
- [RNTuple](rntuple.md): the field layer and schema extension.
