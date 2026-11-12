//! Terrain-only cursor query. Rendered triangles determine the visible cell;
//! original map Z remains separate from bank relief and interpolated height.
use super::{draw_mesh::LandMeshHandles, tile_atlas::TileAtlas, LCMesh, LandUploadTelemetry};
use crate::core::{app_states::AppState, controls::{InjectedPointer, effective_cursor_position},
    render::scene::{camera::PlayerCamera, player::Player}, system_sets::{MovementSysSet, SceneRenderLandSysSet},
    texture_cache::land::cache::LandTextureCache, uo_files_loader::MapPlanesRes};
use crate::configs::settings::LandTextureSource;
use crate::util_lib::uo_coords::{UOVec4, UO_Z_TO_WORLD_SCALE};
use bevy::{prelude::*, window::PrimaryWindow};
use std::collections::HashMap;
use uocf::classic::map::{MapCell, MapCellCoords};

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct TerrainCursorHit {
    pub logical: UOVec4,
    pub graphic: u16,
    pub world_position: Vec3,
}
#[derive(Resource, Default)]
pub(crate) struct TerrainCursor {
    pub hit: Option<TerrainCursorHit>,
    pub pointer: Option<Vec2>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Grid { origin: UVec2, span: u32, step: u32 }
#[derive(PartialEq)]
struct Signature {
    pointer: Vec2, view: Mat4, clip: Mat4, viewport: UVec4, map: u8,
    revision: u64, land_source: Option<LandTextureSource>, submitted: bool, grids: Vec<Grid>,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TerrainCursorQuery;

pub(super) fn install(app: &mut App) {
    app.init_resource::<TerrainCursor>().add_systems(Update, update_terrain_cursor.in_set(TerrainCursorQuery)
        .after(SceneRenderLandSysSet::RenderLandChunks).after(MovementSysSet::UpdateCamera)
        .run_if(in_state(AppState::InGame)));
}

fn update_terrain_cursor(
    mut cursor: ResMut<TerrainCursor>, mut previous: Local<Option<Signature>>,
    windows: Query<&Window, With<PrimaryWindow>>, pointer: Option<Res<InjectedPointer>>,
    cameras: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>, players: Query<&Player>,
    chunks: Query<(&LCMesh, &Mesh3d, &ViewVisibility)>, handles: Res<LandMeshHandles>,
    atlas: Res<TileAtlas>, uploads: Res<LandUploadTelemetry>, maps: Res<MapPlanesRes>,
    land_textures: Option<Res<LandTextureCache>>,
) {
    // The EC-layout land package the terrain draws (tex_land_ec.uddp or tex_land_kr.uddp) decides which cells bank.
    let ec = land_textures.as_deref().and_then(LandTextureCache::enhanced_land);
    let inputs = (|| {
        let pointer = effective_cursor_position(windows.single().ok()?, pointer.as_deref())?;
        let (camera, transform) = cameras.single().ok()?;
        let map = players.single().ok()?.current_pos?.m;
        let viewport = camera.physical_viewport_rect()?;
        let scale = camera.target_scaling_factor()?;
        if !viewport.contains((pointer * scale).floor().as_uvec2()) { return None; }
        let mut grids: Vec<_> = chunks.iter().filter(|(chunk, _, visible)| chunk.parent_map_id == u32::from(map) && visible.get())
            .filter_map(|(chunk, mesh, _)| handles.grid(mesh).map(|(span, step)| Grid {
                origin: UVec2::new(chunk.gx, chunk.gy) * super::TILE_NUM_PER_CHUNK_DIM, span, step })).collect();
        grids.sort_unstable_by_key(|g| (g.origin.x, g.origin.y, g.span, g.step));
        let telemetry = uploads.snapshot();
        let submitted = !atlas.has_pending_uploads() && telemetry.pending_ops == 0
            && telemetry.submitted_metadata_revision == atlas.content_revision;
        let signature = Signature { pointer, view: transform.to_matrix(), clip: camera.clip_from_view(),
            viewport: UVec4::new(viewport.min.x, viewport.min.y, viewport.width(), viewport.height()), map,
            revision: atlas.content_revision, land_source: land_textures.as_deref().map(LandTextureCache::preferred_source),
            submitted, grids };
        Some((signature, cursor_ray(camera, transform, pointer)?, transform.back().as_vec3()))
    })();
    let Some((signature, ray, back)) = inputs else {
        *previous = None;
        if cursor.hit.is_some() || cursor.pointer.is_some() { cursor.hit = None; cursor.pointer = None; }
        return;
    };
    if previous.as_ref() == Some(&signature) { return; }
    let hit = if signature.submitted {
        let read = |cell: IVec2| {
            if cell.min_element() < 0 { return None; }
            let packed = atlas.read_cell(cell.x as u32, cell.y as u32)?;
            let mode = (packed.g >> 8) & 15;
            let ec_present = mode == 2 && ec.is_some_and(|p| p.present_slot(u32::from(packed.r)).is_some_and(|slot| slot.width != 0 && slot.height != 0));
            Some(TerrainSample { height: f32::from(((packed.g & 255) as i16 - 128) as i8) * UO_Z_TO_WORLD_SCALE,
                ec_present, liquid: ((packed.g >> 12) & 4) != 0 })
        };
        trace_grids(ray, back.xz(), &signature.grids, &read).and_then(|world_position| {
            let clip = signature.clip * signature.view.inverse() * world_position.extend(1.0);
            if !clip.is_finite() || clip.w <= 0.0 || !(0.0..=1.0).contains(&(clip.z / clip.w)) { return None; }
            let cell = world_position.xz().floor();
            let plane = maps.0.get(usize::from(signature.map))?.as_ref()?;
            if cell.min_element() < 0.0 || cell.x >= (plane.size_blocks.width * 8) as f32
                || cell.y >= (plane.size_blocks.height * 8) as f32 { return None; }
            let coords = MapCellCoords { x: cell.x as u32, y: cell.y as u32 };
            let rel = MapCell::coords_in_block(&coords);
            let source = plane.block_no_update(MapCell::coords_of_parent_block(&coords))?.cell(rel.x, rel.y).ok()?;
            Some(TerrainCursorHit { logical: UOVec4::new(coords.x as u16, coords.y as u16, source.z, signature.map),
                graphic: source.id, world_position })
        })
    } else { None };
    cursor.hit = hit;
    cursor.pointer = Some(signature.pointer);
    *previous = Some(signature);
}

fn cursor_ray(camera: &Camera, transform: &GlobalTransform, pointer: Vec2) -> Option<Ray3d> {
    let scale = camera.target_scaling_factor()?;
    let pixel = (pointer * scale).floor().as_uvec2();
    if !camera.physical_viewport_rect()?.contains(pixel) { return None; }
    // Match the static GPU query's physical pixel center, including HiDPI and
    // an inset viewport. Logical mouse coordinates themselves remain unchanged.
    camera.viewport_to_world(transform, (pixel.as_vec2() + Vec2::splat(0.5)) / scale).ok()
}

#[derive(Clone, Copy)]
pub(crate) struct TerrainSample { pub height: f32, pub ec_present: bool, pub liquid: bool }

/// CPU counterpart of liquid_bank.wesl::ec_bank_vertex and the vertex shader's
/// view-facing relaxation. The GPU regression compares both implementations.
pub(crate) fn bank_vertex(cell: IVec2, back: Vec2, read: &impl Fn(IVec2) -> Option<TerrainSample>) -> Option<Vec3> {
    let current = read(cell)?;
    let origin = Vec3::new(cell.x as f32, current.height, cell.y as f32);
    if !current.ec_present { return Some(origin); }
    let mut total = current.height * 4.0;
    let mut weight = 4.0;
    let mut liquid_level = f32::NEG_INFINITY;
    let mut diagonal = Vec2::ZERO;
    let mut count = 0.0;
    let mut cardinal = false;
    for index in 0..9 {
        if index == 4 { continue; }
        let offset = IVec2::new(index % 3 - 1, index / 3 - 1);
        let Some(candidate) = read(cell + offset).filter(|s| s.ec_present) else { continue; };
        let kernel = if offset.x == 0 || offset.y == 0 { 2.0 } else { 1.0 };
        total += candidate.height * kernel;
        weight += kernel;
        if current.liquid && !candidate.liquid && candidate.height > current.height + 0.05 {
            if kernel == 2.0 { cardinal = true; } else { diagonal += offset.as_vec2(); count += 1.0; }
        }
        if candidate.liquid && candidate.height < current.height - 0.05 { liquid_level = liquid_level.max(candidate.height); }
    }
    if current.liquid {
        if !cardinal && count > 0.0 {
            let offset = diagonal * (0.25 / count);
            let t = (offset.dot(back) / 0.1).clamp(0.0, 1.0);
            let eased = offset * (t * t * (3.0 - 2.0 * t));
            return Some(origin + Vec3::new(eased.x, 0.0, eased.y));
        }
    } else if liquid_level.is_finite() {
        return Some(Vec3::new(origin.x, liquid_level.max(current.height + ((total / weight - current.height) * 0.35).clamp(-0.15, 0.15)), origin.z));
    }
    Some(origin)
}

fn ray_box(ray: Ray3d, min: Vec3, max: Vec3) -> Option<(f32, f32)> {
    let mut near = 0.0f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let d = ray.direction[axis];
        if d.abs() < 1e-8 {
            if ray.origin[axis] < min[axis] || ray.origin[axis] > max[axis] { return None; }
        } else {
            let a = (min[axis] - ray.origin[axis]) / d;
            let b = (max[axis] - ray.origin[axis]) / d;
            near = near.max(a.min(b)); far = far.min(a.max(b));
        }
    }
    (near <= far && near.is_finite() && far.is_finite()).then_some((near, far))
}

fn triangle_hit(ray: Ray3d, [a, b, c]: [Vec3; 3]) -> Option<f64> {
    // Local differences in double precision avoid facet-sized origins degrading
    // edge decisions. The original mesh winds upward; back faces do not draw.
    let [a, b, c] = [a, b, c].map(Vec3::as_dvec3);
    let direction = ray.direction.as_vec3().as_dvec3();
    let edge = b - a;
    let other = c - a;
    let p = direction.cross(other);
    let det = edge.dot(p);
    if det <= 1e-10 { return None; }
    let delta = ray.origin.as_dvec3() - a;
    let u = delta.dot(p) / det;
    let q = delta.cross(edge);
    let v = direction.dot(q) / det;
    let t = other.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t >= 0.0).then_some(t)
}

