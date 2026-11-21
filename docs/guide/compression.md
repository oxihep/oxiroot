# Compression

oxiroot reads *and writes* every compression codec ROOT writes (except the
legacy `CS`), all in pure Rust. This page covers the `Compression` enum that
every writer takes, the read/write codec coverage, and ROOT's on-disk block
framing.

## Choosing compression on write

Every writer — [`FileWriter`](../getting-started/quickstart.md), `FileUpdater`,
`DatasetWriter`, `Merger` — takes a single
[`Compression`](../api/oxiroot/index.html) value through `.compression(…)`, and
uses Zstd level 5 without it. It is re-exported from the
prelude, so `use oxiroot::prelude::*;` is all you need.

```rust
use oxiroot::prelude::*;

let mut h = Hist::reg(50, 0.0, 100.0).double().named("pt").titled("p_{T}");
h.fill(42.0);

// Zstandard at level 5 — ROOT's modern FileWriter::create("zstd.root").add(&default.h).write()?;

// Or store the payload FileWriter::create("plain.root").add(&uncompressed.h).compression(Compression::Stored).write()?;
```

The same value drives multi-object files and RNTuple/TTree writers:

```rust
use oxiroot::prelude::*;

let h = Hist::reg(50, 0.0, 100.0).double().named("pt");
let prof = Hist::reg(5, 0.0, 5.0).mean().named("prof");

FileWriter::create("out.root")
    .add(&h.to_root()?)
    .add(&prof.to_root()?)
    .compression(Compression::Zlib(1)).write()?;   // older-ROOT-style zlib default
```

## The `Compression` enum

`Compression` lives in `oxiroot-io-core` (`oxiroot::Compression`) and is
re-exported through `oxiroot::prelude`. It is `Copy` and defaults to
`Compression::Stored`.

| Variant         | Algorithm     | Level range            | Encode | Decode |
|-----------------|---------------|------------------------|:------:|:------:|
| `None`          | stored as-is  | —                      | yes    | yes    |
| `Zstd(u32)`     | Zstandard     | 1–22 (ROOT default 5)  | yes    | yes    |
| `Zlib(u32)`     | zlib / DEFLATE| 1–9 (ROOT default 1)   | yes    | yes    |
| `Lz4(u32)`      | LZ4           | 1–9                    | yes    | yes    |
| `Lzma(u32)`     | LZMA (XZ)     | 1–9 (ROOT default 5)   | yes    | yes    |

Two helpers are available on the enum:

| Method            | Returns | Meaning                                              |
|-------------------|---------|------------------------------------------------------|
| `setting()`       | `u32`   | ROOT's setting integer (`algorithm*100 + level`, `0` = none) |
| `is_enabled()`    | `bool`  | `false` only for `Compression::Stored`                 |

```rust
use oxiroot::prelude::*;

assert_eq!(Compression::Zstd(5).setting(), 505);
assert_eq!(Compression::Zlib(1).setting(), 101);
assert_eq!(Compression::Stored.setting(), 0);
assert!(!Compression::Stored.is_enabled());
```

!!! note "Level handling differs per backend"
    The level tunes the zlib and LZMA backends (LZMA maps the level straight onto
    the XZ preset). Zstd levels 1–4 are zrip's; higher levels use its level 4,
    which compresses physics data about as well as C zstd's level 5, ROOT's
    default. The LZ4 backend is fast-mode only and ignores the level. The output
    is always valid ROOT framing and reads back correctly in ROOT, uproot, and
    oxiroot.

## Read vs. write coverage

oxiroot decodes **and encodes every codec ROOT writes except the legacy `CS`**.

| Codec        | Block tag | Read   | Write       |
|--------------|-----------|--------|-------------|
| Zstandard    | `ZS`      | yes    | yes         |
| zlib/DEFLATE | `ZL`      | yes    | yes         |
| LZ4          | `L4`      | yes    | yes         |
| LZMA (XZ)    | `XZ`      | yes    | yes         |
| old ROOT     | `CS`      | no     | no          |

