//! The central database for all tile-related rendering properties.

crate::eyre_imports!();
use crate::enhanced::classic_tile_mapper::ClassicTileMapper;
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::terrain_config::{
    LiquidLandData, SingleLandData, SolidLandData, TerrainDefinition,
};
use crate::enhanced::tileart::{ArtData, TileArtEntry, tileart_entry_hash};
use crate::uop_container::package::UopPackage;
use std::collections::HashMap;
use std::path::Path;

// region: --- Public API (Convenience & Application Use)

/// Higher-level enum used by the application to categorize terrain into its specific
/// behavior types (Solid, Liquid, or Single textures) without worrying about raw ID ranges.
#[derive(Debug, Clone)]
pub enum TerrainProperties<'a> {
    Solid(&'a SolidLandData),
    Liquid(&'a LiquidLandData),
    Single(&'a SingleLandData),
}

/// The high-level facade for all Enhanced Client tile metadata.
///
/// This database orchestrates multiple underlying sources (Terrain Definitions, TileArt,
/// and Classic-to-Enhanced mappings) to provide a unified lookup interface for the engine.
/// It is the primary way the renderer queries tile properties.
#[derive(Debug, Clone)]
pub struct TileDatabase {
    classic_tile_mapper: ClassicTileMapper,
    terrain_definition: TerrainDefinition,
    art_definition: ArtDefinition,
}

impl TileDatabase {
    pub fn new(
        terrain_transcode_path: &Path,
        terrain_definition_path: &Path,
        tileart_path: &Path,
        string_dict_path: &Path,
    ) -> eyre::Result<Self> {
        let classic_tile_mapper = ClassicTileMapper::new(terrain_transcode_path)
            .wrap_err("Failed to load ClassicTileMapper")?;

        let terrain_definition = TerrainDefinition::load(terrain_definition_path)
            .wrap_err("Failed to load TerrainDefinition")?;

        let art_definition = ArtDefinition::load(tileart_path, string_dict_path)
            .wrap_err("Failed to load ArtDefinition")?;

        Ok(Self {
            classic_tile_mapper,
            terrain_definition,
            art_definition,
        })
    }

    pub fn get_terrain_properties(&'_ self, classic_tile_id: u16) -> Option<TerrainProperties<'_>> {
        let base_id = self.classic_tile_mapper.get_base_id(classic_tile_id);

        if let Some(data) = self
            .terrain_definition
            .solid_data
            .iter()
            .find(|&d| d.id == base_id)
        {
            return Some(TerrainProperties::Solid(data));
        }

        if let Some(data) = self
            .terrain_definition
            .liquid_data
            .iter()
            .find(|&d| d.id == base_id)
        {
            return Some(TerrainProperties::Liquid(data));
        }

        if let Some(data) = self
            .terrain_definition
            .single_data
            .iter()
            .find(|&d| d.id == base_id)
        {
            return Some(TerrainProperties::Single(data));
        }

        None
    }

    pub fn get_static_properties(&self, tile_id: u16) -> Option<&ArtData> {
        self.art_definition.definitions.get(&tile_id)
    }
}

/// A collection of all processed static art definitions from `tileart.uop`.
///
/// This structure acts as a container for `ArtData`, providing fast lookup by `tile_id`.
/// It abstracts away the complexity of iterating through UOP blocks and resolving
/// string dictionary references.
#[derive(Debug, Clone)]
pub struct ArtDefinition {
    pub definitions: HashMap<u16, ArtData>,
}

impl ArtDefinition {
    pub fn load(tileart_path: &Path, string_dict_path: &Path) -> eyre::Result<Self> {
        let tileart_package =
            UopPackage::load(tileart_path).wrap_err("Failed to load tileart.uop")?;
        let string_dictionary = UoStringDictionary::load(string_dict_path)
            .wrap_err("Failed to load string_dictionary.uop")?;

        Self::from_package(&tileart_package, &string_dictionary)
    }

    /// Resolve the 16-bit static-art namespace by asset name, as the client does.
    pub fn from_package(package: &UopPackage, strings: &UoStringDictionary) -> eyre::Result<Self> {
        let mut definitions = HashMap::new();
        for id in u16::MIN..=u16::MAX {
            let Some(file) = package.get_file_by_hash(tileart_entry_hash(u32::from(id))) else { continue; };
            // Some original records store another tile's ID. Large creature
            // placeholder IDs also must not wrap into this static namespace.
            let entry = TileArtEntry::parse_raw(file)
                .wrap_err_with(|| format!("parse tileart asset {id}"))?;
            let mut art_data = entry.process(strings);
            art_data.id = id;
            definitions.insert(id, art_data);
        }
        Ok(Self { definitions })
    }
}

// endregion: --- Public API

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::tileart::{TaeTexture, TILEART_RECORD_VERSION, tileart_entry_path};
    use crate::uop_container::file::CompressionFlag;

    #[test]
    fn static_assets_use_entry_names_and_keep_the_stored_source_id() {
        let strings = UoStringDictionary::from_payload_bytes(&[0; 16], "empty test dictionary").unwrap();
        let mut package = UopPackage::new_default();
        let creature = 10_000_001u32;
        let wrapped = creature as u16;
        for (asset, stored) in [(43956, 43956), (43957, 43956),
            (u32::from(wrapped), u32::from(wrapped)), (creature, creature)] {
            let entry = TileArtEntry { version: TILEART_RECORD_VERSION, tile_id: stored,
                texture_vector: vec![TaeTexture::default(); 4], ..TileArtEntry::default() };
            package.add_file_from_memory(&entry.to_bytes().unwrap(), &tileart_entry_path(asset),
                CompressionFlag::Zlib).unwrap();
        }
        let art = ArtDefinition::from_package(&package, &strings).unwrap();
        assert_eq!(art.definitions.len(), 3);
        assert_eq!(art.definitions[&43956].source_tile_id, 43956);
        assert_eq!(art.definitions[&43957].id, 43957);
        assert_eq!(art.definitions[&43957].source_tile_id, 43956);
        assert_eq!(art.definitions[&wrapped].source_tile_id, u32::from(wrapped));
    }

    #[test]
    fn malformed_named_static_is_reported_instead_of_silently_missing() {
        let strings = UoStringDictionary::from_payload_bytes(&[0; 16], "empty test dictionary").unwrap();
        let mut package = UopPackage::new_default();
        package.add_file_from_memory(&[0, 0], &tileart_entry_path(43957), CompressionFlag::Zlib).unwrap();
        let error = ArtDefinition::from_package(&package, &strings).unwrap_err();
        assert!(format!("{error:#}").contains("parse tileart asset 43957"));
    }
}
