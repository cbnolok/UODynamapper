//! Converts an Enhanced Client or Kingdom Reborn facet package (`facetN.uop`) to a Classic facet held in memory
//! ([`ClassicWorld`]), the reverse of `facet_encoder` / `kr::facet::facet_encoder`, and lists what the Classic
//! files cannot hold ([`FacetClassicLosses`]).
//!
//! What a square stores and what the Classic files keep:
//!
//! - land id and z: kept. A KR square keeps its Classic id (`classic_land`); its material and `subtype` are not
//!   Classic data and come back from the shipped land map (`KrLandMap`) when a KR facet is built, so a square
//!   whose pair is not the land map's is a loss ([`FacetClassicLosses::kr_land_not_derived`]).
//! - statics: graphic, z and hue, in the square's order. The Classic record has a `u16` graphic and hue, so a
//!   larger value cannot be written ([`FacetClassicLosses::graphic_overflow`], `hue_overflow`; such statics are
//!   left out). A block's records are written square by square in (y, x) order, the order of the shipped files.
//! - delimiters: not Classic data; the builders derive them from the neighbours' land (the shipped rule), so a
//!   square whose delimiters differ from the rule is a loss ([`FacetClassicLosses::delimiters_off_rule`]).
//! - the padding squares of partial sectors (outside the facet): not kept; the builders write `z 0, land 0`.
//! - missing sectors: the client leaves a hole; the Classic files have none, so their cells are written as land 0
//!   at z 0 without statics ([`FacetClassicLosses::missing_sectors`]).
//!
//! The facet size comes from the package (`data.map`, else the client's size table: `EcFacetPackage`).

use crate::classic::map::{MapBlock, MapSizeCells};
use crate::classic::statics_write::StaticRecord;
use crate::classic::world::{ClassicWorld, LandFill};
use crate::enhanced::facet_decoder::EcFacetTile;
use crate::enhanced::facet_package::EcFacetPackage;
use crate::enhanced::facet_sector::{
    expected_delimiters, square_index, PADDING_KR_SQUARE_LAND, PADDING_LAND_ID, PADDING_LAND_Z, SECTOR_SIDE,
};
use crate::kr::facet::kr_land_map::KrLandMap;

crate::eyre_imports!();

/// Classic map blocks along a sector side.
const BLOCKS_PER_SECTOR_SIDE: u32 = SECTOR_SIDE / MapBlock::CELLS_PER_ROW;
/// The land of the cells of a missing sector.
const MISSING_SECTOR_FILL: LandFill = LandFill { land: PADDING_LAND_ID, z: PADDING_LAND_Z };
/// How many missing sectors [`FacetClassicLosses::describe`] lists.
const LISTED_MISSING_SECTORS: usize = 16;

/// What a facet package holds that the Classic files do not (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetClassicLosses {
    /// Sector ids the package lacks.
    pub missing_sectors: Vec<u32>,
    /// Statics with a graphic above `u16::MAX`, left out.
    pub graphic_overflow: u64,
    /// Statics with a hue above `u16::MAX`, left out.
    pub hue_overflow: u64,
    /// Squares whose delimiters are not the ones the shipped rule derives from their neighbours.
    pub delimiters_off_rule: u64,
    /// Padding squares (outside the facet) that are not `z 0, land 0` without delimiters or statics.
    pub padding_not_default: u64,
    /// Kingdom Reborn squares whose material and subtype are not the shipped land map's for their Classic id.
    pub kr_land_not_derived: u64,
}

impl FacetClassicLosses {
    pub fn is_lossless(&self) -> bool {
        *self == Self::default()
    }

    /// One line per loss.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.missing_sectors.is_empty() {
            let listed: Vec<String> =
                self.missing_sectors.iter().take(LISTED_MISSING_SECTORS).map(u32::to_string).collect();
            lines.push(format!(
                "{} sectors are missing (the client leaves holes; written as land 0 at z 0): {}{}",
                self.missing_sectors.len(),
                listed.join(", "),
                if self.missing_sectors.len() > LISTED_MISSING_SECTORS { ", ..." } else { "" }
            ));
        }
        if self.graphic_overflow > 0 {
            lines.push(format!("{} statics have a graphic above 0xFFFF and are left out", self.graphic_overflow));
        }
        if self.hue_overflow > 0 {
            lines.push(format!("{} statics have a hue above 0xFFFF and are left out", self.hue_overflow));
        }
        if self.delimiters_off_rule > 0 {
            lines.push(format!(
                "{} squares have delimiters other than the ones derived from their neighbours (rebuilt by the rule)",
                self.delimiters_off_rule
            ));
        }
        if self.padding_not_default > 0 {
            lines.push(format!(
                "{} padding squares outside the facet are not empty (written back empty)",
                self.padding_not_default
            ));
        }
        if self.kr_land_not_derived > 0 {
            lines.push(format!(
                "{} Kingdom Reborn squares have a material or subtype other than the shipped land map's for their \
                 Classic id (rebuilt from the land map)",
                self.kr_land_not_derived
            ));
        }
        lines
    }
}

/// A facet package as a Classic facet, with what did not convert.
#[derive(Debug, Clone)]
pub struct FacetToClassic {
    pub world: ClassicWorld,
    pub losses: FacetClassicLosses,
}

