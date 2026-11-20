//! Reading a `TTree` and its branches.
//!
//! `TTree`/`TBranch`/`TBranchElement` are parsed by walking the member list in
//! the file's own `TStreamerInfo` (see [`walk_members`](members::walk_members)) rather than at fixed
//! offsets, so the reader follows whatever schema the file declares; an unknown
//! member type is reported instead of parsed at a guessed offset. (`TLeaf*` are
//! still read by their compact, byte-count-bounded layout.) The branch data
//! itself lives in [`crate::basket`]s. Handles single-leaf branches:
//! scalars, fixed (`x[N]`) and variable (`x[n]`) arrays, and `TLeafC` strings,
//! unsplit `std::vector<T>` `TBranchElement` branches (the element type comes
//! from `fClassName`, and each entry carries a 10-byte streamer header), and
//! *split* (`fSplitLevel > 0`) `std::vector<MyStruct>` branches, which are
//! exposed as their per-member jagged sub-branches (`hits.x`, `hits.y`, …).

use oxiroot_io_core::raw::{Key, StreamerElement, decompress_payload, find_key};
use oxiroot_io_core::{Error, FileReader, Result};

use std::ops::Range;

use crate::value::{BranchValues, FromBranch, Jagged, LeafType};
use oxiroot_io_core::cast::SaturatingFrom;

mod decode;
mod members;
mod parse;
mod types;

use decode::{Decode, decode_baskets, decode_scalar, entry_regions, read_baskets, slice_values};
use parse::read_tree;

/// Reads a `TTree` from a [`FileReader`]. Opening parses the tree's name, entry
/// count, and branches; branch data is read on demand, one branch at a time,
/// with [`branch`](Self::branch).
///
/// This is the low-level, TTree-specific reader. For one API over TTree and
/// RNTuple, read through `oxiroot::data::Dataset`.
#[doc(alias = "TTree", alias = "TTreeReader")]
#[derive(Debug, Clone)]
pub struct TreeReader<'f> {
    /// The file the branch data is read from.
    file: &'f FileReader,
    name: String,
    entries: u64,
    branches: Vec<Branch>,
    /// Branches present in the file that this crate cannot (yet) read, as
    /// `(name, reason)` — surfaced via [`TreeReader::unsupported_branches`].
    unsupported: Vec<(String, String)>,
    /// The classes (and versions) declared in the file's `TStreamerInfo` — the
    /// schema this tree was written against; surfaced via
    /// [`TreeReader::streamer_classes`]. Empty if the file has no streamer info.
    streamer_classes: Vec<(String, i32)>,
    /// Friend trees recorded by `TTree::AddFriend` (read from `fFriends`);
    /// surfaced via [`TreeReader::friends`].
    friends: Vec<Friend>,
    /// `(alias, expression)` pairs set with `TTree::SetAlias` (read from
    /// `fAliases`); surfaced via [`TreeReader::aliases`] / [`TreeReader::alias`].
    aliases: Vec<(String, String)>,
    /// The index built with `TTree::BuildIndex` (read from `fTreeIndex`);
    /// surfaced via [`TreeReader::index`].
    index: Option<TreeIndex>,
}

/// A tree's index (`TTree::BuildIndex`), persisted in its `fTreeIndex`: the
/// `(major, minor)` keys, sorted, and the entry each one names. It is what lets
/// a friend be joined on a key instead of by entry number — the friend's entries
/// need not line up with, or even cover, the main tree's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeIndex {
    /// The branch giving the major key (`fMajorName`, e.g. `"run"`).
    major_name: String,
    /// The branch giving the minor key (`fMinorName`, e.g. `"event"`).
    minor_name: String,
    /// The keys, ascending, as ROOT sorted them.
    keys: Vec<(i64, i64)>,
    /// `entries[i]` is the tree entry holding `keys[i]`.
    entries: Vec<u64>,
}

impl TreeIndex {
    /// Build an index from its parts (the persisted arrays, in ROOT's order).
    pub(crate) fn new(
        major_name: String,
        minor_name: String,
        keys: Vec<(i64, i64)>,
        entries: Vec<u64>,
    ) -> TreeIndex {
        TreeIndex {
            major_name,
            minor_name,
            keys,
            entries,
        }
    }

