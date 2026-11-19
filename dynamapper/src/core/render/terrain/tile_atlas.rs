// ShaderType (from `encase`) generates internal `check()` functions per field for
// alignment validation at compile time — these appear as "function `check` is never used".
#![allow(dead_code)]

//use crate::console_logger::{self, LogAbout, LogSev};
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use std::ops::RangeInclusive;

/// A simple RGBA-like 16-bit unsigned integer pair used for packing tile metadata.
/// This matches the target texture format (Rg16Uint) in the shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Rg16u {
    pub r: u16,
    pub g: u16,
}

// The old repo `policy "smooth"` (bit 0) had no client basis and no shader
// reader; edge softness comes from the TerrainDefinition `coverage_exponent` (coverage params,
// land/coverage.wesl). The old repo `policy "follow-center"` (bit 1) had no client basis either;
// water scrolls by the material's tweaks (UOWaterTerrainLayer::Draw, ec_liquid.wesl).
/// A cell's flag, not a land tile's: the land of the player's cave's mouth (caves.rs `PlayerCave::owns_land`), drawn
/// over the z-view's land cut and not darkened as outside the cave.
pub const TERRAIN_SHADER_FLAG_PLAYER_CAVE: u16 = 1 << 0;
/// Black land (`is_black_land`), and on a cell of the player's cave's mouth a cave exit frame, whatever its source
/// (`cell_terrain_flags`): drawn alone, its edge clean, no neighbour's texture
/// blended into it nor it into a neighbour (land/transitions.wesl, land/cc_transitions.wesl; nor in the EC coverage,
/// `EcTerrainParams::for_tile` for black land, land/coverage.wesl and coverage/cache.rs for the cell's flag).
pub const TERRAIN_SHADER_FLAG_NEVER_BLENDS: u16 = 1 << 1;
/// EC liquid terrain: the TerrainDefinition material is the water layer class (`UOWaterTerrainLayer`). The
/// EC wetness flag; the `is_wet` bit carries the classic IsWet tiledata flag in classic mode only.
pub const TERRAIN_SHADER_FLAG_REVIEWED_LIQUID: u16 = 1 << 2;

/// Texture source modes of a tile (bits 0-3 of the G high byte), mirrored in land/atlas.wesl. The payload (R) is
/// the texmap array layer, the land id or the texmap id, as each mode says.
/// Classic texmap in the small texmap array; payload: array layer.
pub const LAND_SOURCE_CC_SMALL: u32 = 0;
/// Classic texmap in the big texmap array; payload: array layer.
pub const LAND_SOURCE_CC_BIG: u32 = 1;
/// EC-layout terrain on the land page atlas; payload: land id.
pub const LAND_SOURCE_EC_ATLAS: u32 = 2;
/// No source has the id: the missing-texture pattern.
pub const LAND_SOURCE_MISSING: u32 = 3;
/// Classic texmap on the land page atlas; payload: texmap id.
pub const LAND_SOURCE_CC_ATLAS: u32 = 4;
/// Classic land art of land without a texmap; payload: land id.
pub const LAND_SOURCE_CLASSIC_ART: u32 = 5;
/// Classic land the Classic client does not draw (`LAND_NODRAW_ID`): the fragment is discarded; no payload.
pub const LAND_SOURCE_NODRAW: u32 = 6;
/// The land id the Classic client 7.0.103.0 never draws from its texmap or land art (tiledata "NODRAW";
/// `FUN_0044c180` and `FUN_0044fa70` skip it unless a debug flag is set). Ids 0 and 1 are drawn.
pub const LAND_NODRAW_ID: u32 = 2;
/// The land ids whose Classic texmaps are pure black: the black openings of the cave mouths (0x1AE, 0x1DB) and the
/// void (`VOID_LAND_ID`), also the black beside a dungeon's walls.
const BLACK_LAND_IDS: [u32; 3] = [0x1AE, 0x1DB, uocf::classic::maps::world::VOID_LAND_ID as u32];

/// Whether the land id drawn (a route's target) is black land, which never blends with its neighbours
/// (`TERRAIN_SHADER_FLAG_NEVER_BLENDS`): a cave mouth's opening keeps a clean edge, no dirt or rock texture fading
/// into its black nor its black into them.
pub fn is_black_land(land_id: u32) -> bool {
    BLACK_LAND_IDS.contains(&land_id)
}

/// The cave exit frames: land ids whose Classic texmaps are rock around a black opening, which the Enhanced Client
/// and Kingdom Reborn draw all black (their materials 44 and 45 "cave exit", 154 "rock hole"). The Minoc mine's mouth
/// (map 1, 2556,500) is framed by 0x1D3-0x1DA.
const CAVE_EXIT_FRAME_LAND_IDS: [RangeInclusive<u32>; 4] = [0x1D3..=0x1DA, 0x7EC..=0x7F1, 0x834..=0x839, 0x8C1..=0x8C6];

/// Whether the land id (a route's target) is a cave exit frame (`CAVE_EXIT_FRAME_LAND_IDS`).
pub fn is_cave_exit_frame(land_id: u32) -> bool {
    CAVE_EXIT_FRAME_LAND_IDS.iter().any(|frames| frames.contains(&land_id))
}

