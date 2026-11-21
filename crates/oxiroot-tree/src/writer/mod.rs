//! Writing a `TTree` into a ROOT file.
//!
//! Supports scalar, fixed-size array (`x[N]`), variable-length / jagged
//! (`x[n<name>]`, with an auto-generated count branch and an `fLeafCount`
//! reference), string (`TLeafC`), and `std::vector<T>` (`TBranchElement`)
//! branches. Mirrors the layout ROOT/uproot write (`TTree` v20, `TBranch` v13,
//! `TLeaf`* v1, `TBranchElement` v10) so the result reads back in ROOT, uproot, and
//! this crate. The embedded `TStreamerInfo` ([`crate::streamer_gen`]) makes the
//! file self-describing.

use std::collections::HashMap;
use std::io::{Seek, Write};

use oxiroot_io_core::raw::{ContainerWriter, DirId};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Error, Result};

mod baskets;
mod branch;
mod layout;
mod serialize;
mod streaming;

use baskets::{BasketRec, write_branch_baskets};
pub use branch::{Branch, SplitMember};
use branch::{check_split_members, split_class};
use serialize::build_tree_object;
pub use streaming::TreeWriter;

/// `fBits` ROOT writes for embedded `TObject`s.
const OBJ_BITS: u32 = 0x0300_0000;
/// ROOT's object-map displacement (`kMapOffset`): a referenced object is keyed
/// at `byte_count_position + keylen + 2`. Used to point a jagged leaf's
/// `fLeafCount` at the already-written count leaf.
const K_MAP_OFFSET: u32 = 2;

/// Maps a leaf name to the object-reference value (`pos + keylen + kMapOffset`)
/// of the first place that leaf was written, so a later `fLeafCount` can point
/// back to it the way ROOT does.
type LeafRefs = HashMap<String, u32>;

/// A write [`Branch`] is only ever built through the typed constructors, none of
/// which produce a [`BranchValues::Nested`] (a read-only doubly-nested
/// collection), so the writer's value matches never see one.
const NESTED_NOT_WRITABLE: &str =
    "a vector<string> or vector<vector<T>> write Branch is always a std::vector branch";

/// Reject branches whose entry counts differ from the first branch's: the tree
/// has one entry count, so the others would be cut or read past.
fn check_entry_counts(branches: &[Branch]) -> Result<()> {
    let expected = branches.first().map_or(0, Branch::n_entries);
    match branches.iter().find(|b| b.n_entries() != expected) {
        Some(b) => Err(Error::LengthMismatch {
            what: format!("branch {:?} entries", b.name),
            expected: expected as usize,
            found: b.n_entries() as usize,
        }),
        None => Ok(()),
    }
}

/// Reject branches the writer cannot lay out: branches with different entry
/// counts, a fixed-array branch with rows of differing length, and a split
/// branch whose members disagree.
fn check_branches(branches: &[Branch]) -> Result<()> {
    check_entry_counts(branches)?;
    for b in branches {
        if !b.jagged() && !b.stl_vector() && b.is_jagged() {
            return Err(Error::InvalidInput(format!(
                "branch {:?}: rows differ in length; use Branch::jagged_* or Branch::vector_* for \
                 variable-length arrays (Branch::array_* requires every row to have the same length)",
                b.name
            )));
        }
        if let Some(spec) = b.split() {
            check_split_members(&b.name, spec)?;
        }
    }
    Ok(())
}

/// The `TStreamerInfo` entries a tree with these branches needs: the canonical
/// `TTree` hierarchy (including the TBranchElement/TLeafElement `std::vector`
/// streamers), then each split branch's struct.
fn tree_streamer_classes(branches: &[Branch]) -> Vec<ClassDef<'_>> {
    let mut classes = crate::streamer_gen::tree_classes();
    classes.extend(branches.iter().filter_map(Branch::split).map(split_class));
    classes
}

/// Write a tree's baskets at the end of `file`, then the `TTree` that lists
/// them under a key in `dir`. A leaf branch has one basket per chunk of
/// `entries_per_basket` entries (`0` = one basket); a split branch has a count
/// basket plus one per member sub-branch.
fn write_tree_records<W: Write + Seek>(
    file: &mut ContainerWriter<W>,
    dir: DirId,
    tree_name: &str,
    branches: &[Branch],
    entries_per_basket: usize,
) -> Result<()> {
    // Expand each jagged branch into [count branch, jagged branch], matching
    // ROOT/uproot. `counts` owns the synthetic count branches so the effective
    // list `eff` can borrow them alongside the caller's branches.
    let counts: Vec<Branch> = branches.iter().filter_map(Branch::count_branch).collect();
    let mut eff: Vec<&Branch> = Vec::with_capacity(branches.len() + counts.len());
    let mut ci = 0;
    for b in branches {
        if b.jagged() {
            eff.push(&counts[ci]);
            ci += 1;
        }
        eff.push(b);
    }
    let n_entries = eff.first().map_or(0, |b| b.n_entries());

    let basket_groups: Vec<Vec<BasketRec>> = eff
        .iter()
        .map(|&b| write_branch_baskets(file, dir, b, tree_name, entries_per_basket))
        .collect::<Result<_>>()?;
    let tot_bytes: i64 = basket_groups
        .iter()
        .flatten()
        .map(|r| i64::from(r.nbytes))
        .sum();
    let tree_obj = build_tree_object(
        tree_name,
        &eff,
        &basket_groups,
        i64::from(n_entries),
        tot_bytes,
        file.is_big(),
    );
    file.place_key(dir, "TTree", tree_name, "", &tree_obj)?;
    Ok(())
}