    /// The branch giving the major key (`fMajorName`).
    #[must_use]
    pub fn major_name(&self) -> &str {
        &self.major_name
    }

    /// The branch giving the minor key (`fMinorName`), empty when the index has
    /// only a major key.
    #[must_use]
    pub fn minor_name(&self) -> &str {
        &self.minor_name
    }

    /// How many keys the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the index holds no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The keys, ascending, with the entry each names.
    pub fn iter(&self) -> impl Iterator<Item = ((i64, i64), u64)> + '_ {
        self.keys.iter().copied().zip(self.entries.iter().copied())
    }

    /// The entry holding `(major, minor)`, or `None` when the index has no such
    /// key — ROOT's `TTree::GetEntryNumberWithIndex`. The keys are sorted, so
    /// this is a binary search.
    #[must_use]
    pub fn entry_of(&self, major: i64, minor: i64) -> Option<u64> {
        let at = self.keys.binary_search(&(major, minor)).ok()?;
        self.entries.get(at).copied()
    }
}

/// A friend tree attached to a `TTree` via `TTree::AddFriend`, persisted in the
/// main tree's `fFriends` list. Friends are read **positionally**: entry *i* of
/// the main tree pairs with entry *i* of the friend (the standard way HEP
/// analyses join per-event datasets without copying columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    /// The friend tree's key name (`fTreeName`) — what to open in its file.
    tree_name: String,
    /// The file holding the friend, or empty when it lives in the same file as
    /// the main tree (`AddFriend` with no explicit file name).
    file_name: String,
    /// The alias the friend is referred to by (`TFriendElement`'s name);
    /// defaults to the tree name.
    alias: String,
}

impl Friend {
    /// The friend tree's key name (`fTreeName`).
    pub fn tree_name(&self) -> &str {
        &self.tree_name
    }

    /// The file holding the friend, or `""` when it is in the same file as the
    /// main tree.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// The alias the friend is referred to by.
    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// Whether the friend lives in the same file as the main tree (`AddFriend`
    /// called without an explicit file name stores an empty file name).
    pub fn is_same_file(&self) -> bool {
        self.file_name.is_empty()
    }
}

/// One branch's metadata: its leaf type and the location of its baskets.
#[derive(Debug, Clone)]
struct Branch {
    name: String,
    /// `fTitle` — the leaf list / shape string (e.g. `x[3]`, `n`).
    title: String,
    leaf_type: LeafType,
    /// `fLen` — elements per entry (1 for a scalar branch).
    leaf_len: i32,
    /// Number of baskets actually written (`fWriteBasket`).
    n_baskets: usize,
    /// File offset of each basket (`fBasketSeek`).
    basket_seek: Vec<u64>,
    /// On-disk byte size of each basket (`fBasketBytes`), so a basket is fetched
    /// at its exact size — never over-fetched — over a ranged/remote source.
    /// Empty when the file omits it; then the reader probes the key header.
    basket_bytes: Vec<u64>,
    /// First entry number of each basket (`fBasketEntry`, `n_baskets` values),
    /// for selecting the baskets that cover an entry range.
    basket_entry: Vec<i64>,
    /// Per-entry streamer-header bytes to skip before the element data — `0` for
    /// `TLeaf`-based branches, `10` for an unsplit `std::vector<T>`
    /// `TBranchElement` (byte count + version + size).
    elem_header: usize,
    /// For one leaf of a multi-leaf (leaflist) branch: `(byte offset of this leaf
    /// within an entry, total entry stride)`. `None` for a single-leaf branch.
    leaflist: Option<(usize, usize)>,
    /// Per-entry array shape parsed from the leaf title — `[N]` for `x[N]`,
    /// `[N, M]` for a multidimensional `x[N][M]`, empty for a scalar. The data is
    /// stored row-major flat (`fLen` = the product); this records the split.
    dims: Vec<usize>,
    /// Whether an entry holds a variable number of elements: a C-style
    /// `x[n]` leaf, a `std::vector<T>`, or a split-struct member.
    variable: bool,
    /// Set for a `std::vector<std::vector<T>>` branch: `T`'s element type. The
    /// entry data is decoded as a doubly-nested collection ([`BranchValues::Nested`])
    /// rather than a flat jagged array.
    nested_elem: Option<LeafType>,
    /// Set for a synthesized member column of an old unsplit `TBranchObject`: the
    /// object class to decode out of each entry, and which member this column is.
    /// The branch data is one whole object per entry (`[className][version][members]`),
    /// from which this member is extracted at read time.
    object_member: Option<ObjectMember>,
}

