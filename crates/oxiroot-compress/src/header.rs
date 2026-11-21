//! ROOT's 9-byte compression block header.
//!
//! Every compressed ROOT payload — Key object data and `RNTuple` pages alike —
//! is a sequence of independently-compressed blocks, each prefixed by this
//! 9-byte header. A single block holds at most [`MAX_CHUNK_SIZE`] uncompressed
//! bytes; larger payloads are split across consecutive blocks.
//!
//! Layout (offsets within the 9-byte header):
//! - `[0..2]` two ASCII characters identifying the algorithm (e.g. `b"ZS"`)
//! - `[2]` method / version byte (algorithm-specific)
//! - `[3..6]` compressed payload size, 24-bit little-endian
//! - `[6..9]` uncompressed payload size, 24-bit little-endian

use crate::CompressError;

/// Size of the compression block header in bytes.
pub const HDR_SIZE: usize = 9;

/// Maximum uncompressed payload carried by a single block (`0xFFFFFF`, ~16 MiB).
pub const MAX_CHUNK_SIZE: usize = 0xFF_FFFF;

/// A ROOT compression algorithm, identified by the 2-character block tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Algorithm {
    /// zlib / DEFLATE (`b"ZL"`).
    Zlib,
    /// LZMA (`b"XZ"`).
    Lzma,
    /// Legacy ROOT algorithm (`b"CS"`).
    OldRoot,
    /// LZ4 (`b"L4"`).
    Lz4,
    /// Zstandard (`b"ZS"`).
    Zstd,
    /// Unrecognized tag.
    Unknown([u8; 2]),
}

impl Algorithm {
    /// Identify the algorithm from its 2-character block tag.
    #[must_use]
    pub fn from_tag(tag: [u8; 2]) -> Self {
        match &tag {
            b"ZL" => Self::Zlib,
            b"XZ" => Self::Lzma,
            b"CS" => Self::OldRoot,
            b"L4" => Self::Lz4,
            b"ZS" => Self::Zstd,
            _ => Self::Unknown(tag),
        }
    }

    /// The 2-character tag for this algorithm.
    #[must_use]
    pub fn tag(self) -> [u8; 2] {
        match self {
            Self::Zlib => *b"ZL",
            Self::Lzma => *b"XZ",
            Self::OldRoot => *b"CS",
            Self::Lz4 => *b"L4",
            Self::Zstd => *b"ZS",
            Self::Unknown(t) => t,
        }
    }
}

impl From<[u8; 2]> for Algorithm {
    /// The std-trait counterpart to [`from_tag`](Algorithm::from_tag) (total: an
    /// unrecognized tag becomes [`Algorithm::Unknown`]), so callers can `.into()`.
    fn from(tag: [u8; 2]) -> Self {
        Self::from_tag(tag)
    }
}

impl From<Algorithm> for [u8; 2] {
    /// The std-trait counterpart to [`tag`](Algorithm::tag).
    fn from(algorithm: Algorithm) -> [u8; 2] {
        algorithm.tag()
    }
}

/// A parsed ROOT compression block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    /// 2-character algorithm tag.
    pub tag: [u8; 2],
    /// Algorithm-specific method / version byte.
    pub method: u8,
    /// Size of the compressed payload immediately following this header.
    pub compressed_size: u32,
    /// Size of the payload once decompressed.
    pub uncompressed_size: u32,
}

impl BlockHeader {
    /// The algorithm this block was compressed with.
    #[must_use]
    pub fn algorithm(&self) -> Algorithm {
        Algorithm::from_tag(self.tag)
    }

    /// Parse a header from the first [`HDR_SIZE`] bytes of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, CompressError> {
        if buf.len() < HDR_SIZE {
            return Err(CompressError::Truncated {
                needed: HDR_SIZE,
                available: buf.len(),
            });
        }
        Ok(Self {
            tag: [buf[0], buf[1]],
            method: buf[2],
            compressed_size: u24_le(&buf[3..6]),
            uncompressed_size: u24_le(&buf[6..9]),
        })
    }

    /// Append the 9-byte header to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(self.tag[0]);
        out.push(self.tag[1]);
        out.push(self.method);
        put_u24_le(out, self.compressed_size);
        put_u24_le(out, self.uncompressed_size);
    }
}

fn u24_le(b: &[u8]) -> u32 {
    u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16)
}

fn put_u24_le(out: &mut Vec<u8>, v: u32) {
    out.push((v & 0xFF) as u8);
    out.push(((v >> 8) & 0xFF) as u8);
    out.push(((v >> 16) & 0xFF) as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_tag_round_trips() {
        for algo in [
            Algorithm::Zlib,
            Algorithm::Lzma,
            Algorithm::OldRoot,
            Algorithm::Lz4,
            Algorithm::Zstd,
        ] {
            assert_eq!(Algorithm::from_tag(algo.tag()), algo);
        }
        assert_eq!(Algorithm::from_tag(*b"??"), Algorithm::Unknown(*b"??"));
    }

    #[test]
    fn header_round_trips() {
        let hdr = BlockHeader {
            tag: *b"ZS",
            method: 1,
            compressed_size: 0x0001_2345,
            uncompressed_size: 0x0000_ABCD,
        };
        let mut buf = Vec::new();
        hdr.write(&mut buf);
        assert_eq!(buf.len(), HDR_SIZE);
        // 24-bit little-endian sizes.
        assert_eq!(&buf[3..6], &[0x45, 0x23, 0x01]);
        assert_eq!(&buf[6..9], &[0xCD, 0xAB, 0x00]);
        assert_eq!(BlockHeader::parse(&buf).unwrap(), hdr);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            BlockHeader::parse(&[0u8; 4]),
            Err(CompressError::Truncated {
                needed: HDR_SIZE,
                available: 4
            })
        ));
    }

    #[test]
    fn zstd_tag_is_zs() {
        assert_eq!(Algorithm::Zstd.tag(), *b"ZS");
    }
}
