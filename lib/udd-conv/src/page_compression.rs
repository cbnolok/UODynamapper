//! How the lossless RGBA atlas pages of gumps, land and art textures are compressed.
//!
//! `gumps_cc`, `gumps_ec`, `tex_land_cc` and `tex_land_ec` store their lossless pages as RGBA8888 with plain Zstd 19
//! (2026-11-06). Against lossless JPEG XL at effort 6, measured on the installed packages' pages:
//! - pages on disk: gumps_cc 2.02 MB against 1.65 MB, gumps_ec 2.87 against 2.16 (pages are a small share of the 45 and
//!   83 MB packages), tex_land_cc 11.17 against 11.07, tex_land_ec 17.19 against 17.73;
//! - decode: Zstd on one thread is 4..20x faster than JPEG XL on 20 threads with the rayon runner (a gump page about
//!   1.5 ms against 31 ms).
//!
//! `tex_art_cc`, `tex_art_ec` and the `tex_aux_ec` textures followed (the user, 2026-11-06: "Yes switch to zstd 19"),
//! with identical pixels: 48.66 MB against 50.32, 58.19 against 69.20 and 4.61 against 5.19; all pages of each decode
//! 4..60x faster on one thread than JPEG XL on 20 (380 ms against 4249, 602 against 2595, 39 against 2365).
//!
//! JPEG XL stays available as an explicit choice.

use udd_container::CompressionFlag;

/// The Zstd level of lossless gump, land and art pages unless the packer is asked for another.
pub const LOSSLESS_PAGE_ZSTD_LEVEL: i32 = 19;

/// The builder codec of a lossless gump, land or art page unless the packer is asked for another.
pub const LOSSLESS_PAGE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL);
