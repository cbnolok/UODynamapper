//! The Classic map held whole in memory, for editing and writing: `mapN.mul` and `mapNLegacyMUL.uop` (and the
//! `x` editions, which have the same layout), plus the map difs baked in.
//!
//! The writers follow the layout of the modern clients (7.0.20 to the 2024 client), measured on every block of
//! their shipped files (`docs/agent_notes/proposals/2026-11-01-classic-map-writers.md`):
//!
//! - `mapN.mul`: the blocks, 196 bytes each, column by column, nothing else. Each block is a `u32` header holding
//!   the block's index, then 64 cells (`u16` land id, `i8` z) row by row.
//! - `mapNLegacyMUL.uop`: the same blocks, uncompressed, in entries `build/mapNlegacymul/%08d.dat` of 4,096 blocks
//!   (the `x` editions use the same names), plus one trailing block. When the blocks fill whole entries, the
//!   trailing block is an entry of its own holding a copy of the last block; otherwise it ends the last entry, all
//!   zeros. Readers ([`ClassicMapBlocks::from_uop_package`]) drop it.
//!
//! So a modern map file is fully determined by its cells. Reading keeps the cells only: the old clients (Mondain's
//! Legacy, UO Gold) store arbitrary block headers, which a write replaces with the block index (a valid file, not
//! the same bytes).
//!
//! Map difs (`mapdiflN`/`mapdifN`) are baked into the blocks ([`ClassicMapBlocks::bake_map_difs`]): every listed
//! id is applied in file order, so a later duplicate wins, as in the client (`operator[]` over the ids).

use std::path::Path;

use crate::classic::map::{
    classic_map_size, uop_map_entry_name, MapBlock, MapCell, MapSizeBlocks, MapSizeCells, UOP_MAP_BLOCKS_PER_ENTRY,
    UOP_MAP_TRAILING_BLOCKS,
};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{FileReplacement, NewEntryCompression, SaveLayout, UopPackage};

crate::eyre_imports!();

/// Bytes of a map block header (`u32`).
const BLOCK_HEADER_SIZE: usize = size_of::<u32>();
/// Bytes of a map cell in the file: `u16` land id, `i8` z.
const CELL_FILE_SIZE: usize = size_of::<u16>() + size_of::<i8>();
/// Cells in a block.
const CELLS_PER_BLOCK: usize = MapBlock::CELLS_PER_BLOCK as usize;
/// Bytes of a `mapdiflN` record: the `u32` block id.
const DIF_LOOKUP_RECORD_SIZE: usize = size_of::<u32>();
/// UOP version of the shipped `mapNLegacyMUL.uop` packages (the 2024 client).
pub const MAP_UOP_PACKAGE_VERSION: u32 = 5;
/// Entries per block table of the shipped `mapNLegacyMUL.uop` packages.
pub const MAP_UOP_PACKAGE_BLOCK_SIZE: u32 = 1000;
/// Compression of the shipped map entries.
pub const MAP_UOP_COMPRESSION: CompressionFlag = CompressionFlag::None;

/// What [`ClassicMapBlocks::write_uop`] did to the package's entries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MapUopWrite {
    /// Entries whose payload changed (or were added).
    pub written: usize,
    /// Entries of the template left as they were, data block header included.
    pub unchanged: usize,
    /// Template entries past the new last entry, removed.
    pub removed: usize,
}

/// What [`ClassicMapBlocks::bake_map_difs`] applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DifBake {
    /// Ids the lookup file lists (duplicates included); each was applied in order.
    pub listed: usize,
    /// Distinct blocks they patch.
    pub distinct: usize,
    /// Listed ids outside the map, skipped (the client never asks for such a block).
    pub outside: usize,
}

/// A whole Classic map: every block's 64 cells, column by column, as `mapN.mul` stores them.
#[derive(Clone, PartialEq, Eq)]
pub struct ClassicMapBlocks {
    size: MapSizeBlocks,
    /// `CELLS_PER_BLOCK` cells per block, block by block; within a block row by row.
    cells: Vec<(u16, i8)>,
}

impl std::fmt::Debug for ClassicMapBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClassicMapBlocks({}x{} blocks)", self.size.width, self.size.height)
    }
}

