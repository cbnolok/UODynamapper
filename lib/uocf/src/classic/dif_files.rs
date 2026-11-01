//! Classic map and static dif files, read and written as stored: [`MapDifFile`] (`mapdiflN.mul` and `mapdifN.mul`)
//! and [`StaticDifFile`] (`stadiflN.mul`, `stadifiN.mul` and `stadifN.mul`). An unchanged pair or triple comes back
//! byte for byte.
//!
//! The readers that apply the difs are [`crate::classic::map_statics_diff`]; the converters bake them into the
//! map and statics instead ([`crate::classic::map_write`]). These types are for editing the difs themselves.
//!
//! The layouts, as UO Gold, Mondain's Legacy and the current clients ship them:
//!
//! - `mapdiflN` lists the patched block ids (`u32`, in no particular order; UO Gold's `mapdifl1` repeats 4224 of
//!   them), and `mapdifN` holds one 196-byte block per listed id, in list order, with nothing after them.
//! - `stadiflN` lists the patched block ids the same way, and `stadifiN` holds one 12-byte index record per listed
//!   id (lookup, length, extra) over the 7-byte static records of `stadifN`. A record whose lookup, length or
//!   extra is `-1`, or whose length is 0, empties its block. The shipped files lay the payloads out from offset 0
//!   in list order with nothing between them, and fill the extra fields of every record with unused values, which
//!   are kept.
//!
//! When an id is listed more than once, the readers and the client take the last listing.

crate::eyre_imports!();

use std::path::Path;

use crate::classic::indexed_mul::{IndexedEntry, IndexedMul};
use crate::classic::statics_write::StaticRecord;
use crate::utils::output_file::write_through_temp;

/// Bytes of one listed block id.
pub const DIF_LIST_ID_BYTES: usize = size_of::<u32>();
/// Cells of a map block (8 by 8).
pub const MAP_BLOCK_CELLS: usize = 64;
/// Bytes of one map cell: the land id (`u16`) and its z (`i8`).
pub const MAP_CELL_BYTES: usize = 3;
/// Bytes of a map block's header.
pub const MAP_BLOCK_HEADER_BYTES: usize = size_of::<u32>();
/// Bytes of one map block.
pub const MAP_DIF_BLOCK_BYTES: usize = MAP_BLOCK_HEADER_BYTES + MAP_BLOCK_CELLS * MAP_CELL_BYTES;
/// Bytes of one static record.
pub const STATIC_RECORD_BYTES: usize = 7;

fn read_list(bytes: &[u8], name: &str) -> eyre::Result<Vec<u32>> {
    if bytes.len() % DIF_LIST_ID_BYTES != 0 {
        eyre::bail!("{name}: {} bytes are not whole {DIF_LIST_ID_BYTES}-byte block ids", bytes.len());
    }
    Ok(bytes.chunks_exact(DIF_LIST_ID_BYTES).map(|id| u32::from_le_bytes(id.try_into().expect("four bytes"))).collect())
}

fn write_list(ids: &[u32]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

fn read_file(path: &Path) -> eyre::Result<Vec<u8>> {
    std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))
}

fn write_file(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    write_through_temp(path, |temp| std::fs::write(temp, bytes).wrap_err_with(|| format!("write {}", temp.display())))
}

/// One map block as `mapdifN` stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDifBlock {
    /// The block header (unused by the clients).
    pub header: u32,
    /// The 64 cells, row by row: land id and z.
    pub cells: Vec<(u16, i8)>,
}

impl MapDifBlock {
    fn from_bytes(bytes: &[u8]) -> Self {
        let header = u32::from_le_bytes(bytes[..MAP_BLOCK_HEADER_BYTES].try_into().expect("four bytes"));
        let cells = bytes[MAP_BLOCK_HEADER_BYTES..]
            .chunks_exact(MAP_CELL_BYTES)
            .map(|cell| (u16::from_le_bytes([cell[0], cell[1]]), cell[2] as i8))
            .collect();
        Self { header, cells }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.header.to_le_bytes());
        for &(land, z) in &self.cells {
            out.extend_from_slice(&land.to_le_bytes());
            out.push(z as u8);
        }
    }
}

