//! [`ByteSource`] — a random-access byte provider behind [`FileReader`].
//!
//! A ROOT file is read by seeking to absolute offsets (a key's `fSeekKey`, an
//! `RNTuple` page locator, a `TBasket` seek) and pulling out a contiguous range.
//! Abstracting that single operation — "give me `len` bytes at `offset`" — lets
//! the same readers run against an in-memory buffer, a memory map, a local file
//! read positionally, or (with the `http` feature) a remote file fetched with
//! HTTP byte-range requests, downloading only the ranges actually touched.
//!
//! Ranges are returned as [`bytes::Bytes`]: for the in-memory and mmap backings
//! this is a zero-copy view sharing the underlying allocation, so the resident
//! read path stays copy-free.

use crate::cast::SaturatingFrom;
use bytes::Bytes;

use crate::error::{Error, Result};

/// A random-access source of a ROOT file's bytes.
///
/// Implementations must be cheap to share across threads (`Send + Sync`) — the
/// `TTree` reader fetches baskets in parallel under the `rayon` feature.
pub trait ByteSource: Send + Sync + std::fmt::Debug {
    /// Total length of the file in bytes.
    fn len(&self) -> u64;

    /// Whether the file is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read exactly `len` bytes starting at absolute `offset`, or error if the
    /// requested range falls outside the file.
    ///
    /// # Errors
    ///
    /// [`Error::UnexpectedEof`] if the range runs past the end of the source, and any error
    /// reading it.
    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes>;

    /// Read several ranges, returning their bytes in the same order. The
    /// default reads them one at a time; a remote source overrides it to fetch
    /// them in fewer round trips (an `XRootD` vector read, concurrent HTTP
    /// requests).
    ///
    /// # Errors
    ///
    /// As [`read_at`](Self::read_at), for the first range that fails.
    fn read_many(&self, ranges: &[(u64, usize)]) -> Result<Vec<Bytes>> {
        ranges.iter().map(|&(o, l)| self.read_at(o, l)).collect()
    }

    /// Whether batching reads with [`read_many`](ByteSource::read_many) saves
    /// time — true for network sources, where each read is a round trip.
    fn batches_reads(&self) -> bool {
        false
    }

    /// Announce the ranges about to be read, in order (a branch's baskets, a
    /// column's pages), so a source that batches can fetch them together. A
    /// hint only: reads outside the plan, or in another order, still work.
    fn plan_reads(&self, ranges: &[(u64, usize)]) {
        let _ = ranges;
    }
}

/// Bounds-check `[offset, offset + len)` against a file of `total` bytes,
/// returning the `usize` `(start, end)` on success. Never overflows `usize`.
fn checked_range(offset: u64, len: usize, total: u64) -> Result<(usize, usize)> {
    let end = offset
        .checked_add(len as u64)
        .filter(|&e| e <= total)
        .ok_or_else(|| Error::UnexpectedEof {
            needed: len,
            available: usize::saturating_from(total.saturating_sub(offset)),
        })?;
    // `end <= total` and `total` came from a real file/response length, so both
    // fit `usize` on any target that could hold the file in the first place.
    let start = usize::try_from(offset).map_err(|_| Error::UnexpectedEof {
        needed: len,
        available: 0,
    })?;
    let end = usize::try_from(end).map_err(|_| Error::UnexpectedEof {
        needed: len,
        available: 0,
    })?;
    Ok((start, end))
}

/// The whole file resident in memory as [`Bytes`]; `read_at` is a zero-copy
/// slice. Backs [`FileReader::from_bytes`](super::reader::FileReader::from_bytes) and the
/// default [`FileReader::open`](super::reader::FileReader::open).
#[derive(Debug)]
pub struct BytesSource(Bytes);

impl BytesSource {
    /// Wrap an owned in-memory buffer.
    pub fn new(data: impl Into<Bytes>) -> Self {
        Self(data.into())
    }
}

impl ByteSource for BytesSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        let (start, end) = checked_range(offset, len, self.0.len() as u64)?;
        Ok(self.0.slice(start..end))
    }
}

/// A memory-mapped file; `read_at` copies the requested range out of the map.
/// Hand one to [`FileReader::from_source`](super::reader::FileReader::from_source).
#[cfg(feature = "mmap")]
#[derive(Debug)]
pub struct MmapSource(memmap2::Mmap);

#[cfg(feature = "mmap")]
impl MmapSource {
    /// Wrap a read-only memory map of the whole file.
    #[must_use]
    pub fn new(map: memmap2::Mmap) -> Self {
        Self(map)
    }

    /// Memory-map the file at `path`.
    ///
    /// # Safety
    ///
    /// The file must not be modified or truncated, by this process or any
    /// other, while the source (or a reader built on it) is alive. A change
    /// underneath the map is undefined behaviour, and a truncation can crash
    /// the process with `SIGBUS`. Only map files you control.
    ///
    /// # Errors
    ///
    /// If the file cannot be opened or mapped.
    #[allow(unsafe_code)]
    pub unsafe fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the caller upholds the no-modification contract above. This
        // is the only `unsafe` in the workspace (`unsafe_code` is denied
        // everywhere else); memmap2's `map` is unavoidably `unsafe`.
        Ok(Self(unsafe { memmap2::Mmap::map(&file)? }))
    }
}

#[cfg(feature = "mmap")]
impl ByteSource for MmapSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        let (start, end) = checked_range(offset, len, self.0.len() as u64)?;
        Ok(Bytes::copy_from_slice(&self.0[start..end]))
    }
}

/// A local file read with positioned reads (`pread`/`seek_read`) — every
/// `read_at` touches only its range, so a large file is never slurped whole.
/// [`FileReader::open`](super::reader::FileReader::open) uses it for large files.
#[derive(Debug)]
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}

impl FileSource {
    /// Open `path` for positioned reads, recording its length up front.
    ///
    /// # Errors
    ///
    /// If the file cannot be opened or mapped.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        checked_range(offset, len, self.len)?;
        let mut buf = vec![0u8; len];
        read_exact_at(&self.file, &mut buf, offset)?;
        Ok(Bytes::from(buf))
    }
}

/// Fill `buf` from `file` starting at absolute `offset`, looping until full
/// (a positioned read may return fewer bytes than requested). Uses the
/// platform positioned-read primitive so no shared cursor is mutated — several
/// threads may read the same file concurrently.
fn read_exact_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = positioned_read(file, &mut buf[filled..], offset + filled as u64)?;
        if n == 0 {
            return Err(Error::UnexpectedEof {
                needed: buf.len(),
                available: filled,
            });
        }
        filled += n;
    }
    Ok(())
}

#[cfg(unix)]
fn positioned_read(file: &std::fs::File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::unix::fs::FileExt;
    Ok(file.read_at(buf, offset)?)
}

#[cfg(windows)]
fn positioned_read(file: &std::fs::File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::windows::fs::FileExt;
    Ok(file.seek_read(buf, offset)?)
}

#[cfg(not(any(unix, windows)))]
fn positioned_read(file: &std::fs::File, buf: &mut [u8], offset: u64) -> Result<usize> {
    // Fallback for exotic targets: clone the handle and seek+read. Correct but
    // not concurrency-friendly; unix/windows use true positioned reads above.
    use std::io::{Read, Seek, SeekFrom};
    let mut f = file.try_clone()?;
    f.seek(SeekFrom::Start(offset))?;
    Ok(f.read(buf)?)
}
