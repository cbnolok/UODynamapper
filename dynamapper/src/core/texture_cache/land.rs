pub mod cache;
pub mod texture_array;

use crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial;
use crate::core::system_sets::*;
use crate::core::texture_cache::{
    resolve_layer_allocations, TextureResidencyGroupLayers, TextureResidencyStrategy,
};
use crate::prelude::*;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use color_eyre::eyre;
use std::time::{Duration, Instant};
use udd_assets::{
    tex_land_cc::TexLandCcPackage,
    tex_land_ec::TexLandEcPackage,
};
use uocf::classic::land_texture::LandTextureSize;
use uocf::classic::map::MapBlockRelPos;

pub struct LandTextureCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(LandTextureCachePlugin);

// Terrain currently splits texmaps by physical source dimensions because small and big
// texmaps live in different GPU texture arrays. Future collections can define their own
// group table while still reusing the shared residency helpers.
const LAND_TEXTURE_GROUP_LAYERS: [TextureResidencyGroupLayers<LandTextureSize>; 2] = [
    TextureResidencyGroupLayers {
        group: LandTextureSize::Small,
        debug_name: "small texmap",
        initial_layers: texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
        max_layers: texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
    },
    TextureResidencyGroupLayers {
        group: LandTextureSize::Big,
        debug_name: "big texmap",
        initial_layers: texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS,
        max_layers: texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
    },
];

struct LandPageAtlasImages {
    page_atlas: Handle<Image>,
    lookup: Handle<Image>,
}

enum LandShaderAtlasSource<'a> {
    Cc(&'a TexLandCcPackage),
    Ec(&'a TexLandEcPackage),
}

const LAND_PAGE_LOOKUP_WIDTH: u32 = 256;
const LAND_PAGE_LOOKUP_TILE_CAPACITY: u32 = 16_384;
const LAND_PAGE_LOOKUP_ROLE_COUNT: u32 = 4;
const LAND_PAGE_LOOKUP_ROLE_BASE: u32 = 0;
const LAND_PAGE_LOOKUP_ROLE_DETAIL: u32 = 1;
const LAND_PAGE_LOOKUP_ROLE_MASK: u32 = 2;
const LAND_PAGE_LOOKUP_ROLE_NORMAL: u32 = 3;

fn create_blank_land_page_atlas_images(images: &mut Assets<Image>) -> LandPageAtlasImages {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    };

    let mut atlas = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; 4],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    let mut lookup = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; 16],
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    LandPageAtlasImages {
        page_atlas: images.add(atlas),
        lookup: images.add(lookup),
    }
}

fn copy_land_page_into_full_atlas_rgba(
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    used_rgba: &[u8],
) -> eyre::Result<Vec<u8>> {
    let mut atlas_rgba = vec![0u8; (atlas_width * atlas_height * 4) as usize];
    let row_bytes = used_width as usize * 4;
    for row in 0..used_height as usize {
        let src_start = row * row_bytes;
        let src_end = src_start + row_bytes;
        let dst_start = row * atlas_width as usize * 4;
        let dst_end = dst_start + row_bytes;
        atlas_rgba[dst_start..dst_end].copy_from_slice(&used_rgba[src_start..src_end]);
    }

    Ok(atlas_rgba)
}

fn decode_tex_land_cc_page_rgba(package: &TexLandCcPackage, page_index: u32) -> eyre::Result<Vec<u8>> {
    let page = package
        .pages()
        .get(page_index as usize)
        .ok_or_else(|| eyre::eyre!("missing tex_land_cc page metadata for {page_index}"))?;
    let used_rgba = package.read_page_rgba(page_index)?;
    copy_land_page_into_full_atlas_rgba(
        package.atlas_width(),
        package.atlas_height(),
        page.used_width,
        page.used_height,
        &used_rgba,
    )
}

