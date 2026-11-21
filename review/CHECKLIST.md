# Review checklist

One box per fix. Tick a box only when the change is merged and tested.
Details for each item are in [findings.md](findings.md) (implementation) and
[api.md](api.md) (public API); the section is named in brackets.

Legend: `[ ]` to do, `[x]` done, `[~]` partly done (the note says what is left).

## Phase 1: defects (no API change)

- [x] **P1.1** Validate `fXbins` against `fNbins` in `Axis::read` [hist H1]
- [x] **P1.2** `RBuffer` bounded-count helper, used at all five sites [C1]
  - [x] `oxiroot-io-core/src/file/free.rs:48`
  - [x] `oxiroot-tree/src/reader/parse.rs:205`
  - [x] `oxiroot-tree/src/reader/parse.rs:238`
  - [x] `oxiroot-hist/src/collections.rs:133`
  - [x] `oxiroot-hist/src/multierrorgraph.rs:122`
- [x] **P1.3** Graph functions read through `TagReader`, so class back-references resolve [hist H3]: for a graph under its own key, and for a member of a `TMultiGraph`, `TList`, `TObjArray` or `TMap`, which is read in place in the collection's buffer (`Object::within`). A `TagReader` that meets a reference to a class named inside an object it skipped reads the declaration at the position the reference gives. Fixtures `multigraph_functions.root`, `list_functions.root`.
- [x] **P1.4** Reject sparse-histogram coordinates wider than 64 bits, on read and on fill [hist H4]
- [x] **P1.5** CLI `dump`: correct `TH2`/`TH3` axis sizes [cli High]
- [x] **P1.6** CLI `dump`: report read errors and unknown `-b` names [cli Medium]
- [x] **P1.7** CLI `dump`: `u64` above `i64::MAX` in JSON [cli Medium]
- [x] **P1.8** `Hist1D::smooth` does not panic on NaN [hist H5]
- [x] **P1.9** `Batch::append` checks every column type before changing anything [dataset]
- [x] **P1.10** Baskets over the format limit are an error, not a corrupt file [C4]. No test: one needs a 2 GiB buffer.
- [x] **P1.11** Objects over the byte-count limit are an error [C4]
- [x] **P1.12** `FileWriter::write`, `FileUpdater::write` and `hadd`'s whole-file write go to a temporary file that is renamed (`raw::write_atomic`) [io-core file]. No `fsync`, so this protects against a failed or interrupted write, not power loss.
- [x] **P1.13** HTTP cache locks recover from poisoning [io-core file]
- [x] **P1.14** Regression tests (all but P1.10 and P1.13); 645 tests pass with default features

- [x] **P1.13** (found while closing P1.3) A `TList`, `TObjArray` or `TMap` read from a ROOT file and written back made ROOT crash: members were re-encoded one by one, leaving their internal class references pointing into the old buffer. A collection read from a file now keeps that buffer, is written back from it (new members appended, count and byte count patched), and carries a `KeyCheck` so the writer refuses a key whose header length differs (a rename, or nesting in another collection). Checked by reading the output with ROOT 6.40: the list as read, the list with a member added, and the map with an entry added.

