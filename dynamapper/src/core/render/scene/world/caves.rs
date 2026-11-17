//! Caves drawn over the map ("Caves: Drawn Over the Map", `world_rendering.caves`): the cave floors buried under the
//! land show through it, as the old MapRenderer painted them, while only the z-view (`z_view.rs`) shows them by default
//! and only with the player inside.
//!
//! The rule, found on map 0 (`CAVE_MIN_BURY_DEPTH`, `WALK_STEP_Z`):
//! - a cell is a candidate when its highest floor static top (`surface_top`: Surface, neither a roof nor impassable;
//!   z plus its height, half of it for a Bridge) is at least `CAVE_MIN_BURY_DEPTH` below the cell's own land z;
//! - the candidates flood into components through their eight neighbours, two floors at most `WALK_STEP_Z` apart;
//! - a component shows when one of its cells has a neighbour, not a candidate, whose walkable ground (its land, or a
//!   floor static top that is not buried) is at most `WALK_STEP_Z` from the cell's floor: a cave mouth.
//!
//! Basements stay hidden: their floors are reached by stairs, whose steps are more than a walking step apart, and a
//! sealed buried floor has no mouth at all. On map 0 the rule shows 12781 cells, among them the Minoc mine (470) and the
//! caves north of Britain (899), and keeps the Britain basements and the Buccaneer's Den tunnels hidden: the tunnels reach the open
//! only through house basements by stairs.
//!
//! Drawn over the map: the land of a shown cell is lowered to the cave floor (`CaveMap::floor_z`), so the land over
//! the cave opens into a pit whose sides are the land around it, its statics at or above the cave ceiling (floor +
//! `CAVE_CEILING_HEADROOM`, the z-view's cut for a player standing there) are not drawn, and its statics below the
//! ceiling, with the buried statics of the cells around it (the cave walls), are drawn in front of the land and
//! statics around the cave (`CaveStatic::Lifted`, art/depth.wesl `cave_lifted_depth`): in the slanted view the land in
//! front of a cave rises over it. A light in a shown cell is covered as the z-view covers it for a player there
//! (`CavesRes::light_view`).
//!
//! The player's cave (`PlayerCave`): with "Show Only the Player's Floor" on and the player on a buried floor, the cave
//! is drawn whole in every style, in place of both z-view rules' cave cut and of the caves drawn over the map:
//! - the cave is the buried floors reached from the player's floor through the eight neighbours, a step at most
//!   `WALK_STEP_Z` apart, each within `CAVE_LEVEL_HEIGHT` of the player's floor (one level), as far as
//!   `PLAYER_CAVE_REACH_CHUNKS` package chunks around the player's;
//! - on a cave cell the statics within `CAVE_LEVEL_HEIGHT` of its floors are drawn, its floor and what stands on it;
//!   the hill over it and the levels above and below are not;
//! - around the cave, up to `CAVE_WALL_REACH` cells away, the statics buried under their cell's land (`CAVE_MIN_BURY_DEPTH`,
//!   not the rocks and logs lying a little under it outside, nor water) are its rock walls, drawn
//!   up to the land over them with no cut, down to `CAVE_WALL_DROP_PER_CELL` per cell away under its floor, the walls
//!   lower the farther back as the slanted view stacks them; the buried floors there are other tunnels, not drawn, nor
//!   what stands under the cave's floor on them or next to them, a level up from them at most (a tunnel below's walls);
//! - the cave is drawn in front of what is outside it (`CaveStatic::Lifted`), and outside it, as
//!   `world_rendering.cave_surroundings` says (`CaveSurroundings`), the land and the statics not buried are drawn under
//!   ClassicUO's cut and darkened, or nothing is, as the KR client shows a cave; buried statics outside it are not drawn;
//! - a light shows when its static is drawn, darkened with it outside the cave (`CavesRes::light_view`);
//! - the cave's mouth is its own: where its floor walks out onto the land of a cell next to it, at most `WALK_STEP_Z`
//!   from the floor (an exit), the land rising from there over the floor is the rock framing the opening, drawn whole,
//!   in front of the cave's floor behind it and not darkened (`PlayerCave::owns_land`: the land quads with a corner on
//!   a buried cell of the cave sharing a side with an exit; a cave exit frame there never blends, tile_atlas.rs
//!   `cell_terrain_flags`), the floor under that land not drawn,
//!   and what stands on an exit at the cave's level, the cave entrance art of the KR client's arches, is the cave's
//!   (`CaveStatic::Lifted`). Its black opening, the black land and the black of that frame (all of an Enhanced or
//!   Kingdom Reborn frame, the opening painted in a Classic texmap's), is see-through: what lies behind it, the cave's
//!   floor or the land outside, shows darkened through it (land/cave_mouth_veil.rs).
//!   Where a built ceiling covers the rim, a floor static on the land's face over the cave's floor, the opening is a
//!   room's doorway, not a mouth: its land is not the cave's.

use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::maps::{package_chunk_cell, read_package_chunk_land_z, PACKAGE_CHUNK_CELL_DIM};
use crate::core::render::scene::world::land::draw_mesh::PendingTextureBake;
use crate::core::render::scene::world::land::{LCMesh, MAP_STORAGE_BLOCK_TILE_DIM};
use crate::core::render::scene::player::Player;
use crate::core::render::scene::world::art::light_cover::{CLIENT_BAND_HEIGHT, NO_Z_VIEW_LIMIT};
use crate::core::render::scene::world::z_view::{PLAYER_CEILING_MIN_RISE, PLAYER_VIEW_HEADROOM};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::{read_package_chunk_statics, StaticsStoreRes};
use crate::core::uo_files_loader::{MapPlanesRes, TileMetaPackageRes};
use bevy::prelude::*;
use color_eyre::eyre;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;
use udd_assets::tilemeta::TileMetaPackage;
use udd_container::UddpReader;
use uocf::enhanced::tileart::TaeFlag;

/// How the caves buried under the land show. Kept in the app-written world rendering settings as
/// `world_rendering.caves`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaveView {
    /// Under the land, as the client draws them: the z-view shows a cave with the player inside.
    #[default]
    HiddenUntilEntered,
    /// Through the land, the caves connected to a mouth (module docs).
    DrawnOverMap,
}

impl CaveView {
    pub const ALL: [Self; 2] = [Self::HiddenUntilEntered, Self::DrawnOverMap];

    pub fn label(self) -> &'static str {
        match self {
            Self::HiddenUntilEntered => "Hidden Until Entered",
            Self::DrawnOverMap => "Drawn Over the Map",
        }
    }
}

/// What shows around the player's cave (`PlayerCave`). Kept in the app-written world rendering settings as
/// `world_rendering.cave_surroundings`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaveSurroundings {
    /// The land and the statics outside the cave under ClassicUO's cut (the player's z + `PLAYER_VIEW_HEADROOM`),
    /// darkened and partly desaturated (art/depth.wesl `cave_outside_tone`), their lights with them.
    #[default]
    Dimmed,
    /// Nothing outside the cave, as the KR client shows a cave.
    Black,
}

impl CaveSurroundings {
    pub const ALL: [Self; 2] = [Self::Dimmed, Self::Black];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dimmed => "Dimmed",
            Self::Black => "Black",
        }
    }
}

/// How far land must rise over a floor to bury it, in z units: the z-view's cave test, land `PLAYER_VIEW_HEADROOM`
/// over the player.
pub const CAVE_MIN_BURY_DEPTH: i32 = PLAYER_VIEW_HEADROOM;
/// How far in z a walker steps from a cell to the next: the servers' and ClassicUO's `StepHeight`. Dynamapper's player
/// moves free, so no movement constant holds it. On map 0 the house stairs into the Buccaneer's Den tunnels are 5 to
/// 7 apart, so a step of 5 would show the basements; 2 shows every mountain cave found.
pub const WALK_STEP_Z: i32 = 2;
/// The cave ceiling over a shown floor: the z-view's cap for a player under a mountain (`PLAYER_VIEW_HEADROOM`).
pub const CAVE_CEILING_HEADROOM: i32 = PLAYER_VIEW_HEADROOM;
/// One level of the player's cave, in z units: its floors lie within this of the player's floor, and on a cave cell
/// the statics within this of its floors are the cave's. The KR client's 20-z height layer (`CLIENT_BAND_HEIGHT`).
/// The Minoc mine's upper gallery (map 1, 2586,457) is 60 over the main one, the crossing tunnels of the crypt
/// under the meadow at 1008,800 are 40 apart.
pub const CAVE_LEVEL_HEIGHT: i32 = CLIENT_BAND_HEIGHT;
/// How far from the player's cave, in cells, its rock walls stand. On the Minoc mine they reach 7.
pub const CAVE_WALL_REACH: i32 = 8;
/// How far under the cave's floor a rock wall may stand, in z units for each cell it stands away from the cave: a
/// cell's 44 screen pixels at 4 pixels a z unit. A cave's rock face is a stack of walls, each lower one farther back,
/// which the slanted view draws over the same place: on the Minoc mine the walls at z 0 stand next to the floor at 0,
/// those at -20 one or two cells back, at -40 two or three, at -60 five to seven.
pub const CAVE_WALL_DROP_PER_CELL: i32 = 11;
/// The cells added to a wall's distance from the cave for its depth (`CAVE_WALL_DROP_PER_CELL`): the walls at -40 two
/// cells back stay.
const CAVE_WALL_DROP_SLACK_CELLS: i32 = 2;
/// How far the player's cave is found around the player's package chunk, in chunks each way: the chunks read and the
/// bound of the flood, held to the chunk grid so the cave found stays while the player walks in a chunk. Seven chunks,
/// 224 cells, across: the view at the farthest zoom.
pub const PLAYER_CAVE_REACH_CHUNKS: i32 = 3;
/// `CavesRes::land_outside_dimming` with `CaveSurroundings::Dimmed`: the land takes the tone of what is outside the
/// player's cave fully. That tone, darkened to `CAVE_OUTSIDE_SHADE` of its brightness and keeping
/// `CAVE_OUTSIDE_SATURATION` of its colour, is art/depth.wesl's `cave_outside_tone`, the one place it is set: the land
/// (`SceneUniform::land_outside_dimming`), the statics and water (`STATIC_FLAG_CAVE_OUTSIDE`) take it in every style,
/// and their lights (`light_clip_tag`) its colour: the light buffer they draw into multiplies the darkened scene.
const LAND_OUTSIDE_DIMMED: f32 = 1.0;