fn decode_tex_land_ec_page_rgba(package: &TexLandEcPackage, page_index: u32) -> eyre::Result<Vec<u8>> {
    let page = package
        .pages()
        .get(page_index as usize)
        .ok_or_else(|| eyre::eyre!("missing tex_land_ec page metadata for {page_index}"))?;
    let used_rgba = package.read_page_rgba(page_index)?;
    copy_land_page_into_full_atlas_rgba(
        package.atlas_width(),
        package.atlas_height(),
        page.used_width,
        page.used_height,
        &used_rgba,
    )
}

fn packed_page_and_stretch(page_index: u32, texture_repetition: f32) -> u32 {
    let stretch_q8 = if texture_repetition.is_finite() && texture_repetition > 0.0 {
        (texture_repetition * 256.0).round().clamp(0.0, u16::MAX as f32) as u32
    } else {
        0
    };
    (page_index & 0xFFFF) | (stretch_q8 << 16)
}

fn land_lookup_values_from_ec_slot(
    package: &TexLandEcPackage,
    slot_id: u32,
    texture_repetition: f32,
) -> [u32; 4] {
    package
        .slots()
        .get(slot_id as usize)
        .filter(|slot| slot.is_present())
        .map(|slot| {
            [
                packed_page_and_stretch(slot.page_index, texture_repetition),
                slot.x as u32,
                slot.y as u32,
                (slot.width as u32) | ((slot.height as u32) << 16),
            ]
        })
        .unwrap_or([0, 0, 0, 0])
}

