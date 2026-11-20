# Datasets: TTree and RNTuple

ROOT stores event data in two columnar formats: the classic `TTree` and the
newer RNTuple. `oxiroot::data` reads and writes both through one API. A
[`Batch`](https://oxihep.github.io/oxiroot/api/oxiroot/data/struct.Batch.html)
of named, flat columns is what reading returns and writing takes; the format
is detected when reading and is one argument when writing.

```rust,no_run
use oxiroot::prelude::*;

// Write: build a batch, pick a format.
let batch = Batch::new()
    .column("mass", vec![91.2_f64, 125.1, 173.0])?
    .column("charge", vec![0_i32, -1, 1])?
    .column("label", vec!["Z", "H", "top"])?
    .column("jet_pt", vec![vec![30.0_f32, 25.0], vec![], vec![120.0]])?;
let mut out = DatasetWriter::create("events.root", "events", Format::Ntuple)?;
out.write(&batch)?;
out.finish()?;

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
    out.write(&batch.filter(&keep)?)?;
}
out.finish()?;
# Ok::<(), oxiroot::Error>(())
```

## Reading

`Dataset::open(&file, name)` (or `open_in` for a subdirectory) opens a
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
fetched together (see [Reading and writing](reading-writing.md)).

RNTuple records read as `Column::Struct`; variants have no batch column, so
read them with `as_ntuple()`. `as_tree()` likewise reaches the `TTree` reader underneath for
friends, indices and split objects.

## Writing

`DatasetWriter::create(path, name, format)` writes to a file; `new(sink, …)`
writes to any `Write + Seek`.

- `.compression(c)` sets the compression (Zstd level 5 by default);
- `.cluster_bytes(n)` sets the cluster size (32 MiB of values by default);
- `.large(true)` selects ROOT's 64-bit layout, for files past 2 GiB.

`write(&batch)` buffers entries and writes a cluster whenever enough have
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

To put a dataset next to histograms or other datasets in one file, give a
`DatasetObject` to a `FileWriter`:

```rust,no_run
use oxiroot::prelude::*;

let events = Batch::new().column("energy", vec![10.5_f64, 20.1])?;
let runs = Batch::new().column("run", vec![101_i32])?;
FileWriter::create("run.root")
    .put(DatasetObject::new("events", Format::Tree, &events)?)
    .put(DatasetObject::new("runs", Format::Ntuple, &runs)?)
    .write(Compression::Zstd(5))?;
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

- [TTree](ttree.md): C-style `x[n]` arrays, leaflists, split single objects,
  friends and indices, `ChainReader`.
- [RNTuple](rntuple.md): records, variants, maps and sets, reduced-precision
  floats, schema extension.
