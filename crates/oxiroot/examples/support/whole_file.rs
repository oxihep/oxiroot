//! Whole-file writing for the interop harnesses, on the low-level `raw`
//! writers: one `TTree` or RNTuple per file, written in one cluster, with every
//! branch and field kind (split branches included) that ROOT and uproot must
//! read back.

use std::path::Path;

use oxiroot::file::raw::{ContainerWriter, DirId, KSTART_BIG_FILE};
use oxiroot::ntuple::Field;
use oxiroot::prelude::Compression;
use oxiroot::tree::Branch;

fn write(
    path: impl AsRef<Path>,
    compression: Compression,
    layout: impl FnMut(&mut ContainerWriter<std::io::Cursor<Vec<u8>>>) -> oxiroot::file::Result<()>,
) -> oxiroot::file::Result<()> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file.root");
    std::fs::write(
        path,
        ContainerWriter::build(name, compression, KSTART_BIG_FILE, layout)?,
    )?;
    Ok(())
}

/// Write a file holding the tree `name`, one basket per branch.
pub fn write_tree_file(
    path: impl AsRef<Path>,
    name: &str,
    branches: &[Branch],
    compression: Compression,
) -> oxiroot::file::Result<()> {
    let classes = oxiroot::tree::raw::tree_streamer_classes(branches);
    write(path, compression, |f| {
        oxiroot::tree::raw::write_tree(f, DirId::TOP, name, branches, 0)?;
        f.place_streamer_info(&[], &classes)
    })
}

/// Write a file holding the RNTuple `name`, in one cluster.
pub fn write_rntuple_file(
    path: impl AsRef<Path>,
    name: &str,
    fields: &[Field],
    compression: Compression,
) -> oxiroot::file::Result<()> {
    let classes = oxiroot::ntuple::raw::ntuple_streamer_classes(fields);
    write(path, compression, |f| {
        oxiroot::ntuple::raw::write_ntuple(f, DirId::TOP, name, fields, &[])?;
        f.place_streamer_info(&[], &classes)
    })
}
