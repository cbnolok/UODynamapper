//! The Classic statics held whole in memory, for editing and writing: `staidxN.mul` + `staticsN.mul` (and the
//! `x` editions, which have the same layout), plus the static difs baked in. No client stores statics in a UOP.
//!
//! The writer follows the layout of the modern clients (7.0.20 to the 2024 client), measured on every index entry
//! of their shipped files (`docs/agent_notes/proposals/2026-11-01-classic-map-writers.md`):
//!
//! - `staidxN.mul`: one 12-byte entry per map block, in map order: `u32` lookup, `u32` length, `u32` extra. An
//!   empty block is `0xFFFFFFFF` three times; a non-empty one points at its records, with extra `0xAAAAAAAA`.
//! - `staticsN.mul`: the records of the non-empty blocks in index order, back to back from offset 0, nothing
//!   after the last. A record is 7 bytes: `u16` graphic, `u8` x, `u8` y (in the block), `i8` z, `u16` hue.
//! - A block's records keep their order. In the shipped files they are sorted by (y, x), the statics of one cell
//!   in their own order (not by z), so [`ClassicStaticsBlocks::add_static`] puts a new static after the others of
//!   its cell, keeping that order.
//!
//! So a modern statics pair is fully determined by each block's record list. The old clients (Mondain's Legacy,
//! UO Gold) store arbitrary extras, blocks out of order and dead bytes; reading keeps only the records, and a
//! write gives the modern layout (a valid file, not the same bytes).
//!
//! Static difs (`stadiflN`/`stadifiN`/`stadifN`) are baked into the blocks
//! ([`ClassicStaticsBlocks::bake_static_difs`]): every listed id is applied in file order, so a later duplicate
//! wins, and a dif entry that is invalid or empty empties its block, as in the client.

use std::path::Path;

use crate::classic::map::{MapBlock, MapSizeBlocks, MapSizeCells};
use crate::classic::map_write::size_in_blocks;
use crate::classic::statics::StaticTile;
use crate::utils::output_file::write_through_temp;

crate::eyre_imports!();

/// Bytes of an index entry: `u32` lookup, `u32` length, `u32` extra.
pub const STATICS_INDEX_ENTRY_SIZE: usize = 3 * size_of::<u32>();
/// The lookup, length and extra of an empty block.
pub const EMPTY_INDEX_FIELD: u32 = u32::MAX;
/// The extra of a non-empty block in the modern files.
pub const MODERN_INDEX_EXTRA: u32 = 0xAAAA_AAAA;
/// Bytes of a `stadiflN` record: the `u32` block id.
const DIF_LOOKUP_RECORD_SIZE: usize = size_of::<u32>();

/// What [`ClassicStaticsBlocks::bake_static_difs`] applied.
pub use crate::classic::map_write::DifBake;

/// Every block's statics, in map block order (column by column), each block's records in their order.
#[derive(Clone, PartialEq, Eq)]
pub struct ClassicStaticsBlocks {
    size: MapSizeBlocks,
    blocks: Vec<Vec<StaticRecord>>,
}

/// One static record, as the file stores it: `x` and `y` are its cell in the block (0..8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StaticRecord {
    pub graphic: u16,
    pub x: u8,
    pub y: u8,
    pub z: i8,
    pub hue: u16,
}

impl StaticRecord {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            graphic: u16::from_le_bytes([bytes[0], bytes[1]]),
            x: bytes[2],
            y: bytes[3],
            z: bytes[4] as i8,
            hue: u16::from_le_bytes([bytes[5], bytes[6]]),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.graphic.to_le_bytes());
        out.push(self.x);
        out.push(self.y);
        out.push(self.z as u8);
        out.extend_from_slice(&self.hue.to_le_bytes());
    }

    /// The record as [`StaticTile`] (the readers' type).
    pub fn to_tile(self) -> StaticTile {
        StaticTile { graphic: self.graphic, x_offset: self.x, y_offset: self.y, z: self.z, _pad: 0, hue: self.hue }
    }
}

impl std::fmt::Debug for ClassicStaticsBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClassicStaticsBlocks({}x{} blocks, {} statics)", self.size.width, self.size.height, self.static_count())
    }
}

