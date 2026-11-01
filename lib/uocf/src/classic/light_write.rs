//! `light.mul` and `lightidx.mul` as typed entries, written back byte for byte ([`IndexedMul`]).
//!
//! The index record's extra holds the width (low 16 bits) and the height (high 16 bits); the payload holds one
//! intensity byte per pixel, rows top to bottom, as [`decode_light_from_raw`](crate::classic::light::decode_light_from_raw)
//! reads it. Light 50 of every shipped client has 90,000 bytes for its 280 x 280 pixels; the 11,600 bytes after
//! the pixels are kept as [`LightRecord::trailing`].

crate::eyre_imports!();
use std::path::Path;

use crate::classic::indexed_mul::{IndexedEntry, IndexedMul};

pub const LIGHT_DATA_FILE: &str = "light.mul";
pub const LIGHT_INDEX_FILE: &str = "lightidx.mul";
const DIMENSION_BITS: u32 = 16;
const DIMENSION_MASK: u32 = (1 << DIMENSION_BITS) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightRecord {
    pub width: u16,
    pub height: u16,
    /// `width * height` intensity bytes.
    pub intensities: Vec<u8>,
    /// Payload bytes after the pixels.
    pub trailing: Vec<u8>,
}

impl LightRecord {
    pub fn decode(extra: u32, payload: &[u8]) -> eyre::Result<Self> {
        let (width, height) = ((extra & DIMENSION_MASK) as u16, (extra >> DIMENSION_BITS) as u16);
        let pixels = usize::from(width) * usize::from(height);
        if pixels == 0 || payload.len() < pixels {
            eyre::bail!("a {width}x{height} light needs {pixels} bytes, the entry has {}", payload.len());
        }
        Ok(Self { width, height, intensities: payload[..pixels].to_vec(), trailing: payload[pixels..].to_vec() })
    }

    /// The index extra and the payload.
    pub fn encode(&self) -> eyre::Result<(u32, Vec<u8>)> {
        let pixels = usize::from(self.width) * usize::from(self.height);
        if pixels == 0 || self.intensities.len() != pixels {
            eyre::bail!("a {}x{} light needs {pixels} intensities, not {}", self.width, self.height, self.intensities.len());
        }
        let extra = u32::from(self.width) | (u32::from(self.height) << DIMENSION_BITS);
        Ok((extra, [self.intensities.as_slice(), &self.trailing].concat()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightMulFile {
    pub container: IndexedMul,
}

impl LightMulFile {
    pub fn from_bytes(index: &[u8], data: &[u8]) -> eyre::Result<Self> {
        let file = Self { container: IndexedMul::from_bytes(index, data)? };
        for id in 0..file.len() {
            file.light(id).wrap_err_with(|| format!("light {id}"))?;
        }
        Ok(file)
    }

    pub fn load(client_dir: &Path) -> eyre::Result<Self> {
        let find = |name: &str| {
            crate::utils::path::find_file_ignoring_case(client_dir, name).ok_or_else(|| eyre!("{name} not in {}", client_dir.display()))
        };
        let (index, data) = (find(LIGHT_INDEX_FILE)?, find(LIGHT_DATA_FILE)?);
        Self::from_bytes(&std::fs::read(&index)?, &std::fs::read(&data)?).wrap_err_with(|| format!("{}", data.display()))
    }

    pub fn len(&self) -> usize {
        self.container.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.container.entries.is_empty()
    }

    pub fn light(&self, id: usize) -> eyre::Result<Option<LightRecord>> {
        match self.container.entries.get(id) {
            Some(IndexedEntry::Present { extra, payload }) => LightRecord::decode(*extra, payload).map(Some),
            _ => Ok(None),
        }
    }

    /// Replaces light `id`; `None` clears it.
    pub fn set_light(&mut self, id: usize, light: Option<&LightRecord>) -> eyre::Result<()> {
        match light {
            Some(light) => {
                let (extra, payload) = light.encode()?;
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
    fn light_round_trips_with_trailing_bytes() {
        let light = LightRecord { width: 3, height: 2, intensities: vec![0, 1, 2, 0x1F, 0xE1, 0], trailing: vec![9, 9] };
        let (extra, payload) = light.encode().unwrap();
        assert_eq!(extra, 3 | (2 << 16));
        assert_eq!(LightRecord::decode(extra, &payload).unwrap(), light);
        assert!(LightRecord { intensities: vec![0; 5], ..light }.encode().is_err());
    }
}
