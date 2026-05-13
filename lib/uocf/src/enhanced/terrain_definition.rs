//! Decoder for Enhanced Client `TerrainDefinition.uop` entries.
//!
//! Each entry is a terrain material definition:
//! - `id` is the material entry index inside the UOP payload.
//! - `aliases[*].alias` are the actual terrain tile ids using that material.
//! - the trailing texture block is a full layered `TextureItem`-style payload
//!   describing the source textures needed to render that terrain material.
//!
//! This package is therefore authoritative for both:
//! - the land tile ids owned by terrain materials
//! - the world/legacy texture ids that must be excluded from `ec_art`
//! - the primary terrain texture currently packed into `ec_land`

crate::eyre_imports!();
use crate::enhanced::{string_dictionary::UoStringDictionary, textures::TextureItem};
use crate::uop_container::{file::UopFile, package::UopPackage};
use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerrainTextureType {
    #[default]
    Undefined,
    WorldArt,
    TileArtLegacy,
    TileArtEnhanced,
    Textures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerrainDefinitionTileAlias {
    pub count_index: u32,
    pub alias: u32,
    pub tile_flags: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerrainDefinitionTextureLayer {
    pub name_string_off: i32,
    pub texture_id: Option<u32>,
    pub path: Option<String>,
    pub texture_type: TerrainTextureType,
    pub unk4: u8,
    pub texture_repetition: f32,
    pub unk6: i32,
    pub unk7: i32,
}

impl TerrainDefinitionTextureLayer {
    fn is_primary_candidate(&self) -> bool {
        (4.0..=8.0).contains(&self.texture_repetition)
    }

    fn is_support_layer(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };

        let path = path.to_ascii_lowercase();
        path.contains("noise")
            || path.contains("normal")
            || path.contains("mask")
            || path.contains("_alpha")
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerrainDefinitionTexture {
    pub unk1: u8,
    pub shader_name_id: i32,
    pub shader_name: Option<String>,
    pub layers: Vec<TerrainDefinitionTextureLayer>,
    pub unk8: Vec<i32>,
    pub unk9: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerrainDefinitionEntry {
    pub name_id: i32,
    pub id: u32,
    pub unk: f32,
    pub unk2: f32,
    pub unk3: f32,
    pub name: Option<String>,
    pub aliases: Vec<TerrainDefinitionTileAlias>,
    pub texture: Option<TerrainDefinitionTexture>,
}

#[derive(Debug, Clone, Default)]
pub struct TerrainDefinitionPackage {
    pub entries: Vec<TerrainDefinitionEntry>,
}

impl TerrainDefinitionPackage {
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let string_dictionary = find_neighboring_string_dictionary(path)
            .map(|dict_path| UoStringDictionary::load(&dict_path))
            .transpose()?;
        let package = UopPackage::load(path)
            .wrap_err_with(|| format!("Failed to load {:?}", path))?;
        Self::from_package(&package, string_dictionary.as_ref())
    }

    pub fn load_with_dictionary(path: &Path, string_dict_path: &Path) -> eyre::Result<Self> {
        let string_dictionary = UoStringDictionary::load(string_dict_path)
            .wrap_err_with(|| format!("Failed to load {:?}", string_dict_path))?;
        let package = UopPackage::load(path)
            .wrap_err_with(|| format!("Failed to load {:?}", path))?;
        Self::from_package(&package, Some(&string_dictionary))
    }

    pub fn from_package(
        package: &UopPackage,
        string_dictionary: Option<&UoStringDictionary>,
    ) -> eyre::Result<Self> {
        let mut entries = Vec::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            entries.push(parse_entry(file, string_dictionary)?);
        }
        entries.sort_by_key(|entry| entry.id);
        Ok(Self { entries })
    }

    pub fn land_tile_ids(&self) -> BTreeSet<u32> {
        self.entries
            .iter()
            .flat_map(|entry| entry.runtime_slot_ids())
            .collect()
    }

    pub fn land_slot_ids(&self) -> BTreeSet<u32> {
        self.land_tile_ids()
    }

    pub fn land_source_texture_ids(&self) -> BTreeSet<u32> {
        self.entries
            .iter()
            .flat_map(|entry| {
                entry
                    .texture
                    .iter()
                    .flat_map(|texture| texture.layers.iter().filter_map(|layer| layer.texture_id))
            })
            .collect()
    }

    pub fn texture_selections(&self) -> Vec<(u32, u32)> {
        let mut selections = BTreeMap::new();

        for entry in &self.entries {
            let Some(texture_id) = entry.primary_texture_id() else {
                continue;
            };

            for slot_id in entry.runtime_slot_ids() {
                selections.entry(slot_id).or_insert(texture_id);
            }
        }

        selections.into_iter().collect()
    }
}

impl TerrainDefinitionEntry {
    pub fn runtime_slot_ids(&self) -> Vec<u32> {
        let concrete_aliases = self
            .aliases
            .iter()
            .map(|alias| alias.alias)
            .filter(|alias| *alias != 0)
            .collect::<Vec<_>>();

        if concrete_aliases.is_empty() {
            vec![self.id]
        } else {
            concrete_aliases
        }
    }

    pub fn primary_texture_id(&self) -> Option<u32> {
        let texture = self.texture.as_ref()?;

        texture
            .layers
            .iter()
            .filter(|layer| layer.texture_id.is_some())
            .min_by(|left, right| {
                let left_rank = (
                    left.is_support_layer(),
                    !left.is_primary_candidate(),
                    left.unk6,
                    left.name_string_off,
                );
                let right_rank = (
                    right.is_support_layer(),
                    !right.is_primary_candidate(),
                    right.unk6,
                    right.name_string_off,
                );
                left_rank.cmp(&right_rank)
            })
            .and_then(|layer| layer.texture_id)
    }
}

pub fn parse_entry(
    uop_file: &UopFile,
    string_dictionary: Option<&UoStringDictionary>,
) -> eyre::Result<TerrainDefinitionEntry> {
    let bytes = uop_file.unpack()?;
    let mut reader = Cursor::new(bytes.as_slice());

    let name_id = reader.read_i32::<LittleEndian>()?;
    let id = reader.read_u32::<LittleEndian>()?;
    let unk = reader.read_f32::<LittleEndian>()?;
    let unk2 = reader.read_f32::<LittleEndian>()?;
    let unk3 = reader.read_f32::<LittleEndian>()?;
    let alias_count = reader.read_u32::<LittleEndian>()?;

    let mut aliases = Vec::with_capacity(alias_count as usize);
    for _ in 0..alias_count {
        aliases.push(TerrainDefinitionTileAlias {
            count_index: reader.read_u32::<LittleEndian>()?,
            alias: reader.read_u32::<LittleEndian>()?,
            tile_flags: reader.read_u64::<LittleEndian>()?,
        });
    }

    let texture = if reader.position() < bytes.len() as u64 {
        let raw_texture = TextureItem::read(&mut reader)?;
        raw_texture
            .texture_present
            .then(|| resolve_texture_item(raw_texture, string_dictionary))
    } else {
        None
    };

    Ok(TerrainDefinitionEntry {
        name_id,
        id,
        unk,
        unk2,
        unk3,
        name: resolve_dictionary_string(string_dictionary, name_id as u32),
        aliases,
        texture,
    })
}

fn resolve_texture_item(
    raw_texture: TextureItem,
    string_dictionary: Option<&UoStringDictionary>,
) -> TerrainDefinitionTexture {
    let layers = raw_texture
        .images
        .into_iter()
        .map(|image| {
            let path = resolve_dictionary_string_i32(string_dictionary, image.string_dictionary_offset);
            let texture_type = classify_texture_path(path.as_deref());
            let texture_id = path.as_deref().and_then(crate::utils::path::extract_texture_id_from_path);

            TerrainDefinitionTextureLayer {
                name_string_off: image.string_dictionary_offset,
                texture_id,
                path,
                texture_type,
                unk4: image.unk4,
                texture_repetition: image.texture_repetition,
                unk6: image.unk6,
                unk7: image.unk7,
            }
        })
        .collect();

    TerrainDefinitionTexture {
        unk1: raw_texture.unk1,
        shader_name_id: raw_texture.name_index,
        shader_name: resolve_dictionary_string_i32(string_dictionary, raw_texture.name_index),
        layers,
        unk8: raw_texture.unk8,
        unk9: raw_texture.unk9,
    }
}

fn resolve_dictionary_string(
    string_dictionary: Option<&UoStringDictionary>,
    string_offset: u32,
) -> Option<String> {
    if string_offset == 0 {
        return None;
    }
    string_dictionary
        .and_then(|dict| dict.get_string((string_offset - 1) as usize))
        .map(str::to_owned)
}

fn resolve_dictionary_string_i32(
    string_dictionary: Option<&UoStringDictionary>,
    string_offset: i32,
) -> Option<String> {
    (string_offset > 0)
        .then_some(string_offset as u32)
        .and_then(|offset| resolve_dictionary_string(string_dictionary, offset))
}

fn classify_texture_path(path: Option<&str>) -> TerrainTextureType {
    match path.map(crate::utils::path::normalize_dictionary_path) {
        Some(value) if value.contains("data\\worldart\\") => TerrainTextureType::WorldArt,
        Some(value) if value.contains("data\\tileartlegacy\\") => {
            TerrainTextureType::TileArtLegacy
        }
        Some(value) if value.contains("data\\tileartenhanced\\") => {
            TerrainTextureType::TileArtEnhanced
        }
        Some(value) if value.contains("data\\textures\\") => TerrainTextureType::Textures,
        _ => TerrainTextureType::Undefined,
    }
}


fn find_neighboring_string_dictionary(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    ["string_dictionary.uop"]
        .into_iter()
        .map(|name| parent.join(name))
        .find(|candidate| candidate.exists())
}



