//! Reading and writing ROOT `TTree` — the classic columnar store.
//!
//! A `TTree` holds `TBranch`es, each storing its values in `TBasket`s (`Key`s)
//! described by `TLeaf`s. This crate reads and writes: primitive scalars
//! (`bool`/`int`/`float`/… and unsigned variants), fixed-size arrays (`x[N]`),
//! variable-length arrays (`x[n]`, via `fEntryOffset`), `TLeafC` strings,
//! unsplit `std::vector<T>` (`TBranchElement`), and split
//! `std::vector<MyStruct>` exposed as per-member sub-branches (`hits.x`, …) —
//! across any number of baskets, compressed or not. Split writing generates a
//! `TStreamerInfo` for the element struct so the file is self-describing.

mod basket;
mod entrylist;
mod merge;
mod reader;
mod streamer_gen;
mod value;
mod writer;

pub use entrylist::EntryList;
pub use reader::{BranchLayout, Friend, TreeIndex, TreeReader};
pub use value::LeafType;

/// The `TTree` format's own reader and writer types: typed branch values,
/// branch constructors, the streaming writer, and the functions that lay a
/// tree out in a file.
///
/// `oxiroot-dataset` is the API for reading and writing columns, in a tree
/// or an `RNTuple` alike. These are what it is built on, for tools that work
/// at the level of the format (a merger, an inspector).
pub mod raw {
    pub use crate::merge::append_trees;
    pub use crate::reader::BranchRead;
    pub use crate::value::{BranchValues, FromBranch, Jagged};
    pub use crate::writer::raw::*;
    pub use crate::writer::{Branch, SplitMember, TreeWriter};
}
