//! Source-backed UOStaticTerrainShader graphs, sharing the existing auxiliary atlas.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use udd_assets::tilemeta::{EcMaterialPhysicalPackage, TileMetaItemTextureRef, TileMetaPackage};
use uocf::enhanced::tileart::ArtMaterial;

const ITEM_CAPACITY: usize = 65_536;
const LOOKUP_WIDTH: usize = 256;
mod hue_mask;
pub(super) use hue_mask::append as append_sprite_hues;

#[derive(Resource, Clone)]
pub struct EcItemSurfaceImages {
    pub atlas: Handle<Image>,
    pub lookup: Handle<Image>,
}

fn source(refs: &[TileMetaItemTextureRef], stage: u32) -> Option<&TileMetaItemTextureRef> {
    refs.iter().find(|r| r.block_index == 0 && r.sampler_index == stage && r.is_world_art()
        && r.physical_package() == EcMaterialPhysicalPackage::Texture
        && r.uv_set <= 1 && r.texture_stretch.is_finite() && r.texture_stretch > 0.0)
}

fn has_source(refs: &[TileMetaItemTextureRef], stage: u32) -> bool {
    refs.iter().any(|r| r.block_index == 0 && r.sampler_index == stage)
}

fn sprite_source(refs: &[TileMetaItemTextureRef], stage: u32) -> Option<&TileMetaItemTextureRef> {
    source(refs, stage).filter(|r| r.uv_set == 0 && r.texture_stretch == 1.0)
}

fn same_sample(a: &TileMetaItemTextureRef, b: &TileMetaItemTextureRef) -> bool {
    a.texture_id == b.texture_id && a.texture_stretch == b.texture_stretch && a.uv_set == b.uv_set
}

fn append_material(
    records: &mut Vec<[u32; 4]>, item: u32, material: &ArtMaterial,
    refs: &[TileMetaItemTextureRef],
    assign: &mut impl FnMut(&TileMetaItemTextureRef) -> Option<(u32, UVec2)>,
) {
    if item as usize >= ITEM_CAPACITY || material.block_index != 0
        || material.shader_name.as_deref() != Some("UOStaticTerrainShader") { return; }
    let (Some(base), Some(temporary), Some(mask)) = (source(refs, 0), source(refs, 1), source(refs, 2))
        else { return; };
    // The base comes from the item-owned art slot. Never reinterpret a slot that
    // conversion populated from a different source in the material graph.
    if !base.is_primary_selected() { return; }
    let fourth = source(refs, 3);
    if fourth.is_none() && refs.iter().any(|r| r.block_index == 0 && r.sampler_index == 3) { return; }
    let temporary_is_base = same_sample(base, temporary);
    let fourth_is_mask = fourth.is_some_and(|r| same_sample(mask, r));
    let mut descriptors = [[0u32; 4]; 6];
    for (index, reference) in [Some(temporary), Some(mask), fourth].into_iter().enumerate() {
        let Some(reference) = reference else { continue; };
        if (index == 0 && temporary_is_base) || (index == 2 && fourth_is_mask) { continue; }
        let Some((layer, extent)) = assign(reference) else { return; };
        descriptors[index * 2] = [layer, extent.x, extent.y, reference.uv_set];
        descriptors[index * 2 + 1] = [reference.texture_stretch.to_bits(), 0, 0, 0];
    }
    // One direct header per 16-bit graphic; only present graphs have descriptors.
    // flags: bit0=source1 aliases base, bit1=source3 aliases mask, bit2=source3 exists.
    records[item as usize] = [records.len() as u32,
        u32::from(temporary_is_base) | (u32::from(fourth_is_mask) << 1) | (u32::from(fourth.is_some()) << 2),
        base.uv_set, base.texture_stretch.to_bits()];
    records.extend(descriptors);
}

pub(super) fn create_lookup(
    images: &mut Assets<Image>, tilemeta: Option<&TileMetaPackage>,
    mut assign: impl FnMut(&TileMetaItemTextureRef) -> Option<(u32, UVec2)>,
) -> Handle<Image> {
    let mut records = vec![[0u32; 4]; ITEM_CAPACITY];
    if let Some(tilemeta) = tilemeta {
        for tile in tilemeta.item_tiles() {
            let Some(material) = tilemeta.item_material(tile.tile_id, 0) else { continue; };
            append_material(&mut records, tile.tile_id, material, tilemeta.item_texture_refs(tile.tile_id), &mut assign);
        }
    }
    let height = records.len().div_ceil(LOOKUP_WIDTH);
    records.resize(height * LOOKUP_WIDTH, [0; 4]);
    images.add(Image::new(
        Extent3d { width: LOOKUP_WIDTH as u32, height: height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2, bytemuck::cast_slice(&records).to_vec(), TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> (ArtMaterial, [TileMetaItemTextureRef; 4]) {
        let material = ArtMaterial { shader_name: Some("UOStaticTerrainShader".into()), ..Default::default() };
        let refs = std::array::from_fn(|stage| TileMetaItemTextureRef {
            texture_id: if stage < 2 { 2000103 } else { 12943 },
            texture_type: 1, physical_package: EcMaterialPhysicalPackage::Texture as u8,
            flags: if stage == 0 { 2 } else { 0 }, sampler_index: stage as u32,
            texture_stretch: if stage < 2 { 4.0 } else { 1.0 },
            uv_set: u32::from(stage < 2), ..Default::default()
        });
        (material, refs)
    }

    #[test]
    fn surface_graph_keeps_item_masks_and_reuses_identical_samples() {
        let (material, refs) = graph();
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        let mut requested = Vec::new();
        append_material(&mut records, 12943, &material, &refs, &mut |r| {
            requested.push(r.texture_id); Some((7, UVec2::splat(256)))
        });
        assert_eq!(requested, [12943]);
        assert_eq!(records[12943], [ITEM_CAPACITY as u32, 7, 1, 4.0f32.to_bits()]);
        assert_eq!(records[ITEM_CAPACITY + 2], [7, 256, 256, 0]);
        assert_eq!(records[ITEM_CAPACITY + 3][0], 1.0f32.to_bits());
        assert_eq!(records[12942], [0; 4]);
    }

    #[test]
    fn surface_graph_rejects_partial_or_wrongly_owned_inputs() {
        let (material, refs) = graph();
        for wrong in [
            TileMetaItemTextureRef { physical_package: EcMaterialPhysicalPackage::LegacyTexture as u8, ..refs[2] },
            TileMetaItemTextureRef { texture_stretch: f32::NAN, ..refs[2] },
            TileMetaItemTextureRef { uv_set: 2, ..refs[2] },
        ] {
            let mut invalid = refs;
            invalid[2] = wrong;
            let mut records = vec![[0; 4]; ITEM_CAPACITY];
            append_material(&mut records, 12943, &material, &invalid, &mut |_| Some((1, UVec2::splat(256))));
            assert_eq!(records.len(), ITEM_CAPACITY);
            assert_eq!(records[12943], [0; 4]);
        }
        let mut invalid = refs;
        invalid[3].uv_set = 2;
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        append_material(&mut records, 12943, &material, &invalid, &mut |_| Some((1, UVec2::splat(256))));
        assert_eq!(records.len(), ITEM_CAPACITY);
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        append_material(&mut records, 12943, &material, &refs, &mut |_| None);
        assert_eq!(records.len(), ITEM_CAPACITY);
    }
}
