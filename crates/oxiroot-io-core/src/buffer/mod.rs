//! Read and write cursors for ROOT on-disk structures.

mod reader;
mod writer;

pub use reader::{K_BYTE_COUNT_MASK, RBuffer, VersionHeader};
pub use writer::{CountToken, Patch, WBuffer};
