//! Codec backends for ROOT compression blocks.
//!
//! All pure-Rust: Zstd via `zrip`, zlib via `miniz_oxide`, LZ4 via `lz4_flex`
//! (block format), and LZMA via `lzma-rust2` (an XZ stream). Every algorithm
//! ROOT writes can be both decoded and encoded here except the legacy `CS`,
//! which ROOT itself no longer writes.

use std::io::{Read, Write};

use xxhash_rust::xxh64::xxh64;

use crate::{Algorithm, CompressError};

thread_local! {
    /// One Zstd decoder per thread: its tables and workspace are reused from
    /// block to block instead of being allocated for each.
    static ZSTD_DECODER: std::cell::RefCell<zrip::DecompressContext> =
        std::cell::RefCell::new(zrip::DecompressContext::new());
    /// One Zstd encoder per thread, likewise; its level is set per block.
    static ZSTD_ENCODER: std::cell::RefCell<Option<zrip::CompressContext>> =
        const { std::cell::RefCell::new(None) };
}

/// Decode a single Zstd-compressed block payload (a standard Zstd frame) onto
/// the end of `out`, producing at most `uncompressed_size` bytes (the block header's declared
/// size, at most 16 MiB): a frame that would expand further fails with
/// [`CompressError::SizeMismatch`] before it is decoded whole.
pub(crate) fn zstd_decode_into(
    payload: &[u8],
    uncompressed_size: usize,
    out: &mut Vec<u8>,
) -> Result<(), CompressError> {
    ZSTD_DECODER
        .with_borrow_mut(|d| d.decompress_into_with_limit(payload, out, uncompressed_size))
        .map_err(|e| match e {
            zrip::DecompressError::OutputTooSmall => CompressError::SizeMismatch {
                expected: uncompressed_size,
                got: uncompressed_size + 1,
            },
            e => CompressError::codec(Algorithm::Zstd, e),
        })?;
    Ok(())
}

/// Decode a single zlib-compressed block payload (a standard zlib stream),
/// producing at most `uncompressed_size` bytes.
pub(crate) fn zlib_decode(
    payload: &[u8],
    uncompressed_size: usize,
) -> Result<Vec<u8>, CompressError> {
    use miniz_oxide::inflate::TINFLStatus;
    match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(payload, uncompressed_size) {
        Ok(out) => Ok(out),
        // The stream holds more than the header declared: stop instead of
        // inflating it, so a small block cannot expand into a huge buffer.
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(CompressError::SizeMismatch {
            expected: uncompressed_size,
            got: uncompressed_size + 1,
        }),
        Err(e) => Err(CompressError::codec(Algorithm::Zlib, format!("{e:?}"))),
    }
}

/// Read a streaming decoder to the end, producing at most `limit` bytes.
///
/// `limit` is the block header's declared (untrusted, but at most 16 MiB)
/// size. Reading stops one byte past it, so a stream that would expand further
/// fails with [`CompressError::SizeMismatch`] instead of allocating its whole
/// output first.
fn read_bounded(
    decoder: &mut impl Read,
    limit: usize,
    algorithm: Algorithm,
) -> Result<Vec<u8>, CompressError> {
    let mut out = Vec::with_capacity(limit.min(crate::MAX_CHUNK_SIZE));
    decoder
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| CompressError::codec(algorithm, e))?;
    if out.len() > limit {
        return Err(CompressError::SizeMismatch {
            expected: limit,
            got: out.len(),
        });
    }
    Ok(out)
}

/// Decode a ROOT LZ4 block payload: an 8-byte big-endian XXH64 checksum (over
/// the *compressed* bytes, seed 0) followed by an LZ4 block. `uncompressed_size`
/// is the block header's declared output size (always ≤ 16 MiB, so it safely
/// bounds the codec's allocation).
pub(crate) fn lz4_decode(
    payload: &[u8],
    uncompressed_size: usize,
) -> Result<Vec<u8>, CompressError> {
    let Some((checksum, data)) = payload.split_first_chunk::<8>() else {
        return Err(CompressError::Truncated {
            needed: 8,
            available: payload.len(),
        });
    };
    let stored = u64::from_be_bytes(*checksum);
    if xxh64(data, 0) != stored {
        return Err(CompressError::codec(
            Algorithm::Lz4,
            "xxh64 checksum mismatch",
        ));
    }
    // `decompress_into` reports a too-small output as an error, where
    // `decompress` may panic, so a forged size cannot crash the reader.
    let mut out = vec![0u8; uncompressed_size];
    let n = lz4_flex::block::decompress_into(data, &mut out)
        .map_err(|e| CompressError::codec(Algorithm::Lz4, e))?;
    out.truncate(n);
    Ok(out)
}

