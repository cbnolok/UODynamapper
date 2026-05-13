//! Maps classic client (CC) terrain tile IDs to a "base" ID.
//!
//! This module provides a `ClassicTileMapper` struct that loads a mapping
//! configuration from a TOML file.

crate::eyre_imports!();
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// A lookup table for mapping Classic Client tile IDs to their base property IDs.
#[derive(Debug, Clone)]
pub struct ClassicTileMapper {
    lookup_table: Vec<u16>,
}

impl ClassicTileMapper {
    pub fn new<P: AsRef<Path>>(path: P) -> eyre::Result<Self> {
        let content = fs::read_to_string(path.as_ref()).wrap_err_with(|| {
            format!(
                "Failed to read classic tile mapping file at {:?}",
                path.as_ref()
            )
        })?;

        Self::from_toml_str(&content)
    }

    pub fn from_toml_str(content: &str) -> eyre::Result<Self> {
        let file: MapperFile =
            toml::from_str(&content).wrap_err("Failed to parse classic tile mapping TOML")?;

        Ok(Self::from_mapper_file(file))
    }

    fn from_mapper_file(file: MapperFile) -> Self {
        let mut lookup_table = vec![0u16; u16::MAX as usize + 1];
        for mapping in file.mapping {
            for old_id in mapping.old_ids {
                lookup_table[old_id as usize] = mapping.new_id;
            }
        }

        Self { lookup_table }
    }

    #[inline]
    pub fn get_base_id(&self, classic_id: u16) -> u16 {
        *self.lookup_table.get(classic_id as usize).unwrap_or(&0)
    }
}

#[derive(Debug, Deserialize)]
struct MapperFile {
    #[serde(rename = "mapping")]
    mapping: Vec<MappingEntry>,
}

#[derive(Debug, Deserialize)]
struct MappingEntry {
    new_id: u16,
    old_ids: Vec<u16>,
}
