//! Tests that build their inputs in memory or in temporary files, so they need no client. Each module was a
//! separate test binary; they share one so the crate is linked once. Run one module with
//! `cargo test -p uocf --test synthetic <module>::`.
//!
//! A few modules also hold ignored tests over real client files (`string_dictionary`, `uop_data_block_header`,
//! `uop_package_edit`), and `uop_hash_bruteforce` holds the ignored brute-force tests. The tests that only read
//! client files are in the `real_files` binary.

#[path = "../common/mod.rs"]
mod common;

mod art;
mod classic_body_def;
mod classic_bodyconv_def;
mod classic_facet_sources;
mod classic_map_difs;
mod classic_static_difs;
mod classic_verdata;
mod ec_material_axes;
mod fonts;
mod gump;
mod multimap_rle;
mod path_utils;
mod string_dictionary;
mod terrain_definition;
mod tileart;
mod tileart_ref_preservation;
mod uop_data_block_header;
mod uop_hash;
mod uop_hash_bruteforce;
mod uop_legacy_tests;
mod uop_package_edit;
mod uop_uop_tests;
