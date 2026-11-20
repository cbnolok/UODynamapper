//! What static collection hands the draw: the sprite and ground instances with their flags and payloads, the
//! render and pick tiles, and the per-chunk batches.

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bytemuck::{Pod, Zeroable};

// The KR client's static shadow (research P3.4; art/client_shadow.wesl). `SpriteInstance::client_shadow_flags` bits,
// kept in sync with client_shadow.wesl.
pub(crate) const CLIENT_SHADOW_FLAG_CASTS: u32 = 1;
pub(crate) const CLIENT_SHADOW_FLAG_WALL: u32 = 1 << 1;
/// `SpriteInstance::is_wet_flags` bit: the upright card shows flat art, a CC ground diamond above the terrain (a floor
/// or a flat roof, `cc_ground_diamond`), which catches the static local lights evenly. Kept in sync with
/// art/shading.wesl.
pub(crate) const STATIC_FLAG_FLAT_ART: u32 = 1 << 11;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is part of a cave drawn over the map
/// (`CaveStatic::Lifted`), drawn in front of the land and statics around it. Kept in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAVE: u32 = 1 << 12;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is outside the player's cave, drawn
/// darkened and partly desaturated (art/depth.wesl `cave_outside_tone`, caves.rs `CaveSurroundings::Dimmed`). Kept
/// in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAVE_OUTSIDE: u32 = 1 << 15;
/// `SpriteInstance::is_wet_flags` bit: the static is a wall with a floor laid on its top in the farther row, a Surface
/// or Bridge in the cell west or north of it whose top is the wall's (`wall_cap_meets_floor_flag`): the perspective
/// camera orders its cap in that row (art/depth.wesl `perspective_cap_row_shift`). Kept in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAP_MEETS_FLOOR: u32 = 1 << 24;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static continues the land
/// (`static_continues_land`) or is foliage, so it takes the land's colour grading, KR-like's chroma knee included; the
/// other statics keep their own colour (`land_grade_flag`). Kept in sync with art/shading.wesl.
pub(crate) const STATIC_FLAG_LAND_GRADE: u32 = 1 << 25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StaticRenderTile {
    pub graphic: u16,
    pub world_x: f32,
    pub world_z: f32,
    pub z: i8,
    pub hue: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StaticPickTile {
    pub tile: StaticRenderTile,
    pub is_wet: bool,
}

pub(super) const STATIC_HUE_FLAG_APPLY: u32 = 1;
pub(crate) const STATIC_HUE_FLAG_EC_SPRITE: u32 = 2;
pub(super) fn static_hue_payload(hue_id: u16) -> (u32, u32) {
    if hue_id == 0 {
        (0, 0)
    } else {
        (u32::from(hue_id), STATIC_HUE_FLAG_APPLY)
    }
}

pub(crate) fn static_sprite_opacity(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    legacy_art: bool) -> f32 {
    use uocf::enhanced::tileart::TaeFlag;
    if !legacy_art { return 1.0; }
    let Some(tile) = tilemeta else { return 1.0; };
    if tile.flags & TaeFlag::Translucent.bits() == 0 { return 1.0; }
    // Original CC art dispatcher 0x44e3a0 selects the 3/4-source blitter
    // 0x5d8000, or its reverse at 0x5d8050 for Foliage. Keep original flags
    // intact; this is sprite material opacity, never a movement property.
    if tile.flags & TaeFlag::Foliage.bits() != 0 { 0.25 } else { 0.75 }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, ShaderType)]
