//! Item-owned water graphs that need no TerrainDefinition material proxy.
use super::{has_source, source, ITEM_CAPACITY};
use bevy::prelude::*;
use udd_assets::tilemeta::TileMetaItemTextureRef;
use uocf::enhanced::tileart::ArtMaterial;

pub(super) const WATER: u32 = 16;

pub(super) fn append(
    records: &mut Vec<[u32; 4]>, item: u32, material: &ArtMaterial,
    refs: &[TileMetaItemTextureRef], window: Vec4, extent: UVec2,
    assign: &mut impl FnMut(&TileMetaItemTextureRef) -> Option<(u32, UVec2)>,
) {
    if item as usize >= ITEM_CAPACITY || material.block_index != 0
        || material.shader_name.as_deref() != Some("UOWaterShader") { return; }
    let (Some(bump), Some(fallback), Some(reflection)) = (source(refs, 0), source(refs, 1), source(refs, 2)) else { return; };
    // The item's art window is derived from map 1. Keep it even though normal
    // capable rendering samples maps 0/2 instead of that fallback image.
    if !fallback.is_primary_selected() { return; }
    let technique = material.technique.first().copied().unwrap_or(0) as u32;
    if technique > 2 { return; } // Other texgen modes need their own source trace.
    let fourth = source(refs, 3);
    if fourth.is_none() && has_source(refs, 3) { return; }
    if extent.min_element() == 0 || !window.is_finite() || window.xy().min_element() < 0.0
        || window.zw().min_element() <= 0.0 || (window.xy() + window.zw()).cmpgt(extent.as_vec2()).any() { return; }
    let uv_window = Vec4::new(window.x / extent.x as f32, window.y / extent.y as f32,
        window.z / extent.x as f32, window.w / extent.y as f32);
    let mut descriptors = [[0u32; 4]; 7];
    // Match the native primary-image crop, as the hue-mask path already does.
    // Trimming changes a sampling window, not the full support-map pixels.
    descriptors[6] = uv_window.to_array().map(f32::to_bits);
    for (index, reference) in [Some(bump), Some(reflection), fourth].into_iter().enumerate() {
        let Some(reference) = reference else { continue; };
        let Some((layer, extent)) = assign(reference).filter(|(layer, size)| *layer != 0 && size.min_element() != 0) else { return; };
        let velocity = match index {
            0 => [material.tweaks.first().copied().unwrap_or(0.0), material.tweaks.get(1).copied().unwrap_or(0.0)],
            2 => [material.tweaks.get(2).copied().unwrap_or(0.0), material.tweaks.get(3).copied().unwrap_or(0.0)],
            _ => [0.0; 2],
        };
        if velocity.iter().any(|value| !value.is_finite()) { return; }
        descriptors[index * 2] = [layer, extent.x, extent.y, reference.uv_set];
        descriptors[index * 2 + 1] = [reference.texture_stretch.to_bits(), velocity[0].to_bits(), velocity[1].to_bits(),
            if index == 1 { technique } else if index == 2 { material.technique.get(1).copied().unwrap_or(0) as u32 } else { 0 }];
    }
    records[item as usize] = [records.len() as u32, WATER, fallback.uv_set, fallback.texture_stretch.to_bits()];
    records.extend(descriptors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::tilemeta::EcMaterialPhysicalPackage;

    fn graph() -> (ArtMaterial, [TileMetaItemTextureRef; 4]) {
        (ArtMaterial { shader_name: Some("UOWaterShader".into()), technique: vec![0, 1],
            tweaks: vec![-0.01, 0.02, -0.03, 0.04], ..Default::default() },
        std::array::from_fn(|stage| TileMetaItemTextureRef {
            texture_id: [1000020, 4651, 4000001, 4651][stage], texture_type: 1,
            physical_package: EcMaterialPhysicalPackage::Texture as u8,
            flags: if stage == 1 { 2 } else { 0 }, item_index: stage as u8,
            texture_stretch: if stage == 0 { 8.0 } else { 1.0 }, uv_set: u32::from(stage < 3), ..Default::default()
        }))
    }

    #[test]
    fn item_water_graph_keeps_map_positions_parameters_and_full_item_identity() {
        let (material, refs) = graph();
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        let mut assigned = Vec::new();
        append(&mut records, 17000, &material, &refs, Vec4::new(0.0, 0.0, 64.0, 32.0), UVec2::new(64, 32), &mut |r| {
            assigned.push(r.texture_id); Some((assigned.len() as u32, UVec2::new(64, 32)))
        });
        assert_eq!(assigned, [1000020, 4000001, 4651]);
        assert_eq!(records[17000], [ITEM_CAPACITY as u32, WATER, 1, 1.0f32.to_bits()]);
        assert_eq!(records[ITEM_CAPACITY], [1, 64, 32, 1]);
        assert_eq!(records[ITEM_CAPACITY + 1], [8.0f32.to_bits(), (-0.01f32).to_bits(), 0.02f32.to_bits(), 0]);
        assert_eq!(records[ITEM_CAPACITY + 4], [3, 64, 32, 0]);
        assert_eq!(records[ITEM_CAPACITY + 5], [1.0f32.to_bits(), (-0.03f32).to_bits(), 0.04f32.to_bits(), 1]);
        assert_eq!(records[16999], [0; 4]);
    }

    #[test]
    fn item_water_graph_maps_trimmed_art_to_full_source_uvs_and_rejects_bad_windows() {
        let (material, refs) = graph();
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        append(&mut records, 4651, &material, &refs, Vec4::new(16.0, 8.0, 32.0, 16.0), UVec2::new(64, 32),
            &mut |_| Some((1, UVec2::new(64, 32))));
        assert_eq!(records[ITEM_CAPACITY + 6], [0.25f32.to_bits(), 0.25f32.to_bits(), 0.5f32.to_bits(), 0.5f32.to_bits()]);
        for window in [Vec4::new(33.0, 0.0, 32.0, 32.0), Vec4::new(-1.0, 0.0, 32.0, 32.0), Vec4::ZERO] {
            let mut records = vec![[0; 4]; ITEM_CAPACITY];
            append(&mut records, 4651, &material, &refs, window, UVec2::new(64, 32), &mut |_| Some((1, UVec2::new(64, 32))));
            assert_eq!(records[4651], [0; 4]);
        }
    }

    #[test]
    fn item_water_graph_rejects_incomplete_wrong_family_and_wrong_primary_inputs() {
        let (material, refs) = graph();
        for stage in 0..4 {
            let mut invalid = refs;
            invalid[stage].uv_set = 2;
            let mut records = vec![[0; 4]; ITEM_CAPACITY];
            append(&mut records, 4651, &material, &invalid, Vec4::new(0.0, 0.0, 64.0, 64.0), UVec2::splat(64), &mut |_| Some((1, UVec2::splat(64))));
            assert_eq!(records.len(), ITEM_CAPACITY);
        }
        let mut invalid = refs;
        invalid[1].flags = 0;
        let mut records = vec![[0; 4]; ITEM_CAPACITY];
        append(&mut records, 4651, &material, &invalid, Vec4::new(0.0, 0.0, 64.0, 64.0), UVec2::splat(64), &mut |_| Some((1, UVec2::splat(64))));
        assert_eq!(records[4651], [0; 4]);
        append(&mut records, 4651, &material, &refs, Vec4::new(0.0, 0.0, 64.0, 64.0), UVec2::splat(64), &mut |_| None);
        assert_eq!(records[4651], [0; 4]);
        invalid = refs; invalid[0].physical_package = EcMaterialPhysicalPackage::LegacyTexture as u8;
        append(&mut records, 4651, &material, &invalid, Vec4::new(0.0, 0.0, 64.0, 64.0), UVec2::splat(64), &mut |_| Some((1, UVec2::splat(64))));
        assert_eq!(records[4651], [0; 4]);
    }

    #[test]
    fn item_water_graph_accepts_absent_optional_fourth_and_preserves_explicit_uv_modes() {
        let (mut material, refs) = graph();
        for technique in 0..=2 {
            material.technique[0] = technique;
            let mut records = vec![[0; 4]; ITEM_CAPACITY];
            append(&mut records, 4651, &material, &refs[..3], Vec4::new(0.0, 0.0, 64.0, 64.0), UVec2::splat(64), &mut |_| Some((1, UVec2::splat(64))));
            assert_eq!(records[ITEM_CAPACITY + 3][3], technique as u32);
            assert_eq!(records[ITEM_CAPACITY + 4], [0; 4]);
        }
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires current packages; set DYNAMAPPER_TEST_UDDP_DIR"]
fn installed_item_water_graphs_without_terrain_proxy_keep_all_support_sources() {
    use udd_assets::{tex_art_ec::TexArtEcPackage, tex_aux_ec::TexAuxEcPackage,
        tex_land_ec::TexLandEcPackage, tilemeta::TileMetaPackage};
    let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_UDDP_DIR").expect("set DYNAMAPPER_TEST_UDDP_DIR"));
    let meta = TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
    let art = TexArtEcPackage::load(root.join("tex_art_ec.uddp")).unwrap();
    let land = TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
    let aux = TexAuxEcPackage::load(root.join("tex_aux_ec.uddp")).unwrap();
    let mut records = vec![[0; 4]; ITEM_CAPACITY];
    let mut admitted = 0;
    for tile in meta.item_tiles() {
        let Some(material) = meta.item_material(tile.tile_id, 0).filter(|m| m.shader_name.as_deref() == Some("UOWaterShader")) else { continue; };
        let refs = meta.item_texture_refs(tile.tile_id);
        if !art.present_slot(tile.tile_id).is_some_and(|slot| slot.is_land())
            || land.source_liquid_payload_for_item(material, refs).is_some() { continue; }
        let slot = art.present_slot(tile.tile_id).unwrap();
        append(&mut records, tile.tile_id, material, refs,
            Vec4::new(f32::from(tile.ec_start_x), f32::from(tile.ec_start_y), slot.logical_width(), slot.logical_height()),
            UVec2::from_array(aux.water_primary_extent(source(refs, 1).unwrap().texture_id).expect("native primary extent")), &mut |source| {
            let slot = aux.slot_for_texture_id(source.texture_id)?;
            (slot.logical_family == source.texture_type).then_some((1, UVec2::new(slot.width.into(), slot.height.into())))
        });
        assert_eq!(records[tile.tile_id as usize][1], WATER, "item {} lost an authored water stage", tile.tile_id);
        admitted += 1;
    }
    assert!(admitted > 0, "package set did not exercise unmatched item water");
}
