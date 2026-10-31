pub mod cache;
pub mod texture_array;
pub mod coverage;
#[cfg(test)]
mod liquid_sampling_tests;

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
    tex_art_cc::TexArtCcPackage,
    tex_land_cc::TexLandCcPackage,
    tex_land_ec::TexLandEcPackage,
    tex_aux_ec::TexAuxEcPackage,
    terrain_meta::TerrainMetaPackage,
    tilemeta::{TileMetaLandTile, TileMetaPackage},
};
use uocf::classic::land_texture::LandTextureSize;
use uocf::enhanced::terrain_definition::{TerrainLayerClass, TerrainLayerRole};
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

/// Compact Classic land-art tiles used for flat wet terrain.
///
/// Classic land art is a 44×44 isometric diamond in `art.mul`, whereas the
/// ordinary terrain path samples the corresponding square `texmaps.mul`
/// texture. Keeping a separate compact atlas lets flat wet cells use the same
/// authored pixels as shoreline statics without replacing either graphic.
struct ClassicLandArtImages {
    atlas: Handle<Image>,
    lookup: Handle<Image>,
}

const CLASSIC_LAND_ART_LOOKUP_WIDTH: u32 = 256;
const CLASSIC_LAND_ART_TILE_CAPACITY: u32 = 16_384;
const TILEMETA_FLAG_WET: u64 = 0x80;

/// GPU images for the `tex_aux_ec` two-base blend path.
///
/// `aux_atlas`: a `texture_2d_array<f32>` where layer 0 is a transparent sentinel
/// (absent) and layers 1..N each hold the RGBA pixels of one aux slot.
/// All layers share the same `AUX_ATLAS_TILE_SIZE` × `AUX_ATLAS_TILE_SIZE` extent;
/// images smaller than that are placed in the top-left corner.
///
/// `aux_lookup`: a `Rgba32Uint` 2D image of width `LAND_PAGE_LOOKUP_WIDTH`.
/// Each texel holds `[secondary_layer, secondary_repetition_q8, mask_layer, 0]`
/// for the tile at that index (0 = absent).
struct LandAuxAtlasImages {
    aux_atlas: Handle<Image>,
    aux_lookup: Handle<Image>,
    item_lookup: Handle<Image>,
    surface_lookup: Handle<Image>,
}

/// Side length (in pixels) of each layer in the aux atlas.
/// Aux textures (secondary base, alpha mask) are typically 64–256 px;
/// we normalise them all to this common size so the array is homogeneous.
const AUX_ATLAS_TILE_SIZE: u32 = 256;
const AUX_LOOKUP_WIDTH: u32 = 256;
const AUX_LOOKUP_TILE_CAPACITY: u32 = 16_384;