/// The eight neighbours of a cell.
const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// The four neighbours sharing a side with a cell.
const SIDES: [(i32, i32); 4] = [(0, -1), (-1, 0), (1, 0), (0, 1)];
/// A block's cells, `MAP_STORAGE_BLOCK_TILE_DIM` squared, one bit each in a `u64`.
const BLOCK_CELLS: usize = (MAP_STORAGE_BLOCK_TILE_DIM * MAP_STORAGE_BLOCK_TILE_DIM) as usize;
/// The cells of a package chunk.
const CHUNK_CELLS: usize = (PACKAGE_CHUNK_CELL_DIM * PACKAGE_CHUNK_CELL_DIM) as usize;
/// Surface statics no one stands on: roofs, and the impassable ones items are put on (tables, counters).
const NOT_A_FLOOR_FLAGS: u64 = TaeFlag::Roof.bits() | TaeFlag::Impassable.bits();
/// `ItemSurfaces` entry of a graphic that is no floor.
const NOT_A_FLOOR: i16 = i16::MIN;

/// The top a walker stands on of a static with tile flags `flags` and height `height` at `z`: Surface statics that are
/// neither roofs nor impassable, a Bridge at half its height (the servers' stair rule). `None` for the others.
pub fn surface_top(flags: u64, height: i8, z: i8) -> Option<i32> {
    if flags & TaeFlag::Surface.bits() == 0 || flags & NOT_A_FLOOR_FLAGS != 0 {
        return None;
    }
    let height = i32::from(height);
    let stood_height = if flags & TaeFlag::Bridge.bits() != 0 { height / 2 } else { height };
    Some(i32::from(z) + stood_height)
}

/// The cave floor of a cell with land z `land_z` and the Surface tops `tops` (`surface_top`): the highest top, when it
/// is at least `CAVE_MIN_BURY_DEPTH` below the land.
pub fn cave_floor(land_z: i32, tops: &[i32]) -> Option<i32> {
    let floor = *tops.iter().max()?;
    (land_z - floor >= CAVE_MIN_BURY_DEPTH).then_some(floor)
}

/// Whether a static with tile flags `flags` and height `height` at `z` is a cave's floor (`surface_top`) at least
/// `CAVE_MIN_BURY_DEPTH` under its cell's land, `land_z` giving the land z, read only for a floor.
pub fn is_buried_floor(flags: u64, height: i8, z: i8, land_z: impl FnOnce() -> Option<i32>) -> bool {
    surface_top(flags, height, z).is_some_and(|top| land_z().is_some_and(|land_z| land_z - top >= CAVE_MIN_BURY_DEPTH))
}

/// The cells the rule reads.
pub trait CaveCells {
    /// The land z of a cell; `None` off the map.
    fn land_z(&self, x: i32, y: i32) -> Option<i32>;
    /// The tops of the cell's floor statics (`surface_top`).
    fn surface_tops(&self, x: i32, y: i32) -> &[i32];
}

/// The candidates of the cells `x0..x1` by `y0..y1` (`cave_floor`), by cell.
pub fn cave_candidates(cells: &impl CaveCells, x0: i32, y0: i32, x1: i32, y1: i32) -> HashMap<(i32, i32), i32> {
    let mut candidates = HashMap::new();
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(floor) = cells.land_z(x, y).and_then(|land_z| cave_floor(land_z, cells.surface_tops(x, y))) {
                candidates.insert((x, y), floor);
            }
        }
    }
    candidates
}

/// Whether a walker on a floor at `floor` steps onto the cell (`x`, `y`), not a candidate: its land, or a Surface top
/// that is not buried, is at most `WALK_STEP_Z` away.
fn walk_reaches(cells: &impl CaveCells, x: i32, y: i32, floor: i32) -> bool {
    let Some(land_z) = cells.land_z(x, y) else {
        return false;
    };
    (land_z - floor).abs() <= WALK_STEP_Z
        || cells.surface_tops(x, y).iter()
            .any(|&top| land_z - top < CAVE_MIN_BURY_DEPTH && (top - floor).abs() <= WALK_STEP_Z)
}

/// The candidates that show: each component (eight neighbours, floors at most `WALK_STEP_Z` apart) with a mouth.
/// `cells` must hold every candidate's neighbours.
pub fn shown_cave_floors(cells: &impl CaveCells, candidates: &HashMap<(i32, i32), i32>) -> HashMap<(i32, i32), i32> {
    let mut shown = HashMap::new();
    let mut visited = HashSet::with_capacity(candidates.len());
    let mut component = Vec::new();
    let mut stack = Vec::new();
    for (&start, _) in candidates {
        if !visited.insert(start) {
            continue;
        }
        component.clear();
        stack.push(start);
        let mut has_mouth = false;
        while let Some(cell) = stack.pop() {
            component.push(cell);
            let floor = candidates[&cell];
            for (dx, dy) in NEIGHBOURS {
                let next = (cell.0 + dx, cell.1 + dy);
                match candidates.get(&next) {
                    Some(&next_floor) => {
                        if (next_floor - floor).abs() <= WALK_STEP_Z && visited.insert(next) {
                            stack.push(next);
                        }
                    }
                    None => has_mouth = has_mouth || walk_reaches(cells, next.0, next.1, floor),
                }
            }
        }
        if has_mouth {
            shown.extend(component.iter().map(|cell| (*cell, candidates[cell])));
        }
    }
    shown
}

/// What a shown cave, or the player's (`PlayerCave::static_view`), does to a static.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaveStatic {
    /// Drawn as without caves; outside the player's cave, darkened with `CaveSurroundings::Dimmed`.
    Outside,
    /// Over a shown cave floor, at or above its ceiling, or not the player's cave's: not drawn.
    Hidden,
    /// In a shown cave, or a buried wall around one: drawn in front of what is around the cave.
    Lifted,
}

/// The shown caves of one block.
#[derive(Clone)]
struct CaveBlock {
    /// The shown cells.
    inside: u64,
    /// The cells around them.
    ring: u64,
    /// Inside, the ceiling: statics below it are lifted, the others hidden. On the ring, the statics below it are
    /// lifted: the highest adjacent ceiling, lowered to the buried statics (`CAVE_MIN_BURY_DEPTH` under the land).
    below_z: [i16; BLOCK_CELLS],
}

/// The shown caves of a map.
#[derive(Default)]
pub struct CaveMap {
    blocks: HashMap<(u32, u32), CaveBlock>,
    inside_cells: usize,
}

impl CaveMap {
    /// The map of the shown floors `shown` (`shown_cave_floors`); `cells` must hold their neighbours.
    pub fn new(shown: &HashMap<(i32, i32), i32>, cells: &impl CaveCells) -> Self {
        let mut map = Self { blocks: HashMap::new(), inside_cells: shown.len() };
        for (&(x, y), &floor) in shown {
            let ceiling = floor + CAVE_CEILING_HEADROOM;
            map.mark(x, y, true, ceiling);
            for (dx, dy) in NEIGHBOURS {
                let (next_x, next_y) = (x + dx, y + dy);
                if shown.contains_key(&(next_x, next_y)) {
                    continue;
                }
                if let Some(land_z) = cells.land_z(next_x, next_y) {
                    map.mark(next_x, next_y, false, ceiling.min(land_z - CAVE_MIN_BURY_DEPTH + 1));
                }
            }
        }
        map
    }

    fn mark(&mut self, x: i32, y: i32, inside: bool, below_z: i32) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        let (block, bit, index) = Self::locate(x, y);
        let block = self.blocks.entry(block).or_insert(CaveBlock { inside: 0, ring: 0, below_z: [i16::MIN; BLOCK_CELLS] });
        let below_z = below_z.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        if inside {
            block.inside |= bit;
            block.ring &= !bit;
            block.below_z[index] = below_z;
        } else if block.inside & bit == 0 {
            block.ring |= bit;
            block.below_z[index] = block.below_z[index].max(below_z);
        }
    }

    fn locate(x: u32, y: u32) -> ((u32, u32), u64, usize) {
        let index = ((y % MAP_STORAGE_BLOCK_TILE_DIM) * MAP_STORAGE_BLOCK_TILE_DIM + x % MAP_STORAGE_BLOCK_TILE_DIM) as usize;
        ((x / MAP_STORAGE_BLOCK_TILE_DIM, y / MAP_STORAGE_BLOCK_TILE_DIM), 1 << index, index)
    }

    /// (inside, `CaveBlock::below_z`) of a cell inside or around a shown cave.
    fn cell(&self, x: u32, y: u32) -> Option<(bool, i32)> {
        let (block, bit, index) = Self::locate(x, y);
        let block = self.blocks.get(&block)?;
        if (block.inside | block.ring) & bit == 0 {
            return None;
        }
        Some((block.inside & bit != 0, i32::from(block.below_z[index])))
    }

    /// How many cells show.
    pub fn inside_cells(&self) -> usize {
        self.inside_cells
    }

    /// The z the land of the cell is drawn at when it covers a shown cave: the cave floor.
    pub fn floor_z(&self, x: u32, y: u32) -> Option<i8> {
        match self.cell(x, y)? {
            (true, ceiling) => Some((ceiling - CAVE_CEILING_HEADROOM).clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8),
            _ => None,
        }
    }

    /// What the caves do to a static at `z` on the cell (`x`, `y`).
    pub fn static_view(&self, x: i32, y: i32, z: i8) -> CaveStatic {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return CaveStatic::Outside;
        };
        match self.cell(x, y) {
            Some((_, below_z)) if i32::from(z) < below_z => CaveStatic::Lifted,
            Some((true, _)) => CaveStatic::Hidden,
            _ => CaveStatic::Outside,
        }
    }

    /// The z limit of the light cover (art/light_cover.rs) for a light on the cell (`x`, `y`) under the view's limit
    /// `max_z`: a shown cell caps it at its ceiling, as the z-view does for a player standing on its floor.
    pub fn light_max_z(&self, x: u32, y: u32, max_z: i32) -> i32 {
        match self.cell(x, y) {
            Some((true, ceiling)) => max_z.min(ceiling),
            _ => max_z,
        }
    }
}