/// `mapdiflN.mul` and `mapdifN.mul`: the listed block ids and their blocks, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapDifFile {
    pub entries: Vec<(u32, MapDifBlock)>,
}

impl MapDifFile {
    /// Reads the list and the blocks; the blocks file must hold exactly one block per listed id.
    pub fn from_bytes(list: &[u8], blocks: &[u8]) -> eyre::Result<Self> {
        let ids = read_list(list, "mapdifl")?;
        let expected = ids.len() * MAP_DIF_BLOCK_BYTES;
        if blocks.len() != expected {
            eyre::bail!(
                "mapdif: {} bytes, but its list names {} blocks of {MAP_DIF_BLOCK_BYTES} bytes ({expected} bytes)",
                blocks.len(),
                ids.len()
            );
        }
        let entries = ids.into_iter().zip(blocks.chunks_exact(MAP_DIF_BLOCK_BYTES).map(MapDifBlock::from_bytes)).collect();
        Ok(Self { entries })
    }

    pub fn load(list_path: &Path, blocks_path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read_file(list_path)?, &read_file(blocks_path)?)
            .wrap_err_with(|| format!("{} and {}", list_path.display(), blocks_path.display()))
    }

    /// The list and the blocks.
    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        let ids: Vec<u32> = self.entries.iter().map(|(id, _)| *id).collect();
        let mut blocks = Vec::with_capacity(self.entries.len() * MAP_DIF_BLOCK_BYTES);
        for (id, block) in &self.entries {
            if block.cells.len() != MAP_BLOCK_CELLS {
                eyre::bail!("block {id}: {} cells, a map block has {MAP_BLOCK_CELLS}", block.cells.len());
            }
            block.write(&mut blocks);
        }
        Ok((write_list(&ids), blocks))
    }

    /// Writes both files, each through a temporary file in its folder (the blocks first).
    pub fn save(&self, list_path: &Path, blocks_path: &Path) -> eyre::Result<()> {
        let (list, blocks) = self.to_bytes()?;
        write_file(blocks_path, &blocks)?;
        write_file(list_path, &list)
    }

    /// The block the client reads for `block_id`: its last listing.
    pub fn block(&self, block_id: u32) -> Option<&MapDifBlock> {
        self.entries.iter().rev().find(|(id, _)| *id == block_id).map(|(_, block)| block)
    }

    /// Sets the block of `block_id`: in its last listing when it has one, else in a new listing at the end.
    pub fn set_block(&mut self, block_id: u32, block: MapDifBlock) -> eyre::Result<()> {
        if block.cells.len() != MAP_BLOCK_CELLS {
            eyre::bail!("block {block_id}: {} cells, a map block has {MAP_BLOCK_CELLS}", block.cells.len());
        }
        match self.entries.iter_mut().rev().find(|(id, _)| *id == block_id) {
            Some((_, slot)) => *slot = block,
            None => self.entries.push((block_id, block)),
        }
        Ok(())
    }

    /// Removes every listing of `block_id`; returns how many there were.
    pub fn remove_block(&mut self, block_id: u32) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(id, _)| *id != block_id);
        before - self.entries.len()
    }
}

/// `stadiflN.mul`, `stadifiN.mul` and `stadifN.mul`: the listed block ids and, at the same positions, their index
/// entries over the statics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDifFile {
    pub block_ids: Vec<u32>,
    /// One entry per listed id; an absent entry empties its block.
    pub statics: IndexedMul,
}

