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
use crate::uop::{file::UopFile, package::UopPackage};
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

fn parse_entry(
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
            let texture_id = path.as_deref().and_then(extract_texture_id_from_path);

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
    match path.map(normalize_dictionary_path) {
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

fn extract_texture_id_from_path(path: &str) -> Option<u32> {
    let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file_name.split('.').next().unwrap_or(file_name);

    stem.split('_')
        .next()
        .and_then(extract_first_digit_run)
        .or_else(|| extract_first_digit_run(stem))
}

fn normalize_dictionary_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

fn extract_first_digit_run(segment: &str) -> Option<u32> {
    let start = segment.find(|c: char| c.is_ascii_digit())?;
    let digits = segment[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)?.parse().ok()
}

fn find_neighboring_string_dictionary(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    ["string_dictionary.uop", "string_Wdictionary.uop"]
        .into_iter()
        .map(|name| parent.join(name))
        .find(|candidate| candidate.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uop::file::CompressionFlag;
    use byteorder::WriteBytesExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn serialize_dictionary_payload(strings: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
        bytes.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x48u32.to_le_bytes());
        for string in strings {
            let string_bytes = string.as_bytes();
            bytes.extend_from_slice(&(string_bytes.len() as u16).to_le_bytes());
            bytes.extend_from_slice(string_bytes);
        }
        bytes
    }

    fn serialize_entry_payload(
        name_id: i32,
        id: u32,
        aliases: &[(u32, u32, u64)],
        shader_name_string_off: i32,
        texture_images: &[(i32, u8, f32, i32, i32)],
        unk8: &[i32],
        unk9: &[f32],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(name_id).unwrap();
        bytes.write_u32::<LittleEndian>(id).unwrap();
        bytes.write_f32::<LittleEndian>(1.25).unwrap();
        bytes.write_f32::<LittleEndian>(2.5).unwrap();
        bytes.write_f32::<LittleEndian>(3.75).unwrap();
        bytes.write_u32::<LittleEndian>(aliases.len() as u32).unwrap();
        for (count_index, alias, tile_flags) in aliases {
            bytes.write_u32::<LittleEndian>(*count_index).unwrap();
            bytes.write_u32::<LittleEndian>(*alias).unwrap();
            bytes.write_u64::<LittleEndian>(*tile_flags).unwrap();
        }
        bytes.write_u8(1).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_i32::<LittleEndian>(shader_name_string_off).unwrap();
        bytes.write_u8(texture_images.len() as u8).unwrap();
        for (name_string_off, unk4, texture_repetition, unk6, unk7) in texture_images {
            bytes.write_i32::<LittleEndian>(*name_string_off).unwrap();
            bytes.write_u8(*unk4).unwrap();
            bytes.write_f32::<LittleEndian>(*texture_repetition).unwrap();
            bytes.write_i32::<LittleEndian>(*unk6).unwrap();
            bytes.write_i32::<LittleEndian>(*unk7).unwrap();
        }
        bytes.write_u32::<LittleEndian>(unk8.len() as u32).unwrap();
        for value in unk8 {
            bytes.write_i32::<LittleEndian>(*value).unwrap();
        }
        bytes.write_u32::<LittleEndian>(unk9.len() as u32).unwrap();
        for value in unk9 {
            bytes.write_f32::<LittleEndian>(*value).unwrap();
        }
        bytes
    }

    fn temp_uop_path(test_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("uocf_terrain_{test_name}_{timestamp}.uop"))
    }

    fn write_uop_file(test_name: &str, payload: &[u8], virtual_path: &str) -> PathBuf {
        let path = temp_uop_path(test_name);
        let mut package = UopPackage::new_default();
        package
            .add_file_from_memory(payload, virtual_path, CompressionFlag::Zlib)
            .expect("add file payload");
        package.finalize_and_save(&path).expect("save package");
        path
    }

    #[test]
    fn terrain_definition_decoder_reads_ids_aliases_and_texture_path() {
        let dict_path = write_uop_file(
            "dictionary",
            &serialize_dictionary_payload(&[
                "Grass",
                "UOWaterTerrainLayer",
                "01000013_water_alpha.tga",
                "02000051_water.tga",
                "01000017_cube3.tga",
            ]),
            "build/string_dictionary.bin",
        );
        let terrain_path = write_uop_file(
            "terrain",
            &serialize_entry_payload(
                1,
                1,
                &[(0, 3, 0), (1, 561, 0x200), (2, 433, 0x400)],
                2,
                &[(3, 0, 16.0, 0, 0), (4, 0, 4.0, 1, 0), (5, 0, 1.0, 2, 0)],
                &[0, 1, 0, 0],
                &[-0.01, 0.0, 0.0, 0.0],
            ),
            "build/terraindefinition/00000000.bin",
        );

        let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
            .expect("load terrain definition package");

        assert_eq!(package.entries.len(), 1);
        let entry = &package.entries[0];
        assert_eq!(entry.id, 1);
        assert_eq!(entry.name.as_deref(), Some("Grass"));
        assert_eq!(entry.aliases.len(), 3);
        assert_eq!(entry.aliases[0].alias, 3);
        let texture = entry.texture.as_ref().expect("texture block should decode");
        assert_eq!(texture.shader_name.as_deref(), Some("UOWaterTerrainLayer"));
        assert_eq!(texture.layers.len(), 3);
        assert_eq!(texture.layers[0].path.as_deref(), Some("01000013_water_alpha.tga"));
        assert_eq!(texture.layers[1].texture_id, Some(2_000_051));
        assert_eq!(texture.layers[1].texture_repetition, 4.0);
        assert_eq!(texture.unk8, vec![0, 1, 0, 0]);
        assert_eq!(texture.unk9, vec![-0.01, 0.0, 0.0, 0.0]);
        assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![3, 433, 561]);
        assert_eq!(package.land_slot_ids().into_iter().collect::<Vec<_>>(), vec![3, 433, 561]);
        assert_eq!(
            package.land_source_texture_ids().into_iter().collect::<Vec<_>>(),
            vec![1_000_013, 1_000_017, 2_000_051]
        );
        assert_eq!(
            package.texture_selections(),
            vec![(3, 2_000_051), (433, 2_000_051), (561, 2_000_051)]
        );

        std::fs::remove_file(dict_path).ok();
        std::fs::remove_file(terrain_path).ok();
    }

    #[test]
    fn terrain_definition_decoder_keeps_entry_index_separate_from_land_tile_ids() {
        let terrain_path = write_uop_file(
            "terrain_nodict",
            &serialize_entry_payload(0, 77, &[(0, 900, 0), (1, 901, 0)], 0, &[], &[], &[]),
            "build/terraindefinition/00000000.bin",
        );

        let package = TerrainDefinitionPackage::load(&terrain_path)
            .expect("load terrain definition package without dictionary");

        assert_eq!(package.entries[0].id, 77);
        assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![900, 901]);
        assert!(package.land_source_texture_ids().is_empty());
        assert!(package.texture_selections().is_empty());

        std::fs::remove_file(terrain_path).ok();
    }

    #[test]
    fn terrain_definition_primary_texture_ignores_support_layers() {
        let dict_path = write_uop_file(
            "dictionary_support_layers",
            &serialize_dictionary_payload(&[
                "Sand Cliff E-W",
                "UODefaultTerrainLayer",
                "02000540_Sand_Cliff_EW_A.tga",
                "02000541_Sand_Cliff_EW_B.tga",
                "01000003_noise_alpha.tga",
            ]),
            "build/string_dictionary.bin",
        );
        let terrain_path = write_uop_file(
            "terrain_support_layers",
            &serialize_entry_payload(
                1,
                54,
                &[(0, 54, 0)],
                2,
                &[(3, 0, 1.0, 0, 0), (4, 0, 1.0, 1, 0), (5, 0, 8.0, 2, 0)],
                &[],
                &[],
            ),
            "build/terraindefinition/00000000.bin",
        );

        let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
            .expect("load terrain definition package");

        assert_eq!(package.entries.len(), 1);
        assert_eq!(package.entries[0].primary_texture_id(), Some(2_000_540));
        assert_eq!(package.texture_selections(), vec![(54, 2_000_540)]);

        std::fs::remove_file(dict_path).ok();
        std::fs::remove_file(terrain_path).ok();
    }

    #[test]
    fn terrain_definition_placeholder_alias_entries_use_material_id_as_runtime_slot() {
        let dict_path = write_uop_file(
            "dictionary_placeholder_alias_slot",
            &serialize_dictionary_payload(&[
                "Tile (blue slate) Ornate",
                "UODefaultTerrainLayer",
                "02000880_Tile_Blue_Slate_A.tga",
                "02000881_Tile_Blue_Slate_B.tga",
                "01000003_noise_alpha.tga",
            ]),
            "build/string_dictionary.bin",
        );
        let terrain_path = write_uop_file(
            "terrain_placeholder_alias_slot",
            &serialize_entry_payload(
                1,
                100,
                &[(0, 0, 0)],
                2,
                &[(3, 0, 4.0, 0, 0), (4, 0, 4.0, 1, 0), (5, 0, 16.0, 2, 0)],
                &[],
                &[],
            ),
            "build/terraindefinition/00000000.bin",
        );

        let package = TerrainDefinitionPackage::load_with_dictionary(&terrain_path, &dict_path)
            .expect("load terrain definition package");

        assert_eq!(package.entries[0].runtime_slot_ids(), vec![100]);
        assert_eq!(package.land_tile_ids().into_iter().collect::<Vec<_>>(), vec![100]);
        assert_eq!(package.texture_selections(), vec![(100, 2_000_880)]);

        std::fs::remove_file(dict_path).ok();
        std::fs::remove_file(terrain_path).ok();
    }

    #[test]
    fn terrain_definition_extract_texture_id_normalizes_dictionary_name_patterns() {
        assert_eq!(
            extract_texture_id_from_path("Data\\WorldArt\\00000002_ankh.tga"),
            Some(2)
        );
        assert_eq!(
            extract_texture_id_from_path("Data\\TileArtLegacy\\3.tga"),
            Some(3)
        );
        assert_eq!(
            extract_texture_id_from_path("Data/TileArtEnhanced/02000540_Sand_Cliff_EW_A.tga"),
            Some(2000540)
        );
    }

    #[test]
    fn terrain_definition_classify_texture_path_normalizes_case_and_slashes() {
        assert_eq!(
            classify_texture_path(Some("data/worldart/00000002_ankh.tga")),
            TerrainTextureType::WorldArt
        );
        assert_eq!(
            classify_texture_path(Some("DATA\\TILEARTLEGACY\\3.tga")),
            TerrainTextureType::TileArtLegacy
        );
        assert_eq!(
            classify_texture_path(Some("Data/TileArtEnhanced/00000004_tree.tga")),
            TerrainTextureType::TileArtEnhanced
        );
    }
}
