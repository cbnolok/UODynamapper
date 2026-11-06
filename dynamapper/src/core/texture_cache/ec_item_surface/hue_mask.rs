//! UOSprite's source-2 hue coverage, cropped in native source-pixel coordinates.
use super::{ITEM_CAPACITY, LOOKUP_WIDTH};
use crate::core::texture_cache::land::aux_layers::{AuxImage, AuxLayers};
use bevy::prelude::*;
use color_eyre::eyre;
use std::collections::HashMap;
use udd_assets::{tex_art_ec::TexArtEcPackage, tex_aux_ec::TexAuxEcPackage, tilemeta::TileMetaPackage};

const EDGE: u32 = 256;
const SPRITE: u32 = 256;
const MASK: u32 = 512;
#[cfg(test)]
#[path = "hue_mask/shader_tests.rs"]
mod shader_tests;

#[derive(Default)]
struct Shelf {
    layer: Option<u32>,
    x: u32,
    y: u32,
    row_height: u32,
}

impl Shelf {
    fn allocate(&mut self, layers: &mut AuxLayers, width: u32, height: u32) -> (u32, u32, u32) {
        assert!(width > 0 && height > 0 && width <= EDGE && height <= EDGE);
        if self.x + width > EDGE {
            self.x = 0;
            self.y += self.row_height;
            self.row_height = 0;
        }
        if self.layer.is_none() || self.y + height > EDGE {
            self.layer = Some(layers.push_blank());
            self.x = 0;
            self.y = 0;
            self.row_height = 0;
        }
        let result = (self.layer.expect("allocated hue layer"), self.x, self.y);
        self.x += width;
        self.row_height = self.row_height.max(height);
        result
    }
}

/// Packs a crop as shelf tiles of at most `EDGE` texels. A BC7 array copies whole blocks: a tile's shelf rect covers
/// the blocks around it and its record points at the crop inside them, so its texels stay the source's. A tile whose
/// offset into its blocks would push it past the layer edge is encoded again, starting on the grid.
fn append_crop(records: &mut Vec<[u32; 4]>, layers: &mut AuxLayers, shelf: &mut Shelf,
    image: &AuxImage, start: UVec2, extent: UVec2) -> eyre::Result<[u32; 3]> {
    let offset = records.len() as u32;
    let columns = extent.x.div_ceil(EDGE);
    let grid = layers.format().grid();
    for row in 0..extent.y.div_ceil(EDGE) {
        for column in 0..columns {
            let at = UVec2::new(column, row) * EDGE;
            let size = (extent - at).min(UVec2::splat(EDGE));
            let skew = (start + at) % grid;
            let realigned = if (skew + size).max_element() > EDGE { Some(image.crop(start + at, size)?) } else { None };
            let (source, origin, skew) = match &realigned {
                Some(realigned) => (realigned, UVec2::ZERO, UVec2::ZERO),
                None => (image, start + at - skew, skew),
            };
            // A whole number of grid cells: the texels, or the blocks around them.
            let span = (skew + size + UVec2::splat(grid - 1)) / grid * grid;
            let (layer, x, y) = shelf.allocate(layers, span.x, span.y);
            layers.copy(layer, UVec2::new(x, y), source, origin, if realigned.is_some() { size } else { span });
            records.push([layer, x + skew.x, y + skew.y, size.x | (size.y << 16)]);
        }
    }
    Ok([offset, columns, extent.x | (extent.y << 16)])
}

fn native_extent(width: u16, height: u16, upscale: u16) -> Option<UVec2> {
    let scale = upscale.max(1);
    if width == 0 || height == 0 || width % scale != 0 || height % scale != 0 { return None; }
    Some(UVec2::new(u32::from(width / scale), u32::from(height / scale)))
}

