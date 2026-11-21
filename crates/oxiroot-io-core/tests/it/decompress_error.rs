//! A payload that cannot be decompressed is a typed error that names what was
//! being read and keeps the codec's reason as its `source`.

use std::error::Error as _;

use oxiroot_compress::{Algorithm, CompressError};
use oxiroot_io_core::Error;
use oxiroot_io_core::raw::decompress_payload;

#[test]
fn an_unknown_codec_is_a_matchable_error() {
    // A block header with the legacy `CS` tag, which this crate cannot decode.
    let mut block = b"CS\x08".to_vec();
    block.extend_from_slice(&[4, 0, 0]); // compressed size
    block.extend_from_slice(&[8, 0, 0]); // uncompressed size
    block.extend_from_slice(&[0; 4]);
    let err = decompress_payload(&block, 8, "key \"h\"").unwrap_err();
    match &err {
        Error::Decompress {
            context,
            source: CompressError::CodecUnavailable(Algorithm::OldRoot),
        } => assert_eq!(context.as_deref(), Some("key \"h\"")),
        other => panic!("expected an unavailable-codec error, got {other:?}"),
    }
    assert!(
        err.to_string().starts_with("decompressing key \"h\": "),
        "{err}"
    );
    assert!(err.source().is_some());
}

#[test]
fn a_bare_compress_error_converts() {
    let err: Error = CompressError::Codec {
        algorithm: Algorithm::Zstd,
        source: "bad frame".into(),
    }
    .into();
    assert_eq!(
        err.to_string(),
        "decompression failed: Zstandard codec error: bad frame"
    );
    // The codec's own error is still there, two levels down.
    let codec = err.source().and_then(std::error::Error::source);
    assert_eq!(codec.map(ToString::to_string).as_deref(), Some("bad frame"));
}