/// The records of one block's `len` bytes at `lookup` in `data`.
fn parse_records(data: &[u8], lookup: u32, len: u32, block: usize) -> eyre::Result<Vec<StaticRecord>> {
    if len as usize % StaticTile::RAW_SIZE != 0 {
        eyre::bail!("block {block}: {len} bytes of statics are not whole {}-byte records", StaticTile::RAW_SIZE);
    }
    let start = lookup as usize;
    let end = start + len as usize;
    let bytes = data
        .get(start..end)
        .ok_or_else(|| eyre!("block {block}: statics at {start}..{end} past the end of the {} data bytes", data.len()))?;
    let records: Vec<StaticRecord> = bytes.chunks_exact(StaticTile::RAW_SIZE).map(StaticRecord::from_bytes).collect();
    if let Some(record) = records.iter().find(|record| {
        u32::from(record.x) >= MapBlock::CELLS_PER_ROW || u32::from(record.y) >= MapBlock::CELLS_PER_COLUMN
    }) {
        eyre::bail!("block {block}: a static at cell ({}, {}) is outside the block", record.x, record.y);
    }
    Ok(records)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + size_of::<u32>()].try_into().expect("4 bytes"))
}

/// The lookup and length of the index entry at `entry`, `None` for an empty block (an invalid lookup or length,
/// or no bytes).
fn index_entry(index: &[u8], entry: usize) -> Option<(u32, u32)> {
    let offset = entry * STATICS_INDEX_ENTRY_SIZE;
    let (lookup, len) = (u32_at(index, offset), u32_at(index, offset + size_of::<u32>()));
    (lookup != EMPTY_INDEX_FIELD && len != EMPTY_INDEX_FIELD && len != 0).then_some((lookup, len))
}

impl ClassicStaticsBlocks {
    /// Statics for a map of `size` cells (multiples of 8), every block empty.
    pub fn new(size: MapSizeCells) -> eyre::Result<Self> {
        let size = size_in_blocks(size)?;
        Ok(Self { size, blocks: vec![Vec::new(); size.width as usize * size.height as usize] })
    }

    /// Reads `staidxN.mul` and `staticsN.mul` bytes of a map of `size` cells: the index must have one entry per
    /// block, and every record must lie in the data and in its block.
    pub fn from_mul_bytes(index: &[u8], data: &[u8], size: MapSizeCells) -> eyre::Result<Self> {
        let mut statics = Self::new(size)?;
        if index.len() != statics.block_count() * STATICS_INDEX_ENTRY_SIZE {
            eyre::bail!(
                "a {}x{} map has {} blocks, the index {} bytes ({} entries of {STATICS_INDEX_ENTRY_SIZE})",
                size.width,
                size.height,
                statics.block_count(),
                index.len(),
                index.len() as f64 / STATICS_INDEX_ENTRY_SIZE as f64
            );
        }
        for (block, records) in statics.blocks.iter_mut().enumerate() {
            if let Some((lookup, len)) = index_entry(index, block) {
                *records = parse_records(data, lookup, len, block)?;
            }
        }
        Ok(statics)
    }

    /// Reads `staidxN.mul` and `staticsN.mul` (or their `x` editions); see [`Self::from_mul_bytes`].
    pub fn read_mul(index_path: &Path, data_path: &Path, size: MapSizeCells) -> eyre::Result<Self> {
        let index = std::fs::read(index_path).wrap_err_with(|| format!("read {}", index_path.display()))?;
        let data = std::fs::read(data_path).wrap_err_with(|| format!("read {}", data_path.display()))?;
        Self::from_mul_bytes(&index, &data, size)
            .wrap_err_with(|| format!("read {} and {}", index_path.display(), data_path.display()))
    }

    pub fn size_blocks(&self) -> MapSizeBlocks {
        self.size
    }