impl StaticDifFile {
    /// Reads the three files; the index must hold one record per listed id and a present entry whole records.
    pub fn from_bytes(list: &[u8], index: &[u8], statics: &[u8]) -> eyre::Result<Self> {
        let block_ids = read_list(list, "stadifl")?;
        let statics = IndexedMul::from_bytes(index, statics)?;
        if statics.entries.len() != block_ids.len() || !statics.index_tail.is_empty() {
            eyre::bail!(
                "stadifi: {} index records and {} more bytes for the {} ids its list names",
                statics.entries.len(),
                statics.index_tail.len(),
                block_ids.len()
            );
        }
        if let Some((position, payload)) = statics
            .entries
            .iter()
            .enumerate()
            .find_map(|(position, entry)| entry.payload().filter(|payload| payload.len() % STATIC_RECORD_BYTES != 0).map(|payload| (position, payload)))
        {
            eyre::bail!(
                "stadifi record {position} (block {}): {} bytes are not whole {STATIC_RECORD_BYTES}-byte statics",
                block_ids[position],
                payload.len()
            );
        }
        Ok(Self { block_ids, statics })
    }

    pub fn load(list_path: &Path, index_path: &Path, statics_path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read_file(list_path)?, &read_file(index_path)?, &read_file(statics_path)?).wrap_err_with(|| {
            format!("{}, {} and {}", list_path.display(), index_path.display(), statics_path.display())
        })
    }

    /// The list, the index and the statics.
    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        if self.statics.entries.len() != self.block_ids.len() {
            eyre::bail!("{} index entries for {} listed ids", self.statics.entries.len(), self.block_ids.len());
        }
        let (index, statics) = self.statics.to_bytes()?;
        Ok((write_list(&self.block_ids), index, statics))
    }

    /// Writes the three files, each through a temporary file in its folder (the statics first, the list last).
    pub fn save(&self, list_path: &Path, index_path: &Path, statics_path: &Path) -> eyre::Result<()> {
        let (list, index, statics) = self.to_bytes()?;
        write_file(statics_path, &statics)?;
        write_file(index_path, &index)?;
        write_file(list_path, &list)
    }

    fn last_position(&self, block_id: u32) -> Option<usize> {
        self.block_ids.iter().rposition(|id| *id == block_id)
    }

    /// The statics the client reads for `block_id` (its last listing): `None` when it is not listed, empty when
    /// the listing empties the block.
    pub fn block(&self, block_id: u32) -> Option<Vec<StaticRecord>> {
        let position = self.last_position(block_id)?;
        let payload = self.statics.entries[position].payload().unwrap_or_default();
        Some(payload.chunks_exact(STATIC_RECORD_BYTES).map(read_static).collect())
    }

    /// Sets the statics of `block_id`, in its last listing when it has one, else in a new listing at the end. An
    /// empty `records` empties the block (with the file's most common absent record).
    pub fn set_block(&mut self, block_id: u32, records: &[StaticRecord]) {
        let entry = if records.is_empty() {
            self.statics.absent_record()
        } else {
            let extra = self.statics.entries.iter().find_map(IndexedEntry::extra).unwrap_or_default();
            let mut payload = Vec::with_capacity(records.len() * STATIC_RECORD_BYTES);
            records.iter().for_each(|record| write_static(record, &mut payload));
            IndexedEntry::Present { extra, payload }
        };
        let position = self.last_position(block_id).unwrap_or_else(|| {
            self.block_ids.push(block_id);
            self.block_ids.len() - 1
        });
        self.statics.set(position, entry);
    }

    /// Removes every listing of `block_id`; returns how many there were.
    pub fn remove_block(&mut self, block_id: u32) -> usize {
        let positions: Vec<usize> = (0..self.block_ids.len()).filter(|&position| self.block_ids[position] == block_id).collect();
        for &position in positions.iter().rev() {
            self.block_ids.remove(position);
            self.statics.entries.remove(position);
        }
        // Renumber the data order past the removed positions.
        self.statics.data_order = self
            .statics
            .data_order
            .iter()
            .filter(|position| !positions.contains(position))
            .map(|&position| position - positions.iter().filter(|&&removed| removed < position).count())
            .collect();
        positions.len()
    }
}

