//! The z a walker reaches stepping onto a cell, as the Classic client computes it before sending a step (ClassicUO
//! `Pathfinder.CalculateNewZ`, `CalculateMinMaxZ` and `CreateItemList`, for a living walker on foot or mounted; the
//! original client is not traced). The cell's land and its Surface, Bridge and Impassable statics are the objects;
//! mobiles, multis and dynamic items are not read.

use std::collections::HashMap;
use std::sync::Arc;
use udd_assets::tilemeta::TileMetaPackage;
use udd_container::UddpReader;
use uocf::classic::map::MapSizeCells;
use uocf::enhanced::tileart::TaeFlag;
use crate::render::terrain::MAP_STORAGE_BLOCK_TILE_DIM;
use crate::world::maps::{package_chunk_cell, read_package_chunk_land};
use crate::world::statics::LazyStaticsStore;

/// The height over which an object blocks the walker standing below it, and the gap a floor needs under the next one
/// (ClassicUO `DEFAULT_BLOCK_HEIGHT`).
const BLOCK_HEIGHT: i32 = 16;
/// How far above its cell's floor a walker steps up (ClassicUO `CalculateMinMaxZ`, `maxZ += 2`).
const STEP_UP_Z: i32 = 2;
/// The z of no floor, below every object.
const NO_Z: i32 = -128;
/// The z of the ceiling closing the list, above every object (ClassicUO's object at 128).
const CEILING_Z: i32 = 128;
/// No floor chosen yet: farther from the walker than any z.
const NO_DISTANCE: i32 = i32::MAX;
/// The land tiles the client leaves out of the walk (ClassicUO: the void and no-draw tiles 0x1AE ..= 0x1B5, beside
/// 2 and 0x1DB, which count).
const SKIPPED_LAND: std::ops::RangeInclusive<u16> = 0x01AE..=0x01B5;
const KEPT_LAND_BELOW: u16 = 2;
const KEPT_LAND_ABOVE: u16 = 0x01DB;
/// The cell a direction (0 north, clockwise) steps to.
const DIRECTION_OFFSETS: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

/// A cell's land as the walk reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkLand {
    pub tile: u16,
    /// The land tile's flags (`TaeFlag`).
    pub flags: u64,
    /// The z of the cell's corners: its own (top), then the cells east (right), south-east (bottom) and south (left).
    pub corners: [i32; 4],
    /// Drawn stretched over its corners: a land tile with a texture (ClassicUO `ApplyStretch`); flat at its own z
    /// otherwise.
    pub textured: bool,
}

impl WalkLand {
    fn stretched(&self) -> bool {
        self.textured && self.corners.iter().any(|&z| z != self.corners[0])
    }

    /// The land's lowest z and its average (`z_view::average_land_z`); its own z for a flat tile.
    fn min_and_average(&self) -> (i32, i32) {
        if !self.textured {
            return (self.corners[0], self.corners[0]);
        }
        let [top, right, bottom, left] = self.corners;
        let min = top.min(right).min(bottom).min(left);
        (min, crate::world::z_view::average_land_z(top, right, bottom, left))
    }

    /// The z at the cell's edge or corner a walker leaves by in `direction` (ClassicUO `CalculateCurrentAverageZ`).
    fn z_towards(&self, direction: u8) -> i32 {
        let corner = |side: u8| self.corners[usize::from(side & 3)];
        let ahead = corner((direction >> 1) + 1);
        if direction & 1 != 0 {
            return ahead;
        }
        (ahead + corner(direction >> 1)) >> 1
    }
}

/// A static as the walk reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkStatic {
    pub z: i32,
    pub height: i32,
    /// Its tile flags (`TaeFlag`).
    pub flags: u64,
}

/// The cells the walk reads.
pub trait WalkCells {
    /// The land of a cell; `None` off the map.
    fn land(&mut self, x: i32, y: i32) -> Option<WalkLand>;
    /// The statics of a cell, appended to `out`.
    fn statics(&mut self, x: i32, y: i32, out: &mut Vec<WalkStatic>);
}

