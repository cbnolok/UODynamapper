//! UDDP / UDDPI reference implementation.
//!
//! This crate is intentionally written as both implementation and format guide.
//! Comments explain not only *what* the code does, but also *why* the format is
//! laid out this way, so that other programmers can reimplement it in another
//! language without guessing hidden assumptions.
//!
//! ---------------------------------------------------------------------------
//! Package kinds
//! ---------------------------------------------------------------------------
//!
//! - `.uddp`  : runtime package
//! - `.uddpi` : patch package
//!
//! A `.uddpi` is stored using the same container machinery as `.uddp`, but the
//! convention is that file id `0` contains a patch manifest, and files `1..N`
//! contain the full replacement payloads referenced by that manifest.
//!
//! Patching never uses binary diffs. It only replaces whole logical files.
//! This is simpler, much easier to debug, and usually good enough for the kind
//! of asset sets this format targets.
//!
//! ---------------------------------------------------------------------------
//! Module split
//! ---------------------------------------------------------------------------
//!
//! This root file intentionally keeps only the shared format model:
//! - constants and public enums
//! - packed metadata helpers
//! - on-disk header and record structs
//! - small request/response structs used by the reader and builders
//!
//! The executable logic now lives in child modules so each concern can be read
//! in isolation without scrolling through the entire format implementation:
//! - `reader.rs`  : package parsing and payload decoding
//! - `builder.rs` : package construction and compression policy
//! - `patch.rs`   : patch manifest creation and patch application
//! - `support.rs` : hashing, binary helpers, filesystem helpers, and errors
//!
//! The public `uocf::udd::uddp` API stays the same. The split is purely about
//! making the source easier to navigate and document.

#![allow(dead_code)]

mod builder;
mod patch;
mod reader;
mod support;

use self::support::{Cursor, write_u8, write_u16, write_u32, write_u64};

pub use self::builder::{BuildProgress, BuildProgressPhase, UddpBuilder};
pub use self::patch::{UddpiApplier, UddpiBuilder};
pub use self::reader::UddpReader;
pub use self::support::{
    BuildError,
    FormatError,
    PatchError,
    canonical_package_hash64,
    normalize_virtual_path,
    pack_meta32,
    pack_pos64,
    read_package,
    reconstruct_stored_size,
    unpack_codec,
    unpack_delta32,
    unpack_delta_hi8,
    unpack_delta_lo24,
    unpack_offset40,
    unpack_type,
    write_package,
    xxh64_virtual_path,
};

// =============================================================================
// Global limits and constants
// =============================================================================

/// Maximum package size: 1 TiB.
///
/// Design reason:
/// the packed file locator stores the payload offset in 40 bits.
pub const MAX_PACKAGE_SIZE: u64 = 1u64 << 40;

/// Maximum raw file size: `< 4 GiB`.
///
/// Design reason:
/// per-file raw size is stored as `u32` to keep file records compact.
pub const MAX_RAW_FILE_SIZE: u32 = u32::MAX;

/// Maximum number of directly encodable types.
///
/// Design reason:
/// a file has exactly one type, so 6 bits are enough.
pub const MAX_TYPES: usize = 64;

pub const UDDP_MAGIC: u32 = u32::from_le_bytes(*b"UDDP");
pub const UDPI_MAGIC: u32 = u32::from_le_bytes(*b"UDPI");
pub const PMAN_MAGIC: u32 = u32::from_le_bytes(*b"PMAN");

/// Offset of `package_hash64` inside the serialized header.
///
/// The package hash is computed canonically by zeroing these 8 bytes before
/// hashing the whole package with XXH64.
const HEADER_PACKAGE_HASH_OFFSET: usize = 40;

// =============================================================================
// Enums
// =============================================================================

/// How files are addressed logically inside a package.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupMode {
    /// Files are addressed by `XXH64(normalized_virtual_path)`.
    VirtualPathHash = 0,

    /// Files are addressed by dense ids `0..file_count`.
    DenseId = 1,

    /// Files are addressed by sparse ids, so the package stores an explicit TOC.
    SparseId = 2,
}

impl LookupMode {
    pub fn from_u8(v: u8) -> Result<Self, FormatError> {
        match v {
            0 => Ok(Self::VirtualPathHash),
            1 => Ok(Self::DenseId),
            2 => Ok(Self::SparseId),
            _ => Err(FormatError::InvalidLookupMode(v)),
        }
    }
}

/// Compression mode stored in the file metadata.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Stored verbatim.
    None = 0,

    /// Zstd without custom dictionary.
    ZstdNoDict = 1,

    /// Zstd with dictionary resolved implicitly from the file type.
    ZstdTypeDict = 2,

    /// Reserved for future use.
    Reserved = 3,
}

