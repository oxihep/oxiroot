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
    `oxiroot::ntuple::raw` (`Field`, `NtupleWriter`, `FieldValues`): what a
    merger or an inspector works with, and the one field type a batch does not
    hold, `std::variant`, along with schema extension. The header and footer
    (`NtupleReader`) are reached from a dataset with `as_ntuple()`.

## Writing fields

An RNTuple is a set of named `Field`s. The `Field::*` constructors build the
common column types directly from `Vec`s; the type name and physical column
encoding are derived for you.

```rust
use oxiroot::ntuple::{NtupleReader};
use oxiroot::ntuple::raw::{Column, Field, FieldValues, NtupleWriter};

let fields = vec![
    Field::f64("mass", vec![91.2, 125.0]),
    Field::i32("charge", vec![0, -1]),
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

### Scalar field constructors

| Constructor | Element type | On-disk type |
| --- | --- | --- |
| `Field::bool` | `bool` | `bool` |
| `Field::i8` / `Field::u8` | `i8` / `u8` | `std::int8_t` / `std::uint8_t` |
| `Field::i16` / `Field::u16` | `i16` / `u16` | `std::int16_t` / `std::uint16_t` |
| `Field::i32` / `Field::u32` | `i32` / `u32` | `std::int32_t` / `std::uint32_t` |
| `Field::i64` / `Field::u64` | `i64` / `u64` | `std::int64_t` / `std::uint64_t` |
| `Field::f32` / `Field::f64` | `f32` / `f64` | `float` / `double` |
| `Field::string` | `String` | `std::string` |

```rust
let fields = vec![
    Field::bool("trigger", vec![true, false, true]),
    Field::u16("nhits", vec![3, 0, 12]),
    Field::string("label", vec!["mu".into(), "e".into(), "jet".into()]),
];
```

### Reduced-precision reals

A `float` field can be stored in less space by trading precision. Each maps to a
distinct RNTuple real column.

| Constructor | Column | Notes |
| --- | --- | --- |
| `Field::half(name, values)` | `Real16` | half precision (binary16), ~3 decimal digits |
| `Field::truncated(name, values, bits)` | `Real32Trunc` | mantissa truncated to `bits` total bits (`10..=31`) |
| `Field::quantized(name, values, min, max, bits)` | `Real32Quant` | linear quantization into `bits`-wide ints over `[min, max]` |

```rust
let pts = vec![5.5_f32, 12.25, 88.0];
let fields = vec![
    Field::half("pt_half", pts.clone()),
    Field::truncated("pt_trunc", pts.clone(), 16),
    Field::quantized("pt_quant", pts, 0.0, 100.0, 12),
];
```

!!! warning
    `quantized` assumes every value lies within `[min, max]`; values are clamped
    into the range before being encoded.

### Collections

`std::vector<T>` fields take one inner `Vec<T>` per entry:

| Constructor | C++ type |
| --- | --- |
| `Field::vector_bool` | `std::vector<bool>` |
| `Field::vector_i8` / `vector_u8` / `vector_i16` / `vector_u16` | `std::vector<intN_t>` |
| `Field::vector_i32` / `vector_i64` | `std::vector<int32_t>` / `<int64_t>` |
| `Field::vector_f32` / `vector_f64` | `std::vector<float>` / `<double>` |
| `Field::vector_string` | `std::vector<std::string>` |

Doubly-nested vectors take `Vec<Vec<Vec<T>>>` (outer = entries):

| Constructor | C++ type |
| --- | --- |
| `Field::vector_vector_bool` | `std::vector<std::vector<bool>>` |
| `Field::vector_vector_i32` / `vector_vector_i64` | `std::vector<std::vector<int32_t>>` / `<int64_t>` |
| `Field::vector_vector_f32` / `vector_vector_f64` | `std::vector<std::vector<float>>` / `<double>` |
| `Field::vector_vector_string` | `std::vector<std::vector<std::string>>` |

```rust
// Per-entry lists of tags, and per-entry hit patterns.
let tags = Field::vector_string("tags", vec![
    vec![],
    vec!["mu".into(), "iso".into()],
    vec!["jet".into()],
]);
let hits = Field::vector_vector_i32("hits", vec![
    vec![],
    vec![vec![1, 2]],
    vec![vec![3], vec![4, 5]],
]);
```

### Records, nested collections, and variants

For shapes the `Field::*` helpers do not cover, build a `Column` directly and
wrap it with `Field::new(name, column)`.

`Column::Record` is a struct-of-arrays: named sub-fields, each with one value per
record instance. A two-field record serializes as `std::pair`, more as
`std::tuple`. `Column::Nested` is a collection whose element is itself a
collection or a record — its cumulative `offsets` partition the flattened child
column: entry `i` spans `items[offsets[i]..offsets[i + 1]]`, so `offsets` leads
with a `0` and holds one value more than there are entries, the same convention
as a tree's jagged `offsets`. Together they express `std::vector<MyStruct>`:

```rust
// std::vector<std::pair<int32_t, double>>: a vector of records.
let clusters = Field::new(
    "clusters",
    Column::Nested {
        offsets: vec![0, 0, 1, 3], // entry 0: none, entry 1: 1, entry 2: 2
        items: Box::new(Column::Record(vec![
            ("_0".into(), Column::I32(vec![10, 20, 21])),
            ("_1".into(), Column::F64(vec![1.5, 2.5, 3.5])),
        ])),
    },
);
```

A `std::variant` field is built with `Field::variant(name, alternatives, tags)`.
The `alternatives` hold each alternative's densely-packed active values; `tags`
selects the 1-based active alternative per entry (`0` means valueless). Each
alternative must hold exactly as many values as there are entries selecting it.

```rust
// std::variant<int32_t, double>: entry 0 -> int, entry 1 -> double, entry 2 -> int.
let v = Field::variant(
    "payload",
    vec![Column::I32(vec![7, 9]), Column::F64(vec![2.5])],
    vec![1, 2, 1],
);
```

!!! tip
    The full set of `Column` variants — `Bool`, all integer widths, `F32`/`F64`,
    `Str`, the `Vec*` collections, `HalfF32` / `TruncF32` / `QuantF32`, `Record`,
    `Nested`, and `Variant` — is enumerated in the
    [API reference](../api/oxiroot/index.html).

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
    w.write_batch(&[Field::f64("mass", mass)])?;
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
| `.values()` | The `FieldValues` enum, for every shape (below). |
| `.flat()` | A `Jagged` (offsets + one flat column) for a `std::vector` of a scalar type, with no allocation per entry. |
| `.to_field()` | A writable `Field` of the same name and type, for an `NtupleWriter`. |

```rust
let file = FileReader::open("data.root")?;
let ntpl = NtupleReader::open(&file, "events")?;

