//! A bounded native-chunk cache, published only after metadata uploads settle.
use super::{build_stack_layer, HALO, HEIGHT, SIDE, WIDTH};
use super::super::EcTerrainBases;
use crate::configs::settings::{ClientTextureSource, Settings};
use crate::core::{maps::MapPlane, render::scene::{player::Player, world::land::{tile_atlas::TileAtlas, LandUploadTelemetry}}, uo_files_loader::MapPlanesRes};
use bevy::{asset::RenderAssetUsages, prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension}};
use uocf::classic::map::MapBlockRelPos;

pub(crate) const SLOTS: usize = 16;
const BAKES_PER_FRAME: usize = 2;

struct Slot {
    chunk: UVec2,
    limits: [u16; 2],
    raw: Box<[u16; HALO * HALO]>,
    liquids: Box<[bool; HALO * HALO]>,
    checked_revision: Option<u64>,
    resident: bool,
    rebake: bool,
}

pub(crate) struct StackCache {
    pub image: Handle<Image>,
    available: Vec<bool>,
    slots: [Option<Slot>; SLOTS],
    atlas_revision: u64,
}

impl StackCache {
    pub fn new(images: &mut Assets<Image>, package: Option<&udd_assets::tex_land_ec::TexLandEcPackage>) -> Self {
        let mut image = Image::new(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2, vec![0; 16], TextureFormat::Rgba32Uint,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array), ..Default::default()
        });
        let available = (0..16384).map(|id| package.is_some_and(|p|
            p.resolve_effective_runtime_slot_id(id).and_then(|slot| p.present_slot(slot))
                .is_some_and(|slot| slot.width > 0 && slot.height > 0))).collect();
        Self { image: images.add(image), available, slots: std::array::from_fn(|_| None),
            atlas_revision: u64::MAX }
    }

    pub fn clear(&mut self) {
        self.slots = std::array::from_fn(|_| None);
        self.atlas_revision = u64::MAX;
    }
}

fn raw_halo(plane: &MapPlane, chunk: UVec2) -> Option<Box<[u16; HALO * HALO]>> {
    // A 34-square neighborhood touches six native 8-square blocks per axis.
    // Read cached blocks only; missing in-world data defers this bake.
    let origin = IVec2::new(chunk.x as i32 * 4 - 1, chunk.y as i32 * 4 - 1);
    let blocks: [_; 36] = std::array::from_fn(|index| {
        let point = origin + IVec2::new(index as i32 % 6, index as i32 / 6);
        if point.x < 0 || point.y < 0 { return None; }
        plane.block_no_update(MapBlockRelPos { x: point.x as u32, y: point.y as u32 })
    });
    for (index, block) in blocks.iter().enumerate() {
        let point = origin + IVec2::new(index as i32 % 6, index as i32 / 6);
        if point.x >= 0 && point.y >= 0 && (point.x as u32) < plane.size_blocks.width
            && (point.y as u32) < plane.size_blocks.height && block.is_none() { return None; }
    }
    let mut raw = Box::new([u16::MAX; HALO * HALO]);
    for y in 0..HALO {
        for x in 0..HALO {
            let (px, py) = (x + 7, y + 7);
            if let Some(block) = blocks[(py / 8) * 6 + px / 8] {
                raw[y * HALO + x] = block.cells[(py % 8) * 8 + px % 8].id;
            }
        }
    }
    Some(raw)
}

fn resident_matches(atlas: &TileAtlas, chunk: UVec2, raw: &[u16; HALO * HALO], available: &[bool]) -> bool {
    resident_state(atlas, chunk, raw, available, None).0
}

fn liquid_halo(atlas: &TileAtlas, chunk: UVec2) -> Box<[bool; HALO * HALO]> {
    Box::new(std::array::from_fn(|index| {
        let x = chunk.x as i32 * 32 + (index % HALO) as i32 - 1;
        let y = chunk.y as i32 * 32 + (index / HALO) as i32 - 1;
        x >= 0 && y >= 0 && atlas.read_cell(x as u32, y as u32).is_some_and(|p| p.g & 0xC000 != 0)
    }))
}

// Distinguish an unsettled halo from changed wet/reviewed-liquid flags. Only
// the latter needs a new bake; missing pages can reuse immutable cached data.
fn resident_state(atlas: &TileAtlas, chunk: UVec2, raw: &[u16; HALO * HALO], available: &[bool],
    liquids: Option<&[bool; HALO * HALO]>) -> (bool, bool) {
    let mut changed_liquids = false;
    for y in 0..HALO {
        for x in 0..HALO {
            let point = IVec2::new(chunk.x as i32 * 32 + x as i32 - 1, chunk.y as i32 * 32 + y as i32 - 1);
            let meta = if point.x < 0 || point.y < 0 { None } else { atlas.read_cell(point.x as u32, point.y as u32) };
            let payload = raw[y * HALO + x];
            if available.get(usize::from(payload)).copied().unwrap_or(false) {
                if !meta.is_some_and(|p| (p.g >> 8) & 15 == 2 && p.r == payload) { return (false, false); }
                if let Some(liquids) = liquids {
                    changed_liquids |= (meta.unwrap().g & 0xC000 != 0) != liquids[y * HALO + x];
                }
            } else if meta.is_some_and(|p| (p.g >> 8) & 15 == 2) {
                return (false, false);
            }
        }
    }
    (!changed_liquids, changed_liquids)
}