/// The buried floor tops of the cell (`x`, `y`): those at least `CAVE_MIN_BURY_DEPTH` under its land.
fn buried_tops(cells: &impl CaveCells, x: i32, y: i32) -> impl Iterator<Item = i32> + '_ {
    let land_z = cells.land_z(x, y);
    cells.surface_tops(x, y).iter().copied().filter(move |&top| land_z.is_some_and(|land_z| land_z - top >= CAVE_MIN_BURY_DEPTH))
}

/// The floor the player at `z` on (`x`, `y`) stands on in a cave: the highest buried floor top at most `WALK_STEP_Z`
/// above the player and less than `CAVE_LEVEL_HEIGHT` below. `None` out of a cave.
pub fn player_cave_floor(cells: &impl CaveCells, x: i32, y: i32, z: i32) -> Option<i32> {
    buried_tops(cells, x, y).filter(|&top| top <= z + WALK_STEP_Z && z - top < CAVE_LEVEL_HEIGHT).max()
}

/// The cells of the player's cave, with the lowest and highest of their floors: the buried floors reached from
/// `floor`, the player's on (`x`, `y`) (`player_cave_floor`), through the eight neighbours, each step at most
/// `WALK_STEP_Z`, each floor within `CAVE_LEVEL_HEIGHT` of `floor`, on the cells `in_reach` allows.
pub fn player_cave_cells(cells: &impl CaveCells, x: i32, y: i32, floor: i32, in_reach: impl Fn(i32, i32) -> bool)
    -> HashMap<(i32, i32), (i32, i32)> {
    let mut found: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut visited = HashSet::from([(x, y, floor)]);
    let mut stack = vec![(x, y, floor)];
    while let Some((cell_x, cell_y, cell_floor)) = stack.pop() {
        let range = found.entry((cell_x, cell_y)).or_insert((cell_floor, cell_floor));
        *range = (range.0.min(cell_floor), range.1.max(cell_floor));
        for (dx, dy) in NEIGHBOURS {
            let (next_x, next_y) = (cell_x + dx, cell_y + dy);
            if !in_reach(next_x, next_y) {
                continue;
            }
            for top in buried_tops(cells, next_x, next_y) {
                if (top - cell_floor).abs() <= WALK_STEP_Z && (top - floor).abs() < CAVE_LEVEL_HEIGHT
                    && visited.insert((next_x, next_y, top)) {
                    stack.push((next_x, next_y, top));
                }
            }
        }
    }
    found
}

/// A cell of the player's cave or around it (`PlayerCave`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlayerCaveCell {
    /// How far from the cave, in cells: 0 inside it, up to `CAVE_WALL_REACH` around it.
    distance: u8,
    /// The lowest and highest floor z of the cave there, or around it of the nearest cave cells.
    low: i16,
    high: i16,
    /// The cell's land z.
    land_z: i16,
}

/// The player's cave (module docs): its cells and those around it up to `CAVE_WALL_REACH`, and what shows outside it.
#[derive(Debug, PartialEq, Eq)]
pub struct PlayerCave {
    /// The floor the player stands on.
    floor_z: i32,
    cells: HashMap<(i32, i32), PlayerCaveCell>,
    /// The buried floors of other tunnels on the cells of `cells`: those of the cells around the cave, and those of its
    /// cells not its own.
    other_floors: HashMap<(i32, i32), Box<[i16]>>,
    /// How many cells are the cave's.
    inside_cells: usize,
    surroundings: CaveSurroundings,
    /// The cells next to the cave whose land its floor walks out onto (`exits`).
    exits: HashSet<(i32, i32)>,
    /// The cells whose land is the cave's mouth (`owns_land`).
    mouth_land: HashSet<(i32, i32)>,
}

impl PlayerCave {
    /// The cave of the cells `inside` (`player_cave_cells`) from the player's floor `floor_z`, the cells around it read
    /// from `cells` where `in_reach` allows.
    pub fn new(inside: &HashMap<(i32, i32), (i32, i32)>, floor_z: i32, cells: &impl CaveCells,
        in_reach: impl Fn(i32, i32) -> bool, surroundings: CaveSurroundings) -> Self {
        let to_i16 = |z: i32| z.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        let mut found = HashMap::with_capacity(inside.len());
        let mut layer = Vec::with_capacity(inside.len());
        for (&cell, &(low, high)) in inside {
            let land_z = cells.land_z(cell.0, cell.1).unwrap_or(high);
            found.insert(cell, PlayerCaveCell { distance: 0, low: to_i16(low), high: to_i16(high), land_z: to_i16(land_z) });
            layer.push(cell);
        }
        // The rings around the cave, one cell farther each, each cell taking the floors of the nearest ring's cells.
        for distance in 1..=CAVE_WALL_REACH as u8 {
            let mut next_layer = Vec::new();
            for &(x, y) in &layer {
                let from = found[&(x, y)];
                for (dx, dy) in NEIGHBOURS {
                    let next = (x + dx, y + dy);
                    let Some(land_z) = cells.land_z(next.0, next.1).filter(|_| in_reach(next.0, next.1)) else {
                        continue;
                    };
                    match found.get_mut(&next) {
                        Some(cell) if cell.distance == distance => {
                            cell.low = cell.low.min(from.low);
                            cell.high = cell.high.max(from.high);
                        }
                        Some(_) => {}
                        None => {
                            found.insert(next, PlayerCaveCell { distance, low: from.low, high: from.high, land_z: to_i16(land_z) });
                            next_layer.push(next);
                        }
                    }
                }
            }
            layer = next_layer;
        }
        let other_floors = found.iter().filter_map(|(&(x, y), cell)| {
            let own = |top: i32| cell.distance == 0 && i32::from(cell.low) <= top && top <= i32::from(cell.high);
            let tops = buried_tops(cells, x, y).filter(|&top| !own(top)).map(to_i16).collect::<Box<[i16]>>();
            (!tops.is_empty()).then_some(((x, y), tops))
        }).collect();
        let exits = Self::exits(&found);
        let mouth_land = Self::mouth_land(&found, &exits, cells);
        Self { floor_z, cells: found, other_floors, inside_cells: inside.len(), surroundings, exits, mouth_land }
    }

    /// The exits of the cave among `cells`: the cells next to it whose land is at most `WALK_STEP_Z` from the floors
    /// next to them, the land its floor walks out onto at a mouth.
    fn exits(cells: &HashMap<(i32, i32), PlayerCaveCell>) -> HashSet<(i32, i32)> {
        cells.iter().filter(|(_, cell)| {
            let (low, high, land_z) = (i32::from(cell.low), i32::from(cell.high), i32::from(cell.land_z));
            cell.distance == 1 && low - WALK_STEP_Z <= land_z && land_z <= high + WALK_STEP_Z
        }).map(|(&cell, _)| cell).collect()
    }

    /// The land of the cave's mouth (module docs) among `cells`: the cells whose land quad (its corners the cell and
    /// its neighbours at +x, +y and both) has a corner on a rim, a buried cell of the cave sharing a side with one of
    /// the `exits`: the face over the opening and the frame's land beside it. The rock beside the opening, a cell of
    /// the ring next to an exit or a cave cell touching one at a corner only, frames nothing: its quads are the hill's
    /// flank, which lifted stood over the cut as a slab beside the opening. A rim under a built ceiling, a floor static
    /// more than `PLAYER_CEILING_MIN_RISE` over the cave's floor that the land does not bury, is a doorway into a room
    /// set in the hill, not a mouth: its land is no frame, it stood as a slab of rock in the room (the pentagram room
    /// under its stone roof at map 1, 1362,883).
    fn mouth_land(cells: &HashMap<(i32, i32), PlayerCaveCell>, exits: &HashSet<(i32, i32)>, map: &impl CaveCells)
        -> HashSet<(i32, i32)> {
        let mut mouth = HashSet::new();
        for (&(x, y), cell) in cells {
            let (high, land_z) = (i32::from(cell.high), i32::from(cell.land_z));
            let buried = land_z - high >= CAVE_MIN_BURY_DEPTH;
            if cell.distance != 0 || !buried || !SIDES.iter().any(|&(dx, dy)| exits.contains(&(x + dx, y + dy))) {
                continue;
            }
            let built_ceiling = map.surface_tops(x, y).iter()
                .any(|&top| top > high + PLAYER_CEILING_MIN_RISE && land_z - top < CAVE_MIN_BURY_DEPTH);
            if built_ceiling {
                continue;
            }
            for quad in [(x, y), (x - 1, y), (x, y - 1), (x - 1, y - 1)] {
                if cells.contains_key(&quad) {
                    mouth.insert(quad);
                }
            }
        }
        mouth
    }

    pub fn floor_z(&self) -> i32 {
        self.floor_z
    }

    /// How many cells are the cave's.
    pub fn inside_cells(&self) -> usize {
        self.inside_cells
    }

    pub fn surroundings(&self) -> CaveSurroundings {
        self.surroundings
    }

    /// Whether the land of the cell (`x`, `y`) is the cave's mouth (module docs): drawn whole, over the z-view's cut
    /// and in the black surroundings, and not darkened.
    pub fn owns_land(&self, x: i32, y: i32) -> bool {
        self.mouth_land.contains(&(x, y))
    }

