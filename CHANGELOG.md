# Changelog

Notable changes to oxiroot, by release. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): before 1.0, a minor release may
change the API.

## [Unreleased]

### Added

- `oxiroot_io_core::MAX_STREAMER_DEPTH`, the limit every streamer walker uses,
  and a public `StreamerRegistry::from_infos` for hand-built registries.

### Changed

- The oxihep dependencies move to oxiplot v0.2.0 and oxihist v0.4.0. oxiplot
  now reads fonts with skrifa instead of the unmaintained ttf-parser and
  ab_glyph, so the RUSTSEC-2026-0192 advisory ignore is removed. Plots
  render the same.
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
- **`SparseHist::fill` scanned every filled cell.** It now finds the cell
  through a hash index: 160 000 distinct cells take 21 ms instead of 19 s.
- **Reading objects from a subdirectory re-read its key list each time, and
  every object read re-parsed the file's streamer info.** `FileReader` now
  reads each subdirectory once and parses the streamer info once: 3 000
  objects from one subdirectory take 13 ms instead of 425 ms.
- **Decompression is bounded by the block's declared size.** zstd, zlib and
  LZMA blocks used to decode in full and be checked afterwards, so a small
  crafted block could expand into gigabytes before being rejected. Decoding
  now stops one byte past the declared size (at most 16 MiB per block), and
  LZ4 uses the decoder entry point that reports a too-small output as an error
  instead of panicking.
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

### Testing

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

### Documentation

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

[Unreleased]: https://github.com/oxihep/oxiroot/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/oxihep/oxiroot/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/oxihep/oxiroot/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/oxihep/oxiroot/releases/tag/v0.1.0
