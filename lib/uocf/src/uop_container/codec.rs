//! Shared payload codec helpers for legacy UOP files.
//!
//! UOP file entries store a compact `i16 compression_flag` in their per-file
//! metadata. This module owns the byte-level encode/decode path for the legacy
//! UOP payload algorithms only.
//!
//! Supported UOP codecs:
//! - `0`: raw bytes
//! - `1`: zlib
//! - `2`: Mythic
//! - `3`: zlib+bwt
//!
//! Deliberately not supported here:
//! - UDDP/UDDF codec bits
//! - Zstd or LZ4 custom extensions introduced during early UDD prototyping

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UopCompression {
    None,
    Zlib,
    Mythic,
    ZlibBwt,
}

pub fn encode_payload(payload: &[u8], compression: UopCompression) -> io::Result<Vec<u8>> {
    match compression {
        UopCompression::None => Ok(payload.to_vec()),
        UopCompression::Zlib => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(payload)?;
            encoder.finish()
        }
        UopCompression::Mythic => {
            crate::uop_container::compression::mythic_decompress::compress_with_header(payload)
        }
        UopCompression::ZlibBwt => crate::uop_container::compression::zlib_bwt_codec::compress(payload),
    }
}

pub fn decode_payload(
    payload: &[u8],
    _raw_size: usize,
    compression: UopCompression,
) -> io::Result<Vec<u8>> {
    match compression {
        UopCompression::None => Ok(payload.to_vec()),
        UopCompression::Zlib => {
            let mut decoder = ZlibDecoder::new(payload);
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded)?;
            Ok(decoded)
        }
        UopCompression::Mythic => decode_mythic_payload(payload),
        UopCompression::ZlibBwt => crate::uop_container::compression::zlib_bwt_codec::decompress(payload),
    }
}

fn decode_mythic_payload(payload: &[u8]) -> io::Result<Vec<u8>> {
    if let Ok(zlib_decoded) = decode_payload(payload, 0, UopCompression::Zlib) {
        if let Ok(decoded) = crate::uop_container::compression::mythic_decompress::decompress_with_header(&zlib_decoded) {
            return Ok(decoded);
        }
    }

    crate::uop_container::compression::mythic_decompress::decompress_with_header(payload)
}