pub(crate) fn append(lookup: &mut Image, meta: &TileMetaPackage, art: &TexArtEcPackage,
    aux: &TexAuxEcPackage, layers: &mut AuxLayers) {
    let mut records: Vec<[u32; 4]> = bytemuck::cast_slice(lookup.data.as_deref().expect("item lookup pixels")).to_vec();
    let format = layers.format();
    let mut pixels: HashMap<u32, Option<AuxImage>> = HashMap::new();
    let mut crops = HashMap::new();
    let mut shelf = Shelf::default();
    let mut missing = 0;
    let mut registered = 0;
    for tile in meta.item_tiles() {
        let id = tile.tile_id as usize;
        if id >= ITEM_CAPACITY { continue; }
        let Some(material) = meta.item_material(tile.tile_id, 0) else { continue; };
        if material.shader_name.as_deref() != Some("UOSpriteShader") { continue; }
        let Some(slot) = art.present_slot(tile.tile_id).filter(|s| s.is_static()) else { continue; };
        let refs = meta.item_texture_refs(tile.tile_id);
        if !super::sprite_source(refs, 0).is_some_and(|r| r.is_primary_selected()
            && r.texture_id == tile.ec_texture_id) { continue; }
        let declared = super::has_source(refs, 2);
        records[id] = [0, SPRITE | if declared { MASK } else { 0 }, 0, 0];
        registered += 1;
        if !declared { continue; }
        let descriptor = (|| {
            let mask = super::sprite_source(refs, 2)?;
            let source = aux.slot_for_texture_id(mask.texture_id)?;
            if source.logical_family != mask.texture_type { return None; }
            let start = UVec2::new(u32::try_from(tile.ec_start_x).ok()?, u32::try_from(tile.ec_start_y).ok()?);
            let extent = native_extent(slot.width, slot.height, slot.upscale_factor)?;
            if (start + extent).x > u32::from(source.width) || (start + extent).y > u32::from(source.height) { return None; }
            let key = (source.payload_texture_id, start.x, start.y, extent.x, extent.y);
            if let Some(&descriptor) = crops.get(&key) { return Some(descriptor); }
            let warn = |error: eyre::Report| crate::console_logger::one(crate::console_logger::LogSev::Warn,
                crate::console_logger::LogAbout::RenderWorldArt, &format!("EC hue mask {}: {error:#}", mask.texture_id));
            let source_extent = UVec2::new(source.width.into(), source.height.into());
            let image = pixels.entry(source.payload_texture_id)
                .or_insert_with(|| AuxImage::load(aux, source.payload_texture_id, source_extent, format)
                    .unwrap_or_else(|error| { warn(error); None }))
                .as_ref()?;
            let descriptor = append_crop(&mut records, layers, &mut shelf, image, start, extent)
                .map_err(warn).ok()?;
            crops.insert(key, descriptor);
            Some(descriptor)
        })();
        if let Some([offset, columns, extent]) = descriptor {
            records[id] = [offset, SPRITE | MASK, columns, extent];
        } else {
            // A declared but unavailable mask must not turn into a full-art hue.
            missing += 1;
        }
    }
    let height = records.len().div_ceil(LOOKUP_WIDTH);
    records.resize(height * LOOKUP_WIDTH, [0; 4]);
    lookup.texture_descriptor.size.height = height as u32;
    lookup.data = Some(bytemuck::cast_slice(&records).to_vec());
    crate::console_logger::one(crate::console_logger::LogSev::Info,
        crate::console_logger::LogAbout::RenderWorldArt,
        &format!("EC sprite hue coverage: {registered} sprites, {} distinct crops, {missing} unavailable masks", crops.len()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::texture_cache::land::aux_layers::AuxLayerFormat;

    #[test]
    fn cropped_masks_keep_native_offsets_upscale_and_cross_layer_seams() {
        assert_eq!(native_extent(600, 8, 2), Some(UVec2::new(300, 4)));
        assert_eq!(native_extent(599, 8, 2), None);
        let pixels: Vec<u8> = (0..320 * 8).flat_map(|i| [0, 0, 0, (i % 251) as u8]).collect();
        let mut layers = AuxLayers::new(AuxLayerFormat::Rgba8);
        let mut records = vec![[0; 4]];
        let image = AuxImage::Rgba { extent: UVec2::new(320, 8), pixels: pixels.clone() };
        let descriptor = append_crop(&mut records, &mut layers, &mut Shelf::default(), &image,
            UVec2::new(7, 2), UVec2::new(300, 4)).unwrap();
        assert_eq!(descriptor, [1, 2, 300 | (4 << 16)]);
        for x in [0, 1, 255, 256, 299] {
            let record = records[1 + x / 256];
            let destination = (((record[2] + 1) * EDGE + record[1] + (x % 256) as u32) * 4 + 3) as usize;
            assert_eq!(layers.layer(record[0])[destination], pixels[((3 * 320 + 7 + x) * 4) + 3]);
        }
    }

    /// A BC7 crop copies the source's blocks and points into them past its offset, so every texel decodes as the
    /// source's; a full-width tile off the block grid is encoded again on it, and decodes as that encoding.
    #[test]
    fn bc7_crops_keep_the_source_blocks() {
        use udd_assets::bc7::{self, ImageExtent, RawImageFormat};
        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 12;
        const START: UVec2 = UVec2::new(7, 2);
        const EXTENT: UVec2 = UVec2::new(300, 8);
        const RGBA: usize = 4;
        let rgba: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| {
            let (x, y) = (i % WIDTH, i / WIDTH);
            [(x * 7 % 256) as u8, (y * 19) as u8, (x * y % 256) as u8, (x % 256) as u8]
        }).collect();
        let blocks = bc7::encode_for_vram(&rgba, ImageExtent::new(WIDTH, HEIGHT).unwrap(), RawImageFormat::Rgba8888,
            bc7::VramTextureEncoding::Bc7(bc7::preferred_bc7_encoder_backend())).unwrap();
        let source = bc7::decode_from_vram(&blocks, RawImageFormat::Rgba8888).unwrap();
        let image = AuxImage::Bc7(blocks);
        let mut layers = AuxLayers::new(AuxLayerFormat::Bc7);
        let mut records = vec![[0; 4]];
        let descriptor = append_crop(&mut records, &mut layers, &mut Shelf::default(), &image, START, EXTENT).unwrap();
        assert_eq!(descriptor, [1, 2, EXTENT.x | (EXTENT.y << 16)]);
        let layer_extent = ImageExtent::new(EDGE, EDGE).unwrap();
        for (column, record) in records[1..].iter().enumerate() {
            let at = UVec2::new(column as u32 * EDGE, 0);
            let size = (EXTENT - at).min(UVec2::splat(EDGE));
            // The first tile is a whole layer wide but starts 3 texels into its blocks; the second keeps them.
            let realigned = (column == 0).then(|| {
                let AuxImage::Bc7(crop) = image.crop(START + at, size).unwrap() else { unreachable!() };
                bc7::decode_from_vram(&crop, RawImageFormat::Rgba8888).unwrap()
            });
            assert_eq!(realigned.is_some(), (START + at).x % 4 + size.x > EDGE);
            let decoded = bc7::decode_bc7_to_rgba8888(layers.layer(record[0]), layer_extent).unwrap();
            for y in 0..size.y {
                for x in 0..size.x {
                    let texel = ((record[2] + y) * EDGE + record[1] + x) as usize * RGBA;
                    let expected = match &realigned {
                        Some(crop) => &crop[((y * size.x + x) as usize) * RGBA..][..RGBA],
                        None => &source[(((START.y + y) * WIDTH + START.x + at.x + x) as usize) * RGBA..][..RGBA],
                    };
                    assert_eq!(&decoded[texel..texel + RGBA], expected, "tile {column} texel {x},{y}");
                }
            }
        }
        assert_eq!([records[2][1] % 4, records[2][2] % 4], [START.x % 4, START.y % 4], "the record points past the offset");
    }
}
