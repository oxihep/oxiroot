//! The ROOT (TFile) on-disk container: header, keys, directories, free list,
//! and the [`FileReader`] reading entry point.

mod container;
mod directory;
mod free;
mod header;
#[cfg(feature = "http")]
mod http;
mod key;
mod readahead;
mod reader;
mod source;
mod writer;
#[cfg(feature = "xrootd")]
mod xrootd;
#[cfg(feature = "xrootd-tls")]
mod xrootd_tls;

pub(crate) use container::STREAMER_INFO_KEY_LEN;
pub use container::{
    ContainerWriter, DATIME, DirId, FILE_VERSION, KSTART_BIG_FILE, compress_if_smaller,
};
pub use directory::Directory;
pub use free::{FreeSegment, read_free};
pub use header::{BIG_FILE_VERSION, FileHeader, MAGIC, Uuid};
pub use key::{Datime, Key};
pub use reader::{FileReader, find_key, split_cycle};
#[cfg(feature = "mmap")]
pub use source::MmapSource;
pub use source::{ByteSource, BytesSource, FileSource};
pub use writer::{FileWriter, SubdirWriter};
#[cfg(feature = "xrootd")]
pub use xrootd::XrootdSource;
#[cfg(feature = "xrootd-tls")]
pub use xrootd_tls::XrootdOptions;
