//! Original item-only fourth sources. Terrain's blend input is a separate graph.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use udd_assets::tilemeta::{EcMaterialPhysicalPackage, TileMetaItemTextureRef, TileMetaPackage};
use uocf::enhanced::tileart::ArtMaterial;

const LOOKUP_WIDTH: u32 = 256;
const ITEM_CAPACITY: u32 = 65_536;

#[derive(Resource, Clone)]
pub struct EcItemLiquidImages {
    /// Shares the terrain auxiliary atlas, including its transparent layer zero.
    pub atlas: Handle<Image>,
    pub lookup: Handle<Image>,
}

fn fourth_source<'a>(material: &ArtMaterial, refs: &'a [TileMetaItemTextureRef])
    -> Option<&'a TileMetaItemTextureRef> {
    if material.block_index != 0 || material.shader_name.as_deref() != Some("UOWaterShader") {
        return None;
    }
    refs.iter().find(|r| r.block_index == material.block_index && r.sampler_index == 3
        && r.is_world_art() && r.physical_package() == EcMaterialPhysicalPackage::Texture
        && r.uv_set <= 1 && r.texture_stretch.is_finite() && r.texture_stretch > 0.0)
}

pub(super) fn create_lookup(
    images: &mut Assets<Image>,
    tilemeta: Option<&TileMetaPackage>,
    mut assign: impl FnMut(&TileMetaItemTextureRef) -> Option<(u32, UVec2)>,
) -> Handle<Image> {
    // Two Rgba32Uint records per graphic, independent of the 14-bit land IDs.
    // First: atlas layer, width, height, Technique[1].
    // Second: repetition and two scroll velocities as f32 bits, original UV set.
    let mut records = vec![[0u32; 4]; (ITEM_CAPACITY * 2) as usize];
    if let Some(tilemeta) = tilemeta {
        for tile in tilemeta.item_tiles().iter().filter(|tile| tile.tile_id < ITEM_CAPACITY) {
            let Some(material) = tilemeta.item_material(tile.tile_id, 0) else { continue; };
            let Some(source) = fourth_source(material, tilemeta.item_texture_refs(tile.tile_id)) else { continue; };
            let Some((layer, extent)) = assign(source) else { continue; };
            let index = tile.tile_id as usize * 2;
            records[index] = [layer, extent.x, extent.y, material.technique.get(1).copied().unwrap_or(0) as u32];
            records[index + 1] = [source.texture_stretch.to_bits(),
                material.tweaks.get(2).copied().unwrap_or(0.0).to_bits(),
                material.tweaks.get(3).copied().unwrap_or(0.0).to_bits(), source.uv_set];
        }
    }
    images.add(Image::new(
        Extent3d { width: LOOKUP_WIDTH, height: ITEM_CAPACITY * 2 / LOOKUP_WIDTH, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytemuck::cast_slice(&records).to_vec(),
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourth_source_uses_original_shader_sampler_and_physical_family() {
        let material = ArtMaterial { shader_name: Some("UOWaterShader".into()), ..Default::default() };
        let source = TileMetaItemTextureRef {
            texture_id: 6038, texture_type: 1, physical_package: EcMaterialPhysicalPackage::Texture as u8,
            sampler_index: 3, uv_set: 0, texture_stretch: 1.0, ..Default::default()
        };
        let refs = [TileMetaItemTextureRef { block_index: 1, ..source }, source];
        assert_eq!(fourth_source(&material, &refs).unwrap().block_index, 0);
        for wrong in [
            TileMetaItemTextureRef { physical_package: EcMaterialPhysicalPackage::LegacyTexture as u8, ..source },
            TileMetaItemTextureRef { sampler_index: 2, ..source },
            TileMetaItemTextureRef { uv_set: 2, ..source },
            TileMetaItemTextureRef { texture_stretch: f32::NAN, ..source },
        ] {
            assert!(fourth_source(&material, &[wrong]).is_none());
        }
        let terrain = ArtMaterial { shader_name: Some("UOWaterTerrainLayer".into()), ..material };
        assert!(fourth_source(&terrain, &[source]).is_none());
    }

    #[test]
    fn item_lookup_keeps_masks_and_parameters_distinct_above_land_capacity() {
        use bytemuck::Zeroable;
        use udd_assets::tilemeta::*;
        use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder, UddpReader};

        let mut items = vec![TileMetaItemTile::zeroed(); 17002];
        for (id, item) in items.iter_mut().enumerate() { item.tile_id = id as u32; }
        let mut spans = vec![TileMetaItemTextureRefSpan::zeroed(); items.len()];
        spans[17000] = TileMetaItemTextureRefSpan { start: 0, len: 1, ..Default::default() };
        spans[17001] = TileMetaItemTextureRefSpan { start: 1, len: 1, ..Default::default() };
        let refs = [6038, 2100014].map(|texture_id| TileMetaItemTextureRef {
            texture_id, texture_type: 1, physical_package: EcMaterialPhysicalPackage::Texture as u8,
            sampler_index: 3, texture_stretch: 1.0, ..Default::default()
        });
        let source = br#"{"17000":[{"block_index":0,"shader_name":"UOWaterShader","technique":[0,1],"tweaks":[0,0,-0.02,0.001],"unk1":0}],"17001":[{"block_index":0,"shader_name":"UOWaterShader","technique":[0,2],"tweaks":[0,0,0,0],"unk1":0}]}"#;
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data) in [
            (TILEMETA_LAND_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_ENTRY_PATH, bytemuck::cast_slice(&items)),
            (TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, bytemuck::cast_slice(&spans)),
            (TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, bytemuck::cast_slice(&refs)),
            (TILEMETA_ITEM_MATERIAL_ENTRY_PATH, source.as_slice()),
        ] {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDict,
                width: 0, height: 0, virtual_path: Some(path), path_hash64: None, id: None, data,
            }).unwrap();
        }
        let tilemeta = TileMetaPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
        let mut images = Assets::<Image>::default();
        let lookup = create_lookup(&mut images, Some(&tilemeta), |r| Some((r.texture_id, UVec2::splat(64))));
        let records: &[[u32; 4]] = bytemuck::cast_slice(images.get(&lookup).unwrap().data.as_ref().unwrap());
        assert_eq!(records[17000 * 2], [6038, 64, 64, 1]);
        assert_eq!(records[17001 * 2], [2100014, 64, 64, 2]);
        assert_eq!(records[17000 * 2 + 1], [1.0f32.to_bits(), (-0.02f32).to_bits(), 0.001f32.to_bits(), 0]);
        assert_eq!(records[0], [0; 4]);
    }
}