    /// Whether the cave has a mouth whose land it draws (`owns_land`).
    pub fn has_mouth_land(&self) -> bool {
        !self.mouth_land.is_empty()
    }

    /// The cells whose land is the cave's mouth (`owns_land`).
    pub fn mouth_land_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.mouth_land.iter().copied()
    }

    /// What the cave does to a static at `z` with tile flags `flags` and height `height` on the cell (`x`, `y`),
    /// `land_z` giving the cell's land z where the cave did not read it: `Lifted`, the cave's; `Outside`, drawn as
    /// outside the cave (`CaveSurroundings::Dimmed`); `Hidden`, not drawn.
    pub fn static_view(&self, x: i32, y: i32, z: i8, flags: u64, height: i8, land_z: impl FnOnce() -> Option<i32>)
        -> CaveStatic {
        let z = i32::from(z);
        let outside = match self.surroundings {
            CaveSurroundings::Dimmed => CaveStatic::Outside,
            CaveSurroundings::Black => CaveStatic::Hidden,
        };
        let buried = |land_z: i32| land_z - z >= CAVE_MIN_BURY_DEPTH;
        let Some(cell) = self.cells.get(&(x, y)) else {
            return if land_z().is_some_and(buried) { CaveStatic::Hidden } else { outside };
        };
        let (low, high, land_z) = (i32::from(cell.low), i32::from(cell.high), i32::from(cell.land_z));
        let distance = i32::from(cell.distance);
        // Under the mouth's land, drawn whole in front of it, the cave's floor is covered, as in the KR client: its art,
        // standing upright at one depth, would show through the steep land as teeth along the opening's bottom edge.
        if self.mouth_land.contains(&(x, y)) && is_buried_floor(flags, height, z as i8, || Some(land_z)) {
            return CaveStatic::Hidden;
        }
        if distance == 0 {
            return if low - CAVE_LEVEL_HEIGHT <= z && z < high + CAVE_LEVEL_HEIGHT { CaveStatic::Lifted } else { CaveStatic::Hidden };
        }
        // On an exit, what stands at the cave's level, not water, frames its mouth with the land (`owns_land`): the
        // cave entrance art standing in the opening.
        let rock = flags & TaeFlag::Wet.bits() == 0;
        if rock && self.exits.contains(&(x, y)) && low - CAVE_LEVEL_HEIGHT <= z && z < high + CAVE_LEVEL_HEIGHT {
            return CaveStatic::Lifted;
        }
        // Around the cave: a buried floor is another tunnel's, and what stands on one under the cave's floor, or next to
        // one, a level up from it at most: the walls and the bones of a tunnel below.
        if is_buried_floor(flags, height, z as i8, || Some(land_z)) {
            return CaveStatic::Hidden;
        }
        let other_level = |cell: (i32, i32)| self.other_floors.get(&cell).is_some_and(|tops|
            tops.iter().any(|&top| i32::from(top) <= z && z < i32::from(top) + CAVE_LEVEL_HEIGHT));
        if z < low && (other_level((x, y)) || NEIGHBOURS.iter().any(|&(dx, dy)| other_level((x + dx, y + dy)))) {
            return CaveStatic::Hidden;
        }
        let wall_bottom = low - CAVE_WALL_DROP_PER_CELL * (distance + CAVE_WALL_DROP_SLACK_CELLS);
        // Buried, as the rock walls are: the rocks and logs lying on the land outside, a little under it, are not lifted,
        // nor the water under a shore's lip: rock is never wet.
        if rock && buried(land_z) && z < high + CAVE_LEVEL_HEIGHT && z >= wall_bottom {
            CaveStatic::Lifted
        } else if buried(land_z) {
            CaveStatic::Hidden
        } else {
            outside
        }
    }

    /// The z limit of the light cover (art/light_cover.rs) of a light the cave draws (`CaveStatic::Lifted`) on the cell
    /// (`x`, `y`): over its floors' level, what is not drawn covers nothing.
    fn light_max_z(&self, x: i32, y: i32) -> i32 {
        self.cells.get(&(x, y)).map_or(NO_Z_VIEW_LIMIT, |cell| i32::from(cell.high) + CAVE_LEVEL_HEIGHT)
    }
}

/// How a light shows (`CavesRes::light_view`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaveLight {
    /// The z limit of its cover (art/light_cover.rs).
    pub max_z: i32,
    /// Outside the player's cave, darkened with what is around it (`CaveSurroundings::Dimmed`).
    pub outside: bool,
}

/// The floor top over its z (`surface_top`) of each item graphic, `NOT_A_FLOOR` for the others.
struct ItemSurfaces(Vec<i16>);

impl ItemSurfaces {
    fn new(tilemeta: &TileMetaPackage) -> Self {
        Self(tilemeta.item_tiles().iter()
            .map(|item| surface_top(item.flags, item.height, 0).map_or(NOT_A_FLOOR, |top| top as i16))
            .collect())
    }

    fn top(&self, graphic: u16, z: i8) -> Option<i32> {
        let rise = *self.0.get(usize::from(graphic))?;
        (rise != NOT_A_FLOOR).then(|| i32::from(z) + i32::from(rise))
    }
}

/// The cells of one package chunk: land z and Surface tops, the tops of cell `i` in `tops[starts[i]..starts[i + 1]]`.
struct ChunkCells {
    land_z: Vec<i8>,
    starts: Vec<u32>,
    tops: Vec<i32>,
}

impl ChunkCells {
    fn read(map: &UddpReader, statics: &UddpReader, surfaces: &ItemSurfaces, chunk_index: u32) -> eyre::Result<Self> {
        let land_z = read_package_chunk_land_z(map, chunk_index)?;
        let mut cell_tops = read_package_chunk_statics(statics, chunk_index)?
            .into_iter()
            .filter_map(|tile| {
                let cell = usize::from(tile.y_offset) * PACKAGE_CHUNK_CELL_DIM as usize + usize::from(tile.x_offset);
                Some((cell, surfaces.top(tile.graphic, tile.z)?)).filter(|(cell, _)| *cell < CHUNK_CELLS)
            })
            .collect::<Vec<_>>();
        cell_tops.sort_unstable();
        let mut starts = vec![0u32; CHUNK_CELLS + 1];
        for &(cell, _) in &cell_tops {
            starts[cell + 1] += 1;
        }
        for index in 0..CHUNK_CELLS {
            starts[index + 1] += starts[index];
        }
        Ok(Self { land_z, starts, tops: cell_tops.into_iter().map(|(_, top)| top).collect() })
    }

    fn tops(&self, cell: usize) -> &[i32] {
        &self.tops[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }
}

/// The package chunks of a map read so far, as `CaveCells`.
struct MapChunks {
    width: i32,
    height: i32,
    chunks: HashMap<u32, ChunkCells>,
}

impl MapChunks {
    /// The chunk of the cell (`x`, `y`) and the cell's index in it; `None` off the map.
    fn locate(&self, x: i32, y: i32) -> Option<(u32, usize)> {
        package_chunk_cell(self.width as u32, self.height as u32, x, y)
    }
}

impl CaveCells for MapChunks {
    fn land_z(&self, x: i32, y: i32) -> Option<i32> {
        let (chunk, cell) = self.locate(x, y)?;
        Some(i32::from(*self.chunks.get(&chunk)?.land_z.get(cell)?))
    }

    fn surface_tops(&self, x: i32, y: i32) -> &[i32] {
        self.locate(x, y).and_then(|(chunk, cell)| Some(self.chunks.get(&chunk)?.tops(cell))).unwrap_or(&[])
    }
}

/// Finds the player's cave on a map, keeping the package chunks it read around the player.
pub struct PlayerCaveFinder {
    map_id: u32,
    map: Arc<UddpReader>,
    statics: Arc<UddpReader>,
    surfaces: ItemSurfaces,
    chunks: MapChunks,
}

impl PlayerCaveFinder {
    fn new(map_id: u32, width: u32, height: u32, map: Arc<UddpReader>, statics: Arc<UddpReader>, tilemeta: &TileMetaPackage)
        -> Self {
        let chunks = MapChunks { width: width as i32, height: height as i32, chunks: HashMap::new() };
        Self { map_id, map, statics, surfaces: ItemSurfaces::new(tilemeta), chunks }
    }

    /// The package chunk (column, row) of the cell (`x`, `y`).
    fn chunk_of(x: i32, y: i32) -> (i32, i32) {
        let dim = PACKAGE_CHUNK_CELL_DIM as i32;
        (x.div_euclid(dim), y.div_euclid(dim))
    }

    /// Reads the chunk (column, row) unless read or off the map.
    fn load(&mut self, (column, row): (i32, i32)) -> eyre::Result<()> {
        let dim = PACKAGE_CHUNK_CELL_DIM as i32;
        let Some((chunk, _)) = self.chunks.locate(column * dim, row * dim) else {
            return Ok(());
        };
        if !self.chunks.chunks.contains_key(&chunk) {
            let cells = ChunkCells::read(&self.map, &self.statics, &self.surfaces, chunk)?;
            self.chunks.chunks.insert(chunk, cells);
        }
        Ok(())
    }

