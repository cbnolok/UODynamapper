//! Shared binary codec helpers for UOP-compatible payloads and UDDP flag packing.
//!
//! Binary layout covered by this module:
//! - UDDP/UDDF use a packed `u16 codec_bits` field.
//! - Bit allocation:
//!   - bits `0..=3`: base compression id
//!   - bits `4..=6`: custom compression id
//!   - bits `7..=15`: content id
//! - The content id is not a bitmask of features: it identifies one concrete
//!   payload family, for example metadata or a specific raw pixel format.
//! - This module is the canonical encoder/decoder for that field and for the
//!   raw payload byte transforms selected by it.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};

pub const UDDP_DEFAULT_ALIGNMENT: u64 = 4096;
const MAX_CONTENT_ID: u16 = 0x01FF;
const BASE_MASK: u16 = 0x000F;
const CUSTOM_MASK: u16 = 0x0007;
const CONTENT_MASK: u16 = 0x01FF;
const CUSTOM_SHIFT: u16 = 4;
const CONTENT_SHIFT: u16 = 7;
const BASE_CUSTOM_SENTINEL: u8 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UddpCompression {
    None,
    Zlib,
    Mythic,
    ZlibBwt,
    Zstd,
    Lz4,
}

/// Semantic payload kinds currently reserved for the UDDP/UDDF content id field.
///
/// The on-disk format stores only a `u16`, but using named ids in code makes it
/// easier to keep package producers and consumers aligned on what a payload
/// actually contains.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UddpContentId {
    /// Unspecified raw bytes. Useful as a temporary fallback during migration.
    #[default]
    Unknown = 0,
    /// Generic metadata payloads: indices, tile metadata, bookkeeping blobs.
    Metadata = 1,
    /// Raw GPU/CPU pixel data in 8 bits per channel RGBA order.
    Rgba8888 = 2,
    /// Raw 16-bit pixels in RGBA5551 packing.
    Rgba5551 = 3,
}

impl UddpContentId {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Unknown),
            1 => Some(Self::Metadata),
            2 => Some(Self::Rgba8888),
            3 => Some(Self::Rgba5551),
            _ => None,
        }
    }
}

impl From<UddpContentId> for u16 {
    fn from(value: UddpContentId) -> Self {
        value.as_u16()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CodecBits(u16);

impl CodecBits {
    /// Pack compression selection and payload kind into the on-disk `u16` field.
    ///
    /// The resulting value is written verbatim into UDDP entry records and into
    /// the UDDF wrapper header.
    pub fn new(content_id: u16, compression: UddpCompression) -> io::Result<Self> {
        if content_id > MAX_CONTENT_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("content_id {content_id} exceeds 9-bit range"),
            ));
        }

        let (base, custom) = match compression {
            UddpCompression::None => (0u8, 0u8),
            UddpCompression::Zlib => (1u8, 0u8),
            UddpCompression::Mythic => (2u8, 0u8),
            UddpCompression::ZlibBwt => (3u8, 0u8),
            UddpCompression::Zstd => (BASE_CUSTOM_SENTINEL, 0u8),
            UddpCompression::Lz4 => (BASE_CUSTOM_SENTINEL, 1u8),
        };

        Ok(Self(
            base as u16
                | ((custom as u16) << CUSTOM_SHIFT)
                | (content_id << CONTENT_SHIFT),
        ))
    }

    /// Convenience helper for callers that want to avoid raw numeric content ids.
    pub fn new_typed(content_id: UddpContentId, compression: UddpCompression) -> io::Result<Self> {
        Self::new(content_id.into(), compression)
    }

    pub fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u16 {
        self.0
    }

    pub fn base(self) -> u8 {
        (self.0 & BASE_MASK) as u8
    }

    pub fn custom(self) -> u8 {
        ((self.0 >> CUSTOM_SHIFT) & CUSTOM_MASK) as u8
    }

    pub fn content_id(self) -> u16 {
        (self.0 >> CONTENT_SHIFT) & CONTENT_MASK
    }

    /// Best-effort typed view of the payload kind.
    ///
    /// Unknown numeric ids are preserved by `content_id()` and simply return
    /// `None` here, so extending the format later stays backward-compatible.
    pub fn typed_content_id(self) -> Option<UddpContentId> {
        UddpContentId::from_u16(self.content_id())
    }

    pub fn compression(self) -> io::Result<UddpCompression> {
        match self.base() {
            0 => Ok(UddpCompression::None),
            1 => Ok(UddpCompression::Zlib),
            2 => Ok(UddpCompression::Mythic),
            3 => Ok(UddpCompression::ZlibBwt),
            BASE_CUSTOM_SENTINEL => match self.custom() {
                0 => Ok(UddpCompression::Zstd),
                1 => Ok(UddpCompression::Lz4),
                custom => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported custom compression id {custom}"),
                )),
            },
            base => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported base compression id {base}"),
            )),
        }
    }
}

pub fn align_up(value: u64, alignment: u64) -> io::Result<u64> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("alignment {alignment} must be a non-zero power of two"),
        ));
    }

    Ok((value + (alignment - 1)) & !(alignment - 1))
}

/// Encode one payload blob exactly as it will be stored in the package data area.
///
/// No framing is added here beyond the chosen compression algorithm output. The
/// caller is responsible for writing sizes, checksums, offsets, and alignment in
/// the container metadata.
pub fn encode_payload(payload: &[u8], compression: UddpCompression) -> io::Result<Vec<u8>> {
    match compression {
        UddpCompression::None => Ok(payload.to_vec()),
        UddpCompression::Zlib => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(payload)?;
            encoder.finish()
        }
        UddpCompression::Mythic => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Mythic encoding is not implemented",
        )),
        UddpCompression::ZlibBwt => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Zlib+BWT encoding is not implemented",
        )),
        UddpCompression::Zstd => zstd::bulk::compress(payload, 3),
        UddpCompression::Lz4 => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "LZ4 support is reserved but not implemented yet",
        )),
    }
}

/// Decode one payload blob previously stored in a UOP/UDDP/UDDF data area.
///
/// `raw_size` is the expected decompressed size recorded in the surrounding
/// metadata and is required by some codecs such as Zstd.
pub fn decode_payload(
    payload: &[u8],
    raw_size: usize,
    compression: UddpCompression,
) -> io::Result<Vec<u8>> {
    match compression {
        UddpCompression::None => Ok(payload.to_vec()),
        UddpCompression::Zlib => {
            let mut decoder = ZlibDecoder::new(payload);
            let mut decoded = Vec::with_capacity(raw_size);
            decoder.read_to_end(&mut decoded)?;
            Ok(decoded)
        }
        UddpCompression::Mythic => {
            crate::uop::compression::mythic_decompress::decompress_with_header(payload)
        }
        UddpCompression::ZlibBwt => crate::uop::compression::zlib_bwt_codec::decompress(payload),
        UddpCompression::Zstd => zstd::bulk::decompress(payload, raw_size),
        UddpCompression::Lz4 => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "LZ4 support is reserved but not implemented yet",
        )),
    }
}
