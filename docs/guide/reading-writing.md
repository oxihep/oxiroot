# Reading & writing files

Every persistable object — a histogram, a profile, a graph — shares one
persistence model: the `WriteRoot` and `ReadRoot` traits for single objects, and
`FileWriter` for composing several objects, subdirectories, or
appending. There is one way to do each thing, and the files produced open in
official ROOT and uproot.

`TTree` and RNTuple datasets are read and written through one API: a
`Dataset` reads `Batch`es, a `NamedBatch` goes into a `FileWriter` like any
other object, and a `DatasetWriter` streams a dataset too large to hold; see
[Datasets](datasets.md), and [TTree](ttree.md) and
[RNTuple](rntuple.md) for the format-specific layer.

Types that read a file end in `Reader` (`FileReader`, `TreeReader`,
`NtupleReader`) and types that write one end in `Writer` (`FileWriter`,
`FileUpdater` aside, and `DatasetWriter`). A columnar dataset, in either
format, is a `Dataset`.

## One object: `FileWriter::add` / `FileReader::get`

Anything writable implements `WriteRoot`, so `FileWriter::add` stores it; its
`to_root_bytes` is the streamed object payload, without file/key framing.
Anything readable implements `ReadRoot`, so `file.get::<T>(path)` reads it. A
path is `"dir/sub/name;cycle"`: the directories and the cycle are optional.

```rust
use oxiroot::prelude::*;

let mut h = Hist::reg(50, 0.0, 100.0).double().named("pt").titled("p_{T}");
h.fill(42.0);

// `to_root()` is the TH1D a file stores. Write it as a complete ROOT file …
let h = h.to_root()?;
FileWriter::create("hist.root").add(&h).write()?;
// … or get just the streamed object payload (no file/key framing).
let bytes: Vec<u8> = h.to_root_bytes();

// Read it back by key name.
let same = FileReader::open("hist.root")?.get::<Hist1D>("pt")?;
```

!!! note "Reading an older cycle"
    Writing a name a file already holds keeps the old object under a lower
    *cycle*. Every read takes a path — `FileReader::get`, `FileReader::key`,
    `Dataset::open` — and a plain name
    reads the current (highest) cycle. Ask for an older one the way ROOT does,
    with `"name;2"`; a name that carries a `;` of its own is left whole, and a
    cycle the file does not hold is `Error::NotFound` rather than a silent
    fallback. `oxroot ls` shows the cycles a file holds.

!!! note "Names belong to the file, not the object"
    A histogram is just data. It carries a name only when you persist it —
    `.named("pt")` sets the file key, `.titled(...)` the ROOT title. Construct
    with `Hist::reg(nbins, lo, hi).double().to_root()?` and any number of unnamed or same-named
    objects can coexist in memory; the name matters only at write time. See
    [Histograms](histograms.md) for the construction model.

## Any class: the generic reader

The typed readers (`file.get::<Hist1D>(path)`, …) need a Rust model for the class. When you
just want to *inspect* an object — including a class oxiroot has no model for —
`file.get::<Value>(path)` decodes it generically, driven entirely by the file's
`TStreamerInfo`, into a dynamic [`Value`](../api/oxiroot/enum.Value.html) tree:

```rust
use oxiroot::{FileReader, Value};

let f = FileReader::open("hist.root")?;
let h = f.get::<Value>("pt")?; // a TH1D, decoded from streamer info alone

assert_eq!(h.class(), Some("TH1D"));
assert_eq!(h.get("fTitle").and_then(Value::as_str), Some("transverse momentum"));
// Members nest: fXaxis is an Axis object, fArray is the bin-content array.
let nbins = h.get("fXaxis").and_then(|a| a.get("fNbins")).and_then(Value::as_i64);
let bins = h.get("fArray").and_then(Value::as_array);

println!("{h}"); // pretty rootprint-style tree
# Ok::<(), oxiroot::Error>(())
```

`Value` is a tree of primitives, `Str`, `Array`, and `Object { class, members }`
(members keep their on-disk order); accessors are `class()`, `get(name)`,
`as_f64()`/`as_i64()`/`as_str()`/`as_array()`, and `Display` renders the tree.
This is the engine behind `oxroot dump` for classes without a dedicated view.
STL members decode too — `vector`, `set`, `list`, `map` and `pair`, of numbers,
strings, `TArray`s, nested containers, objects or pointers, written objectwise
or memberwise — and a slot pointing at an object written elsewhere in the same
object is `Value::Ref`, naming the class it points at. A member the reader
cannot decode (a class with no streamer info) becomes `Value::Unsupported`
rather than failing the whole object.

## Remote and lazy reads

`FileReader::open` reads the whole file into memory. When you only need a few objects
from a large file — or the file lives on a web server — you can instead read only
the byte ranges each object touches, the way ROOT and uproot do:

```rust
use oxiroot::FileReader;

// Local, positioned reads — never slurps the whole file:
let f = FileReader::open("big.root")?;

// Remote, over HTTP(S) byte-range requests (the `http` feature):
let f = FileReader::open_url("https://example.org/data/big.root")?;

// Remote, over CERN's XRootD protocol (the `xrootd` feature):
let f = FileReader::open_url("root://eospublic.cern.ch//eos/root-eos/hsimple.root")?;

let h = f.get::<oxiroot::hist::Hist1D>("hpx")?; // fetches only that key's bytes
# Ok::<(), oxiroot::Error>(())
```

All return an ordinary [`FileReader`]; every reader (histograms, graphs, `TTree`
branches, RNTuple fields, `get_value`) works unchanged and pulls only what it
reads — a single `TTree` branch fetches just its baskets, an RNTuple field just
its pages. Opening parses only the header, directory, key list, and streamer
info (a few small ranges).

Over a network each range costs a round trip, so a branch's baskets and a
column's pages are fetched in batches rather than one by one: over `root://`
one vector read (`kXR_readv`) carries up to 1024 of them, and over HTTP up to
eight range requests run at once. A batch holds at most 32 MiB, so reading a
large branch keeps memory bounded. Nothing changes in the code that reads.

`open_url` dispatches on the URL scheme:

- `http://` / `https://` — the `http` feature (adds the pure-Rust `ureq`/rustls
  client). The server must honor `Range` requests (`Accept-Ranges: bytes`).
- `root://` — the `xrootd` feature (pure `std::net`, no dependencies). It uses
  the credential-free `unix` security protocol, so it reads world-readable /
  open data from servers that offer it (e.g. `root://eospublic.cern.ch`, CERN's
  anonymous open-data endpoint, which offers `krb5`, `gsi`, `sss` and `unix`).
  A server that offers none of what this client speaks is an error naming what
  it did offer, and why each was skipped.
- `roots://`, and access-controlled data — the `xrootd-tls` feature (adds
  rustls). The connection switches to TLS when the URL is `roots://` or the
  server asks for it, and the client authenticates with a WLCG bearer token
  (the `ztn` protocol) when the server offers it. The token is found the WLCG
  way: `BEARER_TOKEN`, the file `BEARER_TOKEN_FILE` names,
  `$XDG_RUNTIME_DIR/bt_u<uid>`, then `/tmp/bt_u<uid>` (a token file must not
  be readable by others). Server certificates are checked against the Mozilla
  root store plus the grid CAs in `$X509_CERT_DIR` (default
  `/etc/grid-security/certificates`). To pass a token or CA directory
  explicitly, use `XrootdOptions`:

  ```rust,no_run
  use oxiroot::file::{XrootdOptions, XrootdSource};

  let opts = XrootdOptions::new().token(std::fs::read_to_string("token.jwt")?);
  let source = XrootdSource::open_with("roots://eos.example.org//data/run.root", &opts)?;
  let f = FileReader::from_source(Box::new(source))?;
  # Ok::<(), Box<dyn std::error::Error>>(())
  ```

  GSI (X.509 proxy) and Kerberos authentication are not supported.

  The live checks against that endpoint are in the test suite but ignored by
  default, since they need the network and CERN's public servers are slow from
  outside CERN. They read a 966 MB ATLAS open-data sample — its tree's 7.5
  million entries, and one branch over one entry window — in well under a
  second, which is the proof that nothing downloads the file whole:

  ```sh
  cargo test -p oxiroot --features xrootd -- --ignored xrootd_live
  ```

Both features are off by default. The `oxroot` CLI accepts a URL anywhere it
takes a path when built with the matching feature:
`oxroot dump root://eospublic.cern.ch//eos/root-eos/hsimple.root:ntuple -n 5`.

[`FileReader`]: ../api/oxiroot/struct.FileReader.html

## Several objects, subdirectories, appending: `FileWriter`

For more than one object, a `TDirectory`, or appending to an existing file, use
`FileWriter` — the single entry point for file composition. `add`
takes any `&dyn WriteRoot`, `dir` opens a subdirectory, and `write` commits with
a chosen compression.

```rust
let prof = Hist::reg(5, 0.0, 5.0).mean().named("prof").titled("<pt> per region").to_root()?;
let g = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0])?.named("res");

FileWriter::create("out.root")
    .add(&h)                               // any &dyn WriteRoot: hist, profile, graph…
    .add(&g)
    .dir("by_region", |d| {                // a TDirectory
        d.add(&prof)
    })
    .write()?;
```

`dir` nests: call it again inside the closure for a directory within a
directory, as deep as the data is organised.

```rust
FileWriter::create("regions.root")
    .dir("signal", |d| {
        d.add(&h)
            .dir("2018", |d| d.add(&prof))   // signal/2018
            .dir("2017", |d| d.add(&prof))   // signal/2017
    })
    .write()?;
```

Read an object back from a subdirectory by naming its path with `/`:

```rust
let f = FileReader::open("out.root")?;
let p = f.get::<Profile1D>("by_region/prof")?;
let deep = FileReader::open("regions.root")?.get::<Profile1D>("signal/2018/prof")?;
```

### Appending

