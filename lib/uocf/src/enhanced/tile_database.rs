//! The central database for all tile-related rendering properties.

crate::eyre_imports!();
use crate::enhanced::classic_tile_mapper::ClassicTileMapper;
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::terrain_config::{
    LiquidLandData, SingleLandData, SolidLandData, TerrainDefinition,
};
use crate::enhanced::tileart::{ArtData, TileArtEntry};
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

        let mut definitions = HashMap::new();

        for file in tileart_package.iter_files() {
            if let Ok(tae) = TileArtEntry::parse_raw(file) {
                let art_data = tae.process(&string_dictionary);
                definitions.insert(art_data.id, art_data);
            }
        }
        Ok(Self { definitions })
    }
}

// endregion: --- Public API
