//! Builds an Enhanced Client facet package (`facetN.uop`) and its radar image (`facetNN.dds`) from a Classic
//! map and statics.
//!
//! The package follows the client rules in `facet_sector`: column-major sector numbering, the
//! `build/sectors/facet_%02d/%08d.bin` entries with the `u8 facet, u16 sector id` header, the `data.map` entry,
//! partial sectors padded with `z 0, land 0` squares, and the shipped delimiters. Classic land ids are written as
//! they are, and statics keep their order in `statics.mul`, graphic and hue widened to u32. The radar image
//! follows the recipe in `facet_radar`.
//!
//! The same builder makes Kingdom Reborn facets (`crate::kr::facet::facet_encoder`), with the KR square layout and
//! a Classic -> KR land mapping.

use std::path::Path;

use crate::classic::map::{MapBlock, MapBlockRelPos, MapCell, MapPlane};
use crate::classic::statics::{StaticTile as ClassicStaticTile, StaticsReader};
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_decoder::{DecodedEcFacet, EcFacetDelimiter, KrSquareLand, StaticTile};
use crate::enhanced::facet_package::EcFacetPackage;
use crate::enhanced::facet_radar::{radar_dds_file_name, FacetRadarDds, FacetRadarSource, RadarPalette};
use crate::enhanced::facet_sector::{
    delimiter_targets, empty_sector, facet_package_file_name, square_index, FacetGeometry, MAX_SQUARE_RECORDS,
    SECTOR_SIDE,
};

crate::eyre_imports!();

/// Classic map blocks along a sector side.
const BLOCKS_PER_SECTOR_SIDE: u32 = SECTOR_SIDE / MapBlock::CELLS_PER_ROW;

/// How a Classic land id is stored in a square: its land id and, in a Kingdom Reborn square, the KR bytes.
pub(crate) type SquareLandOf<'a> = &'a dyn Fn(u16) -> eyre::Result<(u16, Option<KrSquareLand>)>;

/// Encodes a Classic map plane and its statics into `facet{map_index}.uop` and `facet{map_index:02}.dds` in
/// `output_dir`, with the radar colours of `radarcol_path` (Classic `radarcol.mul`).
pub fn encode_map_plane(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    radarcol_path: &Path,
    output_dir: &Path,
    map_index: u8,
) -> eyre::Result<()> {
    let palette = RadarPalette::load_radarcol(radarcol_path)?;
    let (mut package, radar) = build_facet_from_classic(map_plane, statics, map_index, &palette)?;
    package.save(output_dir.join(facet_package_file_name(map_index, false)))?;
    radar.save(&output_dir.join(radar_dds_file_name(map_index)))
}

/// Builds the facet package (in memory) and the radar image of a Classic map plane and its statics. The facet
/// size is the map plane's.
pub fn build_facet_from_classic(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    facet: u8,
    palette: &RadarPalette,
) -> eyre::Result<(EcFacetPackage, FacetRadarDds)> {
    build_facet(map_plane, statics, facet, palette, ClientDialect::Enhanced, &|id| Ok((id, None)))
}

/// [`build_facet_from_classic`] in the square layout of `dialect`, storing each Classic land id as `land_of`
/// gives it (delimiters take the neighbour's stored land id).
pub(crate) fn build_facet(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    facet: u8,
    palette: &RadarPalette,
    dialect: ClientDialect,
    land_of: SquareLandOf,
) -> eyre::Result<(EcFacetPackage, FacetRadarDds)> {
    let size = map_plane.size_cells();
    let geometry = FacetGeometry::new(size.width, size.height);
    let mut package = EcFacetPackage::new_dialect(facet, geometry, dialect)?;
    let mut radar = FacetRadarSource::new(geometry);
    for sector in 0..geometry.sector_count() {
        let decoded = classic_sector(map_plane, statics, &geometry, facet, sector, dialect, land_of)?;
        radar.add_sector(sector, &decoded, palette);
        package.put_sector(&decoded)?;
    }
    let image = FacetRadarDds::encode_rgb8(geometry.width, geometry.height, &radar.rgb8())?;
    Ok((package, image))
}

