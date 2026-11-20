//! The `tex_aux_ec` array that item liquids and item surfaces sample, built with their two lookups: only the textures
//! the lookups reference become layers (`aux_layers`), the Kingdom Reborn items' after the Enhanced ones'.

use crate::prelude::*;
use bevy::prelude::*;
use udd_assets::{tex_aux_ec::TexAuxEcPackage, tilemeta::TileMetaPackage};
use super::aux_layers;

/// GPU images of the `tex_aux_ec` textures that item liquids and item surfaces sample.
///
/// `aux_atlas`: a `texture_2d_array<f32>` where layer 0 is a transparent sentinel
/// (absent) and layers 1..N each hold the RGBA pixels of one aux slot, or its BC7
/// blocks when every texture of the package is BC7 (`aux_layers::AuxLayerFormat`).
/// All layers share the same `AUX_ATLAS_TILE_SIZE` × `AUX_ATLAS_TILE_SIZE` extent;
/// images smaller than that are placed in the top-left corner.
pub(super) struct LandAuxAtlasImages {
    pub(super) aux_atlas: Handle<Image>,
    pub(super) item_lookup: Handle<Image>,
    pub(super) surface_lookup: Handle<Image>,
}

/// Side length (in pixels) of each layer in the aux atlas.
/// Aux textures (item liquid and surface masks) are typically 64–256 px;
/// we normalise them all to this common size so the array is homogeneous.
pub(super) const AUX_ATLAS_TILE_SIZE: u32 = 256;

/// Copy native RGBA texels into the top-left of a zeroed atlas layer.
/// This pads (or crops) the image; it never rescales its texel coordinates.
pub(super) fn pad_aux_rgba(
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

    LandAuxAtlasImages {
        aux_atlas: images.add(aux_atlas),
        item_lookup: super::super::ec_item_liquid::create_lookup(images, &[], |_, _| None),
        surface_lookup: super::super::ec_item_surface::create_lookup(images, &[], |_, _| None),
    }
}

/// The layer of `texture_id`, whose `slot` (`ec_item_surface::ItemRecordSource::aux_slot`) has its payload in
/// `package`. `assigned` is keyed by package and payload id: equal payload ids of two packages are different images.
fn assign_aux_layer(
    package: &TexAuxEcPackage,
    slot: udd_assets::tex_aux_ec::SlotInfo,
    texture_id: u32,
    assigned: &mut std::collections::HashMap<(usize, u32), u32>,
    layers: &mut aux_layers::AuxLayers,
) -> Option<(u32, UVec2)> {
    let extent = UVec2::new(slot.width.into(), slot.height.into());
    let payload_id = slot.payload_texture_id;
    let source = std::ptr::from_ref(package) as usize;
    if let Some(&layer) = assigned.get(&(source, payload_id)) { return Some((layer, extent)); }
    if extent.min_element() == 0 || extent.max_element() > AUX_ATLAS_TILE_SIZE {
        log::warn!("EC auxiliary source {texture_id} exceeds atlas cell extent: {extent:?}");
        return None;
    }
    let image = match aux_layers::AuxImage::load(package, payload_id, extent, layers.format()) {
        Ok(Some(image)) => image,
        Ok(None) => { log::warn!("EC auxiliary source {texture_id} has no payload"); return None; }
        Err(error) => { log::warn!("EC auxiliary source {texture_id} could not be read: {error:#}"); return None; }
    };
    let layer = layers.push_image(&image);
    assigned.insert((source, payload_id), layer);
    Some((layer, extent))
}

