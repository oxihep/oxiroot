# TTree

The classic ROOT columnar store.

!!! note "Read and write columns with Datasets"
    [Datasets](datasets.md) is the API for reading and writing columns, the
    same way for a `TTree` and an RNTuple, C-style `x[n]` arrays and chains
    included. This page covers the format's own layer underneath,
    `oxiroot::tree::raw` (`Branch`, `TreeWriter`): what a
    merger or an inspector works with, and what a batch does not hold, such as
    leaflists and split single objects. The tree's metadata (`TreeReader`:
    friends, the index, aliases, branch layouts) is reached from a dataset
    with `as_tree()`.

## Branches

A branch is a named [`Column`](datasets.md): the same flat column a dataset's
batch holds. `Branch::new(name, data)` stores it as its shape says:

| Column | From | On-disk form |
|---|---|---|
| a scalar | `Vec<i32>`, `Vec<f64>`, `Vec<bool>`, … | `TLeafI`/`TLeafD`/… |
| a string | `Vec<String>`, `Vec<&str>` | `TLeafC` |
| an array | `Vec<[T; N]>` | fixed `x[N]`: `TLeaf*` with `fLen = N` |
| a list | `Vec<Vec<T>>` | `std::vector<T>`: a `TBranchElement` |
| a list of strings | `Vec<Vec<String>>` | `std::vector<std::string>` |
| a list of lists | `Vec<Vec<Vec<T>>>` | `std::vector<std::vector<T>>` |
| a list of structs | `Column::list_of_structs` | split `std::vector<MyStruct>`: a parent `TBranchElement` and one sub-branch per member |