/// One member column synthesized from an old unsplit `TBranchObject` (a whole
/// object stored per entry behind a `TLeafObject`). Carries the object class's
/// streamer layout so the per-entry object can be decoded without the registry.
#[derive(Debug, Clone)]
struct ObjectMember {
    /// The member this column extracts (e.g. `fName`).
    member: String,
    /// The object class's streamer elements, in order (so the per-entry object
    /// can be walked up to `member`).
    class_elements: Vec<StreamerElement>,
}

/// The write-reconstruction metadata of a read branch — the private-field
/// summary [`BranchRead::to_branch`] needs to rebuild a branch as a
/// writable [`Branch`](crate::Branch) with the correct kind (scalar / fixed
/// array / jagged / `std::vector` / string), or to reject the kinds this crate
/// cannot write back.
pub(crate) struct BranchMetaLite {
    /// Number of fixed-array dimensions parsed from the title (`x[N]` → 1,
    /// `x[N][M]` → 2, scalar / jagged / `std::vector` → 0).
    pub dims_len: usize,
    /// Per-entry streamer-header size: `>0` marks a `std::vector<T>`
    /// `TBranchElement` (distinguishing it from a jagged `x[n]` leaf).
    pub elem_header: usize,
    /// Whether this is one leaf of a multi-leaf (leaflist) branch.
    pub has_leaflist: bool,
    /// Whether this is a synthesized member column of an unsplit object branch.
    pub has_object_member: bool,
    /// Whether this is a `std::vector<std::vector<T>>` branch.
    pub has_nested: bool,
}

/// One `TLeaf` of a branch: its name/title, element type, fixed length, and byte
/// offset within an entry (`fOffset`, non-zero only inside a leaflist).
struct Leaf {
    name: String,
    title: String,
    leaf_type: LeafType,
    len: i32,
    offset: usize,
}