/// An object a walker meets: something that blocks or holds it (`blocks`), and whether it is a floor (`surface`) or
/// a climb (`bridge`).
#[derive(Clone, Copy, Debug)]
struct PathObject {
    blocks: bool,
    surface: bool,
    bridge: bool,
    z: i32,
    average_z: i32,
    height: i32,
    /// Stretched land, read by its edge in `CalculateMinMaxZ`.
    land: Option<WalkLand>,
}

fn cell_objects(cells: &mut impl WalkCells, x: i32, y: i32, statics: &mut Vec<WalkStatic>) -> Vec<PathObject> {
    let mut objects = Vec::new();
    if let Some(land) = cells.land(x, y) {
        let kept = !SKIPPED_LAND.contains(&land.tile) || land.tile == KEPT_LAND_BELOW || land.tile == KEPT_LAND_ABOVE;
        if kept {
            let open = land.flags & TaeFlag::Impassable.bits() == 0;
            let (min_z, average_z) = land.min_and_average();
            objects.push(PathObject {
                blocks: true,
                surface: open,
                bridge: open,
                z: min_z,
                average_z,
                height: average_z - min_z,
                land: land.stretched().then_some(land),
            });
        }
    }
    statics.clear();
    cells.statics(x, y, statics);
    for item in statics.iter() {
        let impassable = item.flags & TaeFlag::Impassable.bits() != 0;
        let surface = item.flags & TaeFlag::Surface.bits() != 0;
        let bridge = item.flags & TaeFlag::Bridge.bits() != 0;
        if !(impassable || surface || bridge) {
            continue;
        }
        let stood_height = if bridge { item.height / 2 } else { item.height };
        objects.push(PathObject {
            blocks: impassable || surface,
            surface: surface && !impassable,
            bridge: bridge && !impassable,
            z: item.z,
            average_z: item.z + stood_height,
            height: item.height,
            land: None,
        });
    }
    objects
}

/// The floor range of the walker leaving its cell at `z` in `direction` towards (`x`, `y`): the lowest z it can be
/// placed at, and the highest top it steps onto (ClassicUO `CalculateMinMaxZ`).
fn min_max_z(cells: &mut impl WalkCells, x: i32, y: i32, z: i32, direction: u8, statics: &mut Vec<WalkStatic>)
    -> (i32, i32) {
    let (dx, dy) = DIRECTION_OFFSETS[usize::from((direction ^ 4) & 7)];
    let objects = cell_objects(cells, x + dx, y + dy, statics);
    let (mut min_z, mut max_z) = (NO_Z, z);
    if objects.is_empty() {
        return (min_z, max_z);
    }
    for object in &objects {
        match object.land {
            Some(land) if object.average_z <= z => {
                let edge_z = land.z_towards(direction & 7);
                min_z = min_z.max(edge_z);
                max_z = max_z.max(edge_z);
            }
            _ => {
                if object.blocks && object.average_z <= z && min_z < object.average_z {
                    min_z = object.average_z;
                }
                if object.bridge && z == object.average_z {
                    max_z = max_z.max(object.z + object.height);
                    min_z = min_z.min(object.z);
                }
            }
        }
    }
    (min_z, max_z + STEP_UP_Z)
}