    pub fn size_cells(&self) -> MapSizeCells {
        MapSizeCells {
            width: self.size.width * MapBlock::CELLS_PER_ROW,
            height: self.size.height * MapBlock::CELLS_PER_COLUMN,
        }
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn static_count(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }

    /// The records of block `block_index` (column by column), in their order.
    pub fn block(&self, block_index: usize) -> &[StaticRecord] {
        &self.blocks[block_index]
    }

    /// Replaces the records of block `block_index`; each must lie in the block.
    pub fn set_block(&mut self, block_index: usize, records: Vec<StaticRecord>) -> eyre::Result<()> {
        let count = self.block_count();
        let slot = self
            .blocks
            .get_mut(block_index)
            .ok_or_else(|| eyre!("block {block_index} is outside the {count} blocks"))?;
        if let Some(record) = records.iter().find(|record| {
            u32::from(record.x) >= MapBlock::CELLS_PER_ROW || u32::from(record.y) >= MapBlock::CELLS_PER_COLUMN
        }) {
            eyre::bail!("block {block_index}: a static at cell ({}, {}) is outside the block", record.x, record.y);
        }
        *slot = records;
        Ok(())
    }

    /// The block index and cell of map cell `(x, y)`.
    fn locate(&self, x: u32, y: u32) -> eyre::Result<(usize, u8, u8)> {
        let size = self.size_cells();
        if x >= size.width || y >= size.height {
            eyre::bail!("({x}, {y}) is outside the {}x{} map", size.width, size.height);
        }
        let block = (x / MapBlock::CELLS_PER_ROW) as usize * self.size.height as usize
            + (y / MapBlock::CELLS_PER_COLUMN) as usize;
        Ok((block, (x % MapBlock::CELLS_PER_ROW) as u8, (y % MapBlock::CELLS_PER_COLUMN) as u8))
    }

    /// The positions in its block's records of the statics of cell `(x, y)`.
    fn cell_positions(&self, x: u32, y: u32) -> eyre::Result<(usize, Vec<usize>)> {
        let (block, cell_x, cell_y) = self.locate(x, y)?;
        let positions = self.blocks[block]
            .iter()
            .enumerate()
            .filter(|(_, record)| record.x == cell_x && record.y == cell_y)
            .map(|(position, _)| position)
            .collect();
        Ok((block, positions))
    }

    /// The statics of map cell `(x, y)`, in their order.
    pub fn cell_statics(&self, x: u32, y: u32) -> eyre::Result<Vec<StaticRecord>> {
        let (block, positions) = self.cell_positions(x, y)?;
        Ok(positions.into_iter().map(|position| self.blocks[block][position]).collect())
    }

    /// Adds a static to map cell `(x, y)`: after the cell's other statics, before the first record of a later
    /// cell in (y, x) order (the order of the shipped blocks). Returns its index among the cell's statics.
    pub fn add_static(&mut self, x: u32, y: u32, graphic: u16, z: i8, hue: u16) -> eyre::Result<usize> {
        let (block, cell_x, cell_y) = self.locate(x, y)?;
        let records = &mut self.blocks[block];
        let position = records
            .iter()
            .position(|record| (record.y, record.x) > (cell_y, cell_x))
            .unwrap_or(records.len());
        records.insert(position, StaticRecord { graphic, x: cell_x, y: cell_y, z, hue });
        Ok(records[..position].iter().filter(|record| record.x == cell_x && record.y == cell_y).count())
    }

    /// Removes static `index` of map cell `(x, y)` and returns it.
    pub fn remove_static(&mut self, x: u32, y: u32, index: usize) -> eyre::Result<StaticRecord> {
        let (block, positions) = self.cell_positions(x, y)?;
        let position = *positions
            .get(index)
            .ok_or_else(|| eyre!("({x}, {y}) has {} statics, no static {index}", positions.len()))?;
        Ok(self.blocks[block].remove(position))
    }

    /// Changes static `index` of map cell `(x, y)`.
    pub fn set_static(&mut self, x: u32, y: u32, index: usize, graphic: u16, z: i8, hue: u16) -> eyre::Result<()> {
        let (block, positions) = self.cell_positions(x, y)?;
        let position = *positions
            .get(index)
            .ok_or_else(|| eyre!("({x}, {y}) has {} statics, no static {index}", positions.len()))?;
        let record = &mut self.blocks[block][position];
        (record.graphic, record.z, record.hue) = (graphic, z, hue);
        Ok(())
    }

    /// The `staidxN.mul` and `staticsN.mul` bytes, in the modern layout.
    pub fn to_mul_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        let mut index = Vec::with_capacity(self.block_count() * STATICS_INDEX_ENTRY_SIZE);
        let mut data = Vec::with_capacity(self.static_count() * StaticTile::RAW_SIZE);
        for records in &self.blocks {
            if records.is_empty() {
                for _ in 0..STATICS_INDEX_ENTRY_SIZE / size_of::<u32>() {
                    index.extend_from_slice(&EMPTY_INDEX_FIELD.to_le_bytes());
                }
                continue;
            }
            index.extend_from_slice(&(data.len() as u32).to_le_bytes());
            index.extend_from_slice(&((records.len() * StaticTile::RAW_SIZE) as u32).to_le_bytes());
            index.extend_from_slice(&MODERN_INDEX_EXTRA.to_le_bytes());
            for record in records {
                record.write(&mut data);
            }
        }
        (index, data)
    }