/// The terrain flags of a cell drawing land whose tile flags are `tile_flags`: `cave_exit_frame` when the land is a
/// cave exit frame, from any source, `player_cave_mouth` when the cell's land is the player's cave's mouth (caves.rs
/// `PlayerCave::owns_land`). The mouth's land takes `TERRAIN_SHADER_FLAG_PLAYER_CAVE`; a frame there never blends, so
/// in the player's cave the dirt below a steep frame, as at the Minoc mine's mouth, does not stretch up its black, and
/// the cave mouth veil draws it (terrain/cave_mouth_veil.rs), its black see-through. Elsewhere the frame blends, the black
/// of the opening fading softly into the rock around it as seen from outside.
pub fn cell_terrain_flags(tile_flags: u16, cave_exit_frame: bool, player_cave_mouth: bool) -> u16 {
    if !player_cave_mouth {
        return tile_flags;
    }
    let never_blends = if cave_exit_frame { TERRAIN_SHADER_FLAG_NEVER_BLENDS } else { 0 };
    tile_flags | TERRAIN_SHADER_FLAG_PLAYER_CAVE | never_blends
}
/// Where `Rg16u::pack` stores the texture source mode (`tex_size_bits`) in G: its high byte's low nibble.
const TEX_SOURCE_SHIFT: u16 = 8;
const TEX_SOURCE_MASK: u16 = 0x0F;
/// Where `Rg16u::pack` stores the land flags (`TERRAIN_SHADER_FLAG_*`) in G: bits 4-6 of its high byte.
const TERRAIN_FLAGS_SHIFT: u16 = 12;
const TERRAIN_FLAGS_MASK: u16 = 0x07;

impl Rg16u {
    /// Packs tile ID, height (Z), texture source mode, land flags, and is_wet into Rg16u.
    ///
    /// G channel high-byte layout (mirrored in atlas.wesl):
    ///   bits 0-3: `tex_size_bits` (texture source mode, `LAND_SOURCE_*`: 0=cc-small, 1=cc-big, 2=ec-atlas,
    ///             3=missing, 4=cc-atlas, 5=classic land art)
    ///   bits 4-6: `land_flags` (EC reviewed land policy/liquid hints, and the player's cave's mouth)
    ///   bit  7:   `is_wet` (classic mode: the IsWet tiledata flag → animated water effect)
    ///
    /// This packing must be manually unrolled in the WGSL shader logic.
    pub fn pack(
        tile_id: u16,
        height_i8: i8,
        tex_size_bits: u16,
        is_wet: bool,
        land_flags: u16,
    ) -> Self {
        // g: low 8 bits: height_i8 + 128
        // g: high 8 bits: lower nibble = tex_size_bits, bits 4-6 = land flags, bit 7 = is_wet
        let height_biased = (height_i8 as i16 + 128).clamp(0, 255) as u8;
        let wet_bit: u16 = if is_wet { 0x80 } else { 0 };
        let flags = (land_flags & TERRAIN_FLAGS_MASK) << 4;
        let g = (height_biased as u16) | (((tex_size_bits & 0x0F) | flags | wet_bit) << 8);
        Self { r: tile_id, g }
    }

    /// The texture source mode (`LAND_SOURCE_*`) `pack` stored.
    pub fn texture_source(self) -> u32 {
        u32::from((self.g >> TEX_SOURCE_SHIFT) & TEX_SOURCE_MASK)
    }

    /// The land flags (`TERRAIN_SHADER_FLAG_*`) `pack` stored.
    pub fn terrain_flags(self) -> u16 {
        (self.g >> TERRAIN_FLAGS_SHIFT) & TERRAIN_FLAGS_MASK
    }
}

/// Uniform parameters passed to the land shader to resolve world coordinates into atlas samples.
/// This struct must be kept in sync with the shader's `AtlasParams` (including std140/std430 alignment).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, PartialEq)]
pub struct AtlasParams {
    /// Dimension of a single page in texels.
    pub page_texels: UVec2,
    /// Number of tiles per page (e.g., 8x8 or 2048x2048).
    pub tiles_per_page: UVec2,
    /// Maximum number of physical layers available in the texture array.
    pub max_layers: u32,
    /// Number of world pages along the X axis, used for flat index calculation.
    pub world_pages_x: u32,
    pub _pad: UVec2,
    /// A flattened array mapping page indices to physical layer indices.
    /// Each u32 stores the layer index, or u32::MAX if not mapped.
    pub page_to_layer: [bevy::math::UVec4; 64], // Stores mapping for up to 256 pages!
}

impl Default for AtlasParams {
    fn default() -> Self {
        Self {
            page_texels: UVec2::ZERO,
            tiles_per_page: UVec2::ZERO,
            max_layers: 0,
            world_pages_x: 0,
            _pad: UVec2::ZERO,
            page_to_layer: [bevy::math::UVec4::MAX; 64],
        }
    }
}

use bevy::render::extract_resource::ExtractResource;

#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct TileAtlasImageHandle(pub Handle<Image>);

