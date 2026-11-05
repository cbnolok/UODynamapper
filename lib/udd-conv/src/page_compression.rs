//! How the lossless RGBA atlas pages of gumps and land textures are compressed.
//!
//! `gumps_cc`, `gumps_ec`, `tex_land_cc` and `tex_land_ec` store their lossless pages as RGBA8888 with plain Zstd 19
//! (2026-11-06). Against lossless JPEG XL at effort 6, measured on the installed packages' pages:
//! - pages on disk: gumps_cc 2.02 MB against 1.65 MB, gumps_ec 2.87 against 2.16 (pages are a small share of the 45 and
//!   83 MB packages), tex_land_cc 11.17 against 11.07, tex_land_ec 17.19 against 17.73;
//! - decode: Zstd on one thread is 4..20x faster than JPEG XL on 20 threads with the rayon runner (a gump page about
//!   1.5 ms against 31 ms).
//!
//! JPEG XL stays available as an explicit choice.

use udd_container::CompressionFlag;

/// The Zstd level of lossless gump and land pages unless the packer is asked for another.
pub const LOSSLESS_PAGE_ZSTD_LEVEL: i32 = 19;

/// The builder codec of a lossless gump or land page unless the packer is asked for another.
pub const LOSSLESS_PAGE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL);
