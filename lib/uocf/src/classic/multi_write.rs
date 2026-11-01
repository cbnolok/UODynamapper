//! `multi.mul` and `multi.idx` as typed entries, written back byte for byte ([`IndexedMul`]).
//!
//! A part record is 12 bytes before High Seas (item id, x, y, z as `i16`, flags as `u32`) and 16 from High Seas
//! on, with one more `u32` (0 or 1 in 7.0.20 and the 2024 client; kept, not interpreted). A file does not mix the
//! two sizes ([`MultiMap::record_size`](crate::classic::multi::MultiMap::record_size)).

crate::eyre_imports!();
use std::path::Path;

use crate::classic::indexed_mul::{IndexedEntry, IndexedMul};
use crate::classic::multi::MultiPart;

pub const LEGACY_PART_BYTES: usize = 12;
pub const HIGH_SEAS_PART_BYTES: usize = 16;
pub const MULTI_DATA_FILE: &str = "multi.mul";
pub const MULTI_INDEX_FILES: [&str; 2] = ["multi.idx", "multiidx.mul"];

/// A part with the High Seas field (0 in 12-byte files).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultiPartRecord {
    pub part: MultiPart,
    pub high_seas_field: u32,
}

/// The multis of a `multi.mul`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiMulFile {
    pub container: IndexedMul,
    /// 12 or 16.
    pub part_bytes: usize,
}

pub fn decode_parts(payload: &[u8], part_bytes: usize) -> eyre::Result<Vec<MultiPartRecord>> {
    if payload.len() % part_bytes != 0 {
        eyre::bail!("{} bytes are not whole {part_bytes}-byte parts", payload.len());
    }
    let i16_at = |p: &[u8], o: usize| i16::from_le_bytes([p[o], p[o + 1]]);
    let u32_at = |p: &[u8], o: usize| u32::from_le_bytes(p[o..o + 4].try_into().expect("four bytes"));
    Ok(payload
        .chunks_exact(part_bytes)
        .map(|p| MultiPartRecord {
            part: MultiPart { item_id: u16::from_le_bytes([p[0], p[1]]), x: i16_at(p, 2), y: i16_at(p, 4), z: i16_at(p, 6), flags: u32_at(p, 8) },
            high_seas_field: if part_bytes == HIGH_SEAS_PART_BYTES { u32_at(p, 12) } else { 0 },
        })
        .collect())
}

pub fn encode_parts(parts: &[MultiPartRecord], part_bytes: usize) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(parts.len() * part_bytes);
    for (i, record) in parts.iter().enumerate() {
        let MultiPart { item_id, x, y, z, flags } = record.part;
        bytes.extend_from_slice(&item_id.to_le_bytes());
        for value in [x, y, z] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&flags.to_le_bytes());
        match part_bytes {
            HIGH_SEAS_PART_BYTES => bytes.extend_from_slice(&record.high_seas_field.to_le_bytes()),
            LEGACY_PART_BYTES if record.high_seas_field != 0 => {
                eyre::bail!("part {i}: a {LEGACY_PART_BYTES}-byte part has no High Seas field ({})", record.high_seas_field)
            }
            LEGACY_PART_BYTES => {}
            other => eyre::bail!("a part has {LEGACY_PART_BYTES} or {HIGH_SEAS_PART_BYTES} bytes, not {other}"),
        }
    }
    Ok(bytes)
}

impl MultiMulFile {
    /// Reads the pair; the part size is the one every multi is a whole number of. When every multi fits both,
    /// there is no telling, so it is an error: give the size with [`Self::from_container`].
    pub fn from_bytes(index: &[u8], data: &[u8]) -> eyre::Result<Self> {
        let container = IndexedMul::from_bytes(index, data)?;
        let fits = |stride: usize| container.entries.iter().filter_map(IndexedEntry::payload).all(|p| p.len() % stride == 0);
        let part_bytes = match (fits(LEGACY_PART_BYTES), fits(HIGH_SEAS_PART_BYTES)) {
            (true, false) => LEGACY_PART_BYTES,
            (false, true) => HIGH_SEAS_PART_BYTES,
            (true, true) => eyre::bail!("every multi fits both part sizes; give the size"),
            (false, false) => eyre::bail!("the multis fit neither part size"),
        };
        Self::from_container(container, part_bytes)
    }

    pub fn from_container(container: IndexedMul, part_bytes: usize) -> eyre::Result<Self> {
        let file = Self { container, part_bytes };
        for id in 0..file.container.entries.len() {
            file.parts(id).wrap_err_with(|| format!("multi {id}"))?;
        }
        Ok(file)
    }

    /// Reads `multi.mul` and `multi.idx` (or `multiidx.mul`) from a client folder.
    pub fn load(client_dir: &Path) -> eyre::Result<Self> {
        let find = |name: &str| crate::utils::path::find_file_ignoring_case(client_dir, name);
        let data = find(MULTI_DATA_FILE).ok_or_else(|| eyre!("{MULTI_DATA_FILE} not in {}", client_dir.display()))?;
        let index = MULTI_INDEX_FILES.iter().find_map(|name| find(name)).ok_or_else(|| eyre!("no multi index in {}", client_dir.display()))?;
        let (index_bytes, data_bytes) = (std::fs::read(&index)?, std::fs::read(&data)?);
        Self::from_bytes(&index_bytes, &data_bytes).wrap_err_with(|| format!("{} and {}", index.display(), data.display()))
    }

    pub fn len(&self) -> usize {
        self.container.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.container.entries.is_empty()
    }

    /// The parts of multi `id`; `None` for an absent or out-of-range id.
    pub fn parts(&self, id: usize) -> eyre::Result<Option<Vec<MultiPartRecord>>> {
        self.container.entries.get(id).and_then(IndexedEntry::payload).map(|p| decode_parts(p, self.part_bytes)).transpose()
    }

    /// Replaces multi `id`; no parts clears it.
    pub fn set_parts(&mut self, id: usize, parts: &[MultiPartRecord]) -> eyre::Result<()> {
        if parts.is_empty() {
            self.container.clear(id);
            return Ok(());
        }
        let extra = self.container.entries.get(id).and_then(IndexedEntry::extra).unwrap_or(0);
        self.container.set(id, IndexedEntry::Present { extra, payload: encode_parts(parts, self.part_bytes)? });
        Ok(())
    }

    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        self.container.to_bytes()
    }

    pub fn save(&self, index_path: &Path, data_path: &Path) -> eyre::Result<()> {
        self.container.save(index_path, data_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_round_trip_in_both_sizes() {
        let part = MultiPart { item_id: 0x1234, x: -3, y: 4, z: 7, flags: 1 };
        for (bytes, field) in [(LEGACY_PART_BYTES, 0), (HIGH_SEAS_PART_BYTES, 1)] {
            let records = vec![MultiPartRecord { part, high_seas_field: field }; 2];
            let encoded = encode_parts(&records, bytes).unwrap();
            assert_eq!(encoded.len(), 2 * bytes);
            assert_eq!(decode_parts(&encoded, bytes).unwrap(), records);
        }
        assert!(encode_parts(&[MultiPartRecord { part, high_seas_field: 1 }], LEGACY_PART_BYTES).is_err());
    }
}
