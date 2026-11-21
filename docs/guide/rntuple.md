# RNTuple

RNTuple is ROOT's columnar event-data format (the successor to `TTree`).
oxiroot reads and writes it in pure Rust — anchor, header/footer envelopes,
page lists, and per-column page decoding — with a typed field API that covers
scalars, strings, nested collections, records, variants, fixed-size arrays, and
user classes. 
!!! note "Read and write columns with Datasets"
    [Datasets](datasets.md) is the API for reading and writing columns, the
    same way for an RNTuple and a `TTree`: records, maps and sets,
    reduced-precision reals, optionals and bitsets included, chosen with a
    schema. This page covers the format's own layer underneath,
    `oxiroot::ntuple::raw` (`Field`, `NtupleWriter`): what a merger or an
    inspector works with, along with schema extension. The header and footer
    (`NtupleReader`) are reached from a dataset with `as_ntuple()`.

## Writing fields

An RNTuple is a set of named `Field`s. A field is a
[`Column`](datasets.md), the same flat column a dataset's batch holds, and
how it is stored. `Field::new(name, data)` takes anything that becomes a
column and stores it as its shape says; the type name and the physical column
encoding are derived for you.

```rust
use oxiroot::data::{Column, DataType};
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::{Field, NtupleWriter};

let fields = vec![
    Field::new("mass", vec![91.2, 125.0]),
    Field::new("charge", vec![0, -1]),
];

let mut w = NtupleWriter::create("data.root", "events", Compression::Stored)?;
w.write_batch(&fields)?; // one cluster
w.finish()?;
```

