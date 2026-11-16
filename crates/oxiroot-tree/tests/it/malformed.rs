//! Hardening: corrupt/truncated TTree files must yield `Err`, never panic.
//!
//! Covers a flat tree, fixed/variable arrays, a split `std::vector<MyStruct>`
//! tree, and a `std::vector<double>` tree. For every mutated file we open the
//! tree and read every branch; the test passes as long as nothing panics.

use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_tree::{TreeReader, concat_trees};

/// Fixtures spanning the supported branch layouts, with their tree name.
const FIXTURES: &[(&str, &str)] = &[
    ("tree_flat.root", "Events"),
    ("tree_arrays.root", "T"),
    ("tree_split.root", "T"),
    ("tree_vector.root", "T"),
];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("read fixture")
}

/// Open the tree (if it parses), read every branch, and run it through the
/// `hadd`-style [`concat_trees`] merge. The point is that none of this panics
/// regardless of the bytes.
fn poke_tree(f: &FileReader, tree: &str) {
    if let Ok(t) = TreeReader::open(f, tree) {
        let names: Vec<String> = t.branch_names().iter().map(|s| s.to_string()).collect();
        for b in &names {
            let _ = t.read_branch(f, b);
        }
        // Merge the (possibly corrupt) tree with itself — exercises the branch
        // kind reconstruction and value concatenation on malformed input.
        let _ = concat_trees(&[(f, &t)]);
    }
}

/// Stride that keeps each fixture to roughly `samples` probes regardless of size.
fn stride(len: usize, samples: usize) -> usize {
    (len / samples).max(1)
}

#[test]
fn tree_byte_flips_never_panic() {
    for (fix, tree) in FIXTURES {
        let data = fixture(fix);
        let step = stride(data.len(), 3000);
        for i in (0..data.len()).step_by(step) {
            for v in [0x00u8, 0xff] {
                let mut c = data.clone();
                c[i] = v;
                if let Ok(f) = FileReader::from_bytes(c) {
                    poke_tree(&f, tree);
                }
            }
        }
    }
}

#[test]
fn tree_truncations_never_panic() {
    for (fix, tree) in FIXTURES {
        let data = fixture(fix);
        let step = stride(data.len(), 2000);
        for len in (0..=data.len()).step_by(step) {
            if let Ok(f) = FileReader::from_bytes(data[..len].to_vec()) {
                poke_tree(&f, tree);
            }
        }
    }
}

/// A basket key whose `fKeyLen` is larger than the bytes probed for it used to
/// slice out of bounds (review finding 1A H1). Patch every `TBasket` key of
/// each fixture so its key header claims to run past the original record, and
/// read everything: errors are fine, panics are not.
#[test]
fn basket_key_longer_than_probe_never_panics() {
    for (name, tree) in FIXTURES {
        let original = fixture(name);
        let mut patched_any = false;
        let mut at = 0;
        while let Some(off) = find(&original[at..], b"\x07TBasket") {
            let class_at = at + off;
            at = class_at + 1;
            // The class name follows the key's fixed header and its two seek
            // pointers: 4-byte seeks (version <= 1000) or 8-byte ones.
            for start in [class_at.checked_sub(26), class_at.checked_sub(34)]
                .into_iter()
                .flatten()
            {
                let version = u16::from_be_bytes([original[start + 4], original[start + 5]]);
                let small = class_at - start == 26 && version <= 1000;
                let big = class_at - start == 34 && version > 1000;
                if !(small || big) {
                    continue;
                }
                let nbytes = i32::from_be_bytes(original[start..start + 4].try_into().unwrap());
                if nbytes <= 0 {
                    continue;
                }
                let mut bytes = original.clone();
                let key_len = (nbytes + 100).min(i32::from(i16::MAX)) as i16;
                bytes[start + 14..start + 16].copy_from_slice(&key_len.to_be_bytes());
                bytes[start..start + 4].copy_from_slice(&(i32::from(key_len) + 100).to_be_bytes());
                patched_any = true;
                if let Ok(f) = FileReader::from_bytes(bytes) {
                    poke_tree(&f, tree);
                }
            }
        }
        assert!(patched_any, "{name}: no TBasket key found to patch");
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
