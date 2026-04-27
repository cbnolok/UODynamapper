//! High-level API for reading enhanced client terrain textures.

use crate::enhanced::textures::{TextureFile, Textures, ECImageFormat};
use color_eyre::eyre::Result;

// region: --- Public API (Convenience & Application Use)

/// A high-level reader for managing Enhanced Client terrain textures.
///
/// This structure acts as an orchestrator that simplifies access to land textures
/// by managing both "World" (native EC) and "Legacy" (CC-style) texture packages.
/// It handles the path formatting and hashing logic internally.
pub struct TerrainReader {
    world_terrain_textures: Textures,
    legacy_terrain_textures: Textures,
}

impl TerrainReader {
    /// Creates a new `TerrainReader`.
    pub fn new(world_terrain_path: &str, legacy_terrain_path: &str) -> Result<Self> {
        let world_terrain_textures = Textures::new(world_terrain_path.as_ref(), None, None)?;
        let legacy_terrain_textures = Textures::new(legacy_terrain_path.as_ref(), None, None)?;
        Ok(Self {
            world_terrain_textures,
            legacy_terrain_textures,
        })
    }

    /// Retrieves a terrain texture.
    pub fn get_terrain_texture(&self, texture_id: u32, is_legacy: bool) -> Result<Option<TextureFile>> {
        use std::io::Write;
        let mut path_buf = [0u8; 64];

        if is_legacy {
            // Note: UOP maps legacy terrain textures mostly to .dds 
            let mut slice = &mut path_buf[..];
            write!(slice, "build/legacyland/{:08}.dat", texture_id).unwrap();
            let len = 64 - slice.len();
            let path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            let hash = crate::uop::hash::hash_file_name_single(path);
            self.legacy_terrain_textures.get_from_hash(hash, Some(path), ECImageFormat::DDS)
        } else {
            let mut slice = &mut path_buf[..];
            write!(slice, "build/worldart/land/{:08}.dds", texture_id).unwrap();
            let len = 64 - slice.len();
            let path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            let hash = crate::uop::hash::hash_file_name_single(path);
            self.world_terrain_textures.get_from_hash(hash, Some(path), ECImageFormat::DDS)
        }
    }
}

// endregion: --- Public API
