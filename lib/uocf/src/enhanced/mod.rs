//! Enhanced Client (EC) asset decoding and orchestration.
//!
//! For a detailed overview of the file relationships and architectural choices,
//! see [ENHANCED_CLIENT_FILES.md](ENHANCED_CLIENT_FILES.md).

pub mod animation_definition;
pub mod animationframe;
pub mod chaos_script;
pub mod classic_tile_mapper;
pub mod client_dialect;
pub mod dds_census;
pub mod dds_writer;
pub mod effect_definition;
pub mod effect_texture;
pub mod facet_decoder;
pub mod facet_definition;
pub mod facet_encoder;
pub mod facet_package;
pub mod facet_radar;
pub mod facet_sector;
pub mod housing;
pub mod hues;
pub mod hues_mul_uop;
pub mod localized_strings;
pub mod mobile_body;
pub mod legacy_terrain;
pub mod legacy_terrain_map;
pub mod multis;
pub mod string_dictionary;
pub mod terrain;
pub mod terrain_config;
pub mod terrain_definition;
pub mod textures;
pub mod tile_database;
pub mod tileart;
mod uop_entries;
pub mod waypoints;
