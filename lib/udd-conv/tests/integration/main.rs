//! The crate's integration tests. Each module was a separate test binary; they share one so the crate is linked
//! once. Run one module with `cargo test -p udd-conv --test integration <module>::`; the ignored tests of
//! `effects_ec_package_client` need a real Enhanced Client, those of `ec_art_trim_client` a Classic one too, and those of
//! `tex_aux_ec_client` and `decoder_robustness` both under `UO_CLIENT_ROOTS`.

mod bc7;
mod cc_statics;
mod decoder_robustness;
mod ec_art_trim_client;
mod effects_ec_package_client;
mod map_statics_retrieval;
mod tex_art_cc;
mod tex_art_ec;
mod tex_aux_ec_client;
mod tex_land_ec;
mod tilemeta;
