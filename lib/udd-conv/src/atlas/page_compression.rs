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

/// The Zstd level of the BC7 art, land and auxiliary texture pages unless the packer is asked for another. BC7 (with
/// Zstd, RDO off) is every packer's default where the package takes it (the user, 2026-11-22: "at this point in the
/// converter I'd always default to block compressed textures"); Classic land stays lossless
/// (`tex_land_cc::CLASSIC_LAND_LOSSY_REFUSAL`).
pub const BC7_PAGE_ZSTD_LEVEL: i32 = 19;

/// The builder codec of a BC7 art, land or auxiliary texture page unless the packer is asked for another.
pub const BC7_PAGE_COMPRESSION: CompressionFlag = CompressionFlag::ZstdNoDictLevel(BC7_PAGE_ZSTD_LEVEL);

/// The Zstd level of the BC7 mobile animation pages unless the packer is asked for another. Measured on every 16th page
/// of the BC7 packages (2026-11-22): level 19 is 2.9 % (EC) to 3.6 % (CC) smaller than 9 for 15–18 times its
/// compression time (about 5 minutes more for EC), level 7 0.6–0.8 % larger; decoding takes 2.3–4.5 ms a page at
/// every level. With the field split the whole packages are 8.0 % (EC) to 8.8 % (CC) smaller at 19 than plain Zstd 9
/// (`docs/agent_notes/bc7_anim_supercompression.md`; the user, 2026-11-26: "okay for zstd 19 where its compression
/// ratio/decompression time makes sense"). Level 17 keeps 8.3 % (CC) and 7.4 % (EC) of that for about half the extra
/// pack time (sampled on every 8th page), which the user took on 2026-11-26 ("you can keep 17 then, if the extra time
/// is halved for only a little less compression efficiency").
pub const MOBILE_ANIM_PAGE_ZSTD_LEVEL: i32 = 17;

/// How a mobile animation page is coded unless the packer is asked for another way.
pub const MOBILE_ANIM_PAGE_CODING: MobileAnimPageCoding = MobileAnimPageCoding::FieldsZstd;

/// The builder codec of a mobile animation page unless the packer is asked for another: the BC7 field split with Zstd
/// at [`MOBILE_ANIM_PAGE_ZSTD_LEVEL`] (container codec 4), which a reader joins per clip.
pub const MOBILE_ANIM_PAGE_COMPRESSION: CompressionFlag = MOBILE_ANIM_PAGE_CODING.compression(MOBILE_ANIM_PAGE_ZSTD_LEVEL);

/// How the BC7 pages of the mobile animation packages (and their 2x pages) are coded after BC7: the user's choice of
/// 2026-11-26 is the field split with Zstd, LZ4 HC kept as an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MobileAnimPageCoding {
    /// Plain Zstd of the blocks (container codec 1).
    Zstd,
    /// The BC7 field split, then Zstd (container codec 4): about 8 % smaller at level 19, and a clip joins only its
    /// blocks.
    FieldsZstd,
    /// The BC7 field split, then LZ4 HC (container codec 5): 5 % larger than plain Zstd 9, decoding a crowd's clips
    /// about 2.4 times as fast. It takes no level.
    FieldsLz4Hc,
}

impl MobileAnimPageCoding {
    /// The builder codec of a page coded this way, Zstd at `level`.
    pub const fn compression(self, level: i32) -> CompressionFlag {
        match self {
            Self::Zstd => CompressionFlag::ZstdNoDictLevel(level),
            Self::FieldsZstd => CompressionFlag::Bc7FieldsZstdLevel(level),
            Self::FieldsLz4Hc => CompressionFlag::Bc7FieldsLz4Hc,
        }
    }

    /// `compression` coded this way when it is a page's Zstd pass ([`CompressionFlag::ZstdNoDictLevel`]); anything
    /// else is kept.
    pub const fn apply(self, compression: CompressionFlag) -> CompressionFlag {
        match compression {
            CompressionFlag::ZstdNoDictLevel(level) => self.compression(level),
            other => other,
        }
    }
}

