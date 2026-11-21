//! ROOT compression framing (the 9-byte block header) plus codec backends.
//!
//! ROOT stores compressed payloads as a sequence of independently-compressed
//! blocks, each prefixed by a 9-byte `header`. This crate is a leaf dependency
//! of the rest of the workspace and owns the choice of codec backends.
//!
//! **Decode and encode** are both implemented for Zstd, zlib, LZ4, and LZMA
//! (XZ) — every codec ROOT writes except the legacy `CS`. All backends are pure
//! Rust and validated against real ROOT output.

mod codec;
mod header;
mod setting;
pub use header::{Algorithm, BlockHeader, HDR_SIZE, MAX_CHUNK_SIZE};
pub use setting::Compression;

use std::fmt;

/// Errors raised while (de)compressing ROOT payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompressError {
    /// The input was shorter than required to read a header or block payload.
    Truncated {
        /// Bytes the codec needed.
        needed: usize,
        /// Bytes available in the input.
        available: usize,
    },
    /// Decompression produced a different number of bytes than expected.
    SizeMismatch {
        /// Uncompressed size the header declared.
        expected: usize,
        /// Bytes the codec actually produced.
        got: usize,
    },
    /// A block uses an algorithm whose codec is not compiled in yet.
    CodecUnavailable(Algorithm),
    /// The underlying codec reported an error.
    Codec(String),
}

impl fmt::Display for CompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated compressed data: needed {needed} bytes, {available} available"
                )
            }
            Self::SizeMismatch { expected, got } => {
                write!(
                    f,
                    "decompressed size mismatch: expected {expected} bytes, got {got}"
                )
            }
            Self::CodecUnavailable(algo) => {
                write!(f, "no codec available for algorithm {algo:?}")
            }
            Self::Codec(msg) => write!(f, "codec error: {msg}"),
        }
    }
}

impl std::error::Error for CompressError {}

/// Decompress `src` into exactly `uncompressed_len` bytes.
///
/// When `src.len() == uncompressed_len` the payload is taken to be stored
/// uncompressed (no block header) and returned verbatim. Otherwise `src` is
/// parsed as a sequence of ROOT compression blocks until `uncompressed_len`
/// bytes have been produced.
pub fn decompress(src: &[u8], uncompressed_len: usize) -> Result<Vec<u8>, CompressError> {
    if src.len() == uncompressed_len {
        return Ok(src.to_vec());
    }

    // `uncompressed_len` comes from the file (an anchor 48-bit length or a Key
    // `fObjLen`), so it is untrusted. Cap the *initial* reservation so a forged
    // length cannot trigger a multi-GB allocation; the buffer still grows to fit
    // legitimately larger output, and the final length is checked below.
    const MAX_PREALLOC: usize = 64 << 20;
    let mut out = Vec::with_capacity(uncompressed_len.min(MAX_PREALLOC));
    let mut cur = src;
    while out.len() < uncompressed_len {
        let hdr = BlockHeader::parse(cur)?;
        // A block may not claim more output than the payload still has room for.
        // This rejects a forged `uncompressed_size` *before* its (possibly large)
        // codec buffer is allocated, and bounds total growth to `uncompressed_len`
        // so the loop cannot be driven to amplify output past the declared size.
        let remaining = uncompressed_len - out.len();
        if hdr.uncompressed_size as usize > remaining {
            return Err(CompressError::SizeMismatch {
                expected: uncompressed_len,
                got: out.len() + hdr.uncompressed_size as usize,
            });
        }
        let payload_end = HDR_SIZE + hdr.compressed_size as usize;
        if cur.len() < payload_end {
            return Err(CompressError::Truncated {
                needed: payload_end,
                available: cur.len(),
            });
        }
        let payload = &cur[HDR_SIZE..payload_end];
        out.extend_from_slice(&decompress_block(&hdr, payload)?);
        cur = &cur[payload_end..];
    }

    if out.len() != uncompressed_len {
        return Err(CompressError::SizeMismatch {
            expected: uncompressed_len,
            got: out.len(),
        });
    }
    Ok(out)
}

fn decompress_block(hdr: &BlockHeader, payload: &[u8]) -> Result<Vec<u8>, CompressError> {
    let n = hdr.uncompressed_size as usize;
    let out = match hdr.algorithm() {
        Algorithm::Zstd => codec::zstd_decode(payload, n)?,
        Algorithm::Zlib => codec::zlib_decode(payload, n)?,
        Algorithm::Lz4 => codec::lz4_decode(payload, n)?,
        Algorithm::Lzma => codec::lzma_decode(payload, n)?,
        // The legacy `CS` ("old ROOT") codec and unknown tags remain unhandled.
        algo => return Err(CompressError::CodecUnavailable(algo)),
    };
    if out.len() != n {
        return Err(CompressError::SizeMismatch {
            expected: n,
            got: out.len(),
        });
    }
    Ok(out)
}

