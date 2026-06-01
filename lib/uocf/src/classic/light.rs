#![allow(dead_code)]
//! # UO Light Data Parser (`light.mul` and `lightidx.mul`)
//!
//! This module handles the loading and decoding of light sources from the Ultima Online
//! client files. Light sources are used to create lighting effects in the game world.

crate::eyre_imports!();

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::enhanced::textures::Textures;

#[derive(Clone)]
pub struct LightMap {
    /// Decoded light data. Index is light_id.
    /// Each entry is (width, height, pixel_data_rgba8888).
    decoded_lights: Vec<Option<(u16, u16, Vec<u8>)>>,
    /// EC lighting textures from TerrainTexture.uop
    ec_textures: Option<Textures>,
}

impl LightMap {
    pub fn load(cc_path: Option<impl AsRef<Path>>, ec_path: Option<impl AsRef<Path>>) -> eyre::Result<Self> {
        Self::load_inner(cc_path, ec_path, None)
    }

    pub fn load_with_verdata(
        cc_path: Option<impl AsRef<Path>>,
        ec_path: Option<impl AsRef<Path>>,
        verdata: Arc<Verdata>,
    ) -> eyre::Result<Self> {
        Self::load_inner(cc_path, ec_path, Some(verdata))
    }

    fn load_inner(
        cc_path: Option<impl AsRef<Path>>,
        ec_path: Option<impl AsRef<Path>>,
        verdata: Option<Arc<Verdata>>,
    ) -> eyre::Result<Self> {
        let mut decoded_lights = Vec::new();
        
        if let Some(cc_path) = cc_path {
            let cc_path = cc_path.as_ref();
            let idx_path = cc_path.join("lightidx.mul");
            let mul_path = cc_path.join("light.mul");

            if idx_path.exists() && mul_path.exists() {
                let idx_file = IndexFile::load(idx_path)?;
                let mut mul_file = File::open(&mul_path)?;
                let mut mul_data = Vec::new();
                mul_file.read_to_end(&mut mul_data)?;

                let count = idx_file.element_count();
                decoded_lights.reserve(count);

                for i in 0..count {
                    let entry = idx_file.element(i)?;
                    if let Some(verdata) = &verdata {
                        if let Some(patch) = verdata.entry(VerFileId::Light, i as i32) {
                            let width = (patch.extra as u32 & 0xFFFF) as u16;
                            let height = ((patch.extra as u32 >> 16) & 0xFFFF) as u16;
                            match verdata
                                .read_patch_data(patch)
                                .and_then(|raw| decode_light_from_raw(&raw, width, height))
                            {
                                Ok(decoded) => {
                                    decoded_lights.push(Some((width, height, decoded)));
                                    continue;
                                }
                                Err(e) => {
                                    log::error!("Failed to decode verdata light {}: {}", i, e);
                                    decoded_lights.push(None);
                                    continue;
                                }
                            }
                        }
                    }

                    let index_patch = verdata
                        .as_ref()
                        .and_then(|verdata| verdata.index_patch(VerFileId::LightIdx, i as i32));
                    let index_values = index_patch.or_else(|| {
                        Some((entry.lookup()?, entry.len()?, entry.extra()?))
                    });
                    if let Some((lookup, size, extra)) = index_values {
                        let width = (extra & 0xFFFF) as u16;
                        let height = ((extra >> 16) & 0xFFFF) as u16;

                        if width == 0 || height == 0 || size == 0 {
                            decoded_lights.push(None);
                            continue;
                        }

                        let lookup = lookup as usize;
                        let size = size as usize;
                        let end = lookup + size;

                        if end > mul_data.len() {
                            log::warn!("Light index {} points outside mul data range", i);
                            decoded_lights.push(None);
                            continue;
                        }

                        let raw_data = &mul_data[lookup..end];
                        match decode_light_from_raw(raw_data, width, height) {
                            Ok(decoded) => decoded_lights.push(Some((width, height, decoded))),
                            Err(e) => {
                                log::error!("Failed to decode light {}: {}", i, e);
                                decoded_lights.push(None);
                            }
                        }
                    } else {
                        decoded_lights.push(None);
                    }
                }
            }
        }

        let ec_textures = if let Some(ec_path) = ec_path {
            let ec_path = ec_path.as_ref();
            // Try different possible locations for TerrainTexture.uop
            let candidates = [
                ec_path.join("TerrainTexture.uop"),
                ec_path.join("uop/TerrainTexture.uop"),
            ];
            
            candidates.into_iter()
                .find(|p| p.exists())
                .and_then(|p| Textures::new(&p, None, None).ok())
        } else {
            None
        };

        Ok(Self { decoded_lights, ec_textures })
    }