#[derive(Clone)]
pub struct AtlasUpload {
    pub layer: u32,
    pub offset: UVec2,
    pub size: UVec2,
    pub data: Vec<u8>,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct LayerDirtyRegion {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl LayerDirtyRegion {
    pub fn from_offset_size(offset: UVec2, size: UVec2) -> Self {
        Self {
            min_x: offset.x,
            min_y: offset.y,
            max_x: offset.x + size.x,
            max_y: offset.y + size.y,
        }
    }

    pub fn width(&self) -> u32 {
        self.max_x.saturating_sub(self.min_x)
    }

    pub fn height(&self) -> u32 {
        self.max_y.saturating_sub(self.min_y)
    }

    pub fn area(&self) -> u32 {
        self.width().saturating_mul(self.height())
    }

    pub fn merged(&self, other: &Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    pub fn touches_or_overlaps(&self, other: &Self) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    pub fn should_merge(&self, other: &Self) -> bool {
        if !self.touches_or_overlaps(other) {
            return false;
        }

        // Merge when the combined bounding box does not introduce a gap.
        // This keeps straightforward horizontal/vertical adjacency collapsed
        // without turning distant L-shaped edits into oversized uploads.
        let merged = self.merged(other);
        merged.area() <= self.area().saturating_add(other.area())
    }
}

fn coalesce_dirty_regions(regions: &mut Vec<LayerDirtyRegion>) {
    let mut i = 0usize;
    while i < regions.len() {
        let mut merged_any = false;
        let mut j = i + 1;
        while j < regions.len() {
            if regions[i].should_merge(&regions[j]) {
                let merged = regions[i].merged(&regions[j]);
                regions[i] = merged;
                regions.swap_remove(j);
                merged_any = true;
            } else {
                j += 1;
            }
        }

        if !merged_any {
            i += 1;
        }
    }
}

/// How many changed cell rectangles `TileAtlas::changed_cells_after` keeps: a consumer further behind treats every
/// cell as changed. A frame of land loading writes one rectangle per 8x8 block, far fewer than this.
const CHANGED_CELLS_LOG_CAPACITY: usize = 8192;

/// A rectangle of world cells, both corners included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasCells {
    pub min: UVec2,
    pub max: UVec2,
}

impl AtlasCells {
    /// The cells from `min` to `max`, both included.
    pub fn new(min: UVec2, max: UVec2) -> Self {
        Self { min: min.min(max), max: min.max(max) }
    }

    /// The `size` cells from `origin`; `size` is at least one cell along each axis.
    fn from_origin_size(origin: UVec2, size: UVec2) -> Self {
        Self::new(origin, origin + size.max(UVec2::ONE) - UVec2::ONE)
    }

    /// These cells grown to hold `other` too.
    pub fn union(self, other: Self) -> Self {
        Self { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }
}

/// Resource managing the paged metadata atlas.
/// It maintains a CPU-side cache and tracks pending uploads to the GPU.
#[derive(Resource)]
pub struct TileAtlas {
    /// Parameters shared with the GPU.
    pub params: AtlasParams,
    /// LRU Cache: Maps logical Page Coordinate (packed u64) to physical Layer Index (u32).
    /// Using a small Vec with linear search is faster than HashMap for the typical number of layers.
    /// mapping: page_u64 -> layer_idx. Sorted by page_u64 for binary search.
    page_to_layer: Vec<(u64, u32)>,
    /// Reverse mapping for eviction logic. Indexed by layer.
    layer_to_page: Vec<IVec2>,
    /// Tracks the 'last used' tick for each layer for LRU eviction. Indexed by layer.
    layer_access_tick: Vec<u64>,
    /// Monotonically increasing counter for LRU tracking.
    current_tick: u64,

    /// CPU mirror of mapped layer data. Indexed by layer.
    pub cpu_mirror: Vec<Option<Vec<Rg16u>>>,
    /// Invalidates CPU consumers whose cached geometry depends on map data.
    pub content_revision: u64,
    /// The world cells whose `read_cell` may have changed, each with the `content_revision` it changed at, oldest first
    /// (`changed_cells_after`), at most `CHANGED_CELLS_LOG_CAPACITY` of them.
    changed_cells: std::collections::VecDeque<(u64, AtlasCells)>,
    /// The revision `changed_cells` holds every change after: an older one dropped out of the log, or
    /// `clear_all_mappings` changed every cell.
    changed_cells_since: u64,
    /// Tracks modified regions per layer.
    ///
    /// We keep multiple rectangles and coalesce only overlapping or edge-sharing ones so the
    /// uploader avoids sending one oversized bounding box for sparse edits on the same layer.
    pub dirty_regions: Vec<Vec<LayerDirtyRegion>>,
    /// Number of changed 8x8 block writes accumulated since the last staging pass.
    pub pending_dirty_write_count: u32,
    /// Staging buffer swapped into place by the clear system so that
    /// the extract system can take ownership without cloning.
    extract_staging: Vec<AtlasUpload>,
    extract_revision: u64,
    extract_image: Option<bevy::asset::AssetId<Image>>,

    /// When set, `sys_apply_tile_atlas_expansion` will create a new GPU image
    /// with this many layers, then clear all page mappings so they are
    /// re-populated by the mesh renderer on subsequent frames.
    pub requested_expansion: Option<u32>,
    /// Hard upper limit for atlas layers (from `TILE_ATLAS_MAX_LAYERS`).
    pub max_layers_limit: u32,
    /// Timestamp of the last time the atlas was at high utilisation (> 75%).
    /// Used by the shrink heuristic to avoid premature downsizing.
    pub last_high_usage_instant: std::time::Instant,
}

impl TileAtlas {
    pub(crate) fn read_cell(&self, x: u32, y: u32) -> Option<Rg16u> {
        let p = &self.params;
        if p.page_texels.x == 0 || p.page_texels.y == 0 { return None; }
        let index = (y / p.page_texels.y) * p.world_pages_x + x / p.page_texels.x;
        if index >= 256 { return None; }
        let layer = p.page_to_layer[(index / 4) as usize][(index % 4) as usize];
        let data = self.cpu_mirror.get(layer as usize)?.as_ref()?;
        data.get(((y % p.page_texels.y) * p.page_texels.x + x % p.page_texels.x) as usize).copied()
    }

    /// Whether the land of the cell (`x`, `y`) is land the renderer does not draw, as the Classic client does not
    /// (`LAND_SOURCE_NODRAW`); false while the atlas does not hold the cell.
    pub(crate) fn land_is_nodraw(&self, x: u32, y: u32) -> bool {
        self.read_cell(x, y).is_some_and(|cell| cell.texture_source() == LAND_SOURCE_NODRAW)
    }

    /// The cells whose `read_cell` may have changed after `content_revision` was `revision`, so a consumer that read cells
    /// at that revision rebuilds only what read them; `None` when every cell may have changed (the log no longer
    /// reaches back to `revision`, or the mappings were cleared since).
    pub(crate) fn changed_cells_after(&self, revision: u64) -> Option<impl Iterator<Item = AtlasCells> + '_> {
        (revision >= self.changed_cells_since).then(|| self.changed_cells.iter()
            .skip_while(move |(changed, _)| *changed <= revision).map(|(_, cells)| *cells))
    }

    /// Bumps `content_revision` for a change of `cells`.
    fn record_changed_cells(&mut self, cells: AtlasCells) {
        self.content_revision = self.content_revision.wrapping_add(1);
        if self.changed_cells.len() == CHANGED_CELLS_LOG_CAPACITY {
            if let Some((dropped, _)) = self.changed_cells.pop_front() {
                self.changed_cells_since = dropped;
            }
        }
        self.changed_cells.push_back((self.content_revision, cells));
    }

    /// The world cells of `page`.
    fn page_cells(&self, page: IVec2) -> AtlasCells {
        AtlasCells::from_origin_size(page.as_uvec2() * self.params.page_texels, self.params.page_texels)
    }

    pub(crate) fn has_pending_uploads(&self) -> bool {
        self.pending_dirty_write_count != 0 || !self.extract_staging.is_empty()
    }

    pub fn new(params: AtlasParams, max_layers_limit: u32) -> Self {
        let max_layers = params.max_layers as usize;
        Self {
            params,
            page_to_layer: Vec::with_capacity(max_layers),
            layer_to_page: vec![IVec2::ZERO; max_layers],
            layer_access_tick: vec![0; max_layers],
            current_tick: 0,
            cpu_mirror: vec![None; max_layers],
            content_revision: 0,
            changed_cells: std::collections::VecDeque::new(),
            changed_cells_since: 0,
            dirty_regions: vec![Vec::new(); max_layers],
            pending_dirty_write_count: 0,
            extract_staging: Vec::new(),
            extract_revision: 0,
            extract_image: None,
            requested_expansion: None,
            max_layers_limit,
            last_high_usage_instant: std::time::Instant::now(),
        }
    }

    /// Assigns or retrieves a physical layer index for a given logical world page.
    /// If no layers are free, it evicts the least recently used (LRU) page.
    /// Returns the layer index and the optionally evicted page coordinate.
    pub fn ensure_layer_for_page(&mut self, page: IVec2) -> (u32, Option<IVec2>) {
        self.current_tick += 1;
        let page_u64 = (page.x as u32 as u64) | ((page.y as u32 as u64) << 32);

        // If it's already in the cache, return it
        if let Ok(pos) = self
            .page_to_layer
            .binary_search_by_key(&page_u64, |(p, _)| *p)
        {
            let layer = self.page_to_layer[pos].1;
            self.layer_access_tick[layer as usize] = self.current_tick;
            return (layer, None);
        }

        // Need to allocate a new layer
        let mut evicted_page = None;
        let layer = if (self.page_to_layer.len() as u32) < self.params.max_layers {
            // Unused layer available
            self.page_to_layer.len() as u32
        } else {
            // All layers occupied — request expansion so the next frame has
            // more headroom, then evict the LRU page for this frame.
            if self.params.max_layers < self.max_layers_limit {
                let target = (self.params.max_layers * 2).min(self.max_layers_limit);
                if self.requested_expansion.is_none_or(|r| target > r) {
                    self.requested_expansion = Some(target);
                }
            }

            // Evict least recently used layer
            let (lru_layer, _) = self
                .layer_access_tick
                .iter()
                .enumerate()
                .min_by_key(|&(_, tick)| tick)
                .expect("At least one layer must exist");
            let lru_layer = lru_layer as u32;

            let old_page = self.layer_to_page[lru_layer as usize];
            let old_page_u64 = (old_page.x as u32 as u64) | ((old_page.y as u32 as u64) << 32);
            if let Ok(pos) = self
                .page_to_layer
                .binary_search_by_key(&old_page_u64, |(p, _)| *p)
            {
                self.page_to_layer.remove(pos);
            }
            evicted_page = Some(old_page);
            lru_layer
        };

        // Insert new entry and keep sorted
        let insert_pos = self
            .page_to_layer
            .binary_search_by_key(&page_u64, |(p, _)| *p)
            .unwrap_err();
        self.page_to_layer.insert(insert_pos, (page_u64, layer));
        // The layer still holds the evicted page's cells until the new page's are written.
        self.record_changed_cells(self.page_cells(page));
        if let Some(old_page) = evicted_page {
            self.record_changed_cells(self.page_cells(old_page));
        }
        self.layer_to_page[layer as usize] = page;
        self.layer_access_tick[layer as usize] = self.current_tick;

        if let Some(old_page) = evicted_page {
            let evicted_page_index =
                (old_page.y as u32) * self.params.world_pages_x + (old_page.x as u32);
            if evicted_page_index < 256 {
                let idx = (evicted_page_index / 4) as usize;
                let comp = evicted_page_index % 4;
                let mut arr = self.params.page_to_layer[idx].to_array();
                arr[comp as usize] = u32::MAX;
                self.params.page_to_layer[idx] = bevy::math::UVec4::from_array(arr);
            }
        }

        let page_index = (page.y as u32) * self.params.world_pages_x + (page.x as u32);
        if page_index < 256 {
            let idx = (page_index / 4) as usize;
            let comp = page_index % 4;
            let mut arr = self.params.page_to_layer[idx].to_array();
            arr[comp as usize] = layer;
            self.params.page_to_layer[idx] = bevy::math::UVec4::from_array(arr);
        }

        // Track high utilisation for the shrink heuristic.
        let usage_ratio = self.page_to_layer.len() as f32 / self.params.max_layers.max(1) as f32;
        if usage_ratio > 0.75 {
            self.last_high_usage_instant = std::time::Instant::now();
        }

        (layer, evicted_page)
    }

    pub fn enqueue_rg16u_block(
        &mut self,
        layer: u32,
        offset: UVec2,
        size: UVec2,
        texels: &[Rg16u],
    ) {
        let page_width = self.params.page_texels.x;
        let layer_idx = layer as usize;

        if self.cpu_mirror[layer_idx].is_none() {
            self.cpu_mirror[layer_idx] = Some(vec![
                Rg16u { r: 0, g: 0 };
                (page_width * self.params.page_texels.y)
                    as usize
            ]);
        }
        let mirror = self.cpu_mirror[layer_idx].as_mut().unwrap();

        // Compare before writing: skip the GPU upload if the data is unchanged.
        // PendingTextureBake chunks re-enqueue every frame until all textures
        // are resident; without this check each re-enqueue marks the region
        // dirty and triggers a redundant write_texture to the GPU.
        let mut any_changed = false;
        for y in 0..size.y {
            let src_start = (y * size.x) as usize;
            let src_end = src_start + size.x as usize;
            let dst_start = ((offset.y + y) * page_width + offset.x) as usize;
            let dst_end = dst_start + size.x as usize;
            if mirror[dst_start..dst_end] != texels[src_start..src_end] {
                mirror[dst_start..dst_end].copy_from_slice(&texels[src_start..src_end]);
                any_changed = true;
            }
        }

        if !any_changed {
            return;
        }
        let page = self.layer_to_page[layer_idx].as_uvec2();
        self.record_changed_cells(AtlasCells::from_origin_size(page * self.params.page_texels + offset, size));

        self.pending_dirty_write_count = self.pending_dirty_write_count.saturating_add(1);

        let regions = &mut self.dirty_regions[layer_idx];
        regions.push(LayerDirtyRegion::from_offset_size(offset, size));
        coalesce_dirty_regions(regions);
    }

    /// Number of currently mapped pages.
    pub fn mapped_page_count(&self) -> u32 {
        self.page_to_layer.len() as u32
    }

    /// Clears all page ↔ layer associations and resets the LRU state.
    /// Called after the atlas GPU image is replaced so mappings are rebuilt
    /// by the mesh renderer.
    pub fn clear_all_mappings(&mut self) {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.changed_cells.clear();
        self.changed_cells_since = self.content_revision;
        self.page_to_layer.clear();
        self.layer_to_page.fill(IVec2::ZERO);
        self.layer_access_tick.fill(0);
        self.current_tick = 0;
        self.params.page_to_layer = [bevy::math::UVec4::MAX; 64];
        self.cpu_mirror.fill(None);
        for regions in &mut self.dirty_regions {
            regions.clear();
        }
        self.pending_dirty_write_count = 0;
    }

    /// Applies a new layer count after the GPU image has been replaced.
    pub fn apply_resize(&mut self, new_max_layers: u32) {
        self.params.max_layers = new_max_layers;
        self.requested_expansion = None;

        let new_len = new_max_layers as usize;
        self.layer_to_page.resize(new_len, IVec2::ZERO);
        self.layer_access_tick.resize(new_len, 0);
        self.cpu_mirror.resize(new_len, None);
        self.dirty_regions.resize_with(new_len, Vec::new);

        self.clear_all_mappings();
    }
}

use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;

/// Middle-man resource that holds uploaded data during the transition from Main world to Render world.
/// Pending writes and the latest CPU revision captured with their snapshot.
#[derive(Resource, Default)]
pub struct RenderAtlasUploads {
    pending: Vec<AtlasUpload>,
    revision: Option<u64>,
    image: Option<bevy::asset::AssetId<Image>>,
}

impl RenderAtlasUploads {
    fn capture(&mut self, atlas: &TileAtlas, image: bevy::asset::AssetId<Image>) {
        if self.image != Some(image) {
            // A resized atlas has new layers. Never send its predecessor's
            // rectangles to the replacement, including this frame's staging.
            self.pending.clear();
            self.revision = None;
            self.image = Some(image);
        }
        if atlas.extract_image != Some(image) { return; }
        // First stages the previous Update's writes. The current CPU revision
        // may already include newer writes that have not been staged yet.
        self.revision = Some(atlas.extract_revision);
        self.pending.extend_from_slice(&atlas.extract_staging);
    }
}

/// System that extracts pending uploads from the `TileAtlas` resource in the Main world
/// and moves them into the `RenderAtlasUploads` resource in the Render world.
pub fn sys_extract_atlas_uploads(
    tile_atlas: Extract<Res<TileAtlas>>,
    atlas_image: Extract<Res<TileAtlasImageHandle>>,
    mut render_uploads: ResMut<RenderAtlasUploads>,
    diagnostics: Res<crate::diagnostics::WorldmapCrossAppDiagnostics>,
) {
    let _trace_span = crate::tracy_span!("worldmap::atlas_extract_uploads");
    let _timer = crate::diagnostics::scoped_worldmap_cross_app_timer(
        &diagnostics,
        crate::diagnostics::WorldmapCrossAppTimedSystem::AtlasExtract,
    );
    render_uploads.capture(&tile_atlas, (**atlas_image).0.id());
}

pub fn sys_clear_atlas_uploads(
    mut tile_atlas: ResMut<TileAtlas>,
    atlas_image: Res<TileAtlasImageHandle>,
    telemetry: Res<super::LandUploadTelemetry>,
    diagnostics: Res<crate::diagnostics::WorldmapCrossAppDiagnostics>,
) {
    let _trace_span = crate::tracy_span!("worldmap::atlas_stage_uploads");
    let _timer = crate::diagnostics::scoped_worldmap_cross_app_timer(
        &diagnostics,
        crate::diagnostics::WorldmapCrossAppTimedSystem::AtlasStage,
    );

    tile_atlas.extract_revision = tile_atlas.content_revision;
    tile_atlas.extract_image = Some(atlas_image.0.id());
    let TileAtlas {
        params,
        cpu_mirror,
        dirty_regions,
        pending_dirty_write_count,
        extract_staging,
        ..
    } = &mut *tile_atlas;

    let mut extracted_uploads = std::mem::take(extract_staging);
    extracted_uploads.clear();

    let page_width = params.page_texels.x;
    let dirty_block_updates = *pending_dirty_write_count;
    let mut queued_bytes = 0u64;

    for (layer, regions) in dirty_regions.iter_mut().enumerate() {
        if regions.is_empty() {
            continue;
        }
        let layer = layer as u32;

        coalesce_dirty_regions(regions);

        let mirror = cpu_mirror[layer as usize].as_ref().unwrap();

        for region in regions.iter() {
            let width = region.max_x.saturating_sub(region.min_x);
            let height = region.max_y.saturating_sub(region.min_y);
            if width == 0 || height == 0 {
                continue;
            }

            let size_bytes = (width * height * 4) as usize;
            let mut data = Vec::with_capacity(size_bytes);

            for y in region.min_y..region.max_y {
                let start = (y * page_width + region.min_x) as usize;
                let end = start + width as usize;
                let row_slice: &[u8] = bytemuck::cast_slice(&mirror[start..end]);
                data.extend_from_slice(row_slice);
            }

            queued_bytes = queued_bytes.saturating_add(data.len() as u64);
            extracted_uploads.push(AtlasUpload {
                layer,
                offset: UVec2::new(region.min_x, region.min_y),
                size: UVec2::new(width, height),
                data,
            });
        }

        regions.clear();
    }

    *pending_dirty_write_count = 0;
    *extract_staging = extracted_uploads;
    telemetry.record_stage(
        dirty_block_updates,
        extract_staging.len() as u32,
        queued_bytes,
    );
}

/// System running in the Render world that drains `RenderAtlasUploads` and issues
/// `write_texture` commands to the GPU queue to update the metadata atlas.
pub fn sys_render_upload_tile_atlas(
    mut uploads: ResMut<RenderAtlasUploads>,
    atlas_handle: Res<TileAtlasImageHandle>,
    upload_budget: Res<super::LandUploadBudget>,
    telemetry: Res<super::LandUploadTelemetry>,
    diagnostics: Res<crate::diagnostics::WorldmapCrossAppDiagnostics>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    let _trace_span = crate::tracy_span!("worldmap::atlas_render_uploads");
    let _timer = crate::diagnostics::scoped_worldmap_cross_app_timer(
        &diagnostics,
        crate::diagnostics::WorldmapCrossAppTimedSystem::AtlasUpload,
    );

    let gpu_image = gpu_images.get(&atlas_handle.0)
        .filter(|_| uploads.image == Some(atlas_handle.0.id()));
    let Some(gpu_image) = gpu_image else {
        // Asset preparation may lag extraction. Retain the queued data and do
        // not publish its revision until every write can actually be issued.
        let bytes = uploads.pending.iter().map(|upload| upload.data.len() as u64).sum();
        telemetry.record_submit(0, 0, uploads.pending.len() as u32, bytes);
        return;
    };
    if uploads.pending.is_empty() {
        telemetry.record_submit(0, 0, 0, 0);
        if let Some(revision) = uploads.revision { telemetry.record_metadata_submission(revision); }
        return;
    }

    use bevy::render::render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo};

    // Apply explicit per-frame upload caps so metadata-atlas updates can spill over to
    // later frames instead of forcing a single large Queue pass after zoom-outs or teleports.
    //
    // We deliberately allow the first upload through even if it exceeds the byte limit.
    // That avoids deadlocking on a single large dirty region when the configured budget is
    // lower than the size of that region.
    let max_ops = upload_budget.effective_upload_max_ops_per_frame();
    let max_bytes = upload_budget.effective_upload_max_bytes_per_frame();
    let mut submitted_ops = 0usize;
    let mut submitted_bytes = 0usize;
    let mut submitted_uploads = 0usize;

    for upload in uploads.pending.iter() {
        let upload_bytes = upload.data.len();
        let ops_would_overflow = submitted_ops >= max_ops;
        let bytes_would_overflow = submitted_bytes.saturating_add(upload_bytes) > max_bytes;

        if submitted_uploads > 0 && (ops_would_overflow || bytes_would_overflow) {
            break;
        }

        let destination = TexelCopyTextureInfo {
            texture: &*gpu_image.texture,
            mip_level: 0,
            origin: Origin3d {
                x: upload.offset.x,
                y: upload.offset.y,
                z: upload.layer,
            },
            aspect: bevy::render::render_resource::TextureAspect::All,
        };

        let data_layout = TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(upload.size.x * 4),
            rows_per_image: Some(upload.size.y),
        };

        let extent = Extent3d {
            width: upload.size.x,
            height: upload.size.y,
            depth_or_array_layers: 1,
        };

        render_queue.write_texture(destination, &upload.data, data_layout, extent);

        submitted_ops += 1;
        submitted_bytes = submitted_bytes.saturating_add(upload_bytes);
        submitted_uploads += 1;
    }

