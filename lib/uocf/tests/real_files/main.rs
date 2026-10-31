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
mod classic_client_eras;
mod classic_dif_usage;
mod classic_facet_difs;
mod client_exe_facet_dimensions;
mod client_login_servers;
mod dds_client_textures;
mod effects_client_files;
mod facet_client_files;
mod facet_variants;
mod hues_mul_uop_client;
mod kr_client_files;
mod kr_facet_client_files;
mod legacy_terrain_client_file;
mod mobile_body_client;
mod multi_collection_client_entries;
mod terrain_definition_client_files;
mod tileart_client_records;
mod waypoints_client_file;