    /// Fetches the decoded pixels for a given `light_id`.
    /// Returns `(width, height, pixel_data)`.
    pub fn decode_light(&self, light_id: u32) -> eyre::Result<(u16, u16, Vec<u8>)> {
        // Try classic decoded lights first
        if let Some(light) = self.decoded_lights.get(light_id as usize).and_then(|l| l.as_ref()) {
            return Ok((light.0, light.1, light.2.clone()));
        }

        // Try EC lighting from TerrainTexture.uop
        if let Some(ec) = &self.ec_textures {
            // EC lighting path pattern: build/terraintexture/%08u.dds
            let path = format!("build/terraintexture/{:08}.dds", light_id);
            if let Ok(Some(file)) = ec.get_from_name(&path) {
                let img = file.decode_to_rgba()?;
                let rgba = img.into_rgba8();
                return Ok((rgba.width() as u16, rgba.height() as u16, rgba.into_raw()));
            }
        }

        Ok((0, 0, Vec::new()))
    }

    pub fn max_id(&self) -> u32 {
        self.decoded_lights.len() as u32
    }

    pub fn has_id(&self, light_id: u32) -> bool {
        if self.decoded_lights.get(light_id as usize).map_or(false, |l| l.is_some()) {
            return true;
        }

        if let Some(ec) = &self.ec_textures {
            let path = format!("build/terraintexture/{:08}.dds", light_id);
            let hash = crate::uop_container::hash::hash_file_name_single(&path);
            return ec.package().get_file_by_hash(hash).is_some();
        }

        false
    }

    pub fn element_count(&self) -> usize {
        self.decoded_lights.len()
    }
}

/// Decodes raw light intensity data into RGBA8888 pixels.
pub fn decode_light_from_raw(raw_data: &[u8], width: u16, height: u16) -> eyre::Result<Vec<u8>> {
    let pixel_count = width as usize * height as usize;
    if raw_data.len() < pixel_count {
        eyre::bail!("Light data truncated: expected {} bytes, got {}", pixel_count, raw_data.len());
    }

    let mut pixel_data_out = vec![0u8; pixel_count * 4];

    for i in 0..pixel_count {
        let mut val = raw_data[i];
        
        // Follow ClassicUO logic for intensity decoding
        // Light can be from -31 to 31. When they are below 0 they are bit inverted
        if val > 0x1F {
            val = !val & 0x1F;
        }

        if val != 0 {
            // Light intensity in UO is 5-bit (0-31).
            // We scale it to 8-bit (0-255).
            let intensity = (val << 3) | (val >> 2);
            let offset = i * 4;
            pixel_data_out[offset] = intensity;     // R
            pixel_data_out[offset + 1] = intensity; // G
            pixel_data_out[offset + 2] = intensity; // B
            pixel_data_out[offset + 3] = 255;       // A
        }
    }

    Ok(pixel_data_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_light_logic() {
        let raw_data = vec![0, 0x1F, 0x20, 0xFF];
        let decoded = decode_light_from_raw(&raw_data, 2, 2).unwrap();
        
        // Pixel 0: 0 -> transparent
        assert_eq!(decoded[0], 0);
        assert_eq!(decoded[3], 0);
        
        // Pixel 1: 0x1F -> 31 -> 255
        assert_eq!(decoded[4], 255);
        assert_eq!(decoded[7], 255);
        
        // Pixel 2: 0x20 -> 31 -> 255
        assert_eq!(decoded[8], 255);
        assert_eq!(decoded[11], 255);
        
        // Pixel 3: 0xFF -> 0 -> transparent
        assert_eq!(decoded[12], 0);
        assert_eq!(decoded[15], 0);
    }
}
