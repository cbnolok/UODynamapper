//! Shared helper code for the UDDP implementation.
//!
//! These helpers are deliberately separated from the reader and builders so the
//! format-specific algorithms can talk in terms of logical records and payloads
//! while this module handles the repetitive mechanics:
//! - path normalization and canonical hashing
//! - Zstd helper wrappers
//! - tiny binary cursor and range checking utilities
//! - error types and small filesystem convenience wrappers

use std::fmt;
use std::ops::Range;
use std::path::Path;

use xxhash_rust::xxh64::xxh64;
use zstd::bulk::{Compressor, Decompressor};

use super::*;

const ZSTD_LEVEL: i32 = 9;


/// Pack the public metadata word stored in every file locator.
pub fn pack_meta32(data_type: u8, codec: Codec, delta32: u32) -> Meta32 {
    let ty = (data_type as u32) & 0x3F;
    let co = ((codec as u32) & 0x03) << 6;
    let delta_hi = ((delta32 >> 24) & 0xFF) << 8;
    ty | co | delta_hi
}

/// Extract the 6-bit type id from a packed metadata word.
#[inline(always)]
pub fn unpack_type(meta32: Meta32) -> u8 {
    (meta32 & 0x3F) as u8
}

/// Extract the 2-bit codec from a packed metadata word.
pub fn unpack_codec(meta32: Meta32) -> Codec {
    Codec::from_u8(((meta32 >> 6) & 0x03) as u8).expect("2-bit codec")
}

/// Extract the high 8 bits of the 32-bit compression delta.
#[inline(always)]
pub fn unpack_delta_hi8(meta32: Meta32) -> u32 {
    (meta32 >> 8) & 0xFF
}


/// Pack the payload offset and low 24 delta bits into the position word.
pub fn pack_pos64(offset: u64, delta32: u32) -> Pos64 {
    let offset_bits = offset & ((1u64 << 40) - 1);
    let delta_lo = ((delta32 & 0x00FF_FFFF) as u64) << 40;
    offset_bits | delta_lo
}

/// Extract the 40-bit payload offset from a packed position word.
#[inline(always)]
pub fn unpack_offset40(pos64: Pos64) -> u64 {
    pos64 & ((1u64 << 40) - 1)
}

/// Extract the low 24 bits of the 32-bit compression delta.
#[inline(always)]
pub fn unpack_delta_lo24(pos64: Pos64) -> u32 {
    (pos64 >> 40) as u32
}

/// Reconstruct the full 32-bit compression delta from the split fields.
pub fn unpack_delta32(meta32: Meta32, pos64: Pos64) -> u32 {
    (unpack_delta_hi8(meta32) << 24) | (unpack_delta_lo24(pos64) & 0x00FF_FFFF)
}

/// Recover the stored payload size from the raw size and packed delta.
pub fn reconstruct_stored_size(raw_size: u32, meta32: Meta32, pos64: Pos64) -> u32 {
    match unpack_codec(meta32) {
        Codec::None => raw_size,
        Codec::ZstdNoDict | Codec::ZstdTypeDict | Codec::JpegXl => {
            raw_size - unpack_delta32(meta32, pos64)
        }
    }
}

/// Normalize a virtual path before hashing.
///
/// Path-hash packages store only the hash, not the original string. Every
/// producer and consumer therefore has to normalize identically.
pub fn normalize_virtual_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut prev_slash = false;

    for ch in path.chars() {
        let ch = if ch == '\\' { '/' } else { ch };
        if ch == '/' {
            if !prev_slash {
                out.push('/');
                prev_slash = true;
            }
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }

    while out.starts_with("./") {
        out.drain(..2);
    }

    out
}

/// Hash a normalized virtual path with XXH64.
pub fn xxh64_virtual_path(path: &str) -> u64 {
    let normalized = normalize_virtual_path(path);
    xxh64(normalized.as_bytes(), 0)
}

/// Plain Zstd compression helper.
pub(crate) fn zstd_compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut compressor = Compressor::new(ZSTD_LEVEL)?;
    compressor.compress(data)
}

/// Dictionary-backed Zstd compression helper.
pub(crate) fn zstd_compress_with_dict(data: &[u8], dict: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut compressor = Compressor::with_dictionary(ZSTD_LEVEL, dict)?;
    compressor.compress(data)
}

