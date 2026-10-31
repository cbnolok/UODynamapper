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
use std::io::{self, Read};

/// The zlib level of the client's streams (header `78 9C`). Classic zlib (`libz-sys`, built from source) at this
/// level, with the default window, memory level and strategy that `compress2` uses, reproduces every zlib entry of
/// the Enhanced Client packages byte for byte; flate2's zlib-ng backend does not (see ENHANCED_CLIENT_FILES.md).
const CLIENT_ZLIB_LEVEL: i32 = 6;

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
        UopCompression::Zlib => compress_classic_zlib(payload, CLIENT_ZLIB_LEVEL),
        UopCompression::Mythic => {
            crate::uop_container::compression::mythic_decompress::compress_with_header(payload)
        }
        UopCompression::ZlibBwt => crate::uop_container::compression::zlib_bwt_codec::compress(payload),
    }
}

/// `payload` as a zlib stream from classic zlib's `compress2` at `level`.
fn compress_classic_zlib(payload: &[u8], level: i32) -> io::Result<Vec<u8>> {
    let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "payload too large for zlib");
    let source_length = libz_sys::uLong::try_from(payload.len()).map_err(|_| too_large())?;
    // SAFETY: `compressBound` only computes a size from its argument.
    let bound = unsafe { libz_sys::compressBound(source_length) };
    let mut encoded = vec![0u8; usize::try_from(bound).map_err(|_| too_large())?];
    let mut encoded_length: libz_sys::uLongf = bound;
    // SAFETY: `encoded` is writable for `encoded_length` bytes and `payload` readable for `source_length` bytes;
    // `compress2` writes the stream length back to `encoded_length`.
    let status = unsafe {
        libz_sys::compress2(encoded.as_mut_ptr(), &mut encoded_length, payload.as_ptr(), source_length, level)
    };
    if status != libz_sys::Z_OK {
        return Err(io::Error::other(format!("zlib compress2 failed with status {status}")));
    }
    encoded.truncate(usize::try_from(encoded_length).map_err(|_| too_large())?);
    Ok(encoded)
}

pub fn decode_payload(
    payload: &[u8],
    raw_size: usize,
    compression: UopCompression,
) -> io::Result<Vec<u8>> {
    match compression {
        UopCompression::None => Ok(payload.to_vec()),
        UopCompression::Zlib => {
            let mut decoder = ZlibDecoder::new(payload);
            let mut decoded = Vec::with_capacity(raw_size);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload that compresses: repeated records with a changing field.
    fn sample_payload() -> Vec<u8> {
        const RECORDS: u32 = 2_000;
        (0..RECORDS).flat_map(|index| [b"record ".as_slice(), &(index % 97).to_le_bytes()].concat()).collect()
    }

    #[test]
    fn zlib_writes_a_client_level_stream_that_round_trips() {
        /// zlib header of a level-6 stream with the default window: CMF `78`, FLG `9C`.
        const CLIENT_ZLIB_HEADER: [u8; 2] = [0x78, 0x9C];
        let payload = sample_payload();
        let encoded = encode_payload(&payload, UopCompression::Zlib).unwrap();
        assert_eq!(encoded[..CLIENT_ZLIB_HEADER.len()], CLIENT_ZLIB_HEADER);
        assert!(encoded.len() < payload.len());
        assert_eq!(decode_payload(&encoded, payload.len(), UopCompression::Zlib).unwrap(), payload);
        assert_eq!(decode_payload(&encode_payload(&[], UopCompression::Zlib).unwrap(), 0, UopCompression::Zlib).unwrap(), b"");
    }
}
