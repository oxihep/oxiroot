# oxiroot code review

A review of the whole workspace at version 0.9.0 (commit `cf6320d`), against
modern Rust practice, the Rust API Guidelines and `clippy::pedantic`.

| File | What it holds |
|---|---|
| **README.md** (this file) | Scope, verdict, what to fix first, and a phased plan |
| [api.md](api.md) | The public API: where there are several ways to do one thing, and the one way to keep |
| [findings.md](findings.md) | Every finding, crate by crate, with location, reason and fix |

## How to read this

- **Nothing in the repository was changed.** These files are the only output.
- **Findings come from reading the code.** None was reproduced by running a
  failing input. Where a finding depends on how ROOT writes a file, that is
  said next to it.
- **Line numbers are for commit `cf6320d`.** They drift as the code changes.
- Severity: **High** can lose data, panic or exhaust memory on real input;
  **Medium** costs performance, hides errors or makes the code hard to change;
  **Low** is style, naming and small simplifications.

## What was covered

| Crate | Lines | Coverage |
|---|---|---|
| `oxiroot-compress` | 0.9k | Read in full |
| `oxiroot-io-core` | 9.5k | Read in full, except `file/http.rs`, `file/xrootd.rs`, `file/xrootd_tls.rs` (scanned) |
| `oxiroot-tree` | 6.7k | Read in full, except `writer/serialize.rs`, `streamer_gen.rs`, `entrylist.rs` (scanned) |
| `oxiroot-rntuple` | 6.0k | Readers and parsers read; `writer/`, `column.rs`, `anchor.rs`, `streamer.rs`, `merge.rs` scanned with excerpts |
| `oxiroot-linalg` | 0.7k | Read in full |
| `oxiroot-hist-func` | 0.8k | Read in full |
| `oxiroot-hist` | 11.1k | Read in full, except `access.rs` (sampled), `profile2d.rs`, `plot.rs`, the statistics modules and the tails of `write.rs` and `derive.rs` |
| `oxiroot-dataset` | 2.9k | Read in full, except `polars_conv.rs` (scanned) |
| `oxiroot` | 1.0k | Read in full |
| `oxiroot-cli` | 1.6k | Read in full |

Test modules, examples and benches were not reviewed.

## Verdict

The code is careful where it matters most. Parsing of untrusted files is
bounds-checked almost everywhere, counts from the file are clamped before
allocating, checksums are verified, `unsafe` is denied outside one documented
`mmap` call, public types are `#[non_exhaustive]`, and the documentation is
thorough.

The weaknesses are of three kinds:

1. **A handful of real defects**, concentrated in the less common readers
   (graphs, stacks, sparse and multi-error types) that parse ROOT's object
   protocol by hand instead of using the shared `TagReader`.
2. **Too many ways to do one thing.** There are three columnar read APIs, three
   columnar write APIs, two histogram models, two read traits with the same
   method names, six `_in` twin methods, and up to five import paths for one
   type. [api.md](api.md) lists them and picks one each.
3. **Duplication held together by macros.** The value enums multiply element
   type by shape, so the same 25-arm match is written about twenty times;
   `oxiroot-hist` has 20 `macro_rules!`; the framing of an embedded object is
   implemented five times.

`oxiroot-dataset` is the best-designed crate: its flat `Column` and `Batch` are
the model the rest of the workspace should converge on.

## Clippy baseline

`cargo clippy --workspace --all-features --lib --bins -- -W clippy::pedantic -W clippy::nursery`
reports 2,022 warnings. Most are mechanical.

| Lint | Count | Recommendation |
|---|---|---|
| `use_self` | 715 | Enable; `cargo clippy --fix` |
| `missing_const_for_fn` | 263 | Leave off (noisy nursery lint) |
| `must_use_candidate` | 177 | Enable |
| `doc_markdown` | 169 | Enable |
| `missing_errors_doc` | 128 | Enable |
| `suboptimal_flops` | 115 | Leave off; changes numerical results |
| `redundant_pub_crate` | 113 | Leave off |
| `format_push_string`, `map_unwrap_or`, `redundant_closure_for_method_calls`, `missing_panics_doc` | 39 | Enable |

Also worth enabling: `rust.elided_lifetimes_in_paths` (`r: &mut RBuffer`
appears throughout without `<'_>`).

Two `suspicious_operation_groupings` hits (`oxiroot-hist/src/stats.rs:16`,
`compare.rs:95`) were checked and are false positives.

## Fix first: correctness