fn write_land_lookup_entry(
    bytes: &mut [u8],
    tile_id: u32,
    role: u32,
    values: [u32; 4],
) {
    let lookup_index = role * LAND_PAGE_LOOKUP_TILE_CAPACITY + tile_id;
    let texel_offset = (lookup_index as usize) * 16;
    for (index, value) in values.into_iter().enumerate() {
        let start = texel_offset + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn create_land_shader_images(
    images: &mut Assets<Image>,
    source: Option<LandShaderAtlasSource<'_>>,
) -> eyre::Result<LandPageAtlasImages> {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    };

    let Some(source) = source else {
        return Ok(create_blank_land_page_atlas_images(images));
    };

    let (page_count, atlas_width, atlas_height) = match source {
        LandShaderAtlasSource::Cc(package) => (
            package.pages().len().max(1) as u32,
            package.atlas_width().max(1),
            package.atlas_height().max(1),
        ),
        LandShaderAtlasSource::Ec(package) => (
            package.pages().len().max(1) as u32,
            package.atlas_width().max(1),
            package.atlas_height().max(1),
        ),
    };

    let mut atlas_bytes =
        Vec::with_capacity((atlas_width * atlas_height * 4 * page_count) as usize);
    for page_index in 0..page_count {
        let rgba = match source {
            LandShaderAtlasSource::Cc(package) => decode_tex_land_cc_page_rgba(package, page_index)?,
            LandShaderAtlasSource::Ec(package) => decode_tex_land_ec_page_rgba(package, page_index)?,
        };
        atlas_bytes.extend_from_slice(&rgba);
    }

    let mut atlas = Image::new(
        Extent3d {
            width: atlas_width,
            height: atlas_height,
            depth_or_array_layers: page_count,
        },
        TextureDimension::D2,
        atlas_bytes,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    let max_cc_tile_id = LAND_PAGE_LOOKUP_TILE_CAPACITY;
    let lookup_width = LAND_PAGE_LOOKUP_WIDTH;
    let lookup_entry_count = max_cc_tile_id * LAND_PAGE_LOOKUP_ROLE_COUNT;
    let lookup_height = (lookup_entry_count + lookup_width - 1) / lookup_width;
    let mut lookup_bytes = vec![0u8; (lookup_width * lookup_height * 16) as usize];

    for tile_id in 0..max_cc_tile_id {
        match source {
            LandShaderAtlasSource::Cc(package) => {
                if let Some(slot) = package.present_slot(tile_id) {
                    write_land_lookup_entry(
                        &mut lookup_bytes,
                        tile_id,
                        LAND_PAGE_LOOKUP_ROLE_BASE,
                        [
                            slot.page_index,
                            slot.x as u32,
                            slot.y as u32,
                            (slot.width as u32) | ((slot.height as u32) << 16),
                        ],
                    );
                }
            }
            LandShaderAtlasSource::Ec(package) => {
                let layer0 = package.resolve_material_layer_slot(tile_id, 0);
                let layer1 = package.resolve_material_layer_slot(tile_id, 1);
                let layer2 = package.resolve_material_layer_slot(tile_id, 2);
                let layer3 = package.resolve_material_layer_slot(tile_id, 3);
                let has_base_layer = layer0
                    .as_ref()
                    .and_then(|layer| layer.runtime_slot_id)
                    .is_some();

                if has_base_layer {
                    for (role, layer) in [
                        (LAND_PAGE_LOOKUP_ROLE_BASE, layer0),
                        (LAND_PAGE_LOOKUP_ROLE_DETAIL, layer1),
                        (LAND_PAGE_LOOKUP_ROLE_MASK, layer2),
                        (LAND_PAGE_LOOKUP_ROLE_NORMAL, layer3),
                    ] {
                        if let Some(layer) = layer {
                            if let Some(slot_id) = layer.runtime_slot_id {
                                let values = land_lookup_values_from_ec_slot(
                                    package,
                                    slot_id,
                                    layer.texture_repetition,
                                );
                                write_land_lookup_entry(&mut lookup_bytes, tile_id, role, values);
                            }
                        }
                    }
                } else if let Some(slot_id) = package.resolve_effective_runtime_slot_id(tile_id) {
                    let texture_repetition = package
                        .resolve_material_decision(tile_id)
                        .primary_layer_index
                        .and_then(|layer_index| {
                            package
                                .resolve_material_layer_slot(tile_id, layer_index)
                                .map(|layer| layer.texture_repetition)
                        })
                        .unwrap_or(0.0);
                    let values =
                        land_lookup_values_from_ec_slot(package, slot_id, texture_repetition);
                    write_land_lookup_entry(
                        &mut lookup_bytes,
                        tile_id,
                        LAND_PAGE_LOOKUP_ROLE_BASE,
                        values,
                    );
                }
            }
        };
    }

    let mut lookup = Image::new(
        Extent3d {
            width: lookup_width,
            height: lookup_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        lookup_bytes,
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    Ok(LandPageAtlasImages {
        page_atlas: images.add(atlas),
        lookup: images.add(lookup),
    })
}

fn land_shader_source_from_cache<'a>(
    cache: &'a cache::LandTextureCache,
) -> Option<LandShaderAtlasSource<'a>> {
    match cache.preferred_source() {
        crate::configs::settings::ClientTextureSource::Cc => {
            cache.tex_land_cc.as_deref().map(LandShaderAtlasSource::Cc)
        }
        crate::configs::settings::ClientTextureSource::Ec => {
            cache.tex_land_ec.as_deref().map(LandShaderAtlasSource::Ec)
        }
    }
}

impl Plugin for LandTextureCachePlugin {
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            cache::TextureArrayImageHandles,
        >::default())
            .add_systems(
                Startup,
                sys_setup_terrain_cache
                    .in_set(StartupSysSet::SetupSceneStage1)
                    .after(StartupSysSet::LoadStartupUOFiles)
                    .run_if(resource_exists::<crate::core::uo_files_loader::TexMap2DRes>),
            );

        // Clear pending uploads at the start of each frame (First schedule), which runs BEFORE Update.
        // This is correct: Extract runs AFTER Last (end of previous frame), so by the time we reach
        // First of the next frame, the previous frame's uploads have already been consumed by Extract.
        // Clearing here ensures Update fills a fresh list for the current frame.
        app.add_systems(First, cache::sys_clear_texture_array_uploads);
        app.add_systems(Update, cache::sys_drain_texture_upload_tasks);
        app.add_systems(Update, sys_apply_land_texture_source_changes);

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        render_app.init_resource::<cache::RenderTextureArrayUploads>();
        render_app.add_systems(
            bevy::render::ExtractSchedule,
            cache::sys_extract_texture_array_uploads,
        );

        render_app.add_systems(
            bevy::render::Render,
            cache::sys_render_upload_texture_array.in_set(bevy::render::RenderSystems::Queue),
        );

        app.add_systems(
            Update,
            (sys_pin_active_textures, sys_evict_idle_land_cache)
                .chain()
                .run_if(on_timer(Duration::from_secs(5))),
        );
        app.add_systems(Update, sys_apply_texture_array_expansion);
        app.add_systems(Update, sys_apply_tile_atlas_expansion);
    }
}

