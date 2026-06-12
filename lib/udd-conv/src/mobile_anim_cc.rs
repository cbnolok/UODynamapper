//! Build-time support for `mobile_anim_cc.uddp`.
//!
//! This package targets classic `anim*.mul` / `anim*.idx` mobile-animation
//! sources plus Classic `AnimationFrame*.uop` packages when present. The atlas
//! packer uses 4-pixel-aligned content extents so the same metadata remains
//! valid for both RGBA8888 and BC7 page payloads.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{hash_map::Entry, BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use crate::progress::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::classic::anim::{AnimFrame, AnimFrameInfo, AnimMap, MAX_ANIM_FILES};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::bc7::{
    encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed, Bc7ProgressStage,
    Bc7RdoOptions, Bc7StageTimings, preferred_bc7_encoder_backend, ImageExtent, RawImageFormat,
    VramTextureEncoding,
};
use crate::package_progress::{
    build_and_write_package_with_progress, AssetProgressReporter, AssetTaskProgress,
    AssetTaskProgressStage,
};
use crate::rgba_bounds::{count_nonzero_alpha, nonzero_alpha_bounds, RgbaBounds};
use crate::source_paths::{find_first_dir_matching, source_path_label};
use crate::upscale::{apply_upscale_passes_owned, UpscaleFilter, UpscalePass};
use crate::upscale_profile::{UpscaleImageType, UpscaleProfile, UpscaleTarget};
use crate::{extrude_rgba_rect_edges, resolve_packing_axis, AtlasPackingMode};
use udd_assets::mobile_anim_cc::{
    page_entry_path, MobileAnimCcAnimationRecord, MobileAnimCcFrameRecord,
    MobileAnimCcBodyResolveRecord, MobileAnimCcBodyTypeRecord, MobileAnimCcPageRecord,
    ANIMATION_MANIFEST_ENTRY_PATH, BODY_RESOLVE_FLAG_BODYCONV_DEF, BODY_RESOLVE_FLAG_BODY_DEF,
    BODY_RESOLVE_MANIFEST_ENTRY_PATH, BODY_TYPE_MANIFEST_ENTRY_PATH, FRAME_MANIFEST_ENTRY_PATH,
    MISSING_PAGE_FRAME_INDEX, MISSING_PAGE_INDEX, PAGE_MANIFEST_ENTRY_PATH,
};
use udd_assets::tex_art_cc::PagePixelFormat;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 4;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MAPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MAAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MAFR";
const BODY_RESOLVE_MANIFEST_MAGIC: [u8; 4] = *b"MABR";
const BODY_TYPE_MANIFEST_MAGIC: [u8; 4] = *b"MABT";
const MOBILE_ANIM_CC_METADATA_VERSION: u32 = 3;
const PLANNED_SOURCE_CACHE_LIMIT: usize = 256;
const PAGE_PACK_CANDIDATE_INITIAL_CAPACITY_LIMIT: usize = 4096;
const BC7_PROGRESS_FLUSH_UNITS: u64 = 256;
const MOBILE_ANIM_CC_FLAG_UNMAPPED_SOURCE_INDEX: u16 = 1 << 15;
const CLASSIC_ANIMATIONFRAME_FILES: &[&str] = &[
    "AnimationFrame1.uop",
    "AnimationFrame2.uop",
    "AnimationFrame3.uop",
    "AnimationFrame4.uop",
    "AnimationFrame5.uop",
    "AnimationFrame6.uop",
    "animationframe1.uop",
    "animationframe2.uop",
    "animationframe3.uop",
    "animationframe4.uop",
    "animationframe5.uop",
    "animationframe6.uop",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcBuildSummary {
    pub animation_count: u32,
    pub frame_count: u32,
    pub packed_frame_count: u32,
    pub page_count: u32,
    pub used_page_pixel_count: u64,
    pub filled_pixel_count: u64,
    pub empty_pixel_count: u64,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

pub struct MobileAnimCcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub crop_transparent_bounds: bool,
    pub compression: CompressionFlag,
    pub pixel_format: PagePixelFormat,
    pub bc7_rdo_lambda: f32,
    pub bc7_rdo_lookback_blocks: usize,
    pub upscale_passes: Vec<UpscalePass>,
    pub upscale_profile: Option<Arc<UpscaleProfile>>,
}

impl Default for MobileAnimCcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            crop_transparent_bounds: false,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedMobileAnimFrame {
    pub global_frame_index: u32,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PresentAnimationCandidate {
    body_id: u16,
    action_id: u16,
    direction: u8,
    file_index: u8,
    source_index: u32,
    flags: u16,
    frames: PresentAnimationFrames,
}

#[derive(Debug, Clone)]
enum PresentAnimationFrames {
    Mul,
    AnimationFrameUop {
        path: Arc<PathBuf>,
        file_hash: u64,
        frame_metadata: Vec<AnimFrameInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PlannedAnimationSource {
    Mul {
        file_index: u8,
        source_index: u32,
    },
    AnimationFrameUop {
        path: Arc<PathBuf>,
        file_hash: u64,
    },
}

#[derive(Debug, Clone)]
struct PlannedMobileAnimFrame {
    body_id: u16,
    global_frame_index: u32,
    width: u16,
    height: u16,
    source_left: u16,
    source_top: u16,
    source_width: u16,
    source_height: u16,
    source: PlannedAnimationSource,
    source_frame_index: u16,
}

#[derive(Debug, Clone)]
pub struct BuiltMobileAnimPage {
    pub record: MobileAnimCcPageRecord,
    pub pixels: Vec<u8>,
}

struct PackedMobileAnimPages {
    records: Vec<MobileAnimCcPageRecord>,
    stats: MobileAnimPageStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AtlasPageSize {
    width: u32,
    height: u32,
}

impl AtlasPageSize {
    fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug, Clone, Copy)]
struct PagePackCandidate<Key> {
    sort_key: Key,
    width_axis: crate::PackingAxis,
    height_axis: crate::PackingAxis,
    alloc_area: u32,
}

struct DecodedPageFrame {
    frame: DecodedMobileAnimFrame,
    width_axis: crate::PackingAxis,
    height_axis: crate::PackingAxis,
    alloc_area: u32,
}

struct PlannedPageFrame {
    frame: PlannedMobileAnimFrame,
    width_axis: crate::PackingAxis,
    height_axis: crate::PackingAxis,
    alloc_area: u32,
}

struct CachedPlannedAnimation {
    frames: Arc<Vec<AnimFrame>>,
    last_used: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct MobileAnimPageStats {
    used_page_pixel_count: u64,
    filled_pixel_count: u64,
    empty_pixel_count: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct MobileAnimEncodeTimings {
    bc7: Bc7StageTimings,
    register_pages: Duration,
}

impl MobileAnimEncodeTimings {
    fn add_assign(&mut self, other: Self) {
        self.bc7.add_assign(other.bc7);
        self.register_pages += other.register_pages;
    }
}

pub fn convert_anim_mul_to_mobile_anim_cc_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<MobileAnimCcBuildSummary> {
    convert_anim_mul_to_mobile_anim_cc_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_anim_mul_to_mobile_anim_cc_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<MobileAnimCcBuildSummary> {
    convert_anim_mul_to_mobile_anim_cc_uddp_from_sources_with_progress(
        source_dirs,
        out_file,
        options,
        |_| {},
        |_| {},
    )
}

pub fn convert_anim_mul_to_mobile_anim_cc_uddp_from_sources_with_progress(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimCcAtlasOptions,
    payload_progress: impl Fn(AssetTaskProgress) + Sync,
    mut package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<MobileAnimCcBuildSummary> {
    validate_options(options)?;
    let payload_progress_reporter = AssetProgressReporter::new(payload_progress);
    let payload_progress = |progress| payload_progress_reporter.report(progress);
    let total_timer = Instant::now();

    let client_dir = find_first_dir_matching(source_dirs, &[&["anim.idx", "anim.mul"]])
        .ok_or_else(|| eyre::eyre!(
            "no classic animation sources found in any provided path: expected anim.mul/anim.idx"
        ))?;

    info!(
        "Converting CC mobile animations from classic MUL format to {}",
        out_file.display()
    );
    print_classic_animation_source_files(&client_dir);

    let planning_timer = Instant::now();
    let anim_map = AnimMap::load(&client_dir)
        .wrap_err_with(|| format!("load animation sources from {}", client_dir.display()))?;
    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 0,
        total: 1,
    });
    let (mut planned_frames, animation_records, mut frame_records) =
        plan_present_animations(&client_dir, &anim_map)?;
    apply_planned_transparent_trim(
        &mut planned_frames,
        &mut frame_records,
        &anim_map,
        options,
    )?;
    apply_planned_upscale(&mut planned_frames, &mut frame_records, options)?;
    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 1,
        total: 1,
    });
    let packed_frame_count = planned_frames.len() as u32;
    let body_resolve_records = build_body_resolve_records(&client_dir)?;
    let body_type_records = build_body_type_records(&client_dir)?;
    info!(
        "CC mobile animation planning prepared {} frames, {} animations, {} body resolves in {:.3}s",
        packed_frame_count,
        animation_records.len(),
        body_resolve_records.len(),
        planning_timer.elapsed().as_secs_f64()
    );

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    let pack_timer = Instant::now();
    let packed_pages = pack_planned_frames_into_package(
        &mut package,
        planned_frames,
        &mut frame_records,
        &anim_map,
        options,
        &payload_progress,
    )?;
    info!(
        "CC mobile animation atlas and texture payload build wrote {} pages in {:.3}s",
        packed_pages.records.len(),
        pack_timer.elapsed().as_secs_f64()
    );

    let page_manifest = serialize_page_record_manifest(&packed_pages.records, options)?;
    let animation_manifest = serialize_animation_manifest(&animation_records)?;
    let frame_manifest = serialize_frame_manifest(&frame_records)?;
    let body_resolve_manifest = serialize_body_resolve_manifest(&body_resolve_records)?;
    let body_type_manifest = serialize_body_type_manifest(&body_type_records)?;

    for (path, data) in [
        (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice()),
        (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice()),
        (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice()),
        (BODY_RESOLVE_MANIFEST_ENTRY_PATH, body_resolve_manifest.as_slice()),
        (BODY_TYPE_MANIFEST_ENTRY_PATH, body_type_manifest.as_slice()),
    ] {
        package.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data,
        })?;
    }

    let package_timer = Instant::now();
    build_and_write_package_with_progress(&mut package, out_file, &mut package_progress)?;
    info!(
        "CC mobile animation package build and write completed in {:.3}s",
        package_timer.elapsed().as_secs_f64()
    );

    let summary = MobileAnimCcBuildSummary {
        animation_count: animation_records.len() as u32,
        frame_count: frame_records.len() as u32,
        packed_frame_count,
        page_count: packed_pages.records.len() as u32,
        used_page_pixel_count: packed_pages.stats.used_page_pixel_count,
        filled_pixel_count: packed_pages.stats.filled_pixel_count,
        empty_pixel_count: packed_pages.stats.empty_pixel_count,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    };
    info!(
        "CC mobile animation conversion completed in {:.3}s",
        total_timer.elapsed().as_secs_f64()
    );
    Ok(summary)
}

fn summarize_mobile_anim_pages(pages: &[BuiltMobileAnimPage]) -> MobileAnimPageStats {
    let mut stats = MobileAnimPageStats::default();
    for page in pages {
        let filled_pixels = count_nonzero_alpha(&page.pixels);
        stats.add_page_bounds(page.record.used_width, page.record.used_height, filled_pixels);
    }
    stats
}

impl MobileAnimPageStats {
    fn add_page_bounds(&mut self, used_width: u32, used_height: u32, filled_pixels: u64) {
        let used_pixels = used_width as u64 * used_height as u64;
        self.used_page_pixel_count += used_pixels;
        self.filled_pixel_count += filled_pixels;
        self.empty_pixel_count += used_pixels.saturating_sub(filled_pixels);
    }
}

fn print_classic_animation_source_files(client_dir: &Path) {
    for file_index in 0..MAX_ANIM_FILES {
        let mul_name = classic_anim_mul_name(file_index);
        let idx_name = if file_index == 0 {
            "anim.idx".to_string()
        } else {
            format!("anim{}.idx", file_index + 1)
        };
        let idx_path = client_dir.join(idx_name);
        let mul_path = client_dir.join(mul_name);
        if idx_path.is_file() && mul_path.is_file() {
            println!("Using CC animation index source file: {}", source_path_label(client_dir, &idx_path));
            println!("Using CC animation source file (MUL): {}", source_path_label(client_dir, &mul_path));
        }
    }
    for path in discover_classic_animationframe_paths(client_dir) {
        println!("Using CC animation source file (UOP): {}", source_path_label(client_dir, &path));
    }
}

fn build_body_resolve_records(client_dir: &Path) -> eyre::Result<Vec<MobileAnimCcBodyResolveRecord>> {
    let mut records = BTreeMap::<u16, MobileAnimCcBodyResolveRecord>::new();

    let body_def_path = client_dir.join("Body.def");
    if body_def_path.is_file() {
        let body_def = BodyDef::load(&body_def_path)
            .wrap_err_with(|| format!("load {}", body_def_path.display()))?;
        for (&body_id, entry) in body_def.iter() {
            records.insert(body_id, MobileAnimCcBodyResolveRecord {
                body_id,
                resolved_body_id: entry.graphic,
                hue: entry.hue,
                file_index: 0,
                mount_height: 0,
                flags: BODY_RESOLVE_FLAG_BODY_DEF,
            });
        }
    }

    let bodyconv_path = client_dir.join("Bodyconv.def");
    if bodyconv_path.is_file() {
        let bodyconv = BodyConvDef::load(&bodyconv_path)
            .wrap_err_with(|| format!("load {}", bodyconv_path.display()))?;
        for (&body_id, entry) in bodyconv.iter() {
            records
                .entry(body_id)
                .and_modify(|record| {
                    record.resolved_body_id = entry.graphic;
                    record.file_index = entry.file_index;
                    record.mount_height = entry.mount_height;
                    record.flags |= BODY_RESOLVE_FLAG_BODYCONV_DEF;
                })
                .or_insert(MobileAnimCcBodyResolveRecord {
                    body_id,
                    resolved_body_id: entry.graphic,
                    hue: 0,
                    file_index: entry.file_index,
                    mount_height: entry.mount_height,
                    flags: BODY_RESOLVE_FLAG_BODYCONV_DEF,
                });
        }
    }

    Ok(records.into_values().collect())
}

fn build_body_type_records(client_dir: &Path) -> eyre::Result<Vec<MobileAnimCcBodyTypeRecord>> {
    let path = client_dir.join("mobtypes.txt");
    if !path.is_file() {
        return Ok(Vec::new());
    }

    parse_mobtypes_txt(&fs::read_to_string(&path).wrap_err_with(|| format!("read {}", path.display()))?)
}

fn parse_mobtypes_txt(text: &str) -> eyre::Result<Vec<MobileAnimCcBodyTypeRecord>> {
    let mut records = BTreeMap::<u16, MobileAnimCcBodyTypeRecord>::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.as_bytes()[0].is_ascii_digit() {
            continue;
        }
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(body) = parts.next() else { continue; };
        let Some(type_name) = parts.next() else { continue; };
        let Some(flags) = parts.next() else { continue; };

        let Ok(body_id) = body.parse::<u16>() else { continue; };
        let Some(group_type) = mob_type_group(type_name) else { continue; };
        let flags = u32::from_str_radix(flags.trim_start_matches("0x").trim_start_matches("0X"), 16)
            .unwrap_or(0);

        records.insert(body_id, MobileAnimCcBodyTypeRecord {
            body_id,
            group_type,
            flags: 0x8000_0000 | flags,
        });
    }
    Ok(records.into_values().collect())
}

fn mob_type_group(type_name: &str) -> Option<u8> {
    match type_name.to_ascii_lowercase().as_str() {
        "monster" => Some(0),
        "sea_monster" => Some(1),
        "animal" => Some(2),
        "human" => Some(3),
        "equipment" => Some(4),
        _ => None,
    }
}

fn validate_options(options: &MobileAnimCcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("mobile animation atlas dimensions must be non-zero");
    }
    Ok(())
}

