//! `RNTuple` — ROOT's columnar event-data format — reader and writer.
//!
//! Implements the on-disk binary specification v1.0 (ROOT v6.34 and later). Reading
//! proceeds anchor → header/footer envelopes → page list → pages → column
//! decode. The anchor is big-endian; envelopes and payloads are little-endian;
//! integrity is checked with XXH3-64 throughout.
//!
//! Spec: <https://github.com/root-project/root/blob/v6-34-00-patches/tree/ntuple/v7/doc/BinaryFormatSpecification.md>

mod anchor;
mod column;
mod envelope;
mod field;
mod footer;
mod header;
mod merge;
mod page;
mod pagelist;
mod reader;
mod streamer;
mod writer;

// The format modules are private: the spec is still moving (the writer already
// emits 1.0.1.0 frames). Reading and writing go through `NtupleReader` and the
// writer types; the anchor, header and footer types stay public for schema
// introspection.
pub use anchor::{ANCHOR_CLASS, RNTupleAnchor};
pub use column::ColumnType;
pub use envelope::Locator;
pub use footer::{ClusterGroup, Footer};
pub use header::{ColumnDescriptor, FieldDescriptor, Header, StructRole};
pub use reader::NtupleReader;

/// The `RNTuple` format's own reader and writer types: typed field values,
/// field constructors (`std::variant` among them), the streaming writer, and
/// the functions that lay an `RNTuple` out in a file.
///
/// `oxiroot-dataset` is the API for reading and writing columns, in a tree
/// or an `RNTuple` alike. These are what it is built on, for tools that work
/// at the level of the format (a merger, an inspector).
pub mod raw {
    pub use crate::field::{FieldValues, FromField, Jagged};
    pub use crate::page::ColumnValues;
    pub use crate::reader::FieldRead;
    pub use crate::writer::raw::*;
    pub use crate::writer::{Column, Field, NtupleWriter};
}