fn sys_pin_active_textures(
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut map_planes_r: ResMut<crate::core::uo_files_loader::MapPlanesRes>,
    scene_state_r: Res<crate::core::render::scene::SceneStateData>,
    chunk_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    cache_r.clear_pinned_textures();

    let plane = map_planes_r
        .0
        .get_mut(scene_state_r.map_id as usize)
        .and_then(|opt| opt.as_mut())
        .expect("Uncached Map in sys_pin_active_textures");

    for mesh in chunk_q.iter() {
        let gx = mesh.gx as i32;
        let gy = mesh.gy as i32;
        let scale = mesh.scale as i32;

        for sx in -1..=scale {
            for sz in -1..=scale {
                let bx = gx + sx;
                let bz = gy + sz;
                if bx >= 0
                    && bx < plane.size_blocks.width as i32
                    && bz >= 0
                    && bz < plane.size_blocks.height as i32
                {
                    let pos = MapBlockRelPos {
                        x: bx as u32,
                        y: bz as u32,
                    };
                    if let Some(block) = plane.block(pos) {
                        for cell in &block.cells {
                            let cell_id = cell.id as usize;
                            let word = cell_id >> 6;
                            if word < cache_r.pinned_visible_bits.len() {
                                cache_r.pinned_visible_bits[word] |= 1u64 << (cell_id & 63);
                            }
                        }
                    }
                }
            }
        }
    }

    cache_r.visible_hint_count = cache_r
        .pinned_visible_bits
        .iter()
        .map(|w| w.count_ones() as usize)
        .sum();
}

