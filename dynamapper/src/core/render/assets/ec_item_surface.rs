//! Source-backed UOStaticTerrainShader graphs, sharing the existing auxiliary atlas.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use udd_assets::tilemeta::{EcMaterialPhysicalPackage, TileMetaItemTextureRef, TileMetaPackage};
use uocf::enhanced::tileart::ArtMaterial;

/// Item records per item source (`ItemRecordSource`): one per 16-bit graphic.
const ITEM_CAPACITY: usize = 65_536;
/// The item sources of the item lookups: the Enhanced items, then the Kingdom Reborn items at `KR_ITEM_RECORD_BASE`.
pub(crate) const ITEM_SOURCE_COUNT: usize = 2;
/// Where the Kingdom Reborn items' records start in the item lookups (this one and `ec_item_liquid`'s): a static drawn
/// from the Kingdom Reborn item packages adds it to its graphic in `material_item_id`.
pub(crate) const KR_ITEM_RECORD_BASE: u32 = ITEM_CAPACITY as u32;
/// The item headers of the lookup, one per graphic of each source; the descriptors follow them.
const ITEM_HEADER_COUNT: usize = ITEM_CAPACITY * ITEM_SOURCE_COUNT;
const LOOKUP_WIDTH: usize = 256;
/// Item position of the static terrain shader's fourth map (coverage alpha).
const FOURTH_SOURCE_POSITION: u8 = 3;
mod hue_mask;
mod water;
pub(super) use hue_mask::append as append_sprite_hues;

/// One item source of the item lookups: its tile data, item art and material inputs, and where its records start.
#[derive(Clone, Copy)]
pub(crate) struct ItemRecordSource<'a> {
    /// What the source adds to a graphic for its record: 0 for the Enhanced items, or `KR_ITEM_RECORD_BASE`.
    pub base: u32,
    pub tilemeta: &'a TileMetaPackage,
    pub art: Option<&'a udd_assets::tex_art_ec::TexArtEcPackage>,
    pub aux: &'a udd_assets::tex_aux_ec::TexAuxEcPackage,
    /// The Enhanced packages an overlay `art` or `aux` (the Kingdom Reborn ones) leaves what it lists to; the Enhanced
    /// packages for the Enhanced items, which read only themselves.
    pub base_art: Option<&'a udd_assets::tex_art_ec::TexArtEcPackage>,
    pub base_aux: &'a udd_assets::tex_aux_ec::TexAuxEcPackage,
}

impl<'a> ItemRecordSource<'a> {
    /// The present art slot of item `tile_id`, read through an overlay `art`.
    pub fn art_slot(&self, tile_id: u32) -> Option<&'a udd_assets::tex_art_ec::TexArtEcSlotRecord> {
        let art = self.art?;
        art.overlay_slot(self.base_art.unwrap_or(art), tile_id).map(|(_, slot)| slot)
    }

    /// The slot of `texture_id`, read through an overlay `aux`, with the package holding its payload.
    pub fn aux_slot(&self, texture_id: u32)
        -> Option<(&'a udd_assets::tex_aux_ec::TexAuxEcPackage, udd_assets::tex_aux_ec::SlotInfo)> {
        self.aux.overlay_slot(self.base_aux, texture_id)
    }
}

#[derive(Resource, Clone)]
pub struct EcItemSurfaceImages {
    pub atlas: Handle<Image>,
    pub lookup: Handle<Image>,
}

/// The block-0 item at position `stage`: the client binds item i to the shader's map i.
fn source(refs: &[TileMetaItemTextureRef], stage: u8) -> Option<&TileMetaItemTextureRef> {
    refs.iter().find(|r| r.block_index == 0 && r.item_index == stage && r.is_world_art()
        && r.physical_package() == EcMaterialPhysicalPackage::Texture
        && r.uv_set <= 1 && r.texture_stretch.is_finite() && r.texture_stretch > 0.0)
}

fn has_source(refs: &[TileMetaItemTextureRef], stage: u8) -> bool {
    refs.iter().any(|r| r.block_index == 0 && r.item_index == stage)
}

fn sprite_source(refs: &[TileMetaItemTextureRef], stage: u8) -> Option<&TileMetaItemTextureRef> {
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
    if item as usize >= ITEM_HEADER_COUNT || material.block_index != 0
        || material.shader_name.as_deref() != Some("UOStaticTerrainShader") { return; }
    let (Some(base), Some(temporary), Some(mask)) = (source(refs, 0), source(refs, 1), source(refs, 2))
        else { return; };
    // The base comes from the item-owned art slot. Never reinterpret a slot that
    // conversion populated from a different source in the material graph.
    if !base.is_primary_selected() { return; }
    let fourth = source(refs, FOURTH_SOURCE_POSITION);
    if fourth.is_none() && has_source(refs, FOURTH_SOURCE_POSITION) { return; }
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

/// `assign` places a texture ref of the source at index `source` of `sources` in the auxiliary atlas.
pub(super) fn create_lookup(
    images: &mut Assets<Image>, sources: &[ItemRecordSource],
    mut assign: impl FnMut(usize, &TileMetaItemTextureRef) -> Option<(u32, UVec2)>,
) -> Handle<Image> {
    let mut records = vec![[0u32; 4]; ITEM_HEADER_COUNT];
    for (index, item_source) in sources.iter().enumerate() {
        let (tilemeta, aux) = (item_source.tilemeta, item_source.aux);
        let mut assign = |r: &TileMetaItemTextureRef| assign(index, r);
        for tile in tilemeta.item_tiles().iter().filter(|tile| (tile.tile_id as usize) < ITEM_CAPACITY) {
            let item = item_source.base + tile.tile_id;
            let Some(material) = tilemeta.item_material(tile.tile_id, 0) else { continue; };
            append_material(&mut records, item, material, tilemeta.item_texture_refs(tile.tile_id), &mut assign);
            let refs = tilemeta.item_texture_refs(tile.tile_id);
            if let (Some(slot), Some(extent)) = (item_source.art_slot(tile.tile_id),
                source(refs, 1).and_then(|r| aux.water_primary_extent(r.texture_id))) {
                water::append(&mut records, item, material, tilemeta.item_texture_refs(tile.tile_id),
                    Vec4::new(f32::from(tile.ec_start_x), f32::from(tile.ec_start_y), slot.logical_width(), slot.logical_height()),
                    UVec2::from_array(extent), tile.flags & uocf::enhanced::tileart::TaeFlag::Rotated.bits() != 0, &mut assign);
            }
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
            flags: if stage == 0 { 2 } else { 0 }, item_index: stage as u8,
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