/// Low-level writing of a whole tree into an open ROOT container — what a
/// [`WriteInto`](oxiroot_io_core::WriteInto) object (such as
/// `oxiroot::data::Dataset`'s) calls to place a tree next to other objects.
/// For a tree written to its own file, entry batch by entry batch, use
/// [`TreeWriter`].
pub mod raw {
    use std::io::{Seek, Write};

    use oxiroot_io_core::Result;
    use oxiroot_io_core::raw::{ContainerWriter, DirId};
    use oxiroot_io_core::streamer_gen::ClassDef;

    use super::Branch;

    /// Write the tree `name` with `branches` into `file` under directory
    /// `dir`: its baskets, then the `TTree` key. Every branch kind is
    /// supported, split `std::vector<Struct>` branches included. A branch is
    /// cut into baskets of `entries_per_basket` entries (`0`: one basket).
    ///
    /// # Errors
    ///
    /// Branches of different lengths, a fixed array with rows of differing
    /// length, split members that disagree, or a write error.
    pub fn write_tree<W: Write + Seek>(
        file: &mut ContainerWriter<W>,
        dir: DirId,
        name: &str,
        branches: &[Branch],
        entries_per_basket: usize,
    ) -> Result<()> {
        super::check_branches(branches)?;
        super::write_tree_records(file, dir, name, branches, entries_per_basket)
    }