/// Decode a ROOT LZMA block payload (a complete XZ stream). `uncompressed_size`
/// is the block header's declared output size (always ≤ 16 MiB, so it bounds the
/// speculative allocation; the caller verifies the produced length afterward).
pub(crate) fn lzma_decode(
    payload: &[u8],
    uncompressed_size: usize,
) -> Result<Vec<u8>, CompressError> {
    // A ROOT block holds exactly one XZ stream, so `allow_multiple_streams=false`.
    let mut reader = lzma_rust2::XzReader::new(payload, false);
    read_bounded(&mut reader, uncompressed_size, Algorithm::Lzma)
}

/// Encode `data` as a single zlib stream — the payload of a ROOT `ZL` block.
/// `level` is ROOT's 1..=9, clamped to the `miniz_oxide` 0..=10 range.
pub(crate) fn zlib_encode(data: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, level.min(10))
}

/// Encode `data` as a ROOT LZ4 block payload: the 8-byte big-endian XXH64
/// checksum of the compressed bytes, then the LZ4 block. Matches ROOT's framing
/// so official ROOT and uproot read it back.
pub(crate) fn lz4_encode(data: &[u8]) -> Vec<u8> {
    let compressed = lz4_flex::block::compress(data);
    let mut out = Vec::with_capacity(8 + compressed.len());
    out.extend_from_slice(&xxh64(&compressed, 0).to_be_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// The zrip level for ROOT's Zstd level `level` (1..=9). zrip's levels stop
/// at 4, which already compresses physics data about as well as C zstd's
/// level 5, ROOT's default; ROOT's higher levels use it too.
fn zrip_level(level: u8) -> i32 {
    i32::from(level.clamp(1, 4))
}

/// Encode `data` as a single standard Zstd frame (pure-Rust `zrip`). The frame
/// is what ROOT stores after a block's 9-byte header.
pub(crate) fn zstd_encode(data: &[u8], level: u8) -> Result<Vec<u8>, CompressError> {
    let level = zrip_level(level);
    ZSTD_ENCODER.with_borrow_mut(|slot| {
        let encoder = match slot {
            Some(e) => {
                e.set_level(level)
                    .map_err(|e| CompressError::codec(Algorithm::Zstd, e))?;
                e
            }
            None => slot.insert(
                zrip::CompressContext::new(level)
                    .map_err(|e| CompressError::codec(Algorithm::Zstd, e))?,
            ),
        };
        encoder
            .compress(data)
            .map(std::borrow::Cow::into_owned)
            .map_err(|e| CompressError::codec(Algorithm::Zstd, e))
    })
}

/// Encode `data` as a ROOT LZMA block payload: a complete XZ stream (pure-Rust
/// `lzma-rust2`). ROOT writes exactly this — an `.xz` container — after a block's
/// 9-byte `XZ` header, so official ROOT and uproot read it back.
///
/// `level` is ROOT's 1..=9, mapped straight onto the XZ preset (also 0..=9), so
/// the ratio actually tracks the requested level.
pub(crate) fn lzma_encode(data: &[u8], level: u8) -> Result<Vec<u8>, CompressError> {
    // XZ has presets 0 to 9; a higher ROOT level means "the strongest".
    let options = lzma_rust2::XzOptions::with_preset(u32::from(level.min(9)));
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), options)
        .map_err(|e| CompressError::codec(Algorithm::Lzma, e))?;
    writer
        .write_all(data)
        .map_err(|e| CompressError::codec(Algorithm::Lzma, e))?;
    writer
        .finish()
        .map_err(|e| CompressError::codec(Algorithm::Lzma, e))
}