fn sys_evict_idle_land_cache(
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut map_planes_r: ResMut<crate::core::uo_files_loader::MapPlanesRes>,
    _texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    scene_state_r: Res<crate::core::render::scene::SceneStateData>,
    mut tile_atlas: ResMut<crate::core::render::scene::world::land::tile_atlas::TileAtlas>,
    time: Res<Time<Real>>,
) {
    let now = time.last_update().unwrap_or_else(Instant::now);
    if !cache_r.preloads_full_collection() {
        // 1. Evict idle GPU layers from the Texture Array cache (VRAM/LRU management)
        let evicted_gpu_layers = cache_r.evict_idle_textures(now);
        if evicted_gpu_layers > 0 {
            console_logger::one(
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Evicted {} idle textures from GPU cache.",
                    evicted_gpu_layers
                ),
            );
        }

        /*
        // 2. Evict idle pixel data from the raw TexMap2D cache (CPU RAM)
        let evicted_pixel_buffers = texmap_2d_r.0.evict_idle_textures(Duration::from_secs(60));
        if evicted_pixel_buffers > 0 {
            console_logger::one(
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Evicted {} idle pixel buffers from TexMap2D cache.",
                    evicted_pixel_buffers
                ),
            );
        }
        */
    }

    // 3. Evict idle map blocks from the active map plane (CPU RAM)
    if let Some(plane) = map_planes_r
        .0
        .get_mut(scene_state_r.map_id as usize)
        .and_then(|opt| opt.as_mut())
    {
        let evicted_blocks = plane.evict_idle_blocks(Duration::from_secs(60));
        if evicted_blocks > 0 {
            console_logger::one(
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Evicted {} idle blocks from MapPlane {}.",
                    evicted_blocks, scene_state_r.map_id
                ),
            );
        }
    };

    // 4. Check whether the texture arrays can be shrunk (usage low for >2 min).
    if !cache_r.preloads_full_collection() {
        let (shrink_small, shrink_big) = cache_r.check_shrink_opportunity(now);
        if let Some(target) = shrink_small {
            cache_r.small.requested_resize_to = Some(target);
            console_logger::one(
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Requesting texture array shrink (Small): {} → {} layers.",
                    cache_r.small.active_layers, target
                ),
            );
        }
        if let Some(target) = shrink_big {
            cache_r.big.requested_resize_to = Some(target);
            console_logger::one(
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Requesting texture array shrink (Big): {} → {} layers.",
                    cache_r.big.active_layers, target
                ),
            );
        }
    }

    // 5. Check whether the tile metadata atlas can be shrunk.
    {
        let timeout = Duration::from_secs(texture_array::RESOURCE_SHRINK_TIMEOUT_SECS);
        let mapped = tile_atlas.mapped_page_count();
        let capacity = tile_atlas.params.max_layers;
        let usage_ratio = mapped as f32 / capacity.max(1) as f32;
        let elapsed = now.duration_since(tile_atlas.last_high_usage_instant);

        if usage_ratio < texture_array::RESOURCE_SHRINK_THRESHOLD
            && elapsed >= timeout
            && capacity > texture_array::TILE_ATLAS_INITIAL_LAYERS
            && tile_atlas.requested_expansion.is_none()
        {
            let target = ((mapped as f32 * 1.5).ceil() as u32)
                .next_power_of_two()
                .max(texture_array::TILE_ATLAS_INITIAL_LAYERS)
                .min(capacity);
            if target < capacity {
                tile_atlas.requested_expansion = Some(target);
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Performance,
                    &format!("Requesting tile atlas shrink: {capacity} → {target} layers.",),
                );
            }
        }
    }
}

fn sys_apply_land_texture_source_changes(
    mut commands: Commands,
    settings: Res<crate::configs::settings::Settings>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
    time: Res<Time<Real>>,
    chunks: Query<Entity, With<crate::core::render::scene::world::land::LCMesh>>,
) {
    let desired_source = settings.graphics.land_texture_source;
    if cache_r.preferred_source() == desired_source {
        return;
    }

    let missing_requested_package = match desired_source {
        crate::configs::settings::ClientTextureSource::Cc => false,
        crate::configs::settings::ClientTextureSource::Ec => cache_r.tex_land_ec.is_none(),
    };

    if missing_requested_package {
        console_logger::one(
            LogSev::Warn,
            LogAbout::General,
            &format!(
                "Requested {} land textures, but the corresponding package is not loaded. Terrain rendering will fall back, so you may see no visible change.",
                desired_source.land_label()
            ),
        );
    }

    if !cache_r.set_preferred_source(desired_source) {
        return;
    }

    // Terrain source flips only change how the shader resolves land ids into
    // EC atlas slots. The fixed texture arrays still back classic texmaps and
    // remain the fallback path for unmapped EC ids, so clearing or reuploading
    // them here creates visible patchwork while those fallbacks stream back in.
    let _ = (&texmap_2d_r, &time);

    let shader_images = create_land_shader_images(&mut images, land_shader_source_from_cache(&cache_r))
        .expect("Failed to rebuild land atlas shader images for source switch");
    if let Some(mat) = materials.get_mut(&shared_mat.0) {
        mat.extension.land_page_atlas = shader_images.page_atlas;
        mat.extension.land_page_lookup = shader_images.lookup;
    }

    console_logger::one(
        LogSev::Info,
        LogAbout::General,
        &format!(
            "Switched land texture source to {}.",
            desired_source.land_label()
        ),
    );

    // ── TEMPORARY EC package diagnostic ─────────────────────────────────────
    if desired_source == crate::configs::settings::ClientTextureSource::Ec {
        if let Some(ec) = &cache_r.tex_land_ec {
            let total_slots = ec.slots().len();
            let present_slots = ec.slots().iter().filter(|s| s.is_present()).count();
            let prov_count = ec.terrain_provenance().len();
            console_logger::one(
                LogSev::Info, LogAbout::General,
                &format!("[EC-DIAG] package: {present_slots}/{total_slots} slots present, {prov_count} provenance records"),
            );
        }
    }
    // ── END TEMPORARY EC package diagnostic ─────────────────────────────────

    for entity in chunks.iter() {
        commands
            .entity(entity)
            .insert(crate::core::render::scene::world::land::draw_mesh::PendingTextureBake);
    }
}