impl ClassicMapBlocks {
    /// A map of `size` cells (multiples of 8), every cell land 0 at z 0.
    pub fn new(size: MapSizeCells) -> eyre::Result<Self> {
        let size = size_in_blocks(size)?;
        let block_count = size.width as usize * size.height as usize;
        Ok(Self { size, cells: vec![(0, 0); block_count * CELLS_PER_BLOCK] })
    }

    /// The size of `mapN.mul` file `bytes` of map `map_index`: `size` when given, else the facet's height and the
    /// width its blocks fill ([`classic_map_size`]).
    fn resolve_size(map_index: u32, block_count: u64, trailing: u64, size: Option<MapSizeCells>) -> eyre::Result<MapSizeCells> {
        match size {
            Some(size) => Ok(size),
            None => classic_map_size(map_index, block_count, trailing),
        }
    }

    /// Reads `mapN.mul` bytes. The size is `size` when given, else the one the file holds for map `map_index`.
    pub fn from_mul_bytes(bytes: &[u8], map_index: u32, size: Option<MapSizeCells>) -> eyre::Result<Self> {
        if bytes.len() % MapBlock::PACKED_SIZE != 0 {
            eyre::bail!("{} bytes are not whole map blocks of {} bytes", bytes.len(), MapBlock::PACKED_SIZE);
        }
        let block_count = bytes.len() / MapBlock::PACKED_SIZE;
        let size = Self::resolve_size(map_index, block_count as u64, 0, size)?;
        let mut map = Self::new(size)?;
        if map.block_count() != block_count {
            eyre::bail!(
                "a {}x{} map has {} blocks, the file {block_count}",
                size.width,
                size.height,
                map.block_count()
            );
        }
        map.read_blocks(bytes);
        Ok(map)
    }