    /// The cave of the player at `z` on (`x`, `y`) (module docs); `None` out of one.
    fn find(&mut self, x: i32, y: i32, z: i32, surroundings: CaveSurroundings) -> eyre::Result<Option<PlayerCave>> {
        let centre = Self::chunk_of(x, y);
        self.load(centre)?;
        let Some(floor) = player_cave_floor(&self.chunks, x, y, z) else {
            return Ok(None);
        };
        let reach = PLAYER_CAVE_REACH_CHUNKS;
        let in_reach = |x: i32, y: i32| {
            let (column, row) = Self::chunk_of(x, y);
            (column - centre.0).abs() <= reach && (row - centre.1).abs() <= reach
        };
        for row in centre.1 - reach..=centre.1 + reach {
            for column in centre.0 - reach..=centre.0 + reach {
                self.load((column, row))?;
            }
        }
        // The chunks out of reach go; those just out stay for the next steps.
        let height_chunks = (self.chunks.height as u32).div_ceil(PACKAGE_CHUNK_CELL_DIM) as i32;
        self.chunks.chunks.retain(|&chunk, _| {
            let (column, row) = (chunk as i32 / height_chunks, chunk as i32 % height_chunks);
            (column - centre.0).abs() <= reach + 1 && (row - centre.1).abs() <= reach + 1
        });
        let inside = player_cave_cells(&self.chunks, x, y, floor, in_reach);
        Ok(Some(PlayerCave::new(&inside, floor, &self.chunks, in_reach, surroundings)))
    }
}

/// What `find_map_caves` reads.
struct CaveScanInput {
    map_id: u32,
    width: u32,
    height: u32,
    map: Arc<UddpReader>,
    statics: Arc<UddpReader>,
    tilemeta: Arc<TileMetaPackage>,
}

/// The shown caves of a whole map: every package chunk is read in parallel for candidates, then the chunks holding
/// them and their neighbours are kept for the flood.
fn find_map_caves(input: &CaveScanInput) -> eyre::Result<CaveMap> {
    let surfaces = ItemSurfaces::new(&input.tilemeta);
    let chunk_rows = input.width.div_ceil(PACKAGE_CHUNK_CELL_DIM);
    let chunk_columns = input.height.div_ceil(PACKAGE_CHUNK_CELL_DIM);
    let chunk_count = chunk_rows * chunk_columns;
    let mut chunks = MapChunks {
        width: input.width as i32,
        height: input.height as i32,
        chunks: HashMap::new(),
    };
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get()) as u32;
    let per_thread = chunk_count.div_ceil(threads);
    let parts = std::thread::scope(|scope| {
        let workers = (0..threads).map(|thread| {
            let (surfaces, input) = (&surfaces, input);
            scope.spawn(move || -> eyre::Result<Vec<(u32, ChunkCells)>> {
                let mut kept = Vec::new();
                for chunk in thread * per_thread..((thread + 1) * per_thread).min(chunk_count) {
                    let cells = ChunkCells::read(&input.map, &input.statics, surfaces, chunk)?;
                    let has_candidate = (0..CHUNK_CELLS)
                        .any(|cell| cave_floor(i32::from(cells.land_z[cell]), cells.tops(cell)).is_some());
                    if has_candidate {
                        kept.push((chunk, cells));
                    }
                }
                Ok(kept)
            })
        }).collect::<Vec<_>>();
        workers.into_iter().map(|worker| worker.join().expect("cave scan thread")).collect::<Vec<_>>()
    });
    for part in parts {
        chunks.chunks.extend(part?);
    }

    let mut candidates = HashMap::new();
    for (&chunk, cells) in &chunks.chunks {
        let (origin_x, origin_y) = ((chunk / chunk_columns) * PACKAGE_CHUNK_CELL_DIM, (chunk % chunk_columns) * PACKAGE_CHUNK_CELL_DIM);
        for cell in 0..CHUNK_CELLS {
            if let Some(floor) = cave_floor(i32::from(cells.land_z[cell]), cells.tops(cell)) {
                let x = (origin_x + cell as u32 % PACKAGE_CHUNK_CELL_DIM) as i32;
                let y = (origin_y + cell as u32 / PACKAGE_CHUNK_CELL_DIM) as i32;
                if x < chunks.width && y < chunks.height {
                    candidates.insert((x, y), floor);
                }
            }
        }
    }
    // The flood and the ring read the candidates' neighbours, which may lie in the chunks around.
    let mut missing = Vec::new();
    for &(x, y) in candidates.keys() {
        for (dx, dy) in NEIGHBOURS {
            if let Some((chunk, _)) = chunks.locate(x + dx, y + dy) {
                if !chunks.chunks.contains_key(&chunk) && !missing.contains(&chunk) {
                    missing.push(chunk);
                }
            }
        }
    }
    for chunk in missing {
        chunks.chunks.insert(chunk, ChunkCells::read(&input.map, &input.statics, &surfaces, chunk)?);
    }

    let shown = shown_cave_floors(&chunks, &candidates);
    Ok(CaveMap::new(&shown, &chunks))
}

/// The shown caves while `world_rendering.caves` draws them over the map, and the player's cave.
#[derive(Resource, Default)]
pub struct CavesRes {
    /// The caves of the current map drawn now; `None` while hidden, while they are found, or while the player is in a
    /// cave (`player`).
    active: Option<Arc<CaveMap>>,
    /// The player's cave, with "Show Only the Player's Floor" on and the player in one.
    player: Option<Arc<PlayerCave>>,
    /// Counts the changes of `active` and `player`, so the caches built from them are rebuilt.
    revision: u64,
    /// The caves found so far, by map.
    found: HashMap<u32, Arc<CaveMap>>,
    /// The map whose caves a background thread is finding, and the thread.
    finding: Option<(u32, JoinHandle<eyre::Result<CaveMap>>)>,
}