pub struct SpriteInstance {
    pub world_x: f32,        // tile_x + x_offset
    pub world_z: f32,        // tile_y + y_offset
    pub world_y: f32,        // z * height_scale (isometric altitude)
    pub layer: u32,          // atlas page layer
    pub depth_class: u32,    // encoded StaticDepthClass for future logical-depth policy
    pub base_world_y: f32,   // raw tile-base height for logical depth
    pub uv_min: [f32; 2],    // normalized UV
    pub uv_max: [f32; 2],    // normalized UV
    pub local_min: [f32; 2], // local quad bounds from tile origin
    pub local_max: [f32; 2], // local quad bounds from tile origin
    /// The draw order's anchor (art/depth.wesl): the cell + 2 in x + y for an upright sprite.
    pub tile_x: f32,
    pub tile_y: f32,
    /// ClassicUO's priority z (`resolve_priority_z_units`).
    pub priority_z_units: f32,
    /// The draw order's tie level (`assign_sprite_tie_levels`): 1..8 for an upright sprite, 0 for a flat quad.
    pub sort_bias_ordinal: u32,
    /// Tiledata flags packed for the GPU.
    /// Bit 0: is_wet (`static_tile_renders_wet` → animated water UV distortion in sprite shader).
    /// Bit 1: ground surface blend; bits 2..9: NW,N,NE,W,E,SW,S,SE same-material neighbors.
    /// Bit 10: `STATIC_FLAG_CLIENT_GRUNGE`, the KR client grunges the static (art/client_grunge.wesl).
    /// Bit 11: `STATIC_FLAG_FLAT_ART`, the upright card shows flat art (art/shading.wesl).
    /// Bit 24: `STATIC_FLAG_CAP_MEETS_FLOOR`, a wall with a floor on its top in the farther row (art/depth.wesl).
    /// Bit 25: `STATIC_FLAG_LAND_GRADE`, the static continues the land and takes its grading (art/shading.wesl).
    pub is_wet_flags: u32,
    pub hue_id: u32,
    pub hue_flags: u32,
    /// Flat-quad flag: 1 = lay this instance flat in the XZ plane (CC surface-like
    /// water statics), 2 = axis-aligned EC surface, 3 = authored EC camera quad,
    /// 0 = upright billboard.
    /// See main.wesl.
    pub flat_quad: u32,
    pub texture_stretch: f32,
    pub material_item_id: u32,
    /// The KR client's static shadow: `CLIENT_SHADOW_FLAG_*` bits (art/client_shadow.wesl).
    pub client_shadow_flags: u32,
    /// The caster's height in UO z units above the client's ground below it; 0 takes the shadow's h = 0 branch.
    pub client_shadow_height: f32,
    /// The KR client's base point of the static on the ground, (tile x + 0.5, tile y + 1.5) in world x and z: the
    /// client grunge maps from it (art/client_grunge.wesl). Takes the former `_pad_client_shadow` slot.
    pub client_grunge_anchor: [f32; 2],
    /// The static local lights at the quad's four corners, in `ART_QUAD_CORNERS` order, each RGBA8 packed
    /// (`static_local_light_corners`); the shaders interpolate them across the quad.
    pub local_light_corners: [u32; 4],
    pub color_rgba: [f32; 4], // for dot mode
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, ShaderType)]
pub struct GroundTileInstance {
    pub world_x: f32,
    pub world_z: f32,
    pub world_y: f32,
    pub layer: u32,
    pub depth_class: u32,
    pub base_world_y: f32,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub local_min: [f32; 2],
    pub local_max: [f32; 2],
    pub tile_x: f32,
    pub tile_y: f32,
    pub priority_z_units: f32,
    pub sort_bias_ordinal: u32,
    /// Tiledata flags packed for the GPU.
    /// Bit 0: is_wet (IsWet tiledata flag → animated water UV distortion in ground shader).
    /// Bit 10: `STATIC_FLAG_CLIENT_GRUNGE`, the KR client grunges the static (art/client_grunge.wesl).
    /// Bit 25: `STATIC_FLAG_LAND_GRADE`, the static continues the land and takes its grading (art/shading.wesl).
    pub is_wet_flags: u32,
    pub texture_stretch: f32,
    pub hue_id: u32,
    pub hue_flags: u32,
    pub material_payload: u32,
    pub material_flags: u32,
    /// Original item graphic selects its own optional fourth water source.
    pub material_item_id: u32,
    pub _pad_material: u32,
    /// The KR client's base point of the static, as `SpriteInstance::client_grunge_anchor`: the client grunge maps
    /// from it (art/client_grunge.wesl).
    pub client_grunge_anchor: [f32; 2],
    /// The static local lights at the quad's corners, as `SpriteInstance::local_light_corners`.
    pub local_light_corners: [u32; 4],
    pub color_rgba: [f32; 4],
}

#[derive(Resource, Default)]
pub struct RenderStaticInstances(pub Vec<SpriteInstance>);

#[derive(Resource, Default)]
pub struct RenderStaticLandInstances(pub Vec<GroundTileInstance>);

/// A hash of every collected static instance in output order, without its art page placement (`layer`, `uv_min`,
/// `uv_max`, which follow the order the pages arrived in): two runs that collected the same statics report the same
/// digest, however the land and the art loaded (`status.art.digest` on the control socket).
pub(crate) fn static_instances_digest(sprites: &[SpriteInstance], ground: &[GroundTileInstance]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    for instance in sprites {
        let unplaced = SpriteInstance { layer: 0, uv_min: [0.0; 2], uv_max: [0.0; 2], ..*instance };
        hasher.write(bytemuck::bytes_of(&unplaced));
    }
    for instance in ground {
        let unplaced = GroundTileInstance { layer: 0, uv_min: [0.0; 2], uv_max: [0.0; 2], ..*instance };
        hasher.write(bytemuck::bytes_of(&unplaced));
    }
    hasher.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StaticChunkBatchKey {
    pub map_id: u32,
    pub gx: u32,
    pub gy: u32,
    pub scale: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticChunkBatch {
    pub key: StaticChunkBatchKey,
    pub start: u32,
    pub count: u32,
}

#[derive(Resource, Default)]
pub struct RenderStaticChunkBatches {
    pub sprite: Vec<StaticChunkBatch>,
    pub ground: Vec<StaticChunkBatch>,
}
