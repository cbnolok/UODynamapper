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
use std::io::Write;
use std::ops::Range;
use std::path::Path;

use xxhash_rust::xxh64::xxh64;
use zstd::bulk::Compressor;
use zstd::dict::DecoderDictionary;

use super::*;

const ZSTD_LEVEL: i32 = 7;

/// The Zstd level of payloads written without an explicit one, and of payloads stored again whose level isn't
/// recorded.
pub const ZSTD_DEFAULT_LEVEL: i32 = ZSTD_LEVEL;

/// The level of payloads written without an explicit one.
pub(crate) const fn zstd_default_level() -> i32 {
    ZSTD_LEVEL
}


/// The Meta32 bit of a Zstd payload stored as independent frames ([`zstd_compress_strips`]), which the reader decodes
/// in parallel. Without it a payload decodes as one stream, whatever frames it holds.
const META32_INDEPENDENT_FRAMES: Meta32 = 1 << 16;

/// Where the low bits of the codec id sit in Meta32, and how many there are.
const META32_CODEC_LOW_SHIFT: u32 = 6;
const META32_CODEC_LOW_BITS: u32 = 2;
const META32_CODEC_LOW_MASK: u32 = (1 << META32_CODEC_LOW_BITS) - 1;
/// The Meta32 bit of the codec id's high bit, a reserved bit once: the codecs 0 to 3 keep their bits.
const META32_CODEC_HIGH_SHIFT: u32 = 17;
/// The Meta32 bits no field uses, zero in a valid locator.
const META32_RESERVED_MASK: Meta32 = !((1 << (META32_CODEC_HIGH_SHIFT + 1)) - 1);

/// Pack the public metadata word stored in every file locator.
pub fn pack_meta32(data_type: u8, codec: Codec, delta32: u32, independent_frames: bool) -> Meta32 {
    let ty = (data_type as u32) & 0x3F;
    let codec = codec as u32;
    let co = ((codec & META32_CODEC_LOW_MASK) << META32_CODEC_LOW_SHIFT)
        | ((codec >> META32_CODEC_LOW_BITS) << META32_CODEC_HIGH_SHIFT);
    let delta_hi = ((delta32 >> 24) & 0xFF) << 8;
    let frames = if independent_frames { META32_INDEPENDENT_FRAMES } else { 0 };
    ty | co | delta_hi | frames
}

/// Whether a packed metadata word marks its payload as independent Zstd frames.
#[inline(always)]
pub fn unpack_independent_frames(meta32: Meta32) -> bool {
    meta32 & META32_INDEPENDENT_FRAMES != 0
}

/// Extract the 6-bit type id from a packed metadata word.
#[inline(always)]
pub fn unpack_type(meta32: Meta32) -> u8 {
    (meta32 & 0x3F) as u8
}

/// Extract the raw 3-bit codec id from a packed metadata word: bits 6..=7 and bit 17.
#[inline(always)]
pub(crate) fn unpack_codec_id(meta32: Meta32) -> u8 {
    (((meta32 >> META32_CODEC_LOW_SHIFT) & META32_CODEC_LOW_MASK)
        | (((meta32 >> META32_CODEC_HIGH_SHIFT) & 1) << META32_CODEC_LOW_BITS)) as u8
}

/// The codec of a packed metadata word, or why the word is invalid: a codec id that names no codec, or a reserved
/// bit set. The reader checks every locator with this when it opens a package.
pub fn try_unpack_codec(meta32: Meta32) -> Result<Codec, FormatError> {
    if meta32 & META32_RESERVED_MASK != 0 {
        return Err(FormatError::ReservedBits(meta32));
    }
    Codec::from_u8(unpack_codec_id(meta32))
}

