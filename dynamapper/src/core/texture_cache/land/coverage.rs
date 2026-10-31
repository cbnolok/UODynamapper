//! Original EC terrain ownership parameters and the first layer of each chunk.

use bevy::prelude::*;
use std::collections::BTreeMap;
use udd_assets::tex_land_ec::TexLandEcPackage;
use uocf::enhanced::terrain_definition::TerrainLayerClass;
mod cache;
pub(crate) use cache::sys_prepare as sys_prepare_stack_cache;

/// GPU role 6: material ID, signed sorting key, cardinal/diagonal bytes,
/// then class (bits 0..2), positive exponent (3), terrain policy (4..6),
/// and linear coverage filtering (7). Mirrored in land/coverage.wesl.
///
/// Edge softness is the client's: `coverage_exponent` 0 gives a hard edge and point filtering, larger values
/// the soft 255 · 0.95^b / 255 · 0.6^b falloff and bilinear filtering (`UOTerrain::BuildTerrainCoverage`).
/// GPU class codes (bits 0..2 of the flags word; 0 is no EC material). Mirrored in land/coverage.wesl.
const GPU_LAYER_CLASS_DEFAULT: u32 = 1;
const GPU_LAYER_CLASS_WATER: u32 = 2;
const GPU_LAYER_CLASS_BUMP: u32 = 3;
/// `UOWaterTerrainLayer::SetMaterialId` 0x004601e0 stores `material × 1000`, so water layers sort after the others.
const WATER_LAYER_SORT_KEY_SCALE: u32 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcTerrainParams(pub [u32; 4]);

impl EcTerrainParams {
    pub fn for_tile(package: &TexLandEcPackage, payload: u32) -> Option<Self> {
        let id = package.resolve_material_decision(payload).material_id?;
        let source = package.source_material(id)?;
        let class = match source.texture.as_ref()?.layer_class() {
            TerrainLayerClass::Default => GPU_LAYER_CLASS_DEFAULT,
            TerrainLayerClass::Water => GPU_LAYER_CLASS_WATER,
            TerrainLayerClass::Bump => GPU_LAYER_CLASS_BUMP,
        };
        let b = source.coverage_exponent();
        if !b.is_finite() { return None; }
        let weights = coverage_weights(b);
        let key = if class == GPU_LAYER_CLASS_WATER { id.checked_mul(WATER_LAYER_SORT_KEY_SCALE)? } else { id };
        let flags = class | (u32::from(b > 0.0) << 3)
            | (u32::from(ec_land_shader_flags(package, payload)) << 4)
            | (u32::from(b != 0.0) << 7);
        Some(Self([id, key, u32::from(weights[0]) | (u32::from(weights[1]) << 8), flags]))
    }
}

fn coverage_weights(b: f32) -> [u8; 2] {
    if b <= 0.0 { return [0, 0]; }
    [0.95f32, 0.60f32].map(|base| (255.0 * f64::from(base).powf(f64::from(b))).trunc() as u8)
}

pub fn ec_land_shader_flags(package: &TexLandEcPackage, tile_id: u32) -> u16 {
    use crate::core::render::scene::world::land::tile_atlas::TERRAIN_SHADER_FLAG_REVIEWED_LIQUID;
    let Some(material_id) = package.resolve_material_decision(tile_id).material_id else {
        return 0;
    };
    let mut flags = 0u16;
    if package.source_material(material_id).and_then(|entry| entry.texture.as_ref())
        .is_some_and(|texture| texture.layer_class() == TerrainLayerClass::Water) {
        flags |= TERRAIN_SHADER_FLAG_REVIEWED_LIQUID;
    }
    let Some(details) = package.terrain_override_details_for(material_id) else {
        return flags;
    };
    if details.liquid.is_some() { flags |= TERRAIN_SHADER_FLAG_REVIEWED_LIQUID; }
    flags
}

#[derive(Resource)]
pub struct EcTerrainBases {
    pub image: Handle<Image>,
    pub params: Vec<Option<EcTerrainParams>>,
    cache: cache::StackCache,
    dimensions: UVec2,
    map: Option<u32>,
    values: Vec<[u16; 4]>,
    dirty: bool,
}