    /// Reads a `mapN.mul` (or `mapNx.mul`) file; see [`Self::from_mul_bytes`].
    pub fn read_mul(path: &Path, map_index: u32, size: Option<MapSizeCells>) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_mul_bytes(&bytes, map_index, size).wrap_err_with(|| format!("read {}", path.display()))
    }

    /// Reads the blocks of a `map{map_index}LegacyMUL.uop` package (or its `x` edition): its entries from the first,
    /// in order, without the trailing block. The size is `size` when given, else the one the blocks hold.
    pub fn from_uop_package(package: &UopPackage, map_index: u32, size: Option<MapSizeCells>) -> eyre::Result<Self> {
        let mut bytes = Vec::new();
        for entry_index in 0.. {
            let name = uop_map_entry_name(map_index, entry_index);
            let Some(payload) = package.unpack_file_by_hash(hash_file_name_single(&name))? else {
                break;
            };
            bytes.extend_from_slice(&payload);
        }
        if bytes.is_empty() || bytes.len() % MapBlock::PACKED_SIZE != 0 {
            eyre::bail!(
                "the map{map_index} entries hold {} bytes, not whole map blocks of {} bytes",
                bytes.len(),
                MapBlock::PACKED_SIZE
            );
        }
        let stored = (bytes.len() / MapBlock::PACKED_SIZE) as u64;
        let size = Self::resolve_size(map_index, stored, UOP_MAP_TRAILING_BLOCKS, size)?;
        let mut map = Self::new(size)?;
        let blocks = map.block_count() as u64;
        if stored != blocks + UOP_MAP_TRAILING_BLOCKS {
            eyre::bail!(
                "a {}x{} map takes {blocks} blocks and {UOP_MAP_TRAILING_BLOCKS} trailing block, the package holds \
                 {stored}",
                size.width,
                size.height
            );
        }
        map.read_blocks(&bytes[..map.block_count() * MapBlock::PACKED_SIZE]);
        Ok(map)
    }

    /// Reads a `mapNLegacyMUL.uop` file; see [`Self::from_uop_package`].
    pub fn read_uop(path: &Path, map_index: u32, size: Option<MapSizeCells>) -> eyre::Result<Self> {
        let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
        Self::from_uop_package(&package, map_index, size).wrap_err_with(|| format!("read {}", path.display()))
    }

    fn read_blocks(&mut self, bytes: &[u8]) {
        for (block, cells) in bytes.chunks_exact(MapBlock::PACKED_SIZE).zip(self.cells.chunks_exact_mut(CELLS_PER_BLOCK)) {
            for (cell, out) in block[BLOCK_HEADER_SIZE..].chunks_exact(CELL_FILE_SIZE).zip(cells) {
                *out = (u16::from_le_bytes([cell[0], cell[1]]), cell[2] as i8);
            }
        }
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
        self.size.width as usize * self.size.height as usize
    }

    /// The index of the block holding cell `(x, y)` and the cell's index in it.
    fn locate(&self, x: u32, y: u32) -> eyre::Result<(usize, usize)> {
        let size = self.size_cells();
        if x >= size.width || y >= size.height {
            eyre::bail!("({x}, {y}) is outside the {}x{} map", size.width, size.height);
        }
        let block = (x / MapBlock::CELLS_PER_ROW) as usize * self.size.height as usize
            + (y / MapBlock::CELLS_PER_COLUMN) as usize;
        let cell = ((y % MapBlock::CELLS_PER_COLUMN) * MapBlock::CELLS_PER_ROW + x % MapBlock::CELLS_PER_ROW) as usize;
        Ok((block, cell))
    }

    /// The land id and z of cell `(x, y)`.
    pub fn cell(&self, x: u32, y: u32) -> eyre::Result<(u16, i8)> {
        let (block, cell) = self.locate(x, y)?;
        Ok(self.cells[block * CELLS_PER_BLOCK + cell])
    }

    /// Sets the land id and z of cell `(x, y)`.
    pub fn set_cell(&mut self, x: u32, y: u32, land: u16, z: i8) -> eyre::Result<()> {
        let (block, cell) = self.locate(x, y)?;
        self.cells[block * CELLS_PER_BLOCK + cell] = (land, z);
        Ok(())
    }

    /// The cells of block `block_index` (column by column), row by row.
    pub fn block_cells(&self, block_index: usize) -> &[(u16, i8)] {
        &self.cells[block_index * CELLS_PER_BLOCK..(block_index + 1) * CELLS_PER_BLOCK]
    }

    /// The cells of block `(block_x, block_y)` as a [`MapBlock`].
    pub fn map_block(&self, block_x: u32, block_y: u32) -> eyre::Result<MapBlock> {
        if block_x >= self.size.width || block_y >= self.size.height {
            eyre::bail!("block ({block_x}, {block_y}) is outside the {}x{} blocks", self.size.width, self.size.height);
        }
        let index = block_x as usize * self.size.height as usize + block_y as usize;
        let mut block = MapBlock::default();
        block.internal_coords = crate::classic::map::MapBlockRelPos { x: block_x, y: block_y };
        for (out, &(id, z)) in block.cells.iter_mut().zip(self.block_cells(index)) {
            *out = MapCell { id, z, _pad: 0 };
        }
        Ok(block)
    }

    /// Writes block `index` (header = its index) into `out`.
    fn write_block(&self, index: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(&(index as u32).to_le_bytes());
        for &(id, z) in self.block_cells(index) {
            out.extend_from_slice(&id.to_le_bytes());
            out.push(z as u8);
        }
    }

    /// The `mapN.mul` bytes, in the modern layout.
    pub fn to_mul_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.block_count() * MapBlock::PACKED_SIZE);
        for index in 0..self.block_count() {
            self.write_block(index, &mut out);
        }
        out
    }

    /// Writes `mapN.mul` (or `mapNx.mul`).
    pub fn write_mul(&self, path: &Path) -> eyre::Result<()> {
        std::fs::write(path, self.to_mul_bytes()).wrap_err_with(|| format!("write {}", path.display()))
    }

    /// The payloads of the `mapNLegacyMUL.uop` entries, in entry order, with the trailing block.
    pub fn uop_entry_payloads(&self) -> Vec<Vec<u8>> {
        let blocks_per_entry = UOP_MAP_BLOCKS_PER_ENTRY as usize;
        let mut entries = Vec::with_capacity(self.block_count().div_ceil(blocks_per_entry) + 1);
        for first in (0..self.block_count()).step_by(blocks_per_entry) {
            let last = (first + blocks_per_entry).min(self.block_count());
            let mut payload = Vec::with_capacity((last - first + 1) * MapBlock::PACKED_SIZE);
            for index in first..last {
                self.write_block(index, &mut payload);
            }
            entries.push(payload);
        }
        if self.block_count() % blocks_per_entry == 0 {
            // A copy of the last block, in an entry of its own.
            let mut trailing = Vec::with_capacity(MapBlock::PACKED_SIZE);
            self.write_block(self.block_count() - 1, &mut trailing);
            entries.push(trailing);
        } else {
            let last = entries.last_mut().expect("a map has at least one block");
            last.resize(last.len() + MapBlock::PACKED_SIZE, 0);
        }
        entries
    }

    /// Writes `path` as `map{map_index}LegacyMUL.uop` (the entry names of its `x` edition are the same). With
    /// `template` (a loaded map package, usually the one read), its layout is kept: entries whose payload is the
    /// same stay byte for byte, data block header included; changed ones get a fresh timestamp header; entries
    /// past the new last one are removed. Without, a new package in the shipped layout (version 5, 1,000 entries
    /// per table, uncompressed entries in name order).
    pub fn write_uop(&self, path: &Path, map_index: u32, template: Option<UopPackage>) -> eyre::Result<MapUopWrite> {
        let payloads = self.uop_entry_payloads();
        let mut report = MapUopWrite::default();
        let (mut package, layout) = match template {
            Some(package) => (package, SaveLayout::Preserve),
            None => (UopPackage::new(MAP_UOP_PACKAGE_VERSION, MAP_UOP_PACKAGE_BLOCK_SIZE)?, SaveLayout::Compact),
        };
        for (entry_index, payload) in payloads.iter().enumerate() {
            let name = uop_map_entry_name(map_index, entry_index as u32);
            let hash = hash_file_name_single(&name);
            if package.get_file_by_hash(hash).is_some() {
                match package.replace_file(hash, payload, MAP_UOP_COMPRESSION)? {
                    FileReplacement::Replaced => report.written += 1,
                    FileReplacement::Unchanged => report.unchanged += 1,
                }
            } else {
                package.add_entry(hash, payload, NewEntryCompression::Flag(MAP_UOP_COMPRESSION))?;
                report.written += 1;
            }
        }
        for entry_index in payloads.len().. {
            let hash = hash_file_name_single(&uop_map_entry_name(map_index, entry_index as u32));
            if !package.remove_entry(hash) {
                break;
            }
            report.removed += 1;
        }
        package
            .finalize_and_save_with_layout(path, layout)
            .wrap_err_with(|| format!("write {}", path.display()))?;
        Ok(report)
    }

    /// Bakes map difs into the blocks: `lookup` is `mapdiflN.mul` (u32 block ids), `blocks` is `mapdifN.mul` (the
    /// 196-byte blocks, one per id). Every id is applied in file order, so a later duplicate wins.
    pub fn bake_map_difs(&mut self, lookup: &[u8], blocks: &[u8]) -> eyre::Result<DifBake> {
        if lookup.len() % DIF_LOOKUP_RECORD_SIZE != 0 {
            eyre::bail!("a map dif lookup of {} bytes is not whole u32 block ids", lookup.len());
        }
        let listed = lookup.len() / DIF_LOOKUP_RECORD_SIZE;
        if blocks.len() < listed * MapBlock::PACKED_SIZE {
            eyre::bail!(
                "the map dif lists {listed} blocks, its data holds {} bytes ({} needed)",
                blocks.len(),
                listed * MapBlock::PACKED_SIZE
            );
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
            let source = &blocks[dif_index * MapBlock::PACKED_SIZE..(dif_index + 1) * MapBlock::PACKED_SIZE];
            let cells = &mut self.cells[block * CELLS_PER_BLOCK..(block + 1) * CELLS_PER_BLOCK];
            for (cell, out) in source[BLOCK_HEADER_SIZE..].chunks_exact(CELL_FILE_SIZE).zip(cells) {
                *out = (u16::from_le_bytes([cell[0], cell[1]]), cell[2] as i8);
            }
        }
        report.distinct = patched.len();
        Ok(report)
    }
}

