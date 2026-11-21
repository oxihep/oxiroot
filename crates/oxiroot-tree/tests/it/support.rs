#![allow(dead_code)]
//! Whole-tree writing for the tests, on top of the low-level
//! [`raw::write_tree`]: a tree written in one go (every branch kind, split
//! branches included) to a file, to bytes, or into a `FileWriter`.

use std::io::Cursor;
use std::path::Path;

use oxiroot_io_core::raw::{ContainerWriter, DirId, KSTART_BIG_FILE};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Compression, Result, WriteInto};
use oxiroot_tree::raw;
use oxiroot_tree::raw::Branch;

fn bytes(
    file_name: &str,
    tree: &str,
    branches: &[Branch],
    compression: Compression,
    entries_per_basket: usize,
) -> Result<Vec<u8>> {
    let classes = raw::tree_streamer_classes(branches);
    ContainerWriter::build(file_name, compression, KSTART_BIG_FILE, |file| {
        raw::write_tree(file, DirId::TOP, tree, branches, entries_per_basket)?;
        file.place_streamer_info(&[], &classes)
    })
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file.root")
}

/// The bytes of a file holding one tree, one basket per branch.
pub fn tree_file_bytes(
    file_name: &str,
    tree: &str,
    branches: &[Branch],
    compression: Compression,
) -> Result<Vec<u8>> {
    bytes(file_name, tree, branches, compression, 0)
}

/// Write a file holding one tree, one basket per branch.
pub fn write_tree_file(
    path: impl AsRef<Path>,
    tree: &str,
    branches: &[Branch],
    compression: Compression,
) -> Result<()> {
    let path = path.as_ref();
    std::fs::write(
        path,
        bytes(file_name(path), tree, branches, compression, 0)?,
    )?;
    Ok(())
}

/// Write a file holding one tree, in baskets of `entries_per_basket` entries.
pub fn write_tree_file_baskets(
    path: impl AsRef<Path>,
    tree: &str,
    branches: &[Branch],
    compression: Compression,
    entries_per_basket: usize,
) -> Result<()> {
    let path = path.as_ref();
    let data = bytes(
        file_name(path),
        tree,
        branches,
        compression,
        entries_per_basket,
    )?;
    std::fs::write(path, data)?;
    Ok(())
}

/// A tree for [`FileWriter::put`](oxiroot_io_core::FileWriter::put).
pub struct TreeObject {
    name: String,
    branches: Vec<Branch>,
}

impl TreeObject {
    pub fn new(name: impl Into<String>, branches: Vec<Branch>) -> Self {
        Self {
            name: name.into(),
            branches,
        }
    }
}

impl oxiroot_io_core::Storable for TreeObject {
    fn into_stored(self) -> oxiroot_io_core::Stored {
        oxiroot_io_core::Stored::records(self)
    }
}

impl WriteInto for TreeObject {
    fn root_class(&self) -> String {
        "TTree".to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        raw::write_tree(file, dir, &self.name, &self.branches, 0)
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        raw::tree_streamer_classes(&self.branches)
    }
}