`T` is any of `bool`, `i8`/`u8`, `i16`/`u16`, `i32`/`u32`, `i64`/`u64`,
`f32`/`f64`. `.counted()` stores a list as a C-style `x[n]` instead, beside
an `n<name>` count branch (uproot's convention).

```rust
use oxiroot::prelude::*;
use oxiroot::tree::raw::Branch;

let branches = vec![
    Branch::new("event", (0..10).collect::<Vec<i32>>()),
    Branch::new("energy", vec![10.5, 20.1, 5.0, /* … */]),
    // Variable-length rows, written with a paired `nhits` count branch.
    Branch::new("hits", (0..10).map(|i| vec![i as f64; i % 3]).collect::<Vec<_>>()).counted(),
    Branch::new("label", (0..10).map(|i| format!("e{i}")).collect::<Vec<_>>()),
    // A fixed array: every entry holds three values.
    Branch::new("position", vec![[0.0, 1.0, 2.0]; 10]),
];
```

!!! note
    The element type is the data's: `vec![1.0, 2.0]` is an `f64` branch, and
    `vec![1.0_f32, 2.0]` an `f32` one. A column a tree cannot hold (an
    optional, a struct on its own, a variant) is refused when the tree is
    written, with the branch named.

### `std::vector<T>` branches

A list is written as a `std::vector<T>`, a `TBranchElement` where each entry
carries the streamer header ROOT writes for a streamed collection:

```rust
let tracks = Branch::new(
    "pt",
    vec![vec![1.0, 2.0], vec![3.5], vec![]], // one inner Vec per entry
);
```

### Split `std::vector<MyStruct>` branches

A split struct collection is a parent `TBranchElement` whose per-member data
lives in sub-branches (`hits.x`, `hits.y`, …). It is a list of structs:
`Column::list_of_structs` zips one list per member into one, checking that
they have the same number of elements in every entry. `.class_name` gives the
struct's C++ class name, and a `TStreamerInfo` for it is generated so the file
is self-describing.

```rust
use oxiroot::data::Column;

// Three events, each with a differing number of hits.
let xs = vec![vec![1.0, 2.0], vec![3.0], vec![]];
let ys = vec![vec![0.1, 0.2], vec![0.3], vec![]];

let hits = Branch::new(
    "hits",
    Column::list_of_structs([("x", Column::from(xs)), ("y", ys.into())])?,
)
.class_name("Hit"); // the C++ struct's class name
```

The members are numbers or `bool`s.

Reading, the members are branches of their own (`hits.x`, …);
`TreeReader::collection_members("hits")` lists them and
`branch_collection("hits.x")` names the collection. A
[`Dataset`](datasets.md) reads the whole collection as one list of structs.

## Writing a tree

`TreeWriter` writes a tree to its own file in batches. Each `write_batch` call
emits one basket per branch straight to the sink, so only the current batch is
in memory, the way ROOT's `TTree::Fill` flushes baskets as they fill. `finish`
writes the `TTree` metadata, the streamer info and the key list, then patches
the file header.

```rust
use oxiroot::tree::raw::TreeWriter;

let mut w = TreeWriter::create("big.root", "Events", Compression::Zstd(5))?;
for batch in 0..1_000 {
    let base = batch * 10_000;
    let x: Vec<f64> = (0..10_000).map(|i| (base + i) as f64).collect();
    w.write_batch(&[Branch::new("x", x)])?; // one basket, flushed now
}
let entries: u64 = w.entries();
w.finish()?; // commit metadata + header; returns the sink
```

`TreeWriter::new` takes any `Write + Seek` sink if you do not want a file path,
and `create_large`/`new_large` write ROOT's 64-bit layout for files past 2 GiB.
Every batch must share the first batch's schema: branch names, element types,
the jagged / `std::vector` flags, fixed-array widths, and a split branch's
class and members. A mismatch is an error. A first batch with no entries
fixes the schema too: an array column says its width whether or not it has
rows.

To place a tree next to other objects in one file, a
[`FileWriter`](../api/oxiroot/file/struct.FileWriter.html) object calls
`oxiroot::tree::raw::write_tree`; see the `raw` module's docs.

## Reading a tree

Open a `TTree` by name from a `FileReader`; the reader keeps the file. Then
read a branch with `branch(name)`, optionally narrowed, and take its values in
the form you need.

```rust
use oxiroot::tree::TreeReader;

let file = FileReader::open("tree.root")?;
let t = TreeReader::open(&file, "Events")?;

println!("{} entries: {:?}", t.entries(), t.branch_names().collect::<Vec<_>>());

let energy: Vec<f64> = t.branch("energy").read()?;
```

### Read forms

| Call | Returns |
|---|---|
| `branch(name).read::<T>()` | `Vec<T>`: `f64` for a `double` branch, `Vec<f32>` for an array or `std::vector<float>`, `String` for a string. Another type than the branch holds is an `Error::TypeMismatch`; nothing is converted. |
| `branch(name).values()` | A `Column`, for code that does not know the type in advance. It is stored flat: one buffer of every value, with per-entry offsets for a branch of lists, and nothing allocated per entry. |
| `branch(name).to_branch()` | A writable `Branch` of the same name and layout, for a `TreeWriter`: the round trip of a skim. |

Before any of them, `.range(a..b)` reads only entries `[a, b)`, fetching just
the baskets that cover the window (clamped to the entry count), and
`.parallel(true)` (with the `rayon` feature) decompresses the baskets in
parallel.

```rust
// Just the baskets covering entries 3..7.
let window: Vec<i32> = t.branch("event").range(3..7).read()?;

// A variable-length branch, typed.
let hits: Vec<Vec<f64>> = t.branch("hits").read()?;
```

### Columns

`values()` returns the branch as a `Column` of its own shape: a scalar branch
is a scalar column (`Column::F64(values)`), a fixed `x[N]` an array
(`Column::Array { len, values }`), a variable `x[n]` or a `std::vector` a list
(`Column::List { offsets, values }`), a `TLeafC` a string column, and a
`std::vector<std::vector<T>>` a list of lists.

A list keeps `num_entries + 1` cumulative `offsets` (`offsets[0] == 0`) over
one buffer of values; entry `i`'s elements are
`values[offsets[i] .. offsets[i + 1]]`:

```rust
use oxiroot::data::Column;

match t.branch("hits").values()? {
    Column::List { offsets, values } => {
        let flat: &[f64] = values.as_slice().expect("a list of f64");
        for (i, w) in offsets.windows(2).enumerate() {
            let row = &flat[w[0] as usize..w[1] as usize];
            println!("entry {i}: {} hits", row.len());
        }
    }
    other => println!("not a list: {}", other.type_name()),
}
```

## Introspection

A `TTree` reports each branch's type and shape without reading its data:

| Method | Returns |
|---|---|
| `branch_names()` | The names of the readable branches, in tree order. |
| `branch_type(name)` | The element `LeafType` (`I32`, `F64`, `Str`, …), or `None`. |
| `branch_layout(name)` | The entry shape: `Scalar`, `Array(n)`, `List` (variable length) or `ListOfLists`. |
| `basket_boundaries(name)` | The entries where the branch's baskets start, then the entry count. |
| `branch_len(name)` | `fLen` — the per-entry element count of a fixed array (`1` for a scalar). |
| `branch_shape(name)` | The fixed-array shape: `[N]` for `x[N]`, `[N, M]` for `x[N][M]`, `[]` for scalar/variable. |
| `branch_title(name)` | `fTitle` — the leaf-list / shape string (e.g. `x[3]`, `n`). |
| `unsupported_branches()` | `(name, reason)` pairs for branches present in the file but not read. |
| `streamer_classes()` | The `(class, version)` pairs declared in the file's `TStreamerInfo`. |

```rust
for name in t.branch_names() {
    println!(
        "{name}: type={:?} shape={:?}",
        t.branch_type(name).unwrap(),
        t.branch_shape(name).unwrap(),
    );
}
// Anything the reader skipped, and why.
for (name, why) in t.unsupported_branches() {
    eprintln!("skipped {name}: {why}");
}
```

!!! note
    The reader is streamer-info-driven: it parses `TTree`/`TBranch`/
    `TBranchElement` by walking the member list in the file's own
    `TStreamerInfo` rather than at fixed offsets, so a schema change is absorbed
    instead of misread, and an unknown member type is reported (via
    `unsupported_branches`) rather than parsed at a guessed offset.
    `streamer_classes()` exposes the schema the file was written against.

## Friends, and joining on an index

A friend tree (`TTree::AddFriend`) is read **positionally**: entry *i* of the
main tree pairs with entry *i* of the friend. `friends()` lists the friends a
tree records, so opening them is all it takes.

When the two trees are not entry-aligned — different order, or the friend
covering only some events — ROOT joins them on a key instead, with
`TTree::BuildIndex(major, minor)`. oxiroot reads that index and joins on it:

```rust
let file = FileReader::open("events.root")?;
let main = TreeReader::open(&file, "main")?;
let friend = TreeReader::open(&file, "fr")?;

// What the friend's index is on, and how many keys it holds.
let index = friend.index().expect("the friend was built with BuildIndex");
println!("{} + {:?}: {} keys", index.major_name(), index.minor_name(), index.len());

// For each entry of `main`, the friend entry with the same key.
let rows = main.join_by_index(&friend)?;

let weight: Vec<f64> = friend.branch("weight").read()?;
// One weight per entry of `main`, in `main`'s order; `None` where the friend
// has no such key.
let aligned: Vec<Option<f64>> = rows
    .iter()
    .map(|entry| entry.and_then(|e| weight.get(e as usize).copied()))
    .collect();
# Ok::<(), oxiroot::Error>(())
```

`index()` gives the keys directly too (`entry_of(major, minor)` is ROOT's
`GetEntryNumberWithIndex`, and `iter()` walks them in order). Joining a friend
that carries no index is an error rather than a silent fall back to pairing by
entry number, which would pair the wrong entries.

## Spanning files

Read a tree split over several files with
[`Dataset::chain`](datasets.md#several-files-as-one-chains).

## See also

- [Datasets](datasets.md) — one API for `TTree` and RNTuple.
- [Quickstart](../getting-started/quickstart.md) — the end-to-end write/read tour.
- [RNTuple](rntuple.md) — ROOT's modern columnar format.
- [Compression](compression.md) — codecs shared by every writer.
- [API reference](../api/oxiroot/index.html) — type-level docs for `oxiroot::tree`.
