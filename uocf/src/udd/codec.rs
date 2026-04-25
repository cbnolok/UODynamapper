//! Shared UDD codec helpers.
//!
//! UDDP and UDDF both expose the same wrapper-level notion of payload
//! compression, but the current UDDF wrapper stores its content id as a
//! separate header field rather than packing it into the wrapper codec word.
//!
//! The current wrapper encoding is intentionally simple:
//! - bits 0..=15 : wrapper compression id

use std::io;

pub const UDDP_DEFAULT_ALIGNMENT: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UddpCompression {
    /// Store the payload exactly as provided.
    None,

    /// Zstd-compressed payload used by the current UDD wrappers.
    Zstd,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UddpContentId {
    #[default]
    Unknown = 0,
    Metadata = 1,
    Rgba8888 = 2,
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
    pub fn new(compression: UddpCompression) -> io::Result<Self> {
        let raw = match compression {
            UddpCompression::None => 0u16,
            UddpCompression::Zstd => 1u16,
        };

        Ok(Self(raw))
    }

    pub fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u16 {
        self.0
    }

    pub fn compression_id(self) -> u16 {
        self.0
    }

    pub fn compression(self) -> io::Result<UddpCompression> {
        match self.compression_id() {
            0 => Ok(UddpCompression::None),
            1 => Ok(UddpCompression::Zstd),
            compression_id => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported compression id {compression_id}"),
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

pub fn encode_payload(payload: &[u8], compression: UddpCompression) -> io::Result<Vec<u8>> {
    match compression {
        UddpCompression::None => Ok(payload.to_vec()),
        UddpCompression::Zstd => zstd::bulk::compress(payload, 3),
    }
}

pub fn decode_payload(
    payload: &[u8],
    raw_size: usize,
    compression: UddpCompression,
) -> io::Result<Vec<u8>> {
    match compression {
        UddpCompression::None => Ok(payload.to_vec()),
        UddpCompression::Zstd => zstd::bulk::decompress(payload, raw_size),
    }
}
