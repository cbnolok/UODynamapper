//! Build-time and runtime support for `tex_art_cc.uddp`.
//!
//! Package layout:
//! - `pages/{page_index}.rgba8888` or `pages/{page_index}.bc7`: atlas page payloads.
//!   When BC7 compression is enabled each page is block-compressed on the CPU before
//!   being stored in the UDDP container. RGBA pages are stored uncompressed (TODO: compress them with zstd instead).
//! - `metadata/pages.bin`: page table with atlas dimensions, per-page occupancy, and
//!   the pixel format used for each page.
//! - `metadata/slots.bin`: sparse slot table with one record per `art_id`, including
//!   empty slots from `artidx.mul`.
//!
//! This module has two layers:
//! - build-time conversion code that reads classic client art, decodes only the
//!   present tiles, and packs them into atlas pages.
//! - runtime loading code that treats the resulting package as a sparse slot table
//!   plus page table, so the renderer can jump directly from `art_id` to page/rect.
//!
//! The key high-level contract is that the slot table is authoritative for lookup,
//! while the page payloads are just backing storage for the rectangles referenced
//! by those slots. Empty records are kept on purpose so classic `art_id` lookups
//! stay O(1) and preserve the original sparse address space.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};

use crate::bc7::{
    bc7_encode_progress_units, encode_for_vram_with_bc7_rdo_lambda_and_progress,
    preferred_bc7_encoder_backend, ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::{
    texture_atlas_packing_mode, AtlasPackingMode, extrude_rgba_rect_edges, merge_unplaced_tiles,
    resolve_packing_axis,
};
use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::package_progress::{
    atlas_payload_finish_message, atlas_payload_progress_message, build_and_write_package,
};
use crate::source_paths::{find_first_dir_matching, source_path_label};
use udd_container::xxh64_virtual_path;
use uocf::classic::art::{ArtMap, ArtSource};

use crate::upscale::{apply_filter_passes, UpscaleFilter};

use udd_assets::tex_art_cc::{
    page_entry_path, TexArtCcPageRecord, TexArtCcSlotRecord, PagePixelFormat, MISSING_PAGE_INDEX,
    MISSING_PAGE_TILE_INDEX, PAGE_MANIFEST_ENTRY_PATH, SLOT_FLAG_LAND, SLOT_FLAG_PRESENT,
    SLOT_FLAG_STATIC, SLOT_MANIFEST_ENTRY_PATH,
};
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtCcBuildSummary {
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

pub struct TexArtCcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub upscale: UpscaleFilter,
    pub upscale_passes: Vec<UpscaleFilter>,
    pub pixel_format: PagePixelFormat,
    pub bc7_rdo_lambda: f32,
}

impl Default for TexArtCcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::JpegXl,
            upscale: UpscaleFilter::default(),
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        }
    }
}

fn packing_mode_repr(mode: AtlasPackingMode) -> u8 {
    match mode {
        AtlasPackingMode::MaximumPacking => 0,
        AtlasPackingMode::Bc7Oriented => 1,
    }
}