enum LandShaderAtlasSource<'a> {
    Cc(&'a TexLandCcPackage),
    Ec(&'a TexLandEcPackage),
}

const LAND_PAGE_LOOKUP_WIDTH: u32 = 256;
const LAND_PAGE_LOOKUP_TILE_CAPACITY: u32 = 16_384;
const LAND_PAGE_LOOKUP_ROLE_COUNT: u32 = 7;
const LAND_PAGE_LOOKUP_ROLE_BASE: u32 = 0;
const LAND_PAGE_LOOKUP_ROLE_DETAIL: u32 = 1;
const LAND_PAGE_LOOKUP_ROLE_MASK: u32 = 2;
const LAND_PAGE_LOOKUP_ROLE_NORMAL: u32 = 3;
const LAND_PAGE_LOOKUP_ROLE_ANIMATED: u32 = 4;
const LAND_PAGE_LOOKUP_ROLE_LIQUID_PARAMS: u32 = 5;
const LAND_PAGE_LOOKUP_ROLE_COVERAGE_PARAMS: u32 = 6;
/// `w` of a liquid-params entry: the scroll comes from the source water tweaks.
const LIQUID_PARAMS_FROM_SOURCE: u32 = 1;

/// The texture and stretch of the item that has `role` in a terrain tile's layer graph (roles are by
/// position per layer class, as the client binds them).
fn terrain_layer_texture_with_role(
    tile_id: u32,
    terrain_meta: &TerrainMetaPackage,
    role: TerrainLayerRole,
) -> Option<(u32, f32)> {
    terrain_meta
        .layers_for_tile(tile_id)
        .iter()
        .find(|layer| layer.role == role && layer.texture_id != 0)
        .map(|layer| (layer.texture_id, layer.texture_repetition))
}

/// The secondary base: colour B of a default or bump layer (item 1), weighted by 1 − blend alpha.
fn secondary_base_texture_id(tile_id: u32, terrain_meta: &TerrainMetaPackage) -> Option<(u32, f32)> {
    terrain_layer_texture_with_role(tile_id, terrain_meta, TerrainLayerRole::ColourB)
}

/// The alpha mask: the blend-alpha item of a default or bump layer (item 2).
fn alpha_mask_texture_id(tile_id: u32, terrain_meta: &TerrainMetaPackage) -> Option<(u32, f32)> {
    terrain_layer_texture_with_role(tile_id, terrain_meta, TerrainLayerRole::BlendAlpha)
}

/// The ripple/perturbation: the du/dv distortion item of a water layer (item 0).
fn ripple_texture_id(tile_id: u32, terrain_meta: &TerrainMetaPackage) -> Option<(u32, f32)> {
    terrain_layer_texture_with_role(tile_id, terrain_meta, TerrainLayerRole::Distortion)
}

/// Copy native RGBA texels into the top-left of a zeroed atlas layer.
/// This pads (or crops) the image; it never rescales its texel coordinates.
fn pad_aux_rgba(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    // Copy up to the min of src and dst dimensions into the top-left corner.
    let copy_w = src_w.min(dst_w);
    let copy_h = src_h.min(dst_h);
    for y in 0..copy_h {
        let src_x = (y * src_w * 4) as usize;
        let dst_x = (y * dst_w * 4) as usize;
        dst[dst_x..dst_x + (copy_w * 4) as usize]
            .copy_from_slice(&src[src_x..src_x + (copy_w * 4) as usize]);
    }
    dst
}

fn create_blank_aux_atlas_images(images: &mut Assets<Image>) -> LandAuxAtlasImages {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    };

    // Layer 0: transparent sentinel.
    let sentinel_bytes = vec![0u8; (AUX_ATLAS_TILE_SIZE * AUX_ATLAS_TILE_SIZE * 4) as usize];
    let mut aux_atlas = Image::new(
        Extent3d {
            width: AUX_ATLAS_TILE_SIZE,
            height: AUX_ATLAS_TILE_SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        sentinel_bytes,
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    aux_atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    aux_atlas.sampler = bevy::image::ImageSampler::linear();
    aux_atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    let lookup_height = (AUX_LOOKUP_TILE_CAPACITY + AUX_LOOKUP_WIDTH - 1) / AUX_LOOKUP_WIDTH;
    let mut aux_lookup = Image::new(
        Extent3d {
            width: AUX_LOOKUP_WIDTH,
            height: lookup_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; (AUX_LOOKUP_WIDTH * lookup_height * 16) as usize],
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    aux_lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    LandAuxAtlasImages {
        aux_atlas: images.add(aux_atlas),
        aux_lookup: images.add(aux_lookup),
        item_lookup: super::ec_item_liquid::create_lookup(images, None, |_| None),
        surface_lookup: super::ec_item_surface::create_lookup(images, None, |_| None),
    }
}

fn assign_aux_layer(
    package: &TexAuxEcPackage,
    texture_id: u32,
    assigned: &mut std::collections::HashMap<u32, u32>,
    layers: &mut Vec<Vec<u8>>,
) -> Option<(u32, UVec2)> {
    let slot = package.slot_for_texture_id(texture_id)?;
    let extent = UVec2::new(slot.width.into(), slot.height.into());
    if let Some(&layer) = assigned.get(&texture_id) { return Some((layer, extent)); }
    if extent.min_element() == 0 || extent.max_element() > AUX_ATLAS_TILE_SIZE {
        log::warn!("EC auxiliary source {texture_id} exceeds atlas cell extent: {extent:?}");
        return None;
    }
    let rgba = match package.load_rgba(texture_id) {
        Ok(Some(rgba)) => rgba,
        result => { log::warn!("EC auxiliary source {texture_id} could not be read: {result:?}"); return None; }
    };
    if rgba.len() != (extent.x * extent.y * 4) as usize {
        log::warn!("EC auxiliary source {texture_id} has an invalid RGBA byte count");
        return None;
    }
    let layer = layers.len() as u32;
    layers.push(pad_aux_rgba(&rgba, extent.x, extent.y, AUX_ATLAS_TILE_SIZE, AUX_ATLAS_TILE_SIZE));
    assigned.insert(texture_id, layer);
    Some((layer, extent))
}

/// Build the `tex_aux_ec` atlas and per-tile lookup texture from the auxiliary
/// texture package and terrain-layer sidecar.
///
/// Item water shares these image layers even without the optional terrain sidecar.
/// Missing auxiliary pixels leave the existing base path intact.
fn create_aux_atlas_images(
    images: &mut Assets<Image>,
    tex_aux_ec: Option<&TexAuxEcPackage>,
    terrain_meta: Option<&TerrainMetaPackage>,
    tex_land_ec: Option<&TexLandEcPackage>,
    tilemeta: Option<&TileMetaPackage>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
) -> LandAuxAtlasImages {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    };

    let Some(tex_aux_ec) = tex_aux_ec else {
        return create_blank_aux_atlas_images(images);
    };

    // Map texture_id → atlas layer index (1-based; 0 = absent sentinel).
    let mut texture_id_to_layer: std::collections::HashMap<u32, u32> =
        std::collections::HashMap::new();
    // Layer 0 is the sentinel. Collect the distinct texture_ids needed.
    let mut layers_rgba: Vec<Vec<u8>> = Vec::new();
    // Sentinel layer.
    layers_rgba.push(vec![
        0u8;
        (AUX_ATLAS_TILE_SIZE * AUX_ATLAS_TILE_SIZE * 4) as usize
    ]);

    let lookup_height = (AUX_LOOKUP_TILE_CAPACITY + AUX_LOOKUP_WIDTH - 1) / AUX_LOOKUP_WIDTH;
    let mut lookup_bytes = vec![0u8; (AUX_LOOKUP_WIDTH * lookup_height * 16) as usize];

    if let (Some(terrain_meta), Some(tex_land_ec)) = (terrain_meta, tex_land_ec) {
        for tile_id in 0..AUX_LOOKUP_TILE_CAPACITY {
            let sec_base = secondary_base_texture_id(tile_id, terrain_meta);
            let alpha_mask = alpha_mask_texture_id(tile_id, terrain_meta);
            let ripple = ripple_texture_id(tile_id, terrain_meta);

            // Liquid terrain: the material is the water layer class (UOWaterTerrainLayer).
            let is_liquid: u32 = u32::from(
                tex_land_ec.resolve_material_decision(tile_id).material_id
                    .and_then(|mid| tex_land_ec.source_material(mid))
                    .and_then(|entry| entry.texture.as_ref())
                    .is_some_and(|texture| texture.layer_class() == TerrainLayerClass::Water),
            );

            if sec_base.is_none() && alpha_mask.is_none() && ripple.is_none() && is_liquid == 0 {
                continue;
            }

            let mut assign_layer = |texture_id: u32| -> u32 {
                assign_aux_layer(tex_aux_ec, texture_id, &mut texture_id_to_layer, &mut layers_rgba)
                    .map_or(0, |(layer, _)| layer)
            };

            let (sec_layer, sec_rep_q8) = if let Some((tid, rep)) = sec_base {
                let layer = assign_layer(tid);
                let rep_q8 = if rep.is_finite() && rep > 0.0 {
                    (rep * 256.0).round().clamp(0.0, u16::MAX as f32) as u32
                } else {
                    0
                };
                (layer, rep_q8)
            } else {
                (0u32, 0u32)
            };

            let (mask_layer, _mask_rep_q8) = if let Some((tid, rep)) = alpha_mask {
                let layer = assign_layer(tid);
                let rep_q8 = if rep.is_finite() && rep > 0.0 {
                    (rep * 256.0).round().clamp(0.0, u16::MAX as f32) as u32
                } else {
                    0
                };
                (layer, rep_q8)
            } else {
                (0u32, 0u32)
            };

            let (ripple_layer, ripple_rep_q8) = if let Some((tid, rep)) = ripple {
                let layer = assign_layer(tid);
                let rep_q8 = if rep.is_finite() && rep > 0.0 {
                    (rep * 256.0).round().clamp(0.0, u16::MAX as f32) as u32
                } else {
                    0
                };
                (layer, rep_q8)
            } else {
                (0u32, 0u32)
            };

            if sec_layer == 0 && mask_layer == 0 && ripple_layer == 0 && is_liquid == 0 {
                continue;
            }

            // Write lookup entry:
            //   x: secondary_layer | (secondary_rep_q8 << 16)
            //   y: mask_layer
            //   z: ripple_layer | (ripple_rep_q8 << 16)
            //   w: is_liquid (1 = liquid terrain material, 0 = solid)
            let lookup_index = tile_id as usize;
            let texel_offset = lookup_index * 16;
            lookup_bytes[texel_offset..texel_offset + 4]
                .copy_from_slice(&((sec_layer & 0xFFFF) | (sec_rep_q8 << 16)).to_le_bytes());
            lookup_bytes[texel_offset + 4..texel_offset + 8]
                .copy_from_slice(&mask_layer.to_le_bytes());
            lookup_bytes[texel_offset + 8..texel_offset + 12]
                .copy_from_slice(&((ripple_layer & 0xFFFF) | (ripple_rep_q8 << 16)).to_le_bytes());
            lookup_bytes[texel_offset + 12..texel_offset + 16]
                .copy_from_slice(&is_liquid.to_le_bytes());
        }
    }
    let item_lookup = super::ec_item_liquid::create_lookup(images, tilemeta, |source| {
        let slot = tex_aux_ec.slot_for_texture_id(source.texture_id)?;
        // The package is keyed by numeric ID; a legacy/sprite family collision
        // must never supply a WorldArt shoreline mask.
        if slot.logical_family != source.texture_type {
            log::warn!("EC item liquid source {} has a different packed family; rebuild tex_aux_ec", source.texture_id);
            return None;
        }
        assign_aux_layer(tex_aux_ec, source.texture_id, &mut texture_id_to_layer, &mut layers_rgba)
    });
    let surface_lookup = super::ec_item_surface::create_lookup(images, tilemeta, |source| {
        let slot = tex_aux_ec.slot_for_texture_id(source.texture_id)?;
        if slot.logical_family != source.texture_type { return None; }
        assign_aux_layer(tex_aux_ec, source.texture_id, &mut texture_id_to_layer, &mut layers_rgba)
    });

    if let (Some(meta), Some(art)) = (tilemeta, tex_art_ec) {
        super::ec_item_surface::append_sprite_hues(&mut images.get_mut(&surface_lookup).expect("new item lookup"),
            meta, art, tex_aux_ec, &mut layers_rgba);
    }

    let layer_count = layers_rgba.len().max(1) as u32;
    let mut atlas_bytes: Vec<u8> = Vec::with_capacity(
        (AUX_ATLAS_TILE_SIZE * AUX_ATLAS_TILE_SIZE * 4 * layer_count) as usize,
    );
    for layer_data in &layers_rgba {
        atlas_bytes.extend_from_slice(layer_data);
    }

    let mut aux_atlas = Image::new(
        Extent3d {
            width: AUX_ATLAS_TILE_SIZE,
            height: AUX_ATLAS_TILE_SIZE,
            depth_or_array_layers: layer_count,
        },
        TextureDimension::D2,
        atlas_bytes,
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    aux_atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    aux_atlas.sampler = bevy::image::ImageSampler::linear();
    aux_atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    let mut aux_lookup = Image::new(
        Extent3d {
            width: AUX_LOOKUP_WIDTH,
            height: lookup_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        lookup_bytes,
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    aux_lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    LandAuxAtlasImages {
        aux_atlas: images.add(aux_atlas),
        aux_lookup: images.add(aux_lookup),
        item_lookup,
        surface_lookup,
    }
}

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
    atlas.sampler = bevy::image::ImageSampler::linear();
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

fn classic_land_art_payload_id(tile: &TileMetaLandTile) -> Option<u32> {
    if tile.flags & TILEMETA_FLAG_WET == 0 {
        return None;
    }

    // The tile metadata's texture id normally addresses texmaps.mul. We can
    // look the corresponding land art up from the shader's existing payload
    // only when that mapping is identity (or zero, where the renderer already
    // falls back to the original land graphic id).
    let texture_id = u32::from(tile.texture_id);
    (texture_id == 0 || texture_id == tile.tile_id).then_some(tile.tile_id)
}

fn create_blank_classic_land_art_images(images: &mut Assets<Image>) -> ClassicLandArtImages {
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

    let lookup_height =
        (CLASSIC_LAND_ART_TILE_CAPACITY + CLASSIC_LAND_ART_LOOKUP_WIDTH - 1)
            / CLASSIC_LAND_ART_LOOKUP_WIDTH;
    let mut lookup = Image::new(
        Extent3d {
            width: CLASSIC_LAND_ART_LOOKUP_WIDTH,
            height: lookup_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![
            0u8;
            (CLASSIC_LAND_ART_LOOKUP_WIDTH * lookup_height * 16) as usize
        ],
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    ClassicLandArtImages {
        atlas: images.add(atlas),
        lookup: images.add(lookup),
    }
}

fn create_classic_land_art_images(
    images: &mut Assets<Image>,
    tex_art_cc: Option<&TexArtCcPackage>,
    tilemeta: Option<&TileMetaPackage>,
) -> eyre::Result<ClassicLandArtImages> {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
        TextureViewDimension,
    };

    let (Some(tex_art_cc), Some(tilemeta)) = (tex_art_cc, tilemeta) else {
        return Ok(create_blank_classic_land_art_images(images));
    };

    let mut candidates = tilemeta
        .land_tiles()
        .iter()
        .filter_map(|tile| {
            let art_id = classic_land_art_payload_id(tile)?;
            if art_id >= CLASSIC_LAND_ART_TILE_CAPACITY {
                return None;
            }
            let slot = *tex_art_cc.present_slot(art_id)?;
            (slot.is_land() && slot.width != 0 && slot.height != 0).then_some((art_id, slot))
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return Ok(create_blank_classic_land_art_images(images));
    }

    // Group by package page so each source page is decoded at most once. The
    // normal sprite cache streams those pages; this one-off derived atlas must
    // not repeatedly decode a full page for every wet tile on it.
    candidates.sort_unstable_by_key(|(art_id, slot)| (slot.page_index, *art_id));

    let atlas_width = candidates
        .iter()
        .map(|(_, slot)| u32::from(slot.width))
        .max()
        .unwrap_or(1)
        .max(1);
    let atlas_height = candidates
        .iter()
        .map(|(_, slot)| u32::from(slot.height))
        .max()
        .unwrap_or(1)
        .max(1);
    let layer_count = candidates.len() as u32 + 1; // layer 0 is the absent sentinel
    let layer_byte_len = (atlas_width * atlas_height * 4) as usize;
    let mut atlas_bytes = vec![0u8; layer_byte_len * layer_count as usize];

    let lookup_height =
        (CLASSIC_LAND_ART_TILE_CAPACITY + CLASSIC_LAND_ART_LOOKUP_WIDTH - 1)
            / CLASSIC_LAND_ART_LOOKUP_WIDTH;
    let mut lookup_bytes =
        vec![0u8; (CLASSIC_LAND_ART_LOOKUP_WIDTH * lookup_height * 16) as usize];

    let mut decoded_page_index = u32::MAX;
    let mut decoded_page_rgba = Vec::new();
    let mut decoded_page_width = 0u32;
    let mut decoded_page_height = 0u32;

    for (candidate_index, (art_id, slot)) in candidates.into_iter().enumerate() {
        if decoded_page_index != slot.page_index {
            let page = tex_art_cc
                .pages()
                .get(slot.page_index as usize)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "missing tex_art_cc page metadata for Classic land art {}",
                        slot.page_index
                    )
                })?;
            decoded_page_rgba = tex_art_cc.read_page_rgba(slot.page_index)?;
            decoded_page_index = slot.page_index;
            decoded_page_width = page.used_width;
            decoded_page_height = page.used_height;
        }

        let slot_width = u32::from(slot.width);
        let slot_height = u32::from(slot.height);
        let slot_x = u32::from(slot.x);
        let slot_y = u32::from(slot.y);
        if slot_x + slot_width > decoded_page_width
            || slot_y + slot_height > decoded_page_height
        {
            eyre::bail!(
                "Classic land art {art_id} rect {slot_width}x{slot_height} at {slot_x},{slot_y} exceeds page {} extent {}x{}",
                slot.page_index,
                decoded_page_width,
                decoded_page_height,
            );
        }

        let layer = candidate_index as u32 + 1;
        let layer_start = layer as usize * layer_byte_len;
        let copy_row_bytes = slot_width as usize * 4;
        for row in 0..slot_height {
            let source_start = ((slot_y + row) * decoded_page_width + slot_x) as usize * 4;
            let source_end = source_start + copy_row_bytes;
            let target_start = layer_start + row as usize * atlas_width as usize * 4;
            let target_end = target_start + copy_row_bytes;
            if source_end > decoded_page_rgba.len() {
                eyre::bail!(
                    "Classic land art {art_id} source pixels exceed decoded page {}",
                    slot.page_index,
                );
            }
            atlas_bytes[target_start..target_end]
                .copy_from_slice(&decoded_page_rgba[source_start..source_end]);
        }

        let lookup_offset = art_id as usize * 16;
        for (component_index, value) in [
            layer,
            slot_width,
            slot_height,
            u32::from(slot.upscale_factor.max(1)),
        ]
        .into_iter()
        .enumerate()
        {
            let start = lookup_offset + component_index * 4;
            lookup_bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    let mut atlas = Image::new(
        Extent3d {
            width: atlas_width,
            height: atlas_height,
            depth_or_array_layers: layer_count,
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

    let mut lookup = Image::new(
        Extent3d {
            width: CLASSIC_LAND_ART_LOOKUP_WIDTH,
            height: lookup_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        lookup_bytes,
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    Ok(ClassicLandArtImages {
        atlas: images.add(atlas),
        lookup: images.add(lookup),
    })
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
        match source {
            LandShaderAtlasSource::Ec(_) => TextureFormat::Rgba8Unorm,
            LandShaderAtlasSource::Cc(_) => TextureFormat::Rgba8UnormSrgb,
        },
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = bevy::image::ImageSampler::linear();
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
                if let Some(params) = coverage::EcTerrainParams::for_tile(package, tile_id) {
                    write_land_lookup_entry(&mut lookup_bytes, tile_id, LAND_PAGE_LOOKUP_ROLE_COVERAGE_PARAMS, params.0);
                }
                if let Some(source) = package.resolve_material_decision(tile_id).material_id
                    .and_then(|id| package.source_material(id))
                    .and_then(|entry| entry.texture.as_ref())
                    .filter(|texture| texture.layer_class() == TerrainLayerClass::Water) {
                    // Original Technique[0] selects coordinates; Tweaks[0..2]
                    // scroll item 0 (du/dv) in UV units per second
                    // (UOWaterTerrainLayer::Draw, see ec_liquid.wesl).
                    write_land_lookup_entry(&mut lookup_bytes, tile_id, LAND_PAGE_LOOKUP_ROLE_LIQUID_PARAMS, [
                        source.techniques().first().copied().unwrap_or(0) as u32,
                        source.tweaks().first().copied().unwrap_or(0.0).to_bits(),
                        source.tweaks().get(1).copied().unwrap_or(0.0).to_bits(),
                        LIQUID_PARAMS_FROM_SOURCE,
                    ]);
                }
                let [layer0, layer1, layer2, layer3, animated] = package.resolve_material_render_layers(tile_id);
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
                        (LAND_PAGE_LOOKUP_ROLE_ANIMATED, animated),
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
        app.add_systems(Update, coverage::sys_prepare_stack_cache
            .after(SceneRenderLandSysSet::RenderLandChunks)
            .run_if(in_state(crate::core::app_states::AppState::InGame)));

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
    if let Some(mut mat) = materials.get_mut(&shared_mat.0) {
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
    tilemeta_r: Option<Res<crate::core::uo_files_loader::TileMetaPackageRes>>,
    tex_art_cc_r: Option<Res<crate::core::uo_files_loader::TexArtCcPackageRes>>,
    tex_art_ec_r: Option<Res<crate::core::uo_files_loader::TexArtEcPackageRes>>,
    tex_land_ec_r: Option<Res<crate::core::uo_files_loader::TexLandEcPackageRes>>,
    tex_aux_ec_r: Option<Res<crate::core::uo_files_loader::TexAuxEcPackageRes>>,
    terrain_meta_r: Option<Res<crate::core::uo_files_loader::TerrainMetaPackageRes>>,
    map_planes: Res<crate::core::uo_files_loader::MapPlanesRes>,
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

    land_texture_cache.tex_art_cc = tex_art_cc_r.as_ref().map(|r| r.0.clone());
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

    let classic_land_art_images = create_classic_land_art_images(
        &mut images,
        tex_art_cc_r.as_deref().map(|resource| resource.0.as_ref()),
        tilemeta_r.as_deref().map(|resource| resource.0.as_ref()),
    )
    .expect("Failed to build Classic wet land-art shader images");

    let aux_images = create_aux_atlas_images(
        &mut images,
        tex_aux_ec_r.as_deref().map(|r| r.0.as_ref()),
        terrain_meta_r.as_deref().map(|r| r.0.as_ref()),
        tex_land_ec_package.as_deref(),
        tilemeta_r.as_deref().map(|r| r.0.as_ref()),
        land_texture_cache.tex_art_ec.as_deref(),
    );
    cmd.insert_resource(super::ec_item_liquid::EcItemLiquidImages {
        atlas: aux_images.aux_atlas.clone(),
        lookup: aux_images.item_lookup.clone(),
    });
    cmd.insert_resource(super::ec_item_surface::EcItemSurfaceImages {
        atlas: aux_images.aux_atlas.clone(),
        lookup: aux_images.surface_lookup.clone(),
    });

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
    let base_dimensions = map_planes.0.iter().flatten().fold(UVec2::ONE, |size, plane| {
        size.max(UVec2::new(plane.size_blocks.width.div_ceil(4), plane.size_blocks.height.div_ceil(4)))
    });
    let terrain_bases = coverage::EcTerrainBases::new(&mut images, base_dimensions, tex_land_ec_package.as_deref());
    let ec_terrain_bases = terrain_bases.image.clone();
    let ec_terrain_stacks = terrain_bases.stack_image();
    cmd.insert_resource(terrain_bases);
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
                    ec_reflection_camera_distance:
                        crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER.length(),
                    _pad: 0.0,
                },
                effects_uniform: Default::default(),
                global_lighting_uniform: Default::default(),
                land_lighting_uniform: Default::default(),
                static_light_uniform: Default::default(),
                visual_grunge_texture,
                ec_aux_atlas: aux_images.aux_atlas,
                ec_aux_lookup: aux_images.aux_lookup,
                classic_land_art_atlas: classic_land_art_images.atlas,
                classic_land_art_lookup: classic_land_art_images.lookup,
                ec_terrain_bases,
                ec_terrain_stacks,
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
        if let Some(mut mat) = materials.get_mut(&shared_mat.0) {
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
    if let Some(mut mat) = materials.get_mut(&shared_mat.0) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::{uuid::Uuid, AssetId};
    use bevy::shader::{
        Shader, ShaderCache, ShaderCacheError, ShaderCacheSource, ValidateShader,
    };
    use wgpu::naga;

    /// Parses and validates the composed WGSL with naga, standing in for the GPU
    /// shader module creation that the real `PipelineCache` would perform.
    fn validate_shader_module(
        _device: &(),
        source: ShaderCacheSource<'_>,
        _validation: &ValidateShader,
    ) -> Result<(), ShaderCacheError> {
        let ShaderCacheSource::Wgsl(wgsl) = source else {
            return Err(ShaderCacheError::CreateShaderModule("expected WGSL output".into()));
        };
        let module = naga::front::wgsl::parse_str(&wgsl)
            .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&wgsl)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&wgsl)))?;
        Ok(())
    }

    fn land_tile(tile_id: u32, texture_id: u16, flags: u64) -> TileMetaLandTile {
        TileMetaLandTile {
            tile_id,
            texture_id,
            tile_type: 0,
            _pad1: 0,
            flags,
            radar_color: [0; 4],
            name: [0; 20],
        }
    }

    #[test]
    fn classic_land_art_payload_requires_wet_identity_mapping() {
        assert_eq!(
            classic_land_art_payload_id(&land_tile(168, 0, TILEMETA_FLAG_WET)),
            Some(168)
        );
        assert_eq!(
            classic_land_art_payload_id(&land_tile(310, 310, TILEMETA_FLAG_WET)),
            Some(310)
        );
        assert_eq!(classic_land_art_payload_id(&land_tile(168, 0, 0)), None);
        assert_eq!(
            classic_land_art_payload_id(&land_tile(168, 42, TILEMETA_FLAG_WET)),
            None
        );
    }

    #[test]
    fn land_sampling_shader_composes() {
        let mut shader_cache = ShaderCache::<(), ()>::new((), validate_shader_module);
        let shader_id = |n: u128| AssetId::<Shader>::Uuid {
            uuid: Uuid::from_u128(n),
        };

        let shaders = [
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/uo_coords.wesl"),
                "shaders/world/uo_coords.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/common_bindings.wesl"),
                "shaders/world/common_bindings.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/land/land_bindings.wesl"),
                "shaders/world/land/land_bindings.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/pixel_art_filters.wesl"),
                "shaders/world/pixel_art_filters.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/texture_data.wesl"),
                "shaders/world/texture_data.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/ec_liquid.wesl"),
                "shaders/world/ec_liquid.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/ec_item_liquid.wesl"),
                "shaders/world/ec_item_liquid.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/ec_item_surface.wesl"),
                "shaders/world/ec_item_surface.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/land/sampling.wesl"),
                "shaders/world/land/sampling.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/land/atlas.wesl"),
                "shaders/world/land/atlas.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/land/coverage.wesl"),
                "shaders/world/land/coverage.wesl",
            ),
            Shader::from_wesl(
                include_str!("../../../assets/shaders/world/land/liquid_bank.wesl"),
                "shaders/world/land/liquid_bank.wesl",
            ),
        ];
        for (n, shader) in (1..).zip(shaders) {
            shader_cache.set_shader(shader_id(n), shader);
        }

        // WESL strips everything an entry point does not reach, so a library module
        // compiled on its own composes to nothing. Compose a minimal entry point that
        // calls the sampling functions used by `land/main.wesl` instead.
        let root_id = shader_id(0);
        shader_cache.set_shader(
            root_id,
            Shader::from_wesl(
                r#"
import package::shaders::world::land::land_bindings::{TileUniform, land_page_atlas, land_page_lookup, tex_small_sampler};
import package::shaders::world::ec_liquid::{sample_ec_liquid, ec_liquid_reflection_uv, ec_liquid_world_uv};
import package::shaders::world::ec_item_liquid::apply_ec_item_liquid;
import package::shaders::world::ec_item_surface::apply_ec_item_surface;
import package::shaders::world::land::coverage::{ec_terrain_cache, ec_cache_layer, ec_layer_tile};
import package::shaders::world::land::liquid_bank::{ec_liquid_bank, ec_bank_vertex};
import package::shaders::world::land::sampling::{
    LandGradients, apply_ec_aux_two_base_blend, apply_sharpening, blurred_albedo, classic_land_art_is_present,
    sample_classic_land_art, sample_ec_material_albedo,
    sample_ec_material_normal_world, sample_ec_ripple_normal, sample_tile_albedo,
};

@fragment
fn fragment() -> @location(0) vec4<f32> {
    let cache = ec_terrain_cache(vec2<f32>(0.0));
    var tile = ec_layer_tile(ec_cache_layer(cache, 0u).material, 0.0);
    tile.tile_height = ec_bank_vertex(vec2<i32>(0), tile).y;
    let bank = ec_liquid_bank(vec3<f32>(0.0), tile, 0u, 1.0);
    tile.tile_height += bank.weight;
    let gradients = LandGradients(vec2<f32>(0.01, 0.0), vec2<f32>(0.0, 0.01));
    let uv = vec2<f32>(0.0);
    var color = sample_tile_albedo(uv, tile, gradients) + sample_ec_material_albedo(uv, uv, tile, gradients);
    color = apply_ec_aux_two_base_blend(color, uv, tile);
    color = apply_sharpening(color, uv, tile, 0.0, gradients) + blurred_albedo(uv, tile, 0.0, uv, gradients);
    if (classic_land_art_is_present(0u)) {
        color += sample_classic_land_art(uv, 0u);
    }
    let normal = sample_ec_material_normal_world(uv, tile, gradients);
    let ripple = sample_ec_ripple_normal(uv, 0u, 0.0, 0.0);
    let reflection_uv = ec_liquid_reflection_uv(vec3<f32>(0.0, 0.0, -1.0));
    color += sample_ec_liquid(land_page_atlas, land_page_lookup, tex_small_sampler,
        0u, uv, ec_liquid_world_uv(uv), reflection_uv, 0.0, true, true).rgb;
    let liquid = apply_ec_item_liquid(vec4<f32>(color, normal.w + ripple.x),
        land_page_atlas, land_page_lookup, tex_small_sampler, 6038u,
        uv, uv, 0.0, true, true);
    return apply_ec_item_surface(liquid, land_page_atlas, land_page_lookup, tex_small_sampler,
        vec4<u32>(0u), uv, uv, true, true);
}
"#,
                "shaders/test/land_sampling_root.wesl",
            ),
        );
        shader_cache.get(0, root_id, &[]).unwrap();
    }
}