/// Build the `tex_aux_ec` atlas and the item liquid and item surface lookups from the auxiliary
/// texture package: the atlas holds only the textures those lookups reference.
/// With the Kingdom Reborn item packages, `kr_items`, the lookups hold their records too, from
/// `ec_item_surface::KR_ITEM_RECORD_BASE`, and the atlas their textures, for the statics drawn from them.
///
/// Missing auxiliary pixels leave the existing base path intact.
pub(super) fn create_aux_atlas_images(
    images: &mut Assets<Image>,
    tex_aux_ec: Option<&TexAuxEcPackage>,
    tilemeta: Option<&TileMetaPackage>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    kr_items: Option<&crate::content::KrItemPackagesRes>,
) -> LandAuxAtlasImages {
    use super::super::ec_item_surface::{ItemRecordSource, KR_ITEM_RECORD_BASE};
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureUsages, TextureViewDescriptor, TextureViewDimension,
    };

    let Some(tex_aux_ec) = tex_aux_ec else {
        return create_blank_aux_atlas_images(images);
    };

    // The item sources: the Enhanced items, then the Kingdom Reborn items.
    // The Kingdom Reborn packages are overlays over the Enhanced ones.
    let sources = tilemeta.map(|tilemeta| ItemRecordSource {
        base: 0, tilemeta, art: tex_art_ec, aux: tex_aux_ec, base_art: tex_art_ec, base_aux: tex_aux_ec,
    })
        .into_iter()
        .chain(kr_items.map(|kr| ItemRecordSource {
            base: KR_ITEM_RECORD_BASE, tilemeta: &kr.tilemeta, art: Some(&kr.art), aux: &kr.aux,
            base_art: tex_art_ec, base_aux: tex_aux_ec,
        }))
        .collect::<Vec<_>>();
    // Map (package, payload id) → atlas layer index (1-based; 0 = absent sentinel).
    let mut texture_id_to_layer: std::collections::HashMap<(usize, u32), u32> =
        std::collections::HashMap::new();
    // Layer 0 is the sentinel. Collect the distinct texture_ids needed.
    // The layers keep the packages' BC7 blocks when every texture of each is BC7, and decode to RGBA8 otherwise.
    let format = sources.iter().map(|source| aux_layers::AuxLayerFormat::for_package(source.aux))
        .reduce(|a, b| if a == b { a } else { aux_layers::AuxLayerFormat::Rgba8 })
        .unwrap_or_else(|| aux_layers::AuxLayerFormat::for_package(tex_aux_ec));
    let mut layers = aux_layers::AuxLayers::new(format);

    let liquid_sources = sources.iter().map(|source| (source.base, source.tilemeta)).collect::<Vec<_>>();
    let item_lookup = super::super::ec_item_liquid::create_lookup(images, &liquid_sources, |index, source| {
        let (package, slot) = sources[index].aux_slot(source.texture_id)?;
        // The package is keyed by numeric ID; a legacy/sprite family collision
        // must never supply a WorldArt shoreline mask.
        if slot.logical_family != source.texture_type {
            log::warn!("EC item liquid source {} has a different packed family; rebuild tex_aux_ec", source.texture_id);
            return None;
        }
        assign_aux_layer(package, slot, source.texture_id, &mut texture_id_to_layer, &mut layers)
    });
    let surface_lookup = super::super::ec_item_surface::create_lookup(images, &sources, |index, source| {
        let (package, slot) = sources[index].aux_slot(source.texture_id)?;
        if slot.logical_family != source.texture_type { return None; }
        assign_aux_layer(package, slot, source.texture_id, &mut texture_id_to_layer, &mut layers)
    });

    for source in &sources {
        // The Kingdom Reborn sprites draw only the statics that continue the land (`statics_collect`
        // `kingdom_reborn_static_visual_kind`), land materials by the Enhanced data or by Kingdom Reborn's: only those
        // need their hue coverage.
        let land_item = |source: &ItemRecordSource, tile_id: u32| source.tilemeta.uses_water_shader(tile_id)
            || source.art_slot(tile_id).is_some_and(|slot| slot.is_land());
        let drawn_item = |tile_id: u32| source.base == 0 || land_item(source, tile_id)
            || sources.first().is_some_and(|enhanced| enhanced.base == 0 && land_item(enhanced, tile_id));
        super::super::ec_item_surface::append_sprite_hues(&mut images.get_mut(&surface_lookup).expect("new item lookup"),
            *source, drawn_item, &mut layers);
    }

    let layer_count = layers.len().max(1) as u32;
    let texture_format = layers.format().texture_format();
    let atlas_bytes = layers.into_bytes();
    console_logger::one(
        LogSev::Info,
        LogAbout::RenderWorldLand,
        &format!(
            "EC auxiliary array: {layer_count} layers of {AUX_ATLAS_TILE_SIZE}x{AUX_ATLAS_TILE_SIZE} {texture_format:?}, {} bytes",
            atlas_bytes.len(),
        ),
    );

    let mut aux_atlas = Image::new(
        Extent3d {
            width: AUX_ATLAS_TILE_SIZE,
            height: AUX_ATLAS_TILE_SIZE,
            depth_or_array_layers: layer_count,
        },
        TextureDimension::D2,
        atlas_bytes,
        texture_format,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    aux_atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    aux_atlas.sampler = bevy::image::ImageSampler::linear();
    aux_atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    LandAuxAtlasImages {
        aux_atlas: images.add(aux_atlas),
        item_lookup,
        surface_lookup,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn missing_aux_aliases_share_one_upload_layer() {
        let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let package = TexAuxEcPackage::load(root.join("tex_aux_ec.uddp")).unwrap();
        let mut assigned = std::collections::HashMap::new();
        let mut layers = aux_layers::AuxLayers::new(aux_layers::AuxLayerFormat::Rgba8);
        let mut aliases = 0;
        for slot in package.slots().iter().filter(|slot| slot.is_missing_source()) {
            let info = package.slot_for_texture_id(slot.texture_id).unwrap();
            assert_eq!(assign_aux_layer(&package, info, slot.texture_id, &mut assigned, &mut layers),
                Some((1, UVec2::new(256, 256))));
            aliases += 1;
        }
        assert!(aliases > 1, "inventory did not exercise shared missing-source aliases");
        assert_eq!(assigned.len(), 1);
        assert_eq!(layers.len(), 2);
        let original = package.load_rgba(udd_assets::tex_aux_ec::TEX_AUX_EC_MISSING_TEXTURE_ID)
            .unwrap().unwrap();
        assert_eq!(layers.layer(1), pad_aux_rgba(&original, 256, 256,
            AUX_ATLAS_TILE_SIZE, AUX_ATLAS_TILE_SIZE));
    }

}