fn encode_and_add_mobile_anim_page_chunk(
    package: &mut UddpBuilder,
    pages: &mut Vec<BuiltMobileAnimPage>,
    options: &MobileAnimCcAtlasOptions,
    payload_progress: &(dyn Fn(AssetTaskProgress) + Sync),
    payload_stage: AssetTaskProgressStage,
    payload_completed: &AtomicU64,
    rdo_payload_completed: &AtomicU64,
    payload_total: u64,
) -> eyre::Result<MobileAnimEncodeTimings> {
    let chunk_frames = pages
        .iter()
        .map(|page| page.record.frame_count as u64)
        .sum::<u64>();
    let chunk_bc7_blocks = if options.pixel_format == PagePixelFormat::Bc7 {
        mobile_anim_chunk_bc7_blocks(pages)
    } else {
        0
    };
    if options.pixel_format != PagePixelFormat::Bc7 {
        for page in pages.drain(..) {
            package.add_owned_file(AddOwnedFileRequest {
                data_type: DataType::Texture as u8,
                compression: options.compression,
                width: page.record.used_width,
                height: page.record.used_height,
                virtual_path: Some(page_entry_path(page.record.page_index, PagePixelFormat::Rgba8888)),
                path_hash64: None,
                id: None,
                data: page.pixels,
            })?;
        }
        let completed = payload_completed
            .fetch_add(chunk_frames, Ordering::Relaxed)
            .saturating_add(chunk_frames)
            .min(payload_total);
        payload_progress(AssetTaskProgress {
            stage: payload_stage,
            completed,
            total: payload_total,
        });
        return Ok(MobileAnimEncodeTimings::default());
    }

    let encode_units_completed = AtomicU64::new(0);
    let rdo_units_completed = AtomicU64::new(0);
    let rdo_enabled =
        options.bc7_rdo_lambda.is_finite() && options.bc7_rdo_lambda > f32::EPSILON;
    let report_bc7_progress = |stage: Bc7ProgressStage, units: u64| {
        let (asset_stage, stage_units, stage_completed) = match stage {
            Bc7ProgressStage::Encode => (
                AssetTaskProgressStage::EncodingBc7,
                encode_units_completed.fetch_add(units, Ordering::Relaxed).saturating_add(units),
                payload_completed.load(Ordering::Relaxed),
            ),
            Bc7ProgressStage::Rdo => (
                AssetTaskProgressStage::ApplyingRdo,
                rdo_units_completed.fetch_add(units, Ordering::Relaxed).saturating_add(units),
                rdo_payload_completed.load(Ordering::Relaxed),
            ),
        };
        let chunk_progress = chunk_frames
            .saturating_mul(stage_units.min(chunk_bc7_blocks))
            .checked_div(chunk_bc7_blocks)
            .unwrap_or(chunk_frames);
        payload_progress(AssetTaskProgress {
            stage: asset_stage,
            completed: stage_completed.saturating_add(chunk_progress).min(payload_total),
            total: payload_total,
        });
    };
    let (encoded_pages, bc7_timings) = encode_mobile_anim_page_chunk(
        pages,
        options,
        None,
        if options.pixel_format == PagePixelFormat::Bc7 {
            Some(&report_bc7_progress)
        } else {
            None
        },
    )?;
    let register_timer = Instant::now();
    for (page_path, stored_page, width, height) in encoded_pages {
        package.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width,
            height,
            virtual_path: Some(page_path),
            path_hash64: None,
            id: None,
            data: stored_page,
        })?;
    }
    let register_pages = register_timer.elapsed();
    pages.clear();
    let completed = payload_completed
        .fetch_add(chunk_frames, Ordering::Relaxed)
        .saturating_add(chunk_frames)
        .min(payload_total);
    if rdo_enabled {
        let completed = rdo_payload_completed
            .fetch_add(chunk_frames, Ordering::Relaxed)
            .saturating_add(chunk_frames)
            .min(payload_total);
        payload_progress(AssetTaskProgress {
            stage: AssetTaskProgressStage::ApplyingRdo,
            completed,
            total: payload_total,
        });
    } else {
        payload_progress(AssetTaskProgress {
            stage: AssetTaskProgressStage::EncodingBc7,
            completed,
            total: payload_total,
        });
    }
    Ok(MobileAnimEncodeTimings {
        bc7: bc7_timings,
        register_pages,
    })
}

fn encode_mobile_anim_page_chunk(
    pages: &[BuiltMobileAnimPage],
    options: &MobileAnimCcAtlasOptions,
    pb: Option<&ProgressBar>,
    progress: Option<&(dyn Fn(Bc7ProgressStage, u64) + Sync)>,
) -> eyre::Result<(Vec<(String, Vec<u8>, u32, u32)>, Bc7StageTimings)> {
    debug_assert!(options.pixel_format == PagePixelFormat::Bc7);
    let encoding = VramTextureEncoding::Bc7(preferred_bc7_encoder_backend());
    let rdo_options = Bc7RdoOptions::sparse_atlas(
        options.bc7_rdo_lambda,
        options.bc7_rdo_lookback_blocks,
    );
    let pending_progress = AtomicU64::new(0);
    let mut encoded_pages = Vec::with_capacity(pages.len());
    let mut bc7_timings = Bc7StageTimings::default();
    for page in pages {
        let extent = ImageExtent::new(page.record.used_width, page.record.used_height)
            .map_err(|e| eyre::eyre!("{e}"))?;
        let (encoded, timings) = encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed(
            &page.pixels,
            extent,
            RawImageFormat::Rgba8888,
            encoding,
            &rdo_options,
            |stage, units| {
                let units = units as u64;
                add_bc7_progress(pb, &pending_progress, units);
                if let Some(progress) = progress {
                    progress(stage, units);
                }
            },
        )
        .map_err(|e| {
            eyre::eyre!("BC7 encode mobile animation page {}: {e}", page.record.page_index)
        })?;
        bc7_timings.add_assign(timings);
        let encoded = encoded
            .into_bytes()
            .to_vec();
        encoded_pages.push((
            page_entry_path(page.record.page_index, PagePixelFormat::Bc7),
            encoded,
            page.record.used_width,
            page.record.used_height,
        ));
    }
    flush_bc7_progress(pb, &pending_progress);
    Ok((encoded_pages, bc7_timings))
}

fn mobile_anim_chunk_bc7_blocks(pages: &[BuiltMobileAnimPage]) -> u64 {
    pages
        .iter()
        .map(|page| {
            let blocks_wide = page.record.used_width.div_ceil(crate::BC7_BLOCK_DIM);
            let blocks_high = page.record.used_height.div_ceil(crate::BC7_BLOCK_DIM);
            blocks_wide as u64 * blocks_high as u64
        })
        .sum()
}

fn add_bc7_progress(pb: Option<&ProgressBar>, pending: &AtomicU64, units: u64) {
    let Some(pb) = pb else {
        return;
    };
    let pending_units = pending.fetch_add(units, Ordering::Relaxed) + units;
    if pending_units >= BC7_PROGRESS_FLUSH_UNITS {
        let flush_units = pending.swap(0, Ordering::AcqRel);
        if flush_units != 0 {
            pb.inc(flush_units);
        }
    }
}

fn flush_bc7_progress(pb: Option<&ProgressBar>, pending: &AtomicU64) {
    if let Some(pb) = pb {
        let flush_units = pending.swap(0, Ordering::AcqRel);
        if flush_units != 0 {
            pb.inc(flush_units);
        }
    }
}

