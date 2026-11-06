//! A Classic facet held whole in memory: its map ([`ClassicMapBlocks`]) and statics ([`ClassicStaticsBlocks`]),
//! the common form every facet conversion goes through (Classic MUL or UOP, Enhanced Client and Kingdom Reborn
//! facet packages; see `crate::enhanced::facet_to_classic` and `crate::enhanced::facet_encoder`).
//!
//! - **Resizing** ([`ClassicWorld::resized`]) keeps the cells and statics at the same coordinates. Growing fills the
//!   new cells with [`VOID_LAND_ID`] at [`VOID_LAND_Z`], the land the shipped clients have where a map holds
//!   nothing: Mondain's Legacy widened Felucca from UO Gold's 6144 to 7168 cells, and 94% of the new strip is
//!   0x244 at z 0, as are the unused parts inside UO Gold's map. Shrinking refuses when content (land other than the
//!   fill, or any static) lies outside the new size, unless cropping is allowed; the report says what was dropped
//!   and where.
//! - **Tile ids** ([`ClassicWorld::tile_ids_beyond`]): statics whose graphic is past a client's `tiledata.mul`
//!   item count (16,384 before 7.0, 32,768 or 65,536 later) are not drawn by that client.

use crate::classic::facet_sources::{ClassicFacetSources, ClassicMapFormat};
use crate::classic::map::{MapBlock, MapSizeCells};
use crate::classic::map_write::{ClassicMapBlocks, DifBake};
use crate::classic::statics_write::{ClassicStaticsBlocks, StaticRecord};

crate::eyre_imports!();

/// The land id the shipped Classic maps hold where they have nothing (the black "void" tile).
pub const VOID_LAND_ID: u16 = 0x244;
/// The z of [`VOID_LAND_ID`] in the empty parts of the shipped maps.
pub const VOID_LAND_Z: i8 = 0;
/// How many dropped cells and statics a [`ResizeReport`] lists by position.
pub const RESIZE_REPORT_EXAMPLES: usize = 8;

/// The land and z a resize fills new cells with, and treats as empty when cropping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandFill {
    pub land: u16,
    pub z: i8,
}

impl LandFill {
    /// [`VOID_LAND_ID`] at [`VOID_LAND_Z`].
    pub const VOID: Self = Self { land: VOID_LAND_ID, z: VOID_LAND_Z };
}

impl Default for LandFill {
    fn default() -> Self {
        Self::VOID
    }
}

/// An inclusive rectangle of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl CellRect {
    fn point(x: u32, y: u32) -> Self {
        Self { x0: x, y0: y, x1: x, y1: y }
    }

    fn include(&mut self, x: u32, y: u32) {
        (self.x0, self.y0, self.x1, self.y1) = (self.x0.min(x), self.y0.min(y), self.x1.max(x), self.y1.max(y));
    }
}

/// What [`ClassicWorld::resized`] found past the new size and added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResizeReport {
    /// Cells past the new size holding land other than the fill.
    pub dropped_land_cells: u64,
    /// Statics past the new size.
    pub dropped_statics: u64,
    /// The cells of both, when there are any.
    pub dropped_area: Option<CellRect>,
    /// The first dropped land cells, as (x, y, land, z).
    pub land_examples: Vec<(u32, u32, u16, i8)>,
    /// The first dropped statics, as (x, y, static).
    pub static_examples: Vec<(u32, u32, StaticRecord)>,
    /// Cells added with the fill.
    pub filled_cells: u64,
}

impl ResizeReport {
    /// Whether any content lies past the new size.
    pub fn drops_content(&self) -> bool {
        self.dropped_land_cells > 0 || self.dropped_statics > 0
    }

    /// One line per finding, for a refusal or a crop report.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(area) = self.dropped_area {
            lines.push(format!(
                "{} cells of land other than the fill and {} statics lie outside the new size, in cells \
                 ({}, {})..=({}, {})",
                self.dropped_land_cells, self.dropped_statics, area.x0, area.y0, area.x1, area.y1
            ));
        }
        for (x, y, land, z) in &self.land_examples {
            lines.push(format!("  land 0x{land:04x} z {z} at ({x}, {y})"));
        }
        for (x, y, record) in &self.static_examples {
            lines.push(format!("  static 0x{:04x} z {} hue {} at ({x}, {y})", record.graphic, record.z, record.hue));
        }
        if self.filled_cells > 0 {
            lines.push(format!("{} new cells filled", self.filled_cells));
        }
        lines
    }
}

/// Statics past a client's `tiledata.mul` ([`ClassicWorld::tile_ids_beyond`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TileIdReport {
    /// Statics whose graphic is at or past the item count.
    pub statics: u64,
    /// The largest such graphic.
    pub max_graphic: Option<u16>,
    /// Cells whose land id is at or past the land count.
    pub land_cells: u64,
    pub max_land: Option<u16>,
}

impl TileIdReport {
    pub fn is_empty(&self) -> bool {
        self.statics == 0 && self.land_cells == 0
    }
}