/// The codec of a packed metadata word of a package the reader opened, which checked its codec
/// ([`try_unpack_codec`]).
pub fn unpack_codec(meta32: Meta32) -> Codec {
    Codec::from_u8(unpack_codec_id(meta32)).expect("the reader checks every locator's codec when it opens a package")
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
        Codec::ZstdNoDict | Codec::ZstdTypeDict | Codec::Lz4Hc | Codec::Bc7FieldsZstd | Codec::Bc7FieldsLz4Hc => {
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
    zstd_compress_level(data, ZSTD_LEVEL)
}

/// Plain Zstd compression helper with an explicit compression level.
pub(crate) fn zstd_compress_level(data: &[u8], level: i32) -> std::io::Result<Vec<u8>> {
    let mut compressor = Compressor::new(level)?;
    compressor.compress(data)
}

/// Plain Zstd compression helper using Zstd's internal worker pool.
pub(crate) fn zstd_compress_parallel(data: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd_compress_level_parallel(data, ZSTD_LEVEL)
}

/// Plain Zstd compression helper with an explicit level and Zstd's internal worker pool.
pub(crate) fn zstd_compress_level_parallel(data: &[u8], level: i32) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::new(Vec::new(), level)?;
    encoder.multithread(zstd_worker_count())?;
    encoder.set_pledged_src_size(Some(data.len() as u64))?;
    encoder.write_all(data)?;
    encoder.finish()
}

/// Dictionary-backed Zstd compression helper.
pub(crate) fn zstd_compress_with_dict(data: &[u8], dict: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut compressor = Compressor::with_dictionary(ZSTD_LEVEL, dict)?;
    compressor.compress(data)
}

/// Dictionary-backed Zstd compression helper using Zstd's internal worker pool.
pub(crate) fn zstd_compress_with_dict_parallel(data: &[u8], dict: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::with_dictionary(Vec::new(), ZSTD_LEVEL, dict)?;
    encoder.multithread(zstd_worker_count())?;
    encoder.set_pledged_src_size(Some(data.len() as u64))?;
    encoder.write_all(data)?;
    encoder.finish()
}

/// Zstd at `level` as independent frames of `strip_bytes` raw bytes each (the last one shorter), compressed in
/// parallel and stored one after another. Its payload is marked with [`META32_INDEPENDENT_FRAMES`], so that
/// [`zstd_decompress`] decodes the frames in parallel.
/// One LZ4 block of `data` at [`LZ4_HC_LEVEL`], with no size prefix: the locator holds the raw size.
pub(crate) fn lz4_hc_compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    lz4::block::compress(data, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(LZ4_HC_LEVEL)), false)
}

/// The `raw_size` bytes of one LZ4 block; anything else is damage.
pub(crate) fn lz4_decompress(data: &[u8], raw_size: usize) -> std::io::Result<Vec<u8>> {
    let decoded = lz4_decompress_bounded(data, raw_size)?;
    if decoded.len() != raw_size {
        return Err(std::io::Error::other("LZ4 payload decodes to the wrong size"));
    }
    Ok(decoded)
}

/// One LZ4 block that decodes to at most `capacity` bytes.
fn lz4_decompress_bounded(data: &[u8], capacity: usize) -> std::io::Result<Vec<u8>> {
    let declared = i32::try_from(capacity).map_err(|_| std::io::Error::other("LZ4 payload too large"))?;
    let mut decoded = vec![0u8; capacity];
    let written = lz4::block::decompress_to_buffer(data, Some(declared), &mut decoded)?;
    decoded.truncate(written);
    Ok(decoded)
}

/// The BC7 field split of `data` as one Zstd frame at `level` ([`Codec::Bc7FieldsZstd`]).
pub(crate) fn bc7_fields_zstd_compress(data: &[u8], level: i32, parallel: bool) -> std::io::Result<Vec<u8>> {
    let mut split = Vec::new();
    crate::bc7_fields::forward(data, &mut split);
    if parallel { zstd_compress_level_parallel(&split, level) } else { zstd_compress_level(&split, level) }
}

/// The BC7 field split of `data` as one LZ4 block at [`LZ4_HC_LEVEL`] ([`Codec::Bc7FieldsLz4Hc`]).
pub(crate) fn bc7_fields_lz4_hc_compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut split = Vec::new();
    crate::bc7_fields::forward(data, &mut split);
    lz4_hc_compress(&split)
}