impl EcTerrainBases {
    pub fn new(images: &mut Assets<Image>, dimensions: UVec2, package: Option<&TexLandEcPackage>) -> Self {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let dimensions = dimensions.max(UVec2::ONE);
        let values = vec![[u16::MAX; 4]; (dimensions.x * dimensions.y) as usize];
        let image = images.add(Image::new(
            Extent3d { width: dimensions.x, height: dimensions.y, depth_or_array_layers: 1 },
            TextureDimension::D2, bytemuck::cast_slice(&values).to_vec(), TextureFormat::Rgba16Uint,
            bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
        ));
        let params = (0..16_384).map(|id| package.and_then(|p| EcTerrainParams::for_tile(p, id))).collect();
        let cache = cache::StackCache::new(images, package);
        Self { image, params, cache, dimensions, map: None, values, dirty: false }
    }

    pub fn stack_image(&self) -> Handle<Image> { self.cache.image.clone() }

    pub fn cached_chunk_count(&self) -> usize {
        self.values.iter().filter(|value| value[2] != u16::MAX).count()
    }

    pub fn begin_map(&mut self, map: u32) {
        if self.map != Some(map) {
            self.map = Some(map);
            self.values.fill([u16::MAX; 4]);
            self.cache.clear();
            self.dirty = true;
        }
    }

    pub fn set(&mut self, chunk: UVec2, limits: [u16; 2]) {
        if chunk.x >= self.dimensions.x || chunk.y >= self.dimensions.y { return; }
        let value = &mut self.values[(chunk.y * self.dimensions.x + chunk.x) as usize];
        if [value[0], value[1]] != limits {
            *value = [limits[0], limits[1], u16::MAX, u16::MAX];
            self.dirty = true;
        }
    }

    pub fn flush(&mut self, images: &mut Assets<Image>) {
        if self.dirty {
            images.get_mut(&self.image).expect("EC terrain base image").data =
                Some(bytemuck::cast_slice(&self.values).to_vec());
            self.dirty = false;
        }
    }
}

/// All chunk materials participate in the original sorting, including a base
/// whose ownership is outside this fragment's immediate neighborhood.
pub fn layer_limits(payloads: impl IntoIterator<Item = u16>, params: &[Option<EcTerrainParams>]) -> Option<[u16; 2]> {
    let mut layers = BTreeMap::new();
    for payload in payloads {
        if let Some(Some(p)) = params.get(usize::from(payload)) {
            layers.entry((p.0[1] as i32, p.0[0]))
                .and_modify(|alias: &mut u16| *alias = (*alias).min(payload))
                .or_insert(payload);
        }
    }
    // Original shader 0x596c80 caps the sorted pass vector at 49 entries.
    let mut passes = layers.into_values().take(49);
    let first = passes.next()?;
    Some([first, passes.last().unwrap_or(first)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_weights_truncate_the_original_float_constants() {
        assert_eq!(coverage_weights(0.0), [0, 0]);
        assert_eq!(coverage_weights(1.0), [242, 153]);
        assert_eq!(coverage_weights(5.0), [197, 19]);
        assert_eq!(coverage_weights(0.15), [253, 236]);
    }

    #[test]
    fn chunk_base_uses_original_sort_key_and_deterministic_alias() {
        let mut params = vec![None; 65536];
        params[6000] = Some(EcTerrainParams([5, 5000, 0, 2]));
        params[200] = Some(EcTerrainParams([70, 70, 0, 1]));
        params[100] = params[200];
        assert_eq!(layer_limits([6000, 200, 100, 65535], &params), Some([100, 6000]));
        assert_eq!(layer_limits([65535], &params), None);
    }

    #[test]
    fn chunk_pass_limit_counts_materials_instead_of_aliases() {
        let mut params = vec![None; 16384];
        for id in 1..=60 {
            params[1000 + id] = Some(EcTerrainParams([id as u32, id as u32, 0, 1]));
            params[2000 + id] = params[1000 + id];
        }
        assert_eq!(layer_limits((1001..=1060).chain(2001..=2060), &params), Some([1001, 1049]));
    }

    #[test]
    fn map_switch_clears_cached_chunk_bases() {
        let mut images = Assets::default();
        let mut bases = EcTerrainBases::new(&mut images, UVec2::splat(2), None);
        bases.begin_map(0);
        bases.set(UVec2::ZERO, [5, 5]);
        bases.flush(&mut images);
        assert_eq!(&images.get(&bases.image).unwrap().data.as_ref().unwrap()[..2], &5u16.to_le_bytes());
        bases.begin_map(1);
        bases.flush(&mut images);
        assert!(bases.values.iter().all(|&value| value == [u16::MAX; 4]));
    }
}