`FileUpdater::open` reopens an existing file so further objects can be appended in a
second pass (ROOT "update" mode):

<!-- fragment -->
```rust
FileUpdater::open("out.root")?
    .add(&extra)
    .write()?;
```

### Deleting, purging and compacting

A `FileUpdater` also takes objects *out* of a file. `delete` drops a name (or
one cycle of it, `"h;1"`), `purge` keeps only each name's current cycle — ROOT's
`TFile::Purge` — and `compact` rewrites the file from what is left, so the space
the rest held is given up:

```rust
FileUpdater::open("out.root")?
    .delete("scratch")   // every cycle of `scratch`
    .delete("h;1")       // and the first cycle of `h`
    .purge()             // then keep only the newest cycle of each name
    .compact()           // and rewrite the file without the dead space
    .write()?;
```

Without `compact` the file does not shrink: the records stay where they are and
the directory stops listing them, which is what ROOT's `TFile::Delete` does.
Deleting a name the file does not hold is an error, not a quiet no-op. A `TTree`
or an RNTuple writes records outside its own key, so compacting a file that
holds one is refused — merge those with `hadd` instead.

!!! warning "Append limitations"
    Append currently targets files of top-level objects. Updating into a file
    that already contains subdirectories or an RNTuple is rejected rather than
    silently corrupting it. Plain (re)writes with subdirectories via
    `FileWriter::create` are fully supported.

## Choosing compression

`FileWriter::compression` takes a `Compression` value applied to every object's
payload; without it a file is written with Zstd level 5:

| Value | Effect |
|-------|--------|
| `Compression::Stored` | Store uncompressed. |
| `Compression::Zstd(level)` | Zstandard (the modern ROOT default). |
| `Compression::Zlib(level)` | zlib/deflate (older ROOT default). |
| `Compression::Lz4(level)` | LZ4 with ROOT's XXH64 block check. |
| `Compression::Lzma(level)` | LZMA (XZ stream), ROOT's high-ratio codec. |

See [Compression](compression.md) for the full codec matrix and how each is
verified against ROOT and uproot.

## Same-name collisions are loud

ROOT silently shadows objects written under the same key in one directory (only
the last is found on read). oxiroot rejects it instead: writing two objects with
the same name into the same directory is a `DuplicateName` error, and an empty
name is also an error. Different subdirectories are independent namespaces, so
the same name in two directories is fine.

```rust
let a = Hist::reg(10, 0.0, 1.0).double().named("h");
let b = Hist::reg(10, 0.0, 1.0).double().named("h");
let err = FileWriter::create("dup.root").add(&a.to_root()?).add(&b.to_root()?).compression(Compression::Stored).write();
assert!(err.is_err()); // Error::DuplicateName { name: "h", .. }
```

## When a read or write fails

Everything returns `oxiroot::Error`, whose variants say what went wrong, so a
caller can handle a missing object differently from a corrupt file:

| Variant | Means |
| --- | --- |
| `NotFound { what, name }` | No key, subdirectory, branch or field of that name |
| `WrongClass { name, found, expected }` | The key holds another class: a `TH2F` read as a `Hist1D`, say |
| `UnsupportedVersion { class, version }` | A class version oxiroot cannot decode, such as one written by a ROOT release older than streamer info |
| `MissingStreamerInfo { class }` | The file has no `TStreamerInfo` for a class it needs to decode |
| `ChecksumMismatch { what, .. }` | An RNTuple checksum does not match its data: the data is corrupt |
| `Format(message)` | The bytes break the ROOT format: a truncated or corrupt file |
| `Unsupported(message)` | Valid ROOT that oxiroot does not read or write yet |
| `InvalidInput(message)` | An argument cannot be used, such as an unnamed object; nothing was written |
| `LengthMismatch { what, expected, found }` | Inputs that must have the same length do not |
| `Io { kind, message }` | An I/O failure, local or remote, with its `std::io::ErrorKind` |

`Error` is `#[non_exhaustive]`, so match the variants you handle and keep a
wildcard arm:

```rust
use oxiroot::prelude::*;
use oxiroot::file::Error;

let file = FileReader::open("hist.root")?;
match file.get::<Hist1D>("h") {
    Ok(h) => println!("{} entries", h.entries()),
    Err(Error::NotFound { .. }) => println!("no object named h"),
    Err(Error::WrongClass { found, .. }) => println!("h is a {found}, not a 1-D histogram"),
    Err(e) => return Err(e.into()),
}
# Ok::<(), oxiroot::Error>(())
```

## Self-describing output

Written files embed a `TStreamerInfo` list describing every class they contain,
so they are self-describing for any ROOT reader — no external dictionary needed.
That is what lets official ROOT (C++) and uproot read oxiroot's output directly;
see [ROOT / uproot interop](interop.md).

## See also

- [Histograms](histograms.md) — the object model and construction
- [Graphs](graphs.md) — the `Graph` family
- [Compression](compression.md) — codec choices and trade-offs
- [ROOT / uproot interop](interop.md) — cross-language round-trips