| # | Severity | Finding | Where |
|---|---|---|---|
| 1 | High | `fXbins` is not validated against `fNbins`; `density`, `rebin` and `slice` then index out of range or hit `.expect` | [axis.rs:162](../crates/oxiroot-hist/src/axis.rs#L162) |
| 2 | High | Five allocations sized by an unclamped count from the file (up to about 150 GiB reserved) | [free.rs:48](../crates/oxiroot-io-core/src/file/free.rs#L48), [parse.rs:205](../crates/oxiroot-tree/src/reader/parse.rs#L205), [parse.rs:238](../crates/oxiroot-tree/src/reader/parse.rs#L238), [collections.rs:133](../crates/oxiroot-hist/src/collections.rs#L133), [multierrorgraph.rs:122](../crates/oxiroot-hist/src/multierrorgraph.rs#L122) |
| 3 | High | A graph's second and later attached functions are dropped when ROOT wrote the file (class back-references are skipped) | [graph.rs:334](../crates/oxiroot-hist/src/graph.rs#L334) |
| 4 | High | Sparse-histogram coordinates wider than 64 bits are misread, and panic or corrupt on write | [sparsehist.rs:156](../crates/oxiroot-hist/src/sparsehist.rs#L156), [sparsehist.rs:293](../crates/oxiroot-hist/src/sparsehist.rs#L293), [write.rs:616](../crates/oxiroot-hist/src/write.rs#L616) |
| 5 | High | `oxroot dump` reports swapped axis sizes for `TH2` and `TH3` | [dump.rs:417](../crates/oxiroot-cli/src/dump.rs#L417), [dump.rs:447](../crates/oxiroot-cli/src/dump.rs#L447) |
| 6 | High | `Hist1D::smooth` panics on a NaN bin (`partial_cmp().unwrap()`) | [sample.rs:86](../crates/oxiroot-hist/src/sample.rs#L86) |
| 7 | Medium | `Batch::append` leaves the batch inconsistent after a type mismatch | [batch.rs:291](../crates/oxiroot-dataset/src/batch.rs#L291) |
| 8 | Medium | Oversized baskets and objects are written corrupt instead of rejected (`saturating_from` on the write side) | [baskets.rs:60](../crates/oxiroot-tree/src/writer/baskets.rs#L60), [writer.rs:225](../crates/oxiroot-io-core/src/buffer/writer.rs#L225) |
| 9 | Medium | Updating a file overwrites it in place; a crash mid-write loses the original | [writer.rs:297](../crates/oxiroot-io-core/src/file/writer.rs#L297) |
| 10 | Medium | `oxroot dump -b <typo>` prints an empty column and exits 0; `u64` values above `i64::MAX` print negative in JSON | [dump.rs:242](../crates/oxiroot-cli/src/dump.rs#L242), [dump.rs:642](../crates/oxiroot-cli/src/dump.rs#L642) |
| 11 | Medium | HTTP cache locks use `.lock().unwrap()`; every other lock recovers from poisoning | [http.rs:204](../crates/oxiroot-io-core/src/file/http.rs#L204) |
| 12 | Medium | Tree members default silently to `0` and `""` when missing from the file's streamer info | [members.rs:19](../crates/oxiroot-tree/src/reader/members.rs#L19) |

Findings 2 and 3 share a root cause: there is no shared helper for "a count
read from the file" or for "an embedded object pointer", so each parser writes
its own. Adding the two helpers (see [findings.md](findings.md#cross-cutting))
fixes both classes and prevents new instances.

## The plan, in phases

Each phase leaves the workspace compiling and tested. Later phases break the
public API, so they belong before 1.0.

### Phase 1: defects (no API change)

1. Validate `fXbins` in `Axis::read`.
2. Add `RBuffer::bounded_count` and use it at the five allocation sites.
3. Read graph functions, efficiency histograms and sparse chunks through
   `TagReader`.
4. Reject sparse coordinates over 64 bits, on read and in `SparseHist::new`.
5. Use `nx()`/`ny()`/`nz()` in the CLI; propagate read errors; fix `u64` JSON.
6. `sort_by(f64::total_cmp)` in `smooth`.
7. Check schemas before mutating in `Batch::append`.
8. Return an error for baskets and objects over the format limits.
9. Write updates to a temporary file and rename.

### Phase 2: mechanical cleanup (no API change)

1. Turn on the curated lint set and run `cargo clippy --fix`.
2. Replace `chunks_exact(..).try_into().unwrap()` with `as_chunks`.
3. Replace `expr_2021` with `expr` in the 14 macros that use it.
4. Fix the four merged doc comments, the stale milestone notes and the
   comments that name renamed types where a ROOT class is meant.
5. Delete exact duplicates (`err2`, `read_basic_array`, `vec_row_lengths`,
   `Branch::n_entries`, the three `check_len`).

### Phase 3: one way per task (API change)

Follow [api.md](api.md). In order of payoff:

1. One path argument instead of the `_in` twins.
2. `file.get::<T>(path)` backed by a one-method `ReadRoot`.
3. One columnar API: `Dataset`, `DatasetWriter`, `Batch`.
4. One options convention: compression and file form as builder options.
5. One error type, one import path per item, a small prelude.
6. Decide the histogram model (see the open decisions in [api.md](api.md#decisions-that-are-yours)).

### Phase 4: structure

1. Move `Column` below the formats so the tree and RNTuple crates read and
   write it directly. This removes `BranchValues`, `FieldValues`, the writer
   `Column`, both `Jagged` structs and about twenty exhaustive matches.
2. `WriteRoot::write_body(&self, &mut WBuffer)` instead of `to_root_bytes`.
3. Types for encodings: `Form` for `big: bool`, `Compression` for the `u32`
   setting, `usize` bin counts, a shared streamer type-code module.
4. Replace the histogram macros with a shared core struct.

## Numbers worth keeping in view

| Measure | Value |
|---|---|
| Public functions in `oxiroot-hist` | 368 |
| `macro_rules!` in `oxiroot-hist` | 20 |
| Exhaustive matches over the value enums (tree + rntuple + cli) | about 24 |
| Typed constructors on `Branch` and `Field` | about 110 |
| `_in` twin methods | 6 public, about 30 private helpers behind them |
| Import paths for `FileWriter` | 5 |
| Implementations of embedded-object framing | 5 |
| `nbins` conversions from `i32` in `oxiroot-hist` | 49 |