/// The BC7 field split stream of a payload of codec 4 or 5 whose file holds `raw_size` bytes, decompressed but not
/// joined. Its length is checked when it is joined.
pub(crate) fn bc7_fields_decompress(codec: Codec, data: &[u8], raw_size: usize) -> std::io::Result<Vec<u8>> {
    let capacity = crate::bc7_fields::split_len_bound(raw_size);
    match codec {
        Codec::Bc7FieldsZstd => DECOMPRESSION_CONTEXT.with(|context| {
            let mut decoded = Vec::with_capacity(capacity);
            context
                .borrow_mut()
                .decompress(&mut decoded, data)
                .map_err(|code| std::io::Error::other(zstd::zstd_safe::get_error_name(code)))?;
            Ok(decoded)
        }),
        Codec::Bc7FieldsLz4Hc => lz4_decompress_bounded(data, capacity),
        Codec::None | Codec::ZstdNoDict | Codec::ZstdTypeDict | Codec::Lz4Hc => {
            Err(std::io::Error::other(format!("{codec:?} payload is no BC7 field split")))
        }
    }
}

/// The `raw_size` bytes of a payload of codec 4 or 5: decompressed, then joined whole.
pub(crate) fn bc7_fields_decode(codec: Codec, data: &[u8], raw_size: usize) -> std::io::Result<Vec<u8>> {
    let split = bc7_fields_decompress(codec, data, raw_size)?;
    let mut decoded = Vec::new();
    crate::bc7_fields::inverse(&split, raw_size, &mut decoded).map_err(std::io::Error::other)?;
    Ok(decoded)
}

pub(crate) fn zstd_compress_strips(data: &[u8], level: i32, strip_bytes: u32) -> std::io::Result<Vec<u8>> {
    use rayon::prelude::*;
    if strip_bytes == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Zstd strips of 0 bytes"));
    }
    let frames = data
        .par_chunks(strip_bytes as usize)
        .map(|strip| zstd_compress_level(strip, level))
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(frames.concat())
}

fn zstd_worker_count() -> u32 {
    std::thread::available_parallelism()
        .map(|threads| threads.get().saturating_sub(1).max(1) as u32)
        .unwrap_or(1)
}

thread_local! {
    /// One Zstd decompression context per thread, reused by every payload decode. Creating a context
    /// for each payload cost more than decoding a 4 KB map chunk.
    static DECOMPRESSION_CONTEXT: std::cell::RefCell<zstd::zstd_safe::DCtx<'static>> =
        std::cell::RefCell::new(zstd::zstd_safe::DCtx::create());
}

/// Zstd-decompresses one payload of `raw_size` bytes on this thread's reused context, with the
/// type's prepared dictionary when the payload has one. A payload marked as `independent_frames` decodes its frames in
/// parallel, each straight into its part of the output; the writer decides which payloads are worth it.
pub(crate) fn zstd_decompress(
    data: &[u8],
    raw_size: usize,
    dictionary: Option<&DecoderDictionary<'static>>,
    independent_frames: bool,
) -> std::io::Result<Vec<u8>> {
    if independent_frames && dictionary.is_none() {
        if let Some(frames) = independent_zstd_frames(data, raw_size) {
            return zstd_decompress_frames(&frames, raw_size);
        }
    }
    DECOMPRESSION_CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let mut decoded = Vec::with_capacity(raw_size);
        let result = match dictionary {
            Some(dictionary) => context.decompress_using_ddict(&mut decoded, data, dictionary.as_ddict()),
            None => context.decompress(&mut decoded, data),
        };
        result.map_err(|code| std::io::Error::other(zstd::zstd_safe::get_error_name(code)))?;
        Ok(decoded)
    })
}