println!("{} entries", ntpl.entries());
println!("fields: {:?}", ntpl.field_names());
println!("clusters start at {:?}", ntpl.cluster_boundaries());

let mass: Vec<f64> = ntpl.field("mass").range(0..1000).read()?;
match ntpl.field("mass").values()? {
    FieldValues::F64(values) => println!("{} masses", values.len()),
    other => println!("unexpected shape: {other:?}"),
}
```

### What `values()` reconstructs

`values()` walks the field tree and reconstructs per-entry values for every
supported shape:

| Shape | `FieldValues` variant |
| --- | --- |
| scalar leaf | `Bool`, `I8`/`U8`/…/`I64`/`U64`, `F32`, `F64` |
| `std::string` | `Str(Vec<String>)` |
| `std::vector<T>` | `VecBool`, `VecI32`, `VecF64`, `VecStr`, … |
| `std::vector<std::vector<T>>` and deeper | `Nested { offsets, items }` |
| record / struct (incl. user classes) | `Record(Vec<(String, FieldValues)>)` |
| `std::vector<MyStruct>` | `Nested` wrapping a `Record` |
| `std::variant` | `Variant { alternatives, tags, indices }` |
| `std::array<T, N>` / `std::bitset<N>` | grouped `N` per entry (a `Vec*` variant) |

`FieldValues::len` gives the element count at a level (entries, for a top-level
field), and `is_empty` reports an empty level.

!!! note
    `Record` is a struct-of-arrays: each named sub-field holds one value per
    record instance. Inside a `Nested` collection the sub-fields hold the
    *flattened* record instances, partitioned by the collection's `offsets`.

### Reading nested values

For a `std::vector<MyStruct>` (a `Nested` wrapping a `Record`), the `offsets`
slice the flattened sub-field arrays into per-entry groups:

```rust
if let FieldValues::Nested { offsets, items } = ntpl.field("clusters").values()? {
    if let FieldValues::Record(fields) = *items {
        // `fields` is [("_0", I32([...])), ("_1", F64([...]))] — flattened.
        // entry k spans offsets[k]..offsets[k + 1], and offsets[0] is 0.
        let entries = offsets.len() - 1;
        println!("{entries} entries, {} record instances", fields[0].1.len());
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
