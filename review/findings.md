# Findings, crate by crate

Every finding from the review, with where it is, why it matters and what to do.
Back to the [summary](README.md). The public-API view is in [api.md](api.md).

Paths are relative to `crates/`. Line numbers are for commit `cf6320d`.
Findings were made by reading the code and were not reproduced by running it.

**Contents**

- [Cross-cutting](#cross-cutting)
- [`oxiroot-compress`](#oxiroot-compress)
- [`oxiroot-io-core`](#oxiroot-io-core)
- [`oxiroot-tree`](#oxiroot-tree)
- [`oxiroot-rntuple`](#oxiroot-rntuple)
- [`oxiroot-linalg`](#oxiroot-linalg)
- [`oxiroot-hist-func`](#oxiroot-hist-func)
- [`oxiroot-hist`](#oxiroot-hist)
- [`oxiroot-dataset`](#oxiroot-dataset)
- [`oxiroot` (facade and `hadd`)](#oxiroot-facade-and-hadd)
- [`oxiroot-cli`](#oxiroot-cli)

---

## Cross-cutting

These patterns appear in several crates. Fixing each once removes many
individual findings below.

### C1. Counts from the file used as allocation sizes (High)

Five sites call `Vec::with_capacity(n)` with an unclamped `i32` or `u32` read
from the file. Everywhere else the count is clamped with
`.min(r.remaining())`.

| Site | Element | Worst case |
|---|---|---|
| `oxiroot-io-core/src/file/free.rs:48` | `FreeSegment` | about 64 GiB |
| `oxiroot-tree/src/reader/parse.rs:205` | `Friend` | about 144 GiB |
| `oxiroot-tree/src/reader/parse.rs:238` | `(String, String)` | about 96 GiB |
| `oxiroot-hist/src/collections.rs:133` | `(String, Range)` | about 80 GiB |
| `oxiroot-hist/src/multierrorgraph.rs:122` | `Vec<f64>` | about 48 GiB |

```rust
impl RBuffer<'_> {
    /// A count read from the file, capped at what the remaining bytes could hold.
    pub fn bounded_count(&self, declared: i32, min_item_bytes: usize) -> usize {
        usize::try_from(declared).unwrap_or(0).min(self.remaining() / min_item_bytes.max(1))
    }
}
```

Use it at every `with_capacity` whose argument came from the file, and add a
grep for that pattern to CI.

### C2. Embedded objects parsed by hand (High, root cause of H3)

`TagReader` (`oxiroot-io-core/src/object.rs`) reads
`{byte count}{class tag}{optional class name}`, validates the end offset and
resolves class back-references. Five modules in `oxiroot-hist` do it
themselves, and cannot resolve back-references:

`graph.rs:306-397`, `func_record.rs:124-143`, `efficiency.rs:115-132`,
`sparsehist.rs:234-244`, `axis.rs:318`.

The literals `0xFFFF_FFFF`, `0x3fff_ffff` and `0x4000_0000` appear 40 times in
`oxiroot-hist`; `K_NEW_CLASS_TAG` is re-declared in `oxiroot-tree`
(`reader/parse.rs:165`) and `oxiroot-hist` (`collections.rs:27`).

Add `TagReader::read_ptr(r) -> Result<Option<Pointer>>` and `skip_ptr(r)`,
export the constants once, and delete the copies.

### C3. Writing an embedded object, implemented five times (Medium)

`write_object_any` (`oxiroot-io-core/src/streamer.rs:97`),
`begin_object_any`/`end_object_any` (`oxiroot-io-core/src/streamer_gen.rs:287`),
the same pair again (`oxiroot-tree/src/writer/serialize.rs:50`),
`write_object_ptr` (`oxiroot-hist/src/write.rs:552`) and its two fixed-class
copies (`write.rs:499-519`). Keep one closure-taking form in the core.

### C4. `saturating_from` on the write side (Medium)

`SaturatingFrom` is the right tool for a length read from a damaged file. Used
for a length being written, it turns "too large" into a silently wrong number.

- `oxiroot-io-core/src/buffer/writer.rs:225`: at 1 GiB the byte count already
  has `K_BYTE_COUNT_MASK`'s bit set, so `nbytes | mask` is wrong.
- `oxiroot-tree/src/writer/baskets.rs:60`, `72`: `border` and `obj_len`.
- `oxiroot-tree/src/writer/layout.rs:440`: an unchecked `u32` accumulator.

Writers should return `Error::InvalidInput` when a format limit is exceeded.

### C5. Value enums that multiply type by shape (Medium, structural)

`BranchValues` (25 variants), `FieldValues` (28) and the RNTuple writer's
`Column` enumerate element type × shape. The same exhaustive match is written
about 24 times:

- `oxiroot-tree`: `value.rs:280`, `319`, `366`; `reader/decode.rs:192`;
  `chain.rs:129`; `merge.rs:134`; `writer/baskets.rs:138`;
  `writer/layout.rs:44`, `99`, `167`, `200`, `249`; `writer/branch.rs:290`,
  `309`; `writer/streaming.rs:63`.
- `oxiroot-rntuple`: `field.rs:111`, `164`, `249`, `325`, `376`, `519`;
  `writer/fields.rs:165`.
- `oxiroot-cli`: `dump.rs:556`, `594`, `631`, `664`.
- `oxiroot-dataset`: `read.rs:459`, `542` (conversions between them).

`oxiroot-dataset`'s `Column` already separates the axes (scalars, plus `List`,
`Array`, `Optional`, `Struct` over a child). Move it below the formats and
have the tree and RNTuple crates produce and consume it.

### C6. The same class-name dispatch in four places (Low)

`oxiroot-cli/src/util.rs:122-145`, `oxiroot/src/hadd/histograms.rs`
(`build_object`, `summable_hist_dim`), `oxiroot-dataset/src/read.rs:104-106`,
`oxiroot-io-core/src/file/writer.rs:545`. One `ClassKind::of(class)` in the
core keeps them in step.

### C7. `bool` and positional parameters (Low to Medium)

- `big: bool` through 9- and 10-argument functions, with
  `#[allow(clippy::too_many_arguments)]` on six of them
  (`oxiroot-io-core/src/file/container.rs:985`, `1024`;
  `oxiroot-hist/src/write.rs:1059`; `oxiroot-rntuple/src/writer/envelopes.rs:168`,
  `231`; constructors in `oxiroot-hist`).
- `parallel: bool`, `forward: bool`, `onto_x: bool`, `along_x: bool`,
  `want_max: bool`, `json: bool`, `keep: u8`.

Use a `Form` enum and small parameter structs.

### C8. Doc comments and stale notes (Low)

- **Merged doc comments** (the first line of one item's doc attached to the
  next): `oxiroot-io-core/src/file/container.rs:645`,
  `oxiroot-io-core/src/file/reader.rs:316`, `oxiroot-rntuple/src/reader.rs:745`,
  `oxiroot-hist-func/src/io.rs:107`.
- **Stray line** after a code fence: `oxiroot-io-core/src/object_io.rs:30`.
- **Stale milestone notes**: `oxiroot-io-core/src/file/reader.rs:3-5`,
  `file/free.rs:5-7`, `oxiroot-compress/src/lib.rs:5`.
- **Renamed Rust types where the ROOT class is meant**:
  `oxiroot-hist/src/base.rs:17`, `53-55`, `345`, `365`;
  `oxiroot-hist-func/src/io.rs:83`, `95`;
  `oxiroot-io-core/src/objects/collection.rs:479`. The example at `base.rs:55`
  is false as written: `class_name("Hist1D")` yields `"Hist1DF"`.
- **Contradiction**: `oxiroot-compress/src/setting.rs:18-20` and `lib.rs:162`
  say the Zstd level is ignored; `codec.rs:149` maps it to zrip levels 1 to 4.

### C9. Modern std APIs not yet used (Low)

`rust-version` is 1.100, so these are available.

```rust
// current, in oxiroot-tree/src/reader/decode.rs:383 and seven other sites
for c in bytes.chunks_exact(8) { v.push(f64::from_be_bytes(c.try_into().unwrap())); }
// idiomatic
let v: Vec<f64> = bytes.as_chunks::<8>().0.iter().map(|c| f64::from_be_bytes(*c)).collect();
```

Other sites: `oxiroot-tree/src/reader/decode.rs:413`, `461-469`;
`oxiroot-rntuple/src/page.rs:73`; `envelope.rs` (two); `writer/mod.rs:99`,
`131`; `writer/streaming.rs:255`; `oxiroot-compress/src/codec.rs:103`.

Also: `fill(0.0)` for `iter_mut().for_each(|v| *v = 0.0)`; `cast_signed()` for
the three `zigzag` functions that carry `#[allow(clippy::cast_possible_wrap)]`
(`oxiroot-rntuple/src/page.rs:125-147`); plain `expr` for the 14 macros that
use `expr_2021`.

---

## `oxiroot-compress`

**Medium**

- **`compress` takes a raw `u32` and matches the algorithm twice**
  (`lib.rs:165-208`). The second match needs `unreachable!`. Take `Compression`
  and dispatch once.

  ```rust
  pub fn compress(src: &[u8], compression: Compression) -> Result<Cow<'_, [u8]>, CompressError> {
      let Some(algorithm) = compression.algorithm() else { return Ok(Cow::Borrowed(src)) };
      let encode: fn(&[u8], u8) -> Result<Vec<u8>, CompressError> = match algorithm { /* … */ };
  ```
- **Stored-uncompressed passthrough copies the input** (`lib.rs:93`, `167`).
  Return `Cow<'_, [u8]>`.
- **Each block is decoded into its own `Vec`, then copied** (`lib.rs:124`,
  `137-154`). Decode into the output buffer.

**Low**

- `Compression::None` beside `from_setting -> Option<Compression>`
  (`setting.rs:50`): `Some(Compression::None)` versus `None`. Use `TryFrom<u32>`.
- Levels are `u32` in `Compression` and `u8` in `split_settings`; `Compression`
  lacks `#[non_exhaustive]`.
- `zlib_encode` clamps its level; `lzma_encode` (`codec.rs:183`) does not.
- `BlockHeader` has `pub u32` fields holding 24-bit values; `write` truncates
  silently (`header.rs:117-134`).
- `CompressError::Codec(String)` flattens the source; `Algorithm` lacks `Hash`
  and `Display`.

---

## `oxiroot-io-core`

### `file/`

**High**

- **Unbounded allocation** at `free.rs:48`. See [C1](#c1-counts-from-the-file-used-as-allocation-sizes-high).

**Medium**

- **The cached streamer registry is deep-cloned on every call**
  (`reader.rs:289-298`). The field comment says it is cached because every
  object read needs it. Return `Result<&StreamerRegistry>`; `OnceLock` already
  hands out a reference. `subdir()` (`reader.rs:184`) likewise clones a
  `Directory` it holds in an `Arc`.
- **`big: bool` and 9 to 10 positional arguments** (`container.rs:985-1053`).
  See [C7](#c7-bool-and-positional-parameters-low-to-medium).

  ```rust
  #[derive(Clone, Copy, PartialEq, Eq)]
  enum Form { Small, Big }
  struct KeySpec<'a> { class: &'a str, name: &'a str, title: &'a str,
                       obj_len: u32, payload_len: u32, seek_key: u64, seek_pdir: u64, cycle: u16 }
  fn write_key_header(w: &mut WBuffer, key: &KeySpec<'_>, form: Form)
  ```
  The patch offsets (`record + 10`, `34`/`26`, the tables at
  `container.rs:782-784`) become named constants on `Form`.
- **Appending holds up to three copies of the file** (`writer.rs:427`,
  `container.rs:874`, `884`). `from_bytes(existing.to_vec())` copies to parse a
  header. Take `impl Into<Bytes>`.
- **`apply_deletions` clones every key and allocates a `String` per lookup**
  (`writer.rs:334-342`). Use `HashSet<(&str, u16)>` borrowing the local keys.
- **Update overwrites in place** (`writer.rs:297`). Write to a temporary file
  and rename.
- **`lock().unwrap()`** in `http.rs:204`, `211`; every other lock uses
  `unwrap_or_else(PoisonError::into_inner)`.
- **Checked payload length written three times** (`reader.rs:378`, `413`;
  `key.rs:187`), while `Key::payload_len` saturates and the public
  `Key::payload_range` is documented as unvalidated. Add
  `checked_payload_len()` and remove `payload_range` from the public API.

**Low**

- `Directory::datime_c`/`datime_m` are raw `u32` though `Datime` exists.
- `while (w.len() as u64) < BEGIN { w.u8(0) }` (`container.rs:205`, `822`) is a
  `resize`.
- Manual `Debug` impls omit fields (`reader.rs:55`, `http.rs:33`, `xrootd.rs:600`).

### `buffer/`

**Medium**

- **`RBuffer::len()` is the total and `is_empty()` means "cursor at end"**
  (`reader.rs:61-68`). Rename to `is_at_end()`.
- **Patch handles are `Copy` and checked at run time** (`writer.rs:13-24`,
  `202-211`). `CountToken` is documented as consumed by `end_object` but can be
  reused; a width mismatch is an `assert_eq!`.

  ```rust
  #[must_use = "a reserved region must be patched"]
  pub struct Patch<const N: usize> { offset: usize }
  pub fn patch_be_u32(&mut self, p: Patch<4>, v: u32);
  ```
- **`end_object` cannot report an oversized object.** See
  [C4](#c4-saturating_from-on-the-write-side-medium).

**Low**

- About ten sites allocate a `String` to discard it (`let _name = r.string()?`).
  Add `skip_string()` and a zero-copy `str() -> Result<&'a str>`.
- `array::<N>()` is private; `header.rs:66-68`, `93-94` copy by hand.
- `WBuffer` lacks `le_i16` and `le_i32`, which the reader has.
- `patch_offset(&self, p)` belongs on `Patch`.

### Object framework and streamers

**Medium**

- **`expect("checked")` after re-running the check** (`read_object.rs:182-183`,
  `302-303`, `373`). Bind once: `if let Some(base) = tarray_elem(class)`.
- **`unreachable!` from matching `el.kind` twice** (`streamer_gen.rs:319-336`).
  Use one match with an early return for the `Stored` case.
- **Streamer type codes are magic numbers on the read side**
  (`read_object.rs:230-271`, `443-461`) while `streamer_gen.rs:27-40` names
  them. `element_class: String` is compared to `"TStreamerBase"` in four
  places. Parse into an `ElementKind` enum once.
- **`StreamerRegistry::get`/`get_at` are linear scans** (`streamer_info.rs:80-93`)
  called per object, base and element. Index by class name.
- **`TagReader` allocates a `String` per object header** (`object.rs:143-148`,
  `157`, `177`). Use `Rc<str>`. `read_cstring` pushes byte by byte.
- **`WriteRoot::root_class(&self) -> String`** (`object_io.rs:48`). Nearly every
  implementation returns a literal; use `Cow<'static, str>`.
- **`ReadRoot` has two required methods**, so every type writes four helper
  functions. See [api.md, section 3](api.md#3-reading-an-object).
- **Tuple-typed members with empty-string sentinels**
  (`objects/collection.rs:51`, `349`, `508-512`). Use `struct Member` and
  `Option<Member>`.

**Low**

- `object_bytes_any` and `object_bytes_any_keyed` have the same body
  (`object_io.rs:211-233`).
- `#[allow(clippy::type_complexity)]` at `streamer_info.rs:202`, directly below
  the alias for the same tuple.
- `decode_objlist` re-derives the kind `read_members` computed
  (`collection.rs:275-279`).
- `Value`'s `Display` builds a `String` with `push_str(&format!(…))`
  (`value.rs:217-247`); write into the `Formatter`.
- `RecordHeader`'s manual `Debug` omits `back_ref` (`object.rs:184-191`).
- `collect_stored` keeps `seen` as a `Vec` and scans it (`streamer_gen.rs:443`).

### `error.rs`

**Medium**

- **The workspace's error lives in the bottom crate.** `BinningMismatch`,
  `TypeMismatch` and `SchemaChanged` belong to higher layers.
- **`Clone + PartialEq + Eq` cost information.** `Io` is flattened to a string;
  `External` compares by `to_string()` (`error.rs:178-182`).

**Low**

- Empty-string sentinels: `WrongClass::name`, `Decompress::context`.
- `Error::context` silently does nothing for most variants (`error.rs:293-306`).

---

## `oxiroot-tree`

### Reader

**High**

- **Unbounded allocations** at `reader/parse.rs:205`, `238`. See
  [C1](#c1-counts-from-the-file-used-as-allocation-sizes-high).

**Medium**

- **`concat_values` re-implements `BranchValues::append`** (`chain.rs:102-185`
  against `value.rs:366-418`) and collects every part first.

  ```rust
  let mut trees = self.chain.trees.iter();
  let mut acc = trees.next().ok_or_else(no_trees)?.branch(self.name).values()?;
  for tree in trees { acc.append(tree.branch(self.name).values()?)?; }
  ```
- **Captured members default silently** (`reader/members.rs:19-47`).
  `member_int` returns `0` and `member_str` returns `""`, so a schema without
  `fEntries` gives a tree of zero entries (`parse.rs:107`). Make required
  members fallible.
- **Every basket is copied before decoding** (`basket.rs:104-116`). `head` is a
  `Bytes`; `head.slice(..)` is free. `basket.rs:60-83` also re-parses the key
  header by hand though `Key::read` exists.
- **`Branch` is a bag of correlated flags** (`reader/mod.rs:177-221`), and
  `BranchMetaLite` exists to turn them back into a kind. One `enum Layout`
  removes four 15-field struct literals (`parse.rs:395`, `423`, `542`, `610`).
- **Per-leaf clones of shared data** (`parse.rs:429-431`, `616-618`, `626`).
  Share with `Arc<[u64]>`.
- **Anonymous tuples as return types**: `select_baskets` (`mod.rs:601`),
  `basket_locators` (`parse.rs:326`), `read_tbranch_base` (`parse.rs:296`),
  and `diag: &mut Vec<(String, String)>` through six functions.
- **`decode_object_members` re-walks every object once per member column**
  (`decode.rs:273-297`), inserting all members into a `HashMap` each time.

**Low**

- Seven `branch_*(name)` accessors each do a linear lookup (`mod.rs:438-518`).
- `branch_names`, `unsupported_branches`, `streamer_classes`,
  `collection_members` return `Vec` where an iterator would do.
- `as_i64_vec` allocates (`value.rs:343`); `as_str` returns `Option<&[String]>`.
- Empty-string sentinels: `Friend::file_name`, `TreeIndex::minor_name`,
  `walk_members(…, stop_after: "")`.
- `header.class_name.clone()` at `parse.rs:650`; `as_deref()` suffices. Same at
  `oxiroot-io-core/src/streamer_info.rs:216`.
- Repeated code: the leaflist region loop (`decode.rs:16-29`, `122-133`); the
  bracket scanner (`parse.rs:686-713`); a third copy of the type-code tables
  (`members.rs:52-61`, `126-153`; `types.rs:11-33`).

### Writer

**Medium**

- **Metadata carried by fabricating values** (`writer/mod.rs:53-56`,
  `writer/streaming.rs:63-96`). `str_rep` builds a string of NULs so a later
  `len()` gives `fLen`; `row_of` and `split_rep` build rows of zeros. Use
  `struct LeafMeta { len: i32, max: i64 }`.
- **A split branch stores an unused dummy payload** (`writer/branch.rs:277-285`).
  Put the payload in the kind.
- **"Impossible" arms return plausible wrong values**: `member_type_info`
  returns `(0, "", 0)` (`branch.rs:304`); `stl_info` returns
  `("vector<float>", 0)` (`layout.rs:305-307`).
- **Oversized baskets written corrupt.** See
  [C4](#c4-saturating_from-on-the-write-side-medium).
- **Extra copies**: `place_basket` copies the payload into a second buffer
  (`baskets.rs:118-119`); `chunk_values` copies every chunk (`baskets.rs:138-187`).

**Low**

- `Branch::n_entries` (`layout.rs:35-72`) is `BranchValues::len`;
  `vec_row_lengths` (`branch.rs:309`) is `row_lengths` (`layout.rs:242`).
- `total_entries: i64` with `u64::try_from(…).unwrap_or(0)` (`streaming.rs:261`).
- Dropping a writer without `finish()` leaves an invalid file; add `#[must_use]`.
- Manual index `counts[ci]; ci += 1` (`writer/mod.rs:117-124`).

---

## `oxiroot-rntuple`

**Medium**

- **The whole reader is cloned to change one flag or one range**
  (`reader.rs:459-471`, used by `FieldRead::reader` and both ranged reads).
  `cluster_window` clones the header, footer, both raw envelope buffers and
  every `PageInfo`. Pass a small context instead.

  ```rust
  struct ReadCtx { clusters: Range<usize>, parallel: bool }
  fn read_top(&self, ctx: &ReadCtx, name: &str) -> Result<FieldValues>
  ```
  This also removes the `file: &FileReader` parameter threaded beside `self.file`.
- **`read_blob` replaces the real error** (near `reader.rs:795`):
  `.map_err(|_| Error::Format("… runs past end of file"))` reports a network
  failure as corruption.
- **Schema lookups scan and allocate on every call** (`reader.rs:645-666`).
  Build the adjacency lists once at open.
- **Grouping clones every element** (`field.rs:574-586`, `490-502`), so a
  `std::vector<std::string>` field clones each `String`. `oxiroot-tree`'s
  `split_by_offsets` does it by move.
- **Read and write enums disagree** (`merge.rs:65-103`): `VecU32` and `VecU64`
  can be read but not written, so `append_ntuples` rejects files the reader
  handles.
- **The envelope writer assembles many small `Vec<u8>`s**
  (`writer/envelopes.rs:9-41`). `WBuffer`'s `reserve`/`patch` are documented
  as being for exactly this.

**Low**

- Duplicated window selection in `read_range` and `read_flat_range`
  (`reader.rs:380-454`).
- `offsets.insert(0, 0)` shifts the vector (`reader.rs:351`).
- Variant columns collected in two passes (`reader.rs:609-613`); use `unzip`.
- `read_column` takes six positional arguments (`page.rs:173-180`); take
  `&ColumnDescriptor`.
- `fixed` and `optional` return `Result` but cannot fail (`page.rs:343`,
  `field.rs:545`).
- `parent_field_id as usize` in six places; use a `FieldId` newtype.
- `lower_column` is a 397-line match (`writer/lower.rs:278`).
- `is_late_field` decides by string prefix (`reader.rs:748`).

---

## `oxiroot-linalg`

**Low**

- **Docs name the wrong error variant**: `Matrix::new` and `SymMatrix::new`
  document `Error::Format` (`lib.rs:216`, `356`); the code returns
  `Error::InvalidInput` (`lib.rs:35`).
- **`get(i, j)` panics** (`lib.rs:257`, `397`). Return `Option`; put the
  panicking form on `Index<(usize, usize)>`.
- **`check_len(what: String, …)`** formats before knowing there is an error
  (`lib.rs:221-225`, `361-365`).
- Missing `From<Vec<f64>>`, `AsRef<[f64]>`, `into_elements()`.
- Class names repeated as literals four times each; `streamer_classes(class)`
  returns an empty list for a typo.

---

## `oxiroot-hist-func`

**Medium**

- **`FuncCore` duplicates `GraphFunction` field for field** (`func.rs:12-23`
  against `oxiroot-hist/src/graph.rs:52-74`). Hold a `GraphFunction` plus the
  parsed `Formula`.
- **Parameter access fails silently** (`func.rs:107-127`): `param(i)` returns
  `0.0`, `set_param` ignores a bad index, `set_params` truncates or pads.
- **Six and eight positional `f64`s** (`func.rs:291-299`, `373-383`). Use
  `RangeInclusive<f64>` per axis.

**Low**

- Three structs share methods through `accessors!()` (`func.rs:80-139`); a
  `Func<const D: usize>` needs no macro.
- `with_params(Vec<f64>)` takes ownership and copies from a borrow.
- `new(name: &str, …)` beside `named(impl Into<String>)`.
- Formula errors flattened to strings (`func.rs:27-28`, `62-64`); `Error::other`
  keeps the source.
- `function_classes` re-implements `StreamerSet::add_classes` (`io.rs:113-124`).

---

## `oxiroot-hist`

### High

**H1. `Axis::read` does not validate `fXbins`** (`axis.rs:162-166`). The edge
array is read with no check that it is empty or has `fNbins + 1` strictly
ascending values. Contents are validated against `fNbins`, so such a file loads
and then panics in ordinary calls: `density` (`hist1d.rs:271`), `rebin`
(`derive.rs:64`, `69`), `slice` (`derive.rs:120`), and through
`Axis::variable`'s `.expect` (`axis.rs:61`) in `rebin`, `slice`, `rebin2d`,
`rebin3d`.

```rust
let nbins = usize::try_from(nbins).map_err(|_| Error::Format(format!("TAxis fNbins {nbins}")))?;
if !xbins.is_empty() && (xbins.len() != nbins + 1 || !xbins.windows(2).all(|w| w[0] < w[1])) {
    return Err(Error::Format("TAxis fXbins does not match fNbins".into()));
}
```

**H2. Unbounded allocations** at `collections.rs:133`, `multierrorgraph.rs:122`.
See [C1](#c1-counts-from-the-file-used-as-allocation-sizes-high).

**H3. Later attached functions are dropped** (`graph.rs:334-353`).
`read_function_element` decodes only when the tag is `kNewClassTag`; a class
back-reference sets `class = String::new()` and skips. ROOT writes the class
name once per buffer and back-references it afterwards, so a ROOT-written graph
with two fitted functions should read back with one. oxiroot's own writer
emits a fresh tag each time, so its round-trip tests cannot catch this. Fix
through [C2](#c2-embedded-objects-parsed-by-hand-high-root-cause-of-h3).

**H4. Sparse coordinates assumed to fit 64 bits** (`sparsehist.rs:156-164`,
`293-297`; `write.rs:616-619`). ROOT packs `Σ bits` over all axes into
`fSingleCoordinateSize` bytes, more than eight for seven axes of 1,000 bins.
On read, the high bytes are shifted out and cells get wrong coordinates. On
write, `pack` and `coord >> (8 * k)` shift past 64: a panic in debug builds,
garbage in release. Reject with `Error::Unsupported`, or support wide
coordinates with a byte buffer.

**H5. `smooth()` panics on a NaN bin** (`sample.rs:86`).

```rust
v[..n].sort_by(f64::total_cmp);
```

### Medium

- **Members copied to satisfy an owned-tuple signature** (`from_member.rs:17`,
  `23`, `30`; `collections.rs:284`; `hist1d.rs:339`). `decode_th1` takes
  `(String, Vec<u8>)` and only borrows. Take `(&str, &[u8])`.
- **`in_range_cells` allocates a `Vec` per cell** (`family.rs:22-38`), used by
  `integral`, `maximum`, `minimum`. Use `[usize; 3]`. `base.rs:153-187` is a
  second implementation of the same walk.
- **`nbins` is an `i32` in memory** (`axis.rs:18`): 49
  `saturating_from(…nbins…)` conversions. Parse to `usize` in `Axis::read`.
- **`ncells` is stored and recomputed** (`hist1d.rs:36`, `family.rs:96`;
  `profile3d.rs:84`). The getter returns `contents.len()`; the writer writes
  the field. Drop the field.
- **Out-of-range access is silent**: `set_bin_content`, `set_bin_error`
  (`access.rs:114-128`), `Axis::set_label` (`axis.rs:244-253`); `bin_center`,
  `bin_width`, `bin_error` return `0.0`.
- **`access.rs` is 1,120 lines of copied accessors** (173 public functions),
  the same eight methods for each of `Hist1D`, `Hist2D`, `Hist3D`.
- **`ToRoot::to_root` panics by default** (`convert.rs:110-126`); one cause is
  an upstream non-exhaustive enum.
- **18 writer functions where the reader has one** (`write.rs:68-110`,
  `348-368` against `base.rs:272`). Add `write_tarray(w, values, kind)`.
- **Reader fragments copied**: `read_basic_array` three times verbatim
  (`graph.rs:401`, `graph2d.rs:62`, `multierrorgraph.rs:109`); the `TGraph`
  base in three places (`graph.rs:278`, `multierrorgraph.rs:143`,
  `polyhist.rs:384`); `Profile3D::read` repeats `Hist3D::read`
  (`profile3d.rs:220-235`, `hist3d.rs:78-93`); `list_member_ranges`
  (`collections.rs:101-150`) is `oxiroot-tree`'s `open_tlist`.
- **`WriteRoot::to_root_bytes` forces ten `*_to_bytes` wrappers** and nested
  buffers (`write.rs:397`, `436`, `477`, `544`, `632`, `830`, `902`, `948`,
  `1025`; `collections.rs:244-249`). Make `write_body(&self, &mut WBuffer)` the
  required method.
- **`write_th1_core` takes 13 positional arguments** (`write.rs:1059-1075`);
  moments are copied field by field (`derive.rs:41-50`, `199-205`, `435-449`).
  A `Copy` struct of moments fixes both.
- **`SparseHist`'s index defends against an edit path that does not exist**
  (`sparsehist.rs:48-85`): the comments call `bins` public; it is
  `pub(crate)` with a read-only accessor. The always-true `PartialEq` exists
  only to keep the derive.
- **Conversion allocates per bin** (`convert.rs:346`); `S::parts(b)` runs three
  times per bin (`482-483`).
- **`HistStack` and `GraphStack` are the same type twice**
  (`collections.rs:158-298`, `305-433`).
- **Sentinels and raw codes**: `PolyHist::fill` returns `0` for "no bin";
  `tsumw2 = -1.0` means "not tracked" (`sparsehist.rs:101`);
  `ErrorMode::from_code` maps unknown codes to `Mean`; `sum_errors_mode` and
  `statistic_option` are public `i32`s.
- **20 `macro_rules!`** generate inherent methods per type. A shared core
  struct lets `scale`, `add`, `integral`, the accessors and `named` be written
  once as ordinary methods.

### Low

- Cell counts in `i32` arithmetic (`hist2d.rs:133`, `hist3d.rs:174`,
  `profile3d.rs:103`); the read side uses the checked `cell_count`.
- `bin_content_type_of(class)` (`base.rs:68`) should be
  `BinContentType::from_code(char)`.
- Indexed loops: `family.rs:170-172`; `ops.rs:43-48`, `62-73`.
- `err2` defined twice (`ops.rs:20`, `family.rs:42`).
- `object_bytes` looks the key up twice (`base.rs:324-327`); six near-identical
  fetch helpers (`base.rs:308-399`).
- `quantiles` scans with `rposition` per probability; use `partition_point`.
- `PolyBin` stores a bounding box derivable from its vertices; `bin(number)`
  scans a sorted list.
- `ThreadedFill1` to `ThreadedFill4` differ only in arity.
- `hist_index`'s doc says it returns `None`; it returns an array
  (`convert.rs:257-259`). `read_bin_content` takes an unused `_ndim`.
- `r.skip(6)` stands in for a version header (`polyhist.rs:249`).
- `smooth_array` is a literal port with `xx`, `yy`, `zz`, `rr`, `hh`.
- Three `#[allow(clippy::needless_range_loop)]` in `derive.rs`.

---

## `oxiroot-dataset`

**Medium**

- **`Batch::append` can leave the batch inconsistent** (`batch.rs:291-297`).
  Columns are extended one at a time; a mismatch at column *k* returns after
  columns `0..k` have grown. `Column::append` does the same inside a struct
  (`column.rs:405-412`). Compare schemas first.
- **`Column::slice` is public and panics on an unvalidated column**
  (`column.rs:264-299`). `Column::List { offsets: vec![], .. }.slice(0, 0)`
  indexes `offsets[0]`.
- **The tree read path treats every error as "shape not supported"**
  (`read.rs:383-396`): `Err(_) =>` retries with `values()`. The RNTuple path
  matches `Err(Error::Unsupported(_))` only (`read.rs:536`).
- **Writing copies each batch about four times** (`write.rs:174-246`): cloned
  into the buffer, sliced per chunk, re-nested by `tree_branch`, then
  serialised. Take `Batch` by value.
- **Flat columns are re-nested to write and nested ones flattened to read**
  (`write.rs:477-494`, `520`, `544-562`, `587`; `read.rs:472-484`, `555-567`).
  See [C5](#c5-value-enums-that-multiply-type-by-shape-medium-structural).
- **Three behaviours for one invariant**: strings are validated on insert
  (`column.rs:216-220`), read with `unwrap_or_default()` (`column.rs:642`) and
  written with `from_utf8_lossy` (`write.rs:488`). Store a `String`.
- **A dropped `DatasetWriter` loses up to 32 MiB** of buffered entries. Add
  `#[must_use]`.

**Low**

- The schema is compared as type-name strings, rebuilt on every `write`
  (`batch.rs:302-306`, `write.rs:175-190`). Add a `DataType` enum.
- `IntoColumn` duplicates `From`.
- `Sink::Gone` and three `unreachable!` (`write.rs:244`, `254`, `296`).
- Four `#[cfg(feature = "rayon")]` pairs with `let _ = parallel;`
  (`read.rs:258`, `375`, `413`, `505`); provide `.parallel()` unconditionally
  in the lower crates.
- `Batches::next` takes and restores `self.cuts` (`read.rs:345-347`);
  `step: 0` is a sentinel.
- `map_scalar!` with two `unreachable!` fallbacks (`column.rs:297`, `344`).
- The field-conversion closure appears three times (`write.rs:235`, `290`, `365`).
- `ntuple_column` in `read.rs:512-523` reaches into the RNTuple header to find
  a top-level field; the RNTuple crate should expose that lookup.

---

## `oxiroot` (facade and `hadd`)

**Medium**

- **`MergeReport` allows states that cannot occur** (`hadd.rs:64-92`).

  ```rust
  pub enum MergeKind {
      Histograms { summed: Vec<String>, copied: Vec<String>, skipped: Vec<Skipped> },
      Tree { name: String, entries: u64 },
      Ntuple { name: String, entries: u64 },
  }
  ```
- **`merge_tree` and `merge_rntuple` are the same 50 lines** (`hadd.rs:313-416`).
- **The histogram merge builds the whole output in memory**, since
  `FileWriter` lays the file out in a `Cursor<Vec<u8>>`, possibly twice.

**Low**

- Local macros `summed!` and `copied!` where a generic function would do.
- `with_key` re-implements `Error::context`.
- `merge_histogram_files(output: &Path, …)` beside
  `merge_files(output: impl AsRef<Path>, …)`.
- `with_form_fallback(large: bool, FnMut(bool))`.
- Facade functions return the core error although the crate exports its own.
- Glob re-exports of whole crates; a prelude of about 80 names.
- `seen.insert(key.name.clone())` clones before checking (`hadd.rs:233`).

---

## `oxiroot-cli`

**High**

- **`dump` swaps axis sizes** (`dump.rs:417-419`, `447-450`).
  `Hist2D::values()` is `values[ix][iy]` (`oxiroot-hist/src/hist2d.rs:117`);
  the CLI reads it as `[iy][ix]`, and for `TH3` exchanges x and z.

  ```rust
  let (nx, ny) = (hist.nx(), hist.ny());
  ```

**Medium**

- **Read errors and mistyped branch names give a blank column and exit 0**
  (`dump.rs:242`, `283`, `311-317`).
- **`u64` above `i64::MAX` prints negative in JSON** (`dump.rs:642`, `654`,
  `675`, `687`); `value_to_json` handles it correctly (`dump.rs:99-101`).
- **`show` reads data to learn a shape** (`show.rs:160-176`);
  `branch_layout` has it from metadata.
- **Four 25-arm matches** (`dump.rs:556-693`). Read through `Dataset` and
  write one function over `Column`.

**Low**

- `num()` prints small values as `0` (`dump.rs:729-738`).
- `json: bool` threaded through every function; five `dump_*` return a
  `Result` they cannot fail.
- `compression_label` decodes by hand (`util.rs:167-178`).
- The binary imports a `#[doc(hidden)]` module of an internal crate
  (`oxiroot_io_core::cast`).
- `run(args: Args, …)` takes ownership it does not need.
- `show_rntuple` finds a field's type by name across all fields, including
  sub-fields (`show.rs:110-114`).
- `Box<dyn Error>` with string errors is adequate here; `anyhow` would add
  context chaining if it is ever wanted.