fn is_default_padding(tile: &EcFacetTile) -> bool {
    tile.z == PADDING_LAND_Z
        && tile.land_graphic == PADDING_LAND_ID
        && tile.kr.is_none_or(|kr| kr == PADDING_KR_SQUARE_LAND)
        && tile.delimiters.is_empty()
        && tile.statics.is_empty()
}

/// Converts `package` (Enhanced Client or Kingdom Reborn, as opened) to a Classic facet of the package's size.
/// `kr_land` is the land map a KR facet would be rebuilt with (the shipped one when `None`), which the KR squares
/// are checked against.
pub fn facet_to_classic(package: &EcFacetPackage, kr_land: Option<&KrLandMap>) -> eyre::Result<FacetToClassic> {
    let geometry = package.geometry();
    let size = MapSizeCells { width: geometry.width, height: geometry.height };
    let mut world = ClassicWorld::filled(size, MISSING_SECTOR_FILL)?;
    let mut losses = FacetClassicLosses::default();
    let shipped;
    let land_map = match (package.dialect().is_kingdom_reborn(), kr_land) {
        (false, _) => None,
        (true, Some(land)) => Some(land),
        (true, None) => {
            shipped = KrLandMap::shipped();
            Some(&shipped)
        }
    };
    // The land id each square stores (the KR material in a KR facet), which the delimiters hold.
    let mut stored_land = vec![PADDING_LAND_ID; geometry.width as usize * geometry.height as usize];
    let stored_index = |x: u32, y: u32| x as usize * geometry.height as usize + y as usize;
    let height_blocks = world.statics.size_blocks().height;

    for sector in 0..geometry.sector_count() {
        let Some(decoded) = package.read_sector_from_package(sector)? else {
            losses.missing_sectors.push(sector);
            continue;
        };
        let (origin_x, origin_y) = geometry.sector_origin(sector);
        for local_x in 0..SECTOR_SIDE {
            for local_y in 0..SECTOR_SIDE {
                let tile = &decoded.tiles[square_index(local_x, local_y)];
                let (x, y) = (origin_x + local_x, origin_y + local_y);
                if !geometry.contains(x, y) {
                    losses.padding_not_default += u64::from(!is_default_padding(tile));
                    continue;
                }
                let classic = tile.classic_land_id();
                world.map.set_cell(x, y, classic, tile.z)?;
                stored_land[stored_index(x, y)] = tile.land_graphic;
                if let Some(land_map) = land_map {
                    let derived = land_map.square_land(classic).ok();
                    losses.kr_land_not_derived += u64::from(derived != Some((tile.land_graphic, tile.kr)));
                }
            }
        }
        // Each block's statics, square by square in (y, x) order.
        for block_x in 0..BLOCKS_PER_SECTOR_SIDE {
            for block_y in 0..BLOCKS_PER_SECTOR_SIDE {
                let (map_block_x, map_block_y) = (
                    origin_x / MapBlock::CELLS_PER_ROW + block_x,
                    origin_y / MapBlock::CELLS_PER_COLUMN + block_y,
                );
                let first_x = map_block_x * MapBlock::CELLS_PER_ROW;
                let first_y = map_block_y * MapBlock::CELLS_PER_COLUMN;
                if !geometry.contains(first_x, first_y) {
                    continue;
                }
                let mut records = Vec::new();
                for cell_y in 0..MapBlock::CELLS_PER_COLUMN {
                    for cell_x in 0..MapBlock::CELLS_PER_ROW {
                        let local = (block_x * MapBlock::CELLS_PER_ROW + cell_x, block_y * MapBlock::CELLS_PER_COLUMN + cell_y);
                        for item in &decoded.tiles[square_index(local.0, local.1)].statics {
                            let graphic = u16::try_from(item.graphic_id);
                            let hue = u16::try_from(item.hue);
                            losses.graphic_overflow += u64::from(graphic.is_err());
                            losses.hue_overflow += u64::from(hue.is_err());
                            if let (Ok(graphic), Ok(hue)) = (graphic, hue) {
                                records.push(StaticRecord { graphic, x: cell_x as u8, y: cell_y as u8, z: item.z, hue });
                            }
                        }
                    }
                }
                world.statics.set_block((map_block_x * height_blocks + map_block_y) as usize, records)?;
            }
        }
    }

    // The delimiters, against the rule over the stored land of the neighbours.
    for sector in 0..geometry.sector_count() {
        let Some(decoded) = package.read_sector_from_package(sector)? else {
            continue;
        };
        let (origin_x, origin_y) = geometry.sector_origin(sector);
        for local_x in 0..SECTOR_SIDE {
            for local_y in 0..SECTOR_SIDE {
                let (x, y) = (origin_x + local_x, origin_y + local_y);
                if !geometry.contains(x, y) {
                    continue;
                }
                let tile = &decoded.tiles[square_index(local_x, local_y)];
                let expected = expected_delimiters(&geometry, x, y, |nx, ny| {
                    let (_, z) = world.map.cell(nx, ny).expect("a neighbour inside the facet");
                    (z, stored_land[stored_index(nx, ny)])
                });
                losses.delimiters_off_rule += u64::from(tile.delimiters != expected);
            }
        }
    }
    Ok(FacetToClassic { world, losses })
}
