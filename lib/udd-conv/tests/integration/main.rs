//! The crate's integration tests. Each module was a separate test binary; they share one so the crate is linked
//! once. Run one module with `cargo test -p udd-conv --test integration <module>::`; the ignored tests of
//! `effects_ec_package_client` need a real Enhanced Client.

mod bc7;
mod cc_statics;
mod effects_ec_package_client;
mod tex_art_cc;
mod tex_art_ec;
mod tex_land_ec;
mod tilemeta;