fn plan_present_animations(
    client_dir: &Path,
    anim_map: &AnimMap,
) -> eyre::Result<(Vec<PlannedMobileAnimFrame>, Vec<MobileAnimCcAnimationRecord>, Vec<MobileAnimCcFrameRecord>)> {
    let mut planned_frames = Vec::new();
    let mut animation_records = Vec::new();
    let mut frame_records = Vec::new();

    let mut candidates = collect_present_animation_candidates(anim_map);
    let animationframe_paths = discover_classic_animationframe_paths(client_dir);
    let animationframe_candidates =
        decode_classic_animationframe_packages(&animationframe_paths)?;
    candidates.extend(animationframe_candidates);
    let source_summary = format_candidate_source_summary(&candidates);
    println!(
        "Classic mobile animation candidates: {}",
        source_summary
    );

    let pb = ProgressBar::new(candidates.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("planning mobile animation candidates");

    let mut active_source = String::new();
    for candidate in candidates {
        pb.inc(1);
        let source_label = candidate_source_label(&candidate);
        if source_label != active_source {
            active_source = source_label;
            pb.set_message(format!("planning {active_source}"));
        }
        let Ok(frames) = decode_candidate_animation_frame_metadata(&candidate, anim_map) else {
            continue;
        };
        if frames.is_empty() || frames.len() > u16::MAX as usize {
            continue;
        }
        let frame_count = frames.len() as u16;
        let source = planned_animation_source(&candidate);

        let animation_index = animation_records.len() as u32;
        let frame_start = frame_records.len() as u32;
        for (frame_index, frame) in frames.iter().enumerate() {
            let global_frame_index = frame_records.len() as u32;
            frame_records.push(empty_frame_record(
                animation_index,
                frame_index as u16,
                frame,
            ));
            if frame.width != 0 && frame.height != 0 {
                planned_frames.push(PlannedMobileAnimFrame {
                    body_id: candidate.body_id,
                    global_frame_index,
                    width: frame.width,
                    height: frame.height,
                    source_left: 0,
                    source_top: 0,
                    source_width: frame.width,
                    source_height: frame.height,
                    source: source.clone(),
                    source_frame_index: frame_index as u16,
                });
            }
        }
        animation_records.push(MobileAnimCcAnimationRecord {
            body_id: candidate.body_id,
            action_id: candidate.action_id,
            direction: candidate.direction,
            file_index: candidate.file_index,
            source_index: candidate.source_index,
            frame_start,
            frame_count,
            flags: candidate.flags,
        });
    }
    pb.finish_with_message(format!("Classic mobile animations planned from {source_summary}"));

    Ok((planned_frames, animation_records, frame_records))
}

fn planned_animation_source(candidate: &PresentAnimationCandidate) -> PlannedAnimationSource {
    match &candidate.frames {
        PresentAnimationFrames::Mul => PlannedAnimationSource::Mul {
            file_index: candidate.file_index,
            source_index: candidate.source_index,
        },
        PresentAnimationFrames::AnimationFrameUop { path, file_hash, .. } => {
            PlannedAnimationSource::AnimationFrameUop {
                path: Arc::clone(path),
                file_hash: *file_hash,
            }
        }
    }
}

fn apply_planned_upscale(
    planned_frames: &mut [PlannedMobileAnimFrame],
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<()> {
    if options.upscale_passes.is_empty() && options.upscale_profile.is_none() {
        return Ok(());
    }

    for frame in planned_frames {
        let scale = mobile_upscale_passes_for(
            options,
            u32::from(frame.body_id),
            u32::from(frame.source_frame_index),
        )
        .iter()
        .copied()
        .fold(1u32, |scale, pass| scale.saturating_mul(pass.scale_factor()));
        if scale == 1 {
            continue;
        }
        frame.width = scale_u16(frame.width, scale, "mobile animation frame width")?;
        frame.height = scale_u16(frame.height, scale, "mobile animation frame height")?;
        if let Some(record) = frame_records.get_mut(frame.global_frame_index as usize) {
            record.width = scale_u16(record.width, scale, "mobile animation frame record width")?;
            record.height = scale_u16(record.height, scale, "mobile animation frame record height")?;
            record.center_x = scale_i16(record.center_x, scale, "mobile animation frame center_x")?;
            record.center_y = scale_i16(record.center_y, scale, "mobile animation frame center_y")?;
        }
    }
    Ok(())
}

fn apply_planned_transparent_trim(
    planned_frames: &mut [PlannedMobileAnimFrame],
    frame_records: &mut [MobileAnimCcFrameRecord],
    anim_map: &AnimMap,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<()> {
    if !options.crop_transparent_bounds {
        return Ok(());
    }

    let mut animationframe_package_cache = HashMap::<Arc<PathBuf>, UopPackage>::new();
    let mut decoded_source_cache = HashMap::<PlannedAnimationSource, CachedPlannedAnimation>::new();
    let mut decoded_source_use_tick = 0u64;

    for frame in planned_frames {
        let decoded_frames = cached_decode_planned_animation_source(
            &frame.source,
            anim_map,
            &mut animationframe_package_cache,
            &mut decoded_source_cache,
            &mut decoded_source_use_tick,
        )?;
        let Some(decoded_frame) = decoded_frames.get(frame.source_frame_index as usize) else {
            eyre::bail!(
                "planned mobile animation frame {} missing source frame {} while trimming",
                frame.global_frame_index,
                frame.source_frame_index
            );
        };
        if decoded_frame.width != frame.width || decoded_frame.height != frame.height {
            eyre::bail!(
                "planned mobile animation frame {} changed dimensions while trimming: planned {}x{}, decoded {}x{}",
                frame.global_frame_index,
                frame.width,
                frame.height,
                decoded_frame.width,
                decoded_frame.height
            );
        }
        let Some(bounds) = transparent_bounds(decoded_frame.width, decoded_frame.height, &decoded_frame.data)? else {
            continue;
        };
        if bounds.left == 0
            && bounds.top == 0
            && bounds.right == usize::from(frame.width)
            && bounds.bottom == usize::from(frame.height)
        {
            continue;
        }

        let record = frame_records
            .get_mut(frame.global_frame_index as usize)
            .context("trimmed mobile animation frame outside frame table")?;
        frame.source_left = bounds.left as u16;
        frame.source_top = bounds.top as u16;
        frame.width = (bounds.right - bounds.left) as u16;
        frame.height = (bounds.bottom - bounds.top) as u16;
        frame.source_width = frame.width;
        frame.source_height = frame.height;
        record.width = frame.width;
        record.height = frame.height;
        record.center_x = adjusted_frame_center(
            record.center_x,
            bounds.left as u32,
            "mobile animation frame center_x",
        )?;
        record.center_y = adjusted_frame_center(
            record.center_y,
            bounds.top as u32,
            "mobile animation frame center_y",
        )?;
    }

    Ok(())
}

fn has_effective_upscale(passes: &[UpscalePass]) -> bool {
    passes
        .iter()
        .any(|pass| pass.filter().is_none_or(|filter| !matches!(filter, UpscaleFilter::None)))
}

fn mobile_upscale_passes_for(
    options: &MobileAnimCcAtlasOptions,
    body_id: u32,
    frame_id: u32,
) -> Vec<UpscalePass> {
    options
        .upscale_profile
        .as_ref()
        .map(|profile| {
            profile.passes_for(
                UpscaleTarget::with_family(UpscaleImageType::CcMobileAnimationFrames, body_id, frame_id),
                &options.upscale_passes,
            )
        })
        .unwrap_or_else(|| options.upscale_passes.clone())
}

fn scale_u16(value: u16, scale: u32, label: &str) -> eyre::Result<u16> {
    let scaled = u32::from(value).saturating_mul(scale);
    if scaled > u16::MAX as u32 {
        eyre::bail!("{label} exceeds u16 after upscaling: {scaled}");
    }
    Ok(scaled as u16)
}

fn scale_i16(value: i16, scale: u32, label: &str) -> eyre::Result<i16> {
    let scaled = i32::from(value).saturating_mul(scale as i32);
    if scaled < i16::MIN as i32 || scaled > i16::MAX as i32 {
        eyre::bail!("{label} exceeds i16 after upscaling: {scaled}");
    }
    Ok(scaled as i16)
}

fn decode_candidate_animation_frame_metadata<'a>(
    candidate: &'a PresentAnimationCandidate,
    anim_map: &AnimMap,
) -> eyre::Result<Cow<'a, [AnimFrameInfo]>> {
    match &candidate.frames {
        PresentAnimationFrames::Mul => {
            anim_map
                .decode_animation_index_metadata(candidate.file_index, candidate.source_index)
                .map(Cow::Owned)
        }
        PresentAnimationFrames::AnimationFrameUop { frame_metadata, .. } => {
            Ok(Cow::Borrowed(frame_metadata))
        }
    }
}

fn decode_planned_animation_source(
    source: &PlannedAnimationSource,
    anim_map: &AnimMap,
    animationframe_packages: &mut HashMap<Arc<PathBuf>, UopPackage>,
) -> eyre::Result<Vec<AnimFrame>> {
    match source {
        PlannedAnimationSource::Mul { file_index, source_index } => {
            anim_map.decode_animation_index(*file_index, *source_index)
        }
        PlannedAnimationSource::AnimationFrameUop { path, file_hash } => {
            let package = match animationframe_packages.entry(Arc::clone(path)) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let package = UopPackage::load_with_mode(path.as_ref(), LoadMode::Lazy)
                        .wrap_err_with(|| format!("load {}", path.display()))?;
                    entry.insert(package)
                }
            };
            decode_classic_animationframe_uop_frames_from_package(package, path, *file_hash)
        }
    }
}

fn cached_decode_planned_animation_source(
    source: &PlannedAnimationSource,
    anim_map: &AnimMap,
    animationframe_packages: &mut HashMap<Arc<PathBuf>, UopPackage>,
    cache: &mut HashMap<PlannedAnimationSource, CachedPlannedAnimation>,
    use_tick: &mut u64,
) -> eyre::Result<Arc<Vec<AnimFrame>>> {
    *use_tick = use_tick.saturating_add(1);
    if let Some(entry) = cache.get_mut(source) {
        entry.last_used = *use_tick;
        return Ok(Arc::clone(&entry.frames));
    }

    let decoded = Arc::new(decode_planned_animation_source(source, anim_map, animationframe_packages)?);
    cache.insert(source.clone(), CachedPlannedAnimation {
        frames: Arc::clone(&decoded),
        last_used: *use_tick,
    });
    while cache.len() > PLANNED_SOURCE_CACHE_LIMIT {
        let Some(oldest) = cache
            .iter()
            .filter(|(key, _)| *key != source)
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest);
    }

    Ok(decoded)
}

fn decode_classic_animationframe_uop_frames_from_package(
    package: &mut UopPackage,
    path: &Path,
    file_hash: u64,
) -> eyre::Result<Vec<AnimFrame>> {
    let data = package
        .unpack_file_by_hash_cached(file_hash)?
        .ok_or_else(|| eyre::eyre!("Classic AnimationFrame payload {file_hash:016x} not found in {}", path.display()))?;
    let animation = AnimationFrameCc::parse(&data)?;
    Ok(animation.frames)
}

fn candidate_source_label(candidate: &PresentAnimationCandidate) -> String {
    match &candidate.frames {
        PresentAnimationFrames::Mul => classic_anim_mul_name(candidate.file_index).to_string(),
        PresentAnimationFrames::AnimationFrameUop { .. } => format!(
            "AnimationFrame{}.uop anim_id {}",
            candidate.file_index + 1,
            candidate.source_index
        ),
    }
}

fn candidate_source_summary_label(candidate: &PresentAnimationCandidate) -> String {
    match &candidate.frames {
        PresentAnimationFrames::Mul => classic_anim_mul_name(candidate.file_index).to_string(),
        PresentAnimationFrames::AnimationFrameUop { .. } => {
            format!("AnimationFrame{}.uop", candidate.file_index + 1)
        }
    }
}

