//! Defines the data structures and loading mechanism for simplified and aggregated terrain properties.
//!
//! This module is responsible for loading and parsing the `terrain.toml` file,
//! which defines the rendering properties for all land tiles in the game.

crate::eyre_imports!();
use bitflags::bitflags;
use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::path::Path;

bitflags! {
/// The fundamental type and behavior of a Land Tile, as defined in the configuration.
///
/// These flags are parsed from human-readable strings in the `terrain.toml` file
/// and determine which shader logic and data structures are used for the tile.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct LandTileType: u8 {
        const NONE = 0x00;
        const SOLID = 0x01;
        const LIQUID = 0x02;
        const SMOOTH = 0x04;
        const FOLLOW_CENTER = 0x08;
        const SINGLE = 0x10;
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SolidLandData {
    pub id: u16,
    pub name: String,
    #[serde(rename = "type")]
    pub tile_type: LandTileType,
    pub texture_ids: Vec<u32>,
    pub alpha_mask_id: u32,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct LiquidLandData {
    pub id: u16,
    pub name: String,
    #[serde(rename = "type")]
    pub tile_type: LandTileType,
    pub texture_id: u32,
    pub normal_map_id: u32,
    pub speed: f32,
    pub wave_height: f32,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SingleLandData {
    pub id: u16,
    pub name: String,
    #[serde(rename = "type")]
    pub tile_type: LandTileType,
    pub texture_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct TerrainDefinition {
    pub solid_data: Vec<SolidLandData>,
    pub liquid_data: Vec<LiquidLandData>,
    pub single_data: Vec<SingleLandData>,
}

#[derive(Debug, Deserialize)]
struct TerrainDefinitionFile {
    #[serde(default)]
    solid: Vec<SolidLandData>,
    #[serde(default)]
    liquid: Vec<LiquidLandData>,
    #[serde(default)]
    single: Vec<SingleLandData>,
}

impl TerrainDefinition {
    pub fn load<P: AsRef<Path>>(path: P) -> eyre::Result<Self> {
        let content = fs::read_to_string(path.as_ref()).wrap_err_with(|| {
            format!(
                "Failed to read terrain definition file at {:?}",
                path.as_ref()
            )
        })?;

        let definition_file: TerrainDefinitionFile =
            toml::from_str(&content).wrap_err("Failed to parse terrain definition TOML")?;

        Ok(TerrainDefinition {
            solid_data: definition_file.solid,
            liquid_data: definition_file.liquid,
            single_data: definition_file.single,
        })
    }
}

impl<'de> Deserialize<'de> for LandTileType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LandTileTypeVisitor;

        impl<'de> Visitor<'de> for LandTileTypeVisitor {
            type Value = LandTileType;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string with comma-separated terrain flags")
            }

            fn visit_str<E>(self, value: &str) -> Result<LandTileType, E>
            where
                E: de::Error,
            {
                let mut flags = LandTileType::NONE;
                for part in value.split(',') {
                    match part.trim().to_lowercase().as_str() {
                        "solid" => flags |= LandTileType::SOLID,
                        "liquid" => flags |= LandTileType::LIQUID,
                        "smooth" => flags |= LandTileType::SMOOTH,
                        "followcenter" => flags |= LandTileType::FOLLOW_CENTER,
                        "single" => flags |= LandTileType::SINGLE,
                        _ => {
                            return Err(de::Error::unknown_variant(
                                part,
                                &["Solid", "Liquid", "Smooth", "FollowCenter", "Single"],
                            ))
                        }
                    }
                }
                Ok(flags)
            }
        }

        deserializer.deserialize_str(LandTileTypeVisitor)
    }
}