/// A Classic facet: map and statics of the same size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicWorld {
    pub map: ClassicMapBlocks,
    pub statics: ClassicStaticsBlocks,
}

impl ClassicWorld {
    /// Loads the facet `sources` names: its map (MUL or UOP) and statics, sized as the map file says unless `size`
    /// overrides it, with the difs `sources` lists baked in (every id in file order, a later one winning, as the
    /// client applies them). Returns the bake reports, map then statics.
    pub fn load(
        sources: &ClassicFacetSources,
        size: Option<MapSizeCells>,
    ) -> eyre::Result<(Self, Option<DifBake>, Option<DifBake>)> {
        let map_file = &sources.map;
        let mut map = match map_file.format {
            ClassicMapFormat::Mul => ClassicMapBlocks::read_mul(&map_file.path, map_file.map_index, size)?,
            ClassicMapFormat::Uop => ClassicMapBlocks::read_uop(&map_file.path, map_file.map_index, size)?,
        };
        let files = sources.statics_files()?;
        let mut statics = ClassicStaticsBlocks::read_mul(&files.index, &files.statics, map.size_cells())?;
        let read = |path: &std::path::Path| std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()));
        let map_bake = match &sources.map_difs {
            Some(difs) => Some(map.bake_map_difs(&read(&difs.lookup)?, &read(&difs.blocks)?)?),
            None => None,
        };
        let static_bake = match &sources.static_difs {
            Some(difs) => {
                Some(statics.bake_static_difs(&read(&difs.lookup)?, &read(&difs.index)?, &read(&difs.statics)?)?)
            }
            None => None,
        };
        Ok((Self::new(map, statics)?, map_bake, static_bake))
    }

    /// A facet of `size` cells: every cell `fill`, no statics.
    pub fn filled(size: MapSizeCells, fill: LandFill) -> eyre::Result<Self> {
        let mut map = ClassicMapBlocks::new(size)?;
        if fill != (LandFill { land: 0, z: 0 }) {
            for x in 0..size.width {
                for y in 0..size.height {
                    map.set_cell(x, y, fill.land, fill.z)?;
                }
            }
        }
        Ok(Self { map, statics: ClassicStaticsBlocks::new(size)? })
    }

    /// Pairs a map and its statics; fails when their sizes differ.
    pub fn new(map: ClassicMapBlocks, statics: ClassicStaticsBlocks) -> eyre::Result<Self> {
        let (a, b) = (map.size_cells(), statics.size_cells());
        if (a.width, a.height) != (b.width, b.height) {
            eyre::bail!("the map is {}x{}, its statics {}x{}", a.width, a.height, b.width, b.height);
        }
        Ok(Self { map, statics })
    }

    pub fn size_cells(&self) -> MapSizeCells {
        self.map.size_cells()
    }

    /// This facet at `size`, the cells and statics at the same coordinates. New cells get `fill`. Content past
    /// `size` (land other than `fill`, any static) fails the resize unless `allow_crop`; the report lists it.
    pub fn resized(&self, size: MapSizeCells, fill: LandFill, allow_crop: bool) -> eyre::Result<(Self, ResizeReport)> {
        let old = self.size_cells();
        let mut report = ResizeReport::default();
        let mut note = |report: &mut ResizeReport, x: u32, y: u32| match &mut report.dropped_area {
            Some(area) => area.include(x, y),
            None => report.dropped_area = Some(CellRect::point(x, y)),
        };
        for x in 0..old.width {
            for y in 0..old.height {
                if x < size.width && y < size.height {
                    continue;
                }
                let (land, z) = self.map.cell(x, y)?;
                if (land, z) != (fill.land, fill.z) {
                    report.dropped_land_cells += 1;
                    if report.land_examples.len() < RESIZE_REPORT_EXAMPLES {
                        report.land_examples.push((x, y, land, z));
                    }
                    note(&mut report, x, y);
                }
            }
        }
        let old_height_blocks = self.map.size_blocks().height;
        for block in 0..self.statics.block_count() {
            let (block_x, block_y) = (block as u32 / old_height_blocks, block as u32 % old_height_blocks);
            for record in self.statics.block(block) {
                let x = block_x * MapBlock::CELLS_PER_ROW + u32::from(record.x);
                let y = block_y * MapBlock::CELLS_PER_COLUMN + u32::from(record.y);
                if x < size.width && y < size.height {
                    continue;
                }
                report.dropped_statics += 1;
                if report.static_examples.len() < RESIZE_REPORT_EXAMPLES {
                    report.static_examples.push((x, y, *record));
                }
                note(&mut report, x, y);
            }
        }
        if report.drops_content() && !allow_crop {
            eyre::bail!(
                "resizing {}x{} to {}x{} drops content (allow cropping to drop it):\n{}",
                old.width,
                old.height,
                size.width,
                size.height,
                report.describe().join("\n")
            );
        }

        let mut resized = Self::filled(size, fill)?;
        for x in 0..old.width.min(size.width) {
            for y in 0..old.height.min(size.height) {
                let (land, z) = self.map.cell(x, y)?;
                resized.map.set_cell(x, y, land, z)?;
            }
        }
        let new_height_blocks = resized.map.size_blocks().height;
        let kept_width_blocks = old.width.min(size.width).div_ceil(MapBlock::CELLS_PER_ROW);
        let kept_height_blocks = old.height.min(size.height).div_ceil(MapBlock::CELLS_PER_COLUMN);
        for block_x in 0..kept_width_blocks {
            for block_y in 0..kept_height_blocks {
                let records: Vec<StaticRecord> = self
                    .statics
                    .block((block_x * old_height_blocks + block_y) as usize)
                    .iter()
                    .filter(|record| {
                        block_x * MapBlock::CELLS_PER_ROW + u32::from(record.x) < size.width
                            && block_y * MapBlock::CELLS_PER_COLUMN + u32::from(record.y) < size.height
                    })
                    .copied()
                    .collect();
                resized.statics.set_block((block_x * new_height_blocks + block_y) as usize, records)?;
            }
        }
        report.filled_cells = u64::from(size.width) * u64::from(size.height)
            - u64::from(old.width.min(size.width)) * u64::from(old.height.min(size.height));
        Ok((resized, report))
    }

    /// The statics at or past `item_count` and the land cells at or past `land_count` (a target client's
    /// `tiledata.mul` sizes), which that client does not have.
    pub fn tile_ids_beyond(&self, land_count: usize, item_count: usize) -> TileIdReport {
        let mut report = TileIdReport::default();
        for block in 0..self.map.block_count() {
            for &(land, _) in self.map.block_cells(block) {
                if usize::from(land) >= land_count {
                    report.land_cells += 1;
                    report.max_land = report.max_land.max(Some(land));
                }
            }
        }
        for block in 0..self.statics.block_count() {
            for record in self.statics.block(block) {
                if usize::from(record.graphic) >= item_count {
                    report.statics += 1;
                    report.max_graphic = report.max_graphic.max(Some(record.graphic));
                }
            }
        }
        report
    }

    /// The blocks whose statics are not in (y, x) order: a facet package keeps each square's statics but not the
    /// order between squares, so these come back sorted.
    pub fn unsorted_static_blocks(&self) -> usize {
        (0..self.statics.block_count())
            .filter(|&block| {
                self.statics.block(block).windows(2).any(|pair| (pair[0].y, pair[0].x) > (pair[1].y, pair[1].x))
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: MapSizeCells = MapSizeCells { width: 16, height: 16 };
    const WIDE: MapSizeCells = MapSizeCells { width: 32, height: 16 };

    fn world_with_content_at(x: u32, y: u32) -> ClassicWorld {
        let mut world = ClassicWorld::filled(WIDE, LandFill::VOID).unwrap();
        world.map.set_cell(1, 1, 3, 0).unwrap();
        world.statics.add_static(2, 2, 0x100, 0, 0).unwrap();
        world.map.set_cell(x, y, 4, 5).unwrap();
        world.statics.add_static(x, y, 0x200, 1, 0).unwrap();
        world
    }

    #[test]
    fn growing_fills_and_shrinking_back_restores() {
        let mut small = ClassicWorld::filled(SMALL, LandFill::VOID).unwrap();
        small.map.set_cell(15, 15, 3, -2).unwrap();
        small.statics.add_static(15, 15, 0x100, 0, 0).unwrap();
        let (grown, report) = small.resized(WIDE, LandFill::VOID, false).unwrap();
        assert_eq!(report.filled_cells, 16 * 16);
        assert_eq!(grown.map.cell(20, 3).unwrap(), (VOID_LAND_ID, VOID_LAND_Z));
        assert_eq!(grown.statics.cell_statics(15, 15).unwrap().len(), 1);
        let (back, report) = grown.resized(SMALL, LandFill::VOID, false).unwrap();
        assert!(!report.drops_content());
        assert_eq!(back, small);
    }

    #[test]
    fn shrinking_refuses_content_outside_unless_cropping() {
        let world = world_with_content_at(20, 5);
        let error = world.resized(SMALL, LandFill::VOID, false).unwrap_err().to_string();
        assert!(error.contains("(20, 5)"), "{error}");
        let (cropped, report) = world.resized(SMALL, LandFill::VOID, true).unwrap();
        assert_eq!((report.dropped_land_cells, report.dropped_statics), (1, 1));
        assert_eq!(report.dropped_area, Some(CellRect { x0: 20, y0: 5, x1: 20, y1: 5 }));
        assert_eq!(cropped.map.cell(1, 1).unwrap(), (3, 0));
        assert_eq!(cropped.statics.static_count(), 1);
    }

    #[test]
    fn tile_ids_past_a_tiledata_are_counted() {
        let world = world_with_content_at(20, 5);
        let report = world.tile_ids_beyond(0x4000, 0x180);
        assert_eq!((report.statics, report.max_graphic), (1, Some(0x200)));
        assert_eq!(report.land_cells, 0);
    }
}
