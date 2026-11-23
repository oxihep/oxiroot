# Changelog

Notable changes to oxiroot, by release. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): before 1.0, a minor release may
change the API.

## [Unreleased]

### Added

- `oxiroot::ntuple::raw::write_ntuple_clusters`: an RNTuple with a cluster per
  set of fields, into an open container.
- `oxiroot::file::raw::PendingFile`: a file written in place of a path,
  which takes the path's place on `commit` and is deleted if dropped first.
- `Efficiency::is_weighted`, read from ROOT's `kUseWeights` bit and written
  back with it.
- The shortest Bayesian interval (ROOT's `SetShortestInterval`):
  `Efficiency::shortest_interval` and `with_shortest_interval`, read from and
  written to ROOT's bit. An efficiency with a Bayesian option is now written
  with ROOT's `kIsBayesian` bit too.

### Changed

- **A `NamedBatch` is written in chunks** of about 1 MiB per column, as a
  `DatasetWriter` cuts a cluster: a basket per branch per chunk in a `TTree`,
  an RNTuple cluster per chunk. It was one basket per branch or one cluster for
  the whole dataset, which a read could neither window nor share across
  threads. ROOT 6.40 reads both forms of a 1-million-entry batch.
- `zrip` is pinned to exactly `0.8.22`: the Zstd encoder calls a function
  zrip hides from its docs, which a patch release could change.
- ROADMAP lists the 0.10.0 and 0.11.0 work; the README and installation guide
  no longer name the removed `FileReader::open_ranged`.

### Fixed

- **A crafted file could make opening it run out of memory.** A key whose
  `fKeyLen` was shorter than its own header (0, say) sent the reader back to
  where the key started, so a key list claiming two billion keys read the same
  key until memory ran out: 77 bytes were enough to make `FileReader::open`
  allocate 2 GB. Such a key is now a format error. The fuzzers found it; their
  three inputs are regression tests (`fixtures/fuzz/`).
- **A `std::variant` column was written once per chunk.** A slice of a union
  kept all its alternatives, and a writer cuts each cluster into chunks of
  about 1 MiB per column, so every chunk stored every alternative value of the
  batch: 400 000 entries in 6 chunks took 27 MB for 11 MB of values, and read
  back six times over. A slice or a filter of a union now keeps only the values
  its entries hold. `hadd` and `NamedBatch` had the same growth.
- **A `std::variant` RNTuple over several clusters read wrong from the second
  cluster on**, for files ROOT wrote: a switch index counts its alternative's
  values within the cluster, and the reader did not add those of the clusters
  before. It now does, from each alternative's own pages, so variant files
  oxiroot 0.10 and 0.11 wrote (every cluster with every value) still read right.
  A ROOT-written fixture over three clusters covers it.
- **A streamed file no longer replaces its path before it is complete.**
  `DatasetWriter::create`, `TreeWriter::create` and `NtupleWriter::create`
  truncated the path at once, so a write that failed or was dropped part-way
  destroyed the file there and left a broken one. They now write beside the
  path and rename over it when `finish` succeeds, as `FileWriter` and `hadd`
  already did.
- **A replaced file reaches the disk before it takes its name**, so a crash
  or power cut right after a write cannot leave an empty file (`FileWriter`,
  `hadd`, and the streaming writers above).
- **`Efficiency::interval` of a weighted efficiency is ROOT's.** It used the
  unweighted formulas whatever the weights. As ROOT does when an efficiency
  was filled with weights (`kUseWeights`), every frequentist option is now the
  normal approximation over the sums of weights and of their squares, and a
  Bayesian one the Beta posterior over the effective counts. Checked against
  ROOT 6.40 on a new fixture (`scripts/gen_efficiency_weighted.C`) to 1e-9.

## [0.11.0] — 2026-11-22

HEP plotting: oxiplot 0.12.0 and oxihist 0.16.0, an efficiency with its
interval, and examples that use them.

### Added

- **oxiplot 0.12.0** behind the `plot` feature: Poisson error bars, per-width
  and density normalisation, under- and overflow bins, hatches, more markers,
  `XyPoints::ratio` / `pull` / `difference`, an uncertainty band around one,
  shaded spans, explicit ticks, a twin y axis, a legend title and columns,
  and the `cms`, `atlas`, `lhcb` and `alice` styles. The plotting guide
  covers them.

- **`Efficiency::interval(bin)`**: the confidence interval of a bin, at the
  object's confidence level and by its statistic option. Clopper–Pearson,
  normal, Wilson, Agresti–Coull and the three Bayesian options are computed
  and agree with ROOT 6.40; Feldman–Cousins and mid-P are refused.
- **An `Efficiency` is plottable** (`plot` feature): `Axes::errorbar(&eff)`
  draws a point per bin that holds trials, with its interval. The
  `efficiency` example draws its turn-on this way.

### Changed

- **oxihist 0.16.0**: a histogram hands its flow bins to a plot.
- Plots have finer ticks, a stat box rounded to its uncertainties, and a
  legend kept clear of the data (oxiplot 0.12.0), so figures differ from
  those of 0.10.
- The examples use the new plotting: `open_data_zmumu` the ATLAS style,
  Poisson errors and a ratio with the simulation's uncertainty band;
  `open_data_dimuon` the CMS style and pairs per GeV; `plot` the ratio
  helper.

### Fixed

- The `plot` example compares simulation with a data histogram in the same
  binning, and its ratio panel is computed bin by bin; the pull plot of
  `tag_and_probe` leaves room for its legend.
- The plotting guide said records implement oxiplot's data traits; they are
  drawn through `to_hist()`.

## [0.10.1] — 2026-11-22

Two examples on real collision data, and oxiparticle 0.2.0. No change to
the library.

### Added

- **Two examples on real collision data**, read from CERN's open-data
  server over `root://`:
  - `open_data_dimuon`: the CMS 2012 dimuon spectrum. It streams 2 million
    events, writes the selected pairs as an `RNTuple` skim one cluster at a
    time with the spectrum and the counts in the same file, and measures
    the J/ψ, Υ(1S) and Z from the skim.
  - `open_data_zmumu`: ATLAS 2012 data against the Z → μμ simulation. One
    selection and cut flow on both, the simulation scaled and subtracted,
    and everything written to one file: histograms, parameters and both
    candidate tables, in a directory per sample.

### Changed

- **oxiparticle 0.2.0** (Rust 1.100 and the 2024 edition; no API change).
- The three open-data examples take their lepton IDs and the J/ψ, Υ(1S)
  and Z masses from `oxiroot::particle`, where the IDs and the Z mass were
  written out as numbers.
- The `analysis` example writes its `RNTuple` and `TTree` next to a
  histogram in one file, and the `tree` example stores a histogram in the
  file its tree is streamed to.

## [0.10.0] — 2026-11-22

One way to do each thing. A review of the whole workspace settled one column
type for every reader and writer, one histogram record, one way to write a
dataset next to histograms, and one naming convention across the API. Most
of it is breaking; see **Changed**.

### Changed

- **Breaking: one column type for every reader and writer.** `Column` and
  `DataType` (now in `oxiroot-io-core`; `oxiroot::data::{Column, DataType}`
  as before) are what the tree and RNTuple layers speak too:
  - `tree.branch(name).values()` and `ntuple.field(name).values()` return a
    `Column`, stored flat: a list is offsets over one buffer, a fixed array
    `Column::Array`, a string column offsets over one text, an RNTuple record
    `Column::Struct`, an optional `Column::Optional`. `BranchValues`,
    `FieldValues` and both `Jagged` structs are removed, and so is `flat()`,
    which `values()` now is. `read::<T>()` is unchanged, and takes
    `Option<T>` for an optional (`FromColumn` replaces `FromBranch` and
    `FromField`).
  - `Branch::new(name, data)` replaces `Branch::f64`, `array_*`, `vector_*`,
    `vector_vector_*`, `string` and `split_vector`: the data's shape says
    how it is stored. `.counted()` replaces `jagged_*`, and
    `Column::list_of_structs` with `.class_name()` replaces `SplitMember`.
    `Branch::with_width` is gone: an array column carries its width.
  - `Field::new(name, data)` replaces the `Field::*` constructors, and
    `.stored_as(DataType::…)` replaces `half`, `truncated`, `quantized`,
    `set_*`, `map`, `unique_ptr_*`, `atomic_*` and `bitset`. The RNTuple
    writer's own `Column` enum is removed; `Column::variant` builds a
    `std::variant`.
  - The element type is the data's: `vec![1.0, 2.0]` is `f64`, where
    `Branch::f32("x", vec![1.0, 2.0])` used to say `f32`. Write
    `vec![1.0_f32, 2.0]`.
- **oxihist 0.15.0**, whose `Hist` builder no longer has `.name(..)` and
  `.title(..)`: name a histogram with `.named(..)` and `.titled(..)`.
- **Breaking: one convention for each thing the API does**, from a second
  review of the public surface:
  - **Names.** No constructor takes an object's name; every object is named
    with `.named(..)`: `Parameter::f64(137.5).named("lumi")`,
    `Func1D::new("[0]*x", 0.0..=1.0)?.named("f")`,
    `Axis::new(50, 0.0, 100.0).named("x")`,
    `GraphFunction::new(formula, params, lo, hi).named("fit")`.
  - **`append`** adds entries: `DatasetWriter::append(batch)` (was `write`)
    and `NamedBatch::append(batch)` (was `push`), as `Batch::append`.
    `write()` only ever produces a file.
  - **`add`** only puts an object in a container. The arithmetic sum is
    `add_scaled(&other, c)` on histograms and `merge(&other)` on
    `Parameter` and `Efficiency`.
  - **`with_schema`** replaces the `schema(..)` builder on `NamedBatch` and
    `DatasetWriter`; `schema()` only reads.
  - **Private fields with accessors** on `Axis` (`name()`, `title()`,
    `labels()`, `is_time()`, `time_format()`, `set_title(..)`),
    `Efficiency`, the profiles and `MultiErrorGraph`, with `with_*`
    builders for what was a public field.
  - **`Option` for a missing entry:** `Strings::get(i)` and
    `Axis::edge(i)`. `Column::as_slice::<T>()` replaces `Column::values`.
  - **One import path:** `FileReader`, `FileWriter`, `FileUpdater`,
    `Compression` and `Value` at the crate root only; `Column` and
    `DataType` in `oxiroot::data` only; `RBuffer`, `WBuffer`, `Patch`,
    `CountToken` and `read_object` in `oxiroot::file::raw`.
  - `Batch::insert`, the matrix `decode_*` functions and
    `linalg::streamer_classes` are no longer public.
  - `NamedBatch::entries()` returns `u64`.
- **Breaking: a dataset in memory is a `NamedBatch`, added to a file like any
  other object.** `NamedBatch::new(name, format, batch)` replaces `DatasetObject`
  (now in the prelude): `push` takes more batches, and
  `FileWriter::add(&named)` stores it next to histograms and graphs, by
  reference like them, so everything is built first and written at the end.
  `.add(named)` hands the values over instead of copying them.
- **Breaking: `Axis::read` and `GraphFunction::read_tf1_body` take the
  buffer's `TagReader`**, and `raw::read_pointer_header` and
  `raw::PointerHeader` are removed. Every object a histogram, profile,
  efficiency, sparse histogram, graph or function holds by pointer is read
  through the class-tag map of its key, which `Object::reader()` builds
  along with the buffer, so a class named by its position resolves there
  too.
- A `std::variant` field is a `Column::Union`: a batch holds it, a
  `Dataset` reads and writes it, and `hadd` merges it. It was refused.
- `FieldRead::to_field` and a dataset round trip keep every stored type the
  RNTuple reader reads (records, nested collections, sets, maps, optionals,
  variants, reduced-precision reals); `to_field` refused most of them.
- `Header::stored_type` says how an RNTuple field is stored, given the shape
  it reads as.
- **Breaking: `Hist1D`, `Hist2D` and `Hist3D` are one type,
  `HistRecord<const D: usize>`**, under the same three names (now type
  aliases), and `Func1D`, `Func2D`, `Func3D` are `Func<const D: usize>`.
  Code that names the types and calls their methods is unchanged; an `impl`
  written for all three by hand can be written once.
- **Breaking: bin and cell counts are `usize`**: `Axis::new` and
  `Axis::nbins`, `ncells()` on every record, `Efficiency::new` and
  `SparseHist::new`. A file whose `fNbins` is negative is a format error.
- **Breaking: `HistStack` and `GraphStack` are `Stack<Hist1D>` and
  `Stack<Graph>`**; `members()` replaces `hists()` and `graphs()`.
- **Breaking: `PolyBin` has no `xmin`/`ymin`/`xmax`/`ymax` fields**;
  `bounds()` gives the box, derived from the vertices.
- **Breaking: the prelude holds what reading and writing a file takes**, 26
  names. Fitting is `oxiroot::fit` (`use oxiroot::fit::{FitExt, Model}`),
  collections and parameters `oxiroot::file`, matrices `oxiroot::linalg`,
  merging `oxiroot::hadd`, and `Graph2D`, `GraphErrors`, `PolyHist`,
  `SparseHist`, `ThreadedHist`, `BinContentType`, `Random` and the stacks
  `oxiroot::hist`.
- **Breaking: `Error` is no longer `Clone`, `PartialEq` or `Eq`**: it holds
  the error it came from. `Error::Io` holds the `std::io::Error`, and
  `source()` returns it; compare errors with `matches!`. `CompressError`
  likewise, with `Codec { algorithm, source }` for a codec's own failure.
- **Breaking: `WriteRoot::write_body(&self, &mut WBuffer)` is the method to
  implement**; `to_root_bytes()` is provided from it. `root_class` returns a
  `Cow<'static, str>`.
- **Breaking: `Column::String` holds its text as a `String`**
  (`{ offsets, text }`, where it held bytes), so a string column is UTF-8 by
  construction. `IntoColumn` is gone: a column is made with `Column::from`
  or `.into()`, and `Batch::column` takes `impl Into<Column>`.
- **Breaking: getters that returned a list return an iterator**:
  `TreeReader::branch_names`, `unsupported_branches`, `streamer_classes` and
  `collection_members`, `NtupleReader::field_names`,
  `StreamerRegistry::class_names`. Add `.collect::<Vec<_>>()` where a list
  is wanted.
- **Breaking, low level:** `compress`, `decompress` and
  `decompress_payload` return a `Cow<[u8]>` (borrowed when nothing was
  copied); `FileReader::subdir` returns an `Arc<Directory>`;
  `Key::payload_range` is replaced by `checked_payload_len`; a directory's
  times are `Datime`s; `streamer_gen`'s builders are named for what they
  describe (`string`, `object_ptr`, `basic_ptr`, …, with `ElementDef`);
  `raw::read_column` takes a `&ColumnDescriptor` and
  `ContainerWriter::build_append` a `&Bytes`.
- `.parallel(bool)` exists on the tree, RNTuple and dataset readers with or
  without the `rayon` feature; without it the read stays on one thread.
- `hadd` writes merged histograms one object at a time, to a temporary file
  it renames when complete: memory no longer grows with the output, and a
  failed merge leaves no partial file.
- `oxroot dump` reads trees and RNTuples through `Dataset`: nested lists,
  fixed arrays, optionals and records print their values, where they
  printed `...` (and `null` in JSON).
- A `THnSparse` whose file lists one cell twice reads as one cell with the
  two contents summed.
- `streamer_types` names ROOT's `fType` codes, and
  `StreamerElement::kind()` classifies one; `Header::top_level_fields`,
  `top_level_field` and `child_fields`, `FieldDescriptor::parent` and
  `ColumnDescriptor::field` walk an RNTuple schema without index casts;
  `TagReader::open_list` opens a `TList` member.
- **Breaking: no value stands in for "nothing".** `Axis::bin_center`,
  `bin_width` and `bin_low_edge` return `Option` (`None` for a bin the axis
  does not have, where they returned `0.0` or clamped);
  `Efficiency::efficiency` returns `Option` (`None` for a bin without
  trials); `PolyHist::fill` and `fill_weight` return `Option` (`None`
  outside every bin, where they returned `0`); `SparseHist::tsumw2` returns
  `Option` (`None` where ROOT stores `-1`); `TreeIndex::minor_name` and
  `Friend::file_name` return `Option<&str>` where they returned `""`.
  `Axis::set_label` and `PolyHist::set_bin_content` return a `Result` and
  report a bin that does not exist; they did nothing before.
- **Breaking: `Error::WrongClass::name` and `Error::Decompress::context` are
  `Option<String>`**, `None` where they were empty.
- **Breaking: `Efficiency::statistic_option` is a `StatisticOption` and
  `MultiErrorGraph::sum_errors_mode` a `SumErrorsMode`**, in place of
  ROOT's integer codes. A file with a code ROOT does not define is an error.
- **Breaking: the `name` and `title` fields of the histogram, profile,
  graph, efficiency, sparse and poly records are private.** Read them with
  `name()` / `title()` and set them with `named()` / `titled()`, on every
  type. `Graph`'s `histogram` and `functions` fields are private too:
  `histogram()`, `functions()`, `with_histogram`, `with_function`. A frame
  set through the field skipped the `TH1F` coercion `with_histogram` does.
- **Breaking: `TreeReader`'s `flat()` returns `Error::Unsupported`** for a
  branch without a flat form (a string or nested branch), as the RNTuple
  reader does; it returned `InvalidInput`.
- **Breaking: a compression level is a `u8`** (`Compression::Zstd(5)` is
  unchanged; a `u32` variable needs a conversion), and `Compression` is
  `#[non_exhaustive]`. `Compression::level()` gives the level.
- Every public function that returns a `Result` documents its errors
  (`clippy::missing_errors_doc` is on for the workspace).
- A list or map read from a file is named after its key, also when ROOT
  stored it with a name of its own.
- `hadd` writes a merged tree or RNTuple file to a temporary file and
  renames it, as every other writer does, so a failed merge no longer
  leaves a cut-short output.
- An LZMA level above 9 selects the strongest XZ preset; it was passed
  through to the encoder.
- `oxroot dump` prints a value below `1e-4` in scientific notation; it
  printed `0`. `oxroot stat` names an unknown compression setting.
- `Algorithm` implements `Hash` and `Display`.
- **Breaking: `Hist1D`, `Hist2D`, `Hist3D` and the `Profile*` types are I/O
  records.** A histogram is oxihist's `Histogram`: build it with `Hist`, fill
  and analyse it, and call `to_root()?` to store it; read one with
  `file.get::<Hist1D>(path)?.to_hist()?`. The records keep what a ROOT file
  holds and oxihist does not: the stored fields (`contents()`, `values()`,
  `sumw2()`, `entries()`, `tsumw()`…, the axes), `class_name()` and
  `with_bin_content_type`, axis labels and time formats (`xaxis_mut()`), and
  `add`, the merge `hadd` performs. Removed from them: `fill`/`fill_weight`,
  `track_sumw2`, `scale`, `multiply`, `divide`, `integral`, `reset`,
  `bin_error`, `set_bin_content`/`set_bin_error`, `contents_mut`/`sumw2_mut`,
  `mean`/`std_dev`/`maximum`/`minimum` and their `_bin` and per-axis forms,
  `effective_entries`, `interpolate`, `quantiles`, `rebin*`, `cumulative`,
  `projection_*`, `profile_*`, `get_random`/`fill_random*`, `smooth`,
  `chi2_test*`, `kolmogorov_test`, the `RootHistogram` trait, the
  `ThreadedFill1`–`4` traits, and their `FitData` and plot-data
  implementations. Each has an oxihist counterpart (`integral(flow)`,
  `rebin(axis, factor)`, `project([axes])`, `profile(axis)`, `sample`,
  `chi2_test(other, kind)`, `value([i])`/`bin_mut([i])`…). `Efficiency`,
  `SparseHist` and `PolyHist` are still filled directly. Differences to
  know: `interpolate` returns `None` outside the axis; `slice` leaves out a
  bin that starts at its upper bound; per-bin errors come from the storage
  (`.weight()`), not from a switch; a time format or a bin content type is
  set on the record after `to_root()`.
- **oxihist 0.14.0**: its histograms keep the statistics of their fills
  (the number of fills, `Σw`, `Σw²`, `Σw·x`, `Σw·x²` and the cross terms; a
  profile's sums of the sampled value) and gain `multiply`, `divide`,
  `chi2_test`, `kolmogorov_test`, `smooth`, `interpolate`, `slice`,
  `profile`, `maximum`, `minimum` and `effective_entries`. See its
  changelog; `mean` and `std_dev` of a filled histogram are now those of the
  values filled, and a fill is slower (figures in
  `docs/reference/performance.md`).
- **`to_root()` writes the fills' own statistics.** `fEntries` is the
  number of fills and `fTsumw*` their sums, so ROOT's `GetEntries`,
  `GetMean` and `GetRMS` on a written file match a histogram filled in
  ROOT. Before, `fEntries` was the sum of the bin values and the sums were
  estimated from the bin centers; a histogram without fill statistics
  (built with `from_bins`, or changed bin by bin) is still written that
  way.
- **`to_hist()` restores them.** The oxihist histogram read from a ROOT
  one has the file's `fEntries` and `fTsumw*` as its fill statistics, so
  they survive a round trip exactly. A ROOT histogram that has entries but
  empty sums (`fTsumw == 0`, as after `SetBinContent`) reads without fill
  statistics, and its mean is computed from the bins, as ROOT does.

### Fixed

- **Zstd gained nothing on repeating data whose bytes look random**
  (identifiers, hashes, a table stored twice): the encoder samples each
  block's byte counts and stores a flat-looking block without searching it
  for matches. Such a block is now encoded a second time with the search
  on, and the smaller result kept: a 2008-byte period goes from 1x to 170x.
  Data that compresses is unaffected; nearly incompressible `f64` takes
  about 18% longer to compress.
- **A `TEfficiency` over `TH1F` histograms could not be read.** Its two
  histograms were taken to be `TH1D`; they are read as the class their
  header names. An efficiency over two- or three-dimensional histograms is
  refused by name, where it failed with an unrelated error.
- A graph's frame histogram is read as the class its header names, where a
  frame named by a back-reference was taken to be a `TH1F`.
- **A function on the second or later graph of a ROOT `TMultiGraph` was
  dropped.** ROOT names `TF1` in full once per buffer and by a
  back-reference afterwards, and each member graph was decoded on its own,
  where a reference into an earlier member could not be followed. The
  members are now read in place with one class map, and so are the members
  of a `TList`, `TObjArray` or `TMap`.
- **A `TList`, `TObjArray` or `TMap` read from a ROOT file and written back
  could crash ROOT.** Its members were re-encoded one by one, which left
  the class references inside them pointing at positions in the file they
  came from: a list of two histograms was enough. A collection read from a
  file is now written back with the bytes it was read from, and members
  added since are appended. Those bytes are only valid under a key with
  the same header, so writing the collection under another name, or inside
  another collection, is an `InvalidInput` error instead of a corrupt file;
  decode its members and build a new one for that. `FileUpdater::compact`
  checks the same for every object it copies.
- **A function with named parameters, read and written back, evaluated to
  0 in ROOT.** ROOT stores a predefined formula such as `gaus` as
  `[Constant]*exp(…[Mean]…[Sigma]…)`; it was written back with those names
  but a parameter map of `p0`, `p1`, `p2`, which gave ROOT six parameters,
  none set. The formula is now read into the positional form (`[p0]`…),
  which is also the form this crate can evaluate. This affected a `Func1D`,
  and a function attached to a graph. The parameter names themselves are
  not kept: ROOT shows `p0`, `p1`, ….
- **`hadd` merges every tree and `RNTuple` a `Dataset` can read.** It copied
  branches and fields through the low-level writers, which refused an
  `RNTuple` with a struct, a nested vector, an optional or a
  `std::vector` of unsigned integers, and a tree with split objects. It
  now reads and writes through `Dataset` and `DatasetWriter`, one cluster
  at a time, keeping each column's stored type. Of the 35 single-dataset
  fixtures, 34 merge and read back equal to a chained read of the inputs;
  the one that does not holds a `std::variant`. Many small inputs become
  clusters of a useful size (one per input before). A tree's counted array
  whose counter is not a branch named `n<array>` is written as a
  `std::vector`. On four 2-million-entry inputs the tree merge takes the
  same time and its output is 5.6% larger (smaller baskets); the `RNTuple`
  merge is 15% faster at the same size.
- **Breaking: `raw::append_trees` and `raw::append_ntuples` are removed.**
  `hadd` was their one user and no longer needs them; `Merger` is the way
  to concatenate, and it handles more. `to_branch` and `to_field` stay.
- A tree or `RNTuple` merge that fails part-way leaves no output: it is
  written beside the output and renamed when complete.
- A histogram record writes the number of cells its arrays hold; it wrote
  back the `fNcells` it had read, which a file could have wrong.
- A key of a class such as `TH1K` read as a `Hist1D` is a `WrongClass`
  error; it was `Unsupported`.
- **A `DatasetWriter` whose file could not be started panicked on the next
  write.** The first failure was returned, but the writer was left in a
  state the next call did not expect. Every later `write` and `finish` now
  returns an error.
- A `TTree` whose description lacks `fEntries` is a format error; it read
  as a tree of no entries.
- `oxroot show` reads a tree's branch types from its metadata; it decoded
  the first entry of every branch to tell a scalar from a list. It also
  takes an `RNTuple` field's type from the top-level field of that name,
  not from the first field so named.
- A `Dataset` read reported any error from the flat read of a tree branch
  as "not flat" and read the branch a second time the slow way; only
  `Unsupported` falls back now, and a read error is returned.
- An RNTuple read reported every failure to fetch a page or envelope as
  "runs past end of file"; a failed read (a dropped connection, say) is now
  returned as it is.
- `Column::slice` panicked on a column built by hand whose offsets did not
  match its values; it now slices as far as the values go.
- A `THnSparse` whose dimension count disagrees with its axes is a format
  error.
- Matrix constructors document the error they return (`InvalidInput`).

### Added

- `DatasetWriter::add(&object)` stores a histogram, a graph or a parameter
  in the file a dataset is streamed to, so a dataset too large to hold
  shares its file with them in one pass. `TreeWriter::finish_with` and
  `NtupleWriter::finish_with` take the same objects (`raw::Objects`).
- `ClassKind`, what a ROOT class name says about how its objects are
  stored (`Tree`, `Ntuple`, `Directory`, `Hist(dim)`, `Other`): the one
  table the readers, the merger and the CLI now share.
- **A `Hist1D` keeps the functions stored with it** (`fFunctions`, where
  ROOT's `TH1::Fit` leaves the fitted curve): `functions()` reads them and
  `with_function` attaches one. They were dropped on read, so a histogram
  read and written back lost its fit. ROOT draws them over the histogram.
  Functions that are not formula `TF1`s are still skipped.
- `BinContentType::from_code`, the inverse of `code`.
- `WriteRoot::key_check` and `KeyCheck`, for an object whose bytes are tied
  to the key it was read from; `Object::member_in` and `Object::within`, so
  that a decoder can read a collection member where it lies; `Object::title`.
- A `TagReader` resolves a class reference it has not seen declared by
  reading the declaration the reference points at.

## [0.9.0] — 2026-11-20

Zstd at C speed in safe Rust (zrip), lists of structs in a `TTree`, a
comparison with uproot, and CI that gates coverage and tracks benchmarks.

### Added

- **Lists of structs in a `TTree`**. A `Batch` column that is a list of
  structs of numbers (a `Column::List` of a `Column::Struct`) writes to a
  `TTree` as a split `std::vector<Struct>`: one sub-branch per member
  (`hits.x`, `hits.id`), the struct a class named after the column. ROOT 6.40
  and uproot read it back. `Dataset` reads a split collection, oxiroot's or
  ROOT's, as that list of structs again, so the same batch round-trips through
  both formats.
- **`TreeWriter` writes split `std::vector<Struct>` branches**, batch by
  batch, a basket per member per batch (they needed `raw::write_tree` before).
  `raw::write_tree` cuts them into baskets of `entries_per_basket` too.
- `TreeReader::collection_members` and `branch_collection`: the members of a
  split collection, and the collection of a member.
- `Branch::with_width`: a fixed-array branch's width, for a branch with no
  rows. `SplitMember::bool`.
- An interop case streams a split `std::vector<Hit>` over several batches
  under Zstd; ROOT C++ and uproot check it.
- **A comparison with uproot**: `scripts/bench_uproot.py` reads the same
  `TTree` and RNTuple files, written by oxiroot and by ROOT's RDataFrame
  (`scripts/bench_write_root.C`), with `Dataset` and with uproot, serial and
  parallel (example `read_timing`). Results and analysis on the
  [Performance](https://oxihep.github.io/oxiroot/reference/performance/#against-uproot)
  page.
- **Coverage in CI**: a PR's changed lines must be at least 80 % covered
  (cargo-llvm-cov and diff-cover), with a comment on the PR listing the
  uncovered ones. main's report and a README badge are published with the docs
  site (`/coverage/html/`). The workspace is at 84 % of lines.
- **Benchmark history**: the weekly bench run (now with the `write` bench and
  `rayon`) is kept on the `bench-data` branch, charted under `/bench/` on the
  docs site, and a result 1.5× slower than the last comments on the commit.
- **CodeQL** scans the Rust, the Python scripts and the workflows on every PR
  and weekly.
- **Releases**: pushing a version tag publishes a GitHub Release with that
  version's CHANGELOG section (`release.yml`; runnable by hand for older tags).
- CI jobs for spelling (`typos`, `_typos.toml`), unused dependencies
  (`cargo-machete`) and the workflows themselves (`actionlint`).
- `.pre-commit-config.yaml`: `cargo fmt`, typos, a gitleaks secret scan and
  file hygiene before a commit, clippy before a push.
- `CITATION.cff`, so GitHub offers a citation; README badges for CI, the
  latest release, coverage and the docs.

### Changed

- **Zstd goes through [zrip](https://crates.io/crates/zrip) instead of
  `ruzstd`**: pure, safe Rust (`forbid(unsafe_code)`), MIT, within about 1.25×
  of C zstd's speed both ways and with its compression ratio. Of seven Rust
  Zstd crates timed on ROOT's blocks, it was the fastest without `unsafe`.
  - Reads: a one-thread read of a ROOT-written `TTree` is 2.4× faster, of an
    RNTuple 2.5×. oxiroot now matches or beats uproot on one thread in seven
    of eight cases of the comparison, and in parallel is up to 3.6× faster.
  - Writes: 3–6× faster, and the files are about a third smaller (`ruzstd`
    had a single fast level, which left many blocks nearly raw). Zstd levels
    1–4 are zrip's; higher levels use its level 4, which compresses physics
    data about as well as C zstd's level 5.
  - A column that barely compressed before (such as random doubles) now
    compresses a little, and reading it costs a decompression where it was a
    copy, as in ROOT and uproot.
  - Decoding stays bounded by the block header's declared size; the
    decompression fuzz target ran 4 million inputs without a crash.
- `Dataset::columns` lists a tree's split `std::vector<Struct>` once, by its
  name (`hits`), instead of its members (`hits.x`, …). The members still read
  on their own.
- Converting a batch to Polars no longer fails on an `Optional` of an
  `Optional`: as in Arrow, it folds into one level of nulls (`Some(None)` and
  `None` are both null). Arrow and Polars have no second level.

### Removed

- `oxiroot-hist`'s unused `rayon` and `thread_local` dependencies
  (`ThreadedHist` and `fill_par` live in oxihist; the `rayon` feature still
  turns on oxihist's).

### Fixed

- An empty `TTree` (written from batches with no entries) keeps its fixed
  arrays' width; it took the width from the first row, so lost it.
- **RNTuples written by ROOT with more than one page of collection or string
  offsets per cluster** read correctly. ROOT delta-encodes a split index column
  page by page, storing each page's first offset whole; the reader summed the
  deltas across pages, and failed with "collection offsets do not partition"
  (or read wrong offsets). RDataFrame snapshots of a few hundred thousand
  entries hit it. A ROOT-written fixture with about twenty pages per cluster
  covers it.

## [0.8.0] — 2026-11-20

Faster dataset reads and writes, more shapes in a `Batch`, and Arrow and
Polars conversion.

### Added

- **Arrow and Polars conversion** (the `arrow` and `polars` features):
  `Batch::to_arrow` / `into_arrow` / `from_arrow` with arrow-rs 60, and
  `Batch::to_polars` / `into_polars` / `from_polars` with Polars 0.55. Every
  column shape converts, structs and optionals (as nulls) included; a column
  with nulls comes back as an `Optional`. `oxiroot::data::arrow_rs` and
  `polars_rs` re-export the crates at the matching versions.
- **More shapes in a `Batch`**:
  - `Column::Struct` (with `Column::member`) holds RNTuple records and user
    classes: read from and written to an RNTuple, with a
    `std::vector<Struct>` as a list of one. Members `_0`, `_1`, … write a
    `std::pair` / `std::tuple`; named members a user class named after the
    column, which ROOT 6.40 reads by type emulation and uproot as is. A `TTree` writer refuses it by
    name; a tree's split members still read as `hits.x` columns.
  - `std::vector<bool>`, `std::vector<std::string>` and
    `std::vector<std::vector<T>>` columns now write to a `TTree` too
    (`Branch::vector_bool`, `vector_string`, `vector_vector_*`), with ROOT's
    class checksums. ROOT 6.40 and uproot read them back.
  - `std::vector<u32>` / `<u64>` columns write to an RNTuple.
  - A `DatasetWriter` given only empty batches writes a dataset with their
    columns and no entries, and the `TreeWriter` / `NtupleWriter` accept an
    empty first batch to fix the schema.

### Changed

- **Faster dataset writes and reads.** Profiling showed zstd was 80–90 % of
  both, so the work is now cut finer and spread over cores.
  - `DatasetWriter` cuts each cluster into chunks of about 1 MiB per column:
    a `TTree` basket per branch, or an RNTuple cluster, per chunk.
  - With the `rayon` feature, every chunk is converted and compressed in
    parallel through the new `TreeWriter::write_batches` /
    `NtupleWriter::write_batches`. For 1M entries (two scalars and a
    `std::vector<double>`), a `TTree` write takes 43 ms instead of 247 ms and
    an RNTuple write 27 ms instead of 190 ms.
  - Reading with `Batches::parallel(true)` decompresses baskets and pages in
    parallel: 17 ms instead of 62 ms (`TTree`) and 14 ms instead of 64 ms
    (RNTuple) for the same data.
  - Without `rayon` nothing runs in parallel and times are unchanged.
- RNTuple reads can decompress a column's pages in parallel
  (`FieldRead::parallel`, the new `rayon` feature of `oxiroot-rntuple`), and
  `NtupleReader` is `Clone`.
- RNTuple fields are written straight from a batch's flat offsets and values:
  no `Vec` per entry is rebuilt.

## [0.7.0] — 2026-11-20

One API to read and write `TTree` and RNTuple (breaking).

### Added

- **`oxiroot::data` (the new `oxiroot-dataset` crate): one way to read and
  write both columnar formats.** A `Batch` of named, flat, Arrow-style columns
  is what `Dataset` reads and `DatasetWriter` writes. The format is detected
  when reading and is one argument (`Format::Tree` / `Format::Ntuple`) when
  writing.
  - `Dataset::open` / `open_in`; `read(columns)` returns one batch, and
    `batches(columns)` iterates one cluster at a time, with `.step(n)`,
    `.range(a..b)` and, with `rayon`, `.parallel(true)`.
  - `DatasetWriter::create` / `new` buffers batches and closes a cluster
    every ~32 MiB of values (`.cluster_bytes`, `.compression`, `.large`).
  - `DatasetObject` places a dataset in a `FileWriter` next to other
    objects.
  - Typed views: `values::<T>` (slices, no copy), `jagged::<T>`, `arrays::<T>`,
    `strings`. `filter`, `slice`, `select` and `append` work on whole batches.
  - Reads are strict, and a column a format cannot hold is refused before
    anything is written.
  - `Dataset::read` is as fast as the fastest low-level read (`flat()`) for
    `std::vector` columns, 12–16 % faster than `values()`, and equal for
    scalars (Criterion `read` bench).
- **A read builder on the low-level readers**, which now hold their file:
  - `TreeReader::branch(name)` and `NtupleReader::field(name)`, with
    `.range(a..b)`, `.parallel(true)` (TTree, `rayon`), then `.read::<T>()`
    (typed; `Error::TypeMismatch` on the wrong type), `.values()`, `.flat()`
    (`std::vector` fields too, for RNTuple), or `.to_branch()` / `.to_field()`
    (the read → write round trip);
  - RNTuple gains entry-range reads over any cluster window, and
    `FieldValues::skip`.
- `TreeReader::branch_layout` (`BranchLayout`), `TreeReader::basket_boundaries`
  and `NtupleReader::cluster_boundaries`; `ChainReader::branch`.
- `Error::TypeMismatch`; `tree::raw::write_tree` and `ntuple::raw::write_ntuple`
  write a whole tree or RNTuple (split branches, schema extension) into an open
  container; `TreeWriter`/`NtupleWriter` both have `entries() -> u64` and
  `finish() -> W`.

### Removed

- The old high-level writers and readers, replaced by `oxiroot::data`:
  - `Tree`, `Ntuple`, `write_tree_file`, `write_tree_file_baskets`,
    `tree_file_bytes`, `write_rntuple_file`, `rntuple_file_bytes`;
  - `concat_trees` and `concat_ntuples` (the streaming `append_*` remain);
  - `TreeReader::read_branch`, `read_branch_range`, `read_branch_flat` and
    their `_par` forms;
  - `NtupleReader::read_field` and `read_field_prefix`.
- The prelude now exports the dataset API (`Batch`, `Dataset`,
  `DatasetWriter`, `DatasetObject`, `Format`) instead of the `TTree` and
  RNTuple types, which stay under `oxiroot::tree` and `oxiroot::ntuple`.

### Changed

- **One naming scheme for the format writers.**
  - TTree fixed arrays are `Branch::array_*` (were `vec_*`).
  - RNTuple `std::vector`s are `Field::vector_*` and `vector_vector_*` (were
    `vec_*`, `vec_vec_*`), and `Field::set_str` is `Field::set_string`.
  - `bools` / `strings` are `bool` / `string` on both.
  - `Branch::name` and `Field::name` / `data` are private, with accessors.
- `TreeReader`, `NtupleReader` and `ChainReader` take the file once, at
  `open`. `TreeReader::join_by_index` takes only the friend tree, and
  `NtupleReader::read_column` only the column index.
- `TreeWriter::entries` returns `u64`; `NtupleWriter::finish` returns the sink.
- `append_trees` / `append_ntuples` take the readers alone.

## [0.6.1] — 2026-11-20

XRootD over TLS with token authentication, a ROOT-confirmed RNTuple
`std::map` write, and shared profile code. Built on oxiplot 0.11.1 and
oxihist 0.13.1.

### Added

- **`Profile3D::bin_error` and `effective_entries`**, and
  `Profile2D::effective_entries`: every profile now has the same per-cell
  `bin_value`, `bin_error` (ROOT's error modes) and `effective_entries`.
- **ROOT confirms the RNTuple `std::map` write.** The interop matrix has ROOT
  C++ read oxiroot's `std::map<std::int32_t, double>` field, and ROOT-written
  map fixtures (`rntuple_map_*.root`) are read in the tests. ROOT 6.40 needs
  the map's dictionary compiled in to write or read one
  (`scripts/rntuple_map_LinkDef.h`); without it its collection proxy crashes,
  which is what had blocked this.
- **XRootD over TLS, with token authentication** (the `xrootd-tls` feature).
  `roots://` URLs, and `root://` servers that require TLS, switch the
  connection to TLS (`kXR_protocol` with `kXR_ableTLS`, then `kXR_gotoTLS`)
  before the login. Over TLS the client authenticates with a WLCG bearer token
  (the `ztn` protocol) when the server offers it, found the WLCG way:
  `BEARER_TOKEN`, `BEARER_TOKEN_FILE`, `$XDG_RUNTIME_DIR/bt_u<uid>`,
  `/tmp/bt_u<uid>`. Certificates are checked against the Mozilla roots plus
  `$X509_CERT_DIR`. `XrootdOptions` and `XrootdSource::open_with` pass a
  token or CA directory explicitly. Tested against a mock server with a test
  CA in CI, and against a real `xrootd` 6.2 by `scripts/xrootd_tls_local.sh`.
  GSI (X.509 proxy) authentication is still not supported.

### Changed

- **oxiplot 0.11.1 and oxihist 0.13.1**: the legend's automatic placement
  keeps off text annotations.
- **The profiles share their per-cell code.** Filling, the profiled value,
  its error and the effective entries come from one macro
  (`impl_profile_family!`) for `Profile1D`, `Profile2D` and `Profile3D`;
  results are unchanged. The readers stay per class, as ROOT's on-disk
  layouts differ.

### Fixed

- **`roots://` without TLS support is an error.** Without the `xrootd-tls`
  feature a `roots://` URL used to be read over a plain connection.
- **A server's authentication offer is read in its order of preference**, and
  the error for one this client cannot meet says why each protocol was
  skipped.

## [0.6.0] — 2026-11-20

API review toward 1.0 (breaking): the histogram, profile and graph state
behind accessors, one naming scheme across the oxihep crates, oxifit's
errors in `oxiroot::Error`, and the file format's records in a `raw`
module. Built on oxistats 0.2.0, oxifit 0.9.0, oxiplot 0.11.0 and oxihist
0.13.0.

### Changed

- **oxistats 0.2.0, oxifit 0.9.0, oxiplot 0.11.0, oxihist 0.13.0** (the
  same review): oxifit's `Efficiency` is `BinomialData`, `TemplateError` is
  part of `FitError`, `Model`'s parameters are read with `params()` /
  `param_names()`, toggles are named after their option and take a `bool`
  (`FitOptions::minos(true)`, `HistOpts::yerr(true)`, …), oxiplot's `Points`
  is `XyPoints`, and standard deviations are `std_dev` everywhere.
- **The histogram, profile and graph state is private.** `Hist1D`/`2D`/`3D`,
  `Profile1D`/`2D`/`3D`, `PolyHist`, `SparseHist`, `Graph`, `Graph2D`,
  `MultiErrorGraph` and `Efficiency` no longer expose their cells, errors,
  moment sums, axes or entry counts as fields, which a caller could put out
  of step (a `TH1` with more cells than its axis, which ROOT rejects, wrote
  without complaint). Read them with methods of the same name
  (`contents()`, `entries()`, `xaxis()`, `tsumw()`, `bin_entries()`, `x()`,
  `errors()`, `passed()`, …). Write through `contents_mut()` (a slice: the
  values change, never the length), `set_bin_content`, `set_bin_error`,
  `set_entries`, `sumw2_mut()`, `x_mut()`/`y_mut()` on graphs, and
  `xaxis_mut()` for titles, labels and time display. `name` and `title`
  stay public. `Axis`'s binning (`nbins`, `xmin`, `xmax`, `xbins`) is read
  with methods; its titles, labels and time format stay public fields.
- **`sumw2()` now returns the Σw² array** (`Option<&[f64]>`, `None` when
  errors are not tracked); turning tracking on is `track_sumw2()`, as the
  profiles' `track_bin_sumw2()`.
- **`oxiroot::Error` wraps oxifit's `FitError`** (a `Fit` variant, with the
  `fit` feature), so `?` works on scans, tests, samplers and template models
  in a function returning `oxiroot::Result`.
- **io-core's format records move to `oxiroot_io_core::raw`**
  (`oxiroot::file::raw`): the file header, keys, directories, free segments,
  streamer info and elements, object and record headers, the format
  constants and the helpers that read and write them. They are
  `#[non_exhaustive]`; reading and writing objects needs none of them.
- **`entries()` everywhere**: the tree and RNTuple readers' and writers'
  `num_entries()` is now `entries()`, ROOT's `GetEntries`.
- **`streamer_gen::Cls` is `ClassDef`**, and `streamer_gen` (plumbing shared
  by the crates that write ROOT classes) is hidden from the docs.


## [0.5.0] — 2026-11-20

Fitting grown into an analysis toolkit, complete analysis examples, and a
cleaner codebase. oxiroot now builds on oxifit 0.8.0, oxiplot 0.10.0 and
oxihist 0.12.0: user-defined costs, CLs limits and significances,
HistFactory-style template fits, sWeights, efficiency and bin-integrated
fits, ensemble and NUTS sampling, faster gradient fits, and plots for
limits, pulls, impacts and posteriors. Four new examples run whole
analyses from a `TTree` to plots and a ROOT file. Breaking: several enums
are now `#[non_exhaustive]` (see Changed).

### Added

- **Four complete analysis examples**, each a `TTree` in, fits, plots and a
  ROOT file out, a few seconds in `--release` and run by CI:
  `resonance_search` (a bump hunt to the Brazil band), `template_fit` (a
  HistFactory-style fit from ROOT histograms to the post-fit stack, pulls
  and impacts), `splot_lifetime` (sWeights as a friend tree, a lifetime fit
  with correct weighted-fit errors) and `tag_and_probe` (pass/fail fits to
  an efficiency map, turn-ons and a binomially fitted `TEfficiency`).
- **oxifit 0.8.0, oxiplot 0.10.0, oxihist 0.12.0.** `oxiroot::fit` gains
  Hamiltonian Monte Carlo (`Nuts`, in the prelude) with `Chain::r_hat` and
  `Chain::divergences`, and fits with an analytic gradient run about twice
  as fast. `oxiroot::plot` gains `fill_between`, `axhline`/`axvline`,
  `text`, `contour`, `y_categories`, the Brazil-band `limit_band`, and with
  the fit feature `fit_limits`, `fit_pulls`, `fit_impacts` and
  `fit_posterior`. oxiplot's public enums are now `#[non_exhaustive]`.
- **`Hist3D::minimum_bin`**, which the 1-D and 2-D histograms had.

- **oxifit 0.7.0, oxiplot 0.9.0, oxihist 0.11.0.** `oxiroot::fit` gains
  user-defined costs (`Cost`, `FnCost`, `ModelCost`, `CostSum`) run by a
  `Fitter`; CLs upper limits and discovery significances, asymptotic or from
  toys (`HypoTest`); HistFactory-style template fits with systematics
  (`TemplateModel`, `Channel`, `Sample`; pyhf JSON via oxifit's `pyhf`
  feature); sWeights with asymptotically correct weighted-fit errors
  (`SWeights`, `FitOptions::with_asymptotic_errors`); bin-integrated fits
  (`FitOptions::with_bin_integration`, ROOT's option `"I"`), for which a
  `Hist1D` now passes its real bin edges; binomial efficiency fits
  (`oxiroot::fit::Efficiency`); and an ensemble MCMC sampler (`Sampler`).
  The prelude exports `Cost`, `CostSum`, `FnCost`, `ModelCost`, `Fitter`,
  `HypoTest`, `TemplateModel`, `SWeights` and `Sampler`; the fitting guide
  has a section on each. The default minimizer is now `Minuit2Combined`
  (MIGRAD with a SIMPLEX fallback), so `charmonium_analysis` no longer sets
  it.

- **oxifit 0.5.0, oxiplot 0.8.0, oxihist 0.10.0.** `oxiroot::fit` gains
  analytic gradients (`Model::with_gradient`) and `Minimizer::LBfgsB`,
  convolution (`Model::convolve`, `Model::normal`), 2-D and 3-D fits
  (`Model::new2`/`new3`/`product`, `Points2`/`Points3`, `Events2`/`Events3`,
  `FitDataNd`, `FitOptions::range_y`/`range_z`), and reports misspelled
  parameter names (`Model::check`). **`Hist2D` and `Hist3D` can be fitted**
  (each in-range bin a point at its center), and **`Func2D::to_model` /
  `Func3D::to_model`** convert ROOT TF2/TF3 functions. The `rayon` feature now
  also parallelizes fits. The prelude exports the new data types; the fitting
  guide covers all of it.

- **CI runs the interop manifest matrix.** Every histogram type, codec, TTree
  and RNTuple shape that `interop_matrix` writes is now read back by ROOT C++
  and uproot on each build; before, only `scripts/interop_local.sh` ran it.
  That script now compiles its ROOT C++ oracles against a Homebrew ROOT too,
  and the `interop` / `interop_matrix` examples point to it when run without
  arguments.

- **`charmonium_analysis` example**: a complete J/ψ and ψ(2S) analysis on
  generated events — a dimuon `TTree` written and read back, a binned
  Crystal Ball mass fit with a shared tail and mass-scaled resolution (MINOS
  on the ψ(2S)/J/ψ ratio), its profile likelihood and 68 %/95 % contours, a
  500-toy pull study, a two-dimensional unbinned fit in mass and decay length
  for the non-prompt fraction and the b-hadron lifetime, five plots, and the
  histograms and toy pulls written to a ROOT file and read back.

### Changed

- **`#[non_exhaustive]` and `Debug`.** The file, tree and RNTuple writers
  and readers, `Tree`, `Branch`, `Ntuple`, `Column`/`Field`, `FileWriter`,
  `SubdirWriter` and the object records implement `Debug`;
  `CompressError`, `ListKind` and `MergeKind` are `#[non_exhaustive]` (a
  `match` on them outside oxiroot needs a `_` arm).
- **Cast lints.** `cast_possible_truncation`, `cast_sign_loss`,
  `cast_possible_wrap` and `cast_lossless` are on across the workspace. The
  conversions between ROOT's on-disk integer types and `usize` go through
  `oxiroot_io_core::cast::SaturatingFrom`: a value out of range (a negative
  count in a damaged file, a length past `i32::MAX`) clamps to the bound
  instead of wrapping. Same-width reinterpretations are explicit
  (`cast_signed`/`cast_unsigned`), and the bit-level encodings (zigzag,
  half floats, packed coordinates, `TH1F`-style narrowing) say why they
  truncate. ROOT C++ and uproot read everything back (the interop matrix).
- **One implementation of the plain histograms' shared methods.** `Hist1D`,
  `Hist2D` and `Hist3D` had their own copies of `sumw2`, `bin_error`,
  `scale`, `add`, `integral`, the extrema, `effective_entries`, `reset` and
  the content-type accessors; a macro now generates them from one body
  (`family.rs`), as the profiles' `add` already was. Same public API and
  results (iteration order kept, so sums and ties are unchanged); about 500
  lines fewer. The on-disk readers and writers stay per class, as ROOT's
  layouts differ.

### Fixed

- **`Func1D::integral` could hang.** It is built on oxifit-formula's
  `integrate`, whose depth-first bisection recursed about 2⁴⁰ times on a
  narrow peak (a width-0.01 Gaussian on [−1, 1]) or on a `NaN` integrand.
  oxifit 0.4.1 makes it globally adaptive and bounded (at most about 60 000
  evaluations); the integrals checked against ROOT are unchanged. Pins
  oxifit 0.4.1, oxiplot 0.7.1, oxihist 0.9.1.

### Changed

- **oxifit 0.4.0, oxiplot 0.7.0, oxihist 0.9.0.** `oxiroot::fit` gains
  unbinned and extended unbinned likelihood fits on `Events` (with the
  SumW2 correction for weighted events), toy studies (`Toys`, `ToyStudy`)
  and simultaneous fits (`Simultaneous`); the prelude exports `Events`,
  `Simultaneous` and `Toys`. The fitting guide has a section on each.

- **oxifit 0.3.0, oxiplot 0.6.0, oxihist 0.8.0.** `oxiroot::fit` gains the
  `CmaEs` and `LevenbergMarquardt` minimizers, sum models (`sig + bkg`),
  Gaussian parameter constraints (`Model::constrain`), and profile
  likelihoods and 2-D contours (`FitExt::profile`, `FitExt::contour`,
  returning `oxiroot::fit::{Profile, Contour}` or a `FitError`); the prelude
  exports `CmaEs` and `FitError`. `oxiroot::plot` gains `Axes::fit_contour`
  and `Axes::fit_profile` to draw them. The fitting and plotting guides cover
  all of it.

- **oxifit 0.2.0, oxiplot 0.5.0, oxihist 0.7.0.** `oxiroot::fit` gains the
  global minimizers `Minimizer::MultiStart` and
  `Minimizer::DifferentialEvolution`, `Minuit2Combined` and `Simplex`, the
  `strategy`/`tolerance`/`max_fcn`/`with_hesse` options, and `fval`, `edm` and
  `nfcn` on `FitResult`, which is now `#[non_exhaustive]` (build one with
  `FitResult::new`). HESSE runs after MIGRAD by default, a minimizer panic is
  reported as an invalid fit, `chi2` is `NaN` under a robust loss, and
  formulas accept `erf`, `erfc` and `lgamma`. The fitting guide documents the
  new minimizers.

## [0.4.0] — 2026-11-19

A release about the family around oxiroot and about reading remotely. The
crates that are not about ROOT now live in their own repositories (oxihist,
oxiplot, oxifit, oxistats, oxiparticle) and are re-exported here; remote
`TTree` and RNTuple reads batch their requests; crafted files, decompression
and recursion are bounded, and inputs that used to panic return errors.
Several APIs changed (see **Breaking** below). Files written by 0.3.0 and
0.4.0 read the same in ROOT, uproot and oxiroot.

The crates are not on crates.io yet; depend on them by git, pinned to this tag.

### Added

- Examples `dimuon_analysis` (a NanoAOD-style toy analysis from jagged
  muon branches to a fitted, plotted Z peak) and `open_data_zee` (Z → ee from
  ATLAS open data streamed over `root://`, with the `xrootd` feature).
- `oxiroot_io_core::MAX_STREAMER_DEPTH`, the limit every streamer walker uses,
  and a public `StreamerRegistry::from_infos` for hand-built registries.

### Changed

- **oxiplot 0.4.3, oxihist 0.6.3 and oxifit 0.1.2.** What they bring to
  `oxiroot::plot`, `Hist` and `oxiroot::fit`:
  - plotting: log axes (`Axes::xscale`/`yscale`), a legend that keeps off
    the data and the fit-statistics box (`legend()`, or `legend_loc` to pin
    it), stacked histograms (`hist_stack`), labelled category axes, figures
    laid out around their labels (no more overlapping titles and axis
    labels in grids), PDF pages and SVGs sized in points, large heatmaps
    drawn as one image, and fonts read with skrifa instead of the
    unmaintained ttf-parser (the RUSTSEC-2026-0192 ignore is gone); fixed:
    limits set before plotting were overwritten, negative bins were cut
    off, NaN or infinite data produced invalid output;
  - histograms: numeric fills 2–3× faster, on par with `Hist1D` (the `fill`
    bench: 185 → 762 M fills/s), category fills 2–3× faster, and
    deserialised histograms validated;
  - fitting: the Nelder–Mead fallback reports `valid` only when it
    converged.
- **Remote reads take fewer round trips.** Reading a `TTree` branch or an
  RNTuple column over `root://` or HTTP used to fetch each basket or page
  with its own request. The readers now announce their ranges
  (`ByteSource::plan_reads`, `FileReader::plan_reads`) and the file fetches
  them in batches of up to 1024 ranges or 32 MiB: one XRootD vector read
  (`kXR_readv`) per batch, or up to eight concurrent HTTP range requests.
  Reading 3 million entries of a branch from `root://eospublic.cern.ch` took
  about 30 % less time; the gain grows with the network's latency. Local and
  in-memory files are unchanged. `ByteSource` gains `read_many`,
  `batches_reads` and `plan_reads`, all with defaults, so existing sources
  keep working.
- **Sampling shares oxihist's `BinnedSampler`.** `Hist1D::fill_random` and
  `fill_random_fn` used their own copy of the inverse-CDF code; they now use
  oxihist 0.6.2's `BinnedSampler`, as `get_random` and `Func1D::get_random`
  already used its `Random::sample_binned`. The draws are unchanged.
- **Rust 1.100 and edition 2024.** The minimum supported Rust version
  (`rust-version`) moves from 1.95 to 1.100, the current stable release that
  CI already builds with, and the crates move to the 2024 edition (migrated
  with `cargo fix --edition`; formatting follows the 2024 style).
- **Breaking: one `ThreadedHist` for oxihist and oxiroot.** `ThreadedHist`,
  `Mergeable` and `fill_par` are now oxihist's (oxihist 0.3.0), re-exported
  here; the copy in oxiroot-hist is gone. oxiroot implements `Mergeable` for
  its ROOT types with `oxiroot::file::Error` as the error, so `merge` and
  `merge_all` work as before. `ThreadedHist<Hist1D>::fill` and friends now
  come from four traits, `ThreadedFill1` to `ThreadedFill4` (by the number of
  values a fill takes), which are in the prelude; code that imports
  `oxiroot_hist::ThreadedHist` directly also imports the trait it fills
  through.
- **Breaking: `Parameter` is no longer `Mergeable`** (it is not a
  histogram, and the trait now lives in oxihist). `Parameter::add` sums two
  values, which is what `hadd` still does.
- The oxihep dependencies move to oxifit/oxifit-formula v0.1.1, oxiplot
  v0.1.1 and oxihist v0.2.0.
- `ToRoot` gains `try_to_root` and `try_to_root_as`, which return an error
  for a histogram ROOT cannot hold (an axis of more than `i32::MAX` bins, or
  an axis kind it does not know, since oxihist's `AxisKind` is now
  non-exhaustive). `to_root` and `to_root_as` keep their signatures and panic
  in those cases; `write_root` returns the error.
- **Breaking: `FileReader::open_mmap` is `unsafe`, and `mmap` is no longer a
  default feature of `oxiroot`.** Its contract (the file must not change while
  mapped, or the result is undefined behaviour) was only in prose, on a safe
  function. Call it as `unsafe { FileReader::open_mmap(path) }` with the
  `mmap` feature enabled, or use `FileReader::open_ranged`, which reads lazily
  with no such contract.
- **Breaking: `oxiroot::Error` is the facade's own error type**, with variants
  `Root` (the ROOT-side `oxiroot::file::Error`), `Stat`, `Formula`, `Hist` and
  `Plot`. `?` still works across the whole API in a function returning
  `oxiroot::Result`. Code that matched ROOT-side variants on `oxiroot::Error`
  matches `oxiroot::file::Error` instead (or `Error::Root(..)` /
  `err.as_root()`), and plotting errors keep their own structure
  (`Error::Plot(oxiplot::Error::..)`) instead of a message.
- **Breaking: `oxiroot-io-core` no longer depends on oxistats, oxifit-formula
  or oxiplot.** Its `stat`, `formula` and `plot` features and its `Stat`,
  `Formula` and `Plot` error variants are gone; a new `Error::Other` (built with
  `Error::other`) carries an error from another crate whole.
- **Breaking: the `Histogram` trait is now `RootHistogram`**, and the prelude's
  `Histogram` is oxihist's histogram (what the `Hist` builder makes).
  `ThreadedHist` and `Mergeable` accept oxihist histograms too, so
  `ThreadedHist::new(Hist::reg(..).int64())` works from the prelude.
- Codec crates upgraded: ruzstd 0.9.1 (fixing RUSTSEC-2026-0344, a
  decompression bomb inside ruzstd), lz4_flex 0.14, lzma-rust2 0.25. Files
  written with each codec read back in ROOT 6.40 unchanged.
- **Histograms are built with [oxihist](https://github.com/oxihep/oxihist)**, a new
  ROOT-free histogram library modelled on boost-histogram and scikit-hep `hist`
  (category and boolean axes, `Double`/`Int64`/`Weight`/`Mean`/`WeightedMean`
  storages, projections, rebinning). oxiroot converts its histograms to and from
  ROOT's, the way uproot does for `hist`: `ToRoot::to_root()` /
  `to_root_as(BinContentType)` / `write_root`, and `Hist1D::to_hist()` /
  `FromRoot::read_root` for the way back (all six of `Hist1D`/`2D`/`3D` and
  `Profile1D`/`2D`/`3D`).

  **Breaking:** oxiroot's own `Hist` builder is gone; `Hist` is now oxihist's.
  Add `.to_root()` after the storage: `Hist::reg(50, 0.0, 100.0).weight()` →
  `Hist::reg(50, 0.0, 100.0).weight().to_root()`. `.profile()` is now
  `.mean().to_root()`, and `.float()`/`.int32()`/`.int16()`/`.int8()` are
  `.double().to_root_as(BinContentType::F32)` (`I32`, `I16`, `I8`). Bin counts
  are `usize`. `Build1D`/`Build2D`/`Build3D` are now `Build1`/`Build2`/`Build3`.
  `Random` is oxihist's (same generator, same draws).

- **The crates that are not about ROOT moved to their own repositories** in the
  [oxihep](https://github.com/oxihep) organization, the way Python's Scikit-HEP
  splits uproot, hist, mplhep and particle. Each kept its history:

  | was | is | repository |
  |---|---|---|
  | `oxiroot-stat` | `oxistats` | [oxihep/oxistats](https://github.com/oxihep/oxistats) |
  | `oxiroot-particle` | `oxiparticle` | [oxihep/oxiparticle](https://github.com/oxihep/oxiparticle) |
  | `oxiroot-fit`, `oxiroot-formula` | `oxifit`, `oxifit-formula` | [oxihep/oxifit](https://github.com/oxihep/oxifit) |
  | `oxiroot-plot`, `oxiroot-rex` | `oxiplot`, `oxiplot-tex` | [oxihep/oxiplot](https://github.com/oxihep/oxiplot) |

  Through the facade nothing changes: `oxiroot::stat`, `oxiroot::particle`,
  `oxiroot::fit` and `oxiroot::plot` re-export the new crates, and the `fit`
  and `plot` features work as before. Code that named the old crates directly
  switches to the new names (`oxiroot_stat::` → `oxistats::`, and so on). Until
  the new crates are published they are git dependencies pinned to their
  `v0.1.0` tags.

- **oxiplot no longer depends on oxiroot.** It plots anything that implements
  its `Hist1dData`/`Hist2dData`/`PointData` traits; `oxiroot-hist` now
  implements them for `Hist1D`, `Profile1D`, `Hist2D` and `Graph` behind a new
  `plot` feature, and `From<oxiplot::Error> for oxiroot::Error` moved into
  `oxiroot-io-core` (its new `plot` feature). The facade's `plot` feature turns
  both on. oxiplot also gained `Binned1D`/`Binned2D`/`Points` for plotting plain
  arrays.
- **Types are named as Rust, not as C++: the `T` prefix is gone.** The workspace
  followed ROOT's C++ convention only halfway — the IO layer was already
  idiomatic (`FileReader`, `TreeReader`, `Directory`, `Value`) while the data
  model was not (`TH1`, `TAxis`, `TF1`, `TMatrixD`) — so nothing told a reader
  which spelling a given type used. Every public type now reads as Rust, and
  **every ROOT class name it models is a `#[doc(alias)]` on it**, so the API
  docs' search box still answers `TH1`, `TProfile`, `TGraph`, `TF1`, `TKey`.

  | was | is | | was | is |
  |---|---|---|---|---|
  | `TH1` `TH2` `TH3` | `Hist1D` `Hist2D` `Hist3D` | | `TKey` | `Key` |
  | `TProfile` `TProfile2D` `TProfile3D` | `Profile1D` `Profile2D` `Profile3D` | | `TNamed` | `Named` |
  | `TH2Poly` | `PolyHist` | | `TObjString` | `ObjString` |
  | `THnSparse` | `SparseHist` | | `TMap` | `ObjMap` |
  | `THStack` | `HistStack` | | `TParameter` | `Parameter` |
  | `TMultiGraph` | `GraphStack` | | `TDatime` | `Datime` |
  | `TGraph` `TGraph2D` | `Graph` `Graph2D` | | `TUuid` | `Uuid` |
  | `TGraphMultiErrors` | `MultiErrorGraph` | | `TObjectHeader` | `ObjectBase` |
  | `TEfficiency` | `Efficiency` | | `ObjHeader` | `RecordHeader` |
  | `TAxis` | `Axis` | | `TEntryList` | `EntryList` |
  | `TF1` `TF2` `TF3` | `Func1D` `Func2D` `Func3D` | | `TH1Core` | `HistBase` |
  | `TMatrixD` `TMatrixDSym` | `Matrix` `SymMatrix` | | `H1` `H2` `H3` (builder) | `Build1D` `Build2D` `Build3D` |
  | `TVectorD` | `Vector` | | | |

  `TObjectHeader` (the streamed `TObject` base) and `ObjHeader` (a record's
  byte-count/class-tag header) were two different things one letter apart, hence
  the two names on the right. The free functions named after the record they
  handle followed their type: `read_tnamed`/`write_tnamed` are now
  `read_named`/`write_named`, and `read_tobject`/`write_tobject` are
  `read_object_base`/`write_object_base`.

- **Nothing changes on disk.** Class names are read from each file's
  `TStreamerInfo` and were never type names: a `Hist1D` writes a `TH1D`,
  `class_name()` reports the ROOT class an object came from, and the ROOT-C++
  and uproot interop suites pass unchanged. The dynamic `Value` tree likewise
  keeps ROOT's member names verbatim (`fName`, `fBins`, …) — they are data read
  out of the file, not a naming choice.

### Fixed

- The weekly CI run named no bench targets, so `cargo bench -p oxiroot`
  also ran the libtest harness, which rejects criterion's options. It now
  runs `--bench fill --bench read --bench decompress`.
- Math subscripts in plot labels sat lower than the font asks: oxiplot read
  the wrong MATH constant. The oxihep dependencies move to oxiplot v0.2.1
  and oxihist v0.4.1, which fix it.
- **A category axis with no categories yet wrote an inconsistent ROOT
  histogram**: one bin on the axis but contents sized for none, with the
  overflow landing in the in-range bin. It now writes one empty bin and puts
  the overflow in ROOT's overflow cell.
- **`SparseHist::fill` scanned every filled cell.** It now finds the cell
  through a hash index: 160 000 distinct cells take 21 ms instead of 19 s.
- **Reading objects from a subdirectory re-read its key list each time, and
  every object read re-parsed the file's streamer info.** `FileReader` now
  reads each subdirectory once and parses the streamer info once: 3 000
  objects from one subdirectory take 13 ms instead of 425 ms.
- **`ThreadedHist` got slower as threads were added.** Every fill took a
  shared read lock to find the thread's copy. The copies now live in
  thread-local storage (the `thread_local` crate). 24 M fills with 12 threads:
  13.5 s before, 26 ms now.
- **`fill_par` cloned the template once per rayon split.** It now fills a few
  chunks per thread. With an 8 MB template: 528 ms before, 13 ms now.
- **Decompression is bounded by the block's declared size.** zstd, zlib and
  LZMA blocks used to decode in full and be checked afterwards, so a small
  crafted block could expand into gigabytes before being rejected. Decoding
  now stops one byte past the declared size (at most 16 MiB per block), and
  LZ4 uses the decoder entry point that reports a too-small output as an error
  instead of panicking.
- **Recursion driven by file or server data is bounded.** The TTree streamer
  walker now stops at `MAX_STREAMER_DEPTH` (64) base-class levels, like the
  generic object reader already did; a class listing itself as its own base
  used to overflow the stack. The XRootD client honours at most 30 `kXR_wait`
  replies while opening a file (it used to recurse once per reply).
- **XRootD responses are capped.** Control requests may return at most 64 KiB
  and reads at most the length requested, and frame data is read as it arrives
  instead of into a buffer sized by the frame header, so a server announcing
  2 GB cannot make the client allocate it.
- **Crafted files that used to panic or abort now return errors:**
  - a TTree basket key whose `fKeyLen` runs past the probed bytes (an
    out-of-bounds slice);
  - object and version byte counts that end outside the buffer or before the
    header (checked once in `TagReader::read_header` and
    `RBuffer::read_version`, so collections and `TMap` entries no longer slice
    with them);
  - collection and `TMap` entry counts, and string-vector branch counts, that
    reserved memory for every claimed entry before reading any (now clamped to
    what the buffer can hold);
  - an RNTuple deferred column starting at an absurd element index (capped at
    65 536 elements per entry, and allocated fallibly);
  - `Real32Trunc`/`Real32Quant` columns with a bit width outside what ROOT
    writes (10–31 and 1–32).

### Documentation

- A Performance reference page with the criterion numbers for filling,
  reading and (de)compression, and how to run the benches (also in the README
  and the installation page).
- The multithreading guide describes `Mergeable` as it is now: oxihist's
  trait with its own error type, implemented for every mergeable ROOT type
  (not `Parameter`), and shared with oxihist histograms.
- The plotting guide says fonts are read with skrifa.
- Every Rust snippet of the guides and the README now compiles in CI
  (`scripts/check_guide_snippets.py`, 129 blocks on 18 pages). Fixed the
  snippets that had drifted: the `Func1D` import in the plotting guide, the
  error match in the reading guide, the non-exhaustive `GraphErrors` match.
- The crate-layout page lists oxihist; the installation page documents the
  `http` and `xrootd` features; rustdoc no longer names the removed
  `.profile()` and `.int32()` builder calls.
- The bridge's lossy cases (Integer and category axes, growth, axis names,
  `fEntries`) are listed in the `convert` module docs and the histograms
  guide, and covered by tests.

### Testing

- The XRootD mock server answers vector reads (in `kXR_oksofar`
  continuation frames) and plays a misbehaving server: short reads, a
  connection dropped mid-reply, an error status, and vector-read replies at
  the wrong offset, cut short or with extra bytes. Each ends in an error,
  never a panic or wrong values. A live test reads 300 000 entries from EOS
  by vector reads.
- Criterion benchmarks in `crates/oxiroot/benches`: `fill` (ROOT and oxihist
  histograms, `ThreadedHist` on 1 and 4 threads), `read` (TTree flat and
  jagged branches, an RNTuple, 500 histograms by key) and `decompress`
  (compress and decompress 4 MiB under each codec). Run them with
  `cargo bench -p oxiroot`; CI builds them on every PR and runs them weekly,
  keeping criterion's report.
- The ROOT C++ and uproot interop jobs read 2-D and 3-D histograms, a
  profile and a 1000-bin histogram written under each of zlib, LZ4, Zstd and
  LZMA. The ROOT job uses a pinned image (6.38.00) on PRs.
- CI runs on macOS too; a weekly run tests the latest stable Rust and ROOT
  and fuzzes every target for two minutes with a 2 GB memory limit. Jobs
  get a read-only token, and a new push cancels a PR's previous run.

## [0.3.0] — 2026-09-24

A release about reading what a ROOT file actually holds, and about finishing the
file container. The generic reader decodes the STL members it used to skip; a
file's objects are reachable at any cycle; directories nest; update mode can
take objects out and give back the space; axes keep their times; and a friend
tree joins on a `BuildIndex` key rather than by entry number. The two column
value types agree on one offset convention, which changes an API. Files written
by 0.2.0 and by 0.3.0 read the same in ROOT, uproot and oxiroot.

### Added

- **The generic reader decodes STL members.** A `std::vector`, `set`, `list`,
  `deque` or `map` holding numbers, strings, `TArray`s, nested containers,
  objects or pointers now decodes, whether ROOT streamed it objectwise or
  memberwise (a column per member), and so does a `TStreamerLoop` array of
  objects. `TF1`'s parameter-name map, `TEfficiency`'s per-bin beta parameters,
  `TGraphMultiErrors`'s error arrays and draw attributes, and `TH2Poly`'s cell
  grid were `Unsupported`; a `TH2Poly` read from a ROOT file now yields its
  bins. The shape comes from the member's C++ type name, and the container's
  byte count says whether that shape was right: a shape read wrong is reported
  as `Unsupported`, never passed off as values.
- `Value::Ref` — a slot holding an object written elsewhere in the same object,
  naming the class it points at. ROOT streams a shared object once and points
  at it from everywhere else it appears (a `TH2Poly`'s `fBins` points at the
  bins its `fCells` grid holds in full); those slots read as `Null` before, as
  if there were no object there. `ObjHeader::back_ref` says where the object
  was written, for a reader that wants to follow it.

- **Index-based friend join.** A tree's `fTreeIndex` — what
  `TTree::BuildIndex(major, minor)` builds — is read into `TreeIndex`, with the
  keys, the entry each names, and `entry_of`, which is ROOT's
  `GetEntryNumberWithIndex`. `TreeReader::join_by_index` gives, for each entry
  of the main tree, the friend entry carrying the same key, so friends whose
  entries do not line up can be joined on `(run, event)` instead of by entry
  number. A friend with no index is an error rather than a silent positional
  join. `BranchValues::as_i64_vec` reads any scalar integer column as `i64`,
  which is what an index key is made of.
- **Delete, purge and compact in update mode.** `FileWriter::open` could only
  add to a file. `delete` takes a name out of it — every cycle, or the one
  `"h;1"` asks for — `purge` keeps only each name's current cycle, as ROOT's
  `TFile::Purge` does, and `compact` rewrites the file from what is left so the
  space the rest held is given up. Deleting without compacting leaves the file
  the same size, which is what ROOT's `TFile::Delete` does; deleting a name the
  file does not hold is an error rather than a quiet no-op; and compacting a
  file that holds a `TTree` or an RNTuple is refused, since their records live
  outside their key.
- **Time axes.** A `TAxis` keeps `fTimeDisplay` and `fTimeFormat`, which it
  used to read and drop, so a histogram or graph whose x values are times keeps
  saying so through oxiroot. `TAxis::set_time_format` sets it in ROOT's own
  spelling (`"%H:%M%F2024-01-01 00:00:00"` — a `strftime` format, then the
  epoch the values count from), and ROOT and uproot read the axis back as a
  time axis. Plotting picks it up: `oxiroot-plot` labels the ticks as dates and
  clock times and steps them in minutes, hours or days rather than decimals,
  with `Axes::x_time_format` for data that carries no axis of its own.
- **Directories nest.** `FileWriter::dir` could only open a directory in the
  top directory; `SubdirWriter::dir` opens one inside a subdirectory, so a file
  can be organised as deep as its data is. A clash in a nested directory is
  reported with the path that names it. ROOT and uproot read three-deep files
  oxiroot writes, which the interop checks now cover.
- **Read an object at an explicit cycle.** Writing a name a file already holds
  keeps the old object under a lower cycle; every read took the highest one and
  had no way to ask for another. A name now takes ROOT's `"name;2"` form
  wherever oxiroot takes one — `FileReader::key` and `get_value`, `read_root`,
  `TreeReader::open`, `NtupleReader::open`, and `oxroot` — and `split_cycle`
  and `find_key` are public for a caller that resolves names itself. A cycle the
  file does not hold is `NotFound`, never a quiet fallback to another one, and a
  name that carries a `;` of its own is left whole. `oxroot ls` names a key
  `name;cycle` when the directory holds more than one cycle of it.

### Changed

- **One offset convention.** An RNTuple's `FieldValues::Nested` and the writer's
  `Column::Nested` and `Column::Assoc` carry their cumulative `offsets` with a
  leading `0`, as a tree's `BranchValues::Nested` and `Jagged` do: element `i`
  spans `items[offsets[i]..offsets[i + 1]]`, and `offsets` holds one value more
  than there are elements. They held one cumulative end per element and no
  leading `0`, so the first element was the one you could not slice like the
  rest, and the two halves of the workspace disagreed. Code that reads or builds
  these offsets adds the leading `0`; the `vec_vec_*`, `set_*` and `map_*`
  constructors do it for you, and a file's bytes are unchanged, since the Index
  column on disk keeps holding one end per element.

### Fixed

- A `TStreamerLoop` element carries its counter member's name, like a
  `TStreamerBasicPointer`; it was parsed without one, so nothing could size the
  array it describes.

## [0.2.0] — 2026-09-23

A release of the follow-ups to the architecture review: errors a caller can act
on, one import path per item, files that describe only the classes they hold,
and a `hadd` that merges everything ROOT's does. Files written by 0.1.0 and by
0.2.0 read the same in ROOT, uproot and oxiroot.

### Added

- **`hadd` merges everything ROOT's does.** `merge_files` sums a `TEfficiency`'s
  passed and total histograms, a `TH2Poly`'s and a `THnSparse`'s bins, a
  `THStack`'s histograms (matched by name) and a `TParameter`'s value, and
  appends the points of `TGraph`, `TGraphErrors` and `TGraphAsymmErrors`, as
  ROOT's `hadd` does; they were copied from the first file. `TF1`/`TF2`/`TF3`,
  `TGraph2D`, `TGraphMultiErrors`, `TMultiGraph`, strings, maps and matrices are
  still copied, which is what ROOT's `hadd` does with them too — it writes one
  key per input, which oxiroot cannot, since it rejects two objects of the same
  name in one directory. The merges are `TEfficiency::add`, `TH2Poly::add`,
  `THnSparse::add`, `TGraph::append` and `TParameter::add`, and `Mergeable` now
  covers them, so the merger cannot be pointed at a type that has no merge.
- `Error::context` puts context in front of an error's message and keeps its
  variant.
- `streamer_gen::StreamerInfoList` parses a stored `TList<TStreamerInfo>` and
  gives the descriptions a set of classes needs, with the classes they depend
  on; `oxiroot_hist::hist_streamer_classes` does so for the histogram family's
  captured list.

### Changed

- **A file describes only the classes it holds.** A histogram-family object
  (histograms, profiles, graphs, `TEfficiency`, `THnSparse`, `TH2Poly`,
  `THStack`, `TMultiGraph`, `TF1`/`TF2`/`TF3`) used to embed the whole
  captured histogram-family streamer info, 38 KB. It now embeds its own class,
  the classes it depends on and the classes of the objects it holds. A file
  holding one histogram, graph or function shrinks from about 38 KB to 6–20
  KB; ROOT and uproot read every object as before.
- **Typed errors.** `Error` gains `NotFound`, `WrongClass`,
  `UnsupportedVersion`, `MissingStreamerInfo`, `ChecksumMismatch`,
  `Unsupported` and `InvalidInput`. The errors that fit them use them instead
  of `Error::Format`, which is left for bytes that break the ROOT format: a
  truncated or corrupt file. Remote (HTTP and XRootD) failures are `Error::Io`,
  with an `ErrorKind` that fits where there is one, such as `NotFound` for an
  HTTP 404. `SchemaChanged` also covers trees and RNTuples being concatenated,
  or the trees of a chain, that do not share a schema. Code that matched
  `Error::Format` for one of these conditions must match the new variant.

- **Each public item has one path.** `oxiroot-io-core`, `oxiroot-plot` and
  `oxiroot-stat` no longer expose their modules next to the root re-exports:
  import from the crate root (`oxiroot_io_core::RBuffer`, not
  `oxiroot_io_core::buffer::RBuffer`; `oxiroot_stat::Normal`, not
  `oxiroot_stat::distributions::Normal`). io-core keeps one public module,
  `streamer_gen`, for the helpers that describe a class's members, whose short
  names read best qualified. The items that were reachable only through a
  module are now at the root: io-core's byte buffers, `MmapSource`,
  `XrootdSource`, `MAGIC`, `BIG_FILE_VERSION` and `read_free`, and plot's
  `TickDir` and `Sides`.
- In the `oxiroot` facade, `oxiroot::file` holds all of io-core, like the
  facade's other per-crate modules, and `oxiroot::buffer` and `oxiroot::error`
  are gone: use `oxiroot::file::RBuffer` and `oxiroot::Error`.

### Removed

- `WriteRoot::streamer_blob` and `WriteInto::streamer_blob`, with
  `StreamerSet`'s serialized list (`add_list`, `list`, `blob`), and
  `oxiroot_hist::hist_streamer_blob`. Describe classes with
  `streamer_classes`, and take them from a captured list with
  `StreamerInfoList` or `hist_streamer_classes`.

### Fixed

- A histogram or profile with bin labels describes `TObjString`, so a reader
  that follows the file's streamer info decodes its labels.
- The captured histogram streamer info describes `THnSparseArrayChunk`,
  `TArrayD` and `TArray`. The script that captures it wrote an empty
  `THnSparse`, which holds no chunk, so a sparse histogram's chunks — and
  every histogram's `TArrayD` base — went undescribed.

### Internal

- The tree reader and writer and the RNTuple writer, which ran to 2,000–2,600
  lines each, are split into modules by concern. The API and the written bytes
  are unchanged.

## [0.1.0] — 2026-09-22

The first release: pure-Rust reading and writing of the CERN ROOT file format,
with no C++/libROOT or Python dependency. Files written by oxiroot open in ROOT
and uproot, and CI checks both directions against ROOT C++ and uproot.

### Crates

Depend on `oxiroot` for everything, or on one crate for a part of it.

| Crate | Provides |
|---|---|
| `oxiroot` | Everything below through one dependency, a prelude, and `hadd`-style file merging |
| `oxiroot-io-core` | The `TFile` container: keys, directories, streamer info, 64-bit files, and reads from memory, disk ranges, mmap, HTTP(S) and XRootD (`FileReader`, `FileWriter`); `TList`/`TMap` collections; a generic reader for any class |
| `oxiroot-compress` | ROOT's compression framing, with pure-Rust Zstd, zlib, LZ4 and LZMA |
| `oxiroot-tree` | `TTree` reading and writing, including split `std::vector<MyStruct>`, a streaming `TreeWriter`, `ChainReader`, friend trees and `TEntryList` |
| `oxiroot-rntuple` | RNTuple reading and writing, including nested collections, schema extension and a streaming `NtupleWriter` |
| `oxiroot-hist` | Histograms (`TH1`/`TH2`/`TH3` in every precision), profiles, graphs, `TEfficiency`, `THnSparse`, `TH2Poly`, `THStack`, `TMultiGraph` |
| `oxiroot-hist-func` | `TF1`/`TF2`/`TF3` |
| `oxiroot-formula` | `TFormula` parsing and evaluation |
| `oxiroot-fit` | Fitting histograms, graphs and points with Minuit2, or Nelder–Mead through argmin |
| `oxiroot-stat` | Special functions, distributions, tests and HEP lineshapes, checked against scipy |
| `oxiroot-linalg` | `TVectorD`, `TMatrixD`, `TMatrixDSym` |
| `oxiroot-particle` | PDG particle data and the Monte Carlo numbering scheme |
| `oxiroot-plot` | Plots to SVG, PNG and PDF, with a matplotlib-like API, the mplhep style and TeX math |
| `oxiroot-rex` | The TeX math layout engine `oxiroot-plot` uses: a trimmed copy of ReX |
| `oxiroot-cli` | `oxroot`, a command-line inspector for ROOT files |

### Naming

A type that reads a file ends in `Reader` (`FileReader`, `TreeReader`,
`NtupleReader`, `ChainReader`), and one that writes a file ends in `Writer`
(`FileWriter`, `TreeWriter`, `NtupleWriter`). ROOT's class names (`TFile`,
`TTree`, `RNTuple`, `TChain`) are doc aliases for them.

### Compatibility

- Rust 1.95 or later.
- Histograms and profiles written by older ROOT releases read, back to the
  first class versions ROOT stores through streamer info.
- Written files are checked with ROOT 6.40 and uproot 5.

### Licence

MIT. `oxiroot-particle` also bundles data under BSD-3-Clause
(`MIT AND BSD-3-Clause`), and `oxiroot-rex` carries the original ReX authors'
copyright notices in its `LICENSE-3rdparty`.

[Unreleased]: https://github.com/oxihep/oxiroot/compare/v0.11.0...HEAD
[0.11.0]: https://github.com/oxihep/oxiroot/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/oxihep/oxiroot/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/oxihep/oxiroot/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/oxihep/oxiroot/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/oxihep/oxiroot/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/oxihep/oxiroot/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/oxihep/oxiroot/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/oxihep/oxiroot/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/oxihep/oxiroot/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/oxihep/oxiroot/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/oxihep/oxiroot/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/oxihep/oxiroot/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/oxihep/oxiroot/releases/tag/v0.1.0