fn effective_packing_mode(options: &TexArtCcAtlasOptions) -> AtlasPackingMode {
    texture_atlas_packing_mode(options.pixel_format)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtTileKind {
    Land,
    Static,
}

impl ArtTileKind {
    pub fn slot_flags(self) -> u16 {
        match self {
            Self::Land => SLOT_FLAG_PRESENT | SLOT_FLAG_LAND,
            Self::Static => SLOT_FLAG_PRESENT | SLOT_FLAG_STATIC,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedArtTile {
    pub art_id: u32,
    pub kind: ArtTileKind,
    pub width: u16,
    pub height: u16,
    pub upscale_factor: u16,
    pub upscale_algorithm: u16,
    pub draw_offset_x: i16,
    pub draw_offset_y: i16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PlacedTile {
    pub art_id: u32,
    pub kind: ArtTileKind,
    pub page_tile_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub upscale_factor: u16,
    pub upscale_algorithm: u16,
    pub draw_offset_x: i16,
    pub draw_offset_y: i16,
}

#[derive(Debug, Clone)]
pub struct BuiltPage {
    pub record: TexArtCcPageRecord,
    /// Raw RGBA8888 pixels as produced by the guillotiere packer.
    /// Encoded to the final format (RGBA or BC7) at pack time.
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CASL";
/// Bump version when the binary layout of either manifest changes.
const TEX_ART_CC_METADATA_VERSION: u32 = 6;

pub(crate) fn upscale_algorithm_code(filter: UpscaleFilter) -> u16 {
    match filter {
        UpscaleFilter::None => 0,
        UpscaleFilter::Nearest2x | UpscaleFilter::Nearest3x | UpscaleFilter::Nearest4x => 1,
        UpscaleFilter::Bilinear2x | UpscaleFilter::Bilinear3x | UpscaleFilter::Bilinear4x => 2,
        UpscaleFilter::CatmullRom2x | UpscaleFilter::CatmullRom3x | UpscaleFilter::CatmullRom4x => 3,
        UpscaleFilter::Lanczos3_2x | UpscaleFilter::Lanczos3_3x | UpscaleFilter::Lanczos3_4x => 4,
        UpscaleFilter::SuperSai2x => 5,
        UpscaleFilter::FsrEasu2x | UpscaleFilter::FsrEasu3x | UpscaleFilter::FsrEasu4x => 6,
        UpscaleFilter::FsrEasuRcas2x | UpscaleFilter::FsrEasuRcas3x | UpscaleFilter::FsrEasuRcas4x => 7,
        UpscaleFilter::KLDepixelize2x | UpscaleFilter::KLDepixelize3x | UpscaleFilter::KLDepixelize4x => 8,
        UpscaleFilter::Nedi2x => 9,
        UpscaleFilter::TwoSai2x => 10,
        UpscaleFilter::SuperEagle2x => 11,
        UpscaleFilter::Lq2x | UpscaleFilter::Lq3x | UpscaleFilter::Lq4x => 12,
        UpscaleFilter::Hq2xSimple | UpscaleFilter::Hq3xSimple | UpscaleFilter::Hq4xSimple => 13,
        UpscaleFilter::Hq2xTrue | UpscaleFilter::Hq3xTrue | UpscaleFilter::Hq4xTrue => 14,
        UpscaleFilter::Epx2x | UpscaleFilter::Epx3x | UpscaleFilter::Epx4x => 15,
        UpscaleFilter::Xbr2x | UpscaleFilter::Xbr3x | UpscaleFilter::Xbr4x => 16,
        UpscaleFilter::Mmpx2x | UpscaleFilter::Mmpx4x => 17,
    }
}

fn art_upscale_passes(options: &TexArtCcAtlasOptions) -> Vec<UpscaleFilter> {
    if options.upscale_passes.is_empty() {
        vec![options.upscale]
    } else {
        options.upscale_passes.clone()
    }
}

pub fn convert_art_mul_to_tex_art_cc_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<TexArtCcBuildSummary> {
    convert_art_mul_to_tex_art_cc_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_art_mul_to_tex_art_cc_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<TexArtCcBuildSummary> {
    convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches(
        source_dirs,
        out_file,
        options,
        &ClassicPatchOptions::NONE,
    )
}

pub fn convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TexArtCcAtlasOptions,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<TexArtCcBuildSummary> {
    validate_options(options)?;

    let client_dir = find_first_dir_matching(source_dirs, &[&["artlegacymul.uop"], &["artLegacyMUL.uop"], &["artidx.mul", "art.mul"]])
        .ok_or_else(|| eyre::eyre!(
            "no art sources found in any provided path: expected artLegacyMUL.uop or art.mul/artidx.mul"
        ))?;

    let art_uop_path = ["artlegacymul.uop", "artLegacyMUL.uop"]
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file());
    let has_uop = art_uop_path.is_some();

    if has_uop {
        info!(
            "Converting CC Art from UOP format to {}",
            out_file.display()
        );
    } else {
        info!(
            "Converting CC Art from classic MUL format to {}",
            out_file.display()
        );
    }
    if let Some(path) = &art_uop_path {
        println!("Using CC art source file (UOP): {}", source_path_label(&client_dir, path));
    } else {
        let artidx_path = client_dir.join("artidx.mul");
        let art_path = client_dir.join("art.mul");
        println!("Using CC art index source file: {}", source_path_label(&client_dir, &artidx_path));
        println!("Using CC art source file (MUL): {}", source_path_label(&client_dir, &art_path));
    }

    let mut art_map = ArtMap::load(&client_dir)
        .wrap_err_with(|| format!("load art sources from {}", client_dir.display()))?;
    if !has_uop {
        if let Some(verdata) = load_verdata_if_enabled(source_dirs, patch_options)? {
            art_map = art_map.with_verdata(verdata);
        }
    }

    let art_source = if has_uop {
        ArtSource::CcUop
    } else {
        ArtSource::Mul
    };
    let slot_count = art_map.max_id_for_source(art_source);
    let decoded_tiles = decode_present_tiles(&art_map, art_source, options)?;
    let populated_slot_count = decoded_tiles.len() as u32;

    let (pages, slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(PAGE_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &page_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;

    // Determine the final pixel format and encoding for atlas pages.
    let use_bc7 = options.pixel_format == PagePixelFormat::Bc7;

    let (encoding, pixel_format) = if use_bc7 {
        (
            VramTextureEncoding::Bc7(preferred_bc7_encoder_backend()),
            PagePixelFormat::Bc7,
        )
    } else {
        (
            VramTextureEncoding::Rgba8UnormSrgb,
            PagePixelFormat::Rgba8888,
        )
    };

    let compression = options.compression;

    let bc7_extent = if use_bc7 {
        Some(ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?)
    } else {
        None
    };
    let progress_len = if let Some(extent) = bc7_extent {
        pages.len() as u64 * bc7_encode_progress_units(extent, options.bc7_rdo_lambda) as u64
    } else {
        pages.len() as u64
    };
    let progress_message = atlas_payload_progress_message(
        "CC art atlas pages",
        use_bc7,
        compression,
        options.bc7_rdo_lambda,
    );

    let pb = ProgressBar::new(progress_len);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(progress_message);

    let encoded_pages = if use_bc7 {
        let extent = bc7_extent.expect("BC7 extent is initialized when BC7 output is selected");
        let encoded_pages = pages
            .par_iter()
            .map(|page| {
                let page_path = page_entry_path(page.record.page_index, pixel_format);
                let encoded =
                    encode_for_vram_with_bc7_rdo_lambda_and_progress(
                        &page.pixels,
                        extent,
                        RawImageFormat::Rgba8888,
                        encoding,
                        options.bc7_rdo_lambda,
                        |units| pb.inc(units as u64),
                    )
                        .map_err(|e| {
                            eyre::eyre!("BC7 encode page {}: {e}", page.record.page_index)
                        })?
                        .into_bytes()
                        .to_vec();
                Ok((
                    page_path,
                    encoded,
                    options.atlas_width,
                    options.atlas_height,
                ))
            })
            .collect::<Vec<eyre::Result<(String, Vec<u8>, u32, u32)>>>();

        let mut resolved = Vec::with_capacity(encoded_pages.len());
        for page in encoded_pages {
            resolved.push(page?);
        }
        resolved
    } else {
        pages
            .iter()
            .map(|page| {
                pb.inc(1);
                (
                    page_entry_path(page.record.page_index, pixel_format),
                    crop_rgba_page(
                        &page.pixels,
                        options.atlas_width,
                        page.record.used_width,
                        page.record.used_height,
                    ),
                    page.record.used_width,
                    page.record.used_height,
                )
            })
            .collect()
    };

    for (page_path, encoded, width, height) in encoded_pages {
        package.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression,
            width,
            height,
            virtual_path: Some(page_path),
            path_hash64: None,
            id: None,
            data: encoded,
        })?;
    }
    pb.finish_with_message(atlas_payload_finish_message(
        "CC art atlas pages",
        use_bc7,
        compression,
        options.bc7_rdo_lambda,
    ));

    build_and_write_package(&mut package, out_file)?;

    Ok(TexArtCcBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &TexArtCcAtlasOptions) -> eyre::Result<()> {
    if options.compression == CompressionFlag::None {
        // Technically this was prohibited for CC Art before, but now we allow it if selected.
        // Actually, let's keep the warning/bail if we really want to prevent it.
        // But for now, let's just allow it since the user can select it.
    }
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    Ok(())
}

fn decode_present_tiles(
    art_map: &ArtMap,
    source: ArtSource,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<Vec<DecodedArtTile>> {
    // Decode every occupied art slot up front so the packer can sort by area and
    // feed the atlas allocator largest-first. Classic clients are messy in practice:
    // some slots are structurally present but malformed, so the converter skips
    // those and reports a compact sample instead of aborting the whole package.
    let mut decoded_tiles = Vec::new();
    let mut skipped_tiles = 0u32;
    let mut skipped_land_tiles = 0u32;
    let mut skipped_static_tiles = 0u32;
    let mut skipped_land_samples = Vec::new();
    let mut skipped_static_samples = Vec::new();

    let max_id = art_map.max_id_for_source(source);
    let art_ids = (0..max_id)
        .filter(|&art_id| art_map.has_id_from_source(art_id, source))
        .collect::<Vec<_>>();
    let pb = ProgressBar::new(art_ids.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} extracting CC art tiles ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    enum DecodeOutcome {
        Decoded(DecodedArtTile),
        SkippedLand(String),
        SkippedStatic(String),
    }

    let decode_outcomes = art_ids
        .into_par_iter()
        .map(|art_id| {
            let kind = if art_id < 0x4000 {
                ArtTileKind::Land
            } else {
                ArtTileKind::Static
            };
            let mut scratch_raw = Vec::new();

            let outcome = match kind {
                ArtTileKind::Land => {
                    let mut rgba = [0u8; 44 * 44 * 4];
                    match art_map.decode_land_tile_from_source(
                        art_id,
                        source,
                        &mut scratch_raw,
                        &mut rgba,
                    ) {
                        Ok(()) => {
                            let upscale_passes = art_upscale_passes(options);
                            let (w, h, rgba, upscale_factor, upscale_filter) =
                                apply_filter_passes(44, 44, &rgba, &upscale_passes);
                            DecodeOutcome::Decoded(DecodedArtTile {
                                art_id,
                                kind,
                                width: w as u16,
                                height: h as u16,
                                upscale_factor: upscale_factor as u16,
                                upscale_algorithm: upscale_algorithm_code(upscale_filter),
                                draw_offset_x: 0,
                                draw_offset_y: 0,
                                rgba,
                            })
                        }
                        Err(error) => DecodeOutcome::SkippedLand(format!("{art_id} ({error})")),
                    }
                }
                ArtTileKind::Static => match art_map.decode_static_tile_from_source(
                    art_id,
                    source,
                    &mut scratch_raw,
                ) {
                    Ok((width, height, rgba)) => {
                        let upscale_passes = art_upscale_passes(options);
                        let (w, h, rgba, upscale_factor, upscale_filter) =
                            apply_filter_passes(width as u32, height as u32, &rgba, &upscale_passes);
                        DecodeOutcome::Decoded(DecodedArtTile {
                            art_id,
                            kind,
                            width: w as u16,
                            height: h as u16,
                            upscale_factor: upscale_factor as u16,
                            upscale_algorithm: upscale_algorithm_code(upscale_filter),
                            draw_offset_x: 0,
                            draw_offset_y: 0,
                            rgba,
                        })
                    }
                    Err(error) => DecodeOutcome::SkippedStatic(format!("{art_id} ({error})")),
                },
            };
            pb.inc(1);
            outcome
        })
        .collect::<Vec<_>>();

    for outcome in decode_outcomes {
        match outcome {
            DecodeOutcome::Decoded(tile) => decoded_tiles.push(tile),
            DecodeOutcome::SkippedLand(sample) => {
                skipped_tiles += 1;
                skipped_land_tiles += 1;
                if skipped_land_samples.len() < 8 {
                    skipped_land_samples.push(sample);
                }
            }
            DecodeOutcome::SkippedStatic(sample) => {
                skipped_tiles += 1;
                skipped_static_tiles += 1;
                if skipped_static_samples.len() < 8 {
                    skipped_static_samples.push(sample);
                }
            }
        }
    }
    if skipped_tiles > 0 {
        let land_summary = format_skip_summary(skipped_land_tiles, &skipped_land_samples);
        let static_summary = format_skip_summary(skipped_static_tiles, &skipped_static_samples);
        pb.finish_with_message(format!(
            "CC art tiles extracted (skipped {skipped_tiles} malformed entries; land: {land_summary}; static: {static_summary})"
        ));
    } else {
        pb.finish_with_message("CC art tiles extracted");
    }

    decoded_tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });

    Ok(decoded_tiles)
}

fn format_skip_summary(skipped_count: u32, samples: &[String]) -> String {
    if skipped_count == 0 {
        return "0".to_string();
    }

    if skipped_count as usize > samples.len() {
        format!("{skipped_count} [{}; ...]", samples.join(", "))
    } else {
        format!("{skipped_count} [{}]", samples.join(", "))
    }
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<TexArtCcSlotRecord>)> {
    // Build full sparse metadata up front. Empty slots are kept explicitly so the
    // runtime can answer `art_id -> atlas location` without a side lookup table.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(TexArtCcSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    let total_tiles = remaining.len() as u64;
    remaining.sort_by_key(|tile| tile.art_id);
    let mut page_index = 0u32;
    let pb = ProgressBar::new(total_tiles);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating CC art atlas pages");

    while !remaining.is_empty() {
        pb.set_message(format!("creating CC art atlas page {page_index}"));
        let (page_tiles, leftovers) = take_page_tile_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_tiles, options)?;
        if page.placed_tiles.is_empty() {
            eyre::bail!(
                "could not fit any art tile into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }

        for placed in &page.placed_tiles {
            let slot = slot_records
                .get_mut(placed.art_id as usize)
                .context("placed tile art_id outside slot table")?;
            *slot = TexArtCcSlotRecord {
                art_id: placed.art_id,
                page_index,
                page_tile_index: placed.page_tile_index,
                flags: placed.kind.slot_flags(),
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
                upscale_factor: placed.upscale_factor,
                upscale_algorithm: placed.upscale_algorithm,
                draw_offset_x: placed.draw_offset_x,
                draw_offset_y: placed.draw_offset_y,
            };
        }

        pb.inc(page.placed_tiles.len() as u64);
        pages.push(page);
        remaining = merge_unplaced_tiles(leftovers, unplaced, |tile| tile.art_id);
        page_index += 1;
    }

    pb.finish_with_message(format!("CC art atlas pages created ({page_index} pages)"));

    Ok((pages, slot_records))
}

fn take_page_tile_prefix(
    tiles: Vec<DecodedArtTile>,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<(Vec<DecodedArtTile>, Vec<DecodedArtTile>)> {
    let prefix_len = max_fitting_page_prefix_len(&tiles, options)?;

    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any art tile into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }

    let mut leftovers = tiles;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((selected, leftovers))
}

fn max_fitting_page_prefix_len(
    tiles: &[DecodedArtTile],
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<usize> {
    let mut low = 1usize;
    let mut high = tiles.len();
    let mut best = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        if page_prefix_fits(&tiles[..mid], options)? {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }

    Ok(best)
}

fn page_prefix_fits(tiles: &[DecodedArtTile], options: &TexArtCcAtlasOptions) -> eyre::Result<bool> {
    let mut to_pack = tiles.to_vec();
    sort_tiles_within_page(&mut to_pack, options);

    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));

    for tile in &to_pack {
        let width_axis = resolve_packing_axis(
            tile.width as u32,
            options.atlas_width,
            options.gutter,
            effective_packing_mode(options),
            false,
        );
        let height_axis = resolve_packing_axis(
            tile.height as u32,
            options.atlas_height,
            options.gutter,
            effective_packing_mode(options),
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!(
                "art tile {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                tile.art_id,
                tile.width,
                tile.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        };

        if allocator
            .allocate(size2(width_axis.alloc_extent as i32, height_axis.alloc_extent as i32))
            .is_none()
        {
            return Ok(false);
        }
    }

    Ok(true)
}

fn sort_tiles_within_page(tiles: &mut [DecodedArtTile], options: &TexArtCcAtlasOptions) {
    tiles.sort_by(|left, right| {
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });
}

fn sort_area(tile: &DecodedArtTile, options: &TexArtCcAtlasOptions) -> u32 {
    match effective_packing_mode(options) {
        AtlasPackingMode::MaximumPacking => tile.width as u32 * tile.height as u32,
        AtlasPackingMode::Bc7Oriented => {
            let width_axis = resolve_packing_axis(
                tile.width as u32,
                options.atlas_width,
                options.gutter,
                effective_packing_mode(options),
                false,
            )
            .unwrap_or_else(|| unreachable!("validated before placement"));
            let height_axis = resolve_packing_axis(
                tile.height as u32,
                options.atlas_height,
                options.gutter,
                effective_packing_mode(options),
                false,
            )
            .unwrap_or_else(|| unreachable!("validated before placement"));
            width_axis.alloc_extent * height_axis.alloc_extent
        }
    }
}

fn build_page(
    page_index: u32,
    mut tiles: Vec<DecodedArtTile>,
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedArtTile>)> {
    // Pages are always assembled as full-size RGBA images in memory even when the
    // stored package payload is later cropped or BC7-encoded. That keeps placement,
    // blitting, and runtime atlas coordinates in one consistent page space.
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut placed_tiles = Vec::new();
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let _gutter = i32::from(options.gutter);

    sort_tiles_within_page(&mut tiles, options);

    for tile in tiles {
        let width_axis = resolve_packing_axis(
            tile.width as u32,
            options.atlas_width,
            options.gutter,
            effective_packing_mode(options),
            false,
        );
        let height_axis = resolve_packing_axis(
            tile.height as u32,
            options.atlas_height,
            options.gutter,
            effective_packing_mode(options),
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!(
                "art tile {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                tile.art_id,
                tile.width,
                tile.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        };

        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                tile.width as u32,
                tile.height as u32,
                &tile.rgba,
            )?;
            let filtering_ready = matches!(tile.kind, ArtTileKind::Land);
            if filtering_ready {
                extrude_rgba_rect_edges(
                    &mut pixels,
                    options.atlas_width,
                    options.atlas_height,
                    allocation.rectangle.min.x as u32,
                    allocation.rectangle.min.y as u32,
                    width_axis.alloc_extent,
                    height_axis.alloc_extent,
                    inner_x as u32,
                    inner_y as u32,
                    tile.width as u32,
                    tile.height as u32,
                );
            }

            // Track the furthest written texel, not the allocator rectangle. The
            // later crop step trims only guaranteed-empty space from the right/bottom
            // edges while preserving the logical tile coordinates recorded in metadata.
            if filtering_ready {
                used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
                used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            } else {
                used_width = used_width.max(inner_x as u32 + width_axis.used_extent);
                used_height = used_height.max(inner_y as u32 + height_axis.used_extent);
            }
            placed_tiles.push(PlacedTile {
                art_id: tile.art_id,
                kind: tile.kind,
                page_tile_index: placed_tiles.len() as u16,
                x: inner_x as u16,
                y: inner_y as u16,
                width: tile.width,
                height: tile.height,
                upscale_factor: tile.upscale_factor,
                upscale_algorithm: tile.upscale_algorithm,
                draw_offset_x: tile.draw_offset_x,
                draw_offset_y: tile.draw_offset_y,
            });
        } else {
            leftovers.push(tile);
        }
    }

    Ok((
        BuiltPage {
            record: TexArtCcPageRecord {
                page_index,
                tile_count: placed_tiles.len() as u32,
                used_width,
                used_height,
                // Pixel format is not yet known here; it will be resolved at
                // pack time once the caller decides the encoding. Use Rgba8888
                // as the placeholder — it is updated in serialize_page_manifest.
                pixel_format: PagePixelFormat::Rgba8888,
            },
            pixels,
            placed_tiles,
        },
        leftovers,
    ))
}

fn blit_rgba_tile(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    tile_width: u32,
    tile_height: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let expected_len = tile_width as usize * tile_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for tile {}x{}: expected {}, got {}",
            tile_width,
            tile_height,
            expected_len,
            src.len()
        );
    }

    let dst_stride = dst_width as usize * 4;
    let src_stride = tile_width as usize * 4;
    for row in 0..tile_height as usize {
        let src_start = row * src_stride;
        let dst_start = ((dst_y as usize + row) * dst_stride) + dst_x as usize * 4;
        let dst_end = dst_start + src_stride;
        dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + src_stride]);
    }

    Ok(())
}

pub fn crop_rgba_page(src: &[u8], src_width: u32, crop_width: u32, crop_height: u32) -> Vec<u8> {
    // Only the stored payload is cropped. The atlas still behaves logically as a
    // full `atlas_width x atlas_height` page because manifests keep the slot coords
    // in that original space plus the `used_width/used_height` bounds needed to read
    // the compact payload back.
    let mut cropped = vec![0u8; crop_width as usize * crop_height as usize * 4];
    let src_stride = src_width as usize * 4;
    let dst_stride = crop_width as usize * 4;

    for row in 0..crop_height as usize {
        let src_start = row * src_stride;
        let dst_start = row * dst_stride;
        cropped[dst_start..dst_start + dst_stride]
            .copy_from_slice(&src[src_start..src_start + dst_stride]);
    }

    cropped
}

pub fn serialize_page_manifest(
    pages: &[BuiltPage],
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // The page manifest carries both the logical atlas dimensions and the per-page
    // used rectangle. Readers reconstruct a full page view from those two facts:
    // atlas coordinates stay stable, but package I/O only touches the occupied area.
    let pixel_format = options.pixel_format;
    let mut bytes = Vec::with_capacity(26 + pages.len() * 17);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_ART_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    // 1 byte: page pixel format (0 = RGBA8888, 1 = BC7)
    bytes.push(pixel_format as u8);
    bytes.push(packing_mode_repr(effective_packing_mode(options)));
    bytes.write_u32::<LittleEndian>(pages.len() as u32)?;
    for page in pages {
        bytes.write_u32::<LittleEndian>(page.record.page_index)?;
        bytes.write_u32::<LittleEndian>(page.record.tile_count)?;
        bytes.write_u32::<LittleEndian>(page.record.used_width)?;
        bytes.write_u32::<LittleEndian>(page.record.used_height)?;
    }
    Ok(bytes)
}

pub fn serialize_slot_manifest(
    slots: &[TexArtCcSlotRecord],
    options: &TexArtCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(25 + slots.len() * 28);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_ART_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(packing_mode_repr(effective_packing_mode(options)));
    bytes.write_u32::<LittleEndian>(slots.len() as u32)?;
    for slot in slots {
        bytes.write_u32::<LittleEndian>(slot.art_id)?;
        bytes.write_u32::<LittleEndian>(slot.page_index)?;
        bytes.write_u16::<LittleEndian>(slot.page_tile_index)?;
        bytes.write_u16::<LittleEndian>(slot.flags)?;
        bytes.write_u16::<LittleEndian>(slot.x)?;
        bytes.write_u16::<LittleEndian>(slot.y)?;
        bytes.write_u16::<LittleEndian>(slot.width)?;
        bytes.write_u16::<LittleEndian>(slot.height)?;
        bytes.write_u16::<LittleEndian>(slot.upscale_factor.max(1))?;
        bytes.write_u16::<LittleEndian>(slot.upscale_algorithm)?;
        bytes.write_i16::<LittleEndian>(slot.draw_offset_x)?;
        bytes.write_i16::<LittleEndian>(slot.draw_offset_y)?;
    }
    Ok(bytes)
}

pub fn encode_slot_manifest(
    slots: &[TexArtCcSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &TexArtCcAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::default(),
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(art_id: u32, width: u16, height: u16) -> DecodedArtTile {
        DecodedArtTile {
            art_id,
            kind: ArtTileKind::Static,
            width,
            height,
            upscale_factor: 1,
            upscale_algorithm: 0,
            draw_offset_x: 0,
            draw_offset_y: 0,
            rgba: vec![255; width as usize * height as usize * 4],
        }
    }

    #[test]
    fn filtering_ready_art_page_keeps_extruded_gutters_in_used_bounds() {
        let options = TexArtCcAtlasOptions {
            atlas_width: 8,
            atlas_height: 8,
            gutter: 1,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::None,
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        };

        let mut tile = tile(7, 2, 2);
        tile.kind = ArtTileKind::Land;

        let (page, leftovers) = build_page(0, vec![tile], &options).unwrap();
        assert!(leftovers.is_empty());
        assert_eq!(page.record.used_width, 4);
        assert_eq!(page.record.used_height, 4);

        let placed = &page.placed_tiles[0];
        let gutter_x = placed.x as u32 + placed.width as u32;
        let gutter_y = placed.y as u32;
        let gutter_offset = ((gutter_y * options.atlas_width + gutter_x) * 4) as usize;
        assert_eq!(&page.pixels[gutter_offset..gutter_offset + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn bc7_oriented_art_page_is_block_aligned() {
        let options = TexArtCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::None,
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        };

        let (page, leftovers) = build_page(0, vec![tile(7, 3, 3)], &options).unwrap();
        assert!(leftovers.is_empty());
        assert_eq!(page.placed_tiles.len(), 1);
        let placed = &page.placed_tiles[0];
        assert_eq!(placed.x % 4, 0);
        assert_eq!(placed.y % 4, 0);
        assert_eq!(page.record.used_width % 4, 0);
        assert_eq!(page.record.used_height % 4, 0);
        assert_eq!(placed.width, 3);
        assert_eq!(placed.height, 3);
    }

    #[test]
    fn packed_art_slot_preserves_draw_offset() {
        let options = TexArtCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::None,
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        };
        let mut tile = tile(7, 3, 3);
        tile.draw_offset_x = -5;
        tile.draw_offset_y = 6;

        let (_pages, slots) = pack_tiles_into_pages(vec![tile], 8, &options).unwrap();

        assert_eq!(slots[7].draw_offset_x, -5);
        assert_eq!(slots[7].draw_offset_y, 6);
    }
}
