//! How the lossless RGBA atlas pages of gumps, land and art textures are compressed.
//!
//! `gumps_cc`, `gumps_ec`, `tex_land_cc`, `tex_land_ec`, `tex_art_cc`, `tex_art_ec` and the `tex_aux_ec` textures store
//! their lossless pages as RGBA8888 with plain Zstd 19 (2026-11-06). The measurements against the image codec it
//! replaced are in `docs/agent_notes/uddp_anim_format_requirements.md`.
//!
//! A gump or texture atlas page that decodes slowly as one frame is stored in strips of [`PAGE_STRIP_ROWS`] rows, each
//! an independent Zstd frame, so a reader decodes it on several threads (2026-11-19). [`PAGE_STRIPS_WORTH`] decides which:
//! at 1x that is the Enhanced art and land pages. A page is stored as one LZ4 HC block instead when that is at most
//! [`PAGE_LZ4_HC_MAX_GROWTH_PERCENT`] larger than one Zstd frame (the user, 2026-11-19): LZ4 HC decodes about four
//! times as fast; at 1x only the densest block-compressed pages pass, and no lossless RGBA page. The codec
//! comparisons and the strip measurements are in `docs/agent_notes/atlas_page_compression.md`.

use udd_assets::tex_art_cc::PagePixelFormat;
use udd_container::{CompressionFlag, StripsWorth};
use uot_texture_codecs::bc7::{BC7_BLOCK_BYTES, BC7_BLOCK_EDGE};

/// The Zstd level of lossless gump, land and art pages unless the packer is asked for another.
pub const LOSSLESS_PAGE_ZSTD_LEVEL: i32 = 19;

/// The builder codec of a lossless gump, land or art page unless the packer is asked for another.
pub const LOSSLESS_PAGE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL);

/// Rows of pixels in each independently compressed strip of an atlas page: 16 strips on a 2048-row page. A multiple of
/// 8, so a strip holds whole block rows of BC7 and of 4x4 and 8x8 ASTC, and a decoded strip can be uploaded to its own
/// rows of a block-compressed texture.
pub const PAGE_STRIP_ROWS: u32 = 128;
const PAGE_STRIP_ROW_MULTIPLE: u32 = 8;
const _: () = assert!(PAGE_STRIP_ROWS % PAGE_STRIP_ROW_MULTIPLE == 0 && PAGE_STRIP_ROWS % BC7_BLOCK_EDGE == 0);

const RGBA8888_PIXEL_BYTES: u32 = 4;

const MIB: u32 = 1 << 20;

/// When a page goes in strips: one frame of an Enhanced RGBA page (13–32 MiB raw, 3.6–7 MiB stored) takes 31–76 ms on
/// one thread and strips add 5–7 %; a Classic page (16 MiB raw, 1.3–2.3 MiB stored) takes 10–14 ms and strips add up to
/// 24 %; a 1x BC7 page (8 MiB raw) takes at most 14 ms. Upscaled Enhanced pages pass, BC7 included.
pub const PAGE_STRIPS_WORTH: StripsWorth = StripsWorth {
    min_raw_bytes: 12 * MIB,
    min_frame_bytes: 3 * MIB,
    max_growth_percent: 10,
};

/// The most an LZ4 HC page may add to the page's size in one Zstd frame and still be stored as LZ4 HC (the user,
/// 2026-11-19). Measured at 1x: Enhanced BC7 art pages add 17–32 %, lossless RGBA pages 44–131 %.
pub const PAGE_LZ4_HC_MAX_GROWTH_PERCENT: u32 = 20;

/// `compression` for an atlas page `width` pixels wide stored in `pixel_format`: an explicit Zstd level becomes LZ4 HC
/// within [`PAGE_LZ4_HC_MAX_GROWTH_PERCENT`] of that level, and otherwise the same level, in strips of
/// [`PAGE_STRIP_ROWS`] rows when they pay by [`PAGE_STRIPS_WORTH`]. Other choices are kept as they are.
pub fn atlas_page_compression(compression: CompressionFlag, pixel_format: PagePixelFormat, width: u32) -> CompressionFlag {
    let CompressionFlag::ZstdNoDictLevel(level) = compression else {
        return compression;
    };
    let row_bytes = match pixel_format {
        PagePixelFormat::Rgba8888 => width * RGBA8888_PIXEL_BYTES,
        // One row of blocks covers BC7_BLOCK_EDGE rows of pixels.
        PagePixelFormat::Bc7 => width.div_ceil(BC7_BLOCK_EDGE) * BC7_BLOCK_BYTES as u32 / BC7_BLOCK_EDGE,
    };
    CompressionFlag::ZstdOrLz4HcLevel {
        level,
        strip_bytes: row_bytes * PAGE_STRIP_ROWS,
        strips: PAGE_STRIPS_WORTH,
        lz4_hc_max_growth_percent: PAGE_LZ4_HC_MAX_GROWTH_PERCENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_strips_hold_whole_pixel_and_block_rows() {
        let strip = |format, width| match atlas_page_compression(LOSSLESS_PAGE_COMPRESSION, format, width) {
            CompressionFlag::ZstdOrLz4HcLevel { level, strip_bytes, strips, lz4_hc_max_growth_percent } => {
                assert_eq!(level, LOSSLESS_PAGE_ZSTD_LEVEL);
                assert_eq!(strips, PAGE_STRIPS_WORTH);
                assert_eq!(lz4_hc_max_growth_percent, PAGE_LZ4_HC_MAX_GROWTH_PERCENT);
                strip_bytes
            }
            other => panic!("not in strips: {other:?}"),
        };
        assert_eq!(strip(PagePixelFormat::Rgba8888, 4095), 4095 * 4 * PAGE_STRIP_ROWS);
        // 1024 blocks of 16 bytes per block row, 32 block rows per strip.
        assert_eq!(strip(PagePixelFormat::Bc7, 4096), 1024 * 16 * (PAGE_STRIP_ROWS / 4));
        assert_eq!(strip(PagePixelFormat::Bc7, 4094), strip(PagePixelFormat::Bc7, 4096));
        assert_eq!(atlas_page_compression(CompressionFlag::None, PagePixelFormat::Rgba8888, 64), CompressionFlag::None);
    }
}