impl CavesRes {
    pub fn active(&self) -> Option<&CaveMap> {
        self.active.as_deref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn player(&self) -> Option<&PlayerCave> {
        self.player.as_deref()
    }

    /// How a light on the static at `z` with tile flags `flags` and height `height` on the cell (`x`, `y`) shows under
    /// the view's limit `max_z`, `land_z` giving the cell's land z: `None` when the player's cave hides its static
    /// (`PlayerCave::static_view`); else the cover limit, `max_z` or the caves' (`CaveMap::light_max_z`,
    /// `PlayerCave::light_max_z`), and whether it is darkened outside the player's cave.
    pub fn light_view(&self, x: u32, y: u32, z: i8, flags: u64, height: i8, land_z: impl FnOnce() -> Option<i32>,
        max_z: i32) -> Option<CaveLight> {
        let Some(cave) = self.player() else {
            let max_z = self.active().map_or(max_z, |caves| caves.light_max_z(x, y, max_z));
            return Some(CaveLight { max_z, outside: false });
        };
        let (x, y) = (x as i32, y as i32);
        match cave.static_view(x, y, z, flags, height, land_z) {
            CaveStatic::Lifted => Some(CaveLight { max_z: cave.light_max_z(x, y), outside: false }),
            CaveStatic::Outside => Some(CaveLight { max_z, outside: true }),
            CaveStatic::Hidden => None,
        }
    }

    /// How far the land takes the tone of what is outside the player's cave (`SceneUniform::land_outside_dimming`,
    /// `LAND_OUTSIDE_DIMMED`): all of it is outside the cave but for its mouth.
    pub fn land_outside_dimming(&self) -> f32 {
        match self.player().map(PlayerCave::surroundings) {
            Some(CaveSurroundings::Dimmed) => LAND_OUTSIDE_DIMMED,
            _ => 0.0,
        }
    }
}

/// What the player's cave was last found from: the map, the player's cell and z, and `world_rendering.cave_surroundings`.
type PlayerCaveInputs = (u32, u16, u16, i8, CaveSurroundings);

/// Finds the player's cave when the player moves, with "Show Only the Player's Floor" on (module docs), and bumps
/// `CavesRes::revision` when it changes. A cave found the same as the last one keeps it: the statics and lights
/// built from it are not rebuilt while the player walks in it. The land is packed again when the new cave or the last
/// one has a mouth whose land it draws (`PlayerCave::owns_land`).
pub fn sys_update_player_cave(
    mut commands: Commands,
    chunks: Query<Entity, With<LCMesh>>,
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    player_q: Query<&Player>,
    map_planes: Option<Res<MapPlanesRes>>,
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut caves: ResMut<CavesRes>,
    mut finder: Local<Option<PlayerCaveFinder>>,
    mut last_inputs: Local<Option<PlayerCaveInputs>>,
) {
    let map_id = scene_state.map_id;
    let pos = player_q.iter().find_map(|player| player.current_pos);
    let surroundings = settings.world_rendering.cave_surroundings;
    let inputs = pos.filter(|_| settings.world_rendering.show_only_player_floor)
        .map(|pos| (map_id, pos.x, pos.y, pos.z, surroundings));
    if *last_inputs == inputs {
        return;
    }
    let next = match inputs {
        None => None,
        Some((_, x, y, z, _)) => {
            if finder.as_ref().is_none_or(|finder| finder.map_id != map_id) {
                let plane = map_planes.as_ref().and_then(|planes| planes.0.get(map_id as usize)?.as_ref());
                let statics = statics_res.0.get(map_id as usize).and_then(|store| store.as_ref());
                *finder = match (plane, statics, tilemeta_res.as_ref()) {
                    (Some(plane), Some(statics), Some(tilemeta)) => {
                        let size = plane.size_cells();
                        Some(PlayerCaveFinder::new(map_id, size.width, size.height, plane.package(),
                            statics.lock().package(), &tilemeta.0))
                    }
                    // Tried again on the next move.
                    _ => return,
                };
            }
            let finder = finder.as_mut().expect("a player cave finder");
            let started = Instant::now();
            match finder.find(i32::from(x), i32::from(y), i32::from(z), surroundings) {
                Ok(cave) => {
                    if let Some(cave) = cave.as_ref().filter(|cave| caves.player.as_deref() != Some(cave)) {
                        console_logger::one(LogSev::Info, LogAbout::RenderWorldLand, &format!(
                            "The player's cave: {} cells from floor z {}, found in {} us.",
                            cave.inside_cells(), cave.floor_z(), started.elapsed().as_micros()));
                    }
                    cave
                }
                Err(error) => {
                    console_logger::one(LogSev::Warn, LogAbout::RenderWorldLand,
                        &format!("The player's cave is not drawn: {error:#}"));
                    None
                }
            }
        }
    };
    *last_inputs = inputs;
    if caves.player.as_deref() == next.as_ref() {
        return;
    }
    let mouth_land = |cave: Option<&PlayerCave>| cave.is_some_and(PlayerCave::has_mouth_land);
    if mouth_land(caves.player()) || mouth_land(next.as_ref()) {
        for entity in &chunks {
            commands.entity(entity).insert(PendingTextureBake);
        }
    }
    caves.player = next.map(Arc::new);
    caves.revision += 1;
}

/// Finds the caves of the current map in the background when the setting draws them, and switches the drawn caves:
/// the land chunks are packed again to lower their land (`CaveMap::floor_z`), and the statics and lights follow `revision`.
pub fn sys_update_caves(
    mut commands: Commands,
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    map_planes: Option<Res<MapPlanesRes>>,
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    chunks: Query<Entity, With<LCMesh>>,
    mut caves: ResMut<CavesRes>,
) {
    let caves = caves.bypass_change_detection();
    if caves.finding.as_ref().is_some_and(|(_, thread)| thread.is_finished()) {
        let (map_id, thread) = caves.finding.take().expect("a finished cave scan");
        let result = thread.join().unwrap_or_else(|_| Err(eyre::eyre!("the cave scan thread panicked")));
        let found = result.unwrap_or_else(|error| {
            console_logger::one(LogSev::Warn, LogAbout::RenderWorldLand,
                &format!("Caves of map {map_id} are not drawn over the map: {error:#}"));
            CaveMap::default()
        });
        caves.found.insert(map_id, Arc::new(found));
    }

    let map_id = scene_state.map_id;
    // The player's cave takes their place.
    let wanted = settings.world_rendering.caves == CaveView::DrawnOverMap && caves.player.is_none();
    let next = wanted.then(|| caves.found.get(&map_id).cloned()).flatten();
    if wanted && next.is_none() && caves.finding.is_none() {
        let plane = map_planes.as_ref().and_then(|planes| planes.0.get(map_id as usize)?.as_ref());
        let statics = statics_res.0.get(map_id as usize).and_then(|store| store.as_ref());
        match (plane, statics, tilemeta_res.as_ref()) {
            (Some(plane), Some(statics), Some(tilemeta)) => {
                let size = plane.size_cells();
                let input = CaveScanInput {
                    map_id,
                    width: size.width,
                    height: size.height,
                    map: plane.package(),
                    statics: statics.lock().package(),
                    tilemeta: tilemeta.0.clone(),
                };
                let spawned = std::thread::Builder::new().name("caves".to_owned()).spawn(move || {
                    let started = Instant::now();
                    let result = find_map_caves(&input);
                    if let Ok(found) = &result {
                        console_logger::one(LogSev::Info, LogAbout::RenderWorldLand, &format!(
                            "Caves of map {}: {} cells drawn over the map, found in {} ms.",
                            input.map_id, found.inside_cells(), started.elapsed().as_millis()));
                    }
                    result
                });
                match spawned {
                    Ok(thread) => caves.finding = Some((map_id, thread)),
                    Err(error) => {
                        console_logger::one(LogSev::Warn, LogAbout::RenderWorldLand,
                            &format!("Caves of map {map_id} are not drawn over the map: {error}"));
                        caves.found.insert(map_id, Arc::new(CaveMap::default()));
                    }
                }
            }
            // Without the map's statics or the tile data no cave is found.
            _ => {
                caves.found.insert(map_id, Arc::new(CaveMap::default()));
            }
        }
    }

    let same = match (&caves.active, &next) {
        (Some(active), Some(next)) => Arc::ptr_eq(active, next),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    caves.active = next;
    caves.revision += 1;
    for entity in &chunks {
        commands.entity(entity).insert(PendingTextureBake);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROCK: i32 = 60;

    /// Synthetic cells: rock at `ROCK` everywhere with no statics, unless set.
    #[derive(Default)]
    struct Cells(HashMap<(i32, i32), (i32, Vec<i32>)>);

    impl Cells {
        fn with(mut self, x: i32, y: i32, land_z: i32, tops: &[i32]) -> Self {
            self.0.insert((x, y), (land_z, tops.to_vec()));
            self
        }

        /// A floor at `floor` under land at `land_z` over `x0..x1` by `y0..y1`.
        fn floor(mut self, x0: i32, y0: i32, x1: i32, y1: i32, land_z: i32, floor: i32) -> Self {
            for y in y0..y1 {
                for x in x0..x1 {
                    self = self.with(x, y, land_z, &[floor]);
                }
            }
            self
        }

        /// A floor at `floor` under rock over `x0..x1` by `y0..y1`.
        fn tunnel(self, x0: i32, y0: i32, x1: i32, y1: i32, floor: i32) -> Self {
            self.floor(x0, y0, x1, y1, ROCK, floor)
        }

        fn shown(&self) -> HashMap<(i32, i32), i32> {
            shown_cave_floors(self, &cave_candidates(self, -2, -2, 20, 20))
        }
    }

    impl CaveCells for Cells {
        fn land_z(&self, x: i32, y: i32) -> Option<i32> {
            Some(self.0.get(&(x, y)).map_or(ROCK, |cell| cell.0))
        }

        fn surface_tops(&self, x: i32, y: i32) -> &[i32] {
            self.0.get(&(x, y)).map_or(&[], |cell| cell.1.as_slice())
        }
    }

    #[test]
    fn surface_tops_skip_roofs_and_impassables_and_halve_bridges() {
        let surface = TaeFlag::Surface.bits();
        assert_eq!(surface_top(surface, 4, 10), Some(14));
        assert_eq!(surface_top(surface | TaeFlag::Bridge.bits(), 5, 10), Some(12));
        assert_eq!(surface_top(surface | TaeFlag::Roof.bits(), 4, 10), None);
        assert_eq!(surface_top(surface | TaeFlag::Impassable.bits() | TaeFlag::Wet.bits(), 0, 10), None);
        assert_eq!(surface_top(TaeFlag::Wall.bits(), 20, 10), None);
    }

    /// A tunnel under rock whose end opens on grass at the floor's level shows, all of it, its floor rising a step at
    /// a time.
    #[test]
    fn cave_with_a_mouth_shows() {
        let cells = Cells::default().tunnel(2, 3, 9, 5, 0).with(5, 3, ROCK, &[2]).with(6, 3, ROCK, &[4])
            .with(1, 3, 1, &[]);
        let shown = cells.shown();
        assert_eq!(shown.len(), 14);
        assert_eq!(shown.get(&(6, 3)), Some(&4));
    }

    /// A floor under the ground reached by stairs, each step more than a walking step, stays hidden.
    #[test]
    fn basement_stays_hidden() {
        let cells = Cells::default().floor(2, 2, 6, 6, 0, -20);
        let cells = (0..10).fold(cells, |cells, x| (0..10).fold(cells, |cells, y| {
            if cells.0.contains_key(&(x, y)) { cells } else { cells.with(x, y, 0, &[]) }
        }));
        // The stairs: the top step is open ground, 7 above the floor.
        let cells = cells.with(1, 3, 0, &[-13]).with(2, 3, 0, &[-20, -18]);
        assert!(cave_candidates(&cells, 0, 0, 10, 10).len() >= 16);
        assert!(cells.shown().is_empty());
    }

    /// A buried floor with rock all around has no mouth.
    #[test]
    fn sealed_floor_stays_hidden() {
        let cells = Cells::default().tunnel(3, 3, 8, 6, 0);
        assert_eq!(cave_candidates(&cells, 0, 0, 10, 10).len(), 15);
        assert!(cells.shown().is_empty());
    }

    /// Two floors more than a walking step apart are two caves: only the one with the mouth shows.
    #[test]
    fn floors_a_step_apart_split_caves() {
        let cells = Cells::default().tunnel(2, 3, 5, 4, 0).tunnel(5, 3, 8, 4, 10).with(1, 3, 0, &[]);
        let shown = cells.shown();
        assert_eq!(shown.len(), 3);
        assert!(shown.keys().all(|&(x, _)| x < 5));
    }

    /// Inside a shown cave the land is lowered to the floor, the statics below the ceiling are lifted and the others hidden; around
    /// it only the buried statics below the ceiling are lifted; lights there are capped at the ceiling.
    #[test]
    fn cave_map_hides_and_lifts() {
        let cells = Cells::default().tunnel(2, 3, 5, 4, 0).with(1, 3, 0, &[]).with(2, 2, 20, &[]);
        let caves = CaveMap::new(&cells.shown(), &cells);
        assert_eq!(caves.inside_cells(), 3);
        assert_eq!(caves.floor_z(3, 3), Some(0));
        assert_eq!(caves.floor_z(3, 2), None);
        assert_eq!(caves.static_view(3, 3, 0), CaveStatic::Lifted);
        assert_eq!(caves.static_view(3, 3, 15), CaveStatic::Lifted);
        assert_eq!(caves.static_view(3, 3, 16), CaveStatic::Hidden);
        // Rock at `ROCK` around: a wall below the ceiling is lifted, and a static on the rock is left alone.
        assert_eq!(caves.static_view(3, 2, 5), CaveStatic::Lifted);
        assert_eq!(caves.static_view(3, 2, ROCK as i8), CaveStatic::Outside);
        // Land at 20 buries statics only below 5.
        assert_eq!(caves.static_view(2, 2, 4), CaveStatic::Lifted);
        assert_eq!(caves.static_view(2, 2, 5), CaveStatic::Outside);
        assert_eq!(caves.static_view(9, 9, 0), CaveStatic::Outside);
        assert_eq!(caves.light_max_z(3, 3, i32::MAX), CAVE_CEILING_HEADROOM);
        assert_eq!(caves.light_max_z(3, 3, 10), 10);
        assert_eq!(caves.light_max_z(3, 2, i32::MAX), i32::MAX);
    }

    /// The caves of map 0 the rule shows around the places it was tuned on: the Minoc mine and the caves north of
    /// Britain show, the Britain basements and the Buccaneer's Den tunnels stay hidden. Prints the counts.
    #[test]
    #[ignore = "needs packages: set UDD_PATH to the folder of map0.uddp, statics0.uddp and tilemeta.uddp, and run \
                with --ignored --nocapture"]
    fn map0_caves_show_around_the_tuned_places() {
        let folder = std::path::PathBuf::from(std::env::var_os("UDD_PATH").expect("set UDD_PATH"));
        let read = |name: &str| Arc::new(UddpReader::load(&folder.join(name)).unwrap());
        let tilemeta = TileMetaPackage::load(folder.join("tilemeta.uddp")).unwrap();
        let input = CaveScanInput {
            map_id: 0,
            width: 7168,
            height: 4096,
            map: read("map0.uddp"),
            statics: read("statics0.uddp"),
            tilemeta: Arc::new(tilemeta),
        };
        let started = Instant::now();
        let caves = find_map_caves(&input).unwrap();
        println!("map 0: {} cells shown in {} ms", caves.inside_cells(), started.elapsed().as_millis());
        let shown_in = |x0: u32, y0: u32, x1: u32, y1: u32| {
            (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y))).filter(|&(x, y)| caves.floor_z(x, y).is_some()).count()
        };
        let places = [
            ("Minoc mine", shown_in(2400, 350, 2700, 600)),
            ("north of Britain", shown_in(1100, 1050, 1400, 1350)),
            ("Britain basements", shown_in(1300, 1500, 1700, 1800)),
            ("Buccaneer's Den", shown_in(2500, 1980, 2950, 2380)),
        ];
        for (name, count) in places {
            println!("{name}: {count}");
        }
        assert!(places[0].1 > 0 && places[1].1 > 0);
        assert_eq!((places[2].1, places[3].1), (0, 0));
    }

    /// The player's cave from (`x`, `y`) at `z` in `cells`, with no reach bound.
    fn player_cave(cells: &Cells, x: i32, y: i32, z: i32, surroundings: CaveSurroundings) -> Option<PlayerCave> {
        let floor = player_cave_floor(cells, x, y, z)?;
        let inside = player_cave_cells(cells, x, y, floor, |_, _| true);
        Some(PlayerCave::new(&inside, floor, cells, |_, _| true, surroundings))
    }

    /// The player stands in a cave on a buried floor at most a walking step over them and less than a level under
    /// them, the highest such; on an open floor, or with only floors above, they are not in one.
    #[test]
    fn the_player_stands_in_a_cave_on_a_buried_floor() {
        let cells = Cells::default().with(0, 0, ROCK, &[0, 40]).with(1, 0, 0, &[0]).with(2, 0, ROCK, &[30]);
        assert_eq!(player_cave_floor(&cells, 0, 0, 0), Some(0));
        assert_eq!(player_cave_floor(&cells, 0, 0, 12), Some(0));
        assert_eq!(player_cave_floor(&cells, 0, 0, 40), Some(40));
        assert_eq!(player_cave_floor(&cells, 0, 0, -30), None);
        assert_eq!(player_cave_floor(&cells, 1, 0, 0), None);
        assert_eq!(player_cave_floor(&cells, 2, 0, 0), None);
        assert!(player_cave(&cells, 1, 0, 0, CaveSurroundings::Dimmed).is_none());
    }

    /// The cave is the buried floors walked to from the player's, a step at a time: not a tunnel apart from it, not
    /// one past a step too high, not one at the same place a level above.
    #[test]
    fn the_players_cave_is_the_floors_walked_to() {
        let cells = Cells::default()
            .tunnel(0, 0, 6, 2, 0)
            // Rising a step at a time to 6 at x 6..8.
            .with(6, 0, ROCK, &[2]).with(7, 0, ROCK, &[4]).with(8, 0, ROCK, &[6])
            // Past a step too high.
            .with(9, 0, ROCK, &[10])
            // A tunnel apart, through rock.
            .tunnel(0, 4, 6, 5, 0)
            // The level above, over the cave's own cells.
            .with(2, 1, ROCK, &[0, 40]).with(3, 1, ROCK, &[0, 42]);
        let inside = player_cave_cells(&cells, 0, 0, 0, |_, _| true);
        assert_eq!(inside.len(), 12 + 3);
        assert_eq!(inside.get(&(8, 0)), Some(&(6, 6)));
        assert!(!inside.contains_key(&(9, 0)));
        assert!(!inside.contains_key(&(0, 4)));
        assert_eq!(inside.get(&(2, 1)), Some(&(0, 0)));
        // The reach bounds it.
        assert_eq!(player_cave_cells(&cells, 0, 0, 0, |x, _| x < 3).len(), 6);
    }

    /// The cave keeps to the player's level: a ramp rising from it stops less than `CAVE_LEVEL_HEIGHT` over the
    /// player's floor, and the floors it reached keep their lowest and highest.
    #[test]
    fn the_players_cave_keeps_to_one_level() {
        let cells = (0..20).fold(Cells::default(), |cells, x| cells.with(x, 0, ROCK, &[2 * x]))
            .with(3, 0, ROCK, &[6, 8]);
        let inside = player_cave_cells(&cells, 0, 0, 0, |_, _| true);
        let highest = inside.values().map(|&(_, high)| high).max();
        assert_eq!(highest, Some(CAVE_LEVEL_HEIGHT - WALK_STEP_Z));
        assert_eq!(inside.len(), (CAVE_LEVEL_HEIGHT / WALK_STEP_Z) as usize);
        assert_eq!(inside.get(&(3, 0)), Some(&(6, 8)));
    }

    /// On the cave's cells its floor and what stands on it are the cave's, the hill over it and the levels above and
    /// below are not.
    #[test]
    fn the_players_cave_draws_its_own_level() {
        let cells = Cells::default().tunnel(0, 0, 6, 3, 0);
        let cave = player_cave(&cells, 2, 1, 0, CaveSurroundings::Dimmed).unwrap();
        assert_eq!((cave.floor_z(), cave.inside_cells()), (0, 18));
        let view = |z: i8| cave.static_view(2, 1, z, 0, 0, || unreachable!());
        assert_eq!(view(0), CaveStatic::Lifted);
        assert_eq!(view(15), CaveStatic::Lifted);
        assert_eq!(view(-15), CaveStatic::Lifted);
        assert_eq!(view(ROCK as i8), CaveStatic::Hidden);
        assert_eq!(view(40), CaveStatic::Hidden);
        assert_eq!(view(-30), CaveStatic::Hidden);
    }

    /// Around the cave, the statics under the land are its rock walls down to `CAVE_WALL_DROP_PER_CELL` a cell away
    /// under its floor; the buried floors are other tunnels; what stands on the land over them is outside.
    #[test]
    fn the_players_cave_draws_its_rock_walls() {
        let cells = Cells::default().tunnel(0, 0, 3, 3, 0);
        let cave = player_cave(&cells, 1, 1, 0, CaveSurroundings::Dimmed).unwrap();
        let wall = TaeFlag::Wall.bits() | TaeFlag::Impassable.bits();
        let view = |x: i32, z: i8, flags: u64| cave.static_view(x, 1, z, flags, 20, || unreachable!());
        // One cell away: the walls at the floor's level and one level under it, not two.
        assert_eq!(view(3, 0, wall), CaveStatic::Lifted);
        assert_eq!(view(3, -20, wall), CaveStatic::Lifted);
        assert_eq!(view(3, -40, wall), CaveStatic::Hidden);
        // Five cells away, three levels under it.
        assert_eq!(view(7, -60, wall), CaveStatic::Lifted);
        assert_eq!(view(7, -90, wall), CaveStatic::Hidden);
        // Over the cave's level, under the land: hidden; on the land: outside.
        assert_eq!(view(3, 25, wall), CaveStatic::Hidden);
        assert_eq!(view(3, ROCK as i8, wall), CaveStatic::Outside);
        // Outside the cave's mouth, land low over its level: a rock lying a little under it is outside, not a wall.
        let mouth = player_cave(&cells.with(3, 1, 10, &[]), 1, 1, 0, CaveSurroundings::Dimmed).unwrap();
        assert_eq!(mouth.static_view(3, 1, 5, 0, 4, || unreachable!()), CaveStatic::Outside);
        // Water under a shore's lip, buried: not a wall.
        assert_eq!(view(3, 0, TaeFlag::Wet.bits()), CaveStatic::Hidden);
        // Another tunnel's floor.
        assert_eq!(view(4, -40, TaeFlag::Surface.bits()), CaveStatic::Hidden);
        // Farther than `CAVE_WALL_REACH`: buried, hidden; over the land, or the land unknown, outside.
        let far = 3 + CAVE_WALL_REACH;
        assert_eq!(cave.static_view(far, 1, 0, wall, 20, || Some(ROCK)), CaveStatic::Hidden);
        assert_eq!(cave.static_view(far, 1, ROCK as i8, wall, 20, || Some(ROCK)), CaveStatic::Outside);
        assert_eq!(cave.static_view(far, 1, 0, wall, 20, || None), CaveStatic::Outside);
    }

    /// A tunnel below the cave, beside it: its floor, its walls and what lies on it are not drawn, the cave's rock
    /// walls down to its level are.
    #[test]
    fn the_players_cave_leaves_out_the_tunnels_below() {
        let cells = Cells::default().tunnel(0, 0, 3, 3, 0).tunnel(5, 0, 8, 3, -40);
        let cave = player_cave(&cells, 1, 1, 0, CaveSurroundings::Dimmed).unwrap();
        assert_eq!(cave.inside_cells(), 9);
        let wall = TaeFlag::Wall.bits();
        let view = |x: i32, z: i8, flags: u64| cave.static_view(x, 1, z, flags, 20, || unreachable!());
        // Next to the tunnel below: its walls go; the cave's at its own floor and the course under it stay.
        assert_eq!(view(4, -40, wall), CaveStatic::Hidden);
        assert_eq!(view(4, -20, wall), CaveStatic::Lifted);
        assert_eq!(view(4, 0, wall), CaveStatic::Lifted);
        // On the tunnel below: its floor and its bones.
        assert_eq!(view(6, -40, TaeFlag::Surface.bits()), CaveStatic::Hidden);
        assert_eq!(view(6, -38, 0), CaveStatic::Hidden);
        // Without it, the wall at its level is a rock wall of the cave.
        let alone = player_cave(&Cells::default().tunnel(0, 0, 3, 3, 0), 1, 1, 0, CaveSurroundings::Dimmed).unwrap();
        assert_eq!(alone.static_view(4, 1, -40, wall, 20, || unreachable!()), CaveStatic::Lifted);
    }

    /// With `CaveSurroundings::Black` nothing outside the cave is drawn.
    #[test]
    fn black_surroundings_hide_what_is_outside_the_players_cave() {
        let cells = Cells::default().tunnel(0, 0, 3, 3, 0);
        let cave = player_cave(&cells, 1, 1, 0, CaveSurroundings::Black).unwrap();
        let wall = TaeFlag::Wall.bits();
        assert_eq!(cave.static_view(3, 1, 0, wall, 20, || unreachable!()), CaveStatic::Lifted);
        assert_eq!(cave.static_view(3, 1, ROCK as i8, wall, 20, || unreachable!()), CaveStatic::Hidden);
        assert_eq!(cave.static_view(40, 1, 0, wall, 20, || Some(0)), CaveStatic::Hidden);
    }

    /// Where the cave's floor walks out onto land at its level, the land rising from there over the floor is its
    /// mouth's, drawn whole: the face over the opening and the frame beside it, the cave's floor under it not drawn;
    /// the rock beside the opening frames nothing, it is the hill's flank. What stands at the cave's level on the exits
    /// is the cave's. A tunnel whose ground outside is a hill slope over its level has no mouth.
    #[test]
    fn the_players_cave_owns_its_mouth() {
        // A tunnel under rock over 0..3 by 0..3, opening east onto land at its floor's level over x = 3..6.
        let open = |cells: Cells| (0..3).fold(cells, |cells, y| (3..6).fold(cells, |cells, x| cells.with(x, y, 0, &[])));
        let cells = open(Cells::default().tunnel(0, 0, 3, 3, 0));
        let cave = player_cave(&cells, 1, 1, 0, CaveSurroundings::Black).unwrap();
        let mouth = (-4..8).flat_map(|y| (-4..8).map(move |x| (x, y))).filter(|&(x, y)| cave.owns_land(x, y))
            .collect::<HashSet<_>>();
        // The rims, the cave's cells sharing a side with the exits (3, 0..3): (2, 0..3). Their quads start a cell back
        // and up from them: 1..3 by -1..3.
        let expected = (-1..3).flat_map(|y| (1..3).map(move |x| (x, y))).collect::<HashSet<_>>();
        assert_eq!(mouth, expected);
        assert!(cave.has_mouth_land());
        // The rock beside the opening, (3, -1) and (3, 3), and the cave's corner cells touching an exit only at a corner
        // are no rims: the hill's flank there, (3, -1) and (2, -2), is not the mouth's.
        assert!(!cave.owns_land(3, -1) && !cave.owns_land(2, -2) && !cave.owns_land(3, 2));
        // Under the mouth's land the cave's floor is not drawn, what stands on it is; deeper in, the floor is.
        let floor = TaeFlag::Surface.bits();
        assert_eq!(cave.static_view(2, 1, 0, floor, 0, || unreachable!()), CaveStatic::Hidden);
        assert_eq!(cave.static_view(2, 1, 0, 0, 20, || unreachable!()), CaveStatic::Lifted);
        assert_eq!(cave.static_view(0, 1, 0, floor, 0, || unreachable!()), CaveStatic::Lifted);
        // On an exit, a rock at the cave's level is the cave's; one a level over it, or water, is not.
        let wall = TaeFlag::Wall.bits();
        assert_eq!(cave.static_view(3, 1, 0, wall, 20, || unreachable!()), CaveStatic::Lifted);
        assert_eq!(cave.static_view(3, 1, CAVE_LEVEL_HEIGHT as i8, wall, 20, || unreachable!()), CaveStatic::Hidden);
        assert_eq!(cave.static_view(3, 1, 0, TaeFlag::Wet.bits(), 0, || unreachable!()), CaveStatic::Hidden);
        // Two cells out, on the open land: outside.
        assert_eq!(cave.static_view(4, 1, 0, wall, 20, || unreachable!()), CaveStatic::Hidden);
        // A room set in the hill, its front under a built ceiling over the floor on the land's face: a doorway, no
        // mouth. A floor buried deeper in the hill over the rim, another tunnel's, is no ceiling of the opening.
        let roof = PLAYER_CEILING_MIN_RISE + 1;
        let tunnel = || open(Cells::default().tunnel(0, 0, 3, 3, 0));
        let room = (0..3).fold(tunnel(), |cells, y| cells.with(2, y, CAVE_MIN_BURY_DEPTH, &[0, roof]));
        assert!(!player_cave(&room, 1, 1, 0, CaveSurroundings::Black).unwrap().has_mouth_land());
        let upper = ROCK - CAVE_MIN_BURY_DEPTH;
        let tunnels = (0..3).fold(tunnel(), |cells, y| cells.with(2, y, ROCK, &[0, upper]));
        assert!(player_cave(&tunnels, 1, 1, 0, CaveSurroundings::Black).unwrap().has_mouth_land());
        // Opening onto a slope a level over the floor: no exit, no mouth.
        let slope = (0..3).fold(Cells::default().tunnel(0, 0, 3, 3, 0), |cells, y| cells.with(3, y, CAVE_MIN_BURY_DEPTH - 1, &[]));
        let cave = player_cave(&slope, 1, 1, 0, CaveSurroundings::Black).unwrap();
        assert!(!cave.has_mouth_land());
        assert_eq!(cave.static_view(3, 1, 0, wall, 20, || unreachable!()), CaveStatic::Hidden);
    }

    /// A cave's floor is buried at least `CAVE_MIN_BURY_DEPTH` under its cell's land: not a floor nearer it, not what
    /// is no floor, not under land not known (or cut away).
    #[test]
    fn floors_buried_under_their_land() {
        let floor = TaeFlag::Surface.bits();
        assert!(is_buried_floor(floor, 0, 0, || Some(CAVE_MIN_BURY_DEPTH)));
        assert!(!is_buried_floor(floor, 0, 0, || Some(CAVE_MIN_BURY_DEPTH - 1)));
        assert!(!is_buried_floor(floor, 1, 0, || Some(CAVE_MIN_BURY_DEPTH)));
        assert!(!is_buried_floor(TaeFlag::Wall.bits(), 0, 0, || unreachable!()));
        assert!(!is_buried_floor(floor, 0, 0, || None));
    }

    /// A light shows with its static: in the cave covered up to its level whatever the view's limit, outside it
    /// under the view's limit and darkened, not at all when hidden. The land is darkened with dimmed surroundings.
    #[test]
    fn lights_follow_the_players_cave() {
        let cells = Cells::default().tunnel(0, 0, 3, 3, 0);
        let mut caves = CavesRes::default();
        let max_z = 16;
        let light = |caves: &CavesRes, x: u32, z: i8| caves.light_view(x, 1, z, 0, 0, || Some(0), max_z);
        assert_eq!(light(&caves, 1, 40), Some(CaveLight { max_z, outside: false }));
        assert_eq!(caves.land_outside_dimming(), 0.0);
        caves.player = player_cave(&cells, 1, 1, 0, CaveSurroundings::Dimmed).map(Arc::new);
        assert_eq!(light(&caves, 1, 0), Some(CaveLight { max_z: CAVE_LEVEL_HEIGHT, outside: false }));
        assert_eq!(light(&caves, 1, 40), None);
        assert_eq!(light(&caves, 40, 5), Some(CaveLight { max_z, outside: true }));
        assert_eq!(caves.land_outside_dimming(), LAND_OUTSIDE_DIMMED);
        caves.player = player_cave(&cells, 1, 1, 0, CaveSurroundings::Black).map(Arc::new);
        assert_eq!(light(&caves, 40, 5), None);
        assert_eq!(caves.land_outside_dimming(), 0.0);
    }

    /// The player's cave found on the packages, from the Minoc mine's entrance, deeper in, out of it, and in the
    /// crypt under the meadow at 1008,805, with the time each find takes: cold, reading the chunks, and a step on.
    /// Needs `UDD_PATH`, a folder of packages.
    #[test]
    #[ignore]
    fn players_caves_on_map_1() {
        let folder = std::path::PathBuf::from(std::env::var_os("UDD_PATH").expect("set UDD_PATH"));
        let read = |name: &str| Arc::new(UddpReader::load(&folder.join(name)).unwrap());
        let tilemeta = TileMetaPackage::load(folder.join("tilemeta.uddp")).unwrap();
        let mut finder = PlayerCaveFinder::new(1, 7168, 4096, read("map1.uddp"), read("statics1.uddp"), &tilemeta);
        let places = [("mine entrance", 2556, 496, 0), ("mine deeper", 2578, 476, 0), ("crypt", 1008, 805, -20),
            ("outside the mine", 2540, 505, 0)];
        for (name, x, y, z) in places {
            let started = Instant::now();
            let cave = finder.find(x, y, z, CaveSurroundings::Dimmed).unwrap();
            let cold = started.elapsed();
            // A step to a neighbouring cell of the cave, its chunks read: the same cave.
            let step = cave.as_ref().and_then(|cave| NEIGHBOURS.into_iter().map(|(dx, dy)| (x + dx, y + dy))
                .find(|cell| cave.cells.get(cell).is_some_and(|cell| cell.distance == 0)));
            let started = Instant::now();
            let again = step.map(|(x, y)| finder.find(x, y, z, CaveSurroundings::Dimmed).unwrap());
            println!("{name}: {:?} cells from floor {:?}, {} us cold, {} us a step on",
                cave.as_ref().map(PlayerCave::inside_cells), cave.as_ref().map(PlayerCave::floor_z),
                cold.as_micros(), started.elapsed().as_micros());
            assert!(cave.is_some() != (name == "outside the mine"));
            assert_eq!(again.flatten(), cave);
        }
    }
}