fn format_candidate_source_summary(candidates: &[PresentAnimationCandidate]) -> String {
    let mut counts = BTreeMap::<String, usize>::new();
    for candidate in candidates {
        *counts
            .entry(candidate_source_summary_label(candidate))
            .or_insert(0) += 1;
    }
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .into_iter()
        .map(|(source, count)| format!("{source}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn classic_anim_mul_name(file_index: u8) -> &'static str {
    match file_index {
        0 => "anim.mul",
        1 => "anim2.mul",
        2 => "anim3.mul",
        3 => "anim4.mul",
        4 => "anim5.mul",
        5 => "anim6.mul",
        _ => "anim*.mul",
    }
}

fn collect_present_animation_candidates(anim_map: &AnimMap) -> Vec<PresentAnimationCandidate> {
    let mut candidates = Vec::new();
    for file_index in 0..MAX_ANIM_FILES {
        let Some(source_index_count) = anim_map.source_index_count(file_index) else {
            continue;
        };
        for source_index in 0..source_index_count {
            let source_index = source_index as u32;
            if !anim_map.has_anim(file_index, source_index) {
                continue;
            }
            let (body_id, action_id, direction, flags) =
                animation_identity_from_source_index(file_index, source_index);
            candidates.push(PresentAnimationCandidate {
                body_id,
                action_id,
                direction,
                file_index,
                source_index,
                flags,
                frames: PresentAnimationFrames::Mul,
            });
        }
    }
    candidates
}

fn discover_classic_animationframe_paths(client_dir: &Path) -> Vec<PathBuf> {
    CLASSIC_ANIMATIONFRAME_FILES
        .iter()
        .map(|name| client_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn decode_classic_animationframe_packages(
    paths: &[PathBuf],
) -> eyre::Result<Vec<PresentAnimationCandidate>> {
    let mut candidates = Vec::new();
    for path in paths {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy)
            .wrap_err_with(|| format!("load {}", path.display()))?;
        let file_index = classic_animationframe_uop_index(path)? - 1;
        let mut decoded = decode_classic_animationframe_package(&package, file_index, path)?;
        candidates.append(&mut decoded);
    }
    Ok(candidates)
}

fn decode_classic_animationframe_package(
    package: &UopPackage,
    file_index: u8,
    path: &Path,
) -> eyre::Result<Vec<PresentAnimationCandidate>> {
    let file_hashes = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.filename_hash())
        .collect::<Vec<_>>();
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("AnimationFrame*.uop");
    let source_path = Arc::new(path.to_path_buf());
    let pb = ProgressBar::new(file_hashes.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("extracting {file_name}"));

    let parsed = file_hashes
        .par_iter()
        .map_init(
            || package.open_payload_reader(),
            |reader, &file_hash| {
                pb.inc(1);
                let Ok(reader) = reader.as_mut() else {
                    return None;
                };
                let Ok(Some(data)) = package.unpack_file_by_hash_from_reader(file_hash, reader) else {
                    return None;
                };
                let Ok(animation) = AnimationFrameCc::parse_metadata(&data) else {
                    return None;
                };
                if animation.frames.is_empty() || animation.frames.len() > u16::MAX as usize {
                    return None;
                }
                let (body_id, action_id, direction) =
                    animation_layout_from_source_index(file_index, animation.anim_id)?;
                Some(PresentAnimationCandidate {
                    body_id,
                    action_id,
                    direction,
                    file_index,
                    source_index: animation.anim_id,
                    flags: 0,
                    frames: PresentAnimationFrames::AnimationFrameUop {
                        path: Arc::clone(&source_path),
                        file_hash,
                        frame_metadata: animation.frames,
                    },
                })
            },
        )
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for candidate in parsed.into_iter().flatten() {
        candidates.push(candidate);
    }
    pb.finish_with_message(format!("{file_name} extracted"));
    Ok(candidates)
}

fn classic_animationframe_uop_index(path: &Path) -> eyre::Result<u8> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        eyre::bail!("invalid Classic AnimationFrame UOP path: {}", path.display());
    };
    let lower = file_name.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("animationframe")
        .and_then(|value| value.strip_suffix(".uop"))
    else {
        eyre::bail!("invalid Classic AnimationFrame UOP name: {file_name}");
    };
    let index = rest
        .parse::<u8>()
        .wrap_err_with(|| format!("invalid Classic AnimationFrame UOP index in {file_name}"))?;
    if !(1..=MAX_ANIM_FILES).contains(&index) {
        eyre::bail!("unsupported Classic AnimationFrame UOP index in {file_name}");
    }
    Ok(index)
}

fn empty_frame_record(
    animation_index: u32,
    frame_index: u16,
    frame: &AnimFrameInfo,
) -> MobileAnimCcFrameRecord {
    MobileAnimCcFrameRecord {
        animation_index,
        frame_index,
        page_index: MISSING_PAGE_INDEX,
        page_frame_index: MISSING_PAGE_FRAME_INDEX,
        x: 0,
        y: 0,
        width: frame.width,
        height: frame.height,
        center_x: frame.center_x,
        center_y: frame.center_y,
    }
}

pub fn animation_source_index(file_index: u8, body_id: u16, action_id: u16, direction: u8) -> u32 {
    let body = body_id as u32;
    let mut index = match file_index {
        0 => {
            if body < 200 {
                body * 110
            } else if body < 400 {
                22000 + ((body - 200) * 65)
            } else {
                35000 + ((body - 400) * 175)
            }
        }
        1 => {
            if body < 200 {
                body * 110
            } else {
                22000 + ((body - 200) * 65)
            }
        }
        2 => {
            if body < 300 {
                body * 65
            } else if body < 400 {
                33000 + ((body - 300) * 110)
            } else {
                35000 + ((body - 400) * 175)
            }
        }
        _ => {
            if body < 200 && !(file_index == 4 && body == 34) {
                body * 110
            } else if body < 400 {
                22000 + (body.saturating_sub(200) * 65)
            } else {
                35000 + ((body - 400) * 175)
            }
        }
    };

    index += action_id as u32 * 5;
    index += direction.min(4) as u32;
    index
}

fn animation_layout_from_source_index(
    file_index: u8,
    source_index: u32,
) -> Option<(u16, u16, u8)> {
    if file_index == 4 {
        if let Some(layout) = animation_layout_from_group(source_index, 34, 35, 22000, 22) {
            return Some(layout);
        }
    }

    match file_index {
        0 => animation_layout_from_groups(source_index, &[
            (0, 200, 0, 22),
            (200, 400, 22000, 13),
            (400, u16::MAX, 35000, 35),
        ]),
        1 => animation_layout_from_groups(source_index, &[
            (0, 200, 0, 22),
            (200, u16::MAX, 22000, 13),
        ]),
        2 => animation_layout_from_groups(source_index, &[
            (0, 300, 0, 13),
            (300, 400, 33000, 22),
            (400, u16::MAX, 35000, 35),
        ]),
        _ => animation_layout_from_groups(source_index, &[
            (0, 200, 0, 22),
            (200, 400, 22000, 13),
            (400, u16::MAX, 35000, 35),
        ]),
    }
}

fn animation_identity_from_source_index(
    file_index: u8,
    source_index: u32,
) -> (u16, u16, u8, u16) {
    if let Some((body_id, action_id, direction)) =
        animation_layout_from_source_index(file_index, source_index)
    {
        return (body_id, action_id, direction, 0);
    }

    (
        (source_index & 0xFFFF) as u16,
        (source_index >> 16) as u16,
        file_index,
        MOBILE_ANIM_CC_FLAG_UNMAPPED_SOURCE_INDEX,
    )
}

fn animation_layout_from_groups(
    source_index: u32,
    groups: &[(u16, u16, u32, u16)],
) -> Option<(u16, u16, u8)> {
    for &(body_start, body_end, source_start, action_count) in groups {
        if let Some(layout) = animation_layout_from_group(
            source_index,
            body_start,
            body_end,
            source_start,
            action_count,
        ) {
            return Some(layout);
        }
    }
    None
}

fn animation_layout_from_group(
    source_index: u32,
    body_start: u16,
    body_end: u16,
    source_start: u32,
    action_count: u16,
) -> Option<(u16, u16, u8)> {
    if source_index < source_start || body_end <= body_start {
        return None;
    }
    let stride = action_count as u32 * 5;
    let body_count = body_end as u32 - body_start as u32;
    let rel = source_index - source_start;
    if rel >= body_count * stride {
        return None;
    }
    let body = body_start as u32 + rel / stride;
    let within_body = rel % stride;
    let action = within_body / 5;
    let direction = within_body % 5;
    Some((body as u16, action as u16, direction as u8))
}

fn body_count_for_source(file_index: u8, source_index_count: usize) -> u16 {
    let count = source_index_count as u32;
    let body_count = match file_index {
        0 => {
            if count <= 35000 {
                400
            } else {
                400 + ((count - 35000) / 175)
            }
        }
        1 => {
            if count <= 22000 {
                200
            } else {
                200 + ((count - 22000) / 65)
            }
        }
        _ => {
            if count <= 35000 {
                400
            } else {
                400 + ((count - 35000) / 175)
            }
        }
    };
    body_count.min(u16::MAX as u32) as u16
}

fn action_count_for_body(file_index: u8, body_id: u16) -> u16 {
    match file_index {
        0 => {
            if body_id < 200 {
                22
            } else if body_id < 400 {
                13
            } else {
                35
            }
        }
        1 => {
            if body_id < 200 {
                22
            } else {
                13
            }
        }
        2 => {
            if body_id < 300 {
                13
            } else if body_id < 400 {
                22
            } else {
                35
            }
        }
        _ => {
            if body_id < 200 {
                22
            } else if body_id < 400 {
                13
            } else {
                35
            }
        }
    }
}

