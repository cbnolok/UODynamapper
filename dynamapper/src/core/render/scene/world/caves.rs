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
//! (`CavesRes::light_max_z`).

use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::maps::{read_package_chunk_land_z, PACKAGE_CHUNK_CELL_DIM};
use crate::core::render::scene::world::land::draw_mesh::PendingTextureBake;
use crate::core::render::scene::world::land::{LCMesh, MAP_STORAGE_BLOCK_TILE_DIM};
use crate::core::render::scene::world::z_view::PLAYER_VIEW_HEADROOM;
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

/// How far land must rise over a floor to bury it, in z units: the z-view's cave test, land `PLAYER_VIEW_HEADROOM`
/// over the player.
pub const CAVE_MIN_BURY_DEPTH: i32 = PLAYER_VIEW_HEADROOM;
/// How far in z a walker steps from a cell to the next: the servers' and ClassicUO's `StepHeight`. Dynamapper's player
/// moves free, so no movement constant holds it. On map 0 the house stairs into the Buccaneer's Den tunnels are 5 to
/// 7 apart, so a step of 5 would show the basements; 2 shows every mountain cave found.
pub const WALK_STEP_Z: i32 = 2;
/// The cave ceiling over a shown floor: the z-view's cap for a player under a mountain (`PLAYER_VIEW_HEADROOM`).
pub const CAVE_CEILING_HEADROOM: i32 = PLAYER_VIEW_HEADROOM;

/// The eight neighbours of a cell.
const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
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

/// What a shown cave does to a static.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaveStatic {
    /// Drawn as without caves.
    Outside,
    /// Over a shown cave floor, at or above its ceiling: not drawn.
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
            .filter(|tile| tile.graphic < crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET)
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
    chunk_columns: u32,
    chunks: HashMap<u32, ChunkCells>,
}

impl MapChunks {
    /// The chunk of the cell (`x`, `y`) and the cell's index in it; `None` off the map.
    fn locate(&self, x: i32, y: i32) -> Option<(u32, usize)> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let (x, y) = (x as u32, y as u32);
        let chunk = (x / PACKAGE_CHUNK_CELL_DIM) * self.chunk_columns + y / PACKAGE_CHUNK_CELL_DIM;
        Some((chunk, ((y % PACKAGE_CHUNK_CELL_DIM) * PACKAGE_CHUNK_CELL_DIM + x % PACKAGE_CHUNK_CELL_DIM) as usize))
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
        chunk_columns,
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

/// The shown caves while `world_rendering.caves` draws them over the map.
#[derive(Resource, Default)]
pub struct CavesRes {
    /// The caves of the current map drawn now; `None` while hidden, or while they are found.
    active: Option<Arc<CaveMap>>,
    /// Counts the changes of `active`, so the caches built from it are rebuilt.
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

    /// `CaveMap::light_max_z` of the active caves; `max_z` without them.
    pub fn light_max_z(&self, x: u32, y: u32, max_z: i32) -> i32 {
        self.active().map_or(max_z, |caves| caves.light_max_z(x, y, max_z))
    }
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
    let wanted = settings.world_rendering.caves == CaveView::DrawnOverMap;
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
}
