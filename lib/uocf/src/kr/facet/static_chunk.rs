//! Kingdom Reborn `StaticChunk.uop`: 32x32 chunks of static objects.
//!
//! Every entry is unnamed (no name pattern is known) and holds `u16 count`, then `count` Classic static records
//! `{u16 graphic, u8 x, u8 y, i8 z, u16 hue}` (the `statics.mul` record), with `x` and `y` in `0..32`. The shipped
//! client has 145 entries (60,713 records; 139 zlib-compressed, 6 not), all decoded and re-encoded byte-identical.
//!
//! The client keys a chunk `cx << 10 | cy` (chunk coordinates, 32 squares each; the facet is the current one),
//! and `StaticChunkBuildRequest` builds the chunk's static objects in memory from the loaded entry; no writer of
//! the file was found, so it is shipped data rather than a cache the client fills (`UOKR.exe`;
//! `scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md`). The entry names, and so which entry
//! holds which chunk, are unresolved: 91 of the 145 chunks are found exactly in the KR facet statics, at origins
//! mostly (16, 8) or (0, 0) modulo 32. The facet sectors are self-contained: map decoding does not need this
//! package. `TerrainChunk.uop` is mounted by the client too but not shipped.

use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};

use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::UopPackage;

crate::eyre_imports!();

/// File name of the package in the client directory.
pub const STATIC_CHUNK_UOP_NAME: &str = "StaticChunk.uop";
/// Squares along each side of a chunk (`x` and `y` are below it in the shipped data).
pub const STATIC_CHUNK_SIDE: u8 = 32;
/// Bytes of a record: u16 graphic, u8 x, u8 y, i8 z, u16 hue.
const STATIC_CHUNK_RECORD_SIZE: usize = 7;
/// Bytes of the record count.
const STATIC_CHUNK_COUNT_SIZE: usize = size_of::<u16>();

/// One static of a chunk, as in `statics.mul`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticChunkRecord {
    pub graphic: u16,
    /// Square within the chunk, `0..STATIC_CHUNK_SIDE`.
    pub x: u8,
    pub y: u8,
    pub z: i8,
    pub hue: u16,
}

/// One `StaticChunk.uop` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticChunk {
    pub records: Vec<StaticChunkRecord>,
}

impl StaticChunk {
    /// Decodes an entry payload, which must end exactly after the last record.
    pub fn decode(data: &[u8]) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(data);
        let count = usize::from(reader.read_u16::<LittleEndian>()?);
        let expected = STATIC_CHUNK_COUNT_SIZE + count * STATIC_CHUNK_RECORD_SIZE;
        if data.len() != expected {
            eyre::bail!("static chunk of {count} records has {} bytes, expected {expected}", data.len());
        }
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push(StaticChunkRecord {
                graphic: reader.read_u16::<LittleEndian>()?,
                x: reader.read_u8()?,
                y: reader.read_u8()?,
                z: reader.read_i8()?,
                hue: reader.read_u16::<LittleEndian>()?,
            });
        }
        Ok(Self { records })
    }

    /// Encodes the entry payload, the exact inverse of [`Self::decode`]. Fails above `u16::MAX` records or on a
    /// record outside the chunk.
    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        let count = u16::try_from(self.records.len())
            .map_err(|_| eyre!("{} records do not fit the u16 count of a static chunk", self.records.len()))?;
        let mut out = Vec::with_capacity(STATIC_CHUNK_COUNT_SIZE + self.records.len() * STATIC_CHUNK_RECORD_SIZE);
        out.extend_from_slice(&count.to_le_bytes());
        for record in &self.records {
            if record.x >= STATIC_CHUNK_SIDE || record.y >= STATIC_CHUNK_SIDE {
                eyre::bail!("static at ({}, {}) is outside the {STATIC_CHUNK_SIDE}x{STATIC_CHUNK_SIDE} chunk", record.x, record.y);
            }
            out.extend_from_slice(&record.graphic.to_le_bytes());
            out.push(record.x);
            out.push(record.y);
            out.extend_from_slice(&record.z.to_le_bytes());
            out.extend_from_slice(&record.hue.to_le_bytes());
        }
        Ok(out)
    }
}

/// Every chunk of a `StaticChunk.uop`, by entry hash in package order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticChunkPackage {
    pub chunks: Vec<(u64, StaticChunk)>,
}

impl StaticChunkPackage {
    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut chunks = Vec::new();
        for file in package.iter_files().filter(|file| file.has_size()) {
            let hash = file.filename_hash();
            let payload = file.unpack().wrap_err_with(|| format!("unpack static chunk {hash:016X}"))?;
            chunks.push((hash, StaticChunk::decode(&payload).wrap_err_with(|| format!("decode static chunk {hash:016X}"))?));
        }
        Ok(Self { chunks })
    }

    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
        Self::from_package(&package)
    }

    /// Writes the chunks back into the entries of the same hashes, keeping each entry's compression (unchanged
    /// payloads keep their entry untouched). Fails on a hash the package lacks. Returns the entries replaced.
    pub fn apply_to_package(&self, package: &mut UopPackage) -> eyre::Result<usize> {
        let mut replaced = 0;
        for (hash, chunk) in &self.chunks {
            let compression: CompressionFlag = package
                .get_file_by_hash(*hash)
                .ok_or_else(|| eyre!("the package has no static chunk {hash:016X}"))?
                .compression();
            let outcome = package.replace_file(*hash, &chunk.encode()?, compression)?;
            if outcome == crate::uop_container::package::FileReplacement::Replaced {
                replaced += 1;
            }
        }
        Ok(replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_records_round_trip_and_lengths_are_exact() {
        let chunk = StaticChunk {
            records: vec![
                StaticChunkRecord { graphic: 0x0EED, x: 0, y: 31, z: -5, hue: 0 },
                StaticChunkRecord { graphic: 0x3FFF, x: 16, y: 8, z: 127, hue: 1153 },
            ],
        };
        let bytes = chunk.encode().unwrap();
        assert_eq!(bytes.len(), STATIC_CHUNK_COUNT_SIZE + 2 * STATIC_CHUNK_RECORD_SIZE);
        assert_eq!(StaticChunk::decode(&bytes).unwrap(), chunk);
        assert!(StaticChunk::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut long = bytes.clone();
        long.push(0);
        assert!(StaticChunk::decode(&long).is_err());
        let outside = StaticChunk { records: vec![StaticChunkRecord { x: STATIC_CHUNK_SIDE, ..chunk.records[0] }] };
        assert!(outside.encode().is_err());
    }
}