/// Jxl compression helper.
pub(crate) fn jxl_compress(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    use jpegxl_rs::encoder_builder;
    use jpegxl_rs::encode::EncoderSpeed;

    let mut encoder = encoder_builder()
        .has_alpha(true)
        .lossless(true)
        .speed(EncoderSpeed::Falcon)
        .build()
        .map_err(|e| e.to_string())?;

    let encoded = encoder.encode::<u8, u8>(data, width, height)
        .map_err(|e| e.to_string())?;

    let mut final_payload = Vec::with_capacity(encoded.len() + 8);
    final_payload.extend_from_slice(&width.to_le_bytes());
    final_payload.extend_from_slice(&height.to_le_bytes());
    final_payload.extend_from_slice(&encoded);
    Ok(final_payload)
}

/// Jxl decompression helper.
pub(crate) fn jxl_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    use jpegxl_rs::decoder_builder;
    use jpegxl_rs::decode::Pixels;

    if data.len() < 8 {
        return Err("JXL payload too small for header".to_string());
    }

    // Header is ignored by decoder, it needs the raw stream
    let encoded = &data[8..];

    let decoder = decoder_builder()
        .build()
        .map_err(|e| e.to_string())?;

    let (_, pixels) = decoder.decode(encoded)
        .map_err(|e| e.to_string())?;
    
    // We expect RGBA8 (8-bit per channel)
    match pixels {
        Pixels::Uint8(v) => Ok(v),
        _ => Err("Unexpected JXL pixel format (expected 8-bit)".to_string()),
    }
}

/// Plain Zstd decompression helper.
pub(crate) fn zstd_decompress(data: &[u8], raw_size: usize) -> std::io::Result<Vec<u8>> {
    let mut decompressor = Decompressor::new()?;
    decompressor.decompress(data, raw_size)
}

/// Dictionary-backed Zstd decompression helper.
pub(crate) fn zstd_decompress_with_dict(
    data: &[u8],
    raw_size: usize,
    dict: &[u8],
) -> std::io::Result<Vec<u8>> {
    let mut decompressor = Decompressor::with_dictionary(dict)?;
    decompressor.decompress(data, raw_size)
}

/// Compute the canonical XXH64 package hash.
///
/// The hash is taken over the final on-disk package image with one exception:
/// the serialized `package_hash64` field itself is treated as zero. This breaks
/// the self-reference while still tying the hash to every other byte.
pub fn canonical_package_hash64(bytes: &[u8]) -> u64 {
    if bytes.len() < HEADER_PACKAGE_HASH_OFFSET + 8 {
        return xxh64(bytes, 0);
    }

    let mut state = xxhash_rust::xxh64::Xxh64::new(0);
    // Hash prefix before the package hash field.
    state.update(&bytes[..HEADER_PACKAGE_HASH_OFFSET]);
    // Hash 8 zero bytes instead of the actual embedded hash.
    state.update(&[0u8; 8]);
    // Hash everything after the hash field.
    state.update(&bytes[HEADER_PACKAGE_HASH_OFFSET + 8..]);
    state.digest()
}

/// Patch the serialized header hash field in-place.
pub(crate) fn patch_header_package_hash64(bytes: &mut [u8], hash: u64) -> Result<(), BuildError> {
    if bytes.len() < HEADER_PACKAGE_HASH_OFFSET + 8 {
        return Err(BuildError::MalformedOutput);
    }
    bytes[HEADER_PACKAGE_HASH_OFFSET..HEADER_PACKAGE_HASH_OFFSET + 8]
        .copy_from_slice(&hash.to_le_bytes());
    Ok(())
}

/// Patch the 4-byte serialized package magic in-place.
pub(crate) fn patch_header_magic(bytes: &mut [u8], magic: u32) -> Result<(), BuildError> {
    if bytes.len() < 4 {
        return Err(BuildError::MalformedOutput);
    }
    bytes[0..4].copy_from_slice(&magic.to_le_bytes());
    Ok(())
}

pub(crate) fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

pub(crate) fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Tiny little-endian cursor over a borrowed byte slice.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn read_exact<const N: usize>(&mut self) -> Result<[u8; N], FormatError> {
        let end = self.pos.checked_add(N).ok_or(FormatError::Overflow)?;
        if end > self.bytes.len() {
            return Err(FormatError::Truncated);
        }

        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.read_exact::<1>()?[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.read_exact::<2>()?))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.read_exact::<4>()?))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64, FormatError> {
        Ok(u64::from_le_bytes(self.read_exact::<8>()?))
    }
}

/// Validate and materialize a byte range covering a fixed-count record array.
pub(crate) fn checked_region(
    start: usize,
    count: usize,
    elem_size: usize,
    total_len: usize,
) -> Result<Range<usize>, FormatError> {
    let size = count.checked_mul(elem_size).ok_or(FormatError::Overflow)?;
    let end = start.checked_add(size).ok_or(FormatError::Overflow)?;
    if end > total_len {
        return Err(FormatError::Truncated);
    }
    Ok(start..end)
}