pub fn sys_setup_terrain_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    settings: Res<crate::configs::settings::Settings>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    tex_art_cc_r: Option<Res<crate::core::uo_files_loader::TexArtCcPackageRes>>,
    tex_art_ec_r: Option<Res<crate::core::uo_files_loader::TexArtEcPackageRes>>,
    tex_land_ec_r: Option<Res<crate::core::uo_files_loader::TexLandEcPackageRes>>,
) {
    log_system_add_startup::<LandTextureCachePlugin>(StartupSysSet::SetupSceneStage1, fname!());

    let residency_strategy = TextureResidencyStrategy::LruCache;
    let residency_plan = residency_strategy
        .preloads_full_collection()
        .then(|| texture_array::build_texture_residency_plan(&texmap_2d_r.0));
    let tex_land_ec_package = tex_land_ec_r.as_ref().map(|res| res.0.clone());
    // Shared layer budgeting keeps the terrain module responsible only for defining its
    // groups; the generic residency layer decides whether startup uses LRU-sized arrays
    // or exact-fit preloaded arrays.
    let layer_allocations = resolve_layer_allocations(
        residency_strategy,
        residency_plan.as_ref(),
        &LAND_TEXTURE_GROUP_LAYERS,
    );
    let small_layers = layer_allocations
        .iter()
        .find(|allocation| allocation.group == LandTextureSize::Small)
        .map(|allocation| allocation.layers)
        .unwrap_or(texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS);
    let big_layers = layer_allocations
        .iter()
        .find(|allocation| allocation.group == LandTextureSize::Big)
        .map(|allocation| allocation.layers)
        .unwrap_or(texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS);

    let handle_small = texture_array::create_gpu_texture_array(
        "land_small_texture_cache",
        &mut images,
        LandTextureSize::Small,
        small_layers,
    );
    let handle_big = texture_array::create_gpu_texture_array(
        "land_big_texture_cache",
        &mut images,
        LandTextureSize::Big,
        big_layers,
    );
    let mut land_texture_cache = cache::LandTextureCache::new(
        handle_small.clone(),
        handle_big.clone(),
        small_layers,
        big_layers,
        residency_strategy,
        settings.graphics.land_texture_source,
    );

    land_texture_cache.tex_art_cc = tex_art_cc_r.map(|r| r.0.clone());
    land_texture_cache.tex_land_cc = Some(texmap_2d_r.0.clone());
    land_texture_cache.tex_art_ec = tex_art_ec_r.map(|r| r.0.clone());
    land_texture_cache.tex_land_ec = tex_land_ec_package.clone();

    if let Some(plan) = residency_plan.as_ref() {
        land_texture_cache.prime_full_file_residency(
            plan,
            texmap_2d_r.0.clone(),
            Instant::now(),
        );
        console_logger::one(
            LogSev::Info,
            LogAbout::Startup,
            &format!(
                "Preloaded full texmaps collection into fixed texture-array layers ({} textures).",
                plan.total_texture_count()
            ),
        );
    }

    use crate::core::render::scene::world::land::{
        draw_mesh::SharedLandMaterial,
        mesh_material::{LandMaterialExtension, SceneUniform},
        tile_atlas::{AtlasParams, TileAtlas, TileAtlasImageHandle},
    };
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let shader_images = create_land_shader_images(&mut images, land_shader_source_from_cache(&land_texture_cache))
        .expect("Failed to build land atlas shader images");

    cmd.insert_resource(land_texture_cache);

    cmd.insert_resource(cache::TextureArrayImageHandles {
        small: handle_small.clone(),
        big: handle_big.clone(),
    });

    let page_texels = UVec2::splat(texture_array::TILE_ATLAS_PAGE_TEXELS);
    let tiles_per_page = UVec2::splat(texture_array::TILE_ATLAS_TILES_PER_PAGE);
    // Start small — the runtime will grow the atlas on demand if more pages
    // are needed (see sys_apply_tile_atlas_expansion).
    let max_layers = texture_array::TILE_ATLAS_INITIAL_LAYERS;
    let params = AtlasParams {
        page_texels,
        tiles_per_page,
        max_layers,
        world_pages_x: texture_array::TILE_ATLAS_WORLD_PAGES_X,
        _pad: UVec2::ZERO,
        page_to_layer: [bevy::math::UVec4::MAX; 64],
    };

    let tile_atlas = TileAtlas::new(params, texture_array::TILE_ATLAS_MAX_LAYERS);
    cmd.insert_resource(tile_atlas);

    let extent = Extent3d {
        width: page_texels.x,
        height: page_texels.y,
        depth_or_array_layers: max_layers,
    };
    let size_bytes = (extent.width
        * extent.height
        * extent.depth_or_array_layers
        * texture_array::TILE_ATLAS_BYTES_PER_TEXEL) as usize;
    let data: Vec<u8> = vec![0u8; size_bytes];

    let mut image = Image::new(
        extent,
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD, //| bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let atlas_image_handle = images.add(image);
    cmd.insert_resource(TileAtlasImageHandle(atlas_image_handle.clone()));
    let visual_grunge_texture = images.add(crate::util_lib::image::visual_grunge_image(128));

    let shared_mat =
        crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial {
            base: StandardMaterial {
                unlit: true,
                ..Default::default()
            },
            extension: LandMaterialExtension {
                texarray_small: handle_small,
                texarray_big: handle_big,
                land_page_atlas: shader_images.page_atlas,
                land_page_lookup: shader_images.lookup,
                tile_meta_atlas: atlas_image_handle,
                atlas_params: params,
                scene_uniform: SceneUniform {
                    camera_position:
                        crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER,
                    _pad_cam: 0.0,
                    light_direction: crate::core::constants::BAKED_GLOBAL_LIGHT.normalize(),
                    global_lighting: 1.0,
                    render_zoom: 1.0,
                    adaptive_zoom_simplification: 0.0,
                    _pad: Vec2::ZERO,
                },
                effects_uniform: Default::default(),
                global_lighting_uniform: Default::default(),
                land_lighting_uniform: Default::default(),
                static_light_uniform: Default::default(),
                visual_grunge_texture,
            },
        };

    let shared_mat_handle = materials.add(shared_mat);
    cmd.insert_resource(SharedLandMaterial(shared_mat_handle));
}

fn sys_apply_texture_array_expansion(
    mut images: ResMut<Assets<Image>>,
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut handles_r: ResMut<cache::TextureArrayImageHandles>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    _settings: Res<crate::configs::settings::Settings>,
    time: Res<Time<Real>>,
) {
    let now = time.last_update().unwrap_or_else(Instant::now);

    let (small_req, big_req) = cache_r.take_resize_requests();
    if small_req.is_none() && big_req.is_none() {
        return;
    }

    let mut resized_small: Option<u32> = None;
    let mut resized_big: Option<u32> = None;

    if let Some(req) = small_req {
        let new_layers = req.clamp(
            texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
            texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
        );
        if new_layers != cache_r.small.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_small_texture_cache",
                &mut images,
                LandTextureSize::Small,
                new_layers,
            );
            handles_r.small = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Small, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Small,
                texmap_2d_r.0.clone(),
                now,
            );
            resized_small = Some(new_layers);
        }
    }

    if let Some(req) = big_req {
        let new_layers = req.clamp(
            texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS,
            texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
        );
        if new_layers != cache_r.big.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_big_texture_cache",
                &mut images,
                LandTextureSize::Big,
                new_layers,
            );
            handles_r.big = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Big, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Big,
                texmap_2d_r.0.clone(),
                now,
            );
            resized_big = Some(new_layers);
        }
    }

    // CRITICAL: Only call materials.get_mut() when a resize actually happened.
    // Calling get_mut() every frame triggers Bevy's asset change detection,
    // which forces re-extraction of the entire material bind group (3 texture
    // arrays + 5 uniform buffers) for ALL chunk entities every frame. This was
    // the root cause of 95% GPU usage at idle — Bevy re-uploaded all texture
    // array bind groups every frame instead of reusing the cached GPU state.
    if resized_small.is_some() || resized_big.is_some() {
        if let Some(mat) = materials.get_mut(&shared_mat.0) {
            mat.extension.texarray_small = handles_r.small.clone();
            mat.extension.texarray_big = handles_r.big.clone();
        }

        console_logger::one(
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Resized terrain texture arrays: small={} layers, big={} layers.",
                cache_r.small.active_layers, cache_r.big.active_layers
            ),
        );
    }
}

