// Category modules own the source files; re-exports preserve existing import paths.
pub mod animation;
pub mod audio;
pub mod formats;
pub mod graphics;
pub mod maps;
pub mod multis;
pub mod tables;
pub mod target_client;
pub mod tiledata;
pub mod ui;

pub use animation::{anim, anim_frames, animationframe_cc, animdata, body_def, bodyconv_def};
pub use audio::{audio_def, music, sound, sound_edit};
pub use graphics::{
    art, art_write, hues, image_files, land_texture, light, light_write, texmap_write,
};
pub use maps::{
    dif_files, facet_image, facet_image_render, facet_sources, map, map_statics_diff,
    map_write, multimap_render, multimap_rle, radar, radarcol, statics, statics_write, world,
};
pub use multis::{multi, multi_write};
pub use formats::{
    generic_index, iff, indexed_mul, michelangelo_uop_codec, small_binaries, vd_codec, verdata,
};
pub(crate) use tables::text_mode_lines;
pub use tables::{
    asset_map, cliloc, def_files, filter_lists, generic_def, house_tables, key_names,
    legacy_tables, localized_text, professions, skills, speech,
};
pub use ui::{credits, fonts, gump, gump_write, help_index, paperdoll_profiles};