- [x] **P1.14** (found by reading oxiroot's output with ROOT) A `Hist1D` dropped the functions stored with it: a fitted histogram read and written back lost its fit. `Hist1D` now reads and writes `fFunctions` (`functions()`, `with_function`), under its own key and inside a `THStack`, `TList`, `TObjArray` or `TMap`. `Hist2D` / `Hist3D` and the profiles still write an empty list (`TF2` / `TF3` are not modelled on them). Fixture `hist_functions.root`.
- [x] **P1.15** (same) A formula with named parameters (ROOT's `gaus`: `[Constant]`, `[Mean]`, `[Sigma]`) was written back with a positional parameter map, and evaluated to 0 in ROOT. The formula is read into positional form. Checked with ROOT 6.40: `gaus` evaluates the same before and after.

## Phase 2: mechanical cleanup (no API change)

- [x] **P2.1** Curated lint set in `[workspace.lints.clippy]`: `use_self`, `must_use_candidate`, `doc_markdown`, `missing_errors_doc`, `missing_panics_doc`, `format_push_string`, `map_unwrap_or`, `redundant_closure_for_method_calls`. All enabled: the 100 public functions that lacked an `# Errors` section have one.
- [x] **P2.2** `rust.elided_lifetimes_in_paths = "warn"`
- [x] **P2.3** `cargo clippy --fix`, then clear what remains by hand
- [x] **P2.4** `as_chunks` / `first_chunk` / `last_chunk` in place of `chunks_exact(..).try_into().unwrap()` (9 sites) [C9]
- [x] **P2.5** `expr` in place of `expr_2021` (14 macros) [C9]
- [x] **P2.6** `fill(0.0)`, `cast_signed()` for the three `zigzag` functions [C9]
- [x] **P2.7** Four merged doc comments [C8]
- [x] **P2.8** Stray doc line in `object_io.rs:30` [C8]
- [x] **P2.9** Stale milestone notes (3) [C8]
- [x] **P2.10** Comments naming renamed Rust types where a ROOT class is meant (7 sites; `base.rs:55` is false as written) [C8]
- [x] **P2.11** Zstd level documentation agrees with `codec.rs` [C8]
- [x] **P2.12** Delete exact duplicates
  - [x] `err2` (`ops.rs`, `family.rs`)
  - [x] `read_basic_array` (three files)
  - [x] `vec_row_lengths` / `row_lengths`
  - [x] `Branch::n_entries` / `BranchValues::len`
  - [x] `check_len` (linalg, hist, rntuple) One `raw::check_len` for linalg and hist; the RNTuple one builds a different error and stays.
  - [x] `object_bytes_any` / `object_bytes_any_keyed`
- [ ] **P2.13** One `ClassKind::of(class)` replacing the four dispatch tables [C6]
- [x] **P2.14** Export `K_NEW_CLASS_TAG`, `K_CLASS_MASK` once; remove the 40 literals in `oxiroot-hist` [C2]
- [x] **P2.15** One embedded-object writer in the core; delete the four copies [C3]
- [~] **P2.16** All embedded-object readers through `TagReader` (`func_record.rs`, `efficiency.rs`, `sparsehist.rs`, `axis.rs`) [C2] One shared `raw::read_pointer_header` replaces the five hand-written copies. It does not follow class back-references; graphs no longer depend on it inside collections (see P1.3). The other decoders still take a collection member as a detached slice: they skip what they cannot resolve, and none has a known failing case.

## Phase 3: one way per task (API change)

Numbers match the table in [api.md](api.md).

- [x] **A1** One `open`; `open_ranged` and `open_mmap` removed; mmap through `MmapSource::open` and `from_source`; `from_bytes` takes `impl Into<Bytes>`. Measured first (decision D): positioned reads were 5% slower on a column read and 12% slower on 500 small objects, so `open` reads a file up to 64 MiB whole and uses positioned reads above that. `hadd` asks for positioned reads explicitly.
- [x] **A2** One path argument; all `_in` twins removed. `FileReader::key(path)` walks `dir/sub/name;cycle` and returns `Result<Key>`; `raw::object_bytes(file, path)` is the one raw fetch.
  - [x] `ReadRoot::read_root_in`
  - [x] `TreeReader::open_in`
  - [x] `NtupleReader::open_in`
  - [x] `Dataset::open_in`
  - [x] `FileReader::get_value_in`
  - [x] `FileReader::object_in`, `object_in_keyed`
- [x] **A3** `file.get::<T>(path)`; one-method `ReadRoot`; `FromRoot` and `FromMember` removed. `ReadRoot` is `const CLASS` plus `from_object(&Object) -> Option<Result<Self>>`; `Value` implements it, so `get_value` is gone. Differs from api.md in one place: the orphan rule forbids `impl ReadRoot for oxihist::Histogram` in `oxiroot-hist`, so an oxihist histogram is read as `file.get::<Hist1D>(p)?.to_hist()?`.
- [x] **A4** `FileWriter::create(p).add(..).write()`; `write_root` removed; `put` merged into `add`. `add` takes `impl Storable`: `&T` for any `WriteRoot`, a dataset by value. `write_threshold` became the hidden `large_above(n)`.
- [x] **A5** Updating is its own type, `FileUpdater::open(path)`, so `delete`, `purge` and `compact` on a fresh file no longer compile.
- [x] **A6** Columnar read through `Dataset` and `Batch`: `BranchValues`, `FieldValues`, both `Jagged`, `FromBranch`, `FromField`, `BranchRead`, `FieldRead`, `ColumnValues` now live under `oxiroot_tree::raw` / `oxiroot_rntuple::raw`. `Dataset::chain` replaces the tree-only `ChainReader` and works across both formats. `TreeReader::branch()` and `NtupleReader::field()` remain as methods (they return the `raw` types); hadd and the CLI use them.
- [x] **A7** Columnar write through `DatasetWriter` and `Batch`: `TreeWriter`, `NtupleWriter`, `Branch`, `Field`, `SplitMember`, the RNTuple `Column`, `append_trees` and `append_ntuples` are under `raw`. `write` takes the `Batch` by value; `DatasetWriter` is `#[must_use]`; `.large()` takes no argument; `DatasetObject::new(name, format, batch)` is a builder with `.schema(..)`, checked when the file is written.
- [x] **A8** `Merger` only; `merge_files` removed; `merge_histogram_files` private; `MergeKind` carries its data (`Histograms(HistMergeOutcome)`, `Tree { name, entries }`, `Ntuple { name, entries }`). `append_trees` and `append_ntuples` are under `raw`.
- [x] **A9** `Hist1D` / `Hist2D` / `Hist3D` / `Profile*` are I/O records only (decision A). The histogram is oxihist's `Histogram`: built with `Hist`, filled and analysed there, stored with `to_root()?`, read with `file.get::<Hist1D>(path)?.to_hist()?`.
  - Removed from the records: `fill` / `fill_weight`, `track_sumw2`, `scale`, `multiply`, `divide`, `integral`, `reset`, `bin_error`, `set_bin_content`, `set_bin_error`, `contents_mut`, `sumw2_mut`, the statistics (`mean`, `std_dev`, `maximum`, `minimum`, `effective_entries`, `interpolate`, `quantiles`), the derived histograms (`rebin*`, `cumulative`, `projection_*`, `profile_*`, `slice`), sampling, `smooth`, `chi2_test*`, `kolmogorov_test`, the `RootHistogram` trait, `ThreadedFill1`–`4`, and the records' `FitData` / plot-data impls. `derive.rs`, `compare.rs`, `stats.rs`, `sample.rs` are deleted.
  - Kept on the records: the stored fields read-only, `class_name()` / `with_bin_content_type`, `named` / `titled`, `xaxis_mut()` … for axis labels and time formats, `to_hist()`, `Display`, `Mergeable`, and `add(&other, c)`, which `hadd` needs to sum ROOT's own statistics. `Hist1D::fill` survives as `pub(crate)` for `Efficiency`. `Efficiency`, `PolyHist` and `SparseHist` have no oxihist counterpart and stay fillable.
  - The CLI's `dump` takes its statistics from `to_hist()`. Examples, benches, guides, README and changelog are ported; `docs/guide/histograms.md` is rewritten around the histogram / record split.
  - Tests: `derive`, `stats`, `weighted_fill`, `quick_hist`, `threaded`, `analysis` (6 files) and 22 tests in `fit`, `ops`, `profile_merge` covered behaviour that now lives in oxihist and were deleted, not ported; oxihist's own suite covers it. Added `adding_two_records_sums_bins_errors_entries_and_statistics`.
  - Behaviour that differs from the old record API, noted in the changelog: `slice` leaves out a bin that starts at the upper bound; `interpolate` returns `None` outside the axis; equality ignores fill statistics; `.double()` has no Σw² (use `.weight()`); a time format or bin content type is set on the record after `to_root()`.
- [x] **A10** `to_root() -> Result`; the four other `ToRoot` methods removed; `Axis::variable -> Result` (`try_variable` removed).
- [x] **A11** `name` and `title` are private on the twelve records, each with `name()`, `title()`, `named()`, `titled()` (one macro in `naming.rs`). `GraphFunction` and `Axis` keep public fields: they are plain data with no invariant.
- [x] **A12** `contents()` and `values()` only: `Index<usize>` and `IntoIterator for &Hist` removed; `RootHistogram` is gone with A9. `contents()` is the flat form on every record; the nested `values()` of `Hist2D` / `Hist3D` stays as the in-range view. The flat `values()` with `shape()` is oxihist's, where the analysis is.
- [x] **A13** Bin geometry on `Axis` only (`h.xaxis().bin_center(i)`); the forwarding methods on `Hist1D` and `Profile1D` are gone. `Hist2D::find_bin(x, y)` stays: it returns the global cell, which no single axis knows.
- [x] **A14** `scale(c)` only; `Mul` impls removed
- [x] **A15** `Graph::new(x, y)?.with_errors(GraphErrors::..)?` replaces the two positional constructors (named `with_errors`, not `errors`, because `errors()` is already the accessor). `functions` and `histogram` are private, read with `functions()` / `histogram()` and set with `with_function` / `with_histogram`.
- [x] **A16** `Func*`: ranges as `RangeInclusive`; `params_mut()`; `with_params(&[f64]) -> Result`; `set_param` and `set_params` removed
- [x] **A17** `.compression(c)` on every writer, default Zstd 5: `FileWriter`, `FileUpdater`, `DatasetWriter`, `Merger`; `Compression::default()` is Zstd 5. The tree and RNTuple writers are under `raw` since A7 and take it as an argument there.
- [x] **A18** File form automatic for `FileWriter` (the test-only override is the hidden `large_above`) and chosen by `DatasetWriter::large()`; `Form` enum inside. The streaming tree and RNTuple writers under `raw` keep `create_large` / `new_large`: they write as they go and must pick the form before the first byte.
- [~] **A19** One error type at the facade. Kept `oxiroot::Error` (its typed `Stat` / `Formula` / `Plot` / `Fit` variants are matched on by users and tests, which `Error::Other` would lose) and made the facade's own API return it: `Merger::merge -> oxiroot::Result`. `Clone`/`PartialEq` on the core error and the empty-string sentinels are still to do.
- [~] **A20** One import path per item: `oxiroot-hist` and `oxiroot-rntuple` no longer re-export `oxiroot-io-core` types (`FileWriter`, `Compression`, `ObjList`, …). The facade's glob re-exports and the size of the prelude are unchanged.
- [x] **A21** `Dataset::schema()` gives each column's `DataType`, for both formats (decision E). `TreeReader::branch_names()` and `branch_layout(name)` stay for tree-only metadata; a `BranchInfo` iterator would be a second way to the same answer.
- [x] **A22** `ObjList::new(kind)`; one `ObjMap::insert`. `add` stays a builder (`self -> Self`), as on `FileWriter`: one style of adding across the crate, so no `&mut self` form.
- [x] **A23** `Compression` only: `compression_settings` / `split_settings` / `from_setting` removed, `compress` and `compress_if_smaller` take `Compression`, `TryFrom<u32>`; the writers carry `Compression`, not a `u32`.
- [x] **A24** `streamer_registry()` returns `&StreamerRegistry`; the two byte accessors are one `raw::streamer_info_bytes(file)`.

### Out-of-range and sentinel returns (rule 4)

- [x] `PolyHist::set_bin_content` and `Axis::set_label` return a `Result` for a bad index. The records' `set_bin_content` / `set_bin_error` are gone with A9.
- [x] `Axis::bin_center`, `bin_width`, `bin_low_edge` and `Efficiency::efficiency` return `Option`. `bin_error` is gone with A9.
- [x] `Func*::param` returns `Option`
- [x] `PolyHist::fill` and `fill_weight` return `Option`
- [x] `Matrix::get`, `SymMatrix::get` return `Option`; `Index<(usize, usize)>` for the panicking form
- [x] `ErrorMode::from_code` rejects unknown codes
- [x] `sum_errors_mode`, `statistic_option` are enums (`SumErrorsMode`, `StatisticOption`); an unknown code is a format error
- [x] `SparseHist::tsumw2` is `Option<f64>`
- [x] Empty-string sentinels are `Option`: `Friend::file_name`, `TreeIndex::minor_name`, `WrongClass::name`, `Decompress::context`

### Renames

- [x] `Graph::with_errors`, `with_asymm_errors` → `.with_errors(GraphErrors)`
- [x] `MultiErrorGraph::add_y_error` → `with_y_error`
- [x] `as_i64_vec` → `to_i64_vec`. The `as_str -> Option<&[String]>` it was paired with no longer exists.
- [x] `ErrorMode::to_code` → `code`
- [x] `RBuffer::is_empty` → `is_at_end`
- [x] `Compression::None` → `Stored`; `GraphErrors::None` → `Plain`
- [ ] `streamer_gen::{El, Ek, strf, objptr, objanyptr}` spelled out
- [x] `elements` → `as_slice` on `Vector`, `Matrix`, `SymMatrix`, plus `AsRef<[f64]>` and `From<Vec<f64>>` for `Vector`
- [ ] `IntoColumn` → `From<…> for Column`
- [x] `cumulative(bool)`, `project(.., bool)`, `profile(.., bool)`, `project_axis(.., u8)`: gone from the records with A9 (oxihist's API now).
- [x] `projection_*(name)`: gone with A9

### Decisions

- [x] **A** Which type is the histogram: **oxihist's `Histogram`**. `Hist1D` and friends become ROOT I/O records.
- [x] **B** Remove `write_root`: **yes**. Done.
- [x] **C** `TreeReader` / `NtupleReader` public for metadata only: **yes, after the encoding gap is closed** (decision E first).
- [x] **D** Lazy `open` by default: **yes, if the `read` bench shows it is not slower on small files**. It was slower, so `open` picks by size (see A1).
- [x] **E** How `Column` expresses RNTuple-only encodings and C-style `x[n]` branches: **a schema**. `DataType` is the stored type per column (`Half`, `Truncated`, `Quantized`, `CArray`, `Set`, `Map`, `Bitset`, `UniquePtr`, `Atomic`, plus the plain shapes); `Dataset::schema()` reads it, `DatasetWriter::schema(..)` takes it, and a column is stored as its own shape without one. `BranchLayout::CArray` tells a counted array from a `std::vector`. `std::variant` stays under `raw`, as decided. 39 of the 40 fixture datasets report a schema matching what is read; the 40th holds a variant.

## Phase 4: structure

- [ ] **S1** `Column` moved below the formats; `BranchValues`, `FieldValues`, the writer `Column` and both `Jagged` structs removed [C5]
- [ ] **S2** `WriteRoot::write_body(&self, &mut WBuffer)`; the ten `*_to_bytes` wrappers removed
- [~] **S3** `Form` enum (`Small` / `Big`) replaces `big: bool` throughout `container.rs` and in `ContainerWriter::new` / `append`. `KeySpec` and the named patch offsets are not done.
- [ ] **S4** `usize` bin counts in memory; `ncells` field removed
- [ ] **S5** `ElementKind` and a shared streamer type-code module
- [ ] **S6** Shared histogram core struct replacing the per-type macros; `Moments` struct
- [x] **S7** `Patch<const N>` (`w.reserve::<4>()`, `Patch::offset()`), `#[must_use]`, neither it nor `CountToken` is `Copy`
- [ ] **S8** `ReadCtx` in the RNTuple reader instead of `cluster_window` clones
- [ ] **S9** Tree reader `Branch` as an `enum Layout`; `BranchMetaLite` removed
- [ ] **S10** Tree writer `LeafMeta` instead of fabricated representative values

## Remaining findings by crate

Items not already covered by a phase entry above.

### `oxiroot-compress`

- [ ] Passthrough returns `Cow` instead of copying
- [ ] Blocks decoded into the output buffer
- [x] Level type consistent (`u8`); `#[non_exhaustive]` on `Compression`; `Compression::level()`
- [x] `lzma_encode` clamps its level to XZ's 0 to 9
- [x] `BlockHeader` 24-bit fields: `write` asserts them in debug builds (`compress` is the only producer and splits to fit). The assertion found a test whose forged size was truncated to 0.
- [~] `Algorithm: Hash + Display` done. `CompressError::Codec` still holds a message, not the source error: the type is `Clone + Eq`, which io-core's `Error` relies on.

### `oxiroot-io-core`

- [ ] `subdir()` returns the shared `Arc<Directory>`
- [~] `from_bytes(impl Into<Bytes>)` done; the append path still copies the file
- [ ] `apply_deletions` borrows key names
- [ ] One `checked_payload_len`; `payload_range` not public
- [ ] `Directory` times are `Datime`
- [ ] Header padding with `resize`
- [ ] Manual `Debug` impls list their fields
- [ ] `skip_string()` and zero-copy `str()`; `array::<N>()` public
- [ ] `WBuffer::le_i16`, `le_i32`; `Patch::offset`
- [x] `read_object.rs`: three `expect("checked")` removed (`if let` guards)
- [ ] `streamer_gen.rs`: `unreachable!` removed
- [ ] `StreamerRegistry` indexed by class name; `class_names()` an iterator
- [ ] `TagReader` uses `Rc<str>`; `read_cstring` scans for NUL
- [ ] `root_class() -> Cow<'static, str>`
- [ ] `struct Member` and `Option<Member>` in collections
- [~] `Value`'s `Display` writes into the formatter Uses `writeln!` into one `String` now; still not written straight to the formatter.
- [x] `RecordHeader` derives `Debug`, `back_ref` included
- [x] `collect_stored` uses a set
- [ ] Error: histogram, column and schema variants moved or documented
- [ ] Error: `Clone`/`PartialEq` dropped
- [ ] `Error::context` covers every variant or is removed

### `oxiroot-tree`

- [ ] `concat_values` replaced by `BranchValues::append`
- [ ] Required members fallible (`member_int`, `member_str`)
- [ ] Baskets sliced from `Bytes`, not copied; `Key::read` reused
- [ ] Leaflist and object branches share basket vectors
- [ ] Return structs in place of tuples (`select_baskets`, `basket_locators`, `read_tbranch_base`, `diag`)
- [ ] `decode_object_members` walks each object once
- [ ] `Vec`-returning getters become iterators
- [x] `header.class_name.as_deref()` (the clone in `parse.rs`)
- [ ] Leaflist region loop, bracket scanner written once
- [ ] Split branch carries no dummy payload
- [ ] "Impossible" arms are `unreachable!`, not wrong values
- [ ] `place_basket` and `chunk_values` stop copying
- [x] `#[must_use]` on the writers. `total_entries` stays an `i64`: it is ROOT's `Long64_t fEntries`, which the serializers take.

### `oxiroot-rntuple`

- [x] `read_blob` keeps the source error: only a read past the end is a format error
- [ ] Field and column adjacency built once at open
- [ ] `group` and `strings` move instead of cloning
- [ ] `VecU32` / `VecU64` writable, or the gap documented
- [ ] Envelopes written into one `WBuffer`
- [ ] Window selection written once
- [ ] `read_offsets` emits the leading zero
- [ ] Variant columns via `unzip`
- [ ] `read_column(&ColumnDescriptor, ..)`
- [ ] `fixed` and `optional` infallible signatures
- [ ] `FieldId` newtype
- [ ] `lower_column` split up

### `oxiroot-linalg`

- [x] Docs name the error actually returned (`InvalidInput`)
- [x] `check_len` formats lazily
- [x] Class names as constants

### `oxiroot-hist-func`

- [ ] `FuncCore` holds a `GraphFunction`
- [ ] `Func<const D: usize>` in place of the macro
- [ ] Formula errors keep their source
- [ ] `function_classes` uses `StreamerSet`

### `oxiroot-hist`

- [x] `decode_th1/2/3` take `(&str, &[u8])`
- [ ] `in_range_cells` without per-cell allocation; `in_range_sum` merged into it
- [ ] `access.rs` accessors written once
- [ ] `write_tarray(w, values, kind)` replacing 18 functions
- [ ] `TGraph` base read in one place; `Profile3D` reuses the `TH3` base reader
- [ ] `list_member_ranges` shared with the tree and core readers
- [ ] `write_th1_core` takes a struct
- [ ] `SparseHist` index simplified; `PartialEq` hack removed
- [ ] Conversion: no per-bin `Vec`; `parts` evaluated once
- [ ] `HistStack` / `GraphStack` as one generic type
- [x] Cell counts with checked arithmetic (`cell_count`, tested against a forged axis)
- [x] `BinContentType::from_code`
- [ ] Indexed loops rewritten (`family.rs`, `ops.rs`)
- [x] One key lookup per read; fetch helpers merged
- [x] `quantiles`: gone with A9
- [ ] `PolyBin` bounding box derived; `bin(number)` binary search
- [x] One `ThreadedFill` trait: gone with A9 (`ThreadedHist` is oxihist's)
- [x] Stale docs (`hist_index`, `_ndim`): the doc is fixed, the unused argument removed, and the `THnSparse` dimension count is now checked against its axes
- [x] `smooth_array`: gone with A9

### `oxiroot-dataset`

- [x] `Column::slice` cannot panic on an unvalidated column
- [x] Tree read path matches `Error::Unsupported` only; `TreeReader`'s `flat()` returns it, like the RNTuple reader
- [x] `DatasetWriter::write(Batch)` by value
- [ ] String columns stored as `String`
- [x] `#[must_use]` on `DatasetWriter`
- [ ] `Sink` without `Gone` and `unreachable!`
- [ ] `.parallel()` unconditional in the lower crates
- [ ] `Batches` without take-and-restore; `step` as `Option<NonZeroU64>`
- [ ] `map_scalar!` without `unreachable!`
- [ ] Field-conversion closure written once

### `oxiroot`

- [ ] `merge_tree` / `merge_rntuple` as one generic function
- [ ] `summed!` / `copied!` as generic functions
- [x] `with_key` replaced by `Error::context`
- [ ] Histogram merge does not build the whole output in memory

### `oxiroot-cli`

- [ ] `show` uses `branch_layout`, reads no data
- [ ] One cell function over `Column` (after A6)
- [x] `num()` keeps small values (scientific notation below `1e-4`)
- [ ] Output through one render step instead of `json: bool`
- [x] `compression_label` uses `Compression`
- [ ] No import of the hidden `cast` module
- [ ] `show_rntuple` looks up top-level fields only
