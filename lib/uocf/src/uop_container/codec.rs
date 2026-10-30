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
use std::ops::RangeInclusive;

/// zlib levels [`encode_payload`] tries; the smallest stream wins (the first level on a tie). The client stores
/// classic zlib level-6 streams (header `78 9C`), which this zlib-ng backend does not reproduce: its level 6 is
/// often a few bytes longer, which moves an edited entry out of its old place in a saved package. Trying levels 6
/// to 9 brings re-compressed client entries to their stored size or below for 35,176 of the 37,756 tileart
/// records (4,811 at level 6 alone) and 800 of the 863 MultiCollection entries (448); see ENHANCED_CLIENT_FILES.md.
const ZLIB_LEVELS: RangeInclusive<u32> = 6..=9;

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
            let mut smallest: Option<Vec<u8>> = None;
            for level in ZLIB_LEVELS {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level));
                encoder.write_all(payload)?;
                let encoded = encoder.finish()?;
                if smallest.as_ref().is_none_or(|best| encoded.len() < best.len()) {
                    smallest = Some(encoded);
                }
            }
            Ok(smallest.expect("ZLIB_LEVELS is not empty"))
        }
        UopCompression::Mythic => {
            crate::uop_container::compression::mythic_decompress::compress_with_header(payload)
        }
        UopCompression::ZlibBwt => crate::uop_container::compression::zlib_bwt_codec::compress(payload),
    }
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
    fn zlib_keeps_the_smallest_level() {
        let payload = sample_payload();
        let encoded = encode_payload(&payload, UopCompression::Zlib).unwrap();
        for level in ZLIB_LEVELS {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level));
            encoder.write_all(&payload).unwrap();
            assert!(encoded.len() <= encoder.finish().unwrap().len(), "level {level} is smaller");
        }
        assert_eq!(decode_payload(&encoded, payload.len(), UopCompression::Zlib).unwrap(), payload);
    }
}