/// The z of a walker at `z` stepping in `direction` (0 north, clockwise) onto the cell (`x`, `y`); `None` when the
/// step is blocked, as by a wall, or the cell has no floor within reach.
pub fn walk_z(cells: &mut impl WalkCells, x: i32, y: i32, z: i32, direction: u8) -> Option<i32> {
    let mut statics = Vec::new();
    let (mut min_z, max_z) = min_max_z(cells, x, y, z, direction, &mut statics);
    let mut objects = cell_objects(cells, x, y, &mut statics);
    if objects.is_empty() {
        return None;
    }
    objects.sort_by_key(|object| (object.z, object.height));
    objects.push(PathObject {
        blocks: true,
        surface: false,
        bridge: false,
        z: CEILING_Z,
        average_z: CEILING_Z,
        height: CEILING_Z,
        land: None,
    });
    let z = z.max(min_z);
    let (mut result, mut nearest, mut current_z) = (NO_Z, NO_DISTANCE, NO_Z);
    for (index, object) in objects.iter().enumerate() {
        if !object.blocks {
            continue;
        }
        if object.z - min_z >= BLOCK_HEIGHT {
            // A floor below this object, with room under it, that the walker reaches.
            for floor in objects[..index].iter().rev() {
                if !(floor.surface || floor.bridge) {
                    continue;
                }
                let reached = (floor.surface && floor.average_z <= max_z) || (floor.bridge && floor.z <= max_z);
                if floor.average_z >= current_z && object.z - floor.average_z >= BLOCK_HEIGHT && reached {
                    let distance = (z - floor.average_z).abs();
                    if distance < nearest {
                        nearest = distance;
                        result = floor.average_z;
                    }
                }
            }
        }
        min_z = min_z.max(object.average_z);
        current_z = current_z.max(object.average_z);
    }
    (result != NO_Z).then_some(result)
}

/// The cell `direction` steps to from (`x`, `y`).
pub fn step_cell(x: i32, y: i32, direction: u8) -> (i32, i32) {
    let (dx, dy) = DIRECTION_OFFSETS[usize::from(direction & 7)];
    (x + dx, y + dy)
}

/// The turns the client tries beside a diagonal: clockwise, then anticlockwise (ClassicUO `_dirOffset`).
const DIAGONAL_TURNS: [u8; 2] = [1, 7];

/// A step of a walker at (`x`, `y`, `z`) in `direction`, as the client takes it (ClassicUO `CanWalk`): the direction
/// taken, the cell and its z. A diagonal step passes only when both cells beside it do; otherwise the walker tries
/// the directions beside the diagonal. `None` when every try is blocked.
pub fn walk_step(cells: &mut impl WalkCells, x: i32, y: i32, z: i32, direction: u8) -> Option<(u8, i32, i32, i32)> {
    let direction = direction & 7;
    let attempt = |cells: &mut _, direction: u8| {
        let (to_x, to_y) = step_cell(x, y, direction);
        walk_z(cells, to_x, to_y, z, direction).map(|to_z| (direction, to_x, to_y, to_z))
    };
    let straight = attempt(cells, direction);
    if direction & 1 == 0 {
        return straight;
    }
    let beside = |turn: u8| (direction + turn) & 7;
    if straight.is_some() && DIAGONAL_TURNS.iter().all(|&turn| attempt(cells, beside(turn)).is_some()) {
        return straight;
    }
    DIAGONAL_TURNS.iter().find_map(|&turn| attempt(cells, beside(turn)))
}

/// A drawn body's step from (`x`, `y`, `z`) in `direction`: the cell and z of the step the client takes
/// (`walk_step`), or `None` where it blocks every try, so that the body turns and stays. A body the client lets out
/// in no direction, as one set inside a rock or into a river, steps as asked onto the floor nearest its z
/// (`nearest_floor_z`) instead, so that the viewer is never stuck.
pub fn body_step(cells: &mut impl WalkCells, x: i32, y: i32, z: i32, direction: u8) -> Option<(i32, i32, i32)> {
    if let Some((_, to_x, to_y, to_z)) = walk_step(cells, x, y, z, direction) {
        return Some((to_x, to_y, to_z));
    }
    let directions = 0..DIRECTION_OFFSETS.len() as u8;
    let boxed_in = directions.into_iter().all(|other| walk_step(cells, x, y, z, other).is_none());
    boxed_in.then(|| {
        let (to_x, to_y) = step_cell(x, y, direction);
        (to_x, to_y, nearest_floor_z(cells, to_x, to_y, z).unwrap_or(z))
    })
}