fn publish(values: &mut [[u16; 4]], dimensions: UVec2, dirty: &mut bool,
    chunk: UVec2, slot: Option<usize>, map: u32) {
    let at = (chunk.y * dimensions.x + chunk.x) as usize;
    let value = &mut values[at];
    let next = [value[0], value[1], slot.map_or(u16::MAX, |id| id as u16), map as u16];
    if *value != next { *value = next; *dirty = true; }
}

pub(crate) fn sys_prepare(
    settings: Res<Settings>, maps: Res<MapPlanesRes>, player: Query<&Player>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    atlas: Res<TileAtlas>, telemetry: Res<LandUploadTelemetry>,
    mut bases: ResMut<EcTerrainBases>, mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial>>,
    shared_material: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
) {
    if settings.graphics.land_texture_source != ClientTextureSource::Ec { return; }
    let Some(position) = player.single().ok().and_then(|p| p.current_pos) else { return; };
    let map = u32::from(position.m);
    if bases.map != Some(map) { return; }
    let Some(plane) = maps.0.get(map as usize).and_then(|p| p.as_ref()) else { return; };
    let Ok((camera, transform)) = camera.single() else { return; };
    let Some(viewport) = camera.logical_viewport_rect() else { return; };
    let Ok(ray) = camera.viewport_to_world(transform, viewport.center()) else { return; };
    if ray.direction.y.abs() <= 1e-6 { return; }
    let distance = -ray.origin.y / ray.direction.y;
    if distance < 0.0 { return; }
    let focus = ray.origin + ray.direction * distance;
    let center = IVec2::new((focus.x / SIDE as f32).floor() as i32, (focus.z / SIDE as f32).floor() as i32);
    let dimensions = bases.dimensions;
    let mut desired = Vec::with_capacity(SLOTS);
    for dy in -1..=2 {
        for dx in -1..=2 {
            let point = center + IVec2::new(dx, dy);
            if point.x >= 0 && point.y >= 0 && (point.x as u32) < dimensions.x && (point.y as u32) < dimensions.y {
                desired.push(UVec2::new(point.x as u32, point.y as u32));
            }
        }
    }
    desired.sort_by(|a, b| {
        let distance = |p: &UVec2| ((p.as_vec2() + Vec2::splat(0.5)) * SIDE as f32 - focus.xz()).length_squared();
        distance(a).total_cmp(&distance(b))
    });

    let EcTerrainBases { cache, params, values, dirty, .. } = &mut *bases;
    let changed = cache.atlas_revision != atlas.content_revision;
    if changed {
        cache.atlas_revision = atlas.content_revision;
        for slot in cache.slots.iter_mut().flatten() { slot.checked_revision = None; }
    }
    let uploads = telemetry.snapshot();
    // The render thread acknowledges a revision only after the entire budgeted
    // upload queue has been issued. This is a dependency, not a timed delay.
    let ready = !atlas.has_pending_uploads() && uploads.submitted_metadata_revision == atlas.content_revision;
    if ready {
        for slot in cache.slots.iter_mut().flatten() {
            if slot.checked_revision != Some(atlas.content_revision) {
                let (resident, rebake) = resident_state(&atlas, slot.chunk, &slot.raw,
                    &cache.available, Some(slot.liquids.as_ref()));
                slot.resident = resident;
                slot.rebake = rebake;
                slot.checked_revision = Some(atlas.content_revision);
            }
        }
    }
    let mut baked = Vec::new();
    for &chunk in &desired {
        let at = (chunk.y * dimensions.x + chunk.x) as usize;
        let limits = [values[at][0], values[at][1]];
        if limits[0] == u16::MAX { continue; }
        let existing = cache.slots.iter().position(|slot| slot.as_ref().is_some_and(|s| s.chunk == chunk));
        if existing.is_some_and(|id| cache.slots[id].as_ref().is_some_and(|s| s.limits == limits && !s.rebake)) { continue; }
        // Grime decisions use the same resident flags as the original shader.
        if !ready { continue; }
        if baked.len() >= BAKES_PER_FRAME { break; }
        let Some(raw) = raw_halo(plane, chunk) else { continue; };
        if !resident_matches(&atlas, chunk, &raw, &cache.available) { continue; }
        let liquids = liquid_halo(&atlas, chunk);
        let Some(records) = build_stack_layer(chunk, map, &raw, params, &cache.available, &liquids, limits) else { continue; };
        let slot = existing.or_else(|| cache.slots.iter().position(Option::is_none))
            .or_else(|| cache.slots.iter().position(|slot| slot.as_ref().is_some_and(|s| !desired.contains(&s.chunk))));
        let Some(slot) = slot else { continue; };
        if let Some(old) = &cache.slots[slot] { publish(values, dimensions, dirty, old.chunk, None, map); }
        cache.slots[slot] = Some(Slot { chunk, limits, raw, liquids,
            checked_revision: Some(atlas.content_revision), resident: true, rebake: false });
        baked.push((slot, records));
    }
    if !baked.is_empty() {
        let mut image = images.get_mut(&cache.image).expect("terrain stack cache image");
        let layer_bytes = WIDTH * HEIGHT * 16;
        if image.texture_descriptor.size.width != WIDTH as u32 {
            // CC-only previews retain the tiny sentinel instead of allocating
            // a full cache that no fragment can use.
            image.texture_descriptor.size = Extent3d {
                width: WIDTH as u32, height: HEIGHT as u32, depth_or_array_layers: SLOTS as u32,
            };
            image.data = Some(vec![0; layer_bytes * SLOTS]);
            // Resizing replaces the GPU texture/view. Rebind once so a
            // material prepared before metadata settled cannot keep the tiny
            // sentinel. Subsequent bakes reuse this texture and its view.
            if let Some(mut material) = materials.get_mut(&shared_material.0) {
                material.extension.ec_terrain_stacks = cache.image.clone();
            }
        }
        let data = image.data.as_mut().expect("terrain stack cache pixels");
        for (slot, records) in baked {
            data[slot * layer_bytes..(slot + 1) * layer_bytes].copy_from_slice(bytemuck::cast_slice(&records));
        }
    }
    for (id, slot) in cache.slots.iter_mut().enumerate() {
        let Some(slot) = slot else { continue; };
        let at = (slot.chunk.y * dimensions.x + slot.chunk.x) as usize;
        let limits_match = slot.limits == [values[at][0], values[at][1]];
        publish(values, dimensions, dirty, slot.chunk, (ready && slot.resident && limits_match).then_some(id), map);
    }
    bases.flush(&mut images);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u};

    #[test]
    fn residency_requires_matching_ec_halo_and_rejects_eviction_and_source_changes() {
        let mut atlas = TileAtlas::new(AtlasParams {
            page_texels: UVec2::splat(128), tiles_per_page: UVec2::splat(128),
            max_layers: 1, world_pages_x: 2, ..Default::default()
        }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        let raw = [2; HALO * HALO];
        let mut available = vec![false; 3];
        available[2] = true;
        assert!(!resident_matches(&atlas, UVec2::ONE, &raw, &available));
        let data = vec![Rg16u::pack(2, 0, 2, false, 0); 128 * 128];
        atlas.enqueue_rg16u_block(layer, UVec2::ZERO, UVec2::splat(128), &data);
        assert!(resident_matches(&atlas, UVec2::ONE, &raw, &available));
        let liquids = liquid_halo(&atlas, UVec2::ONE);
        assert_eq!(resident_state(&atlas, UVec2::ONE, &raw, &available, Some(&liquids)), (true, false));
        for (wet, flags) in [(true, 0), (false, 4)] {
            atlas.enqueue_rg16u_block(layer, UVec2::splat(31), UVec2::ONE, &[Rg16u::pack(2, 0, 2, wet, flags)]);
            assert_eq!(resident_state(&atlas, UVec2::ONE, &raw, &available, Some(&liquids)), (false, true));
        }
        atlas.enqueue_rg16u_block(layer, UVec2::splat(31), UVec2::ONE, &[Rg16u::pack(2, 5, 2, false, 3)]);
        assert_eq!(resident_state(&atlas, UVec2::ONE, &raw, &available, Some(&liquids)), (true, false));
        // Checking just the current cell would overlook an unsettled neighbor.
        atlas.enqueue_rg16u_block(layer, UVec2::splat(31), UVec2::ONE, &[Rg16u::pack(1, 0, 2, false, 0)]);
        assert!(!resident_matches(&atlas, UVec2::ONE, &raw, &available));
        atlas.enqueue_rg16u_block(layer, UVec2::splat(31), UVec2::ONE, &[data[0]]);
        assert!(resident_matches(&atlas, UVec2::ONE, &raw, &available));
        // A CC payload with the same number is not an EC source match.
        atlas.enqueue_rg16u_block(layer, UVec2::splat(31), UVec2::ONE, &[Rg16u::pack(2, 0, 0, false, 0)]);
        assert!(!resident_matches(&atlas, UVec2::ONE, &raw, &available));
        atlas.ensure_layer_for_page(IVec2::X);
        assert!(!resident_matches(&atlas, UVec2::ONE, &raw, &available));
    }
}