    /// Writes `staidxN.mul` and `staticsN.mul` (or their `x` editions), each through a temporary file. Both are
    /// written before either replaces anything, so a failed write leaves both existing files as they were.
    pub fn write_mul(&self, index_path: &Path, data_path: &Path) -> eyre::Result<()> {
        let (index, data) = self.to_mul_bytes();
        u32::try_from(data.len()).map_err(|_| eyre!("{} bytes of statics do not fit u32 lookups", data.len()))?;
        write_through_temp(index_path, |index_temp| {
            std::fs::write(index_temp, &index).wrap_err_with(|| format!("write {}", index_path.display()))?;
            write_through_temp(data_path, |data_temp| std::fs::write(data_temp, &data).wrap_err_with(|| format!("write {}", data_path.display())))
        })
    }

    /// Bakes static difs into the blocks: `lookup` is `stadiflN.mul` (u32 block ids), `index` is `stadifiN.mul`
    /// (one 12-byte entry per id), `data` is `stadifN.mul`. Every id is applied in file order, so a later
    /// duplicate wins; an invalid or empty entry empties its block.
    pub fn bake_static_difs(&mut self, lookup: &[u8], index: &[u8], data: &[u8]) -> eyre::Result<DifBake> {
        if lookup.len() % DIF_LOOKUP_RECORD_SIZE != 0 {
            eyre::bail!("a static dif lookup of {} bytes is not whole u32 block ids", lookup.len());
        }
        let listed = lookup.len() / DIF_LOOKUP_RECORD_SIZE;
        if index.len() < listed * STATICS_INDEX_ENTRY_SIZE {
            eyre::bail!("the static dif lists {listed} blocks, its index has {} bytes", index.len());
        }
        let mut patched = std::collections::HashSet::new();
        let mut report = DifBake { listed, ..DifBake::default() };
        for (dif_index, id) in lookup.chunks_exact(DIF_LOOKUP_RECORD_SIZE).enumerate() {
            let block = u32::from_le_bytes(id.try_into().expect("a 4-byte id")) as usize;
            if block >= self.block_count() {
                report.outside += 1;
                continue;
            }
            patched.insert(block);
            self.blocks[block] = match index_entry(index, dif_index) {
                Some((lookup, len)) => parse_records(data, lookup, len, block)?,
                None => Vec::new(),
            };
        }
        report.distinct = patched.len();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::output_file::block_temp_file;

    const SIZE: MapSizeCells = MapSizeCells { width: 16, height: 16 };

    #[test]
    fn a_failed_write_leaves_both_existing_files() {
        const EXISTING: &[u8] = b"existing";
        /// The two files and the folder that blocks the temporary one.
        const FOLDER_ENTRIES: usize = 3;
        let dir = std::env::temp_dir().join(format!("uocf_statics_failed_write_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (index, data) = (dir.join("staidx0.mul"), dir.join("statics0.mul"));
        for path in [&index, &data] {
            std::fs::write(path, EXISTING).unwrap();
        }
        block_temp_file(&data);
        assert!(ClassicStaticsBlocks::new(SIZE).unwrap().write_mul(&index, &data).is_err());
        for path in [&index, &data] {
            assert_eq!(std::fs::read(path).unwrap(), EXISTING, "{}", path.display());
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), FOLDER_ENTRIES, "no temporary index left behind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn statics_write_in_the_modern_layout_and_read_back() {
        let mut statics = ClassicStaticsBlocks::new(SIZE).unwrap();
        statics.add_static(1, 2, 0x100, 5, 0).unwrap();
        statics.add_static(9, 1, 0x200, -3, 0x21).unwrap();
        let (index, data) = statics.to_mul_bytes();
        assert_eq!(index.len(), 4 * STATICS_INDEX_ENTRY_SIZE);
        // Block 0: lookup 0, one record, the modern extra; block 1 (x 0, y 1) empty; block 2 (x 1, y 0) next.
        assert_eq!([u32_at(&index, 0), u32_at(&index, 4), u32_at(&index, 8)], [0, 7, MODERN_INDEX_EXTRA]);
        assert_eq!([u32_at(&index, 12), u32_at(&index, 16), u32_at(&index, 20)], [EMPTY_INDEX_FIELD; 3]);
        assert_eq!([u32_at(&index, 24), u32_at(&index, 28)], [7, 7]);
        assert_eq!(data.len(), 2 * StaticTile::RAW_SIZE);
        assert_eq!(ClassicStaticsBlocks::from_mul_bytes(&index, &data, SIZE).unwrap(), statics);
    }

    #[test]
    fn an_added_static_follows_its_cell_in_y_x_order() {
        let mut statics = ClassicStaticsBlocks::new(SIZE).unwrap();
        for (x, y, graphic) in [(0, 0, 1u16), (3, 0, 2), (1, 1, 3), (5, 2, 4)] {
            statics.add_static(x, y, graphic, 0, 0).unwrap();
        }
        assert_eq!(statics.add_static(3, 0, 5, 1, 0).unwrap(), 1);
        assert_eq!(statics.add_static(2, 1, 6, 0, 0).unwrap(), 0);
        let graphics: Vec<u16> = statics.block(0).iter().map(|record| record.graphic).collect();
        assert_eq!(graphics, [1, 2, 5, 3, 6, 4]);
        assert_eq!(statics.remove_static(3, 0, 0).unwrap().graphic, 2);
        statics.set_static(3, 0, 0, 7, 2, 3).unwrap();
        assert_eq!(statics.cell_statics(3, 0).unwrap(), [StaticRecord { graphic: 7, x: 3, y: 0, z: 2, hue: 3 }]);
        assert!(statics.remove_static(3, 0, 1).is_err());
    }

    #[test]
    fn difs_bake_in_file_order_and_an_invalid_entry_empties_its_block() {
        let mut statics = ClassicStaticsBlocks::new(SIZE).unwrap();
        statics.add_static(0, 8, 1, 0, 0).unwrap();
        let record = |graphic: u16| {
            let mut bytes = Vec::new();
            StaticRecord { graphic, x: 2, y: 3, z: 4, hue: 5 }.write(&mut bytes);
            bytes
        };
        let data = [record(10), record(11)].concat();
        let mut lookup = Vec::new();
        let mut index = Vec::new();
        // Block 2 gets record 10, then record 11 (the later wins); block 1 is emptied; id 7 is outside.
        for (id, entry) in [(2u32, [0, 7, 0]), (1, [EMPTY_INDEX_FIELD; 3]), (2, [7, 7, 0]), (7, [0, 7, 0])] {
            lookup.extend_from_slice(&id.to_le_bytes());
            entry.iter().for_each(|field: &u32| index.extend_from_slice(&field.to_le_bytes()));
        }
        let report = statics.bake_static_difs(&lookup, &index, &data).unwrap();
        assert_eq!(report, DifBake { listed: 4, distinct: 2, outside: 1 });
        assert_eq!(statics.block(2).iter().map(|record| record.graphic).collect::<Vec<_>>(), [11]);
        assert!(statics.block(1).is_empty());
    }
}
