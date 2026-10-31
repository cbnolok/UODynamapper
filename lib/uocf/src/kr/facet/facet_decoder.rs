//! # UO Kingdom Reborn Facet Decoder
//!
//! Decodes the sectors of the Kingdom Reborn facet packages (`facetN.uop` of `UOKR.exe`, 2008). They are the
//! Enhanced Client sectors (`crate::enhanced::facet_sector`) with 3 more bytes per square, so they decode with
//! the shared codec in its `ClientDialect::KingdomReborn` square layout into a [`DecodedEcFacet`] whose tiles
//! carry [`KrSquareLand`](crate::enhanced::facet_decoder::KrSquareLand).
//!
//! ## Sector entry `build/sectors/facet_%02d/%08d.bin` (little endian)
//!
//! ```text
//! u8  facet                    == the package's facet index
//! u16 sector_id                == the entry's sector id (column-major over ceil(w/64) x ceil(h/64))
//! 4096 squares, x outer, y inner:
//!   i8  z
//!   u16 material               the TerrainDefinition id (EcFacetTile::land_graphic)
//!   u8  subtype                the alias of the material (0/1/2 in the data)
//!   u16 classic_land           the legacy (Classic) land id, stored but not read by the client
//!   u8  n; n x { u8 direction, i8 z, u32 land }        delimiters: the neighbour's (z, material), as in EC
//!   u8  m; m x { u32 graphic, i8 z, u32 hue }          statics, as in EC
//! ```
//!
//! Field names follow `UOFacet::Sector_Read` 0x007b688f (`ghidra-facets/results/uokr_facets.md`). The client
//! only logs leftover bytes, a direction above 7 and a header naming another facet or sector; this decoder
//! rejects them. The package also holds `build/sectors/facet_%02d/data.map` (u32 height, u32 width,
//! uncompressed), as in EC, which the client does not read. The squares past the map edge of a partial sector are all zero bytes. Decoding needs no tile dictionary:
//! [`decode_facet_bin`] takes the land from the stored `classic_land`, and keeps every static.

use crate::classic::map::MapBlock;
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_decoder::{decode_sector, DecodedBlockClassic, DecodedEcFacet, DecodedFacet, StaticTile};
use crate::enhanced::facet_sector::{sector_entry_path, SECTOR_SIDE};
use crate::uop_container as uop;
use crate::uop_container::package::UopPackage;

crate::eyre_imports!();

/// Classic map blocks along a sector side.
const BLOCKS_PER_SECTOR_SIDE: u32 = SECTOR_SIDE / MapBlock::CELLS_PER_ROW;

// region: --- Public API (Convenience & Application Use)

/// Decodes a KR sector payload into the 64 Classic 8x8 blocks it covers (block `i` at `(i % 8, i / 8)`), with
/// the stored Classic land ids and every static.
pub fn decode_facet_bin(data: &[u8]) -> eyre::Result<DecodedFacet> {
    let sector = decode_facet_bin_raw(data)?;
    let mut blocks: Vec<DecodedBlockClassic> = (0..BLOCKS_PER_SECTOR_SIDE * BLOCKS_PER_SECTOR_SIDE)
        .map(|i| {
            let mut block = MapBlock::default();
            block.internal_coords.x = i % BLOCKS_PER_SECTOR_SIDE;
            block.internal_coords.y = i / BLOCKS_PER_SECTOR_SIDE;
            DecodedBlockClassic { block, statics: Vec::new() }
        })
        .collect();
    for (index, tile) in sector.tiles.iter().enumerate() {
        let (x, y) = (index as u32 / SECTOR_SIDE, index as u32 % SECTOR_SIDE);
        let block_index = ((y / MapBlock::CELLS_PER_COLUMN) * BLOCKS_PER_SECTOR_SIDE + x / MapBlock::CELLS_PER_ROW) as usize;
        let (cell_x, cell_y) = (x % MapBlock::CELLS_PER_ROW, y % MapBlock::CELLS_PER_COLUMN);
        let decoded = &mut blocks[block_index];
        let cell = decoded.block.cell_as_mut(cell_x, cell_y)?;
        cell.id = tile.classic_land_id();
        cell.z = tile.z;
        decoded.statics.extend(tile.statics.iter().map(|item| StaticTile {
            x: cell_x as u8,
            y: cell_y as u8,
            ..item.clone()
        }));
    }
    Ok(DecodedFacet { blocks })
}

