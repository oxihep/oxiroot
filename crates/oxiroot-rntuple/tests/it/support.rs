//! Whole-RNTuple writing for the tests, on top of the low-level
//! [`raw::write_ntuple`]: an RNTuple written in one go (schema-extension
//! fields included) to a file, to bytes, or into a `FileWriter`.

use std::io::Cursor;
use std::path::Path;

use oxiroot_io_core::raw::{ContainerWriter, DirId, KSTART_BIG_FILE};
use oxiroot_io_core::streamer_gen::ClassDef;
use oxiroot_io_core::{Compression, Result, WriteInto};
use oxiroot_rntuple::{Field, raw};

fn bytes(
    file_name: &str,
    name: &str,
    fields: &[Field],
    late: &[(u64, Field)],
    compression: Compression,
) -> Result<Vec<u8>> {
    let classes = raw::ntuple_streamer_classes(fields.iter().chain(late.iter().map(|(_, f)| f)));
    ContainerWriter::build(file_name, compression, KSTART_BIG_FILE, |file| {
        raw::write_ntuple(file, DirId::TOP, name, fields, late)?;
        file.place_streamer_info(&[], &classes)
    })
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file.root")
}

/// The bytes of a file holding one RNTuple, in one cluster.
pub fn rntuple_file_bytes(
    file_name: &str,
    name: &str,
    fields: &[Field],
    compression: Compression,
) -> Result<Vec<u8>> {
    bytes(file_name, name, fields, &[], compression)
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
        bytes(file_name(path), name, fields, &[], compression)?,
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
        bytes(file_name, &self.name, &self.fields, &[], compression)
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
            bytes(file_name(path), &self.name, &self.fields, late, compression)?,
        )?;
        Ok(())
    }
}

impl WriteInto for NtupleObject {
    fn root_class(&self) -> String {
        oxiroot_rntuple::ANCHOR_CLASS.to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn write_into(&self, file: &mut ContainerWriter<Cursor<Vec<u8>>>, dir: DirId) -> Result<()> {
        raw::write_ntuple(file, dir, &self.name, &self.fields, &[])
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        raw::ntuple_streamer_classes(&self.fields)
    }
}
