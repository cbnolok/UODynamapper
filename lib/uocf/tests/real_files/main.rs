//! Ignored tests over real client files; the variables naming the client directories are documented in
//! `common` and in each module. Each module was a separate test binary; they share one so the crate is linked
//! once, and their tests now run in parallel with each other. Run one module with
//! `cargo test -p uocf --test real_files <module>:: -- --ignored`.

#[path = "../common/mod.rs"]
mod common;

mod animation_client_entries;
mod animation_definition_editing;
mod animation_frame_encoding;
mod audio_client_files;
mod audio_writers;
mod classic_animation_frame_encoding;
mod classic_art_encoding;
mod classic_client_eras;
mod classic_def_files;
mod classic_difs_fonts_and_music;
mod classic_dif_usage;
mod classic_image_import;
mod classic_indexed_mul_writers;
mod classic_map_writers;
mod classic_facet_difs;
mod classic_mul_writers;
mod classic_speech_and_skills;
mod classic_text_tables;
mod classic_credits_help_asset_map;
mod classic_facet_images;
mod classic_ui_files;
mod client_exe_facet_dimensions;
mod client_login_servers;
mod dds_client_textures;
mod effects_client_files;
mod entry_editor_client_packages;
mod facet_client_files;
mod facet_variants;
mod filter_lists;
mod game_data_client_tables;
mod gump_client_encoding;
mod hues_mul_uop_client;
mod kr_client_files;
mod kr_facet_client_files;
mod legacy_terrain_client_file;
mod mobile_body_client;
mod multi_collection_client_entries;
mod paperdoll_frame_sets;
mod string_tables_client_files;
mod syntax_client_files;
mod terrain_definition_client_files;
mod tga_client_encoding;
mod tileart_client_records;
mod uop_package_unchanged_save;
mod waypoints_client_file;
