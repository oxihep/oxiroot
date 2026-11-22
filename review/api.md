# API review: one way to do each thing

This file looks only at the public API. For each task a user has, it lists the
ways the workspace offers today, picks one, and says what to remove or rename.
The goal is a smaller surface: fewer names to learn, fewer code paths to test,
and documentation that never has to say "or alternatively".

Back to the [summary](README.md). Implementation findings are in
[findings.md](findings.md).

## Rules

These nine rules decide every choice below.

1. **One entry point per task.** A shorthand that saves two lines is a second
   way. Remove it.
2. **One name per concept.** `add`, `put`, `insert` and `push` for "put this in
   that" is four names.
3. **Addresses are paths.** An object is named by one string,
   `"dir/sub/name;cycle"`. No `_in(dir, name)` twins.
4. **Fallible means `Result`.** No panicking twin (`to_root` / `try_to_root`),
   and no silent sentinel (`0.0`, `""`, a no-op) for bad input.
5. **Options are builder methods with defaults.** Not positional arguments in
   one place, constructor variants in another and a `bool` in a third.
6. **Conversions use the standard traits and prefixes.** `From`/`TryFrom`;
   `as_` borrows, `to_` allocates, `into_` consumes; `with_` is a builder on
   `self`, never a constructor; `get` returns `Option`.
7. **One import path per item.** Sub-crates do not re-export each other.
8. **One error type** at the facade.
9. **State lives in one place.** A value is a private field with an accessor,
   or a public field. Never both, and never also a builder that sets it.

## The table