/// Builds sector `sector` from the Classic map and statics.
fn classic_sector(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    geometry: &FacetGeometry,
    facet: u8,
    sector: u32,
    dialect: ClientDialect,
    land_of: SquareLandOf,
) -> eyre::Result<DecodedEcFacet> {
    let (origin_x, origin_y) = geometry.sector_origin(sector);
    let size_blocks = map_plane.size_blocks;
    let first_block_x = (origin_x / MapBlock::CELLS_PER_ROW).saturating_sub(1);
    let first_block_y = (origin_y / MapBlock::CELLS_PER_COLUMN).saturating_sub(1);
    // One extra ring of blocks for the delimiters, which read the neighbouring sectors' edge squares.
    let end_block_x = (origin_x / MapBlock::CELLS_PER_ROW + BLOCKS_PER_SECTOR_SIDE + 1).min(size_blocks.width);
    let end_block_y = (origin_y / MapBlock::CELLS_PER_COLUMN + BLOCKS_PER_SECTOR_SIDE + 1).min(size_blocks.height);
    let mut blocks: Vec<MapBlockRelPos> = (first_block_x..end_block_x)
        .flat_map(|x| (first_block_y..end_block_y).map(move |y| MapBlockRelPos { x, y }))
        .collect();
    map_plane.load_blocks(&mut blocks)?;

    let mut decoded = empty_sector(facet, sector, dialect)?;
    for block_x in 0..BLOCKS_PER_SECTOR_SIDE {
        for block_y in 0..BLOCKS_PER_SECTOR_SIDE {
            let map_block_x = origin_x / MapBlock::CELLS_PER_ROW + block_x;
            let map_block_y = origin_y / MapBlock::CELLS_PER_COLUMN + block_y;
            if map_block_x >= size_blocks.width || map_block_y >= size_blocks.height {
                continue;
            }
            let block_statics = statics.read_block(map_block_x, map_block_y)?;
            for cell_x in 0..MapBlock::CELLS_PER_ROW {
                for cell_y in 0..MapBlock::CELLS_PER_COLUMN {
                    let local_x = block_x * MapBlock::CELLS_PER_ROW + cell_x;
                    let local_y = block_y * MapBlock::CELLS_PER_COLUMN + cell_y;
                    let (x, y) = (origin_x + local_x, origin_y + local_y);
                    let cell = cell_at(map_plane, x, y)?;
                    let mut delimiters = Vec::new();
                    for (direction, nx, ny) in delimiter_targets(geometry, x, y) {
                        let neighbour = cell_at(map_plane, nx, ny)?;
                        let (land, _) = land_of(neighbour.id)?;
                        delimiters.push(EcFacetDelimiter { direction, z: neighbour.z, graphic_id: u32::from(land) });
                    }
                    let square_statics: Vec<StaticTile> = block_statics
                        .iter()
                        .filter(|item| u32::from(item.x_offset) == cell_x && u32::from(item.y_offset) == cell_y)
                        .map(|item: &ClassicStaticTile| StaticTile {
                            graphic_id: u32::from(item.graphic),
                            x: local_x as u8,
                            y: local_y as u8,
                            z: item.z,
                            hue: u32::from(item.hue),
                        })
                        .collect();
                    if square_statics.len() > MAX_SQUARE_RECORDS {
                        eyre::bail!(
                            "square ({x}, {y}) has {} statics, at most {MAX_SQUARE_RECORDS}",
                            square_statics.len()
                        );
                    }
                    let tile = &mut decoded.tiles[square_index(local_x, local_y)];
                    tile.z = cell.z;
                    (tile.land_graphic, tile.kr) = land_of(cell.id)?;
                    tile.delimiters = delimiters;
                    tile.statics = square_statics;
                }
            }
        }
    }
    Ok(decoded)
}

/// The map cell at `(x, y)`; its block must be loaded.
fn cell_at(map_plane: &mut MapPlane, x: u32, y: u32) -> eyre::Result<MapCell> {
    let block = map_plane
        .block(MapBlockRelPos { x: x / MapBlock::CELLS_PER_ROW, y: y / MapBlock::CELLS_PER_COLUMN })
        .ok_or_else(|| eyre!("map block of ({x}, {y}) is not loaded"))?;
    Ok(*block.cell(x % MapBlock::CELLS_PER_ROW, y % MapBlock::CELLS_PER_COLUMN)?)
}