/// The floor of the cell (`x`, `y`) nearest to `z`: its land, even water or rock, or the top of a static a walker
/// stands on. For a walker set where the client would not step, as into a river; `None` when the cell has neither.
pub fn nearest_floor_z(cells: &mut impl WalkCells, x: i32, y: i32, z: i32) -> Option<i32> {
    let mut statics = Vec::new();
    let land = cells.land(x, y).map(|land| land.min_and_average().1);
    cell_objects(cells, x, y, &mut statics).iter()
        .filter(|object| object.surface || object.bridge)
        .map(|object| object.average_z)
        .chain(land)
        .min_by_key(|floor| (z - floor).abs())
}

/// The walk's cells of a map: its package's land and its statics, with the clients' tile data.
pub struct MapWalkCells<'a> {
    package: Arc<UddpReader>,
    size: MapSizeCells,
    chunks: HashMap<u32, Option<Vec<(u16, i8)>>>,
    statics: &'a mut LazyStaticsStore,
    tilemeta: &'a TileMetaPackage,
}

impl<'a> MapWalkCells<'a> {
    pub fn new(package: Arc<UddpReader>, size: MapSizeCells, statics: &'a mut LazyStaticsStore,
        tilemeta: &'a TileMetaPackage) -> Self {
        Self { package, size, chunks: HashMap::new(), statics, tilemeta }
    }

    fn land_cell(&mut self, x: i32, y: i32) -> Option<(u16, i32)> {
        let (chunk, cell) = package_chunk_cell(self.size.width, self.size.height, x, y)?;
        let package = &self.package;
        let cells = self.chunks.entry(chunk).or_insert_with(|| read_package_chunk_land(package, chunk).ok());
        cells.as_ref()?.get(cell).map(|&(tile, z)| (tile, i32::from(z)))
    }
}