/// The frames of `data` with the raw size each declares, when there are several and the sizes add up to `raw_size`.
/// `None` sends the payload to the single-stream decoder, which also reports any damage.
fn independent_zstd_frames(data: &[u8], raw_size: usize) -> Option<Vec<(&[u8], usize)>> {
    let mut frames = Vec::new();
    let mut rest = data;
    let mut total = 0usize;
    while !rest.is_empty() {
        let compressed = zstd::zstd_safe::find_frame_compressed_size(rest).ok()?;
        if compressed == 0 || compressed > rest.len() || (frames.is_empty() && compressed == rest.len()) {
            return None;
        }
        let (frame, tail) = rest.split_at(compressed);
        let decoded = usize::try_from(zstd::zstd_safe::get_frame_content_size(frame).ok()??).ok()?;
        total = total.checked_add(decoded)?;
        frames.push((frame, decoded));
        rest = tail;
    }
    (total == raw_size).then_some(frames)
}

/// Decodes `frames` in parallel on the parallel executor (`crate::executor`), each on its thread's reused context into its
/// own part of the output.
fn zstd_decompress_frames(frames: &[(&[u8], usize)], raw_size: usize) -> std::io::Result<Vec<u8>> {
    let mut decoded = vec![0u8; raw_size];
    let mut parts = Vec::with_capacity(frames.len());
    let mut rest = decoded.as_mut_slice();
    for &(_, size) in frames {
        let (part, tail) = rest.split_at_mut(size);
        parts.push(std::sync::Mutex::new(part));
        rest = tail;
    }
    let first_error = std::sync::Mutex::new(None);
    crate::executor::parallel_executor().for_each_index(frames.len(), &|index| {
        let (frame, size) = frames[index];
        let mut part = parts[index].lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = DECOMPRESSION_CONTEXT.with(|context| {
            let written = context
                .borrow_mut()
                .decompress(&mut **part, frame)
                .map_err(|code| std::io::Error::other(zstd::zstd_safe::get_error_name(code)))?;
            if written == size {
                Ok(())
            } else {
                Err(std::io::Error::other(format!("Zstd frame decoded to {written} bytes, not the {size} it declares")))
            }
        });
        if let Err(err) = result {
            first_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert(err);
        }
    });
    drop(parts);
    match first_error.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Some(err) => Err(err),
        None => Ok(decoded),
    }
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
    UnsupportedVersion { major: u16, minor: u16 },
    InvalidLookupMode(u8),
    InvalidType(u8),
    InvalidCodec(u8),
    /// A locator's metadata word with reserved bits set.
    ReservedBits(Meta32),
    MissingDictionary(u8),
    FileNotFound,
    WrongLookupMode,
    Truncated,
    Overflow,
    UnsupportedCodec,
    /// A package nested in another one ([`super::UddpReader::open_section`]) must be stored uncompressed.
    CompressedSection,
    Io(std::io::Error),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic(v) => write!(f, "invalid magic: {v:#010x}"),
            Self::UnsupportedVersion { major, minor } => write!(
                f,
                "package version {major}.{minor} is not supported; repack with the current udd-cli"
            ),
            Self::InvalidLookupMode(v) => write!(f, "invalid lookup mode: {v}"),
            Self::InvalidType(v) => write!(f, "invalid type {v}"),
            Self::InvalidCodec(v) => write!(f, "invalid codec: {v}"),
            Self::ReservedBits(meta32) => write!(f, "reserved bits set in locator metadata {meta32:#010x}"),
            Self::MissingDictionary(t) => write!(f, "missing dictionary for type {t}"),
            Self::FileNotFound => write!(f, "file not found"),
            Self::WrongLookupMode => write!(f, "wrong lookup mode for requested operation"),
            Self::Truncated => write!(f, "truncated data"),
            Self::Overflow => write!(f, "integer overflow"),
            Self::UnsupportedCodec => write!(f, "unsupported codec"),
            Self::CompressedSection => write!(f, "the nested package is stored compressed"),
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
    /// The streaming builder compresses files as they are added, before a type
    /// dictionary could be trained over all of them.
    DictionaryCompressionWhileStreaming(CompressionFlag),
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
            Self::DictionaryCompressionWhileStreaming(compression) => write!(
                f,
                "compression {compression:?} needs a type dictionary and is not supported by the streaming builder"
            ),
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