/// `size` in blocks; fails when a side is zero or not a multiple of the block side.
pub(crate) fn size_in_blocks(size: MapSizeCells) -> eyre::Result<MapSizeBlocks> {
    if size.width == 0
        || size.height == 0
        || size.width % MapBlock::CELLS_PER_ROW != 0
        || size.height % MapBlock::CELLS_PER_COLUMN != 0
    {
        eyre::bail!(
            "a {}x{} map is not whole blocks of {}x{} cells",
            size.width,
            size.height,
            MapBlock::CELLS_PER_ROW,
            MapBlock::CELLS_PER_COLUMN
        );
    }
    Ok(MapSizeBlocks { width: size.width / MapBlock::CELLS_PER_ROW, height: size.height / MapBlock::CELLS_PER_COLUMN })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: MapSizeCells = MapSizeCells { width: 16, height: 24 };

    fn numbered(size: MapSizeCells) -> ClassicMapBlocks {
        let mut map = ClassicMapBlocks::new(size).unwrap();
        for x in 0..size.width {
            for y in 0..size.height {
                map.set_cell(x, y, (x * 1000 + y) as u16, (x as i32 - y as i32) as i8).unwrap();
            }
        }
        map
    }

    #[test]
    fn mul_bytes_hold_the_block_index_and_read_back() {
        let map = numbered(SMALL);
        let bytes = map.to_mul_bytes();
        assert_eq!(bytes.len(), map.block_count() * MapBlock::PACKED_SIZE);
        for index in 0..map.block_count() {
            let header = &bytes[index * MapBlock::PACKED_SIZE..][..BLOCK_HEADER_SIZE];
            assert_eq!(u32::from_le_bytes(header.try_into().unwrap()), index as u32);
        }
        // Cell (9, 2): block x 1, y 0 = index 3 (3 blocks per column); cell 2 * 8 + 1.
        let offset = 3 * MapBlock::PACKED_SIZE + BLOCK_HEADER_SIZE + (2 * 8 + 1) * CELL_FILE_SIZE;
        assert_eq!(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]), 9002);
        assert_eq!(ClassicMapBlocks::from_mul_bytes(&bytes, 0, Some(SMALL)).unwrap(), map);
    }

    #[test]
    fn uop_entries_end_with_the_trailing_block_rule() {
        let map = numbered(SMALL);
        // 6 blocks: one partial entry, a zero block appended.
        let entries = map.uop_entry_payloads();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].len(), (map.block_count() + 1) * MapBlock::PACKED_SIZE);
        assert!(entries[0][map.block_count() * MapBlock::PACKED_SIZE..].iter().all(|&byte| byte == 0));

        // 4,096 blocks: a full entry, then a copy of the last block.
        let full = numbered(MapSizeCells { width: 8 * 8, height: 8 * 512 });
        let entries = full.uop_entry_payloads();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1], entries[0][entries[0].len() - MapBlock::PACKED_SIZE..]);
    }

    #[test]
    fn uop_write_reads_back_and_keeps_unchanged_entries() {
        let dir = std::env::temp_dir().join(format!("uocf-map-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map2LegacyMUL.uop");
        let mut map = numbered(SMALL);
        assert_eq!(map.write_uop(&path, 2, None).unwrap(), MapUopWrite { written: 1, unchanged: 0, removed: 0 });
        assert_eq!(ClassicMapBlocks::read_uop(&path, 2, Some(SMALL)).unwrap(), map);

        let again = dir.join("again.uop");
        let template = UopPackage::load(&path).unwrap();
        assert_eq!(map.write_uop(&again, 2, Some(template)).unwrap().unchanged, 1);
        assert_eq!(std::fs::read(&again).unwrap(), std::fs::read(&path).unwrap());

        map.set_cell(3, 4, 0x1234, -5).unwrap();
        let edited = dir.join("edited.uop");
        let template = UopPackage::load(&path).unwrap();
        assert_eq!(map.write_uop(&edited, 2, Some(template)).unwrap().written, 1);
        assert_eq!(ClassicMapBlocks::read_uop(&edited, 2, Some(SMALL)).unwrap().cell(3, 4).unwrap(), (0x1234, -5));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn difs_bake_in_file_order_and_skip_outside_ids() {
        let mut map = numbered(SMALL);
        let mut lookup = Vec::new();
        let mut blocks = Vec::new();
        for (id, land) in [(1u32, 10u16), (99, 11), (1, 12), (4, 13)] {
            lookup.extend_from_slice(&id.to_le_bytes());
            blocks.extend_from_slice(&id.to_le_bytes());
            for _ in 0..CELLS_PER_BLOCK {
                blocks.extend_from_slice(&land.to_le_bytes());
                blocks.push(7);
            }
        }
        let report = map.bake_map_difs(&lookup, &blocks).unwrap();
        assert_eq!(report, DifBake { listed: 4, distinct: 2, outside: 1 });
        assert!(map.block_cells(1).iter().all(|&cell| cell == (12, 7)));
        assert!(map.block_cells(4).iter().all(|&cell| cell == (13, 7)));
        assert_eq!(map.block_cells(0)[0], (0, 0));
    }
}