impl WalkCells for MapWalkCells<'_> {
    fn land(&mut self, x: i32, y: i32) -> Option<WalkLand> {
        let (tile, z) = self.land_cell(x, y)?;
        // A corner off the map takes the cell's own z.
        let mut corner = |dx, dy| self.land_cell(x + dx, y + dy).map_or(z, |(_, z)| z);
        let corners = [z, corner(1, 0), corner(1, 1), corner(0, 1)];
        let meta = self.tilemeta.land_tile(u32::from(tile));
        Some(WalkLand {
            tile,
            flags: meta.map_or(0, |meta| meta.flags),
            corners,
            textured: meta.is_some_and(|meta| meta.texture_id != 0),
        })
    }

    fn statics(&mut self, x: i32, y: i32, out: &mut Vec<WalkStatic>) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else { return };
        let (block_x, block_y) = (x / MAP_STORAGE_BLOCK_TILE_DIM, y / MAP_STORAGE_BLOCK_TILE_DIM);
        let (cell_x, cell_y) = (x % MAP_STORAGE_BLOCK_TILE_DIM, y % MAP_STORAGE_BLOCK_TILE_DIM);
        let Ok(tiles) = self.statics.block_tiles(block_x, block_y) else { return };
        let tilemeta = self.tilemeta;
        out.extend(tiles.iter()
            .filter(|tile| u32::from(tile.x_offset()) == cell_x && u32::from(tile.y_offset()) == cell_y)
            .filter_map(|tile| tilemeta.item_tile(u32::from(tile.graphic)).map(|meta| WalkStatic {
                z: i32::from(tile.z),
                height: i32::from(meta.height),
                flags: meta.flags,
            })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRASS: u16 = 3;
    const EAST: u8 = 2;

    #[derive(Default)]
    struct Cells {
        land: HashMap<(i32, i32), i32>,
        statics: HashMap<(i32, i32), Vec<WalkStatic>>,
    }

    impl Cells {
        fn flat(z: i32) -> Self {
            let mut cells = Self::default();
            for y in -2..4 {
                for x in -2..4 {
                    cells.land.insert((x, y), z);
                }
            }
            cells
        }
    }

    impl WalkCells for Cells {
        fn land(&mut self, x: i32, y: i32) -> Option<WalkLand> {
            let z = |x, y| self.land.get(&(x, y)).copied();
            Some(WalkLand {
                tile: GRASS,
                flags: 0,
                corners: [z(x, y)?, z(x + 1, y)?, z(x + 1, y + 1)?, z(x, y + 1)?],
                textured: true,
            })
        }

        fn statics(&mut self, x: i32, y: i32, out: &mut Vec<WalkStatic>) {
            out.extend(self.statics.get(&(x, y)).into_iter().flatten().copied());
        }
    }

    fn floor(z: i32, height: i32) -> WalkStatic {
        WalkStatic { z, height, flags: TaeFlag::Surface.bits() }
    }

    #[test]
    fn a_walker_follows_flat_land() {
        assert_eq!(walk_z(&mut Cells::flat(5), 1, 0, 5, EAST), Some(5));
    }

    #[test]
    fn a_walker_climbs_a_slope_and_steps_onto_a_low_floor() {
        let mut cells = Cells::flat(0);
        for y in -2..4 {
            cells.land.insert((2, y), 4);
            cells.land.insert((3, y), 8);
        }
        // The cell (1, 0) rises from 0 to 4 eastwards: the walker reaches its middle, 2.
        assert_eq!(walk_z(&mut cells, 1, 0, 0, EAST), Some(2));
        let mut flat = Cells::flat(0);
        flat.statics.insert((1, 0), vec![floor(0, 2)]);
        assert_eq!(walk_z(&mut flat, 1, 0, 0, EAST), Some(2));
    }

    #[test]
    fn a_wall_blocks_and_a_high_floor_is_out_of_reach() {
        let mut walled = Cells::flat(0);
        walled.statics.insert((1, 0), vec![WalkStatic { z: 0, height: 20, flags: TaeFlag::Impassable.bits() }]);
        assert_eq!(walk_z(&mut walled, 1, 0, 0, EAST), None);
        let mut upstairs = Cells::flat(0);
        upstairs.statics.insert((1, 0), vec![floor(0, 20)]);
        assert_eq!(walk_z(&mut upstairs, 1, 0, 0, EAST), None);
    }

    #[test]
    fn a_bridge_lets_the_walker_climb_half_its_height_per_step() {
        let mut cells = Cells::flat(0);
        let step = |z| WalkStatic { z, height: 10, flags: (TaeFlag::Surface | TaeFlag::Bridge).bits() };
        cells.statics.insert((1, 0), vec![step(0)]);
        cells.statics.insert((2, 0), vec![step(5)]);
        let first = walk_z(&mut cells, 1, 0, 0, EAST);
        assert_eq!(first, Some(5));
        assert_eq!(walk_z(&mut cells, 2, 0, first.unwrap(), EAST), Some(10));
    }

    #[test]
    fn a_diagonal_step_does_not_cut_a_corner() {
        const SOUTH_EAST: u8 = 3;
        const SOUTH: u8 = 4;
        let mut cells = Cells::flat(0);
        assert_eq!(walk_step(&mut cells, 0, 0, 0, SOUTH_EAST), Some((SOUTH_EAST, 1, 1, 0)));
        // A wall east: the walker goes south instead.
        cells.statics.insert((1, 0), vec![WalkStatic { z: 0, height: 20, flags: TaeFlag::Impassable.bits() }]);
        assert_eq!(walk_step(&mut cells, 0, 0, 0, SOUTH_EAST), Some((SOUTH, 0, 1, 0)));
    }

    #[test]
    fn a_walker_set_through_a_wall_lands_on_the_nearest_floor() {
        let mut cells = Cells::flat(0);
        cells.statics.insert((1, 0), vec![floor(0, 20), floor(40, 2)]);
        assert_eq!(nearest_floor_z(&mut cells, 1, 0, 3), Some(0));
        assert_eq!(nearest_floor_z(&mut cells, 1, 0, 30), Some(20));
        assert_eq!(nearest_floor_z(&mut cells, 1, 0, 45), Some(42));
    }

    #[test]
    fn a_walker_set_into_water_under_a_bridge_lands_in_the_water() {
        struct River;
        impl WalkCells for River {
            fn land(&mut self, _: i32, _: i32) -> Option<WalkLand> {
                Some(WalkLand { tile: GRASS, flags: TaeFlag::Impassable.bits(), corners: [-5; 4], textured: true })
            }
            fn statics(&mut self, _: i32, _: i32, out: &mut Vec<WalkStatic>) {
                out.push(WalkStatic { z: 18, height: 2, flags: TaeFlag::Surface.bits() });
            }
        }
        assert_eq!(walk_z(&mut River, 1, 0, -1, EAST), None);
        assert_eq!(nearest_floor_z(&mut River, 1, 0, -1), Some(-5));
    }

    #[test]
    fn a_body_stops_at_an_impassable_planter_and_walks_on_beside_it() {
        // Britain's planter (1480..=1481, 1722..=1724, map 1): flower beds 0x4B22.. at z 0, 5 high, Impassable.
        let bed = WalkStatic { z: 0, height: 5, flags: TaeFlag::Impassable.bits() };
        let mut cells = Cells::flat(0);
        cells.statics.insert((1, 0), vec![bed]);
        assert_eq!(body_step(&mut cells, 0, 0, 0, EAST), None);
        assert_eq!(body_step(&mut cells, 0, 1, 0, EAST), Some((1, 1, 0)));
        // A body set inside it walks out.
        assert_eq!(body_step(&mut cells, 1, 0, 0, EAST), Some((2, 0, 0)));
    }

    #[test]
    fn a_body_stops_at_the_statue_and_climbs_its_pedestal() {
        // Britain's black statue (1480..=1481, 1713..=1714, map 1) on stone pedestals 0x0750: z 0, 10 high, Surface
        // and Bridge. The statue's pieces 0x139E.. stand on them at z 6, Impassable.
        let pedestal = WalkStatic { z: 0, height: 10, flags: (TaeFlag::Surface | TaeFlag::Bridge).bits() };
        let statue = WalkStatic { z: 6, height: 15, flags: TaeFlag::Impassable.bits() };
        let mut cells = Cells::flat(0);
        cells.statics.insert((1, 0), vec![pedestal]);
        cells.statics.insert((2, 0), vec![pedestal, statue]);
        assert_eq!(body_step(&mut cells, 0, 0, 0, EAST), Some((1, 0, 5)), "onto the pedestal's half height");
        assert_eq!(body_step(&mut cells, 1, 0, 5, EAST), None, "into the statue");
    }

    #[test]
    fn a_body_boxed_in_steps_as_asked_onto_the_nearest_floor() {
        let wall = WalkStatic { z: 0, height: 20, flags: TaeFlag::Impassable.bits() };
        let mut cells = Cells::flat(0);
        for y in -1..=1 {
            for x in -1..=1 {
                if (x, y) != (0, 0) {
                    cells.statics.insert((x, y), vec![wall]);
                }
            }
        }
        assert_eq!(body_step(&mut cells, 0, 0, 0, EAST), Some((1, 0, 0)));
    }

    #[test]
    fn a_walker_keeps_to_its_floor_of_a_building() {
        let mut cells = Cells::flat(0);
        cells.statics.insert((1, 0), vec![floor(20, 1)]);
        assert_eq!(walk_z(&mut cells, 1, 0, 0, EAST), Some(0), "under the upper floor");
        assert_eq!(walk_z(&mut cells, 1, 0, 21, EAST), Some(21), "on it");
    }
}