fn trace_grids(ray: Ray3d, back: Vec2, grids: &[Grid], read: &impl Fn(IVec2) -> Option<TerrainSample>) -> Option<Vec3> {
    let mut vertices = HashMap::<IVec2, Option<Vec3>>::new();
    let mut nearest = f64::INFINITY;
    for grid in grids {
        let origin = grid.origin.as_vec2();
        // Every rendered vertex stays in this signed-Z band, including bank
        // easing and its quarter-cell lateral bevel. Restrict work to the ray.
        let min = Vec3::new(origin.x - 0.25, f32::from(i8::MIN) * UO_Z_TO_WORLD_SCALE - 0.15, origin.y - 0.25);
        let max = Vec3::new(origin.x + grid.span as f32 + 0.25,
            f32::from(i8::MAX) * UO_Z_TO_WORLD_SCALE + 0.15, origin.y + grid.span as f32 + 0.25);
        let Some((near, far)) = ray_box(ray, min, max) else { continue; };
        if f64::from(near) > nearest { continue; }
        let start = ray.get_point(near).xz(); let end = ray.get_point(far).xz();
        let low = ((start.min(end) - origin - Vec2::splat(0.25)) / grid.step as f32).floor().as_ivec2().max(IVec2::ZERO);
        let high = ((start.max(end) - origin + Vec2::splat(0.25)) / grid.step as f32).floor().as_ivec2()
            .min(IVec2::splat((grid.span / grid.step) as i32 - 1));
        for y in low.y..=high.y { for x in low.x..=high.x {
            let cell = grid.origin.as_ivec2() + IVec2::new(x, y) * grid.step as i32;
            let corners = [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE].map(|offset| {
                let key = cell + offset * grid.step as i32;
                *vertices.entry(key).or_insert_with(|| bank_vertex(key, back, read))
            });
            let [Some(nw), Some(ne), Some(sw), Some(se)] = corners else { continue; };
            // Identical topology to setup_base_mesh.rs, including its NW-SE diagonal.
            for triangle in [[nw, se, ne], [nw, sw, se]] {
                if let Some(t) = triangle_hit(ray, triangle) { nearest = nearest.min(t); }
            }
        }}
    }
    nearest.is_finite().then(|| (ray.origin.as_dvec3() + ray.direction.as_vec3().as_dvec3() * nearest).as_vec3())
}

#[cfg(test)]
mod tests;