/// The builder codec of a mobile animation page (1x or 2x) stored in `pixel_format`: the field split codes only BC7
/// blocks, so an RGBA8 page takes plain Zstd at the same level instead of [`CompressionFlag::Bc7FieldsZstdLevel`],
/// and plain LZ4 HC instead of [`CompressionFlag::Bc7FieldsLz4Hc`].
pub const fn mobile_anim_page_compression(compression: CompressionFlag, pixel_format: PagePixelFormat)
    -> CompressionFlag {
    match (compression, pixel_format) {
        (CompressionFlag::Bc7FieldsZstdLevel(level), PagePixelFormat::Rgba8888) => CompressionFlag::ZstdNoDictLevel(level),
        (CompressionFlag::Bc7FieldsLz4Hc, PagePixelFormat::Rgba8888) => CompressionFlag::Lz4Hc,
        (compression, _) => compression,
    }
}

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

    #[test]
    fn mobile_pages_split_only_bc7_blocks() {
        const LEVEL: i32 = 12;
        assert_eq!(MOBILE_ANIM_PAGE_COMPRESSION, CompressionFlag::Bc7FieldsZstdLevel(MOBILE_ANIM_PAGE_ZSTD_LEVEL));
        for (coding, bc7, rgba) in [
            (MobileAnimPageCoding::Zstd, CompressionFlag::ZstdNoDictLevel(LEVEL), CompressionFlag::ZstdNoDictLevel(LEVEL)),
            (MobileAnimPageCoding::FieldsZstd, CompressionFlag::Bc7FieldsZstdLevel(LEVEL),
                CompressionFlag::ZstdNoDictLevel(LEVEL)),
            (MobileAnimPageCoding::FieldsLz4Hc, CompressionFlag::Bc7FieldsLz4Hc, CompressionFlag::Lz4Hc),
        ] {
            let compression = coding.apply(CompressionFlag::ZstdNoDictLevel(LEVEL));
            assert_eq!(compression, bc7, "{coding:?}");
            assert_eq!(mobile_anim_page_compression(compression, PagePixelFormat::Bc7), bc7, "{coding:?}");
            assert_eq!(mobile_anim_page_compression(compression, PagePixelFormat::Rgba8888), rgba, "{coding:?}");
            // A page without a Zstd pass stays without one.
            assert_eq!(coding.apply(CompressionFlag::None), CompressionFlag::None, "{coding:?}");
        }
    }

    /// Every packer defaults to block-compressed textures with Zstd and no RDO where its package takes them; Classic
    /// land, the gump atlas pages and the paperdoll frames stay lossless. The Kingdom Reborn material textures take
    /// the Enhanced ones' encoding.
    #[test]
    fn packers_default_to_block_compressed_textures_with_zstd() {
        use crate::bc7::NO_BC7_RDO_LAMBDA;
        use crate::single_gumps::default_single_gump_compression;
        use crate::{cc_gumps, ec_gumps, effects_ec, mobile_anim_cc, mobile_anim_ec, paperdoll_ec};
        use crate::{kr_items, tex_art_cc, tex_art_ec, tex_aux_ec, tex_land_cc, tex_land_ec};

        let art_cc = tex_art_cc::TexArtCcAtlasOptions::default();
        let art_ec = tex_art_ec::TexArtEcAtlasOptions::default();
        let land_ec = tex_land_ec::TexLandEcAtlasOptions::default();
        let aux_ec = tex_aux_ec::TexAuxEcPackOptions::default();
        for (package, pixel_format, compression, rdo_lambda) in [
            ("tex_art_cc", art_cc.pixel_format, art_cc.compression, art_cc.bc7_rdo_lambda),
            ("tex_art_ec", art_ec.pixel_format, art_ec.compression, art_ec.bc7_rdo_lambda),
            ("tex_land_ec", land_ec.pixel_format, land_ec.compression, land_ec.bc7_rdo_lambda),
            ("tex_aux_ec", aux_ec.pixel_format, aux_ec.compression, aux_ec.bc7_rdo_lambda),
        ] {
            assert_eq!(pixel_format, PagePixelFormat::Bc7, "{package}");
            assert_eq!(compression, BC7_PAGE_COMPRESSION, "{package}");
            assert_eq!(rdo_lambda, NO_BC7_RDO_LAMBDA, "{package}");
        }
        let mobile_cc = mobile_anim_cc::MobileAnimCcAtlasOptions::default();
        let mobile_ec = mobile_anim_ec::MobileAnimEcAtlasOptions::default();
        for (package, pixel_format, compression, rdo_lambda, lossy) in [
            ("mobile_anim_cc", mobile_cc.pixel_format, mobile_cc.compression, mobile_cc.bc7_rdo_lambda,
                mobile_cc.bc7_lossy),
            ("mobile_anim_ec", mobile_ec.pixel_format, mobile_ec.compression, mobile_ec.bc7_rdo_lambda,
                mobile_ec.bc7_lossy),
        ] {
            assert_eq!(pixel_format, PagePixelFormat::Bc7, "{package}");
            assert_eq!(compression, MOBILE_ANIM_PAGE_COMPRESSION, "{package}");
            assert_eq!(rdo_lambda, NO_BC7_RDO_LAMBDA, "{package}");
            assert_eq!(lossy, crate::bc7_lossy::MOBILE_ANIM_BC7_LOSSY_LEVEL, "{package}");
        }
        let gumps_cc = cc_gumps::CcGumpsOptions::default();
        let gumps_ec = ec_gumps::EcGumpsOptions::default();
        for (package, single, compression, atlas, lossy) in [
            ("gumps_cc", gumps_cc.single_pixel_format, gumps_cc.compression, gumps_cc.atlas_compression,
                cc_gumps::CC_LOSSY_SINGLE_GUMP_FORMAT),
            ("gumps_ec", gumps_ec.single_pixel_format, gumps_ec.compression, gumps_ec.atlas_compression,
                ec_gumps::EC_LOSSY_SINGLE_GUMP_FORMAT),
        ] {
            assert_eq!(single, lossy, "{package}");
            assert_eq!(compression, default_single_gump_compression(lossy), "{package}");
            assert_eq!(atlas, LOSSLESS_PAGE_COMPRESSION, "{package}");
        }
        assert!(effects_ec::EffectsEcPackOptions::default().bc7);

        let land_cc = tex_land_cc::TexLandCcAtlasOptions::default();
        assert_eq!((land_cc.pixel_format, land_cc.compression), (PagePixelFormat::Rgba8888, LOSSLESS_PAGE_COMPRESSION));
        assert_eq!(paperdoll_ec::PaperdollEcPackOptions::default().compression, LOSSLESS_PAGE_COMPRESSION);
        let kr_aux = kr_items::kr_aux_options(&aux_ec);
        assert_eq!((kr_aux.pixel_format, kr_aux.compression, kr_aux.bc7_rdo_lambda),
            (PagePixelFormat::Bc7, BC7_PAGE_COMPRESSION, NO_BC7_RDO_LAMBDA));
    }
}