fn read_static(bytes: &[u8]) -> StaticRecord {
    StaticRecord {
        graphic: u16::from_le_bytes([bytes[0], bytes[1]]),
        x: bytes[2],
        y: bytes[3],
        z: bytes[4] as i8,
        hue: u16::from_le_bytes([bytes[5], bytes[6]]),
    }
}

fn write_static(record: &StaticRecord, out: &mut Vec<u8>) {
    out.extend_from_slice(&record.graphic.to_le_bytes());
    out.push(record.x);
    out.push(record.y);
    out.push(record.z as u8);
    out.extend_from_slice(&record.hue.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(seed: u16) -> MapDifBlock {
        MapDifBlock { header: u32::from(seed), cells: (0..MAP_BLOCK_CELLS as u16).map(|cell| (seed + cell, -(cell as i8))).collect() }
    }

    #[test]
    fn map_difs_round_trip_and_edit_the_last_listing() {
        let mut file = MapDifFile { entries: vec![(7, block(1)), (3, block(2)), (7, block(3))] };
        let (list, blocks) = file.to_bytes().unwrap();
        assert_eq!(blocks.len(), 3 * MAP_DIF_BLOCK_BYTES);
        let read = MapDifFile::from_bytes(&list, &blocks).unwrap();
        assert_eq!(read, file);
        assert_eq!(read.block(7), Some(&block(3)), "the last listing wins");
        file.set_block(7, block(9)).unwrap();
        assert_eq!(file.entries[0].1, block(1));
        assert_eq!(file.entries[2].1, block(9));
        file.set_block(11, block(4)).unwrap();
        assert_eq!(file.entries.last().unwrap().0, 11);
        assert_eq!(file.remove_block(7), 2);
        assert!(MapDifFile::from_bytes(&list, &blocks[1..]).is_err());
        assert!(file.set_block(1, MapDifBlock { header: 0, cells: Vec::new() }).is_err());
    }

    #[test]
    fn static_difs_round_trip_and_edit_the_last_listing() {
        let record = |graphic| StaticRecord { graphic, x: 1, y: 2, z: -3, hue: 4 };
        let mut list = Vec::new();
        let mut index = Vec::new();
        let mut statics = Vec::new();
        for (id, records) in [(5u32, vec![record(10), record(11)]), (2, vec![]), (5, vec![record(12)])] {
            list.extend_from_slice(&id.to_le_bytes());
            let (lookup, size) = if records.is_empty() { (u32::MAX, u32::MAX) } else { (statics.len() as u32, (records.len() * STATIC_RECORD_BYTES) as u32) };
            records.iter().for_each(|r| write_static(r, &mut statics));
            [lookup, size, 0xDEAD_0000 + id].iter().for_each(|field| index.extend_from_slice(&field.to_le_bytes()));
        }
        let mut file = StaticDifFile::from_bytes(&list, &index, &statics).unwrap();
        assert_eq!(file.to_bytes().unwrap(), (list.clone(), index.clone(), statics.clone()));
        assert_eq!(file.block(5), Some(vec![record(12)]));
        assert_eq!(file.block(2), Some(Vec::new()), "an absent record empties the block");
        assert_eq!(file.block(9), None);
        file.set_block(9, &[record(20)]);
        file.set_block(2, &[record(21)]);
        assert_eq!(file.remove_block(5), 2);
        let (list, index, statics) = file.to_bytes().unwrap();
        let read = StaticDifFile::from_bytes(&list, &index, &statics).unwrap();
        assert_eq!(read.block_ids, vec![2, 9]);
        assert_eq!(read.block(2), Some(vec![record(21)]));
        assert_eq!(read.block(9), Some(vec![record(20)]));
        assert!(StaticDifFile::from_bytes(&list[4..], &index, &statics).is_err());
    }
}