pub fn pack_frames_into_pages(
    frames: Vec<DecodedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<BuiltMobileAnimPage>> {
    let mut remaining = prepare_decoded_frames(frames, frame_records, options)?;
    let total_frames = remaining.len() as u64;
    remaining.sort_by_key(|frame| frame.global_frame_index);
    let mut pages = Vec::new();
    let mut page_index = 0u32;
    let mut page_pixels = Vec::new();
    let pb = ProgressBar::new(total_frames);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating mobile animation atlas pages");
    pb.enable_steady_tick(Duration::from_millis(100));

    while !remaining.is_empty() {
        pb.set_message(format!("creating mobile animation atlas page {page_index}"));
        let (page_size, prefix_len) = select_page_bucket(&remaining, options)?;
        if prefix_len == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        let tail = remaining.split_off(prefix_len);
        let page_frames = std::mem::replace(&mut remaining, tail);
        let (page, unplaced, _) =
            build_page(page_index, page_size, page_frames, frame_records, options, &mut page_pixels)?;
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        if !unplaced.is_empty() {
            eyre::bail!(
                "validated mobile animation atlas page {} left {} frames unplaced",
                page_index,
                unplaced.len()
            );
        }

        pb.inc(page.record.frame_count as u64);
        pages.push(page);
        page_index += 1;
    }
    pb.finish_with_message(format!("Mobile animation atlas pages created ({page_index} pages)"));

    Ok(pages)
}

fn pack_frames_into_package(
    package: &mut UddpBuilder,
    frames: Vec<DecodedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<PackedMobileAnimPages> {
    let mut remaining = prepare_decoded_frames(frames, frame_records, options)?;
    let total_frames = remaining.len() as u64;
    remaining.sort_by_key(|frame| frame.global_frame_index);
    let mut records = Vec::new();
    let mut stats = MobileAnimPageStats::default();
    let mut page_index = 0u32;
    let mut pending_pages = Vec::new();
    let mut page_pixels = Vec::new();
    let chunk_size = rayon::current_num_threads().max(1);
    let payload_progress = |_progress: AssetTaskProgress| {};
    let payload_stage = if options.pixel_format == PagePixelFormat::Bc7 {
        AssetTaskProgressStage::EncodingBc7
    } else {
        AssetTaskProgressStage::RegisteringPages
    };
    let payload_completed = AtomicU64::new(0);
    let rdo_payload_completed = AtomicU64::new(0);
    let mut atlas_build_time = Duration::ZERO;
    let mut encode_timings = MobileAnimEncodeTimings::default();

    let pb = ProgressBar::new(total_frames);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating mobile animation atlas pages");
    pb.enable_steady_tick(Duration::from_millis(100));

    while !remaining.is_empty() {
        pb.set_message(format!("creating mobile animation atlas page {page_index}"));
        let (page_size, prefix_len) = select_page_bucket(&remaining, options)?;
        if prefix_len == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        let tail = remaining.split_off(prefix_len);
        let page_frames = std::mem::replace(&mut remaining, tail);
        let page_timer = Instant::now();
        let (page, unplaced, filled_pixel_count) =
            build_page(page_index, page_size, page_frames, frame_records, options, &mut page_pixels)?;
        atlas_build_time += page_timer.elapsed();
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        if !unplaced.is_empty() {
            eyre::bail!(
                "validated mobile animation atlas page {} left {} frames unplaced",
                page_index,
                unplaced.len()
            );
        }

        pb.inc(page.record.frame_count as u64);
        stats.add_page_bounds(page.record.used_width, page.record.used_height, filled_pixel_count);
        records.push(page.record);
        pending_pages.push(page);
        if pending_pages.len() >= chunk_size {
            let timings = encode_and_add_mobile_anim_page_chunk(
                package,
                &mut pending_pages,
                options,
                &payload_progress,
                payload_stage,
                &payload_completed,
                &rdo_payload_completed,
                total_frames,
            )?;
            encode_timings.add_assign(timings);
        }
        page_index += 1;
    }

    if !pending_pages.is_empty() {
        let timings = encode_and_add_mobile_anim_page_chunk(
            package,
            &mut pending_pages,
            options,
            &payload_progress,
            payload_stage,
            &payload_completed,
            &rdo_payload_completed,
            total_frames,
        )?;
        encode_timings.add_assign(timings);
    }

    pb.finish_with_message(format!("Mobile animation atlas pages created ({page_index} pages)"));
    info!(
        "CC mobile animation decoded atlas timing: pages {}, atlas build/blit {:.3}s, BC7 input {:.3}s, BC7 encode {:.3}s, BC7 RDO {:.3}s, BC7 flatten {:.3}s, page registration {:.3}s",
        page_index,
        atlas_build_time.as_secs_f64(),
        encode_timings.bc7.input.as_secs_f64(),
        encode_timings.bc7.encode.as_secs_f64(),
        encode_timings.bc7.rdo.as_secs_f64(),
        encode_timings.bc7.flatten.as_secs_f64(),
        encode_timings.register_pages.as_secs_f64()
    );

    Ok(PackedMobileAnimPages { records, stats })
}

fn pack_planned_frames_into_package(
    package: &mut UddpBuilder,
    frames: Vec<PlannedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    anim_map: &AnimMap,
    options: &MobileAnimCcAtlasOptions,
    payload_progress: &(dyn Fn(AssetTaskProgress) + Sync),
) -> eyre::Result<PackedMobileAnimPages> {
    let mut remaining = frames;
    let total_frames = remaining.len() as u64;
    remaining.sort_by_key(|frame| frame.global_frame_index);
    let mut records = Vec::new();
    let mut stats = MobileAnimPageStats::default();
    let mut page_index = 0u32;
    let mut pending_pages = Vec::new();
    let chunk_size = rayon::current_num_threads().max(1);
    let mut animationframe_package_cache = HashMap::<Arc<PathBuf>, UopPackage>::new();
    let mut decoded_source_cache = HashMap::<PlannedAnimationSource, CachedPlannedAnimation>::new();
    let mut decoded_source_use_tick = 0u64;

    let pb = ProgressBar::new(total_frames);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating mobile animation atlas pages");
    pb.enable_steady_tick(Duration::from_millis(100));
    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::PackingAtlas,
        completed: 0,
        total: total_frames,
    });
    let mut packed_frames = 0u64;
    let payload_stage = if options.pixel_format == PagePixelFormat::Bc7 {
        AssetTaskProgressStage::EncodingBc7
    } else {
        AssetTaskProgressStage::RegisteringPages
    };
    let payload_completed = AtomicU64::new(0);
    let rdo_payload_completed = AtomicU64::new(0);
    let mut atlas_build_time = Duration::ZERO;
    let mut encode_timings = MobileAnimEncodeTimings::default();

    while !remaining.is_empty() {
        pb.set_message(format!("creating mobile animation atlas page {page_index}"));
        let (page_size, prefix_len) = select_planned_page_bucket(&remaining, options)?;
        if prefix_len == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        let tail = remaining.split_off(prefix_len);
        let page_frames = std::mem::replace(&mut remaining, tail);
        let page_timer = Instant::now();
        let (page, unplaced, filled_pixel_count) = build_planned_page(
            page_index,
            page_size,
            page_frames,
            frame_records,
            anim_map,
            options,
            &mut animationframe_package_cache,
            &mut decoded_source_cache,
            &mut decoded_source_use_tick,
        )?;
        atlas_build_time += page_timer.elapsed();
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        if !unplaced.is_empty() {
            eyre::bail!(
                "validated mobile animation atlas page {} left {} frames unplaced",
                page_index,
                unplaced.len()
            );
        }

        pb.inc(page.record.frame_count as u64);
        packed_frames = packed_frames
            .saturating_add(page.record.frame_count as u64)
            .min(total_frames);
        payload_progress(AssetTaskProgress {
            stage: AssetTaskProgressStage::PackingAtlas,
            completed: packed_frames,
            total: total_frames,
        });
        stats.add_page_bounds(page.record.used_width, page.record.used_height, filled_pixel_count);
        records.push(page.record);
        pending_pages.push(page);
        if pending_pages.len() >= chunk_size {
            let timings = encode_and_add_mobile_anim_page_chunk(
                package,
                &mut pending_pages,
                options,
                payload_progress,
                payload_stage,
                &payload_completed,
                &rdo_payload_completed,
                total_frames,
            )?;
            encode_timings.add_assign(timings);
        }
        page_index += 1;
    }

    if !pending_pages.is_empty() {
        let timings = encode_and_add_mobile_anim_page_chunk(
            package,
            &mut pending_pages,
            options,
            payload_progress,
            payload_stage,
            &payload_completed,
            &rdo_payload_completed,
            total_frames,
        )?;
        encode_timings.add_assign(timings);
    }

    pb.finish_with_message(format!("Mobile animation atlas pages created ({page_index} pages)"));
    info!(
        "CC mobile animation planned atlas timing: pages {}, atlas build/blit {:.3}s, BC7 input {:.3}s, BC7 encode {:.3}s, BC7 RDO {:.3}s, BC7 flatten {:.3}s, page registration {:.3}s",
        page_index,
        atlas_build_time.as_secs_f64(),
        encode_timings.bc7.input.as_secs_f64(),
        encode_timings.bc7.encode.as_secs_f64(),
        encode_timings.bc7.rdo.as_secs_f64(),
        encode_timings.bc7.flatten.as_secs_f64(),
        encode_timings.register_pages.as_secs_f64()
    );

    Ok(PackedMobileAnimPages { records, stats })
}

