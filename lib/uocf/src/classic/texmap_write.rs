//! `texmaps.mul` and `texidx.mul` as typed entries, written back byte for byte ([`IndexedMul`]).
//!
//! A texture is 64 x 64 (8,192 bytes, index extra 0) or 128 x 128 (32,768 bytes, extra 1) `u16` colors, rows
//! top to bottom, 5 bits per channel (red in bits 10-14), bit 15 unused; on every shipped client the extra
//! matches the size.

crate::eyre_imports!();
use std::path::Path;

use crate::classic::indexed_mul::{IndexedEntry, IndexedMul};
use crate::classic::land_texture::LandTextureSize;

pub const TEXMAP_DATA_FILE: &str = "texmaps.mul";
pub const TEXMAP_INDEX_FILE: &str = "texidx.mul";
const COLOR_BYTES: usize = 2;
const SMALL_EXTRA: u32 = 0;
const BIG_EXTRA: u32 = 1;
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MAX: u32 = (1 << CHANNEL_BITS) - 1;
const BYTE_MAX: u32 = u8::MAX as u32;
const RGBA_CHANNELS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TexmapRecord {
    pub size: LandTextureSize,
    /// Rows top to bottom, `0RRRRRGGGGGBBBBB`.
    pub colors: Vec<u16>,
}

fn pixel_count(size: LandTextureSize) -> usize {
    let (width, height) = size.dimensions();
    width as usize * height as usize
}

impl TexmapRecord {
    pub fn decode(extra: u32, payload: &[u8]) -> eyre::Result<Self> {
        let size = [LandTextureSize::Small, LandTextureSize::Big]
            .into_iter()
            .find(|&size| pixel_count(size) * COLOR_BYTES == payload.len())
            .ok_or_else(|| eyre!("{} bytes are no 64x64 or 128x128 texture", payload.len()))?;
        let expected_extra = if size == LandTextureSize::Big { BIG_EXTRA } else { SMALL_EXTRA };
        if extra != expected_extra {
            eyre::bail!("a {:?} texture has extra {expected_extra}, not {extra}", size.dimensions());
        }
        Ok(Self { size, colors: payload.chunks_exact(COLOR_BYTES).map(|c| u16::from_le_bytes([c[0], c[1]])).collect() })
    }

    pub fn encode(&self) -> eyre::Result<(u32, Vec<u8>)> {
        if self.colors.len() != pixel_count(self.size) {
            eyre::bail!("a {:?} texture needs {} colors, not {}", self.size.dimensions(), pixel_count(self.size), self.colors.len());
        }
        let extra = if self.size == LandTextureSize::Big { BIG_EXTRA } else { SMALL_EXTRA };
        Ok((extra, self.colors.iter().flat_map(|c| c.to_le_bytes()).collect()))
    }

    /// From RGBA8 pixels of a 64 x 64 or 128 x 128 image: each channel rounded to 5 bits; alpha is ignored
    /// (land textures are opaque).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> eyre::Result<Self> {
        let size = LandTextureSize::from_dimensions(width, height).ok_or_else(|| eyre!("a texture is 64x64 or 128x128, not {width}x{height}"))?;
        if rgba.len() != pixel_count(size) * RGBA_CHANNELS {
            eyre::bail!("{} RGBA bytes are not {width}x{height}", rgba.len());
        }
        let five = |v: u8| (u32::from(v) * CHANNEL_MAX + BYTE_MAX / 2) / BYTE_MAX;
        let colors = rgba
            .chunks_exact(RGBA_CHANNELS)
            .map(|p| ((five(p[0]) << (2 * CHANNEL_BITS)) | (five(p[1]) << CHANNEL_BITS) | five(p[2])) as u16)
            .collect();
        Ok(Self { size, colors })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TexmapMulFile {
    pub container: IndexedMul,
}

impl TexmapMulFile {
    pub fn from_bytes(index: &[u8], data: &[u8]) -> eyre::Result<Self> {
        let file = Self { container: IndexedMul::from_bytes(index, data)? };
        for id in 0..file.len() {
            file.texture(id).wrap_err_with(|| format!("texture {id}"))?;
        }
        Ok(file)
    }

    pub fn load(client_dir: &Path) -> eyre::Result<Self> {
        let find = |name: &str| {
            crate::utils::path::find_file_ignoring_case(client_dir, name).ok_or_else(|| eyre!("{name} not in {}", client_dir.display()))
        };
        let (index, data) = (find(TEXMAP_INDEX_FILE)?, find(TEXMAP_DATA_FILE)?);
        Self::from_bytes(&std::fs::read(&index)?, &std::fs::read(&data)?).wrap_err_with(|| format!("{}", data.display()))
    }

    pub fn len(&self) -> usize {
        self.container.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.container.entries.is_empty()
    }

    pub fn texture(&self, id: usize) -> eyre::Result<Option<TexmapRecord>> {
        match self.container.entries.get(id) {
            Some(IndexedEntry::Present { extra, payload }) => TexmapRecord::decode(*extra, payload).map(Some),
            _ => Ok(None),
        }
    }

    /// Replaces texture `id`; `None` clears it.
    pub fn set_texture(&mut self, id: usize, texture: Option<&TexmapRecord>) -> eyre::Result<()> {
        match texture {
            Some(texture) => {
                let (extra, payload) = texture.encode()?;
                self.container.set(id, IndexedEntry::Present { extra, payload });
            }
            None => self.container.clear(id),
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
        self.container.to_bytes()
    }

    pub fn save(&self, index_path: &Path, data_path: &Path) -> eyre::Result<()> {
        self.container.save(index_path, data_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_round_trips_and_imports_rgba() {
        let rgba: Vec<u8> = (0..64 * 64).flat_map(|i| [255, (i % 256) as u8, 0, 255]).collect();
        let texture = TexmapRecord::from_rgba(64, 64, &rgba).unwrap();
        assert_eq!(texture.colors[0], 0x7C00);
        let (extra, payload) = texture.encode().unwrap();
        assert_eq!((extra, payload.len()), (SMALL_EXTRA, 8192));
        assert_eq!(TexmapRecord::decode(extra, &payload).unwrap(), texture);
        assert!(TexmapRecord::decode(BIG_EXTRA, &payload).is_err());
        assert!(TexmapRecord::from_rgba(32, 32, &rgba).is_err());
    }
}
