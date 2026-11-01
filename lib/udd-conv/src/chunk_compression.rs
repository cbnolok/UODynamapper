//! How the map and statics packers store their chunks.
//!
//! Chunks are stored by default. dynamapper reads them by id from a mapped package, and a stored
//! chunk costs a copy (or nothing, through a borrowing read) where a Zstd chunk costs a decode.
//! Measured on the shipped Britannia (`map0`/`statics0`, `tests/integration/map_statics_retrieval.rs`),
//! a warm 4 KB map chunk reads in 0.2 us stored against 3.3 us at Zstd 7, and a statics chunk in
//! 0.06 us against 1.3 us, for packages 6.6x and 2.8x larger (118 MB and 24 MB). Plain Zstd stays
//! available for a smaller package; the type dictionary is not offered, since it made map chunks
//! larger than plain Zstd 7 and decoded no faster.

use udd_container::CompressionFlag;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChunkCompression {
    /// Chunks are stored as they are.
    #[default]
    Stored,
    /// Chunks are compressed with plain Zstd at this level.
    Zstd(i32),
}

impl ChunkCompression {
    /// The packers' `--zstd[=LEVEL]`: without it, chunks are stored.
    pub fn from_zstd_level(level: Option<i32>) -> Self {
        level.map_or(Self::Stored, Self::Zstd)
    }

    /// The builder codec for a non-empty chunk.
    pub fn flag(self) -> CompressionFlag {
        match self {
            Self::Stored => CompressionFlag::None,
            Self::Zstd(level) => CompressionFlag::ZstdNoDictLevel(level),
        }
    }
}