| # | Task | Ways today | The one way |
|---|---|---|---|
| 1 | Open a file | `open`, `open_ranged`, `open_mmap`, `open_url`, `from_bytes`, `from_source` | `open` (lazy), `open_url`, `from_bytes`, `from_source` |
| 2 | Address an object | name, plus `_in(dir, name)` twins on six types | one path string |
| 3 | Read an object | `T::read_root`, `T::read_root_in`, `FromRoot::read_root`, `file.get_value`, `get_value_in`, `read_object`, `object_in`, `object_in_keyed` | `file.get::<T>(path)` |
| 4 | Write objects | `obj.write_root(path, c)`, `ToRoot::write_root`, `FileWriter::create().add().write(c)`, `.put()`, `.to_bytes(c)`, `.write_threshold(c, n)` | `FileWriter::create(path).add(&obj).write()` |
| 5 | Update a file | `FileWriter::open(path)` | `FileWriter::update(path)` |
| 6 | Read columns | `TreeReader::branch().read/values/flat`, `NtupleReader::field().read/values/flat`, `read_column`, `ChainReader`, `Dataset::read/batches` | `Dataset` and `Batch` |
| 7 | Write columns | `TreeWriter` + `Branch::*`, `NtupleWriter` + `Field::*`, `raw::write_tree`, `raw::write_ntuple`, `DatasetWriter` + `Batch`, `DatasetObject` | `NamedBatch` (in memory, added to a `FileWriter`) and `DatasetWriter` (streamed), both taking `Batch` |
| 8 | Merge files | `merge_files`, `Merger`, `merge_histogram_files`, `append_trees`, `append_ntuples`, `to_branch`, `to_field` | `Merger` |
| 9 | Hold a histogram | `oxihist::Histogram<S, D>` and `Hist1D`/`Hist2D`/`Hist3D`/`Profile*` | a decision: see [below](#decisions-that-are-yours) |
| 10 | Convert a histogram | `to_root`, `try_to_root`, `to_root_as`, `try_to_root_as`, `write_root`, `to_hist`, `FromRoot` | `to_root() -> Result`, `to_hist() -> Result` |
| 11 | Name an object | public `name` field, `name()` getter, `named()` builder | private field, `name()`, `named()` |
| 12 | Read bin contents | `contents()`, `h[cell]`, `for c in &h`, `values()`, `RootHistogram::contents()` | `contents()` and `values()` |
| 13 | Bin geometry | `h.find_bin(x)`, `h.bin_center(i)`, … and the same on `h.xaxis()` | on the axis only |
| 14 | Scale a histogram | `scale(c)`, `h *= c`, `h * c` | `scale(c)` |
| 15 | Build a graph | `Graph::new`, `Graph::with_errors`, `Graph::with_asymm_errors` | `Graph::new(x, y)?.errors(GraphErrors::…)?` |
| 16 | Set function parameters | `with_params`, `set_params`, `set_param` | `with_params` and `params_mut()` |
| 17 | Choose compression | argument to `write(c)`; argument to `TreeWriter::create(.., c)`; `.compression(c)` builder | `.compression(c)` everywhere, default Zstd 5 |
| 18 | Choose the 64-bit form | `create_large`, `new_large`, `.large(bool)`, `write_threshold`, `big: bool` | automatic; `.large()` only on streaming writers |
| 19 | Handle errors | `oxiroot::Error` (enum of sub-errors), `oxiroot_io_core::Error` (with `Other`) | one `oxiroot::Error` |
| 20 | Import a type | up to five paths (`FileWriter`), four for `Compression` | one |
| 21 | Describe a tree | eight `branch_*(name)` methods | `tree.branches()` yielding `BranchInfo` |
| 22 | Build a list | `ObjList::list()`, `ObjList::array()`; `ObjMap::insert`, `ObjMap::add` | `ObjList::new(kind)`; `ObjMap::insert` |
| 23 | Compression settings | `compression_settings`, `split_settings`, `Compression::setting`, `Compression::from_setting`, `compress(src, u32)` | `Compression` only |
| 24 | Streamer info | `streamer_registry`, `streamer_info_object`, `streamer_info_list` | `streamer_registry` (the others under `raw`) |

The sections below give the reasoning and a before/after for each.

---

## 1. Opening a file

**Today.** `FileReader::open` reads the whole file into memory.
`open_ranged` reads lazily. `open_mmap` is `unsafe`. `open_url` reads remotely.

**Problem.** A new user picks `open`, which is the one that does not scale.
`hadd` already uses `open_ranged` internally because of this.

**One way.**

```rust
FileReader::open("events.root")?        // positioned reads; only what is touched
FileReader::open_url("root://…")?       // remote
FileReader::from_bytes(bytes)?          // already in memory
FileReader::from_source(Box::new(src))? // anything else, including an mmap
```

- Make `open` do what `open_ranged` does, and delete `open_ranged`.
- Move the `unsafe` to `MmapSource::open(path)` and pass it to `from_source`.
  The reader then has no `unsafe` method.
- Take `impl Into<Bytes>` in `from_bytes`, which `BytesSource::new` already does.

Check first: run the `read` bench on small files. If reading whole is clearly
faster for them, keep it as an internal choice by file size, not as a second
public method.

## 2. Addressing an object

**Today.** Six public pairs: `ReadRoot::read_root` / `read_root_in`,
`TreeReader::open` / `open_in`, `NtupleReader::open` / `open_in`,
`Dataset::open` / `open_in`, `FileReader::get_value` / `get_value_in`,
`object_in` / `object_in_keyed`. Behind them sit about thirty private
`read_x` / `read_x_in` helper pairs.

**Problem.** Every reader is written twice. The CLI already takes
`file.root:dir/name` and splits it itself (`oxiroot-cli/src/util.rs:76`).

**One way.** The name argument is a path. `find_key` already understands
`name;cycle`; extend it to walk `/`.

```rust
// before
let h = Hist1D::read_root_in(&file, "regions/signal", "pt")?;
let t = TreeReader::open_in(&file, "ntuples", "events")?;

// after
let h: Hist1D = file.get("regions/signal/pt")?;
let d = Dataset::open(&file, "ntuples/events")?;
```

One private `FileReader::locate(path) -> Result<Located>` returns the class,
the decompressed bytes and the key length. Every `_in` method and helper goes.

## 3. Reading an object

**Today.** Two traits with identical method names (`ReadRoot` in
`oxiroot-io-core`, `FromRoot` in `oxiroot-hist`), a third decode trait
(`FromMember`), a dynamic reader with its own two methods, and a free function.

**One way.** One generic method on the file, one trait with one required
method behind it.

```rust
pub trait ReadRoot: Sized {
    /// Decode from a located object. `None` if the class is not this type.
    fn from_object(object: &Object<'_>) -> Option<Result<Self>>;
}

impl FileReader {
    pub fn get<T: ReadRoot>(&self, path: &str) -> Result<T>;
}
```

```rust
let h: Hist1D = file.get("pt")?;
let v: Value  = file.get("anything")?;               // dynamic: Value implements ReadRoot
let p: Histogram<Weight, 1> = file.get("pt")?;       // oxihist: same trait
let hs: Vec<Hist1D> = list.items()?;                 // collections use the same method
```

- `FromRoot` is deleted; `Histogram<S, D>` implements `ReadRoot`.
- `FromMember` is deleted; `from_object` is the same operation.
- `get_value`, `get_value_in`, `object_in`, `object_in_keyed` are deleted.
  `read_object` moves under `raw`.
- Each type goes from four functions (`read_x`, `read_x_in`, `decode_x`, a
  `FromMember` impl) to one.

## 4. Writing objects

**Today.**

```rust
h.write_root("out.root", Compression::Zstd(5))?;          // WriteRoot shorthand
hist.write_root("out.root", Compression::Zstd(5))?;       // ToRoot, same name, other trait
FileWriter::create("out.root").add(&h).write(Compression::Zstd(5))?;
FileWriter::create("out.root").put(tree_object).write(c)?; // multi-record objects
FileWriter::create("out.root").add(&h).to_bytes(c)?;
FileWriter::create("out.root").add(&h).write_threshold(c, 0)?;
```

**One way.**

```rust
FileWriter::create("out.root")
    .add(&h)
    .add(&events)                       // a dataset: same verb
    .dir("cuts", |d| d.add(&cut))
    .compression(Compression::Zstd(5))  // optional; this is the default
    .write()?;
```

- Delete `write_root` from both traits. It is the most-used shorthand (43 call
  sites in the docs and examples), so this is the costliest removal here, and
  it is the one that makes "how do I write an object" have one answer.
- Merge `put` into `add`. The two verbs exist because there are two traits.
  Give `WriteInto` a blanket implementation for every `WriteRoot`, and let
  `add` take `impl WriteInto`.
- Compression becomes a builder option (rule 5).
- `to_bytes` stays as the in-memory terminal; `write_threshold` becomes a
  test-only helper, since the form is chosen automatically.
- `add` currently needs `#[allow(clippy::should_implement_trait)]` in five
  places. That is acceptable for the one verb; do not add a second.

## 5. Updating a file

`FileReader::open` reads. `FileWriter::open` appends in place. The same word
for opposite intents is the problem. Rename to `FileWriter::update(path)`,
which is also what ROOT calls the mode. `delete`, `purge` and `compact` are
only meaningful after `update`, and today fail at `write` time if used after
`create`; a separate `FileUpdater` type returned by `update` would make that
a compile error.

## 6. Reading columns

**Today.** Three levels, each with its own value type.

| Level | Entry | Returns |
|---|---|---|
| Tree | `TreeReader::open(&f, n)?.branch("x").read::<T>()` / `.values()` / `.flat()` | `Vec<T>` / `BranchValues` / `Jagged` |
| RNTuple | `NtupleReader::open(&f, n)?.field("x").read::<T>()` / `.values()` / `.flat()` / `read_column(i)` | `Vec<T>` / `FieldValues` / `Jagged` / `ColumnValues` |
| Dataset | `Dataset::open(&f, n)?.read(["x"])` / `.batches(["x"])` | `Batch` |

Plus `ChainReader`, which exists for trees only.

**One way.** `Dataset` and `Batch`.

```rust
let events = Dataset::open(&file, "events")?;
let batch = events.read(["mass", "jet_pt"])?;
let mass: &[f64] = batch.values("mass")?;
let jets = batch.jagged::<f32>("jet_pt")?;

for batch in events.batches(["mass"]).step(1_000_000) { … }
```

- `TreeReader` and `NtupleReader` stay public for what only one format has
  (friends, the index, aliases, the header and footer), reached through
  `Dataset::as_tree()` and `as_ntuple()`. Their data-reading methods
  (`branch().read`, `field().values`, …) leave the public API.
- `BranchValues`, `FieldValues`, `ColumnValues`, both `Jagged` structs,
  `FromBranch` and `FromField` leave the public API. Phase 4 of the plan
  removes them entirely.
- Chains move up: `Dataset::chain(&[(&file, "events"), …])`, working for both
  formats. `ChainReader` goes.
- Typed convenience (`read::<Vec<f32>>()`) is not lost: add
  `Batch::to_vec::<T>("x")` if it is wanted. One method, both formats.

## 7. Writing columns

**Today.** Three writers.

- `TreeWriter` with four constructors (`create`, `create_large`, `new`,
  `new_large`), two write methods, and about 60 typed constructors on `Branch`
  (`Branch::f32`, `array_f32`, `jagged_f32`, `vector_f32`, `vector_vector_f32`,
  for eleven types).
- `NtupleWriter` with the same four constructors and about 50 typed
  constructors on `Field`.
- `DatasetWriter` with `Batch`, where the type decides the layout:
  `Vec<f32>`, `Vec<Vec<f32>>`, `Vec<[f32; 3]>`, `Vec<Option<f32>>`.

**One way.**

```rust
let batch = Batch::new()
    .column("mass", vec![91.2_f64, 125.1])?
    .column("jet_pt", vec![vec![40.0_f32, 31.5], vec![]])?;

let mut out = DatasetWriter::create("out.root", "events", Format::Tree)?
    .compression(Compression::Zstd(5));
out.write(batch)?;
out.finish()?;
```

- `TreeWriter`, `NtupleWriter`, `Branch`, `Field`, `SplitMember` and the two
  `raw::write_*` functions leave the public API.
- `write` takes `Batch` by value (it currently takes `&Batch` and clones it).
- Mark `DatasetWriter` `#[must_use]`: dropping it without `finish()` loses up
  to 32 MiB of buffered entries.
- **Gap to close first.** `Field` can express shapes `Column` cannot:
  `std::variant`, half/truncated/quantized floats, `std::map`, `std::set`,
  `std::atomic`, `std::bitset`, and `std::unique_ptr` as distinct from
  `std::optional`. A C-style `x[n]` tree branch (as opposed to
  `std::vector<T>`) is in the same position. Either add an encoding hint to
  the column (`batch.column_with("x", data, Encoding::Half)`) or accept that
  these are not writable. Decide before removing `Field`.

## 8. Merging

Keep the builder, because it is the one that can grow options.

```rust
let report = Merger::new().inputs(["a.root", "b.root"]).merge("all.root")?;
```

`merge_files` is deleted. `merge_histogram_files`, `append_trees`,
`append_ntuples`, `to_branch` and `to_field` become private. Put each merge
kind's data in its `MergeKind` variant (see
[findings.md](findings.md#oxiroot-facade-and-hadd)).

## 9 and 10. Histograms

This is the largest duplication in the workspace, and the choice is yours; see
[the decision](#decisions-that-are-yours). Whatever you choose, these apply:

- **Conversion.** Five `ToRoot` methods become one: `to_root() -> Result<Root>`.
  The content type is set on the result with the existing
  `with_bin_content_type`. A panicking default is a hazard here, because one
  failure cause is an upstream `AxisKind` this crate does not know yet, and
  that enum is non-exhaustive.
- **`Axis::variable`** should return `Result`. Today it panics and
  `try_variable` is the fallible twin; keep one fallible constructor.
- **Construction today is roundabout**: `Hist1D::new` is `pub(crate)`, so the
  documented way to get a ROOT histogram is to build an oxihist one and call
  `.to_root()`. If `Hist1D` stays a type users fill, give it its own
  constructor; if it does not, do not give it `fill`.

## 11 to 14. The histogram types' own surface

`access.rs` has 173 public functions, largely the same accessors copied for
each type.

- **Name and title** (rule 9). `name` and `title` are public fields, and have
  getters, and have `named()` / `titled()`. Make the fields private.
- **Contents.** Keep `contents()` (all cells) and `values()` (in range). Remove
  `Index<usize>`, `IntoIterator for &Hist` and the `RootHistogram` trait's
  duplicate `contents()`. If generic code needs the trait, keep the trait and
  remove the inherent duplicates instead; either way, one.
- **`values()` shape.** `Hist2D::values()` returns `Vec<Vec<f64>>` and
  `Hist3D::values()` three levels. Return a flat slice plus `shape()`, which is
  what the type stores, what Arrow and NumPy users expect, and what would have
  prevented the CLI's swapped-axis bug.
- **Bin geometry.** `Hist1D::find_bin`, `bin_center`, `bin_width` and
  `bin_low_edge` delegate to the axis. Keep them on `Axis` only:
  `h.xaxis().bin_center(i)`.
- **Scaling.** `scale(c)` sits beside `add`, `multiply` and `divide`, which
  cannot be operators because they can fail. Remove `Mul<f64>` and `MulAssign`.
- **Out-of-range access.** `set_bin_content`, `set_bin_error` and
  `Axis::set_label` do nothing for a bad index; `bin_center`, `bin_width`,
  `bin_error` and `Efficiency::efficiency` return `0.0`. Return `Option` or
  `Result` (rule 4).
- **Derived histograms.** `projection_x(name)` takes a name where everything
  else uses `.named()`. `cumulative(forward: bool)` wants a `Direction` enum.
- **Mutators that can break the invariants the private fields protect**:
  `contents_mut`, `sumw2_mut`, `set_entries`. Keep them only if a real use
  needs them; they are the reason the writer has `debug_assert`s.

## 15. Graphs

`with_` means "builder on `self`" in this workspace (`with_histogram`,
`with_function`, `with_x_label`), except on `Graph`, where `with_errors` and
`with_asymm_errors` are constructors taking four and six positional
`Vec<f64>`.

```rust
// before
Graph::with_asymm_errors(x, y, exl, exh, eyl, eyh)?
// after
Graph::new(x, y)?.errors(GraphErrors::Asymmetric { ex_low, ex_high, ey_low, ey_high })?
```

`GraphErrors` already exists and names every field. `functions` and
`histogram` are public fields with builders; make them private with accessors.
`MultiErrorGraph::add_y_error` consumes `self`; rename it `with_y_error`.
Rename `GraphErrors::None` (and `Compression::None`) so they do not read as
`Option::None`; `Plain` and `Stored` are candidates.

## 16. Functions

`Func1D`, `Func2D` and `Func3D` share methods through a macro and take six or
eight positional `f64`s.

```rust
// before
Func2D::new("f", "x*y", 0.0, 1.0, 0.0, 2.0)?.with_params(vec![1.0])
// after
Func2D::new("f", "x*y", 0.0..=1.0, 0.0..=2.0)?.with_params(&[1.0])?
```

- Ranges are `RangeInclusive<f64>`; `range()` returns them.
- `param(i)` returns `Option<f64>`. `set_param` and `set_params` are replaced
  by `params_mut() -> &mut [f64]`, which cannot be out of range, and
  `with_params(&[f64]) -> Result<Self>`, which checks the length.

## 17 and 18. Options

Compression is chosen three different ways and the 64-bit form four.

| Type | Compression today | Form today |
|---|---|---|
| `FileWriter` | argument to `write(c)` | automatic, or `write_threshold(c, n)` |
| `TreeWriter`, `NtupleWriter` | argument to `create(.., c)` | `create_large`, `new_large` |
| `DatasetWriter` | `.compression(c)`, default Zstd 5 | `.large(bool)` |
| `Merger` | `.compression(c)`, default Zstd 5 | automatic |
| `ContainerWriter` | argument | `big: bool` |

**One way.** `.compression(c)` with a default of `Compression::Zstd(5)` on
every writer. The form is automatic wherever the writer can know the size; a
streaming writer keeps `.large()` with no argument. Inside the crates, a
`Form` enum replaces `big: bool`.

## 19. Errors

There are two public error types and two ways to wrap a foreign error.
`oxiroot::Error` is an enum with a variant per sub-crate. `oxiroot_io_core::Error`
has an `Other` variant that wraps any error. The facade's own `hadd` module
returns the core type, not the facade's.

**One way.** One type. The simpler direction is
`pub use oxiroot_io_core::Error` at the facade, with foreign errors carried by
`Error::Other`, because every sub-crate and `hadd` already return it. Delete
the facade enum. While there:

- Drop `Clone` and `PartialEq` from the error. They force `Io` to be flattened
  to a string and make `External` compare by `to_string()`.
- Replace the empty-string sentinels (`WrongClass::name`,
  `Decompress::context`) with `Option`.
- Move the histogram, column and schema variants out of the bottom crate, or
  accept and document that the core error is the workspace error.

## 20. Import paths

`FileWriter` can be named as `oxiroot_io_core::FileWriter`,
`oxiroot_hist::FileWriter`, `oxiroot::file::FileWriter`,
`oxiroot::hist::FileWriter` and through `oxiroot::prelude`. `Compression` is
re-exported by four crates.

**One way.**

- Sub-crates export only what they define. Delete the "so these paths keep
  resolving" re-exports in `oxiroot-hist` (`lib.rs`, `read.rs:19`, `write.rs:13`)
  and the `Compression` re-exports in `oxiroot-rntuple` and `oxiroot-hist`.
- The facade re-exports named items, not `pub use crate::*`. A glob makes every
  future item of a sub-crate, including `raw` and the hidden `cast` module,
  part of the facade's API.
- The prelude holds what almost every program needs, about fifteen names:
  `FileReader`, `FileWriter`, `Dataset`, `DatasetWriter`, `Batch`, `Format`,
  `Compression`, the histogram types, `Graph`, `Merger`. Optimiser types such
  as `CmaEs` and `Nuts` are reached as `oxiroot::fit::…`.

## 21. Describing a tree or dataset

`TreeReader` has `branch_names`, `branch_type`, `branch_len`, `branch_title`,
`branch_layout`, `branch_shape`, `branch_collection` and `collection_members`,
each a linear search by name returning `Option`.

**One way.** One iterator of a small view type, and the same at dataset level.

```rust
for b in tree.branches() { println!("{} {:?} {:?}", b.name(), b.leaf_type(), b.layout()); }
let schema: Vec<(&str, DataType)> = dataset.schema();
```

`DataType` is the column's shape without its data. It replaces the type-name
strings that `Batch::schema()` compares today, and gives `Dataset` a schema
without reading any data, which `oxroot show` currently lacks (it reads the
first entry of every branch to guess).

## 22 to 24. Smaller duplicates

- **Collections.** `ObjList::list()` and `ObjList::array()` become
  `ObjList::new(ListKind)`. `ObjMap::insert` (string key) and `ObjMap::add`
  (object key) become one `insert` taking `impl Into<MapKey>`. Add a
  `&mut self` method for use in loops; today only the consuming builder exists.
- **Compression settings.** Delete the free functions `compression_settings`
  and `split_settings`. `compress` takes a `Compression`, not a `u32`. Use
  `TryFrom<u32>` in place of `from_setting -> Option<Compression>`, which today
  can return `Some(Compression::None)`.
- **Streamer info.** `streamer_registry()` returns a reference (it currently
  deep-clones a cached value on every call). `streamer_info_object` and
  `streamer_info_list` move under `raw`.
- **Constructors by type name.** `Parameter::f64("lumi", 1.0)`,
  `Parameter::i32(..)` and so on become `Parameter::new("lumi", 1.0)` with
  `From<f64>`, `From<i32>` … for `ParamValue`.
- **Matrix access.** `Matrix::get(i, j)` panics. `get` returns `Option`; put
  the panicking form on `Index<(usize, usize)>`.
- **Ranges.** `.range(a..b)` on readers and `Batch::slice(a..b)` take a
  `Range`; `Column::slice(start, end)` takes two integers. Use `Range`.

## Naming, in one table

| Today | Convention | Rename to |
|---|---|---|
| `read_root_in`, `open_in`, `get_value_in`, `object_in` | paths | removed |
| `FileWriter::open` | verb matches intent | `update` |
| `FileWriter::put` | one verb | `add` |
| `Graph::with_errors`, `with_asymm_errors` | `with_` is a builder | `.errors(..)` |
| `MultiErrorGraph::add_y_error(self)` | consuming builder | `with_y_error` |
| `BranchValues::as_i64_vec` | `as_` does not allocate | `to_i64_vec` |
| `BranchValues::as_str -> Option<&[String]>` | name matches type | `as_strings` |
| `ErrorMode::to_code` | `Copy` getter | `code` |
| `Matrix::get`, `SymMatrix::get` | `get` returns `Option` | `get -> Option`, plus `Index` |
| `RBuffer::is_empty` (cursor at end) beside `len` (total) | `len() == 0 ⇔ is_empty()` | `is_at_end` |
| `Compression::None`, `GraphErrors::None` | do not shadow `Option::None` | `Stored`, `Plain` |
| `streamer_gen::{El, Ek, strf, objptr, objanyptr}` | spelled-out names | `Element`, `ElementKind`, `string`, `object_ptr`, `any_ptr` |
| `Kind::RNtuple` (CLI) | consistent casing | `Ntuple` |
| `Vector::elements()` | slice view | `as_slice`, plus `AsRef<[f64]>`, `From<Vec<f64>>` |
| `IntoColumn` | standard trait | `From<Vec<T>> for Column` |

## Decisions that are yours

Four of the choices above change what the library is, so they should be made
deliberately. A recommendation is given for each.

### A. Which type is "the histogram"?

Today a user builds an `oxihist::Histogram`, calls `.to_root()` to get a
`Hist1D`, and can then fill, scale, add, rebin, project, fit and plot either
one. Both types have a full analysis API.

| | Option 1: oxihist is the histogram | Option 2: `Hist1D` is the histogram |
|---|---|---|
| In memory | `Histogram<S, D>` | `Hist1D` / `Hist2D` / `Hist3D` / `Profile*` |
| ROOT types | I/O records: read, write, inspect ROOT-only fields; no `fill`, no arithmetic | full analysis API, with public constructors |
| oxihist | the analysis API | an optional conversion, off the main path |
| Removes | most of `ops.rs`, `derive.rs`, `stats.rs`, `compare.rs`, `family.rs`, `sample.rs` and the fill methods | the `Hist::reg(…).to_root()` detour in every example |
| Costs | conversion derives the moment sums from bin centres, so `mean()` after a round trip differs slightly from ROOT's fill-time value; `Efficiency`, `PolyHist`, `SparseHist` have no oxihist form and keep their own API | two analysis libraries remain in the ecosystem; oxifit and oxiplot integrate with oxihist first |

**Recommendation: option 1.** oxihist, oxifit and oxiplot are separate crates
built around `Histogram`; oxiroot's distinct job is ROOT I/O. It is also the
larger reduction: roughly half of `oxiroot-hist`'s 368 public functions.
Before committing, decide whether the moment-sum difference matters to you; if
it does, carry the fill-time sums across in the conversion.

### B. Remove `write_root`?

It is the shortest way to write one object and the most used in the docs. The
rule says remove it. **Recommendation: remove**, and make the
`FileWriter` form as short as it can be (default compression, so
`FileWriter::create(p).add(&h).write()?`).

### C. Do `TreeReader` and `NtupleReader` stay public?

**Recommendation: yes, for metadata only**, reached through
`Dataset::as_tree()` / `as_ntuple()`. Their data-reading methods and value
enums become private. Users who need a raw column or a `std::variant` field
today would lose that until `Column` can represent it, so list those cases
before removing.

### D. Does lazy `open` become the default?

**Recommendation: yes**, after the benchmark check in section 1.

## Order of work

1. Paths instead of `_in` (section 2). Self-contained, removes the most code.
2. `file.get::<T>` and the one-method `ReadRoot` (section 3).
3. Options as builders; `add` absorbs `put`; `update` (sections 4, 5, 17, 18).
4. One error type, one import path, small prelude (sections 19, 20).
5. Columnar API narrowed to `Dataset` (sections 6, 7), after the encoding gap
   is closed.
6. Decision A, then the histogram surface (sections 9 to 14).
7. The renames in the naming table, as each area is touched.
