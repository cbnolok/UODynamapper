//! Enhanced Client (EC) asset decoding and orchestration.
//!
//! For a detailed overview of the file relationships and architectural choices,
//! see [ENHANCED_CLIENT_FILES.md](ENHANCED_CLIENT_FILES.md).

// Category modules own the source files; re-exports preserve existing import paths.
mod uop_entries;
pub mod animation;
pub mod client_dialect;
pub mod effects;
pub mod graphics;
pub mod land;
pub mod maps;
pub mod metadata;
pub mod packages;
pub mod sound;
pub mod strings;
pub mod structures;

pub use animation::{animation_definition, animationframe, mobile_body, paperdoll};
pub use sound::{audio, audio_edit, kr_audio};
pub use effects::{chaos_script, effect_definition, effect_texture};
pub use graphics::{dds_writer, hues, hues_mul_uop, textures, tga_writer};
pub use maps::{
    facet_decoder, facet_definition, facet_encoder, facet_package, facet_radar, facet_sector,
    facet_to_classic, waypoints,
};
pub use land::{
    classic_tile_mapper, legacy_terrain, legacy_terrain_map, terrain, terrain_config,
    terrain_definition,
};
pub use metadata::{game_data, tile_database, tileart};
pub use structures::{housing, multis};
pub use strings::{localized_strings, string_dictionary};
pub use packages::{entry_edit, entry_image, entry_text, image_census, syntax_check};
