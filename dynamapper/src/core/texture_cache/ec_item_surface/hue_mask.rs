//! UOSprite's source-2 hue coverage, cropped in native source-pixel coordinates.
use super::{ITEM_CAPACITY, LOOKUP_WIDTH};
use bevy::prelude::*;
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
    fn allocate(&mut self, layers: &mut Vec<Vec<u8>>, width: u32, height: u32) -> (u32, u32, u32) {
        assert!(width > 0 && height > 0 && width <= EDGE && height <= EDGE);
        if self.x + width > EDGE {
            self.x = 0;
            self.y += self.row_height;
            self.row_height = 0;
        }
        if self.layer.is_none() || self.y + height > EDGE {
            self.layer = Some(layers.len() as u32);
            layers.push(vec![0; (EDGE * EDGE * 4) as usize]);
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

fn append_crop(records: &mut Vec<[u32; 4]>, layers: &mut Vec<Vec<u8>>, shelf: &mut Shelf,
    pixels: &[u8], source_width: u32, start: UVec2, extent: UVec2) -> [u32; 3] {
    let offset = records.len() as u32;
    let columns = extent.x.div_ceil(EDGE);
    for row in 0..extent.y.div_ceil(EDGE) {
        for column in 0..columns {
            let at = UVec2::new(column, row) * EDGE;
            let size = (extent - at).min(UVec2::splat(EDGE));
            let (layer, x, y) = shelf.allocate(layers, size.x, size.y);
            for line in 0..size.y {
                let source = (((start.y + at.y + line) * source_width + start.x + at.x) * 4) as usize;
                let destination = (((y + line) * EDGE + x) * 4) as usize;
                let length = (size.x * 4) as usize;
                layers[layer as usize][destination..destination + length].copy_from_slice(&pixels[source..source + length]);
            }
            records.push([layer, x, y, size.x | (size.y << 16)]);
        }
    }
    [offset, columns, extent.x | (extent.y << 16)]
}

fn native_extent(width: u16, height: u16, upscale: u16) -> Option<UVec2> {
    let scale = upscale.max(1);
    if width == 0 || height == 0 || width % scale != 0 || height % scale != 0 { return None; }
    Some(UVec2::new(u32::from(width / scale), u32::from(height / scale)))
}

pub(crate) fn append(lookup: &mut Image, meta: &TileMetaPackage, art: &TexArtEcPackage,
    aux: &TexAuxEcPackage, layers: &mut Vec<Vec<u8>>) {
    let mut records: Vec<[u32; 4]> = bytemuck::cast_slice(lookup.data.as_deref().expect("item lookup pixels")).to_vec();
    let mut pixels: HashMap<u32, Option<Vec<u8>>> = HashMap::new();
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
            let key = (mask.texture_id, start.x, start.y, extent.x, extent.y);
            if let Some(&descriptor) = crops.get(&key) { return Some(descriptor); }
            let rgba = pixels.entry(mask.texture_id).or_insert_with(|| match aux.load_rgba(mask.texture_id) {
                Ok(value) => value,
                Err(error) => {
                    crate::console_logger::one(crate::console_logger::LogSev::Warn,
                        crate::console_logger::LogAbout::RenderWorldArt, &format!("EC hue mask {}: {error}", mask.texture_id));
                    None
                }
            }).as_deref()?;
            if rgba.len() != usize::from(source.width) * usize::from(source.height) * 4 { return None; }
            let descriptor = append_crop(&mut records, layers, &mut shelf, rgba, u32::from(source.width), start, extent);
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

    #[test]
    fn cropped_masks_keep_native_offsets_upscale_and_cross_layer_seams() {
        assert_eq!(native_extent(600, 8, 2), Some(UVec2::new(300, 4)));
        assert_eq!(native_extent(599, 8, 2), None);
        let pixels: Vec<u8> = (0..320 * 8).flat_map(|i| [0, 0, 0, (i % 251) as u8]).collect();
        let mut layers = vec![vec![0; (EDGE * EDGE * 4) as usize]];
        let mut records = vec![[0; 4]];
        let descriptor = append_crop(&mut records, &mut layers, &mut Shelf::default(), &pixels,
            320, UVec2::new(7, 2), UVec2::new(300, 4));
        assert_eq!(descriptor, [1, 2, 300 | (4 << 16)]);
        for x in [0, 1, 255, 256, 299] {
            let record = records[1 + x / 256];
            let destination = (((record[2] + 1) * EDGE + record[1] + (x % 256) as u32) * 4 + 3) as usize;
            assert_eq!(layers[record[0] as usize][destination], pixels[((3 * 320 + 7 + x) * 4) + 3]);
        }
    }
}