/// Compress `src` with `compression`.
///
/// [`Compression::Stored`] means "store uncompressed": the input is returned
/// unchanged (the caller stores it without a block header). Otherwise the data
/// is encoded into ROOT compression blocks. The level tunes zlib, selects the
/// XZ preset for LZMA, and for Zstd is mapped onto the encoder's four levels
/// (ROOT's 4 and above all use the highest). The LZ4 encoder has one level and
/// ignores it.
///
/// # Errors
///
/// [`CompressError::Codec`] if an encoder fails or a block does not fit ROOT's
/// 24-bit block size.
pub fn compress(src: &[u8], compression: Compression) -> Result<Vec<u8>, CompressError> {
    // (tag, method byte) for each encoder, matching ROOT's framing.
    let (tag, method, level): ([u8; 2], u8, u32) = match compression {
        Compression::Stored => return Ok(src.to_vec()),
        Compression::Zlib(level) => (*b"ZL", 8, level), // Z_DEFLATED
        Compression::Lzma(level) => (*b"XZ", 0, level), // an XZ stream, per R__zipLZMA
        Compression::Lz4(level) => (*b"L4", 1, level),  // version byte 1
        Compression::Zstd(level) => (*b"ZS", 1, level),
    };
    let level = u8::try_from(level.min(99)).unwrap_or(99);

    let mut out = Vec::new();
    for chunk in src.chunks(MAX_CHUNK_SIZE.max(1)) {
        let frame = match compression {
            Compression::Stored => unreachable!("returned above"),
            Compression::Zlib(_) => codec::zlib_encode(chunk, level),
            Compression::Lzma(_) => codec::lzma_encode(chunk, level)?,
            Compression::Lz4(_) => codec::lz4_encode(chunk),
            Compression::Zstd(_) => codec::zstd_encode(chunk, level)?,
        };
        if frame.len() > MAX_CHUNK_SIZE {
            return Err(CompressError::Codec(
                "compressed block exceeds 24-bit size".into(),
            ));
        }
        BlockHeader {
            tag,
            method,
            // Both checked above to fit 24 bits.
            compressed_size: u32::try_from(frame.len()).unwrap_or(u32::MAX),
            uncompressed_size: u32::try_from(chunk.len()).unwrap_or(u32::MAX),
        }
        .write(&mut out);
        out.extend_from_slice(&frame);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compression a ROOT setting integer names.
    fn setting(setting: u32) -> Compression {
        Compression::try_from(setting).expect("a known algorithm")
    }

    #[test]
    fn decompress_uncompressed_passthrough() {
        let data = b"hello root".to_vec();
        let out = decompress(&data, data.len()).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn compress_uncompressed_passthrough() {
        let data = b"hello root".to_vec();
        assert_eq!(compress(&data, setting(0)).unwrap(), data);
    }

    #[test]
    fn compress_zstd_round_trips() {
        // Zstd level 5 -> settings 505. Compress, then decompress back.
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(40);
        let compressed = compress(&data, setting(505)).unwrap();
        // The first block must carry the Zstd tag.
        assert_eq!(&compressed[0..2], b"ZS");
        let out = decompress(&compressed, data.len()).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn decompress_reports_unavailable_codec() {
        // The legacy "CS" (old-ROOT) codec is still unhandled, so such a block
        // must report the codec as unavailable rather than silently mis-decoding.
        let mut buf = Vec::new();
        BlockHeader {
            tag: *b"CS",
            method: 1,
            compressed_size: 4,
            uncompressed_size: 8,
        }
        .write(&mut buf);
        buf.extend_from_slice(&[0, 1, 2, 3]);
        assert!(matches!(
            decompress(&buf, 8),
            Err(CompressError::CodecUnavailable(Algorithm::OldRoot))
        ));
    }

    #[test]
    fn compress_zlib_and_lz4_round_trip() {
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(40);
        for (settings, tag) in [(105u32, b"ZL"), (404, b"L4")] {
            let compressed = compress(&data, setting(settings)).unwrap();
            assert_eq!(&compressed[0..2], tag, "wrong block tag for {settings}");
            assert!(compressed.len() < data.len(), "should actually shrink");
            assert_eq!(decompress(&compressed, data.len()).unwrap(), data);
        }
    }

    /// A block whose payload expands far beyond its declared size (a
    /// decompression bomb) is rejected after producing at most one byte more
    /// than declared, not after inflating the whole payload.
    #[test]
    fn a_block_cannot_expand_past_its_declared_size() {
        let data = vec![0u8; 4 << 20]; // 4 MiB of zeros: a few KiB compressed
        for setting in [101, 505, 205] {
            let mut block = compress(&data, Compression::try_from(setting).unwrap()).unwrap();
            assert!(block.len() < data.len() / 64, "setting {setting}");
            // Declare 100 bytes in the header and ask for 100 bytes.
            block[6..9].copy_from_slice(&[100, 0, 0]);
            match decompress(&block, 100) {
                Err(CompressError::SizeMismatch { expected, got }) => {
                    assert_eq!(expected, 100, "setting {setting}");
                    assert!(got <= 101, "setting {setting}: inflated {got} bytes first");
                }
                other => panic!("setting {setting}: expected SizeMismatch, got {other:?}"),
            }
        }
        // LZ4 decodes into a buffer of the declared size and fails on overflow.
        let mut block = compress(&data, setting(404)).unwrap();
        block[6..9].copy_from_slice(&[100, 0, 0]);
        assert!(decompress(&block, 100).is_err());
    }

    #[test]
    fn lz4_block_rejects_a_corrupted_checksum() {
        // A valid LZ4 block whose stored XXH64 prefix has been flipped must be
        // rejected, not silently decoded.
        let data = b"oxiroot oxiroot oxiroot oxiroot".repeat(8);
        let mut block = compress(&data, setting(404)).unwrap();
        block[HDR_SIZE] ^= 0xff; // corrupt the first checksum byte
        assert!(matches!(
            decompress(&block, data.len()),
            Err(CompressError::Codec(_))
        ));
    }

    #[test]
    fn compress_lzma_round_trips() {
        // LZMA level 5 -> settings 205. The block must carry the "XZ" tag (a
        // complete XZ stream) and decode back to the original bytes.
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(40);
        let compressed = compress(&data, setting(205)).unwrap();
        assert_eq!(&compressed[0..2], b"XZ", "wrong block tag for LZMA");
        assert!(compressed.len() < data.len(), "should actually shrink");
        assert_eq!(decompress(&compressed, data.len()).unwrap(), data);
    }

    #[test]
    fn compress_chunks_large_input_into_multiple_blocks() {
        // Input larger than one 16 MiB chunk must be split into several ROOT
        // blocks on write and stitched back on read — neither the >chunk write
        // path nor the multi-block read path was previously exercised.
        let data: Vec<u8> = b"oxiroot "
            .iter()
            .copied()
            .cycle()
            .take(MAX_CHUNK_SIZE + 4096)
            .collect();
        let compressed = compress(&data, setting(505)).unwrap();

        let mut blocks = 0;
        let mut cur = &compressed[..];
        while !cur.is_empty() {
            let hdr = BlockHeader::parse(cur).unwrap();
            blocks += 1;
            cur = &cur[HDR_SIZE + hdr.compressed_size as usize..];
        }
        assert!(blocks >= 2, "expected multiple blocks, got {blocks}");
        assert_eq!(decompress(&compressed, data.len()).unwrap(), data);
    }

    #[test]
    fn decompress_rejects_block_overshooting_declared_length() {
        // A real zlib block, but its header lies that it expands to 64 MiB while
        // the caller only expects a few bytes. The per-block bound must reject it
        // before allocating, not decode the whole thing first.
        let original = b"small".to_vec();
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&original, 6);
        let mut block = Vec::new();
        BlockHeader {
            tag: *b"ZL",
            method: 8,
            compressed_size: u32::try_from(compressed.len()).unwrap(),
            uncompressed_size: 64 << 20, // lie: claims 64 MiB
        }
        .write(&mut block);
        block.extend_from_slice(&compressed);

        assert!(matches!(
            decompress(&block, original.len()),
            Err(CompressError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn decompress_zlib_block_round_trips() {
        // Build a ROOT "ZL" block by hand (zlib stream behind the 9-byte header)
        // and confirm we decode it back to the original bytes.
        let original = b"the quick brown fox jumps over the lazy dog. ".repeat(20);
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&original, 6);

        let mut block = Vec::new();
        BlockHeader {
            tag: *b"ZL",
            method: 8, // Z_DEFLATED
            compressed_size: u32::try_from(compressed.len()).unwrap(),
            uncompressed_size: u32::try_from(original.len()).unwrap(),
        }
        .write(&mut block);
        block.extend_from_slice(&compressed);

        let out = decompress(&block, original.len()).unwrap();
        assert_eq!(out, original);
    }
}
