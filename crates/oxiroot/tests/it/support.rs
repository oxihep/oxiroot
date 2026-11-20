#![allow(dead_code)]
//! Whole-tree and whole-RNTuple writing for the tests, on top of the low-level
//! [`raw::write_tree`]: a tree written in one go (every branch kind, split
//! branches included) to a file, to bytes, or into a `FileWriter`.

use std::io::Cursor;
use std::path::Path;

use oxiroot::file::raw::{ContainerWriter, DirId, KSTART_BIG_FILE};
use oxiroot::file::streamer_gen::ClassDef;
use oxiroot::file::{Compression, Result, WriteInto};
use oxiroot::ntuple::Field;
use oxiroot::tree::{Branch, raw};

fn tree_bytes(
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
    tree_bytes(file_name, tree, branches, compression, 0)
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
        tree_bytes(file_name(path), tree, branches, compression, 0)?,
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
    let data = tree_bytes(
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
    pub fn new(name: impl Into<String>, branches: Vec<Branch>) -> TreeObject {
        TreeObject {
            name: name.into(),
            branches,
        }
    }

    pub fn write_root(&self, path: impl AsRef<Path>, compression: Compression) -> Result<()> {
        write_tree_file(path, &self.name, &self.branches, compression)
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

fn ntuple_bytes(
    file_name: &str,
    name: &str,
    fields: &[Field],
    late: &[(u64, Field)],
    compression: Compression,
) -> Result<Vec<u8>> {
    let classes = oxiroot::ntuple::raw::ntuple_streamer_classes(
        fields.iter().chain(late.iter().map(|(_, f)| f)),
    );
    ContainerWriter::build(file_name, compression, KSTART_BIG_FILE, |file| {
        oxiroot::ntuple::raw::write_ntuple(file, DirId::TOP, name, fields, late)?;
        file.place_streamer_info(&[], &classes)
    })
}

/// The bytes of a file holding one RNTuple, in one cluster.
pub fn rntuple_file_bytes(
    file_name: &str,
    name: &str,
    fields: &[Field],
    compression: Compression,
) -> Result<Vec<u8>> {
    ntuple_bytes(file_name, name, fields, &[], compression)
}

/// Write a file holding one RNTuple, in one cluster.
pub fn write_rntuple_file(
    path: impl AsRef<Path>,
    name: &str,
    fields: &[Field],
    compression: Compression,
) -> Result<()> {
    let path = path.as_ref();
    std::fs::write(
        path,
        ntuple_bytes(file_name(path), name, fields, &[], compression)?,
    )?;
    Ok(())
}

/// An RNTuple for [`FileWriter::put`](oxiroot_io_core::FileWriter::put), or to
/// write on its own.
pub struct NtupleObject {
    name: String,
    fields: Vec<Field>,
}

impl NtupleObject {
    pub fn new(name: impl Into<String>, fields: Vec<Field>) -> NtupleObject {
        NtupleObject {
            name: name.into(),
            fields,
        }
    }

    pub fn write_root(&self, path: impl AsRef<Path>, compression: Compression) -> Result<()> {
        write_rntuple_file(path, &self.name, &self.fields, compression)
    }

    pub fn to_root_bytes(&self, file_name: &str, compression: Compression) -> Result<Vec<u8>> {
        ntuple_bytes(file_name, &self.name, &self.fields, &[], compression)
    }

    /// Write with schema-extension fields, each `(first entry, field)`.
    pub fn write_root_extended(
        &self,
        path: impl AsRef<Path>,
        late: &[(u64, Field)],
        compression: Compression,
    ) -> Result<()> {
        let path = path.as_ref();
        std::fs::write(
            path,
            ntuple_bytes(file_name(path), &self.name, &self.fields, late, compression)?,
        )?;
        Ok(())
    }
}

impl WriteInto for NtupleObject {
    fn root_class(&self) -> String {
        oxiroot::ntuple::ANCHOR_CLASS.to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        oxiroot::ntuple::raw::write_ntuple(file, dir, &self.name, &self.fields, &[])
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        oxiroot::ntuple::raw::ntuple_streamer_classes(&self.fields)
    }
}