fn select_page_bucket(
    frames: &[DecodedMobileAnimFrame],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(AtlasPageSize, usize)> {
    let mut best = None::<(AtlasPageSize, usize, u64)>;
    let candidates = atlas_page_buckets(options)
        .into_par_iter()
        .map(|page_size| -> eyre::Result<Option<(AtlasPageSize, usize, u64)>> {
            let page_candidates = page_prefix_pack_candidates(frames, page_size, options)?;
            let prefix_len = max_fitting_candidate_prefix_len(&page_candidates, page_size);
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area = candidate_prefix_alloc_area(&page_candidates[..prefix_len]);
            Ok(Some((page_size, prefix_len, alloc_area)))
        })
        .collect::<Vec<_>>();

    for candidate in candidates {
        let Some((page_size, prefix_len, alloc_area)) = candidate? else {
            continue;
        };
        let replace = best
            .map(|(best_size, best_len, best_area)| {
                alloc_area * best_size.area() > best_area * page_size.area()
                    || (alloc_area * best_size.area() == best_area * page_size.area()
                        && (prefix_len > best_len
                            || (prefix_len == best_len && page_size.area() < best_size.area())))
            })
            .unwrap_or(true);
        if replace {
            best = Some((page_size, prefix_len, alloc_area));
        }
    }
    best.map(|(page_size, prefix_len, _)| (page_size, prefix_len))
        .ok_or_else(|| eyre::eyre!(
            "could not fit any mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        ))
}

fn select_planned_page_bucket(
    frames: &[PlannedMobileAnimFrame],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(AtlasPageSize, usize)> {
    let mut best = None::<(AtlasPageSize, usize, u64)>;
    let candidates = atlas_page_buckets(options)
        .into_par_iter()
        .map(|page_size| -> eyre::Result<Option<(AtlasPageSize, usize, u64)>> {
            let page_candidates = planned_page_prefix_pack_candidates(frames, page_size, options)?;
            let prefix_len = max_fitting_candidate_prefix_len(&page_candidates, page_size);
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area = candidate_prefix_alloc_area(&page_candidates[..prefix_len]);
            Ok(Some((page_size, prefix_len, alloc_area)))
        })
        .collect::<Vec<_>>();

    for candidate in candidates {
        let Some((page_size, prefix_len, alloc_area)) = candidate? else {
            continue;
        };
        let replace = best
            .map(|(best_size, best_len, best_area)| {
                alloc_area * best_size.area() > best_area * page_size.area()
                    || (alloc_area * best_size.area() == best_area * page_size.area()
                        && (prefix_len > best_len
                            || (prefix_len == best_len && page_size.area() < best_size.area())))
            })
            .unwrap_or(true);
        if replace {
            best = Some((page_size, prefix_len, alloc_area));
        }
    }
    best.map(|(page_size, prefix_len, _)| (page_size, prefix_len))
        .ok_or_else(|| eyre::eyre!(
            "could not fit any mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        ))
}

fn atlas_page_buckets(options: &MobileAnimCcAtlasOptions) -> Vec<AtlasPageSize> {
    let mut buckets = Vec::new();
    for (width, height) in [
        (512, 512),
        (1024, 512),
        (512, 1024),
        (1024, 1024),
        (2048, 1024),
        (1024, 2048),
        (2048, 2048),
    ] {
        if width <= options.atlas_width && height <= options.atlas_height {
            buckets.push(AtlasPageSize { width, height });
        }
    }
    let max_size = AtlasPageSize {
        width: options.atlas_width,
        height: options.atlas_height,
    };
    if !buckets.contains(&max_size) {
        buckets.push(max_size);
    }
    buckets.sort_by_key(|size| size.area());
    buckets
}

fn max_fitting_candidate_prefix_len<Key: Ord + Copy>(
    candidates: &[PagePackCandidate<Key>],
    page_size: AtlasPageSize,
) -> usize {
    let mut best = 0usize;
    let mut high = 1usize;
    let mut scratch = Vec::new();
    while high <= candidates.len() {
        if candidate_prefix_fits(&candidates[..high], page_size, &mut scratch) {
            best = high;
            if high == candidates.len() {
                return best;
            }
            high = high.saturating_mul(2).min(candidates.len());
        } else {
            break;
        }
    }
    if best == 0 {
        return 0;
    }

    let mut low = best + 1;
    let mut high = high.saturating_sub(1);
    while low <= high {
        let mid = low + (high - low) / 2;
        if candidate_prefix_fits(&candidates[..mid], page_size, &mut scratch) {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }

    best
}

fn candidate_prefix_fits<Key: Ord + Copy>(
    candidates: &[PagePackCandidate<Key>],
    page_size: AtlasPageSize,
    scratch: &mut Vec<PagePackCandidate<Key>>,
) -> bool {
    scratch.clear();
    scratch.extend_from_slice(candidates);
    page_pack_candidates_fit(scratch, page_size)
}

fn candidate_prefix_alloc_area<Key>(candidates: &[PagePackCandidate<Key>]) -> u64 {
    candidates
        .iter()
        .map(|candidate| candidate.alloc_area as u64)
        .sum()
}

fn page_prefix_pack_candidates(
    frames: &[DecodedMobileAnimFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<PagePackCandidate<u32>>> {
    let mut candidates = Vec::with_capacity(page_pack_candidate_initial_capacity(frames.len()));
    for frame in frames {
        let Ok((width_axis, height_axis)) = packing_axes(frame, page_size, options) else {
            break;
        };
        candidates.push(PagePackCandidate {
            sort_key: frame.global_frame_index,
            width_axis,
            height_axis,
            alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
        });
    }
    Ok(candidates)
}

fn planned_page_prefix_pack_candidates(
    frames: &[PlannedMobileAnimFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<PagePackCandidate<u32>>> {
    let mut candidates = Vec::with_capacity(page_pack_candidate_initial_capacity(frames.len()));
    for frame in frames {
        let Ok((width_axis, height_axis)) = planned_packing_axes(frame, page_size, options) else {
            break;
        };
        candidates.push(PagePackCandidate {
            sort_key: frame.global_frame_index,
            width_axis,
            height_axis,
            alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
        });
    }
    Ok(candidates)
}

fn page_pack_candidate_initial_capacity(frame_count: usize) -> usize {
    frame_count.min(PAGE_PACK_CANDIDATE_INITIAL_CAPACITY_LIMIT)
}

fn page_pack_candidates_fit<Key: Ord + Copy>(
    candidates: &mut [PagePackCandidate<Key>],
    page_size: AtlasPageSize,
) -> bool {
    candidates.sort_by_key(|candidate| (Reverse(candidate.alloc_area), candidate.sort_key));
    let mut allocator = AtlasAllocator::new(size2(
        page_size.width as i32,
        page_size.height as i32,
    ));
    for candidate in candidates {
        if allocator
            .allocate(size2(
                candidate.width_axis.alloc_extent as i32,
                candidate.height_axis.alloc_extent as i32,
            ))
            .is_none()
        {
            return false;
        }
    }
    true
}

fn build_page(
    page_index: u32,
    page_size: AtlasPageSize,
    frames: Vec<DecodedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
    pixels: &mut Vec<u8>,
) -> eyre::Result<(BuiltMobileAnimPage, Vec<DecodedMobileAnimFrame>, u64)> {
    let mut allocator = AtlasAllocator::new(size2(
        page_size.width as i32,
        page_size.height as i32,
    ));
    let page_len = page_size.width as usize * page_size.height as usize * 4;
    pixels.clear();
    pixels.resize(page_len, 0);
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let mut page_frame_index = 0u16;
    let mut filled_pixel_count = 0u64;

    let page_frames = decoded_page_frames_with_axes(frames, page_size, options)?;

    for page_frame in page_frames {
        let frame = page_frame.frame;
        let width_axis = page_frame.width_axis;
        let height_axis = page_frame.height_axis;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            filled_pixel_count += blit_rgba_frame(
                pixels,
                page_size.width,
                inner_x as u32,
                inner_y as u32,
                frame.width as u32,
                frame.height as u32,
                &frame.rgba,
            )?;
            extrude_rgba_rect_edges(
                pixels,
                page_size.width,
                page_size.height,
                allocation.rectangle.min.x as u32,
                allocation.rectangle.min.y as u32,
                width_axis.alloc_extent,
                height_axis.alloc_extent,
                inner_x as u32,
                inner_y as u32,
                frame.width as u32,
                frame.height as u32,
            );

            used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
            used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            let record = frame_records
                .get_mut(frame.global_frame_index as usize)
                .context("placed mobile animation frame outside frame table")?;
            record.page_index = page_index;
            record.page_frame_index = page_frame_index;
            record.x = inner_x as u16;
            record.y = inner_y as u16;
            page_frame_index += 1;
        } else {
            leftovers.push(frame);
        }
    }

    let pixels = crate::tex_art_cc::crop_rgba_page(
        pixels,
        page_size.width,
        used_width,
        used_height,
    );

    Ok((
        BuiltMobileAnimPage {
            record: MobileAnimCcPageRecord {
                page_index,
                frame_count: page_frame_index as u32,
                atlas_width: page_size.width,
                atlas_height: page_size.height,
                used_width,
                used_height,
                pixel_format: options.pixel_format,
            },
            pixels,
        },
        leftovers,
        filled_pixel_count,
    ))
}

fn build_planned_page(
    page_index: u32,
    page_size: AtlasPageSize,
    frames: Vec<PlannedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    anim_map: &AnimMap,
    options: &MobileAnimCcAtlasOptions,
    animationframe_packages: &mut HashMap<Arc<PathBuf>, UopPackage>,
    decoded_sources: &mut HashMap<PlannedAnimationSource, CachedPlannedAnimation>,
    decoded_source_use_tick: &mut u64,
) -> eyre::Result<(BuiltMobileAnimPage, Vec<PlannedMobileAnimFrame>, u64)> {
    let mut allocator = AtlasAllocator::new(size2(
        page_size.width as i32,
        page_size.height as i32,
    ));
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let mut page_frame_index = 0u16;
    let mut filled_pixel_count = 0u64;
    let upscale_active = has_effective_upscale(&options.upscale_passes) || options.upscale_profile.is_some();

    struct PendingPlannedBlit {
        body_id: u16,
        global_frame_index: u32,
        inner_x: u32,
        inner_y: u32,
        expected_width: u16,
        expected_height: u16,
        source_left: u16,
        source_top: u16,
        source_crop_width: u16,
        source_crop_height: u16,
        decoded_frames: Arc<Vec<AnimFrame>>,
        source_frame_index: u16,
    }

    struct PreparedPlannedBlit {
        global_frame_index: u32,
        inner_x: u32,
        inner_y: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    struct DirectPlannedBlit {
        global_frame_index: u32,
        inner_x: u32,
        inner_y: u32,
        width: u16,
        height: u16,
        source_left: u16,
        source_top: u16,
        source_crop_width: u16,
        source_crop_height: u16,
        decoded_frames: Arc<Vec<AnimFrame>>,
        source_frame_index: u16,
    }

    struct PlannedExtrusion {
        rect_x: u32,
        rect_y: u32,
        rect_width: u32,
        rect_height: u32,
        content_x: u32,
        content_y: u32,
        content_width: u32,
        content_height: u32,
    }

    let page_frames = planned_page_frames_with_axes(frames, page_size, options)?;
    let page_frame_capacity = page_frames.len();
    let mut planned_extrusions = Vec::with_capacity(page_frame_capacity);
    let mut pending_blits = if upscale_active {
        Vec::with_capacity(page_frame_capacity)
    } else {
        Vec::new()
    };
    let mut pending_direct_blits = if upscale_active {
        Vec::new()
    } else {
        Vec::with_capacity(page_frame_capacity)
    };

    for page_frame in page_frames {
        let frame = page_frame.frame;
        let width_axis = page_frame.width_axis;
        let height_axis = page_frame.height_axis;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            let global_frame_index = frame.global_frame_index;
            let source_frame_index = frame.source_frame_index;
            let width = frame.width;
            let height = frame.height;
            planned_extrusions.push(PlannedExtrusion {
                rect_x: allocation.rectangle.min.x as u32,
                rect_y: allocation.rectangle.min.y as u32,
                rect_width: width_axis.alloc_extent,
                rect_height: height_axis.alloc_extent,
                content_x: inner_x as u32,
                content_y: inner_y as u32,
                content_width: width as u32,
                content_height: height as u32,
            });
            if upscale_active {
                let decoded_frames = cached_decode_planned_animation_source(
                    &frame.source,
                    anim_map,
                    animationframe_packages,
                    decoded_sources,
                    decoded_source_use_tick,
                )?;
                let Some(decoded_frame) = decoded_frames.get(source_frame_index as usize) else {
                    eyre::bail!(
                        "planned mobile animation frame {} missing source frame {}",
                        global_frame_index,
                        source_frame_index
                    );
                };
                pending_blits.push(PendingPlannedBlit {
                    body_id: frame.body_id,
                    global_frame_index,
                    inner_x: inner_x as u32,
                    inner_y: inner_y as u32,
                    expected_width: width,
                    expected_height: height,
                    source_left: frame.source_left,
                    source_top: frame.source_top,
                    source_crop_width: frame.source_width,
                    source_crop_height: frame.source_height,
                    decoded_frames,
                    source_frame_index,
                });
            } else {
                let decoded_frames = cached_decode_planned_animation_source(
                    &frame.source,
                    anim_map,
                    animationframe_packages,
                    decoded_sources,
                    decoded_source_use_tick,
                )?;
                if decoded_frames.get(source_frame_index as usize).is_none() {
                    eyre::bail!(
                        "planned mobile animation frame {} missing source frame {}",
                        global_frame_index,
                        source_frame_index
                    );
                }
                pending_direct_blits.push(DirectPlannedBlit {
                    global_frame_index,
                    inner_x: inner_x as u32,
                    inner_y: inner_y as u32,
                    width,
                    height,
                    source_left: frame.source_left,
                    source_top: frame.source_top,
                    source_crop_width: frame.source_width,
                    source_crop_height: frame.source_height,
                    decoded_frames,
                    source_frame_index,
                });
            }

            used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
            used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            let record = frame_records
                .get_mut(global_frame_index as usize)
                .context("placed mobile animation frame outside frame table")?;
            record.page_index = page_index;
            record.page_frame_index = page_frame_index;
            record.x = inner_x as u16;
            record.y = inner_y as u16;
            page_frame_index += 1;
        } else {
            leftovers.push(frame);
        }
    }

    let mut pixels = vec![0u8; used_width as usize * used_height as usize * 4];
    if upscale_active {
        let prepared_blits = pending_blits
            .into_par_iter()
            .map(|pending| -> eyre::Result<PreparedPlannedBlit> {
                let Some(decoded_frame) = pending.decoded_frames.get(pending.source_frame_index as usize) else {
                    eyre::bail!(
                        "planned mobile animation frame {} missing source frame {}",
                        pending.global_frame_index,
                        pending.source_frame_index
                    );
                };
                let rgba = crop_rgba_frame_window_borrowed(
                    decoded_frame.width,
                    decoded_frame.height,
                    &decoded_frame.data,
                    u32::from(pending.source_left),
                    u32::from(pending.source_top),
                    u32::from(pending.source_crop_width),
                    u32::from(pending.source_crop_height),
                )?
                .into_owned();
                let passes = mobile_upscale_passes_for(
                    options,
                    u32::from(pending.body_id),
                    u32::from(pending.source_frame_index),
                );
                let (width, height, rgba, _, _) = apply_upscale_passes_owned(
                    pending.source_crop_width as u32,
                    pending.source_crop_height as u32,
                    rgba,
                    &passes,
                );
                if width as u16 != pending.expected_width || height as u16 != pending.expected_height {
                    eyre::bail!(
                        "planned mobile animation frame {} changed dimensions: planned {}x{}, decoded {}x{}",
                        pending.global_frame_index,
                        pending.expected_width,
                        pending.expected_height,
                        width,
                        height
                    );
                }
                Ok(PreparedPlannedBlit {
                    global_frame_index: pending.global_frame_index,
                    inner_x: pending.inner_x,
                    inner_y: pending.inner_y,
                    width,
                    height,
                    rgba,
                })
            })
            .collect::<Vec<_>>();

        for prepared in prepared_blits {
            let prepared = prepared?;
            filled_pixel_count += blit_rgba_frame(
                &mut pixels,
                used_width,
                prepared.inner_x,
                prepared.inner_y,
                prepared.width,
                prepared.height,
                &prepared.rgba,
            )
            .wrap_err_with(|| format!("blit mobile animation frame {}", prepared.global_frame_index))?;
        }
    } else {
        for pending in pending_direct_blits {
            let Some(decoded_frame) = pending.decoded_frames.get(pending.source_frame_index as usize) else {
                eyre::bail!(
                    "planned mobile animation frame {} missing source frame {}",
                    pending.global_frame_index,
                    pending.source_frame_index
                );
            };
            filled_pixel_count += blit_rgba_frame_window(
                &mut pixels,
                used_width,
                pending.inner_x,
                pending.inner_y,
                u32::from(decoded_frame.width),
                u32::from(decoded_frame.height),
                &decoded_frame.data,
                u32::from(pending.source_left),
                u32::from(pending.source_top),
                u32::from(pending.source_crop_width),
                u32::from(pending.source_crop_height),
            )
            .wrap_err_with(|| format!("blit mobile animation frame {}", pending.global_frame_index))?;
        }
    }
    for extrusion in planned_extrusions {
        extrude_rgba_rect_edges(
            &mut pixels,
            used_width,
            used_height,
            extrusion.rect_x,
            extrusion.rect_y,
            extrusion.rect_width,
            extrusion.rect_height,
            extrusion.content_x,
            extrusion.content_y,
            extrusion.content_width,
            extrusion.content_height,
        );
    }

    Ok((
        BuiltMobileAnimPage {
            record: MobileAnimCcPageRecord {
                page_index,
                frame_count: page_frame_index as u32,
                atlas_width: page_size.width,
                atlas_height: page_size.height,
                used_width,
                used_height,
                pixel_format: options.pixel_format,
            },
            pixels,
        },
        leftovers,
        filled_pixel_count,
    ))
}

fn packing_axes(
    frame: &DecodedMobileAnimFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    packing_axes_for_dimensions(
        frame.global_frame_index,
        frame.width,
        frame.height,
        page_size,
        options,
    )
}

fn planned_packing_axes(
    frame: &PlannedMobileAnimFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    packing_axes_for_dimensions(
        frame.global_frame_index,
        frame.width,
        frame.height,
        page_size,
        options,
    )
}

fn packing_axes_for_dimensions(
    global_frame_index: u32,
    width: u16,
    height: u16,
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    let width_axis = resolve_packing_axis(
        width as u32,
        page_size.width,
        options.gutter,
        AtlasPackingMode::Bc7Oriented,
        false,
    );
    let height_axis = resolve_packing_axis(
        height as u32,
        page_size.height,
        options.gutter,
        AtlasPackingMode::Bc7Oriented,
        false,
    );
    match (width_axis, height_axis) {
        (Some(width_axis), Some(height_axis)) => Ok((width_axis, height_axis)),
        _ => eyre::bail!(
            "mobile animation frame {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
            global_frame_index,
            width,
            height,
            page_size.width,
            page_size.height,
            options.gutter
        ),
    }
}

fn decoded_page_frames_with_axes(
    frames: Vec<DecodedMobileAnimFrame>,
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<DecodedPageFrame>> {
    let mut page_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        let (width_axis, height_axis) = packing_axes(&frame, page_size, options)?;
        page_frames.push(DecodedPageFrame {
            alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
            frame,
            width_axis,
            height_axis,
        });
    }
    page_frames.sort_by_key(|page_frame| {
        (
            Reverse(page_frame.alloc_area),
            page_frame.frame.global_frame_index,
        )
    });
    Ok(page_frames)
}

fn planned_page_frames_with_axes(
    frames: Vec<PlannedMobileAnimFrame>,
    page_size: AtlasPageSize,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<PlannedPageFrame>> {
    let mut page_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        let (width_axis, height_axis) = planned_packing_axes(&frame, page_size, options)?;
        page_frames.push(PlannedPageFrame {
            alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
            frame,
            width_axis,
            height_axis,
        });
    }
    page_frames.sort_by_key(|page_frame| {
        (
            Reverse(page_frame.alloc_area),
            page_frame.frame.global_frame_index,
        )
    });
    Ok(page_frames)
}

fn prepare_decoded_frames(
    frames: Vec<DecodedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<DecodedMobileAnimFrame>> {
    if !options.crop_transparent_bounds {
        return Ok(frames);
    }

    frames
        .into_iter()
        .map(|frame| trim_decoded_frame(frame, frame_records))
        .collect()
}

fn trim_decoded_frame(
    frame: DecodedMobileAnimFrame,
    frame_records: &mut [MobileAnimCcFrameRecord],
) -> eyre::Result<DecodedMobileAnimFrame> {
    let Some(bounds) = transparent_bounds(frame.width, frame.height, &frame.rgba)? else {
        return Ok(frame);
    };
    if bounds.left == 0
        && bounds.top == 0
        && bounds.right == usize::from(frame.width)
        && bounds.bottom == usize::from(frame.height)
    {
        return Ok(frame);
    }

    let record = frame_records
        .get_mut(frame.global_frame_index as usize)
        .context("trimmed mobile animation frame outside frame table")?;
    let width = (bounds.right - bounds.left) as u16;
    let height = (bounds.bottom - bounds.top) as u16;
    record.width = width;
    record.height = height;
    record.center_x = adjusted_frame_center(
        record.center_x,
        bounds.left as u32,
        "mobile animation frame center_x",
    )?;
    record.center_y = adjusted_frame_center(
        record.center_y,
        bounds.top as u32,
        "mobile animation frame center_y",
    )?;
    let global_frame_index = frame.global_frame_index;
    let rgba = crop_rgba_frame_window(
        frame.width,
        frame.height,
        frame.rgba,
        bounds.left as u32,
        bounds.top as u32,
        u32::from(width),
        u32::from(height),
    )?;

    Ok(DecodedMobileAnimFrame {
        global_frame_index,
        width,
        height,
        rgba,
    })
}

fn transparent_bounds(
    width: u16,
    height: u16,
    rgba: &[u8],
) -> eyre::Result<Option<RgbaBounds>> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for mobile animation trim {}x{}: expected {}, got {}",
            width,
            height,
            expected_len,
            rgba.len()
        );
    }

    Ok(nonzero_alpha_bounds(rgba, width as usize, height as usize))
}

fn crop_rgba_frame_window(
    source_width: u16,
    source_height: u16,
    rgba: Vec<u8>,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
) -> eyre::Result<Vec<u8>> {
    if left == 0 && top == 0 && width == u32::from(source_width) && height == u32::from(source_height) {
        return Ok(rgba);
    }
    if left + width > u32::from(source_width) || top + height > u32::from(source_height) {
        eyre::bail!(
            "mobile animation frame crop {},{} {}x{} is outside source {}x{}",
            left,
            top,
            width,
            height,
            source_width,
            source_height
        );
    }

    let source_stride = source_width as usize * 4;
    let target_stride = width as usize * 4;
    let mut cropped = vec![0u8; width as usize * height as usize * 4];
    for row in 0..height as usize {
        let source_start = ((top as usize + row) * source_stride) + left as usize * 4;
        let target_start = row * target_stride;
        cropped[target_start..target_start + target_stride]
            .copy_from_slice(&rgba[source_start..source_start + target_stride]);
    }
    Ok(cropped)
}

fn crop_rgba_frame_window_borrowed<'a>(
    source_width: u16,
    source_height: u16,
    rgba: &'a [u8],
    left: u32,
    top: u32,
    width: u32,
    height: u32,
) -> eyre::Result<Cow<'a, [u8]>> {
    if left == 0 && top == 0 && width == u32::from(source_width) && height == u32::from(source_height) {
        return Ok(Cow::Borrowed(rgba));
    }
    if left + width > u32::from(source_width) || top + height > u32::from(source_height) {
        eyre::bail!(
            "mobile animation frame crop {},{} {}x{} is outside source {}x{}",
            left,
            top,
            width,
            height,
            source_width,
            source_height
        );
    }

    let source_stride = source_width as usize * 4;
    let target_stride = width as usize * 4;
    let mut cropped = vec![0u8; width as usize * height as usize * 4];
    for row in 0..height as usize {
        let source_start = ((top as usize + row) * source_stride) + left as usize * 4;
        let target_start = row * target_stride;
        cropped[target_start..target_start + target_stride]
            .copy_from_slice(&rgba[source_start..source_start + target_stride]);
    }
    Ok(Cow::Owned(cropped))
}

fn adjusted_frame_center(center: i16, trim_start: u32, label: &str) -> eyre::Result<i16> {
    let adjusted = i32::from(center) - trim_start as i32;
    i16::try_from(adjusted)
        .map_err(|_| eyre::eyre!("{label} exceeds i16 after transparent trim: {adjusted}"))
}

fn blit_rgba_frame(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    frame_width: u32,
    frame_height: u32,
    src: &[u8],
) -> eyre::Result<u64> {
    blit_rgba_frame_window(
        dst,
        dst_width,
        dst_x,
        dst_y,
        frame_width,
        frame_height,
        src,
        0,
        0,
        frame_width,
        frame_height,
    )
}

fn blit_rgba_frame_window(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    source_width: u32,
    source_height: u32,
    src: &[u8],
    source_left: u32,
    source_top: u32,
    frame_width: u32,
    frame_height: u32,
) -> eyre::Result<u64> {
    let expected_len = source_width as usize * source_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for mobile animation frame {}x{}: expected {}, got {}",
            source_width,
            source_height,
            expected_len,
            src.len()
        );
    }
    if source_left + frame_width > source_width || source_top + frame_height > source_height {
        eyre::bail!(
            "mobile animation frame crop {},{} {}x{} is outside source {}x{}",
            source_left,
            source_top,
            frame_width,
            frame_height,
            source_width,
            source_height
        );
    }

    let dst_stride = dst_width as usize * 4;
    if dst_stride == 0 || dst.len() % dst_stride != 0 {
        eyre::bail!("invalid mobile animation atlas row stride for width {}", dst_width);
    }
    let dst_height = dst.len() / dst_stride;
    if dst_x + frame_width > dst_width || dst_y + frame_height > dst_height as u32 {
        eyre::bail!(
            "mobile animation frame destination {},{} {}x{} is outside atlas {}x{}",
            dst_x,
            dst_y,
            frame_width,
            frame_height,
            dst_width,
            dst_height
        );
    }

    let source_stride = source_width as usize * 4;
    let src_stride = frame_width as usize * 4;
    let mut filled_pixel_count = 0u64;
    let source_base = source_top as usize * source_stride + source_left as usize * 4;
    let dst_base = dst_y as usize * dst_stride + dst_x as usize * 4;
    let row_count = frame_height as usize;
    let row_pixels = frame_width as usize;

    // SAFETY: lengths and crop/destination bounds are validated above. The source
    // frame and destination atlas are distinct buffers in all call sites.
    unsafe {
        let source_base_ptr = src.as_ptr().add(source_base);
        let dst_base_ptr = dst.as_mut_ptr().add(dst_base);
        for row in 0..row_count {
            let source_row = source_base_ptr.add(row * source_stride);
            let dst_row = dst_base_ptr.add(row * dst_stride);
            filled_pixel_count += count_nonzero_alpha_pixels_unchecked(source_row, row_pixels);
            std::ptr::copy_nonoverlapping(source_row, dst_row, src_stride);
        }
    }

    Ok(filled_pixel_count)
}

unsafe fn count_nonzero_alpha_pixels_unchecked(row: *const u8, pixels: usize) -> u64 {
    let mut count = 0u64;
    let mut alpha = unsafe { row.add(3) };
    for _ in 0..pixels {
        count += unsafe { (*alpha != 0) as u64 };
        alpha = unsafe { alpha.add(4) };
    }
    count
}

pub fn serialize_page_manifest(
    pages: &[BuiltMobileAnimPage],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let records = pages.iter().map(|page| page.record).collect::<Vec<_>>();
    serialize_page_record_manifest(&records, options)
}

fn serialize_page_record_manifest(
    records: &[MobileAnimCcPageRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(26 + records.len() * 24);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(options.pixel_format as u8);
    bytes.push(AtlasPackingMode::Bc7Oriented as u8);
    bytes.write_u32::<LittleEndian>(records.len() as u32)?;
    for record in records {
        bytes.write_u32::<LittleEndian>(record.page_index)?;
        bytes.write_u32::<LittleEndian>(record.frame_count)?;
        bytes.write_u32::<LittleEndian>(record.atlas_width)?;
        bytes.write_u32::<LittleEndian>(record.atlas_height)?;
        bytes.write_u32::<LittleEndian>(record.used_width)?;
        bytes.write_u32::<LittleEndian>(record.used_height)?;
    }
    Ok(bytes)
}

pub fn serialize_animation_manifest(
    animations: &[MobileAnimCcAnimationRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + animations.len() * 20);
    bytes.extend_from_slice(&ANIMATION_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(animations.len() as u32)?;
    for animation in animations {
        bytes.write_u16::<LittleEndian>(animation.body_id)?;
        bytes.write_u16::<LittleEndian>(animation.action_id)?;
        bytes.push(animation.direction);
        bytes.push(animation.file_index);
        bytes.write_u32::<LittleEndian>(animation.source_index)?;
        bytes.write_u32::<LittleEndian>(animation.frame_start)?;
        bytes.write_u16::<LittleEndian>(animation.frame_count)?;
        bytes.write_u16::<LittleEndian>(animation.flags)?;
    }
    Ok(bytes)
}

pub fn serialize_frame_manifest(frames: &[MobileAnimCcFrameRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + frames.len() * 24);
    bytes.extend_from_slice(&FRAME_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(frames.len() as u32)?;
    for frame in frames {
        bytes.write_u32::<LittleEndian>(frame.animation_index)?;
        bytes.write_u16::<LittleEndian>(frame.frame_index)?;
        bytes.write_u32::<LittleEndian>(frame.page_index)?;
        bytes.write_u16::<LittleEndian>(frame.page_frame_index)?;
        bytes.write_u16::<LittleEndian>(frame.x)?;
        bytes.write_u16::<LittleEndian>(frame.y)?;
        bytes.write_u16::<LittleEndian>(frame.width)?;
        bytes.write_u16::<LittleEndian>(frame.height)?;
        bytes.write_i16::<LittleEndian>(frame.center_x)?;
        bytes.write_i16::<LittleEndian>(frame.center_y)?;
    }
    Ok(bytes)
}

pub fn serialize_body_resolve_manifest(
    records: &[MobileAnimCcBodyResolveRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + records.len() * 10);
    bytes.extend_from_slice(&BODY_RESOLVE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(records.len() as u32)?;
    for record in records {
        bytes.write_u16::<LittleEndian>(record.body_id)?;
        bytes.write_u16::<LittleEndian>(record.resolved_body_id)?;
        bytes.write_u16::<LittleEndian>(record.hue)?;
        bytes.push(record.file_index);
        bytes.write_i8(record.mount_height)?;
        bytes.write_u16::<LittleEndian>(record.flags)?;
    }
    Ok(bytes)
}

pub fn serialize_body_type_manifest(
    records: &[MobileAnimCcBodyTypeRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + records.len() * 7);
    bytes.extend_from_slice(&BODY_TYPE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(records.len() as u32)?;
    for record in records {
        bytes.write_u16::<LittleEndian>(record.body_id)?;
        bytes.push(record.group_type);
        bytes.write_u32::<LittleEndian>(record.flags)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, DataType, UddpReader};
    use udd_assets::MobileAnimCcPackage;

    fn frame(global_frame_index: u32, width: u16, height: u16) -> DecodedMobileAnimFrame {
        DecodedMobileAnimFrame {
            global_frame_index,
            width,
            height,
            rgba: vec![255u8; width as usize * height as usize * 4],
        }
    }

    fn sparse_frame(global_frame_index: u32) -> DecodedMobileAnimFrame {
        let width = 6u16;
        let height = 6u16;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        for y in 2..5usize {
            for x in 1..4usize {
                let index = (y * width as usize + x) * 4;
                rgba[index..index + 4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        DecodedMobileAnimFrame {
            global_frame_index,
            width,
            height,
            rgba,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mobile_anim_cc_{name}_{}_{}",
            std::process::id(),
            timestamp
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn cropped_window_blit_copies_source_rect_and_counts_alpha() {
        let mut src = vec![0u8; 4 * 3 * 4];
        for y in 0..3usize {
            for x in 0..4usize {
                let index = (y * 4 + x) * 4;
                let alpha = if x == 2 && y == 1 { 0 } else { 255 };
                src[index..index + 4].copy_from_slice(&[x as u8, y as u8, (x + y) as u8, alpha]);
            }
        }
        let mut dst = vec![0u8; 5 * 4 * 4];

        let filled = blit_rgba_frame_window(
            &mut dst,
            5,
            2,
            1,
            4,
            3,
            &src,
            1,
            1,
            2,
            2,
        )
        .unwrap();

        assert_eq!(filled, 3);
        for y in 0..2usize {
            for x in 0..2usize {
                let dst_index = ((y + 1) * 5 + x + 2) * 4;
                let src_index = ((y + 1) * 4 + x + 1) * 4;
                assert_eq!(&dst[dst_index..dst_index + 4], &src[src_index..src_index + 4]);
            }
        }
        assert_eq!(&dst[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn source_index_matches_classic_layout() {
        assert_eq!(animation_source_index(0, 0, 0, 0), 0);
        assert_eq!(animation_source_index(0, 1, 0, 0), 110);
        assert_eq!(animation_source_index(0, 200, 0, 0), 22000);
        assert_eq!(animation_source_index(0, 400, 0, 0), 35000);
        assert_eq!(animation_source_index(2, 300, 0, 0), 33000);
        assert_eq!(animation_source_index(4, 34, 0, 0), 22000);
        assert_eq!(animation_source_index(0, 0, 2, 3), 13);
    }

    #[test]
    fn source_index_reverse_mapping_matches_classic_layout() {
        for &(file_index, body_id, action_id, direction) in &[
            (0, 0, 0, 0),
            (0, 1, 0, 0),
            (0, 200, 0, 0),
            (0, 400, 0, 0),
            (2, 300, 0, 0),
            (0, 0, 2, 3),
        ] {
            let source_index = animation_source_index(file_index, body_id, action_id, direction);
            assert_eq!(
                animation_layout_from_source_index(file_index, source_index),
                Some((body_id, action_id, direction))
            );
        }
    }

    #[test]
    fn unmapped_source_index_keeps_original_index_identity() {
        let source_index = 0xFFFF_FFFE;
        let (body_id, action_id, direction, flags) =
            animation_identity_from_source_index(1, source_index);

        assert_eq!(body_id, 0xFFFE);
        assert_eq!(action_id, 0xFFFF);
        assert_eq!(direction, 1);
        assert_eq!(flags, MOBILE_ANIM_CC_FLAG_UNMAPPED_SOURCE_INDEX);
    }

    #[test]
    fn packer_uses_four_pixel_aligned_extents() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 0,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut records = vec![
            MobileAnimCcFrameRecord {
                animation_index: 0,
                frame_index: 0,
                page_index: MISSING_PAGE_INDEX,
                page_frame_index: MISSING_PAGE_FRAME_INDEX,
                x: 0,
                y: 0,
                width: 5,
                height: 5,
                center_x: 0,
                center_y: 0,
            },
            MobileAnimCcFrameRecord {
                animation_index: 0,
                frame_index: 1,
                page_index: MISSING_PAGE_INDEX,
                page_frame_index: MISSING_PAGE_FRAME_INDEX,
                x: 0,
                y: 0,
                width: 5,
                height: 5,
                center_x: 0,
                center_y: 0,
            },
        ];

        let pages = pack_frames_into_pages(vec![frame(0, 5, 5), frame(1, 5, 5)], &mut records, &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(records[0].page_index, 0);
        assert_eq!(records[1].page_index, 0);
        assert_eq!(records[0].x % 4, 0);
        assert_eq!(records[1].x % 4, 0);
        assert!(pages[0].record.used_width % 4 == 0 || pages[0].record.used_height % 4 == 0);
    }

    #[test]
    fn packer_retains_extruded_filter_gutter() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 5,
            height: 5,
            center_x: 0,
            center_y: 0,
        }];

        let pages = pack_frames_into_pages(vec![frame(0, 5, 5)], &mut records, &options).unwrap();

        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page.record.used_width, 16);
        assert_eq!(page.record.used_height, 16);
        let placed = records[0];
        let right_gutter = ((placed.y as u32 * page.record.used_width
            + placed.x as u32
            + placed.width as u32)
            * 4) as usize;
        let left_gutter = ((placed.y as u32 * page.record.used_width + placed.x as u32 - 1) * 4) as usize;
        assert_eq!(&page.pixels[right_gutter..right_gutter + 4], &[255, 255, 255, 255]);
        assert_eq!(&page.pixels[left_gutter..left_gutter + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn transparent_trim_updates_frame_bounds_and_centers() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 0,
            crop_transparent_bounds: true,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 6,
            height: 6,
            center_x: 4,
            center_y: 5,
        }];

        let pages = pack_frames_into_pages(vec![sparse_frame(0)], &mut records, &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(records[0].width, 3);
        assert_eq!(records[0].height, 3);
        assert_eq!(records[0].center_x, 3);
        assert_eq!(records[0].center_y, 3);
        assert_eq!(pages[0].record.used_width % 4, 0);
        assert_eq!(pages[0].record.used_height % 4, 0);
    }

    #[test]
    fn packer_uses_smallest_effective_bucket() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 2048,
            atlas_height: 2048,
            gutter: 4,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 16,
            height: 16,
            center_x: 0,
            center_y: 0,
        }];

        let pages = pack_frames_into_pages(vec![frame(0, 16, 16)], &mut records, &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].record.atlas_width, 512);
        assert_eq!(pages[0].record.atlas_height, 512);
    }

    #[test]
    fn packer_skips_buckets_that_cannot_fit_candidate_frame() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 2048,
            atlas_height: 2048,
            gutter: 4,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 768,
            height: 768,
            center_x: 0,
            center_y: 0,
        }];

        let pages = pack_frames_into_pages(vec![frame(0, 768, 768)], &mut records, &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages[0].record.atlas_width >= 1024);
        assert!(pages[0].record.atlas_height >= 1024);
        assert_eq!(records[0].page_index, 0);
    }

    #[test]
    fn serialized_package_roundtrips_through_reader() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            crop_transparent_bounds: false,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut frame_records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 4,
            height: 4,
            center_x: 2,
            center_y: -1,
        }];
        let pages = pack_frames_into_pages(vec![frame(0, 4, 4)], &mut frame_records, &options).unwrap();
        let animations = vec![MobileAnimCcAnimationRecord {
            body_id: 1,
            action_id: 2,
            direction: 3,
            file_index: 0,
            source_index: 13,
            frame_start: 0,
            frame_count: 1,
            flags: 0,
        }];
        let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
        let animation_manifest = serialize_animation_manifest(&animations).unwrap();
        let frame_manifest = serialize_frame_manifest(&frame_records).unwrap();
        let body_resolve_records = vec![MobileAnimCcBodyResolveRecord {
            body_id: 7,
            resolved_body_id: 8,
            hue: 9,
            file_index: 2,
            mount_height: -1,
            flags: BODY_RESOLVE_FLAG_BODY_DEF | BODY_RESOLVE_FLAG_BODYCONV_DEF,
        }];
        let body_type_records = vec![MobileAnimCcBodyTypeRecord {
            body_id: 7,
            group_type: 3,
            flags: 0x8000_0001,
        }];
        let body_resolve_manifest = serialize_body_resolve_manifest(&body_resolve_records).unwrap();
        let body_type_manifest = serialize_body_type_manifest(&body_type_records).unwrap();
        let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
        let stored_page = pages[0].pixels.clone();

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice(), DataType::Metadata),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice(), DataType::Metadata),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice(), DataType::Metadata),
            (BODY_RESOLVE_MANIFEST_ENTRY_PATH, body_resolve_manifest.as_slice(), DataType::Metadata),
            (BODY_TYPE_MANIFEST_ENTRY_PATH, body_type_manifest.as_slice(), DataType::Metadata),
            (page_path.as_str(), stored_page.as_slice(), DataType::Texture),
        ] {
            builder.add_file(AddFileRequest {
                data_type: data_type as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            }).unwrap();
        }

        let package = MobileAnimCcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(package.pages().len(), 1);
        assert_eq!(package.animation(1, 2, 3).unwrap().source_index, 13);
        assert_eq!(package.animation_frames(package.animation(1, 2, 3).unwrap())[0].center_y, -1);
        assert_eq!(package.body_resolve_record(7).unwrap().file_index, 2);
        assert_eq!(package.body_type_record(7).unwrap().flags, 0x8000_0001);
        assert_eq!(package.read_page_bytes(0).unwrap().len(), stored_page.len());
    }

    #[test]
    fn streaming_packer_writes_package_pages_without_retaining_page_pixels() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            crop_transparent_bounds: false,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
        };
        let mut frame_records = vec![MobileAnimCcFrameRecord {
            animation_index: 0,
            frame_index: 0,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 4,
            height: 4,
            center_x: 0,
            center_y: 0,
        }];
        let animations = vec![MobileAnimCcAnimationRecord {
            body_id: 1,
            action_id: 0,
            direction: 0,
            file_index: 0,
            source_index: 0,
            frame_start: 0,
            frame_count: 1,
            flags: 0,
        }];

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        let packed_pages =
            pack_frames_into_package(&mut builder, vec![frame(0, 4, 4)], &mut frame_records, &options).unwrap();
        let page_manifest = serialize_page_record_manifest(&packed_pages.records, &options).unwrap();
        let animation_manifest = serialize_animation_manifest(&animations).unwrap();
        let frame_manifest = serialize_frame_manifest(&frame_records).unwrap();
        let body_resolve_manifest = serialize_body_resolve_manifest(&[]).unwrap();
        let body_type_manifest = serialize_body_type_manifest(&[]).unwrap();

        for (path, data) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice()),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice()),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice()),
            (BODY_RESOLVE_MANIFEST_ENTRY_PATH, body_resolve_manifest.as_slice()),
            (BODY_TYPE_MANIFEST_ENTRY_PATH, body_type_manifest.as_slice()),
        ] {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            }).unwrap();
        }

        let package = MobileAnimCcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(packed_pages.records.len(), 1);
        assert_eq!(package.pages().len(), 1);
        assert_eq!(package.animation(1, 0, 0).unwrap().frame_count, 1);
        let page = package.pages()[0];
        assert_eq!(
            package.read_page_bytes(0).unwrap().len(),
            page.used_width as usize * page.used_height as usize * 4
        );
    }

    #[test]
    fn mobtypes_parser_normalizes_known_types_and_flags() {
        let records = parse_mobtypes_txt(
            "
            # comment
            7 human 00000001
            8 monster 0x00000002 # trailing comment
            invalid animal 00000004
            ",
        )
        .unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0], MobileAnimCcBodyTypeRecord {
            body_id: 7,
            group_type: 3,
            flags: 0x8000_0001,
        });
        assert_eq!(records[1], MobileAnimCcBodyTypeRecord {
            body_id: 8,
            group_type: 0,
            flags: 0x8000_0002,
        });
    }

    #[test]
    fn body_resolve_records_merge_body_def_and_bodyconv_def() {
        let dir = temp_dir("body_resolve");
        std::fs::write(dir.join("Body.def"), "7 {8} 9\n10 {11} 12\n").unwrap();
        std::fs::write(dir.join("Bodyconv.def"), "7 20\n8 21\n").unwrap();

        let records = build_body_resolve_records(&dir).unwrap();

        assert_eq!(records.len(), 3);
        assert_eq!(records[0], MobileAnimCcBodyResolveRecord {
            body_id: 7,
            resolved_body_id: 20,
            hue: 9,
            file_index: 1,
            mount_height: 0,
            flags: BODY_RESOLVE_FLAG_BODY_DEF | BODY_RESOLVE_FLAG_BODYCONV_DEF,
        });
        assert_eq!(records[1], MobileAnimCcBodyResolveRecord {
            body_id: 8,
            resolved_body_id: 21,
            hue: 0,
            file_index: 1,
            mount_height: 0,
            flags: BODY_RESOLVE_FLAG_BODYCONV_DEF,
        });
        assert_eq!(records[2], MobileAnimCcBodyResolveRecord {
            body_id: 10,
            resolved_body_id: 11,
            hue: 12,
            file_index: 0,
            mount_height: 0,
            flags: BODY_RESOLVE_FLAG_BODY_DEF,
        });
        std::fs::remove_dir_all(dir).ok();
    }
}
