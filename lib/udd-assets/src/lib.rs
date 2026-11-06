// Category modules own the source files; re-exports preserve existing import paths.
pub mod animation;
pub mod metadata;
pub mod overrides;
pub mod textures;

pub use animation::{mobile_anim_cc, mobile_anim_ec, paperdoll_ec};
pub use metadata::{hues, map_metadata, tilemeta, world_lights};
pub use overrides::{ec_surface_overrides, ec_terrain_overrides, eckr_terrain_kdl};
pub use textures::{tex_art_cc, tex_art_ec, tex_aux_ec, tex_land_cc, tex_land_ec};

pub mod common;
pub use uot_texture_codecs::bc7;
pub use uot_texture_codecs::premultiplied;
pub mod effects_ec;
pub mod gumps;

pub use common::AtlasCacheOptions;
pub use tex_art_cc::TexArtCcPackage;
pub use tex_land_cc::TexLandCcPackage;
pub use tex_art_ec::TexArtEcPackage;
pub use tex_land_ec::TexLandEcPackage;
pub use tilemeta::TileMetaPackage;
pub use world_lights::WorldLightsPackage;
pub use hues::HuesPackage;
pub use mobile_anim_cc::MobileAnimCcPackage;
pub use mobile_anim_ec::MobileAnimEcPackage;
pub use paperdoll_ec::PaperdollEcPackage;
pub use effects_ec::EffectsEcPackage;
pub use gumps::GumpsPackage;