/// Decodes a KR sector payload ([`decode_sector`] with `ClientDialect::KingdomReborn`): fails on a delimiter
/// direction above 7, a read past the end or bytes left after the last square.
pub fn decode_facet_bin_raw(data: &[u8]) -> eyre::Result<DecodedEcFacet> {
    decode_sector(data, ClientDialect::KingdomReborn)
}

/// Reads and decodes sector `block_id` of facet `map_index` into Classic blocks ([`decode_facet_bin`]).
pub fn read_facet_block(package: &UopPackage, map_index: u8, block_id: u32) -> eyre::Result<DecodedFacet> {
    decode_facet_bin(&read_facet_block_bytes(package, map_index, block_id)?)
}

/// Reads and decodes sector `block_id` of facet `map_index` ([`decode_facet_bin_raw`]), checking that its header
/// names that facet and sector.
pub fn read_facet_block_raw(package: &UopPackage, map_index: u8, block_id: u32) -> eyre::Result<DecodedEcFacet> {
    let sector = decode_facet_bin_raw(&read_facet_block_bytes(package, map_index, block_id)?)?;
    if sector.facet_id != map_index || u32::from(sector.file_id) != block_id {
        eyre::bail!(
            "{}: header facet {} sector {}, expected facet {map_index} sector {block_id}",
            sector_entry_path(map_index, block_id),
            sector.facet_id,
            sector.file_id
        );
    }
    Ok(sector)
}

fn read_facet_block_bytes(package: &UopPackage, map_index: u8, block_id: u32) -> eyre::Result<Vec<u8>> {
    let target_path = sector_entry_path(map_index, block_id);
    let target_hash: u64 = uop::hash::hash_file_name_single(&target_path);
    package.unpack_file_by_hash(target_hash)?.ok_or_else(|| eyre!("File not found in package: {target_path}"))
}

// endregion: --- Public API

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::facet_decoder::{EcFacetDelimiter, KrSquareLand};
    use crate::enhanced::facet_sector::{empty_sector, encode_sector, square_index, SECTOR_HEADER_SIZE};

    const FACET: u8 = 4;
    const SECTOR: u32 = 7;
    /// Bytes of a KR square without delimiters and statics.
    const KR_SQUARE_MIN_SIZE: usize = 8;

    fn sample() -> DecodedEcFacet {
        let mut sector = empty_sector(FACET, SECTOR, ClientDialect::KingdomReborn).unwrap();
        let tile = &mut sector.tiles[square_index(9, 10)];
        tile.z = -3;
        tile.land_graphic = 54;
        tile.kr = Some(KrSquareLand { subtype: 2, classic_land: 0x1A4 });
        tile.delimiters.push(EcFacetDelimiter { direction: 4, z: 7, graphic_id: 1 });
        tile.statics.push(StaticTile { graphic_id: 6345, x: 9, y: 10, z: 12, hue: 8883 });
        sector
    }

    #[test]
    fn kr_squares_round_trip_with_their_three_extra_bytes() {
        let sector = sample();
        let bytes = encode_sector(&sector, ClientDialect::KingdomReborn).unwrap();
        // 6-byte delimiters and 9-byte statics, as in EC.
        assert_eq!(bytes.len(), SECTOR_HEADER_SIZE + 4096 * KR_SQUARE_MIN_SIZE + 6 + 9);
        assert_eq!(decode_facet_bin_raw(&bytes).unwrap(), sector);
        assert!(decode_sector(&bytes, ClientDialect::Enhanced).is_err());
        assert!(encode_sector(&sector, ClientDialect::Enhanced).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_facet_bin_raw(&trailing).is_err());
    }

    #[test]
    fn classic_view_takes_the_stored_classic_land_and_keeps_every_static() {
        let bytes = encode_sector(&sample(), ClientDialect::KingdomReborn).unwrap();
        let decoded = decode_facet_bin(&bytes).unwrap();
        // Square (9, 10) is cell (1, 2) of block (1, 1).
        let block = &decoded.blocks[(BLOCKS_PER_SECTOR_SIDE + 1) as usize];
        let cell = block.block.cell(1, 2).unwrap();
        assert_eq!((cell.id, cell.z), (0x1A4, -3));
        assert_eq!(block.statics, vec![StaticTile { graphic_id: 6345, x: 1, y: 2, z: 12, hue: 8883 }]);
    }
}
