use std::path::Path;
use crate::models::{UpscalePreviewTarget};
use uocf::classic::art::ArtMap;
use uocf::classic::land_texture::TexMap;
use uocf::enhanced::textures::{Textures};
use color_eyre::eyre;

pub struct RawAssetData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn load_raw_asset(
    cc_dir: Option<&Path>,
    ec_dir: Option<&Path>,
    target: UpscalePreviewTarget,
    id: u32,
) -> eyre::Result<RawAssetData> {
    match target {
        UpscalePreviewTarget::TexArtCc => {
            let dir = cc_dir.ok_or_else(|| eyre::eyre!("CC directory not configured"))?;
            let art_map = ArtMap::load(dir)?;
            let mut scratch = Vec::new();
            if id < 0x4000 {
                let mut rgba = [0u8; 44 * 44 * 4];
                art_map.decode_land_tile(id, &mut scratch, &mut rgba)?;
                Ok(RawAssetData { width: 44, height: 44, rgba: rgba.to_vec() })
            } else {
                let (w, h, rgba) = art_map.decode_static_tile(id, &mut scratch)?;
                Ok(RawAssetData { width: w as u32, height: h as u32, rgba })
            }
        }
        UpscalePreviewTarget::TexLandCc64 | UpscalePreviewTarget::TexLandCc128 => {
            let dir = cc_dir.ok_or_else(|| eyre::eyre!("CC directory not configured"))?;
            let texmap_source = TexMap::load(dir.join("texmaps.mul"), dir.join("texidx.mul"))?;
            let now = std::time::Instant::now();
            let rgba_arc = texmap_source.get_pixel_data(id as usize, now)
                .ok_or_else(|| eyre::eyre!("Texmap ID {} not found", id))?;
            let element = texmap_source
                .element(id as usize)
                .ok_or_else(|| eyre::eyre!("Texmap metadata for ID {} not found", id))?;
            let (w, h) = element.size().dimensions();
            Ok(RawAssetData { width: w as u32, height: h as u32, rgba: rgba_arc.to_vec() })
        }
        UpscalePreviewTarget::TexArtEc => {
            let dir = ec_dir.ok_or_else(|| eyre::eyre!("EC directory not configured"))?;
            let textures = Textures::new(&dir.join("LegacyTexture.uop"), None, None)?;
            let file = textures.get_from_id(id)?
                .ok_or_else(|| eyre::eyre!("EC Art ID {} not found in LegacyTexture.uop", id))?;
            let img = file.decode_to_rgba()?;
            let rgba = img.to_rgba8();
            Ok(RawAssetData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
        }
        UpscalePreviewTarget::TexLandEc64 | UpscalePreviewTarget::TexLandEc128 | UpscalePreviewTarget::TexLandEc256 | UpscalePreviewTarget::TexLandEc512 => {
             let dir = ec_dir.ok_or_else(|| eyre::eyre!("EC directory not configured"))?;
             // Try WorldArt land first
             let textures = Textures::new(&dir.join("Texture.uop"), None, None)?;
             let path = format!("build/worldart/land/{:08}.dds", id);
             let file = textures.get_from_name(&path)?;
             
             let file = if let Some(f) = file {
                 f
             } else {
                 // Try legacy land
                 let legacy_textures = Textures::new(&dir.join("LegacyTexture.uop"), None, None)?;
                 let path = format!("build/legacyland/{:08}.dat", id);
                 legacy_textures.get_from_name(&path)?
                    .ok_or_else(|| eyre::eyre!("EC Land ID {} not found", id))?
             };

             let img = file.decode_to_rgba()?;
             let rgba = img.to_rgba8();
             Ok(RawAssetData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
        }
    }
}