    if submitted_uploads > 0 {
        uploads.pending.drain(0..submitted_uploads);
    }
    if uploads.pending.is_empty() {
        if let Some(revision) = uploads.revision { telemetry.record_metadata_submission(revision); }
    }

    let pending_bytes = uploads
        .pending
        .iter()
        .fold(0u64, |acc, upload| acc.saturating_add(upload.data.len() as u64));
    telemetry.record_submit(
        submitted_ops as u32,
        submitted_bytes as u64,
        uploads.pending.len() as u32,
        pending_bytes,
    );
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    fn cells(min: (u32, u32), max: (u32, u32)) -> AtlasCells {
        AtlasCells::new(UVec2::new(min.0, min.1), UVec2::new(max.0, max.1))
    }

    fn changed_after(atlas: &TileAtlas, revision: u64) -> Option<Vec<AtlasCells>> {
        atlas.changed_cells_after(revision).map(Iterator::collect)
    }

    #[test]
    fn the_atlas_names_the_cells_changed_since_a_revision() {
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(16), max_layers: 1, world_pages_x: 4,
            ..Default::default() }, 1);
        let start = atlas.content_revision;
        // Mapping a page changes its cells: they read the layer's former contents until written.
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::new(1, 0));
        assert_eq!(changed_after(&atlas, start), Some(vec![cells((16, 0), (31, 15))]));
        let mapped = atlas.content_revision;
        atlas.enqueue_rg16u_block(layer, UVec2::new(8, 0), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(changed_after(&atlas, mapped), Some(vec![cells((24, 0), (31, 7))]));
        // An identical write changes nothing.
        let written = atlas.content_revision;
        atlas.enqueue_rg16u_block(layer, UVec2::new(8, 0), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(changed_after(&atlas, written), Some(Vec::new()));
        assert_eq!(changed_after(&atlas, start).map(|changed| changed.len()), Some(2));
        // Evicting the page for another changes both.
        atlas.ensure_layer_for_page(IVec2::new(0, 1));
        assert_eq!(changed_after(&atlas, written), Some(vec![cells((0, 16), (15, 31)), cells((16, 0), (31, 15))]));
        // Cleared mappings change every cell, for whoever read before.
        let before_clear = atlas.content_revision;
        atlas.clear_all_mappings();
        assert_eq!(changed_after(&atlas, before_clear), None);
        assert_eq!(changed_after(&atlas, atlas.content_revision), Some(Vec::new()));
    }

    #[test]
    fn a_reader_behind_the_change_log_takes_every_cell_as_changed() {
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(16), max_layers: 1, world_pages_x: 1,
            ..Default::default() }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        let start = atlas.content_revision;
        for value in 0..=CHANGED_CELLS_LOG_CAPACITY as u16 {
            atlas.enqueue_rg16u_block(layer, UVec2::ZERO, UVec2::ONE, &[Rg16u { r: value + 1, g: 0 }]);
        }
        assert_eq!(changed_after(&atlas, start), None);
        assert_eq!(changed_after(&atlas, atlas.content_revision - 1), Some(vec![cells((0, 0), (0, 0))]));
    }

    #[test]
    fn uploads_follow_the_staged_revision_and_never_cross_image_generations() {
        let first = bevy::asset::AssetId::Uuid { uuid: bevy::asset::uuid::Uuid::from_u128(1) };
        let second = bevy::asset::AssetId::Uuid { uuid: bevy::asset::uuid::Uuid::from_u128(2) };
        let mut atlas = TileAtlas::new(AtlasParams::default(), 1);
        let upload = |value| AtlasUpload { layer: 0, offset: UVec2::ZERO, size: UVec2::ONE, data: vec![value; 4] };
        atlas.extract_image = Some(first);
        atlas.extract_revision = 3;
        atlas.content_revision = 4; // A newer Update has not reached staging.
        atlas.extract_staging.push(upload(3));
        let mut uploads = RenderAtlasUploads::default();
        uploads.capture(&atlas, first);
        assert_eq!(uploads.revision, Some(3));
        atlas.extract_revision = 4;
        atlas.extract_staging = vec![upload(4)];
        uploads.capture(&atlas, first);
        assert_eq!(uploads.revision, Some(4));
        assert_eq!(uploads.pending.len(), 2); // Keep the earlier budget backlog.
        uploads.capture(&atlas, second); // Resize between First and extraction.
        assert!(uploads.pending.is_empty());
        assert_eq!(uploads.revision, None);
        atlas.extract_image = Some(second);
        atlas.extract_revision = 5;
        atlas.extract_staging = vec![upload(5)];
        uploads.capture(&atlas, second);
        assert_eq!(uploads.revision, Some(5));
        assert_eq!(uploads.pending.len(), 1);
        assert_eq!(uploads.pending[0].data, vec![5; 4]);
    }
}