impl Codec {
    pub fn from_u8(v: u8) -> Result<Self, FormatError> {
        match v {
            0 => Ok(Self::None),
            1 => Ok(Self::ZstdNoDict),
            2 => Ok(Self::ZstdTypeDict),
            3 => Ok(Self::Reserved),
            _ => Err(FormatError::InvalidCodec(v)),
        }
    }
}

/// User-facing compression preference for package builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFlag {
    /// Never compress this file.
    None,

    /// Builder decides based on file size, type statistics, and measured gain.
    Auto,

    /// Force plain Zstd compression.
    ZstdNoDict,

    /// Force Zstd compression with the implicit type dictionary.
    ZstdDict,
}

/// Convenience starter type ids.
///
/// Projects may extend this list as needed, as long as values stay in `0..64`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataType {
    Unknown = 0,
    Art = 1,
    Anim = 2,
    Map = 3,
    Gump = 4,
    GumpArt = 5,
    Sound = 6,
    Music = 7,
    Multi = 8,
    Texture = 9,
    Light = 10,
    Metadata = 11,
    Sector = 12,
    Tile = 13,
    Static = 14,
}

// =============================================================================
// Packed metadata helpers
// =============================================================================

/// Per-file metadata packed in 32 bits.
///
/// Layout:
/// - bits  0..= 5 : type (6 bits)
/// - bits  6..= 7 : codec (2 bits)
/// - bits  8..=15 : high 8 bits of the compression delta
/// - bits 16..=31 : reserved
///
/// `delta = raw_size - stored_size`
pub type Meta32 = u32;

/// Packed position word.
///
/// Layout:
/// - bits  0..=39 : payload offset in package
/// - bits 40..=63 : low 24 bits of the compression delta
pub type Pos64 = u64;

// =============================================================================
// On-disk structs (represented as normal Rust structs, serialized manually)
// =============================================================================

/// Main package header.
///
/// Sequential layout in the file:
/// 1. Header
/// 2. Dictionary reference table
/// 3. Main index section
/// 4. Dictionary blobs
/// 5. File payload blobs
///
/// Offsets are absolute from the beginning of the file.
#[derive(Debug, Clone, Copy, Default)]
pub struct UddpHeader {
    pub magic: u32,
    pub version_major: u16,
    pub version_minor: u16,

    pub lookup_mode: u8,
    pub reserved0: u8,
    pub dict_count: u16,

    pub file_count: u32,
    pub dict_table_offset: u64,
    pub index_offset: u64,
    pub blob_offset: u64,

    pub package_hash64: u64,
}

impl UddpHeader {
    pub const SERIALIZED_SIZE: usize = 48;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u32(out, self.magic);
        write_u16(out, self.version_major);
        write_u16(out, self.version_minor);
        write_u8(out, self.lookup_mode);
        write_u8(out, self.reserved0);
        write_u16(out, self.dict_count);
        write_u32(out, self.file_count);
        write_u64(out, self.dict_table_offset);
        write_u64(out, self.index_offset);
        write_u64(out, self.blob_offset);
        write_u64(out, self.package_hash64);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            magic: c.read_u32()?,
            version_major: c.read_u16()?,
            version_minor: c.read_u16()?,
            lookup_mode: c.read_u8()?,
            reserved0: c.read_u8()?,
            dict_count: c.read_u16()?,
            file_count: c.read_u32()?,
            dict_table_offset: c.read_u64()?,
            index_offset: c.read_u64()?,
            blob_offset: c.read_u64()?,
            package_hash64: c.read_u64()?,
        })
    }
}

/// Dynamic dictionary reference.
///
/// Design reason:
/// dictionary refs are not stored as a fixed 64-entry array because most real
/// packages only use a few data types. The reader builds an in-memory lookup
/// table from this compact dynamic list.
#[derive(Debug, Clone, Copy, Default)]
pub struct UddpDictRef {
    pub data_type: u8,
    pub codec: u8,
    pub reserved0: u16,
    pub offset: u64,
    pub size: u32,
    pub reserved1: u32,
}

impl UddpDictRef {
    pub const SERIALIZED_SIZE: usize = 20;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u8(out, self.data_type);
        write_u8(out, self.codec);
        write_u16(out, self.reserved0);
        write_u64(out, self.offset);
        write_u32(out, self.size);
        write_u32(out, self.reserved1);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            data_type: c.read_u8()?,
            codec: c.read_u8()?,
            reserved0: c.read_u16()?,
            offset: c.read_u64()?,
            size: c.read_u32()?,
            reserved1: c.read_u32()?,
        })
    }
}

/// Physical locator for one payload.
///
/// The logical lookup mode changes only the key. The physical payload location
/// format remains the same across all package types.
#[derive(Debug, Clone, Copy, Default)]
pub struct UddpLocator {
    pub raw_size: u32,
    pub meta32: Meta32,
    pub pos64: Pos64,
}

