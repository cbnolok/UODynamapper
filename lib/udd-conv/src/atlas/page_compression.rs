//! How the lossless RGBA atlas pages of gumps, land and art textures are compressed.
//!
//! `gumps_cc`, `gumps_ec`, `tex_land_cc`, `tex_land_ec`, `tex_art_cc`, `tex_art_ec` and the `tex_aux_ec` textures store
//! their lossless pages as RGBA8888 with plain Zstd 19 (2026-11-06). The measurements against the image codec it
//! replaced are in `docs/agent_notes/uddp_anim_format_requirements.md`.

use udd_container::CompressionFlag;

/// The Zstd level of lossless gump, land and art pages unless the packer is asked for another.
pub const LOSSLESS_PAGE_ZSTD_LEVEL: i32 = 19;

/// The builder codec of a lossless gump, land or art page unless the packer is asked for another.
pub const LOSSLESS_PAGE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL);