impl<'f> TreeReader<'f> {
    /// Open the `TTree` named `name` in `file`.
    pub fn open(file: &'f FileReader, name: &str) -> Result<TreeReader<'f>> {
        let key = file.key(name).ok_or_else(|| Error::NotFound {
            what: "key",
            name: name.to_string(),
        })?;
        Self::open_from_key(file, key)
    }

    /// Open the `TTree` named `name` from the subdirectory `subdir` (a
    /// `/`-separated path descends through nested `TDirectory`s).
    pub fn open_in(file: &'f FileReader, subdir: &str, name: &str) -> Result<TreeReader<'f>> {
        let dir = file.subdir(subdir)?;
        // The cycle `name` asks for, or the highest — `FileReader::key`'s rule.
        let key = find_key(&dir.keys, name).ok_or_else(|| Error::NotFound {
            what: "key",
            name: format!("{}/{name}", subdir.trim_end_matches('/')),
        })?;
        // Detach from the borrowed `dir` so the returned tree owns nothing tied to
        // it; the key's seek offsets are absolute, so decoding is identical.
        let key = key.clone();
        Self::open_from_key(file, &key)
    }

    /// Decode a `TTree` (or `TNtuple`/`TNtupleD`) from an already-located key.
    fn open_from_key(file: &'f FileReader, key: &Key) -> Result<TreeReader<'f>> {
        // `TNtuple` / `TNtupleD` are `TTree` subclasses (a `TTree` base wrapped in
        // one extra header plus a trailing `Int_t fNvar`); read them as trees too.
        if !matches!(key.class_name.as_str(), "TTree" | "TNtuple" | "TNtupleD") {
            return Err(Error::WrongClass {
                name: key.name.clone(),
                found: key.class_name.clone(),
                expected: "TTree".to_string(),
            });
        }
        // The file's TStreamerInfo is the authoritative schema: the reader walks
        // each class's declared member list rather than assuming a fixed layout,
        // so it adapts to the version the file was written with.
        let registry = file.streamer_registry()?;

        let payload = file.key_payload(key)?;
        let object = decompress_payload(&payload, key.obj_len as usize, "TTree")?;
        let mut tree = read_tree(
            file,
            &object,
            key.key_len as usize,
            &registry,
            &key.class_name,
        )?;
        tree.streamer_classes = registry
            .infos()
            .iter()
            .map(|i| (i.class_name.clone(), i.class_version))
            .collect();
        Ok(tree)
    }

    /// Total number of entries in the tree (`fEntries`).
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// The tree name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The names of the (readable) branches, in tree order.
    pub fn branch_names(&self) -> Vec<&str> {
        self.branches.iter().map(|b| b.name.as_str()).collect()
    }

    /// The friend trees attached with `TTree::AddFriend` (persisted in the main
    /// tree's `fFriends`). A friend is read positionally — entry *i* of this tree
    /// pairs with entry *i* of the friend. Open a friend with
    /// [`TreeReader::open`]`(file, friend.tree_name())` (the same `file` when
    /// [`Friend::is_same_file`]) and read its branches as usual; the columns line
    /// up by entry.
    pub fn friends(&self) -> &[Friend] {
        &self.friends
    }

    /// The index built with `TTree::BuildIndex`, read from the tree's
    /// `fTreeIndex`, or `None` for a tree without one.
    ///
    /// The index maps a `(major, minor)` key to the entry holding it, which is
    /// what [`join_by_index`](Self::join_by_index) joins a friend on.
    pub fn index(&self) -> Option<&TreeIndex> {
        self.index.as_ref()
    }

    /// For each entry of this tree, the entry of `friend` carrying the same
    /// index key — ROOT's index-based friend join (`TTree::BuildIndex`, then
    /// `AddFriend`). An entry whose key the friend does not hold is `None`.
    ///
    /// `friend` must carry an index, and this tree must hold the branches the
    /// index names (`run` and `event`, say); both are read as integers. A friend
    /// with no index, or a missing branch, is an error rather than a silent
    /// positional join, which would pair the wrong entries.
    ///
    /// The mapping is what a caller permutes a friend column with:
    ///
    /// ```no_run
    /// # use oxiroot_io_core::FileReader;
    /// # use oxiroot_tree::{BranchValues, TreeReader};
    /// # fn main() -> oxiroot_io_core::Result<()> {
    /// # let (file, friend_file) = (FileReader::open("f.root")?, FileReader::open("f.root")?);
    /// let main = TreeReader::open(&file, "main")?;
    /// let friend = TreeReader::open(&friend_file, "fr")?;
    /// let rows = main.join_by_index(&friend)?;
    ///
    /// let BranchValues::F64(weights) = friend.branch("weight").values()? else {
    ///     unreachable!("weight is a double branch")
    /// };
    /// // One weight per entry of `main`, in `main`'s order.
    /// let aligned: Vec<Option<f64>> = rows
    ///     .iter()
    ///     .map(|entry| entry.and_then(|e| weights.get(e as usize).copied()))
    ///     .collect();
    /// # let _ = aligned;
    /// # Ok(()) }
    /// ```
    pub fn join_by_index(&self, friend: &TreeReader<'_>) -> Result<Vec<Option<u64>>> {
        let index = friend.index().ok_or_else(|| {
            Error::InvalidInput(format!(
                "tree {:?} has no index to join on: build one with TTree::BuildIndex",
                friend.name()
            ))
        })?;
        let major = self.index_column(index.major_name())?;
        let minor = match index.minor_name() {
            "" => vec![0; major.len()],
            name => self.index_column(name)?,
        };
        Ok(major
            .iter()
            .zip(&minor)
            .map(|(&major, &minor)| index.entry_of(major, minor))
            .collect())
    }

    /// Read branch `name` as the integers an index key is made of.
    fn index_column(&self, name: &str) -> Result<Vec<i64>> {
        let values = self.branch(name).values()?;
        values.as_i64_vec().ok_or_else(|| {
            Error::InvalidInput(format!(
                "branch {name:?} of tree {:?} holds {:?} values, which are not index keys",
                self.name(),
                values.leaf_type()
            ))
        })
    }

    /// The `(alias, expression)` pairs defined with `TTree::SetAlias` (read from
    /// `fAliases`). An alias is a shorthand name standing for a branch
    /// expression (e.g. `"pt"` → `"sqrt(px*px+py*py)"`); oxiroot reads the pairs
    /// but does not evaluate the expressions.
    pub fn aliases(&self) -> &[(String, String)] {
        &self.aliases
    }

    /// The expression that alias `name` stands for, or `None` if there is no
    /// such alias.
    pub fn alias(&self, name: &str) -> Option<&str> {
        self.aliases
            .iter()
            .find(|(a, _)| a == name)
            .map(|(_, expr)| expr.as_str())
    }

    /// The element type of branch `name` (without reading its data), or `None`
    /// if there is no such readable branch.
    pub fn branch_type(&self, name: &str) -> Option<LeafType> {
        self.find_branch(name).map(|b| b.leaf_type)
    }

    /// `fLen` for branch `name`: the per-entry element count of a fixed-size
    /// array branch (`1` for a scalar; jagged branches report `1` and vary per
    /// entry). `None` if there is no such branch.
    pub fn branch_len(&self, name: &str) -> Option<i32> {
        self.find_branch(name).map(|b| b.leaf_len)
    }

    /// The title (`fTitle`) of branch `name` — the leaf-list / shape string such
    /// as `x[3]` or `n` — or `None` if there is no such branch.
    pub fn branch_title(&self, name: &str) -> Option<&str> {
        self.find_branch(name).map(|b| b.title.as_str())
    }

    /// How branch `name`'s entries are shaped — one value, a fixed array, a
    /// variable-length list, or a list of lists — or `None` if there is no
    /// such branch.
    pub fn branch_layout(&self, name: &str) -> Option<BranchLayout> {
        self.find_branch(name).map(|b| {
            if b.nested_elem.is_some() {
                BranchLayout::ListOfLists
            } else if b.variable {
                BranchLayout::List
            } else if b.leaf_type == LeafType::Str {
                // A `TLeafC`'s fLen is its longest string, not an array length.
                BranchLayout::Scalar
            } else if b.leaf_len > 1 || !b.dims.is_empty() {
                let n = b
                    .dims
                    .iter()
                    .product::<usize>()
                    .max(usize::saturating_from(b.leaf_len));
                BranchLayout::Array(n)
            } else {
                BranchLayout::Scalar
            }
        })
    }

    /// The per-entry fixed array shape of branch `name`: `[N]` for `x[N]`,
    /// `[N, M]` for a multidimensional `x[N][M]`, and `[]` for a scalar or a
    /// variable-length branch. The values are stored row-major flat (each entry's
    /// inner vector has `N` or `N*M` elements); use this to reshape them. `None`
    /// if there is no such branch.
    pub fn branch_shape(&self, name: &str) -> Option<&[usize]> {
        self.find_branch(name).map(|b| b.dims.as_slice())
    }

    /// Branches present in the file that this crate cannot read yet, as
    /// `(name, reason)` pairs (e.g. multi-leaf/leaflist branches or unsupported
    /// element types). These are absent from [`branch_names`](Self::branch_names),
    /// so this is the way to see what was skipped and why.
    pub fn unsupported_branches(&self) -> Vec<(&str, &str)> {
        self.unsupported
            .iter()
            .map(|(n, r)| (n.as_str(), r.as_str()))
            .collect()
    }

    /// The classes (and their versions) declared in the file's `TStreamerInfo` —
    /// the schema this tree was written against (e.g. `("TTree", 20)`,
    /// `("TBranch", 13)`). Empty if the file carries no streamer info. This is the
    /// member layout the reader walks on [`open`](Self::open) to parse the tree.
    pub fn streamer_classes(&self) -> Vec<(&str, i32)> {
        self.streamer_classes
            .iter()
            .map(|(n, v)| (n.as_str(), *v))
            .collect()
    }

    fn find_branch(&self, name: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.name == name)
    }

    /// The write-reconstruction metadata of branch `name` (for
    /// [`BranchRead::to_branch`]), or `None` if there is no such branch.
    pub(crate) fn branch_meta(&self, name: &str) -> Option<BranchMetaLite> {
        let b = self.find_branch(name)?;
        Some(BranchMetaLite {
            dims_len: b.dims.len(),
            elem_header: b.elem_header,
            has_leaflist: b.leaflist.is_some(),
            has_object_member: b.object_member.is_some(),
            has_nested: b.nested_elem.is_some(),
        })
    }

    /// Read the branch `name`: set a [`range`](BranchRead::range) of entries
    /// or [`parallel`](BranchRead::parallel) decompression on the returned
    /// [`BranchRead`], then take the values typed
    /// ([`read`](BranchRead::read)), as the [`BranchValues`] enum
    /// ([`values`](BranchRead::values)), or flat ([`flat`](BranchRead::flat)).
    ///
    /// ```no_run
    /// # use oxiroot_io_core::FileReader;
    /// # use oxiroot_tree::TreeReader;
    /// let file = FileReader::open("events.root")?;
    /// let tree = TreeReader::open(&file, "events")?;
    /// let pt: Vec<Vec<f32>> = tree.branch("lep_pt").range(0..1000).read()?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    pub fn branch<'a>(&'a self, name: &'a str) -> BranchRead<'a, 'f> {
        BranchRead {
            tree: self,
            name,
            range: None,
            decode: Decode::Serial,
        }
    }

    /// The entry numbers at which `name`'s baskets start, followed by the
    /// entry count: basket `i` holds entries `[b[i], b[i + 1])`. Reading a
    /// window that starts and ends on these decompresses no basket twice.
    pub fn basket_boundaries(&self, name: &str) -> Result<Vec<u64>> {
        let branch = self.require_branch(name)?;
        let mut bounds: Vec<u64> = if branch.basket_entry.len() == branch.n_baskets {
            branch
                .basket_entry
                .iter()
                .map(|&e| e.max(0).cast_unsigned())
                .collect()
        } else {
            vec![0]
        };
        bounds.push(self.entries);
        bounds.dedup();
        Ok(bounds)
    }

    fn require_branch(&self, name: &str) -> Result<&Branch> {
        self.find_branch(name).ok_or_else(|| Error::NotFound {
            what: "branch",
            name: name.to_string(),
        })
    }

    /// The baskets of `branch` overlapping entries `[start, stop)` (clamped to
    /// the entry count), and the first entry the first of them holds.
    fn select_baskets(
        &self,
        branch: &Branch,
        start: u64,
        stop: u64,
    ) -> (Vec<usize>, u64, u64, u64) {
        let stop = stop.min(self.entries);
        let start = start.min(stop);

        // Per-basket entry boundaries: basket i covers [start_i, start_{i+1}),
        // the last basket ending at the tree's entry count.
        let have_bounds = branch.basket_entry.len() == branch.n_baskets;
        let basket_start = |i: usize| {
            branch
                .basket_entry
                .get(i)
                .map_or(0, |&e| e.max(0).cast_unsigned())
        };
        let basket_stop = |i: usize| {
            if i + 1 < branch.basket_entry.len() {
                basket_start(i + 1)
            } else {
                self.entries
            }
        };

        // Select the baskets overlapping [start, stop). Without boundaries we
        // can't tell, so read them all (still correct after slicing).
        let mut indices = Vec::new();
        let mut first_entry = 0u64;
        for i in 0..branch.n_baskets {
            let keep = !have_bounds || (basket_start(i) < stop && basket_stop(i) > start);
            if keep {
                if indices.is_empty() {
                    first_entry = if have_bounds { basket_start(i) } else { 0 };
                }
                indices.push(i);
            }
        }
        (indices, first_entry, start, stop)
    }

    fn read_values(
        &self,
        name: &str,
        range: Option<Range<u64>>,
        decode: Decode,
    ) -> Result<BranchValues> {
        let branch = self.require_branch(name)?;
        // A window covering every entry is the whole branch: skip the slicing.
        let range = range.filter(|r| r.start > 0 || r.end < self.entries);
        let Some(range) = range else {
            let baskets = read_baskets(self.file, branch, 0..branch.n_baskets, decode)?;
            return decode_baskets(branch, &baskets);
        };
        let (indices, first_entry, start, stop) =
            self.select_baskets(branch, range.start, range.end);
        let baskets = read_baskets(self.file, branch, indices.iter().copied(), decode)?;
        let values = decode_baskets(branch, &baskets)?;
        // `values` covers [first_entry, ..); slice out [start, stop).
        let off = usize::saturating_from(start.saturating_sub(first_entry));
        let len = usize::saturating_from(stop - start);
        Ok(slice_values(values, off, len))
    }

    fn read_flat(&self, name: &str, range: Option<Range<u64>>, decode: Decode) -> Result<Jagged> {
        let branch = self.require_branch(name)?;
        if branch.leaf_type == LeafType::Str {
            return Err(Error::InvalidInput(format!(
                "branch {name:?} is a string branch, which has no flat form; use values()"
            )));
        }
        if branch.object_member.is_some() || branch.nested_elem.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch {name:?} has no flat form; use values()"
            )));
        }
        let range = range.filter(|r| r.start > 0 || r.end < self.entries);
        let (indices, first_entry, start, stop) = match range {
            Some(r) => self.select_baskets(branch, r.start, r.end),
            None => ((0..branch.n_baskets).collect(), 0, 0, self.entries),
        };
        let baskets = read_baskets(self.file, branch, indices.iter().copied(), decode)?;
        let regions = entry_regions(branch, &baskets);
        let skip = usize::saturating_from(start.saturating_sub(first_entry)).min(regions.len());
        let take = usize::saturating_from(stop - start).min(regions.len() - skip);
        let regions = &regions[skip..skip + take];
        let size = branch.leaf_type.size().max(1);

        let total: usize = regions.iter().map(|r| r.len()).sum();
        let mut offsets = Vec::with_capacity(regions.len() + 1);
        offsets.push(0u64);
        let mut bytes = Vec::with_capacity(total);
        let mut acc = 0u64;
        for r in regions {
            bytes.extend_from_slice(r);
            acc += (r.len() / size) as u64;
            offsets.push(acc);
        }
        Ok(Jagged {
            offsets,
            values: decode_scalar(branch.leaf_type, &bytes)?,
        })
    }
}