#[cfg(test)]
mod black_land_tests {
    use super::*;

    /// The cave mouth's black opening and the void never blend; the dirt and rock beside them, and the white-texmap
    /// void id 1, do.
    #[test]
    fn only_black_land_never_blends() {
        const MOUTH_ROCK_HOLE: u32 = 0x1DB;
        const MOUTH_BLACK: u32 = 0x1AE;
        const MOUTH_ROCK: u32 = 0x1DA;
        const DIRT: u32 = 0x72;
        const WHITE_VOID: u32 = 1;
        for id in [MOUTH_ROCK_HOLE, MOUTH_BLACK, uocf::classic::maps::world::VOID_LAND_ID as u32] {
            assert!(is_black_land(id), "{id:#x}");
        }
        for id in [MOUTH_ROCK, DIRT, WHITE_VOID, LAND_NODRAW_ID] {
            assert!(!is_black_land(id), "{id:#x}");
        }
    }

    /// The cave exit frames are not black land: drawn from any source, they blend outside the player's cave.
    #[test]
    fn cave_exit_frames_are_not_black_land() {
        const MINOC_MOUTH_FRAMES: [u32; 2] = [0x1D3, 0x1DA];
        const CAVE_EXITS: [u32; 2] = [0x7EC, 0x839];
        const NO_NAME_FRAMES: [u32; 2] = [0x8C1, 0x8C6];
        const MOUTH_ROCK_HOLE: u32 = 0x1DB;
        const MOUTH_DIRT: u32 = 0x74;
        const ROCK: u32 = 0x22C;
        const BEFORE_FRAMES: u32 = 0x1D2;
        for id in MINOC_MOUTH_FRAMES.into_iter().chain(CAVE_EXITS).chain(NO_NAME_FRAMES) {
            assert!(is_cave_exit_frame(id) && !is_black_land(id), "{id:#x}");
        }
        for id in [MOUTH_ROCK_HOLE, MOUTH_DIRT, ROCK, BEFORE_FRAMES, LAND_NODRAW_ID] {
            assert!(!is_cave_exit_frame(id), "{id:#x}");
        }
    }

    /// A black Enhanced or Kingdom Reborn cave exit frame never blends on the player's cave's mouth only; the frame
    /// drawn from its Classic texmap, and the land beside it there, blend; black land never blends anywhere.
    #[test]
    fn cave_exit_frames_never_blend_only_on_the_player_caves_mouth() {
        const LIQUID: u16 = TERRAIN_SHADER_FLAG_REVIEWED_LIQUID;
        const MOUTH: u16 = TERRAIN_SHADER_FLAG_PLAYER_CAVE;
        const NEVER: u16 = TERRAIN_SHADER_FLAG_NEVER_BLENDS;
        assert_eq!(cell_terrain_flags(0, true, true), MOUTH | NEVER);
        assert_eq!(cell_terrain_flags(0, true, false), 0);
        assert_eq!(cell_terrain_flags(0, false, true), MOUTH);
        assert_eq!(cell_terrain_flags(LIQUID, false, false), LIQUID);
        assert_eq!(cell_terrain_flags(NEVER, false, false), NEVER);
        assert_eq!(cell_terrain_flags(NEVER, false, true), NEVER | MOUTH);
    }
}