impl UddpLocator {
    pub const SERIALIZED_SIZE: usize = 16;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u32(out, self.raw_size);
        write_u32(out, self.meta32);
        write_u64(out, self.pos64);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            raw_size: c.read_u32()?,
            meta32: c.read_u32()?,
            pos64: c.read_u64()?,
        })
    }
}

/// Entry for path-hash lookup packages.
#[derive(Debug, Clone, Copy, Default)]
pub struct UddpPathEntry {
    pub path_hash64: u64,
    pub locator: UddpLocator,
}

impl UddpPathEntry {
    pub const SERIALIZED_SIZE: usize = 24;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u64(out, self.path_hash64);
        self.locator.write_to(out);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            path_hash64: c.read_u64()?,
            locator: UddpLocator::read_from(c)?,
        })
    }
}

/// Entry for sparse-id lookup packages.
#[derive(Debug, Clone, Copy, Default)]
pub struct UddpSparseIdEntry {
    pub id: u32,
    pub reserved0: u32,
    pub locator: UddpLocator,
}

impl UddpSparseIdEntry {
    pub const SERIALIZED_SIZE: usize = 24;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u32(out, self.id);
        write_u32(out, self.reserved0);
        self.locator.write_to(out);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            id: c.read_u32()?,
            reserved0: c.read_u32()?,
            locator: UddpLocator::read_from(c)?,
        })
    }
}

/// Patch manifest header stored inside file id 0 of a `.uddpi`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PatchManifestHeader {
    pub magic: u32,
    pub version_major: u16,
    pub version_minor: u16,
    pub old_package_hash64: u64,
    pub new_package_hash64: u64,
    pub record_count: u32,
    pub target_lookup_mode: u32,
}

impl PatchManifestHeader {
    pub const SERIALIZED_SIZE: usize = 32;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u32(out, self.magic);
        write_u16(out, self.version_major);
        write_u16(out, self.version_minor);
        write_u64(out, self.old_package_hash64);
        write_u64(out, self.new_package_hash64);
        write_u32(out, self.record_count);
        write_u32(out, self.target_lookup_mode);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            magic: c.read_u32()?,
            version_major: c.read_u16()?,
            version_minor: c.read_u16()?,
            old_package_hash64: c.read_u64()?,
            new_package_hash64: c.read_u64()?,
            record_count: c.read_u32()?,
            target_lookup_mode: c.read_u32()?,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PatchPathRecord {
    pub path_hash64: u64,
    pub old_file_hash64: u64,
    pub new_file_hash64: u64,
    pub payload_file_id: u32,
    pub new_raw_size: u32,
}

impl PatchPathRecord {
    pub const SERIALIZED_SIZE: usize = 32;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u64(out, self.path_hash64);
        write_u64(out, self.old_file_hash64);
        write_u64(out, self.new_file_hash64);
        write_u32(out, self.payload_file_id);
        write_u32(out, self.new_raw_size);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            path_hash64: c.read_u64()?,
            old_file_hash64: c.read_u64()?,
            new_file_hash64: c.read_u64()?,
            payload_file_id: c.read_u32()?,
            new_raw_size: c.read_u32()?,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PatchIdRecord {
    pub id: u32,
    pub payload_file_id: u32,
    pub old_file_hash64: u64,
    pub new_file_hash64: u64,
    pub new_raw_size: u32,
    pub reserved0: u32,
}

impl PatchIdRecord {
    pub const SERIALIZED_SIZE: usize = 32;

    fn write_to(&self, out: &mut Vec<u8>) {
        write_u32(out, self.id);
        write_u32(out, self.payload_file_id);
        write_u64(out, self.old_file_hash64);
        write_u64(out, self.new_file_hash64);
        write_u32(out, self.new_raw_size);
        write_u32(out, self.reserved0);
    }

    fn read_from(c: &mut Cursor<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            id: c.read_u32()?,
            payload_file_id: c.read_u32()?,
            old_file_hash64: c.read_u64()?,
            new_file_hash64: c.read_u64()?,
            new_raw_size: c.read_u32()?,
            reserved0: c.read_u32()?,
        })
    }
}

// =============================================================================
// Public reader-facing structs
// =============================================================================

#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub raw_size: u32,
    pub stored_size: u32,
    pub data_type: u8,
    pub codec: Codec,
    pub payload_offset: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKey {
    PathHash(u64),
    Id(u32),
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub key: FileKey,
    pub locator: UddpLocator,
}

#[derive(Debug, Clone)]
pub struct AddFileRequest<'a> {
    pub data_type: u8,
    pub compression: CompressionFlag,
    pub virtual_path: Option<&'a str>,
    pub path_hash64: Option<u64>,
    pub id: Option<u32>,
    pub data: &'a [u8],
}

// Reader, builder, patch, and low-level support logic live in child modules.
// Keeping those large executable sections out of this file leaves the shared
// format model above as a much smaller reference surface.