/// How a branch's entries are shaped, from its metadata alone: what
/// [`TreeReader::branch_layout`] reports. The element type is
/// [`TreeReader::branch_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BranchLayout {
    /// One value per entry (a string branch holds one string per entry).
    Scalar,
    /// The same number of values in every entry: a fixed array `x[N]` (a
    /// multidimensional `x[N][M]` counts `N × M`).
    Array(usize),
    /// A variable number of values per entry: a C-style `x[n]`, a
    /// `std::vector<T>`, or a member of a split `std::vector<Struct>`.
    List,
    /// A `std::vector<std::vector<T>>`.
    ListOfLists,
}

/// One branch read, set up by [`TreeReader::branch`]: the entries to read and
/// how, then the form to read them in.
#[must_use = "a BranchRead does nothing until read, values or flat is called"]
pub struct BranchRead<'a, 'f> {
    tree: &'a TreeReader<'f>,
    name: &'a str,
    range: Option<Range<u64>>,
    decode: Decode,
}

impl<'a, 'f> BranchRead<'a, 'f> {
    pub(crate) fn name(&self) -> &'a str {
        self.name
    }

    pub(crate) fn tree(&self) -> &'a TreeReader<'f> {
        self.tree
    }

    /// Read only entries `range` (clamped to the entry count: a window past
    /// the end yields fewer entries, not an error). Only the baskets the window
    /// touches are fetched and decompressed.
    pub fn range(mut self, range: Range<u64>) -> Self {
        self.range = Some(range);
        self
    }

    /// Decompress the baskets in parallel on rayon's global thread pool.
    #[cfg(feature = "rayon")]
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.decode = if parallel {
            Decode::Parallel
        } else {
            Decode::Serial
        };
        self
    }

    /// The values as `Vec<T>`: `f32` for a `float` branch, `Vec<f32>` for an
    /// array or `std::vector<float>`, `String` for a string. Asking for another
    /// type than the branch holds is an [`Error::TypeMismatch`]; nothing is
    /// converted.
    pub fn read<T: FromBranch>(self) -> Result<Vec<T>> {
        let name = self.name;
        T::from_values(name, self.values()?)
    }

    /// The values as the [`BranchValues`] enum, for code that does not know the
    /// branch's type in advance.
    pub fn values(self) -> Result<BranchValues> {
        self.tree.read_values(self.name, self.range, self.decode)
    }

    /// The values flat: one buffer of every element plus per-entry offsets —
    /// no allocation per entry. Scalar, fixed-array, variable-array and
    /// `std::vector<T>` branches; not strings or nested vectors.
    pub fn flat(self) -> Result<Jagged> {
        self.tree.read_flat(self.name, self.range, self.decode)
    }
}
