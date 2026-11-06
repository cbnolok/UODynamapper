//! How the map and statics packers compress their chunks.
//!
//! Chunks are compressed with plain Zstd, at a level per package chosen by runtime decode speed
//! (`tests/integration/map_statics_retrieval.rs`, on the shipped Britannia `map0`/`statics0`):
//! - map chunks at 19: 15.96 MB against 17.76 MB at level 7, with no slower decode (3.4 us per
//!   warm 4 KB chunk; 3.3 against 3.6 us over a view region); level 22 gives the same bytes, and
//!   packing takes 2.5 s against 0.3 s;
//! - statics chunks at 7: from level 12 on, decode gets 5..10 % slower for at most 3 % fewer bytes
//!   (8.17 MB at 19 against 8.46 MB).
//!
//! No dictionary: it made map chunks larger than plain Zstd 7 and decoded about 10 % slower.
//! Stored chunks read fastest (0.2 us per map chunk) but make the map 6.6x larger (118 MB) and
//! statics 2.8x (24 MB), so storing is an explicit opt-in.

use udd_container::CompressionFlag;

/// The Zstd level of map chunks unless the packer is asked for another.
pub const MAP_CHUNK_ZSTD_LEVEL: i32 = 19;
/// The Zstd level of statics chunks unless the packer is asked for another.
pub const STATICS_CHUNK_ZSTD_LEVEL: i32 = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChunkCompression {
    /// Plain Zstd at the package's own level ([`MAP_CHUNK_ZSTD_LEVEL`], [`STATICS_CHUNK_ZSTD_LEVEL`]).
    #[default]
    PackageDefault,
    /// Plain Zstd at this level.
    Zstd(i32),
    /// Chunks are stored as they are.
    Stored,
}

impl ChunkCompression {
    /// The packers' `--stored` and `--zstd[=LEVEL]`: a bare `--zstd`, like neither, keeps the
    /// package's own level.
    pub fn from_cli(stored: bool, zstd: Option<Option<i32>>) -> Self {
        match (stored, zstd.flatten()) {
            (true, _) => Self::Stored,
            (false, Some(level)) => Self::Zstd(level),
            (false, None) => Self::PackageDefault,
        }
    }

    /// The builder codec for a non-empty chunk of a package whose own level is `package_level`.
    pub fn flag(self, package_level: i32) -> CompressionFlag {
        match self {
            Self::PackageDefault => CompressionFlag::ZstdNoDictLevel(package_level),
            Self::Zstd(level) => CompressionFlag::ZstdNoDictLevel(level),
            Self::Stored => CompressionFlag::None,
        }
    }
}