    /// The `TStreamerInfo` classes a tree with `branches` needs in its file:
    /// the `TTree` hierarchy, and each split branch's struct.
    #[must_use]
    pub fn tree_streamer_classes(branches: &[Branch]) -> Vec<ClassDef<'static>> {
        super::tree_streamer_classes(branches)
            .into_iter()
            .map(ClassDef::into_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::BranchValues;
    use oxiroot_io_core::Compression;
    use oxiroot_io_core::raw::KSTART_BIG_FILE;

    /// A whole file holding one tree, switching to the 64-bit form past
    /// `big_threshold` bytes.
    fn tree_bytes(
        file_name: &str,
        tree_name: &str,
        branches: &[Branch],
        compression: Compression,
        entries_per_basket: usize,
        big_threshold: u64,
    ) -> Result<Vec<u8>> {
        let classes = raw::tree_streamer_classes(branches);
        ContainerWriter::build(file_name, compression, big_threshold, |file| {
            raw::write_tree(file, DirId::TOP, tree_name, branches, entries_per_basket)?;
            file.place_streamer_info(&[], &classes)
        })
    }

    use crate::TreeReader;
    use oxiroot_io_core::FileReader;
    use std::io::Cursor;

    fn branches() -> Vec<Branch> {
        vec![
            Branch::i32("x", vec![1, 2, 3, 4]),
            Branch::jagged_f32("v", vec![vec![1.0], vec![], vec![2.0, 3.0], vec![4.0]]),
            Branch::string(
                "s",
                vec!["a".into(), "bb".into(), String::new(), "d".into()],
            ),
        ]
    }

    /// Each leaf's `(name, fIsRange, fMaximum)`, as a generic reader sees them.
    fn leaf_ranges(bytes: Vec<u8>) -> Vec<(String, bool, i64)> {
        use oxiroot_io_core::Value;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a leaf's fMaximum: a whole number read as a double"
        )]
        fn walk(v: &Value, out: &mut Vec<(String, bool, i64)>) {
            // A branch's leaf list points back at the leaves the tree holds, so
            // only the object itself carries the members; a reference to it does
            // not.
            if matches!(v, Value::Object { .. })
                && v.class().is_some_and(|c| c.starts_with("TLeaf"))
            {
                let name = v.get("fName").and_then(Value::as_str).unwrap_or("");
                if !out.iter().any(|(n, ..)| n == name) {
                    let range = v.get("fIsRange").and_then(Value::as_bool).unwrap();
                    let max = v.get("fMaximum").and_then(Value::as_f64).unwrap() as i64;
                    out.push((name.to_string(), range, max));
                }
                return;
            }
            for (_, member) in v.members().unwrap_or_default() {
                walk(member, out);
            }
            for element in v.as_array().unwrap_or_default() {
                walk(element, out);
            }
        }
        let f = FileReader::from_bytes(bytes).unwrap();
        let mut out = Vec::new();
        walk(&f.get::<Value>("T").unwrap(), &mut out);
        out
    }

    #[test]
    fn both_writers_fill_leaf_ranges_as_root_does() {
        // ROOT keeps a plain leaf's fMaximum at 0, marks a count leaf as a range
        // with the largest count, and gives a TLeafC the longest string + 1.
        // The streamed file sees the largest values only in its second batch.
        let first = || {
            vec![
                Branch::i32("x", vec![3, 7]),
                Branch::jagged_f32("v", vec![vec![1.0], vec![]]),
                Branch::string("s", vec!["a".into(), "bbb".into()]),
            ]
        };
        let second = || {
            vec![
                Branch::i32("x", vec![9, 1]),
                Branch::jagged_f32("v", vec![vec![2.0, 3.0, 4.0], vec![5.0]]),
                Branch::string("s", vec!["cc".into(), "dddd".into()]),
            ]
        };
        let all = vec![
            Branch::i32("x", vec![3, 7, 9, 1]),
            Branch::jagged_f32("v", vec![vec![1.0], vec![], vec![2.0, 3.0, 4.0], vec![5.0]]),
            Branch::string(
                "s",
                vec!["a".into(), "bbb".into(), "cc".into(), "dddd".into()],
            ),
        ];
        let expected: Vec<(String, bool, i64)> = [
            ("x", false, 0),
            ("nv", true, 3),
            ("v", false, 0),
            ("s", false, 5),
        ]
        .into_iter()
        .map(|(n, r, m)| (n.to_string(), r, m))
        .collect();

        let one_shot =
            tree_bytes("t.root", "T", &all, Compression::Stored, 0, KSTART_BIG_FILE).unwrap();
        assert_eq!(leaf_ranges(one_shot), expected, "one-shot writer");

        let mut w =
            TreeWriter::new(Cursor::new(Vec::new()), "t.root", "T", Compression::Stored).unwrap();
        w.write_batch(&first()).unwrap();
        w.write_batch(&second()).unwrap();
        assert_eq!(
            leaf_ranges(w.finish().unwrap().into_inner()),
            expected,
            "streaming writer"
        );
    }

    #[test]
    fn one_shot_switches_to_the_big_form_past_the_threshold() {
        // Forced into the 64-bit form, a tiny tree matches the streaming writer's
        // big output for the same single batch.
        let scalars = || {
            vec![
                Branch::i32("x", vec![3, 7, 5]),
                Branch::f64("y", vec![0.5, 2.0, 1.0]),
            ]
        };
        let one_shot = tree_bytes("t.root", "T", &scalars(), Compression::Zstd(3), 0, 0).unwrap();
        let mut w =
            TreeWriter::new_large(Cursor::new(Vec::new()), "t.root", "T", Compression::Zstd(3))
                .unwrap();
        w.write_batch(&scalars()).unwrap();
        assert_eq!(one_shot, w.finish().unwrap().into_inner());

        let small = tree_bytes(
            "t.root",
            "T",
            &scalars(),
            Compression::Zstd(3),
            0,
            KSTART_BIG_FILE,
        )
        .unwrap();
        assert!(!FileReader::from_bytes(small).unwrap().header().is_big());

        // Every branch kind reads back from the big form.
        let bytes = tree_bytes("t.root", "T", &branches(), Compression::Stored, 2, 0).unwrap();
        let f = FileReader::from_bytes(bytes).unwrap();
        assert!(f.header().is_big());
        let t = TreeReader::open(&f, "T").unwrap();
        assert_eq!(
            t.branch("x").values().unwrap(),
            BranchValues::I32(vec![1, 2, 3, 4])
        );
        assert_eq!(
            t.branch("v").values().unwrap(),
            BranchValues::VecF32(vec![vec![1.0], vec![], vec![2.0, 3.0], vec![4.0]])
        );
        assert_eq!(
            t.branch("s").values().unwrap(),
            BranchValues::Str(vec!["a".into(), "bb".into(), String::new(), "d".into()])
        );
    }

    #[test]
    fn a_branch_name_longer_than_255_bytes_round_trips() {
        let name = "b".repeat(300);
        let bytes = tree_bytes(
            "t.root",
            "T",
            &[Branch::f64(name.as_str(), vec![0.5, 1.5])],
            Compression::Stored,
            1,
            KSTART_BIG_FILE,
        )
        .unwrap();
        let f = FileReader::from_bytes(bytes).unwrap();
        let t = TreeReader::open(&f, "T").unwrap();
        assert_eq!(
            t.branch(&name).values().unwrap(),
            BranchValues::F64(vec![0.5, 1.5])
        );
    }
}