[`NtupleWriter`](#streaming-writes) writes one cluster per `write_batch`.

!!! note
    Every field must have the same number of entries (the first field defines
    the entry count). The entry count is a 32-bit field on disk, so a single
    cluster is capped at `u32::MAX` entries: split a larger dataset across
    several `write_batch` calls.

### What a column is stored as

| Data | Column | On-disk type |
| --- | --- | --- |
| `Vec<bool>`, `Vec<i8>` … `Vec<u64>`, `Vec<f32>`, `Vec<f64>` | scalar | `bool`, `std::int8_t` … `std::uint64_t`, `float`, `double` |
| `Vec<String>`, `Vec<&str>` | string | `std::string` |
| `Vec<Vec<T>>` | list | `std::vector<T>` |
| `Vec<Vec<Vec<T>>>` | list of lists | `std::vector<std::vector<T>>` |
| `Vec<[T; N]>` | array | `std::array<T, N>` |
| `Vec<Option<T>>` | optional | `std::optional<T>` |
| `Column::Struct` with members `_0`, `_1`, … | struct | `std::pair` / `std::tuple` |
| `Column::Struct` with named members | struct | a user class |
| `Column::variant` | union | `std::variant` |

```rust
let fields = vec![
    Field::new("trigger", vec![true, false, true]),
    Field::new("nhits", vec![3_u16, 0, 12]),
    Field::new("label", vec!["mu", "e", "jet"]),
    Field::new("vertex", vec![[0.0, 0.1, 0.2]; 3]),
    Field::new("charge", vec![Some(1_i32), None, Some(-1)]),
];
```

The element type is the data's: `vec![1.0, 2.0]` is a `double` field, and
`vec![1.0_f32, 2.0]` a `float` one.

### Choosing the storage

Some shapes have more than one storage. `.stored_as(data_type)` picks one; the
type must have the data's shape, or the field is refused when it is written.

| `DataType` | Data | Stored as |
| --- | --- | --- |
| `Half` | `f32` | `Real16`: half precision (binary16), about 3 decimal digits |
| `Truncated { bits }` | `f32` | `Real32Trunc`: mantissa cut to `bits` bits in all (`10..=31`) |
| `Quantized { bits, min, max }` | `f32` | `Real32Quant`: `bits`-wide integers across `[min, max]` |
| `Set(item)` | a list | `std::set<T>` |
| `Map(key, value)` | a list of `{_0, _1}` structs | `std::map<K, V>` |
| `UniquePtr(item)` | an optional | `std::unique_ptr<T>` |
| `Atomic(item)` | the bare value | `std::atomic<T>` |
| `Bitset(n)` | an array of `n` `bool`s | `std::bitset<N>` |

```rust
let pts = vec![5.5_f32, 12.25, 88.0];
let fields = vec![
    Field::new("pt_half", pts.clone()).stored_as(DataType::Half),
    Field::new("pt_trunc", pts.clone()).stored_as(DataType::Truncated { bits: 16 }),
    Field::new("pt_quant", pts).stored_as(DataType::Quantized {
        bits: 12,
        min: 0.0,
        max: 100.0,
    }),
    Field::new("ids", vec![vec![1_i32, 2], vec![], vec![7]])
        .stored_as(DataType::Set(Box::new(DataType::I32))),
];
```

!!! warning
    A quantized value outside `[min, max]` is clamped into the range before it
    is encoded.

### Collections

A `std::vector<T>` field takes one inner `Vec<T>` per entry, and a
doubly-nested one a `Vec<Vec<Vec<T>>>` (outer = entries):

```rust
// Per-entry lists of tags, and per-entry hit patterns.
let tags = Field::new("tags", vec![
    vec![],
    vec!["mu".to_string(), "iso".into()],
    vec!["jet".into()],
]);
let hits = Field::new("hits", vec![
    vec![],
    vec![vec![1, 2]],
    vec![vec![3], vec![4, 5]],
]);
```

### Records, nested collections, and variants

A column is flat however deep its type nests, so any shape can be built
directly.

`Column::Struct` is a struct of arrays: named members, each with one value per
record. A struct whose members are named `_0`, `_1`, … is a `std::pair` or
`std::tuple`; any other names make a user class, named after the field unless
`.class_name` says otherwise. `Column::List` is a collection: its cumulative
`offsets` cut the child `values` into entries, entry `i` spanning
`values[offsets[i]..offsets[i + 1]]`, so `offsets` leads with a `0` and holds
one value more than there are entries. Together they express
`std::vector<MyStruct>`:

```rust
// std::vector<std::pair<int32_t, double>>: a vector of records.
let clusters = Field::new(
    "clusters",
    Column::List {
        offsets: vec![0, 0, 1, 3], // entry 0: none, entry 1: 1, entry 2: 2
        values: Box::new(Column::Struct(vec![
            ("_0".into(), Column::I32(vec![10, 20, 21])),
            ("_1".into(), Column::F64(vec![1.5, 2.5, 3.5])),
        ])),
    },
);
```

A `std::variant` is a union column, built with
`Column::variant(alternatives, tags)`. The `alternatives` each hold the values
of the entries that chose them, in entry order; `tags` says which alternative
each entry holds, counted from 1 (`0` means it holds nothing). An alternative
must hold as many values as there are entries choosing it.

```rust
// std::variant<int32_t, double>: entry 0 -> int, entry 1 -> double, entry 2 -> int.
let v = Field::new(
    "payload",
    Column::variant(
        vec![Column::I32(vec![7, 9]), Column::F64(vec![2.5])],
        vec![1, 2, 1],
    ),
);
```

## Streaming writes

`NtupleWriter` writes one *cluster* per call, so a large dataset is never held
in memory all at once. Each `write_batch` flushes a cluster; the first batch
fixes the schema (and writes the header); `finish` writes the page list, footer,
and anchor.

```rust
let mut w = NtupleWriter::create("big.root", "events", Compression::Zstd(5))?;

for chunk in 0..3 {
    let base = chunk * 1000;
    let mass: Vec<f64> = (base..base + 1000).map(|i| i as f64).collect();
    w.write_batch(&[Field::new("mass", mass)])?;
}

let entries: u64 = w.entries();
w.finish()?; // returns the sink
```

| Constructor | Container | Use when |
| --- | --- | --- |
| `NtupleWriter::create(path, name, compression)` | 32-bit | total file `<= 2 GiB` |
| `NtupleWriter::create_large(path, name, compression)` | 64-bit | file may exceed 2 GiB |
| `NtupleWriter::new(sink, file_name, name, compression)` | 32-bit | any `Write + Seek` sink |
| `NtupleWriter::new_large(sink, file_name, name, compression)` | 64-bit | large file into a custom sink |

!!! warning
    Every batch must share the same field schema (field names, types, and
    column layout). A differing batch is rejected with an error rather than
    silently mis-described. A 32-bit writer whose stream passes 2 GiB errors at
    `finish` — use `create_large` / `new_large` for those.

A whole RNTuple can also be placed into an open container, next to other
objects, by `oxiroot::ntuple::raw::write_ntuple`. That path also writes
schema-extension fields: late fields with a first entry.

## Reading

[`NtupleReader::open`](#reading) reads the anchor, parses the schema, and
indexes the clusters; it keeps the file, and column data is decoded on demand.
Read a top-level field with `field(name)`, optionally narrowed with
`.range(a..b)` (only the clusters the window overlaps are read), then:

| Call | Returns |
| --- | --- |
| `.read::<T>()` | `Vec<T>`: `f64`, `String`, `Vec<f32>` for a `std::vector<float>`, `Option<i32>` for a `std::optional<int32_t>`. Another type than the field holds is an `Error::TypeMismatch`. |
| `.values()` | A `Column`, for every shape (below). It is stored flat, as the pages are: nothing is allocated per entry. |
| `.to_field()` | A writable `Field` of the same name, stored the way the file stores it, for an `NtupleWriter`. |

```rust
let file = FileReader::open("data.root")?;
let ntpl = NtupleReader::open(&file, "events")?;

println!("{} entries", ntpl.entries());
println!("fields: {:?}", ntpl.field_names().collect::<Vec<_>>());
println!("clusters start at {:?}", ntpl.cluster_boundaries());

let mass: Vec<f64> = ntpl.field("mass").range(0..1000).read()?;
match ntpl.field("mass").values()? {
    Column::F64(values) => println!("{} masses", values.len()),
    other => println!("unexpected shape: {}", other.type_name()),
}
```

### What `values()` returns

`values()` walks the field tree and returns the field as a column of its
shape:

| Field | `Column` |
| --- | --- |
| scalar leaf | `Bool`, `I8`/`U8`/…/`I64`/`U64`, `F32`, `F64` |
| `std::string` | `String { offsets, text }` |
| `std::vector<T>`, `std::set<T>` | `List { offsets, values }` |
| `std::vector<std::vector<T>>` and deeper | a `List` of `List`s |
| record / struct (incl. user classes) | `Struct(Vec<(String, Column)>)` |
| `std::vector<MyStruct>`, `std::map<K, V>` | a `List` of `Struct`s |
| `std::optional<T>`, `std::unique_ptr<T>` | `Optional { valid, values }` |
| `std::variant` | `Union { tags, indices, alternatives }` |
| `std::array<T, N>` / `std::bitset<N>` | `Array { len, values }` |

`Column::len` gives the number of entries, and `Header::stored_type` how a
field is stored where that is more than its shape says (a half-precision
float, a set).

!!! note
    A `Struct` is a struct of arrays: each named member holds one value per
    record. Inside a `List` the members hold the records of every entry back
    to back, cut into entries by the list's `offsets`.

### Reading nested values

For a `std::vector<MyStruct>` (a list of structs), the `offsets` cut the
members' values into per-entry groups:

```rust
if let Column::List { offsets, values } = ntpl.field("clusters").values()? {
    if let Column::Struct(members) = *values {
        // `members` is [("_0", I32([...])), ("_1", F64([...]))], flat.
        // entry k spans offsets[k]..offsets[k + 1], and offsets[0] is 0.
        let entries = offsets.len() - 1;
        println!("{entries} entries, {} records", members[0].1.len());
    }
}
```

### Lower-level access

For schema introspection and raw decoding, the reader also exposes:

| Method | Returns |
| --- | --- |
| `ntpl.header()` | parsed schema (fields and columns) |
| `ntpl.footer()` | cluster groups |
| `ntpl.anchor()` | the verified anchor |
| `ntpl.read_column(index)` | a single physical column as `ColumnValues` |

## Compression

The `compression` argument on every write path is a `Compression`: `None`, or
`Zstd`, `Zlib`, `Lz4`, or `Lzma` at a given level (e.g. `Compression::Zstd(5)`).
Pages are compressed only when the result is actually smaller, exactly as ROOT
does. oxiroot both reads and writes Zstd, zlib, LZ4, and LZMA. See
[Compression](compression.md) for the full codec matrix.

## Interoperability

Files written by oxiroot are read by official ROOT and uproot, and oxiroot reads
RNTuples those tools write — including split, zigzag, and delta column encodings,
all integer widths, reduced-precision reals, the `Switch` (variant) column, and
user classes split into a record of their members. For a runnable end-to-end
example of the nested shapes, see `crates/oxiroot/examples/rntuple_nested.rs`:

```sh
cargo run -p oxiroot --example rntuple_nested
```

## See also

- [Datasets](datasets.md) — one API for RNTuple and `TTree`.
- [Getting started](../getting-started/quickstart.md) — install and first read/write.
- [TTree](ttree.md) — the classic row-wise event format.
- [Compression](compression.md) — codecs available on read and write.
- [API reference](../api/oxiroot/index.html) — full type-level docs.