Reading is fully automatic: a `Key`'s payload (or an RNTuple page) carries its
own algorithm tag, so the reader picks the codec per block. Uncompressed
payloads — written with `Compression::Stored` or where the compressed form would
not be smaller — pass through directly.

`Compression::Lzma(level)` writes a complete XZ stream per block (block tag
`XZ`), exactly as ROOT's own liblzma path does, so official ROOT and uproot read
it back unchanged.

## On-disk block framing

ROOT stores a compressed payload as a sequence of independently compressed
blocks, each prefixed by a fixed **9-byte header**. A single block carries at
most ~16 MiB (`0xFFFFFF`) of uncompressed data; larger payloads are split across
consecutive blocks and stitched back on read.

The header layout (offsets within the 9 bytes):

| Bytes   | Field                | Notes                                  |
|---------|----------------------|----------------------------------------|
| `[0..2]`| algorithm tag        | two ASCII chars, e.g. `ZS`, `ZL`, `L4` |
| `[2]`   | method / version     | algorithm-specific                     |
| `[3..6]`| compressed size      | 24-bit little-endian                   |
| `[6..9]`| uncompressed size    | 24-bit little-endian                   |

These framing primitives live in the `oxiroot-compress` crate (re-exported as
`oxiroot::compress`): `BlockHeader`, `HDR_SIZE` (`9`), `MAX_CHUNK_SIZE`
(`0xFF_FFFF`), and the `Algorithm` tag enum. Most code never touches them
directly — they are the building blocks `decompress` / `compress` use — but they
are public for low-level inspection.

!!! tip "LZ4 integrity check"
    ROOT prefixes each LZ4 block payload with an 8-byte big-endian **XXH64**
    checksum of the compressed bytes. oxiroot writes that checksum on encode and
    **verifies it on read** — a corrupted LZ4 block is rejected with an error
    rather than silently mis-decoded.

## Pure-Rust backends

All codecs are pure Rust, so the no-libROOT promise holds with no C toolchain:

| Codec        | Crate                                                |
|--------------|------------------------------------------------------|
| Zstandard    | [`zrip`](https://crates.io/crates/zrip)              |
| zlib/DEFLATE | [`miniz_oxide`](https://crates.io/crates/miniz_oxide)|
| LZ4          | [`lz4_flex`](https://crates.io/crates/lz4_flex)      |
| LZMA (XZ)    | [`lzma-rust2`](https://crates.io/crates/lzma-rust2)  |

The LZ4 XXH64 integrity check uses
[`xxhash-rust`](https://crates.io/crates/xxhash-rust), shared with RNTuple's
page hashing.

## Low-level (de)compression

The `oxiroot::compress` module exposes the raw round-trip functions for callers
that hold ROOT block bytes directly. They take the same `Compression` value the writers do.

```rust
use oxiroot::compress::{compress, decompress};
use oxiroot::Compression;

let payload = b"the quick brown fox ".repeat(64);

let blocks = compress(&payload, Compression::Zstd(5))?; // ROOT block framing
let back = decompress(&blocks, payload.len())?;      // needs the original length
assert_eq!(back, payload);
```

| Function                              | Purpose                                            |
|---------------------------------------|----------------------------------------------------|
| `compress(src, compression)`          | Encode into ROOT blocks; `Compression::Stored` stores as-is |
| `decompress(src, uncompressed_len)`   | Decode; passthrough when `src.len() == uncompressed_len` |
| `Compression::setting()`              | ROOT's `algo*100 + level` integer                  |
| `Compression::try_from(setting)`      | The value a setting integer names                  |

`decompress` needs the expected uncompressed length (ROOT stores it in the
enclosing `Key` or RNTuple anchor) and validates the produced size against it,
returning a `CompressError` on any truncation or size mismatch. For normal
file IO you never call these directly — `FileWriter` / `FileReader` apply them
for you from the `Compression` value.

## See also

- [Quick start](../getting-started/quickstart.md) — writing files with a chosen compression
- [Histograms](histograms.md) — writing with `FileWriter`
- [RNTuple](rntuple.md) — per-page compression in the columnar format
- [API reference](../api/oxiroot/index.html) — `Compression` and the `compress` module