/// Grows (or shrinks) the tile metadata atlas when `TileAtlas::requested_expansion`
/// has been set by the LRU paging logic (or when the shrink heuristic fires).
fn sys_apply_tile_atlas_expansion(
    mut images: ResMut<Assets<Image>>,
    mut tile_atlas: ResMut<crate::core::render::scene::world::land::tile_atlas::TileAtlas>,
    mut atlas_handle: ResMut<
        crate::core::render::scene::world::land::tile_atlas::TileAtlasImageHandle,
    >,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
    mut commands: Commands,
    chunks: Query<Entity, With<crate::core::render::scene::world::land::LCMesh>>,
) {
    let Some(new_layers) = tile_atlas.requested_expansion else {
        return;
    };
    let new_layers = new_layers.clamp(
        texture_array::TILE_ATLAS_INITIAL_LAYERS,
        tile_atlas.max_layers_limit,
    );
    if new_layers == tile_atlas.params.max_layers {
        tile_atlas.requested_expansion = None;
        return;
    }

    let old_layers = tile_atlas.params.max_layers;
    let page_texels = tile_atlas.params.page_texels;

    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let extent = Extent3d {
        width: page_texels.x,
        height: page_texels.y,
        depth_or_array_layers: new_layers,
    };
    let size_bytes = (extent.width
        * extent.height
        * extent.depth_or_array_layers
        * texture_array::TILE_ATLAS_BYTES_PER_TEXEL) as usize;
    let data: Vec<u8> = vec![0u8; size_bytes];

    let mut image = Image::new(
        extent,
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let new_handle = images.add(image);
    atlas_handle.0 = new_handle.clone();

    // Clear all page mappings — the mesh renderer will re-populate them.
    tile_atlas.apply_resize(new_layers);

    for e in chunks.iter() {
        commands
            .entity(e)
            .insert(crate::core::render::scene::world::land::draw_mesh::PendingTextureBake);
    }

    // Update the shared material so the shader sees the new texture.
    if let Some(mat) = materials.get_mut(&shared_mat.0) {
        mat.extension.tile_meta_atlas = new_handle;
        mat.extension.atlas_params = tile_atlas.params;
    }

    let direction = if new_layers > old_layers {
        "Expanded"
    } else {
        "Shrunk"
    };
    console_logger::one(
        LogSev::Info,
        LogAbout::Performance,
        &format!(
            "{direction} tile metadata atlas: {old_layers} → {new_layers} layers \
             ({} MB VRAM).",
            (page_texels.x as u64
                * page_texels.y as u64
                * new_layers as u64
                * texture_array::TILE_ATLAS_BYTES_PER_TEXEL as u64)
                / (1024 * 1024)
        ),
    );
}