/// Structural parse and decode errors for UDDP containers.
#[derive(Debug)]
pub enum FormatError {
    InvalidMagic(u32),
    InvalidLookupMode(u8),
    InvalidCodec(u8),
    MissingDictionary(u8),
    FileNotFound,
    WrongLookupMode,
    Truncated,
    Overflow,
    UnsupportedCodec,
    Io(std::io::Error),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic(v) => write!(f, "invalid magic: {v:#010x}"),
            Self::InvalidLookupMode(v) => write!(f, "invalid lookup mode: {v}"),
            Self::InvalidCodec(v) => write!(f, "invalid codec: {v}"),
            Self::MissingDictionary(t) => write!(f, "missing dictionary for type {t}"),
            Self::FileNotFound => write!(f, "file not found"),
            Self::WrongLookupMode => write!(f, "wrong lookup mode for requested operation"),
            Self::Truncated => write!(f, "truncated data"),
            Self::Overflow => write!(f, "integer overflow"),
            Self::UnsupportedCodec => write!(f, "unsupported codec"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Errors produced while building runtime or patch packages.
#[derive(Debug)]
pub enum BuildError {
    MissingIdForSparseMode,
    MissingPathForPathMode,
    InvalidType(u8),
    FileTooLarge(u64),
    PackageTooLarge(u64),
    MissingDictionaryForType(u8),
    DenseIdGap,
    DuplicatePathHash(u64),
    DuplicateId(u32),
    WrongKeyForLookupMode,
    PatchKeyModeMismatch,
    MalformedOutput,
    Io(std::io::Error),
    CodecError(String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIdForSparseMode => write!(f, "missing id for sparse-id package"),
            Self::MissingPathForPathMode => {
                write!(f, "missing path or path hash for path-hash package")
            }
            Self::InvalidType(v) => write!(f, "invalid type {v}"),
            Self::FileTooLarge(v) => write!(f, "file too large: {v}"),
            Self::PackageTooLarge(v) => write!(f, "package too large: {v}"),
            Self::MissingDictionaryForType(t) => write!(f, "missing dictionary for type {t}"),
            Self::DenseIdGap => write!(f, "dense id package contains gaps"),
            Self::DuplicatePathHash(v) => write!(f, "duplicate path hash {v}"),
            Self::DuplicateId(v) => write!(f, "duplicate id {v}"),
            Self::WrongKeyForLookupMode => write!(f, "wrong key for lookup mode"),
            Self::PatchKeyModeMismatch => write!(f, "patch key does not match target lookup mode"),
            Self::MalformedOutput => write!(f, "malformed output buffer"),
            Self::Io(e) => write!(f, "{e}"),
            Self::CodecError(e) => write!(f, "Codec error: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<std::io::Error> for BuildError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Errors specific to patch application.
#[derive(Debug)]
pub enum PatchError {
    Format(FormatError),
    Build(BuildError),
    InvalidPatchContainer,
    InvalidPatchManifest,
    WrongBasePackageHash { expected: u64, got: u64 },
    WrongResultPackageHash { expected: u64, got: u64 },
    OldFileHashMismatch,
    ReplacementHashMismatch,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(e) => write!(f, "format error: {e}"),
            Self::Build(e) => write!(f, "build error: {e}"),
            Self::InvalidPatchContainer => write!(f, "invalid patch container"),
            Self::InvalidPatchManifest => write!(f, "invalid patch manifest"),
            Self::WrongBasePackageHash { expected, got } => {
                write!(
                    f,
                    "wrong base package hash: expected {expected:#x}, got {got:#x}"
                )
            }
            Self::WrongResultPackageHash { expected, got } => {
                write!(
                    f,
                    "wrong resulting package hash: expected {expected:#x}, got {got:#x}"
                )
            }
            Self::OldFileHashMismatch => write!(f, "old file hash mismatch"),
            Self::ReplacementHashMismatch => write!(f, "replacement file hash mismatch"),
        }
    }
}

impl std::error::Error for PatchError {}

/// Write a package image to disk without reparsing it.
pub fn write_package(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), std::io::Error> {
    std::fs::write(path, bytes)
}

/// Read a package image from disk and immediately parse it with `UddpReader`.
pub fn read_package(
    path: impl AsRef<Path>,
) -> Result<super::reader::UddpReader, Box<dyn std::error::Error>> {
    Ok(super::reader::UddpReader::load(path)?)
}


