//! Build-time support for `mobile_anim_ec.uddp`.

use std::cmp::Reverse;
use std::collections::{hash_map::Entry, BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use crate::progress::{ProgressBar, ProgressStyle};
use log::{info, warn};
use rayon::prelude::*;
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode,
    UddpStreamingBuilder,
};
use uocf::animation_sequence::{
    AnimationSequence, SequenceAction, FIRST_FRAME_NUMBER, FIRST_STORED_DIRECTION, MIRROR_DIRECTION_SUM,
    MISSING_ACTION_FPS, STORED_DIRECTION_COUNT,
};
use uocf::enhanced::animationframe::{AnimationFrame, AnimationFrameMetadata, DecodedFrame, FrameEntry};
use uocf::enhanced::hues::ShadowPaletteStats;
use uocf::enhanced::mobile_body::{MobileBodyDefinition, MobileBodyMap};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::bc7::{
    encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed, Bc7ProgressStage,
    Bc7RdoOptions, Bc7StageTimings, preferred_bc7_encoder_backend, ImageExtent, RawImageFormat,
    VramTextureEncoding,
};
use crate::package_progress::{
    finish_streaming_package_with_progress, AssetProgressReporter, AssetTaskProgress,
    AssetTaskProgressStage,
};
use crate::rgba_bounds::{count_nonzero_alpha, nonzero_alpha_bounds, RgbaBounds};
use crate::prefix_search::{longest_fitting_prefix, LazyPrefix};
use crate::conversion_tables::EC_MOBILE_ANIMATIONS_KDL;
use crate::source_paths::find_first_dir_matching;
use crate::upscale::{apply_upscale_passes_owned, UpscaleFilter, UpscalePass};
use crate::upscale_profile::{UpscaleImageType, UpscaleProfile, UpscaleTarget};
use crate::{extrude_rgba_rect_edges, resolve_packing_axis, AtlasPackingMode};
use udd_assets::mobile_anim_ec::{
    EcMobileAnimationsKdl,
    hue_mask_entry_path, page_entry_path, MobileAnimEcAnimationRecord, MobileAnimEcBodyRecord,
    MobileAnimEcFrameRecord, MobileAnimEcItemRecord, MobileAnimEcPageRecord,
    MobileAnimEcSourceHintRecord, ANIMATION_FLAG_MIRRORED, ANIMATION_FLAG_NO_SEQUENCE,
    ANIMATION_FLAG_STAND_ACTION, ANIMATION_FLAG_UNDIRECTED, ANIMATION_MANIFEST_ENTRY_PATH,
    BODY_FLAG_FRAMES_FALLBACK, BODY_FLAG_GHOST, BODY_FLAG_HAS_GENDER, BODY_FLAG_HAS_RACE,
    BODY_FLAG_NO_PLACEHOLDER, BODY_FLAG_NO_SEQUENCE,
    BODY_MANIFEST_ENTRY_PATH, DIRECTION_COUNT, FRAME_HUE_ALL, FRAME_HUE_MASKED, FRAME_HUE_NONE,
    FRAME_MANIFEST_ENTRY_PATH, HUE_MASK_HUEABLE, ITEM_MANIFEST_ENTRY_PATH, MISSING_PAGE_FRAME_INDEX,
    MISSING_PAGE_INDEX, MISSING_SOURCE_FRAME_INDEX, MOBILE_ANIM_EC_METADATA_VERSION,
    NO_ALIAS_ACTION, NO_FORCED_HUE, PAGE_FLAG_HUE_MASK, PAGE_MANIFEST_ENTRY_PATH,
    SKIPPED_FRAME_MASK_BITS, SOURCE_HINT_MANIFEST_ENTRY_PATH,
};
use udd_assets::tex_art_cc::PagePixelFormat;
use uocf::utils::bundled_files;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 4;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const ITEM_MANIFEST_MAGIC: [u8; 4] = *b"MEIT";
const SOURCE_HINT_MANIFEST_MAGIC: [u8; 4] = *b"MESH";
const BODY_MANIFEST_MAGIC: [u8; 4] = *b"MEBD";
/// Serialized sizes: manifest header (magic, version, count) and one record.
const MANIFEST_HEADER_SIZE: usize = 12;
const PAGE_MANIFEST_HEADER_SIZE: usize = 26;
const PAGE_RECORD_SIZE: usize = 25;
/// 31 bytes up to `hue_mode`, then the three shadow palette statistics.
const FRAME_RECORD_SIZE: usize = 34;
/// 20 bytes up to `flags`, then `sequence_stored_id` (u32), `race` and `gender` (u8 each).
const BODY_RECORD_SIZE: usize = 26;
const TILEART_UOP_NAMES: [&str; 2] = ["tileart.uop", "TileArt.uop"];
const RGBA_CHANNELS: usize = 4;
const RGBA_ALPHA_CHANNEL: usize = 3;
/// Highest body type whose `AnimationSequence` entry name is looked up (body ids are u16).
const MAX_SEQUENCE_BODY_TYPE: u32 = u16::MAX as u32;
const BC7_PROGRESS_FLUSH_UNITS: u64 = 256;
const EC_ANIMATIONFRAME_FILES: [&str; 12] = [
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
pub struct MobileAnimEcBuildSummary {
    /// Server bodies with animation records (`metadata/bodies.bin` records).
    pub body_count: u32,
    pub animation_count: u32,
    pub frame_count: u32,
    pub packed_frame_count: u32,
    pub page_count: u32,
    pub used_page_pixel_count: u64,
    pub filled_pixel_count: u64,
    pub empty_pixel_count: u64,
    pub item_metadata_count: u32,
    pub source_hint_count: u32,
    pub source_uop_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub layout: MobileAnimEcLayoutStats,
}

/// How the source frames were turned into animation records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MobileAnimEcLayoutStats {
    /// `AnimationSequence` entries that failed to parse (their bodies get no sequence records).
    pub sequence_parse_error_count: u32,
    /// `tileart.uop` creature placeholders (server bodies the client can draw).
    pub placeholder_body_count: u32,
    /// Placeholder bodies whose appearance has no frames: they show the frames of body 0.
    pub frames_fallback_body_count: u32,
    /// Placeholder bodies without frames, even under body 0: no records.
    pub body_without_frames_count: u32,
    /// Bodies whose (sequence, frame body) set was already laid out by another body: their
    /// records share its frame records and pixels.
    pub shared_animation_set_body_count: u32,
    /// Bodies with a sequence or frames but no placeholder: they use their own sequence and
    /// frames (`BODY_FLAG_NO_PLACEHOLDER`).
    pub unmapped_body_count: u32,
    /// Ghost bodies (`BODY_FLAG_GHOST`).
    pub ghost_body_count: u32,
    /// Bodies with sequence records.
    pub sequence_body_count: u32,
    /// Unmapped sequences whose own body has no frames: no records.
    pub sequence_without_frames_count: u32,
    /// Bodies with frames but no sequence: fallback records (`ANIMATION_FLAG_NO_SEQUENCE`).
    pub no_sequence_body_count: u32,
    /// Sequence actions with their own frames.
    pub plain_action_count: u32,
    /// Sequence actions that alias another action.
    pub alias_action_count: u32,
    /// Sequence actions without records because the sequence has no plain action.
    pub unresolved_action_count: u32,
    /// No-sequence actions split into directions by frame number.
    pub fallback_directional_action_count: u32,
    /// No-sequence actions shown undirected (`ANIMATION_FLAG_UNDIRECTED`).
    pub fallback_undirected_action_count: u32,
    /// Records with `ANIMATION_FLAG_MIRRORED`.
    pub mirrored_record_count: u32,
    /// Plain actions without an `AnimationFrame` entry: records with no frames.
    pub plain_action_without_frames_count: u32,
    /// Plain actions whose entry has a frame count other than
    /// `STORED_DIRECTION_COUNT * frame_count`.
    pub frame_count_mismatch_action_count: u32,
    /// Placeholder frames for frame numbers missing from an entry (gaps).
    pub missing_frame_count: u32,
    /// Source frames no record shows, not packed: numbers beyond the sequence frame count,
    /// repeated numbers, and actions or bodies the sequences do not use.
    pub unreferenced_frame_count: u32,
    /// Skipped frames at or beyond `SKIPPED_FRAME_MASK_BITS`, not carried.
    pub dropped_skipped_frame_count: u32,
}

pub struct MobileAnimEcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub pixel_format: PagePixelFormat,
    pub bc7_rdo_lambda: f32,
    pub bc7_rdo_lookback_blocks: usize,
    pub upscale_passes: Vec<UpscalePass>,
    pub upscale_profile: Option<Arc<UpscaleProfile>>,
    /// The EcMobileAnimations.kdl to read; when unset, the copy shipped with the tools ([`EC_MOBILE_ANIMATIONS_KDL`]).
    pub metadata_path: Option<PathBuf>,
    /// Write empty item metadata instead of failing when that table is not found.
    pub allow_missing_metadata: bool,
}

impl Default for MobileAnimEcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedMobileAnimEcFrame {
    pub body_id: u32,
    pub source_frame_index: u16,
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub rgba: Vec<u8>,
    /// One byte per pixel, `HUE_MASK_HUEABLE` where the pixel takes the hue; empty when none
    /// does.
    pub hue_mask: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PlannedMobileAnimEcSource {
    path: Arc<PathBuf>,
    file_hash: u64,
}

#[derive(Debug, Clone)]
struct PlannedMobileAnimEcFrame {
    body_id: u32,
    /// Shadow hue statistics of the palette of the frame's `AnimationFrame` file.
    shadow: ShadowPaletteStats,
    source_frame_index: u16,
    source_entry_index: u16,
    /// `FrameEntry::action`.
    action: u16,
    /// `FrameEntry::frame_number` (1-based).
    frame_number: u16,
    width: u16,
    height: u16,
    source_left: u16,
    source_top: u16,
    source_width: u16,
    source_height: u16,
    center_x: i16,
    center_y: i16,
    source: PlannedMobileAnimEcSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FramePlacement {
    page_index: u32,
    page_frame_index: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    center_x: i16,
    center_y: i16,
    /// `FRAME_HUE_*`, known once the frame is decoded.
    hue_mode: u8,
}

#[derive(Debug, Clone)]
pub struct BuiltMobileAnimEcPage {
    pub record: MobileAnimEcPageRecord,
    pub pixels: Vec<u8>,
    /// R8 hue mask of the used area when some frame is `FRAME_HUE_MASKED`.
    pub hue_mask: Option<Vec<u8>>,
}

struct PackedMobileAnimEcPages {
    records: Vec<MobileAnimEcPageRecord>,
    stats: MobileAnimPageStats,
    timings: AtlasStageTimings,
}

/// Wall time of the atlas stages of a pack: the page bucket and prefix search, the source entry loads (the frame
/// pixels are decoded in the page build), the page allocation, decodes, blits, gutter extrusion and hue masks, the
/// page encode (BC7: summed per page, see [`MobileAnimEncodeTimings`]) and the page registration.
#[derive(Debug, Clone, Copy, Default)]
struct AtlasStageTimings {
    bucket_selection: Duration,
    source_load: Duration,
    page_build: Duration,
    page_encode: Duration,
    encode: MobileAnimEncodeTimings,
}

/// Wall time of every stage of a pack, logged once at the end.
#[derive(Debug, Clone, Copy, Default)]
struct MobileAnimEcStageTimings {
    metadata: Duration,
    plan: Duration,
    layout: Duration,
    trim: Duration,
    upscale: Duration,
    atlas: AtlasStageTimings,
    records_and_manifests: Duration,
    package_write: Duration,
    total: Duration,
}

impl MobileAnimEcStageTimings {
    fn log(&self) {
        let seconds = |duration: Duration| duration.as_secs_f64();
        let atlas = &self.atlas;
        info!(
            "EC mobile animation stage timing: metadata {:.3}s, plan {:.3}s, layout {:.3}s, trim {:.3}s, upscale {:.3}s, \
             bucket selection {:.3}s, source load {:.3}s, page build/decode/blit {:.3}s, page encode {:.3}s \
             (BC7 input {:.3}s, encode {:.3}s, RDO {:.3}s, flatten {:.3}s), page registration {:.3}s, \
             records and manifests {:.3}s, package write {:.3}s, total {:.3}s",
            seconds(self.metadata),
            seconds(self.plan),
            seconds(self.layout),
            seconds(self.trim),
            seconds(self.upscale),
            seconds(atlas.bucket_selection),
            seconds(atlas.source_load),
            seconds(atlas.page_build),
            seconds(atlas.page_encode),
            seconds(atlas.encode.bc7.input),
            seconds(atlas.encode.bc7.encode),
            seconds(atlas.encode.bc7.rdo),
            seconds(atlas.encode.bc7.flatten),
            seconds(atlas.encode.register_pages),
            seconds(self.records_and_manifests),
            seconds(self.package_write),
            seconds(self.total),
        );
    }
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
    frame: DecodedMobileAnimEcFrame,
    width_axis: crate::PackingAxis,
    height_axis: crate::PackingAxis,
    alloc_area: u32,
}

struct PlannedPageFrame {
    frame: PlannedMobileAnimEcFrame,
    width_axis: crate::PackingAxis,
    height_axis: crate::PackingAxis,
    alloc_area: u32,
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

pub fn convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<MobileAnimEcBuildSummary> {
    convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources_with_progress(
        source_dirs,
        out_file,
        options,
        |_| {},
        |_| {},
    )
}

pub fn convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources_with_progress(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimEcAtlasOptions,
    payload_progress: impl Fn(AssetTaskProgress) + Sync,
    mut package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<MobileAnimEcBuildSummary> {
    validate_options(options)?;
    let payload_progress_reporter = AssetProgressReporter::new(payload_progress);
    let payload_progress = |progress| payload_progress_reporter.report(progress);
    let total_timer = Instant::now();
    let client_dir = find_first_dir_matching(source_dirs, &[&["AnimationFrame1.uop"], &["animationframe1.uop"]])
        .ok_or_else(|| eyre::eyre!(
            "no EC animation sources found in any provided path: expected AnimationFrame1.uop"
        ))?;
    let animationframe_paths = discover_animationframe_paths(&client_dir);
    if animationframe_paths.is_empty() {
        eyre::bail!("missing EC AnimationFrame1.uop..AnimationFrame6.uop in {}", client_dir.display());
    }
    let mut stage_timings = MobileAnimEcStageTimings::default();
    let planning_timer = Instant::now();
    let stage_timer = Instant::now();
    let metadata_path = resolve_ec_mobile_animations_kdl(options)?;
    let (items, source_hints) = if let Some(metadata_path) = metadata_path.as_ref() {
        let metadata = EcMobileAnimationsKdl::load(metadata_path)?;
        println!("Using EC mobile animation metadata: {}", metadata_path.display());
        build_item_metadata(&metadata)?
    } else {
        println!("EcMobileAnimations.kdl was not found; writing empty EC mobile animation item metadata.");
        (Vec::new(), Vec::new())
    };
    stage_timings.metadata = stage_timer.elapsed();

    info!(
        "Converting EC mobile animations from {} UOP files to {}",
        animationframe_paths.len(),
        out_file.display()
    );
    println!("Using EC animation source dir: {}", client_dir.display());

    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 0,
        total: 1,
    });
    let stage_timer = Instant::now();
    let mut planned_by_body = plan_animationframe_packages(&animationframe_paths)?;
    stage_timings.plan = stage_timer.elapsed();
    let stage_timer = Instant::now();
    let (sequences, sequence_parse_error_count) = load_animation_sequences(&client_dir)?;
    let body_map = load_mobile_body_map(&client_dir)?;
    let mut layout = build_animation_layout(&source_frames_by_body(&planned_by_body), &sequences, &body_map)?;
    let shadow_by_source = shadow_stats_by_source(&planned_by_body);
    layout.stats.sequence_parse_error_count = sequence_parse_error_count;
    let referenced_frames = layout.referenced_source_frames();
    for (&body_id, frames) in planned_by_body.iter_mut() {
        frames.retain(|frame| referenced_frames.contains(&(body_id, frame.source_frame_index)));
    }
    planned_by_body.retain(|_, frames| !frames.is_empty());
    stage_timings.layout = stage_timer.elapsed();
    info!(
        "EC mobile animation layout: {} records, {} frame records, {:?}",
        layout.animations.len(),
        layout.slots.len(),
        layout.stats
    );
    let stage_timer = Instant::now();
    apply_planned_transparent_trim(&mut planned_by_body, options)?;
    stage_timings.trim = stage_timer.elapsed();
    let stage_timer = Instant::now();
    apply_planned_upscale(&mut planned_by_body, options)?;
    stage_timings.upscale = stage_timer.elapsed();
    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 1,
        total: 1,
    });
    let planned_frames = planned_by_body
        .values()
        .flat_map(|frames| frames.iter().cloned())
        .collect::<Vec<_>>();
    info!(
        "EC mobile animation planning prepared {} bodies and {} frames in {:.3}s",
        planned_by_body.len(),
        planned_frames.len(),
        planning_timer.elapsed().as_secs_f64()
    );
    let mut package = UddpStreamingBuilder::create(LookupMode::VirtualPathHash, out_file)
        .wrap_err_with(|| format!("create payload spill file for {}", out_file.display()))?;
    let pack_timer = Instant::now();
    let (packed_pages, placements) =
        pack_planned_frames_into_package(&mut package, planned_frames, options, &payload_progress)?;
    info!(
        "EC mobile animation atlas and texture payload build wrote {} pages in {:.3}s",
        packed_pages.records.len(),
        pack_timer.elapsed().as_secs_f64()
    );
    stage_timings.atlas = packed_pages.timings;
    let stage_timer = Instant::now();
    let packed_frame_count = placements.len() as u32;
    let layout_stats = layout.stats;
    let (bodies, animations, frames) = layout.into_records(&placements, &shadow_by_source)?;

    let page_manifest = serialize_page_record_manifest(&packed_pages.records, options)?;
    let animation_manifest = serialize_animation_manifest(&animations)?;
    let frame_manifest = serialize_frame_manifest(&frames)?;
    let item_manifest = serialize_item_manifest(&items)?;
    let source_hint_manifest = serialize_source_hint_manifest(&source_hints)?;
    let body_manifest = serialize_body_manifest(&bodies)?;

    for (path, data) in [
        (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice()),
        (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice()),
        (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice()),
        (ITEM_MANIFEST_ENTRY_PATH, item_manifest.as_slice()),
        (SOURCE_HINT_MANIFEST_ENTRY_PATH, source_hint_manifest.as_slice()),
        (BODY_MANIFEST_ENTRY_PATH, body_manifest.as_slice()),
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
    stage_timings.records_and_manifests = stage_timer.elapsed();

    let package_timer = Instant::now();
    finish_streaming_package_with_progress(package, &mut package_progress)?;
    stage_timings.package_write = package_timer.elapsed();
    info!(
        "EC mobile animation package build and write completed in {:.3}s",
        stage_timings.package_write.as_secs_f64()
    );

    let summary = MobileAnimEcBuildSummary {
        body_count: bodies.len() as u32,
        animation_count: animations.len() as u32,
        frame_count: frames.len() as u32,
        packed_frame_count,
        page_count: packed_pages.records.len() as u32,
        used_page_pixel_count: packed_pages.stats.used_page_pixel_count,
        filled_pixel_count: packed_pages.stats.filled_pixel_count,
        empty_pixel_count: packed_pages.stats.empty_pixel_count,
        item_metadata_count: items.len() as u32,
        source_hint_count: source_hints.len() as u32,
        source_uop_count: animationframe_paths.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
        layout: layout_stats,
    };
    stage_timings.total = total_timer.elapsed();
    stage_timings.log();
    info!(
        "EC mobile animation conversion completed in {:.3}s",
        stage_timings.total.as_secs_f64()
    );
    Ok(summary)
}

fn summarize_mobile_anim_pages(pages: &[BuiltMobileAnimEcPage]) -> MobileAnimPageStats {
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

fn validate_options(options: &MobileAnimEcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("EC mobile animation atlas dimensions must be non-zero");
    }
    Ok(())
}

fn encode_and_add_mobile_anim_page_chunk(
    package: &mut UddpStreamingBuilder,
    pages: &mut Vec<BuiltMobileAnimEcPage>,
    options: &MobileAnimEcAtlasOptions,
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
    let hue_mask_files = pages
        .iter_mut()
        .filter_map(|page| {
            let hue_mask = page.hue_mask.take()?;
            Some(AddOwnedFileRequest {
                data_type: DataType::Texture as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: page.record.used_width,
                height: page.record.used_height,
                virtual_path: Some(hue_mask_entry_path(page.record.page_index)),
                path_hash64: None,
                id: None,
                data: hue_mask,
            })
        })
        .collect::<Vec<_>>();
    package.add_owned_files(hue_mask_files)?;
    if options.pixel_format != PagePixelFormat::Bc7 {
        let page_files = pages
            .drain(..)
            .map(|page| AddOwnedFileRequest {
                data_type: DataType::Texture as u8,
                compression: options.compression,
                width: page.record.used_width,
                height: page.record.used_height,
                virtual_path: Some(page_entry_path(page.record.page_index, PagePixelFormat::Rgba8888)),
                path_hash64: None,
                id: None,
                data: page.pixels,
            })
            .collect::<Vec<_>>();
        package.add_owned_files(page_files)?;
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
    let page_files = encoded_pages
        .into_iter()
        .map(|(page_path, stored_page, width, height)| AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width,
            height,
            virtual_path: Some(page_path),
            path_hash64: None,
            id: None,
            data: stored_page,
        })
        .collect::<Vec<_>>();
    package.add_owned_files(page_files)?;
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
    pages: &[BuiltMobileAnimEcPage],
    options: &MobileAnimEcAtlasOptions,
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
            eyre::eyre!("BC7 encode EC mobile animation page {}: {e}", page.record.page_index)
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

fn mobile_anim_chunk_bc7_blocks(pages: &[BuiltMobileAnimEcPage]) -> u64 {
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

fn discover_animationframe_paths(client_dir: &Path) -> Vec<PathBuf> {
    EC_ANIMATIONFRAME_FILES
        .iter()
        .map(|name| client_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

/// The EcMobileAnimations.kdl to read: the one given, else the copy shipped with the tools. A given table must exist;
/// `None` only when none was given, the shipped copy is not found and the options allow packing without it.
fn resolve_ec_mobile_animations_kdl(options: &MobileAnimEcAtlasOptions) -> eyre::Result<Option<PathBuf>> {
    match bundled_files::resolve(options.metadata_path.as_deref(), EC_MOBILE_ANIMATIONS_KDL) {
        Ok(path) => Ok(Some(path)),
        Err(_) if options.allow_missing_metadata && options.metadata_path.is_none() => Ok(None),
        Err(error) => Err(error),
    }
}

fn build_item_metadata(
    metadata: &EcMobileAnimationsKdl,
) -> eyre::Result<(Vec<MobileAnimEcItemRecord>, Vec<MobileAnimEcSourceHintRecord>)> {
    let mut items = Vec::with_capacity(metadata.items.len());
    let mut source_hints = Vec::new();
    for item in &metadata.items {
        if item.animations.len() > u16::MAX as usize {
            eyre::bail!("EC mobile animation item {} has too many source hints", item.id);
        }
        let source_hint_start = source_hints.len() as u32;
        for animation in &item.animations {
            source_hints.push(MobileAnimEcSourceHintRecord {
                item_id: item.id,
                action_id: animation.action_id,
                uop_index: animationframe_uop_index(&animation.uop)?,
                block_index: animation.block,
                file_index: animation.file,
            });
        }
        items.push(MobileAnimEcItemRecord {
            item_id: item.id,
            item_type: item.item_type,
            layer: item.layer,
            flags: item.flags(),
            name: item.name.clone(),
            source_hint_start,
            source_hint_count: item.animations.len() as u16,
        });
    }
    Ok((items, source_hints))
}

fn animationframe_uop_index(name: &str) -> eyre::Result<u8> {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("animationframe")
        .and_then(|value| value.strip_suffix(".uop"))
    else {
        eyre::bail!("invalid EC AnimationFrame UOP name in metadata: {name}");
    };
    let index = rest.parse::<u8>()
        .wrap_err_with(|| format!("invalid EC AnimationFrame UOP index in metadata: {name}"))?;
    if !(1..=6).contains(&index) {
        eyre::bail!("unsupported EC AnimationFrame UOP index in metadata: {name}");
    }
    Ok(index)
}

fn plan_animationframe_packages(
    paths: &[PathBuf],
) -> eyre::Result<BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>> {
    let mut planned_by_body = BTreeMap::<u32, Vec<PlannedMobileAnimEcFrame>>::new();
    for path in paths {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy)
            .wrap_err_with(|| format!("load {}", path.display()))?;
        let planned = plan_animationframe_package(&package, path)?;
        for (body_id, frames) in planned {
            append_planned_body_frames(&mut planned_by_body, body_id, frames)?;
        }
    }
    Ok(planned_by_body)
}

fn append_decoded_body_frames(
    decoded_by_body: &mut BTreeMap<u32, Vec<DecodedMobileAnimEcFrame>>,
    body_id: u32,
    mut frames: Vec<DecodedMobileAnimEcFrame>,
) -> eyre::Result<()> {
    let existing = decoded_by_body.entry(body_id).or_default();
    let offset = existing.len();
    for frame in &mut frames {
        let source_frame_index = offset + frame.source_frame_index as usize;
        if source_frame_index > u16::MAX as usize {
            eyre::bail!("EC mobile animation {} has more than {} frames", body_id, u16::MAX);
        }
        frame.source_frame_index = source_frame_index as u16;
    }
    existing.append(&mut frames);
    Ok(())
}

fn append_planned_body_frames(
    planned_by_body: &mut BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
    body_id: u32,
    mut frames: Vec<PlannedMobileAnimEcFrame>,
) -> eyre::Result<()> {
    let existing = planned_by_body.entry(body_id).or_default();
    let offset = existing.len();
    for frame in &mut frames {
        let source_frame_index = offset + frame.source_frame_index as usize;
        if source_frame_index > u16::MAX as usize {
            eyre::bail!("EC mobile animation {} has more than {} frames", body_id, u16::MAX);
        }
        frame.source_frame_index = source_frame_index as u16;
    }
    existing.append(&mut frames);
    Ok(())
}

fn apply_planned_upscale(
    planned_by_body: &mut BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<()> {
    if options.upscale_passes.is_empty() && options.upscale_profile.is_none() {
        return Ok(());
    }

    for frames in planned_by_body.values_mut() {
        for frame in frames {
            let scale = mobile_upscale_passes_for(
                options,
                frame.body_id,
                u32::from(frame.source_frame_index),
            )
            .iter()
            .copied()
            .fold(1u32, |scale, pass| scale.saturating_mul(pass.scale_factor()));
            if scale == 1 {
                continue;
            }
            frame.width = scale_u16(frame.width, scale, "EC mobile animation frame width")?;
            frame.height = scale_u16(frame.height, scale, "EC mobile animation frame height")?;
            frame.center_x = scale_i16(frame.center_x, scale, "EC mobile animation frame center_x")?;
            frame.center_y = scale_i16(frame.center_y, scale, "EC mobile animation frame center_y")?;
        }
    }
    Ok(())
}

fn apply_planned_transparent_trim(
    planned_by_body: &mut BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<()> {
    let frame_bounds = planned_frame_opaque_bounds(planned_by_body)?;
    let mut frame_bounds = frame_bounds.into_iter();
    for frames in planned_by_body.values_mut() {
        for frame in frames {
            let Some(bounds) = frame_bounds.next().context("EC mobile animation trim bounds ended early")? else {
                continue;
            };
            if bounds.left == 0
                && bounds.top == 0
                && bounds.right == usize::from(frame.width)
                && bounds.bottom == usize::from(frame.height)
            {
                continue;
            }

            frame.source_left = bounds.left as u16;
            frame.source_top = bounds.top as u16;
            frame.width = (bounds.right - bounds.left) as u16;
            frame.height = (bounds.bottom - bounds.top) as u16;
            frame.source_width = frame.width;
            frame.source_height = frame.height;
            frame.center_x = adjusted_frame_center(
                frame.center_x,
                bounds.left as u32,
                "EC mobile animation frame center_x",
            )?;
            frame.center_y = adjusted_frame_center(
                frame.center_y,
                bounds.top as u32,
                "EC mobile animation frame center_y",
            )?;
        }
    }

    Ok(())
}

/// The opaque bounds of every planned frame, body by body in order. Frames sharing a source come in runs; each run
/// parses its source once, and the runs are decoded in parallel.
fn planned_frame_opaque_bounds(
    planned_by_body: &BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
) -> eyre::Result<Vec<Option<RgbaBounds>>> {
    let animationframe_packages = load_planned_animationframe_packages(planned_by_body.values().flatten())?;

    let runs: Vec<&[PlannedMobileAnimEcFrame]> = planned_by_body
        .values()
        .flat_map(|frames| frames.chunk_by(|left, right| left.source == right.source))
        .collect();
    let run_bounds = runs
        .par_iter()
        .map(|run| -> eyre::Result<Vec<Option<RgbaBounds>>> {
            let Some(first) = run.first() else {
                return Ok(Vec::new());
            };
            let animation = decode_planned_animation_source_shared(&first.source, &animationframe_packages)?;
            run.iter()
                .map(|frame| {
                    let Some(source_entry) = animation.frames.get(frame.source_entry_index as usize).copied() else {
                        eyre::bail!(
                            "planned EC mobile animation body {} frame {} missing source entry {} while trimming",
                            frame.body_id,
                            frame.source_frame_index,
                            frame.source_entry_index
                        );
                    };
                    let decoded = animation.decode_frame(&source_entry)?;
                    if decoded.width != frame.width || decoded.height != frame.height {
                        eyre::bail!(
                            "planned EC mobile animation body {} frame {} changed dimensions while trimming: planned {}x{}, decoded {}x{}",
                            frame.body_id,
                            frame.source_frame_index,
                            frame.width,
                            frame.height,
                            decoded.width,
                            decoded.height
                        );
                    }
                    transparent_bounds(decoded.width, decoded.height, &decoded.data)
                })
                .collect()
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    Ok(run_bounds.into_iter().flatten().collect())
}

/// `decode_planned_animation_source` over packages shared between threads, read without keeping their payloads.
fn decode_planned_animation_source_shared(
    source: &PlannedMobileAnimEcSource,
    animationframe_packages: &HashMap<Arc<PathBuf>, UopPackage>,
) -> eyre::Result<AnimationFrame> {
    let package = animationframe_packages
        .get(&source.path)
        .with_context(|| format!("EC AnimationFrame package {} not loaded", source.path.display()))?;
    let data = package.unpack_file_by_hash(source.file_hash)?.ok_or_else(|| {
        eyre::eyre!("EC AnimationFrame payload {:016x} not found in {}", source.file_hash, source.path.display())
    })?;
    AnimationFrame::load(&data)
}

fn has_effective_upscale(passes: &[UpscalePass]) -> bool {
    passes
        .iter()
        .any(|pass| pass.filter().is_none_or(|filter| !matches!(filter, UpscaleFilter::None)))
}

fn mobile_upscale_passes_for(
    options: &MobileAnimEcAtlasOptions,
    body_id: u32,
    frame_id: u32,
) -> Vec<UpscalePass> {
    options
        .upscale_profile
        .as_ref()
        .map(|profile| {
            profile.passes_for(
                UpscaleTarget::with_family(UpscaleImageType::EcMobileAnimationFrames, body_id, frame_id),
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

fn plan_animationframe_package(
    package: &UopPackage,
    path: &Path,
) -> eyre::Result<BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>> {
    let file_hashes = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.filename_hash())
        .collect::<Vec<_>>();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("AnimationFrame UOP");
    let source_path = Arc::new(path.to_path_buf());
    let pb = ProgressBar::new(file_hashes.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("extracting EC mobile animations from {file_name}"));

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
                let Ok(animation) = AnimationFrame::load_metadata(&data) else {
                    return None;
                };
                let mut frames = Vec::with_capacity(animation.frames.len());
                let shadow = ShadowPaletteStats::from_palette(&animation.colours);
                for (index, entry) in animation.frames.iter().enumerate() {
                    if index > u16::MAX as usize {
                        continue;
                    }
                    match planned_frame_from_entry(
                        &animation,
                        shadow,
                        entry,
                        index as u16,
                        Arc::clone(&source_path),
                        file_hash,
                    ) {
                        Ok(frame) => frames.push(frame),
                        Err(error) => return Some(Err(error.wrap_err(format!(
                            "EC animation {} frame entry {index} in {file_name}",
                            animation.animation_id
                        )))),
                    }
                }
                Some(Ok((animation.animation_id, frames)))
            },
        )
        .collect::<Vec<_>>();

    let mut planned_by_body = BTreeMap::<u32, Vec<PlannedMobileAnimEcFrame>>::new();
    for body_frames in parsed.into_iter().flatten() {
        let (body_id, frames) = body_frames?;
        append_planned_body_frames(&mut planned_by_body, body_id, frames)?;
    }
    pb.finish_with_message(format!("EC mobile animations extracted from {file_name}"));
    Ok(planned_by_body)
}

/// Planned frame of one `FrameEntry`. The header `animation_id` is the body of the entry name in
/// every client file, so it keys the frames by body. The anchor is `(-x0, -y0)`.
fn planned_frame_from_entry(
    animation: &AnimationFrameMetadata,
    shadow: ShadowPaletteStats,
    entry: &FrameEntry,
    source_entry_index: u16,
    path: Arc<PathBuf>,
    file_hash: u64,
) -> eyre::Result<PlannedMobileAnimEcFrame> {
    let (width, height) = entry.size()?;
    let (center_x, center_y) = if width == 0 || height == 0 { (0, 0) } else { entry.center() };
    Ok(PlannedMobileAnimEcFrame {
        body_id: animation.animation_id,
        shadow,
        source_frame_index: source_entry_index,
        source_entry_index,
        action: entry.action,
        frame_number: entry.frame_number,
        width,
        height,
        source_left: 0,
        source_top: 0,
        source_width: width,
        source_height: height,
        center_x,
        center_y,
        source: PlannedMobileAnimEcSource { path, file_hash },
    })
}

/// The parsed sources of one atlas page's frames.
type PlannedPageSources = HashMap<PlannedMobileAnimEcSource, Arc<AnimationFrame>>;

/// The EC AnimationFrame packages the planned frames read, opened lazily.
fn load_planned_animationframe_packages<'a>(
    planned_frames: impl Iterator<Item = &'a PlannedMobileAnimEcFrame>,
) -> eyre::Result<HashMap<Arc<PathBuf>, UopPackage>> {
    let mut packages = HashMap::<Arc<PathBuf>, UopPackage>::new();
    for frame in planned_frames {
        if let Entry::Vacant(entry) = packages.entry(Arc::clone(&frame.source.path)) {
            let package = UopPackage::load_with_mode(frame.source.path.as_ref(), LoadMode::Lazy)
                .wrap_err_with(|| format!("load {}", frame.source.path.display()))?;
            entry.insert(package);
        }
    }
    Ok(packages)
}

/// The parsed source of every frame of a page: the sources the previous page also used are reused, the others
/// are loaded in parallel.
fn load_planned_page_sources(
    frames: &[PlannedMobileAnimEcFrame],
    animationframe_packages: &HashMap<Arc<PathBuf>, UopPackage>,
    previous_page_sources: &PlannedPageSources,
) -> eyre::Result<PlannedPageSources> {
    let mut page_sources = PlannedPageSources::new();
    let mut missing_seen = HashSet::new();
    let mut missing = Vec::new();
    for frame in frames {
        if page_sources.contains_key(&frame.source) {
            continue;
        }
        match previous_page_sources.get(&frame.source) {
            Some(animation) => {
                page_sources.insert(frame.source.clone(), Arc::clone(animation));
            }
            None => {
                if missing_seen.insert(&frame.source) {
                    missing.push(&frame.source);
                }
            }
        }
    }
    let loaded = missing
        .par_iter()
        .map(|source| decode_planned_animation_source_shared(source, animationframe_packages).map(Arc::new))
        .collect::<eyre::Result<Vec<_>>>()?;
    for (source, animation) in missing.into_iter().zip(loaded) {
        page_sources.insert(source.clone(), animation);
    }
    Ok(page_sources)
}




/// Identity of one source frame of a body: its position among the body's `AnimationFrame`
/// entries (the atlas key) and its `FrameEntry` action and 1-based frame number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceFrameIdentity {
    source_frame_index: u16,
    action: u16,
    frame_number: u16,
}

/// One frame record before atlas placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameSlot {
    /// Owner record, set when the stored-direction record of the action is emitted.
    animation_index: Option<u32>,
    frame_index: u16,
    source_body_id: u32,
    source_action: u16,
    source_frame_number: u16,
    /// `None` for a placeholder of a frame number missing from the source entry.
    source_frame_index: Option<u16>,
}

/// Body, animation records and their frame slots, built from the sequences before atlas packing.
#[derive(Default)]
struct AnimationLayout {
    bodies: Vec<MobileAnimEcBodyRecord>,
    animations: Vec<MobileAnimEcAnimationRecord>,
    slots: Vec<FrameSlot>,
    /// Frame slots of the animation sets already laid out, shared by the bodies that use them.
    sets: HashMap<AnimationSetKey, AnimationSet>,
    stats: MobileAnimEcLayoutStats,
}

/// One animation set: the actions of a sequence (or the no-sequence fallback) over the frames of
/// one frame body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AnimationSetKey {
    Sequence { sequence_id: u32, frames_body_id: u32 },
    NoSequence { frames_body_id: u32 },
}

#[derive(Debug, Clone)]
enum AnimationSet {
    /// Plain action -> (first slot, frames per direction).
    Sequence(BTreeMap<u32, (u32, u16)>),
    NoSequence(Vec<NoSequenceActionSpan>),
}

#[derive(Debug, Clone, Copy)]
struct NoSequenceActionSpan {
    action: u16,
    slot_start: u32,
    frame_count: u16,
    flags: u16,
    directional: bool,
}

/// Frames of one action of a body, by `FrameEntry` frame number.
#[derive(Default)]
struct ActionSourceFrames {
    /// The first entry of each number, in entry order.
    by_number: BTreeMap<u16, SourceFrameIdentity>,
    /// All entries of the action, repeated numbers included.
    entry_count: usize,
}

fn source_frames_by_body(
    planned_by_body: &BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
) -> BTreeMap<u32, Vec<SourceFrameIdentity>> {
    planned_by_body
        .iter()
        .map(|(&body_id, frames)| {
            (
                body_id,
                frames
                    .iter()
                    .map(|frame| SourceFrameIdentity {
                        source_frame_index: frame.source_frame_index,
                        action: frame.action,
                        frame_number: frame.frame_number,
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// Shadow hue palette statistics of every source `AnimationFrame` file, keyed by (body, action).
fn shadow_stats_by_source(
    planned_by_body: &BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
) -> HashMap<(u32, u16), ShadowPaletteStats> {
    planned_by_body
        .iter()
        .flat_map(|(&body_id, frames)| frames.iter().map(move |frame| ((body_id, frame.action), frame.shadow)))
        .collect()
}

fn source_frames_by_action(frames: &[SourceFrameIdentity]) -> BTreeMap<u16, ActionSourceFrames> {
    let mut by_action = BTreeMap::<u16, ActionSourceFrames>::new();
    for frame in frames {
        let action_frames = by_action.entry(frame.action).or_default();
        action_frames.entry_count += 1;
        action_frames.by_number.entry(frame.frame_number).or_insert(*frame);
    }
    by_action
}

/// Loads every `AnimationSequence` entry, keyed by body type (the entry name). Returns the
/// sequences and the number of entries that failed to parse.
fn load_animation_sequences(client_dir: &Path) -> eyre::Result<(BTreeMap<u32, AnimationSequence>, u32)> {
    let sequence_path = ["AnimationSequence.uop", "animationsequence.uop"]
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file());
    let Some(sequence_path) = sequence_path else {
        warn!("AnimationSequence.uop not found in {}; every EC body uses the no-sequence fallback", client_dir.display());
        return Ok((BTreeMap::new(), 0));
    };
    let mut package = UopPackage::load_with_mode(&sequence_path, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", sequence_path.display()))?;
    let entry_hashes = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.filename_hash())
        .collect::<HashSet<_>>();
    let mut sequences = BTreeMap::new();
    let mut named_entry_count = 0usize;
    let mut parse_error_count = 0u32;
    for body_type in 0..=MAX_SEQUENCE_BODY_TYPE {
        let hash = hash_file_name_single(&AnimationSequence::entry_path(body_type));
        if !entry_hashes.contains(&hash) {
            continue;
        }
        named_entry_count += 1;
        let Some(data) = package.unpack_file_by_hash_cached(hash)? else {
            continue;
        };
        match AnimationSequence::parse(&data) {
            Ok(sequence) => {
                sequences.insert(body_type, sequence);
            }
            Err(error) => {
                parse_error_count += 1;
                warn!("EC animation sequence of body {body_type} failed to parse: {error}");
            }
        }
    }
    let unnamed_entry_count = entry_hashes.len() - named_entry_count;
    if unnamed_entry_count != 0 {
        warn!("{unnamed_entry_count} AnimationSequence.uop entries have no body type name up to {MAX_SEQUENCE_BODY_TYPE}");
    }
    info!("EC mobile animation sequences loaded ({})", sequences.len());
    Ok((sequences, parse_error_count))
}

fn action_id_u16(action: u32) -> eyre::Result<u16> {
    u16::try_from(action)
        .ok()
        .filter(|&action| action != NO_ALIAS_ACTION)
        .ok_or_else(|| eyre::eyre!("EC animation action {action} does not fit the package action ids"))
}

/// Loads the creature placeholders of `tileart.uop`; an empty map (every body unmapped) when the
/// file is missing.
fn load_mobile_body_map(client_dir: &Path) -> eyre::Result<MobileBodyMap> {
    let Some(tileart_path) = TILEART_UOP_NAMES
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file())
    else {
        warn!(
            "tileart.uop not found in {}; EC bodies use their own sequence and frames instead of the client body mapping",
            client_dir.display()
        );
        return Ok(MobileBodyMap::default());
    };
    let map = MobileBodyMap::load(&tileart_path).wrap_err_with(|| format!("load {}", tileart_path.display()))?;
    info!("EC mobile body placeholders loaded ({})", map.definitions.len());
    Ok(map)
}

/// Builds the records the client resolves for "server body, action, direction":
///
/// - A body with a `tileart.uop` creature placeholder (`MobileBodyDefinition`) takes the actions,
///   directions and fps of the sequence named by its body type and the frames of its appearance,
///   falling back to body 0 when the appearance has no frames. Bodies with the same sequence and
///   frame body share one set of frame records (and so the pixels). Ghost bodies are flagged to
///   be drawn with `GHOST_HUE`.
/// - A body without a placeholder (the client cannot show it) uses the sequence of its own name
///   and its own frames.
/// - Each plain action lays out `STORED_DIRECTION_COUNT * frame_count` frame slots, slot `n - 1`
///   holding the frame whose `FrameEntry` number is `n` (directions 3..7, `frame_count` frames
///   each); a missing number gets a placeholder, numbers beyond the slots and repeated numbers are
///   not used.
/// - Every sequence action gets one record per direction. Directions below
///   `FIRST_STORED_DIRECTION` point at the frames of `MIRROR_DIRECTION_SUM - direction` and are
///   flagged mirrored. Alias actions point at the frames of `resolve_action` (flying enabled)
///   and keep the raw alias and the flying-disabled resolution.
/// - A body whose sequence is missing gets `ANIMATION_FLAG_NO_SEQUENCE` records per `FrameEntry`
///   action, at `MISSING_ACTION_FPS`: when the highest frame number is a positive multiple of
///   `STORED_DIRECTION_COUNT`, it is split into directions like a plain action of
///   `highest / STORED_DIRECTION_COUNT` frames; otherwise every direction shows all frames in
///   number order (`ANIMATION_FLAG_UNDIRECTED`).
fn build_animation_layout(
    frames_by_body: &BTreeMap<u32, Vec<SourceFrameIdentity>>,
    sequences: &BTreeMap<u32, AnimationSequence>,
    body_map: &MobileBodyMap,
) -> eyre::Result<AnimationLayout> {
    let mut layout = AnimationLayout::default();
    let body_ids = body_map
        .definitions
        .keys()
        .chain(sequences.keys())
        .chain(frames_by_body.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for body_id in body_ids {
        if let Some(definition) = body_map.definition(body_id) {
            layout.add_placeholder_body(definition, frames_by_body, sequences)?;
        } else {
            layout.add_unmapped_body(body_id, frames_by_body, sequences)?;
        }
    }
    let source_frame_count = frames_by_body.values().map(Vec::len).sum::<usize>();
    layout.stats.unreferenced_frame_count = (source_frame_count - layout.referenced_source_frames().len()) as u32;
    Ok(layout)
}

impl AnimationLayout {
    fn add_placeholder_body(
        &mut self,
        definition: &MobileBodyDefinition,
        frames_by_body: &BTreeMap<u32, Vec<SourceFrameIdentity>>,
        sequences: &BTreeMap<u32, AnimationSequence>,
    ) -> eyre::Result<()> {
        self.stats.placeholder_body_count += 1;
        let Some(frames_body) = definition.resolve_frames_body(|body| frames_by_body.contains_key(&body)) else {
            self.stats.body_without_frames_count += 1;
            return Ok(());
        };
        let mut flags = 0u16;
        if frames_body.fallback {
            flags |= BODY_FLAG_FRAMES_FALLBACK;
            self.stats.frames_fallback_body_count += 1;
        }
        if definition.is_ghost() {
            flags |= BODY_FLAG_GHOST;
            self.stats.ghost_body_count += 1;
        }
        let frames = &frames_by_body[&frames_body.body];
        let sequence = sequences.get(&definition.sequence_id);
        if let Some(sequence) = sequence {
            self.add_sequence_body(definition.body, definition.sequence_id, sequence, frames_body.body, frames)?;
        } else {
            flags |= BODY_FLAG_NO_SEQUENCE;
            self.add_no_sequence_body(definition.body, frames_body.body, frames)?;
        }
        let race = placeholder_byte(definition.body, "race", definition.race)?;
        let gender = placeholder_byte(definition.body, "gender", definition.gender)?;
        if race.is_some() {
            flags |= BODY_FLAG_HAS_RACE;
        }
        if gender.is_some() {
            flags |= BODY_FLAG_HAS_GENDER;
        }
        self.bodies.push(MobileAnimEcBodyRecord {
            body_id: definition.body,
            sequence_id: definition.sequence_id,
            frames_body_id: frames_body.body,
            appearance_body_id: definition.requested_frames_body(),
            forced_hue: definition.forced_hue().unwrap_or(NO_FORCED_HUE),
            flags,
            sequence_stored_id: sequence.map_or(definition.sequence_id, |sequence| sequence.stored_id),
            race: race.unwrap_or_default(),
            gender: gender.unwrap_or_default(),
        });
        Ok(())
    }

    fn add_unmapped_body(
        &mut self,
        body_id: u32,
        frames_by_body: &BTreeMap<u32, Vec<SourceFrameIdentity>>,
        sequences: &BTreeMap<u32, AnimationSequence>,
    ) -> eyre::Result<()> {
        let sequence = sequences.get(&body_id);
        let Some(frames) = frames_by_body.get(&body_id) else {
            if sequence.is_some() {
                self.stats.sequence_without_frames_count += 1;
            }
            return Ok(());
        };
        self.stats.unmapped_body_count += 1;
        let mut flags = BODY_FLAG_NO_PLACEHOLDER;
        if let Some(sequence) = sequence {
            self.add_sequence_body(body_id, body_id, sequence, body_id, frames)?;
        } else {
            flags |= BODY_FLAG_NO_SEQUENCE;
            self.add_no_sequence_body(body_id, body_id, frames)?;
        }
        self.bodies.push(MobileAnimEcBodyRecord {
            body_id,
            sequence_id: body_id,
            frames_body_id: body_id,
            appearance_body_id: body_id,
            forced_hue: NO_FORCED_HUE,
            flags,
            sequence_stored_id: sequence.map_or(body_id, |sequence| sequence.stored_id),
            race: 0,
            gender: 0,
        });
        Ok(())
    }

    /// Lays out the frame slots of the plain actions of `sequence` over `frames`; returns the
    /// span (first slot, frames per direction) of each plain action.
    fn push_sequence_slots(
        &mut self,
        sequence_id: u32,
        sequence: &AnimationSequence,
        frames_body_id: u32,
        frames: &[SourceFrameIdentity],
    ) -> eyre::Result<BTreeMap<u32, (u32, u16)>> {
        let frames_by_action = source_frames_by_action(frames);
        let mut spans = BTreeMap::<u32, (u32, u16)>::new();
        for (&action, record) in sequence.actions.iter().filter(|(_, record)| record.alias.is_none()) {
            self.stats.plain_action_count += 1;
            let action_id = action_id_u16(action)?;
            let slot_start = self.slots.len() as u32;
            let Some(action_frames) = frames_by_action.get(&action_id) else {
                self.stats.plain_action_without_frames_count += 1;
                spans.insert(action, (slot_start, 0));
                continue;
            };
            let frame_count = u16::try_from(record.frame_count).map_err(|_| {
                eyre::eyre!("EC sequence {sequence_id} action {action} frame count {} exceeds u16", record.frame_count)
            })?;
            let slot_count = STORED_DIRECTION_COUNT * u32::from(frame_count);
            if action_frames.entry_count != slot_count as usize {
                self.stats.frame_count_mismatch_action_count += 1;
            }
            self.push_numbered_slots(frames_body_id, action_id, action_frames, slot_count)?;
            spans.insert(action, (slot_start, frame_count));
        }
        Ok(spans)
    }

    fn add_sequence_body(
        &mut self,
        body_id: u32,
        sequence_id: u32,
        sequence: &AnimationSequence,
        frames_body_id: u32,
        frames: &[SourceFrameIdentity],
    ) -> eyre::Result<()> {
        self.stats.sequence_body_count += 1;
        let key = AnimationSetKey::Sequence { sequence_id, frames_body_id };
        let (spans, owns_frames) = match self.sets.get(&key) {
            Some(AnimationSet::Sequence(spans)) => {
                self.stats.shared_animation_set_body_count += 1;
                (spans.clone(), false)
            }
            _ => {
                let spans = self.push_sequence_slots(sequence_id, sequence, frames_body_id, frames)?;
                self.sets.insert(key, AnimationSet::Sequence(spans.clone()));
                (spans, true)
            }
        };

        for (&action, record) in &sequence.actions {
            let (Some(resolved), Some(resolved_no_flying)) =
                (sequence.resolve_action(action, true), sequence.resolve_action(action, false))
            else {
                self.stats.unresolved_action_count += 1;
                continue;
            };
            if record.alias.is_some() {
                self.stats.alias_action_count += 1;
            }
            let (span_start, frame_count) = spans[&resolved];
            let flags = if sequence.is_stand_action(resolved) { ANIMATION_FLAG_STAND_ACTION } else { 0 };
            let template = MobileAnimEcAnimationRecord {
                body_id,
                action_id: action_id_u16(action)?,
                direction: 0,
                frame_start: 0,
                frame_count,
                flags,
                source_body_id: frames_body_id,
                resolved_action_id: action_id_u16(resolved)?,
                alias_action_id: record.alias.map(action_id_u16).transpose()?.unwrap_or(NO_ALIAS_ACTION),
                resolved_action_id_no_flying: action_id_u16(resolved_no_flying)?,
                fps: sequence.action_fps(action, true),
                fps_no_flying: sequence.action_fps(action, false),
                skipped_frame_mask: self.skipped_frame_mask(record),
            };
            for direction in 0..DIRECTION_COUNT {
                let direction_frames = sequence
                    .direction_frames(resolved, direction)
                    .ok_or_else(|| eyre::eyre!("EC sequence {sequence_id} action {resolved} has no direction {direction}"))?;
                let frame_offset = if frame_count == 0 {
                    0
                } else {
                    direction_frames.first_frame_number - FIRST_FRAME_NUMBER
                };
                self.push_record(
                    template,
                    direction,
                    span_start + frame_offset,
                    direction_frames.mirrored,
                    owns_frames && record.alias.is_none() && !direction_frames.mirrored,
                );
            }
        }
        Ok(())
    }

    /// Lays out the no-sequence fallback actions of `frames`.
    fn push_no_sequence_slots(
        &mut self,
        frames_body_id: u32,
        frames: &[SourceFrameIdentity],
    ) -> eyre::Result<Vec<NoSequenceActionSpan>> {
        let mut spans = Vec::new();
        for (action, action_frames) in source_frames_by_action(frames) {
            let highest_number = action_frames.by_number.keys().next_back().copied().unwrap_or(0);
            let slot_start = self.slots.len() as u32;
            let directional =
                highest_number != 0 && u32::from(highest_number) % STORED_DIRECTION_COUNT == 0;
            let (frame_count, flags) = if directional {
                self.stats.fallback_directional_action_count += 1;
                self.push_numbered_slots(frames_body_id, action, &action_frames, u32::from(highest_number))?;
                (
                    (u32::from(highest_number) / STORED_DIRECTION_COUNT) as u16,
                    ANIMATION_FLAG_NO_SEQUENCE,
                )
            } else {
                self.stats.fallback_undirected_action_count += 1;
                for frame in action_frames.by_number.values() {
                    self.slots.push(FrameSlot {
                        animation_index: None,
                        frame_index: 0,
                        source_body_id: frames_body_id,
                        source_action: action,
                        source_frame_number: frame.frame_number,
                        source_frame_index: Some(frame.source_frame_index),
                    });
                }
                (
                    action_frames.by_number.len() as u16,
                    ANIMATION_FLAG_NO_SEQUENCE | ANIMATION_FLAG_UNDIRECTED,
                )
            };
            spans.push(NoSequenceActionSpan {
                action,
                slot_start,
                frame_count,
                flags,
                directional,
            });
        }
        Ok(spans)
    }

    fn add_no_sequence_body(
        &mut self,
        body_id: u32,
        frames_body_id: u32,
        frames: &[SourceFrameIdentity],
    ) -> eyre::Result<()> {
        self.stats.no_sequence_body_count += 1;
        let key = AnimationSetKey::NoSequence { frames_body_id };
        let (spans, owns_frames) = match self.sets.get(&key) {
            Some(AnimationSet::NoSequence(spans)) => {
                self.stats.shared_animation_set_body_count += 1;
                (spans.clone(), false)
            }
            _ => {
                let spans = self.push_no_sequence_slots(frames_body_id, frames)?;
                self.sets.insert(key, AnimationSet::NoSequence(spans.clone()));
                (spans, true)
            }
        };
        for span in spans {
            let template = MobileAnimEcAnimationRecord {
                body_id,
                action_id: span.action,
                direction: 0,
                frame_start: 0,
                frame_count: span.frame_count,
                flags: span.flags,
                source_body_id: frames_body_id,
                resolved_action_id: span.action,
                alias_action_id: NO_ALIAS_ACTION,
                resolved_action_id_no_flying: span.action,
                fps: MISSING_ACTION_FPS,
                fps_no_flying: MISSING_ACTION_FPS,
                skipped_frame_mask: 0,
            };
            for direction in 0..DIRECTION_COUNT {
                if span.directional {
                    let mirrored = direction < FIRST_STORED_DIRECTION;
                    let stored_direction = if mirrored { MIRROR_DIRECTION_SUM - direction } else { direction };
                    let frame_offset =
                        u32::from(stored_direction - FIRST_STORED_DIRECTION) * u32::from(span.frame_count);
                    self.push_record(template, direction, span.slot_start + frame_offset, mirrored, owns_frames && !mirrored);
                } else {
                    self.push_record(
                        template,
                        direction,
                        span.slot_start,
                        false,
                        owns_frames && direction == FIRST_STORED_DIRECTION,
                    );
                }
            }
        }
        Ok(())
    }

    /// Pushes `slot_count` slots, slot `i` holding frame number `FIRST_FRAME_NUMBER + i`.
    fn push_numbered_slots(
        &mut self,
        source_body_id: u32,
        action: u16,
        action_frames: &ActionSourceFrames,
        slot_count: u32,
    ) -> eyre::Result<()> {
        for number in FIRST_FRAME_NUMBER..FIRST_FRAME_NUMBER + slot_count {
            let number = u16::try_from(number)
                .map_err(|_| eyre::eyre!("EC body {source_body_id} action {action} frame number {number} exceeds u16"))?;
            let source = action_frames.by_number.get(&number);
            if source.is_none() {
                self.stats.missing_frame_count += 1;
            }
            self.slots.push(FrameSlot {
                animation_index: None,
                frame_index: 0,
                source_body_id,
                source_action: action,
                source_frame_number: number,
                source_frame_index: source.map(|frame| frame.source_frame_index),
            });
        }
        Ok(())
    }

    /// Pushes the record of `direction`; the owner record also claims its frame slots.
    fn push_record(
        &mut self,
        template: MobileAnimEcAnimationRecord,
        direction: u8,
        frame_start: u32,
        mirrored: bool,
        owns_frames: bool,
    ) {
        let animation_index = self.animations.len() as u32;
        if mirrored {
            self.stats.mirrored_record_count += 1;
        }
        if owns_frames {
            let start = frame_start as usize;
            let slots = &mut self.slots[start..start + usize::from(template.frame_count)];
            for (frame_index, slot) in slots.iter_mut().enumerate() {
                slot.animation_index = Some(animation_index);
                slot.frame_index = frame_index as u16;
            }
        }
        self.animations.push(MobileAnimEcAnimationRecord {
            direction,
            frame_start,
            flags: template.flags | if mirrored { ANIMATION_FLAG_MIRRORED } else { 0 },
            ..template
        });
    }

    fn skipped_frame_mask(&mut self, record: &SequenceAction) -> u32 {
        let mut mask = 0u32;
        for &frame in &record.skipped_frames {
            if frame < SKIPPED_FRAME_MASK_BITS {
                mask |= 1 << frame;
            } else {
                self.stats.dropped_skipped_frame_count += 1;
            }
        }
        mask
    }

    /// Source frames (body, source frame index) shown by some record.
    fn referenced_source_frames(&self) -> HashSet<(u32, u16)> {
        self.slots
            .iter()
            .filter_map(|slot| slot.source_frame_index.map(|index| (slot.source_body_id, index)))
            .collect()
    }

    fn into_records(
        self,
        placements: &HashMap<(u32, u16), FramePlacement>,
        shadow_by_source: &HashMap<(u32, u16), ShadowPaletteStats>,
    ) -> eyre::Result<(
        Vec<MobileAnimEcBodyRecord>,
        Vec<MobileAnimEcAnimationRecord>,
        Vec<MobileAnimEcFrameRecord>,
    )> {
        let frames = self
            .slots
            .iter()
            .map(|slot| {
                let animation_index = slot.animation_index.ok_or_else(|| {
                    eyre::eyre!(
                        "EC body {} action {} frame {} has no owner animation record",
                        slot.source_body_id,
                        slot.source_action,
                        slot.source_frame_number
                    )
                })?;
                let placement = slot
                    .source_frame_index
                    .and_then(|index| placements.get(&(slot.source_body_id, index)));
                let shadow = shadow_by_source
                    .get(&(slot.source_body_id, slot.source_action))
                    .copied()
                    .unwrap_or_default();
                Ok(frame_record_from_placement(animation_index, slot, placement, shadow))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        Ok((self.bodies, self.animations, frames))
    }
}

/// A placeholder property stored as one byte in the body record (every client value fits).
fn placeholder_byte(body: u32, name: &str, value: Option<u32>) -> eyre::Result<Option<u8>> {
    value
        .map(|value| {
            u8::try_from(value).map_err(|_| eyre::eyre!("EC body {body} placeholder {name} {value} does not fit a byte"))
        })
        .transpose()
}

fn frame_record_from_placement(
    animation_index: u32,
    slot: &FrameSlot,
    placement: Option<&FramePlacement>,
    shadow: ShadowPaletteStats,
) -> MobileAnimEcFrameRecord {
    let source_frame_index = slot.source_frame_index.unwrap_or(MISSING_SOURCE_FRAME_INDEX);
    if let Some(placement) = placement {
        MobileAnimEcFrameRecord {
            animation_index,
            frame_index: slot.frame_index,
            source_frame_index,
            source_action: slot.source_action,
            source_frame_number: slot.source_frame_number,
            page_index: placement.page_index,
            page_frame_index: placement.page_frame_index,
            x: placement.x,
            y: placement.y,
            width: placement.width,
            height: placement.height,
            center_x: placement.center_x,
            center_y: placement.center_y,
            hue_mode: placement.hue_mode,
            shadow,
        }
    } else {
        MobileAnimEcFrameRecord {
            animation_index,
            frame_index: slot.frame_index,
            source_frame_index,
            source_action: slot.source_action,
            source_frame_number: slot.source_frame_number,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            center_x: 0,
            center_y: 0,
            hue_mode: FRAME_HUE_NONE,
            shadow,
        }
    }
}

pub(crate) fn pack_frames_into_pages(
    frames: Vec<DecodedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(Vec<BuiltMobileAnimEcPage>, HashMap<(u32, u16), FramePlacement>)> {
    let mut remaining = prepare_decoded_frames(frames, options)?
        .into_iter()
        .filter(|frame| frame.width != 0 && frame.height != 0 && !frame.rgba.is_empty())
        .collect::<Vec<_>>();
    let pb = ProgressBar::new(remaining.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating EC mobile animation atlas pages");
    pb.enable_steady_tick(Duration::from_millis(100));

    remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
    let mut pages = Vec::new();
    let mut placements = HashMap::new();
    let mut page_index = 0u32;
    let mut page_pixels = Vec::new();

    while !remaining.is_empty() {
        pb.set_message(format!("creating EC mobile animation atlas page {}", page_index + 1));
        let (page_size, page_frames, leftovers) = take_page_frame_prefix(remaining, options)?;
        let (page, unplaced, _) =
            build_page(page_index, page_size, page_frames, &mut placements, options, &mut page_pixels)?;
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any EC mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        pb.inc(page.record.frame_count as u64);
        pages.push(page);
        remaining = leftovers;
        remaining.extend(unplaced);
        remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
        page_index += 1;
    }

    pb.finish_with_message(format!("EC mobile animation atlas pages created ({})", pages.len()));
    Ok((pages, placements))
}

fn pack_planned_frames_into_package(
    package: &mut UddpStreamingBuilder,
    frames: Vec<PlannedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
    payload_progress: &(dyn Fn(AssetTaskProgress) + Sync),
) -> eyre::Result<(PackedMobileAnimEcPages, HashMap<(u32, u16), FramePlacement>)> {
    let mut remaining = frames
        .into_iter()
        .filter(|frame| frame.width != 0 && frame.height != 0)
        .collect::<Vec<_>>();
    let pb = ProgressBar::new(remaining.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating EC mobile animation atlas pages");
    pb.enable_steady_tick(Duration::from_millis(100));
    let total_frames = remaining.len() as u64;
    payload_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::PackingAtlas,
        completed: 0,
        total: total_frames,
    });

    remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
    let mut records = Vec::new();
    let mut stats = MobileAnimPageStats::default();
    let mut placements = HashMap::new();
    let mut page_index = 0u32;
    let mut pending_pages = Vec::new();
    let chunk_size = rayon::current_num_threads().max(1);
    let animationframe_packages = load_planned_animationframe_packages(remaining.iter())?;
    let mut previous_page_sources = PlannedPageSources::new();
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
    let mut timings = AtlasStageTimings::default();

    while !remaining.is_empty() {
        pb.set_message(format!("creating EC mobile animation atlas page {}", page_index + 1));
        let bucket_timer = Instant::now();
        let (page_size, prefix_len) = select_planned_page_bucket(&remaining, options)?;
        timings.bucket_selection += bucket_timer.elapsed();
        if prefix_len == 0 {
            eyre::bail!(
                "could not fit any EC mobile animation frame into atlas page {}x{}",
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
            &mut placements,
            options,
            &animationframe_packages,
            &mut previous_page_sources,
            &mut timings.source_load,
        )?;
        atlas_build_time += page_timer.elapsed();
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any EC mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        if !unplaced.is_empty() {
            eyre::bail!(
                "validated EC mobile animation atlas page {} left {} frames unplaced",
                page_index + 1,
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
            let encode_timer = Instant::now();
            let chunk_timings = encode_and_add_mobile_anim_page_chunk(
                package,
                &mut pending_pages,
                options,
                payload_progress,
                payload_stage,
                &payload_completed,
                &rdo_payload_completed,
                total_frames,
            )?;
            timings.page_encode += encode_timer.elapsed();
            encode_timings.add_assign(chunk_timings);
        }
        page_index += 1;
    }

    if !pending_pages.is_empty() {
        let encode_timer = Instant::now();
        let chunk_timings = encode_and_add_mobile_anim_page_chunk(
            package,
            &mut pending_pages,
            options,
            payload_progress,
            payload_stage,
            &payload_completed,
            &rdo_payload_completed,
            total_frames,
        )?;
        timings.page_encode += encode_timer.elapsed();
        encode_timings.add_assign(chunk_timings);
    }
    timings.page_build = atlas_build_time.saturating_sub(timings.source_load);
    timings.encode = encode_timings;

    pb.finish_with_message(format!("EC mobile animation atlas pages created ({})", records.len()));
    info!(
        "EC mobile animation planned atlas timing: pages {}, atlas build/blit {:.3}s, BC7 input {:.3}s, BC7 encode {:.3}s, BC7 RDO {:.3}s, BC7 flatten {:.3}s, page registration {:.3}s",
        records.len(),
        atlas_build_time.as_secs_f64(),
        encode_timings.bc7.input.as_secs_f64(),
        encode_timings.bc7.encode.as_secs_f64(),
        encode_timings.bc7.rdo.as_secs_f64(),
        encode_timings.bc7.flatten.as_secs_f64(),
        encode_timings.register_pages.as_secs_f64()
    );
    Ok((PackedMobileAnimEcPages { records, stats, timings }, placements))
}

fn take_page_frame_prefix(
    frames: Vec<DecodedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(AtlasPageSize, Vec<DecodedMobileAnimEcFrame>, Vec<DecodedMobileAnimEcFrame>)> {
    let (page_size, prefix_len) = select_page_bucket(&frames, options)?;
    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any EC mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }
    let mut leftovers = frames;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((page_size, selected, leftovers))
}

fn select_page_bucket(
    frames: &[DecodedMobileAnimEcFrame],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(AtlasPageSize, usize)> {
    let mut best = None::<(AtlasPageSize, usize, u64)>;
    let candidates = atlas_page_buckets(options)
        .into_par_iter()
        .map(|page_size| -> eyre::Result<Option<(AtlasPageSize, usize, u64)>> {
            let mut page_candidates =
                LazyPrefix::new(frames, |frame| pack_candidate(frame, page_size, options));
            let prefix_len =
                longest_fitting_prefix(&mut page_candidates, |prefix| candidate_prefix_fits(prefix, page_size));
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area = candidate_prefix_alloc_area(page_candidates.load(prefix_len));
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
            "could not fit any EC mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        ))
}

fn select_planned_page_bucket(
    frames: &[PlannedMobileAnimEcFrame],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(AtlasPageSize, usize)> {
    let mut best = None::<(AtlasPageSize, usize, u64)>;
    let candidates = atlas_page_buckets(options)
        .into_par_iter()
        .map(|page_size| -> eyre::Result<Option<(AtlasPageSize, usize, u64)>> {
            let mut page_candidates =
                LazyPrefix::new(frames, |frame| planned_pack_candidate(frame, page_size, options));
            let prefix_len =
                longest_fitting_prefix(&mut page_candidates, |prefix| candidate_prefix_fits(prefix, page_size));
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area = candidate_prefix_alloc_area(page_candidates.load(prefix_len));
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
            "could not fit any EC mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        ))
}

fn atlas_page_buckets(options: &MobileAnimEcAtlasOptions) -> Vec<AtlasPageSize> {
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


fn candidate_prefix_fits<Key: Ord + Copy>(candidates: &[PagePackCandidate<Key>], page_size: AtlasPageSize) -> bool {
    page_pack_candidates_fit(&mut candidates.to_vec(), page_size)
}

fn candidate_prefix_alloc_area<Key>(candidates: &[PagePackCandidate<Key>]) -> u64 {
    candidates
        .iter()
        .map(|candidate| candidate.alloc_area as u64)
        .sum()
}

/// The pack candidate of `frame` in a page of `page_size`, or none when it does not fit one.
fn pack_candidate(
    frame: &DecodedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> Option<PagePackCandidate<(u32, u16)>> {
    let (width_axis, height_axis) = packing_axes(frame, page_size, options).ok()?;
    Some(PagePackCandidate {
        sort_key: (frame.body_id, frame.source_frame_index),
        width_axis,
        height_axis,
        alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
    })
}

/// The pack candidate of `frame` in a page of `page_size`, or none when it does not fit one.
fn planned_pack_candidate(
    frame: &PlannedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> Option<PagePackCandidate<(u32, u16)>> {
    let (width_axis, height_axis) = planned_packing_axes(frame, page_size, options).ok()?;
    Some(PagePackCandidate {
        sort_key: (frame.body_id, frame.source_frame_index),
        width_axis,
        height_axis,
        alloc_area: width_axis.alloc_extent * height_axis.alloc_extent,
    })
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
    frames: Vec<DecodedMobileAnimEcFrame>,
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    options: &MobileAnimEcAtlasOptions,
    pixels: &mut Vec<u8>,
) -> eyre::Result<(BuiltMobileAnimEcPage, Vec<DecodedMobileAnimEcFrame>, u64)> {
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
    let mut placed_hue_masks = Vec::new();

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
            let extrusion = RectExtrusion {
                rect_x: allocation.rectangle.min.x as u32,
                rect_y: allocation.rectangle.min.y as u32,
                rect_width: width_axis.alloc_extent,
                rect_height: height_axis.alloc_extent,
                content_x: inner_x as u32,
                content_y: inner_y as u32,
                content_width: frame.width as u32,
                content_height: frame.height as u32,
            };
            extrusion.extrude_rgba(pixels, page_size.width, page_size.height);
            used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
            used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            let hue_mode = frame_window_hue_mode(
                &frame.rgba,
                &frame.hue_mask,
                u32::from(frame.width),
                FrameWindow::full(frame.width, frame.height),
            );
            if hue_mode == FRAME_HUE_MASKED {
                placed_hue_masks.push(PlacedHueMask {
                    extrusion,
                    mask: frame.hue_mask,
                });
            }
            placements.insert((frame.body_id, frame.source_frame_index), FramePlacement {
                page_index,
                page_frame_index,
                x: inner_x as u16,
                y: inner_y as u16,
                width: frame.width,
                height: frame.height,
                center_x: frame.center_x,
                center_y: frame.center_y,
                hue_mode,
            });
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
    let hue_mask = build_page_hue_mask(used_width, used_height, placed_hue_masks)?;

    Ok((
        BuiltMobileAnimEcPage {
            record: MobileAnimEcPageRecord {
                page_index,
                frame_count: page_frame_index as u32,
                atlas_width: page_size.width,
                atlas_height: page_size.height,
                used_width,
                used_height,
                pixel_format: options.pixel_format,
                flags: page_flags(&hue_mask),
            },
            pixels,
            hue_mask,
        },
        leftovers,
        filled_pixel_count,
    ))
}

fn build_planned_page(
    page_index: u32,
    page_size: AtlasPageSize,
    frames: Vec<PlannedMobileAnimEcFrame>,
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    options: &MobileAnimEcAtlasOptions,
    animationframe_packages: &HashMap<Arc<PathBuf>, UopPackage>,
    previous_page_sources: &mut PlannedPageSources,
    source_load_time: &mut Duration,
) -> eyre::Result<(BuiltMobileAnimEcPage, Vec<PlannedMobileAnimEcFrame>, u64)> {
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
        body_id: u32,
        source_frame_index: u16,
        extrusion: RectExtrusion,
        inner_x: u32,
        inner_y: u32,
        expected_width: u16,
        expected_height: u16,
        source_left: u16,
        source_top: u16,
        source_crop_width: u16,
        source_crop_height: u16,
        animation: Arc<AnimationFrame>,
        source_entry: FrameEntry,
    }

    struct PreparedPlannedBlit {
        body_id: u32,
        source_frame_index: u16,
        extrusion: RectExtrusion,
        inner_x: u32,
        inner_y: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        hue_mode: u8,
        /// Frame-sized hue mask of a `FRAME_HUE_MASKED` frame.
        hue_mask: Option<Vec<u8>>,
    }

    let load_timer = Instant::now();
    let page_sources = load_planned_page_sources(&frames, animationframe_packages, previous_page_sources)?;
    *source_load_time += load_timer.elapsed();
    let page_frames = planned_page_frames_with_axes(frames, page_size, options)?;
    let page_frame_capacity = page_frames.len();
    let mut planned_extrusions = Vec::with_capacity(page_frame_capacity);
    let mut pending_blits = Vec::with_capacity(page_frame_capacity);

    for page_frame in page_frames {
        let frame = page_frame.frame;
        let width_axis = page_frame.width_axis;
        let height_axis = page_frame.height_axis;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let animation = page_sources
                .get(&frame.source)
                .cloned()
                .context("planned EC mobile animation source was not loaded for its page")?;
            let Some(source_entry) = animation.frames.get(frame.source_entry_index as usize).copied() else {
                eyre::bail!(
                    "planned EC mobile animation body {} frame {} missing source entry {}",
                    frame.body_id,
                    frame.source_frame_index,
                    frame.source_entry_index
                );
            };
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            let extrusion = RectExtrusion {
                rect_x: allocation.rectangle.min.x as u32,
                rect_y: allocation.rectangle.min.y as u32,
                rect_width: width_axis.alloc_extent,
                rect_height: height_axis.alloc_extent,
                content_x: inner_x as u32,
                content_y: inner_y as u32,
                content_width: frame.width as u32,
                content_height: frame.height as u32,
            };
            planned_extrusions.push(extrusion);
            pending_blits.push(PendingPlannedBlit {
                body_id: frame.body_id,
                source_frame_index: frame.source_frame_index,
                extrusion,
                inner_x: inner_x as u32,
                inner_y: inner_y as u32,
                expected_width: frame.width,
                expected_height: frame.height,
                source_left: frame.source_left,
                source_top: frame.source_top,
                source_crop_width: frame.source_width,
                source_crop_height: frame.source_height,
                animation,
                source_entry,
            });
            used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
            used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            placements.insert((frame.body_id, frame.source_frame_index), FramePlacement {
                page_index,
                page_frame_index,
                x: inner_x as u16,
                y: inner_y as u16,
                width: frame.width,
                height: frame.height,
                center_x: frame.center_x,
                center_y: frame.center_y,
                // Set once the frame is decoded below.
                hue_mode: FRAME_HUE_NONE,
            });
            page_frame_index += 1;
        } else {
            leftovers.push(frame);
        }
    }

    let mut pixels = vec![0u8; used_width as usize * used_height as usize * 4];
    let mut placed_hue_masks = Vec::new();
    if upscale_active {
        let prepared_blits = pending_blits
            .into_par_iter()
            .map(|pending| -> eyre::Result<PreparedPlannedBlit> {
                let decoded = pending.animation.decode_frame(&pending.source_entry)?;
                let window = FrameWindow {
                    left: u32::from(pending.source_left),
                    top: u32::from(pending.source_top),
                    width: u32::from(pending.source_crop_width),
                    height: u32::from(pending.source_crop_height),
                };
                let hue_mode =
                    frame_window_hue_mode(&decoded.data, &decoded.hue_mask, u32::from(decoded.width), window);
                let source_hue_mask = (hue_mode == FRAME_HUE_MASKED)
                    .then(|| crop_hue_mask_window(&decoded, window))
                    .transpose()?;
                let rgba = crop_rgba_frame_window(
                    decoded.width,
                    decoded.height,
                    decoded.data,
                    window.left,
                    window.top,
                    window.width,
                    window.height,
                )?;
                let passes = mobile_upscale_passes_for(
                    options,
                    pending.body_id,
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
                        "planned EC mobile animation body {} frame {} changed dimensions: planned {}x{}, decoded {}x{}",
                        pending.body_id,
                        pending.source_frame_index,
                        pending.expected_width,
                        pending.expected_height,
                        width,
                        height
                    );
                }
                let hue_mask = source_hue_mask
                    .map(|mask| resample_hue_mask_nearest(&mask, window.width, window.height, width, height));
                Ok(PreparedPlannedBlit {
                    body_id: pending.body_id,
                    source_frame_index: pending.source_frame_index,
                    extrusion: pending.extrusion,
                    inner_x: pending.inner_x,
                    inner_y: pending.inner_y,
                    width,
                    height,
                    rgba,
                    hue_mode,
                    hue_mask,
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
            .wrap_err_with(|| format!(
                "blit EC mobile animation body {} frame {}",
                prepared.body_id,
                prepared.source_frame_index
            ))?;
            set_placement_hue_mode(placements, prepared.body_id, prepared.source_frame_index, prepared.hue_mode);
            if let Some(mask) = prepared.hue_mask {
                placed_hue_masks.push(PlacedHueMask {
                    extrusion: prepared.extrusion,
                    mask,
                });
            }
        }
    } else {
        // Decode and hue-scan the frames in parallel; blit them in placement order.
        let decoded_frames = pending_blits
            .par_iter()
            .map(|pending| -> eyre::Result<_> {
                let decoded = pending.animation.decode_frame(&pending.source_entry)?;
                let window = FrameWindow {
                    left: u32::from(pending.source_left),
                    top: u32::from(pending.source_top),
                    width: u32::from(pending.source_crop_width),
                    height: u32::from(pending.source_crop_height),
                };
                let hue_mode =
                    frame_window_hue_mode(&decoded.data, &decoded.hue_mask, u32::from(decoded.width), window);
                let hue_mask = (hue_mode == FRAME_HUE_MASKED)
                    .then(|| crop_hue_mask_window(&decoded, window))
                    .transpose()?;
                Ok((decoded, hue_mode, hue_mask))
            })
            .collect::<Vec<_>>();
        for (pending, decoded) in pending_blits.into_iter().zip(decoded_frames) {
            let (decoded, hue_mode, hue_mask) = decoded?;
            set_placement_hue_mode(placements, pending.body_id, pending.source_frame_index, hue_mode);
            if let Some(mask) = hue_mask {
                placed_hue_masks.push(PlacedHueMask {
                    extrusion: pending.extrusion,
                    mask,
                });
            }
            filled_pixel_count += blit_rgba_frame_window(
                &mut pixels,
                used_width,
                pending.inner_x,
                pending.inner_y,
                u32::from(decoded.width),
                u32::from(decoded.height),
                &decoded.data,
                u32::from(pending.source_left),
                u32::from(pending.source_top),
                u32::from(pending.source_crop_width),
                u32::from(pending.source_crop_height),
            )
            .wrap_err_with(|| format!(
                "blit EC mobile animation body {} frame {}",
                pending.body_id,
                pending.source_frame_index
            ))?;
        }
    }
    *previous_page_sources = page_sources;
    for extrusion in planned_extrusions {
        extrusion.extrude_rgba(&mut pixels, used_width, used_height);
    }
    let hue_mask = build_page_hue_mask(used_width, used_height, placed_hue_masks)?;

    Ok((
        BuiltMobileAnimEcPage {
            record: MobileAnimEcPageRecord {
                page_index,
                frame_count: page_frame_index as u32,
                atlas_width: page_size.width,
                atlas_height: page_size.height,
                used_width,
                used_height,
                pixel_format: options.pixel_format,
                flags: page_flags(&hue_mask),
            },
            pixels,
            hue_mask,
        },
        leftovers,
        filled_pixel_count,
    ))
}

/// Allocation rect of a frame in an atlas page and the frame content inside it; the gutter
/// around the content repeats its nearest edge pixel.
#[derive(Debug, Clone, Copy)]
struct RectExtrusion {
    rect_x: u32,
    rect_y: u32,
    rect_width: u32,
    rect_height: u32,
    content_x: u32,
    content_y: u32,
    content_width: u32,
    content_height: u32,
}

impl RectExtrusion {
    fn extrude_rgba(&self, pixels: &mut [u8], atlas_width: u32, atlas_height: u32) {
        extrude_rgba_rect_edges(
            pixels,
            atlas_width,
            atlas_height,
            self.rect_x,
            self.rect_y,
            self.rect_width,
            self.rect_height,
            self.content_x,
            self.content_y,
            self.content_width,
            self.content_height,
        );
    }

    /// `extrude_rgba` for a one byte per pixel plane.
    fn extrude_r8(&self, plane: &mut [u8], plane_width: u32, plane_height: u32) {
        if self.content_width == 0 || self.content_height == 0 {
            return;
        }
        let rect_right = (self.rect_x + self.rect_width).min(plane_width);
        let rect_bottom = (self.rect_y + self.rect_height).min(plane_height);
        let content_right = self.content_x + self.content_width;
        let content_bottom = self.content_y + self.content_height;
        for y in self.rect_y..rect_bottom {
            let sample_y = y.clamp(self.content_y, content_bottom - 1);
            for x in self.rect_x..rect_right {
                if (self.content_x..content_right).contains(&x) && (self.content_y..content_bottom).contains(&y) {
                    continue;
                }
                let sample_x = x.clamp(self.content_x, content_right - 1);
                plane[y as usize * plane_width as usize + x as usize] =
                    plane[sample_y as usize * plane_width as usize + sample_x as usize];
            }
        }
    }
}

/// Window of a decoded frame that goes into the atlas (the transparent trim).
#[derive(Debug, Clone, Copy)]
struct FrameWindow {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

impl FrameWindow {
    fn full(width: u16, height: u16) -> Self {
        Self {
            left: 0,
            top: 0,
            width: u32::from(width),
            height: u32::from(height),
        }
    }
}

/// Frame-sized hue mask of a `FRAME_HUE_MASKED` frame placed at `extrusion`.
struct PlacedHueMask {
    extrusion: RectExtrusion,
    mask: Vec<u8>,
}

/// `FRAME_HUE_*` of a frame window: which coded (alpha > 0) pixels the hue mask marks.
fn frame_window_hue_mode(rgba: &[u8], hue_mask: &[u8], source_width: u32, window: FrameWindow) -> u8 {
    if hue_mask.is_empty() {
        return FRAME_HUE_NONE;
    }
    let mut hueable = false;
    let mut kept = false;
    for row in window.top..window.top + window.height {
        for column in window.left..window.left + window.width {
            let pixel = row as usize * source_width as usize + column as usize;
            if rgba[pixel * RGBA_CHANNELS + RGBA_ALPHA_CHANNEL] == 0 {
                continue;
            }
            if hue_mask[pixel] == HUE_MASK_HUEABLE {
                hueable = true;
            } else {
                kept = true;
            }
            if hueable && kept {
                return FRAME_HUE_MASKED;
            }
        }
    }
    if hueable { FRAME_HUE_ALL } else { FRAME_HUE_NONE }
}

fn crop_hue_mask_window(frame: &DecodedFrame, window: FrameWindow) -> eyre::Result<Vec<u8>> {
    let source_width = frame.width as usize;
    if frame.hue_mask.len() != source_width * frame.height as usize
        || window.left + window.width > u32::from(frame.width)
        || window.top + window.height > u32::from(frame.height)
    {
        eyre::bail!(
            "EC mobile animation hue mask crop {},{} {}x{} is outside source {}x{}",
            window.left,
            window.top,
            window.width,
            window.height,
            frame.width,
            frame.height
        );
    }
    let mut cropped = Vec::with_capacity(window.width as usize * window.height as usize);
    for row in window.top as usize..(window.top + window.height) as usize {
        let start = row * source_width + window.left as usize;
        cropped.extend_from_slice(&frame.hue_mask[start..start + window.width as usize]);
    }
    Ok(cropped)
}

/// Scales a hue mask to the upscaled frame size, nearest neighbour (the mask is binary).
fn resample_hue_mask_nearest(
    mask: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    if source_width == width && source_height == height {
        return mask.to_vec();
    }
    let mut scaled = Vec::with_capacity(width as usize * height as usize);
    for y in 0..u64::from(height) {
        let source_y = (y * u64::from(source_height) / u64::from(height)) as usize;
        for x in 0..u64::from(width) {
            let source_x = (x * u64::from(source_width) / u64::from(width)) as usize;
            scaled.push(mask[source_y * source_width as usize + source_x]);
        }
    }
    scaled
}

fn set_placement_hue_mode(
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    body_id: u32,
    source_frame_index: u16,
    hue_mode: u8,
) {
    if let Some(placement) = placements.get_mut(&(body_id, source_frame_index)) {
        placement.hue_mode = hue_mode;
    }
}

/// R8 hue mask of a page's used area holding the masks of its `FRAME_HUE_MASKED` frames (and
/// their gutters); `None` when the page has none.
fn build_page_hue_mask(
    used_width: u32,
    used_height: u32,
    placed: Vec<PlacedHueMask>,
) -> eyre::Result<Option<Vec<u8>>> {
    if placed.is_empty() {
        return Ok(None);
    }
    let mut page_mask = vec![0u8; used_width as usize * used_height as usize];
    for placed in placed {
        let extrusion = placed.extrusion;
        let (width, height) = (extrusion.content_width as usize, extrusion.content_height as usize);
        if placed.mask.len() != width * height
            || extrusion.content_x + extrusion.content_width > used_width
            || extrusion.content_y + extrusion.content_height > used_height
        {
            eyre::bail!(
                "EC mobile animation hue mask {}x{} does not fit its atlas rect at {},{}",
                extrusion.content_width,
                extrusion.content_height,
                extrusion.content_x,
                extrusion.content_y
            );
        }
        for row in 0..height {
            let start = (extrusion.content_y as usize + row) * used_width as usize + extrusion.content_x as usize;
            page_mask[start..start + width].copy_from_slice(&placed.mask[row * width..(row + 1) * width]);
        }
        extrusion.extrude_r8(&mut page_mask, used_width, used_height);
    }
    Ok(Some(page_mask))
}

fn page_flags(hue_mask: &Option<Vec<u8>>) -> u8 {
    if hue_mask.is_some() { PAGE_FLAG_HUE_MASK } else { 0 }
}

fn packing_axes(
    frame: &DecodedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    packing_axes_for_dimensions(
        frame.body_id,
        frame.source_frame_index,
        frame.width,
        frame.height,
        page_size,
        options,
    )
}

fn planned_packing_axes(
    frame: &PlannedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    packing_axes_for_dimensions(
        frame.body_id,
        frame.source_frame_index,
        frame.width,
        frame.height,
        page_size,
        options,
    )
}

fn packing_axes_for_dimensions(
    body_id: u32,
    source_frame_index: u16,
    width: u16,
    height: u16,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
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
            "EC mobile animation frame body {} frame {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
            body_id,
            source_frame_index,
            width,
            height,
            page_size.width,
            page_size.height,
            options.gutter
        ),
    }
}

fn decoded_page_frames_with_axes(
    frames: Vec<DecodedMobileAnimEcFrame>,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
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
            page_frame.frame.body_id,
            page_frame.frame.source_frame_index,
        )
    });
    Ok(page_frames)
}

fn planned_page_frames_with_axes(
    frames: Vec<PlannedMobileAnimEcFrame>,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
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
            page_frame.frame.body_id,
            page_frame.frame.source_frame_index,
        )
    });
    Ok(page_frames)
}

fn prepare_decoded_frames(
    frames: Vec<DecodedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<Vec<DecodedMobileAnimEcFrame>> {
    frames.into_iter().map(trim_decoded_frame).collect()
}

fn trim_decoded_frame(frame: DecodedMobileAnimEcFrame) -> eyre::Result<DecodedMobileAnimEcFrame> {
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

    let width = (bounds.right - bounds.left) as u16;
    let height = (bounds.bottom - bounds.top) as u16;
    let center_x = adjusted_frame_center(
        frame.center_x,
        bounds.left as u32,
        "EC mobile animation frame center_x",
    )?;
    let center_y = adjusted_frame_center(
        frame.center_y,
        bounds.top as u32,
        "EC mobile animation frame center_y",
    )?;
    let body_id = frame.body_id;
    let source_frame_index = frame.source_frame_index;
    let hue_mask = if frame.hue_mask.is_empty() {
        Vec::new()
    } else {
        let source_width = usize::from(frame.width);
        (bounds.top..bounds.bottom)
            .flat_map(|row| &frame.hue_mask[row * source_width + bounds.left..row * source_width + bounds.right])
            .copied()
            .collect()
    };
    let rgba = crop_rgba_frame_window(
        frame.width,
        frame.height,
        frame.rgba,
        bounds.left as u32,
        bounds.top as u32,
        u32::from(width),
        u32::from(height),
    )?;

    Ok(DecodedMobileAnimEcFrame {
        body_id,
        source_frame_index,
        width,
        height,
        center_x,
        center_y,
        rgba,
        hue_mask,
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
            "invalid RGBA payload length for EC mobile animation trim {}x{}: expected {}, got {}",
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
            "EC mobile animation frame crop {},{} {}x{} is outside source {}x{}",
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
            "invalid RGBA payload length for EC mobile animation frame {}x{}: expected {}, got {}",
            source_width,
            source_height,
            expected_len,
            src.len()
        );
    }
    if source_left + frame_width > source_width || source_top + frame_height > source_height {
        eyre::bail!(
            "EC mobile animation frame crop {},{} {}x{} is outside source {}x{}",
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
        eyre::bail!("invalid EC mobile animation atlas row stride for width {}", dst_width);
    }
    let dst_height = dst.len() / dst_stride;
    if dst_x + frame_width > dst_width || dst_y + frame_height > dst_height as u32 {
        eyre::bail!(
            "EC mobile animation frame destination {},{} {}x{} is outside atlas {}x{}",
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
    pages: &[BuiltMobileAnimEcPage],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let records = pages.iter().map(|page| page.record).collect::<Vec<_>>();
    serialize_page_record_manifest(&records, options)
}

fn serialize_page_record_manifest(
    records: &[MobileAnimEcPageRecord],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(PAGE_MANIFEST_HEADER_SIZE + records.len() * PAGE_RECORD_SIZE);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
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
        bytes.push(record.flags);
    }
    Ok(bytes)
}

pub fn serialize_animation_manifest(
    animations: &[MobileAnimEcAnimationRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + animations.len() * 37);
    bytes.extend_from_slice(&ANIMATION_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(animations.len() as u32)?;
    for animation in animations {
        bytes.write_u32::<LittleEndian>(animation.body_id)?;
        bytes.write_u16::<LittleEndian>(animation.action_id)?;
        bytes.push(animation.direction);
        bytes.write_u32::<LittleEndian>(animation.frame_start)?;
        bytes.write_u16::<LittleEndian>(animation.frame_count)?;
        bytes.write_u16::<LittleEndian>(animation.flags)?;
        bytes.write_u32::<LittleEndian>(animation.source_body_id)?;
        bytes.write_u16::<LittleEndian>(animation.resolved_action_id)?;
        bytes.write_u16::<LittleEndian>(animation.alias_action_id)?;
        bytes.write_u16::<LittleEndian>(animation.resolved_action_id_no_flying)?;
        bytes.write_f32::<LittleEndian>(animation.fps)?;
        bytes.write_f32::<LittleEndian>(animation.fps_no_flying)?;
        bytes.write_u32::<LittleEndian>(animation.skipped_frame_mask)?;
    }
    Ok(bytes)
}

pub fn serialize_frame_manifest(frames: &[MobileAnimEcFrameRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MANIFEST_HEADER_SIZE + frames.len() * FRAME_RECORD_SIZE);
    bytes.extend_from_slice(&FRAME_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(frames.len() as u32)?;
    for frame in frames {
        bytes.write_u32::<LittleEndian>(frame.animation_index)?;
        bytes.write_u16::<LittleEndian>(frame.frame_index)?;
        bytes.write_u16::<LittleEndian>(frame.source_frame_index)?;
        bytes.write_u16::<LittleEndian>(frame.source_action)?;
        bytes.write_u16::<LittleEndian>(frame.source_frame_number)?;
        bytes.write_u32::<LittleEndian>(frame.page_index)?;
        bytes.write_u16::<LittleEndian>(frame.page_frame_index)?;
        bytes.write_u16::<LittleEndian>(frame.x)?;
        bytes.write_u16::<LittleEndian>(frame.y)?;
        bytes.write_u16::<LittleEndian>(frame.width)?;
        bytes.write_u16::<LittleEndian>(frame.height)?;
        bytes.write_i16::<LittleEndian>(frame.center_x)?;
        bytes.write_i16::<LittleEndian>(frame.center_y)?;
        bytes.push(frame.hue_mode);
        bytes.push(frame.shadow.min_inverse);
        bytes.push(frame.shadow.mean_inverse);
        bytes.push(frame.shadow.max_inverse);
    }
    Ok(bytes)
}

pub fn serialize_item_manifest(items: &[MobileAnimEcItemRecord]) -> eyre::Result<Vec<u8>> {
    let mut strings = Vec::new();
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let name = item.name.as_bytes();
        if name.len() > u16::MAX as usize {
            eyre::bail!("EC mobile animation item {} name is too long", item.item_id);
        }
        let start = strings.len();
        strings.extend_from_slice(name);
        names.push((start as u32, name.len() as u16));
    }

    let mut bytes = Vec::with_capacity(16 + items.len() * 23 + strings.len());
    bytes.extend_from_slice(&ITEM_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(items.len() as u32)?;
    bytes.write_u32::<LittleEndian>(strings.len() as u32)?;
    for (item, (name_start, name_len)) in items.iter().zip(names) {
        bytes.write_i32::<LittleEndian>(item.item_id)?;
        bytes.write_i16::<LittleEndian>(item.item_type)?;
        bytes.write_i16::<LittleEndian>(item.layer)?;
        bytes.push(item.flags);
        bytes.write_u32::<LittleEndian>(name_start)?;
        bytes.write_u16::<LittleEndian>(name_len)?;
        bytes.write_u32::<LittleEndian>(item.source_hint_start)?;
        bytes.write_u16::<LittleEndian>(item.source_hint_count)?;
    }
    bytes.extend_from_slice(&strings);
    Ok(bytes)
}

pub fn serialize_source_hint_manifest(source_hints: &[MobileAnimEcSourceHintRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + source_hints.len() * 15);
    bytes.extend_from_slice(&SOURCE_HINT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(source_hints.len() as u32)?;
    for source_hint in source_hints {
        bytes.write_i32::<LittleEndian>(source_hint.item_id)?;
        bytes.write_u16::<LittleEndian>(source_hint.action_id)?;
        bytes.push(source_hint.uop_index);
        bytes.write_u32::<LittleEndian>(source_hint.block_index)?;
        bytes.write_u32::<LittleEndian>(source_hint.file_index)?;
    }
    Ok(bytes)
}

/// `metadata/bodies.bin` ("MEBD"): `MobileAnimEcBodyRecord`s sorted by body id.
pub fn serialize_body_manifest(bodies: &[MobileAnimEcBodyRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MANIFEST_HEADER_SIZE + bodies.len() * BODY_RECORD_SIZE);
    bytes.extend_from_slice(&BODY_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(bodies.len() as u32)?;
    for body in bodies {
        bytes.write_u32::<LittleEndian>(body.body_id)?;
        bytes.write_u32::<LittleEndian>(body.sequence_id)?;
        bytes.write_u32::<LittleEndian>(body.frames_body_id)?;
        bytes.write_u32::<LittleEndian>(body.appearance_body_id)?;
        bytes.write_u16::<LittleEndian>(body.forced_hue)?;
        bytes.write_u16::<LittleEndian>(body.flags)?;
        bytes.write_u32::<LittleEndian>(body.sequence_stored_id)?;
        bytes.push(body.race);
        bytes.push(body.gender);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::MobileAnimEcPackage;
    use udd_container::{UddpBuilder, UddpReader};

    fn frame(body_id: u32, source_frame_index: u16, width: u16, height: u16) -> DecodedMobileAnimEcFrame {
        DecodedMobileAnimEcFrame {
            body_id,
            source_frame_index,
            width,
            height,
            center_x: 1,
            center_y: -1,
            rgba: vec![255u8; width as usize * height as usize * 4],
            hue_mask: Vec::new(),
        }
    }

    /// Opaque frame whose left half takes the hue.
    fn half_hued_frame(body_id: u32, source_frame_index: u16, width: u16, height: u16) -> DecodedMobileAnimEcFrame {
        let mut decoded = frame(body_id, source_frame_index, width, height);
        decoded.hue_mask = (0..usize::from(height))
            .flat_map(|_| (0..usize::from(width)).map(|x| if x < usize::from(width) / 2 { HUE_MASK_HUEABLE } else { 0 }))
            .collect();
        decoded
    }

    fn sparse_frame(body_id: u32, source_frame_index: u16) -> DecodedMobileAnimEcFrame {
        let width = 6u16;
        let height = 6u16;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        for y in 2..5usize {
            for x in 1..4usize {
                let index = (y * width as usize + x) * 4;
                rgba[index..index + 4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        DecodedMobileAnimEcFrame {
            body_id,
            source_frame_index,
            width,
            height,
            center_x: 4,
            center_y: 5,
            rgba,
            hue_mask: Vec::new(),
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("udd_mobile_anim_ec_{}_{}", name, nanos))
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

    /// No table given: the shipped one. A given table must exist, even when the options allow packing without one.
    #[test]
    fn mobile_animation_kdl_defaults_to_the_shipped_table() {
        let shipped = EC_MOBILE_ANIMATIONS_KDL.checkout_dir().join(EC_MOBILE_ANIMATIONS_KDL.relative_path());
        let options = MobileAnimEcAtlasOptions::default();
        assert_eq!(resolve_ec_mobile_animations_kdl(&options).unwrap(), Some(shipped));

        let missing = temp_test_dir("tables").join("EcMobileAnimations.kdl");
        let options = MobileAnimEcAtlasOptions { metadata_path: Some(missing.clone()), ..Default::default() };
        let error = resolve_ec_mobile_animations_kdl(&options).unwrap_err().to_string();
        assert!(error.contains(&missing.display().to_string()), "{error}");
        let options = MobileAnimEcAtlasOptions { allow_missing_metadata: true, ..options };
        let error = resolve_ec_mobile_animations_kdl(&options).unwrap_err().to_string();
        assert!(error.contains(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn packer_uses_four_pixel_aligned_extents() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 0,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };

        let (pages, placements) = pack_frames_into_pages(
            vec![frame(42, 0, 5, 5), frame(42, 1, 5, 5)],
            &options,
        )
        .unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(placements[&(42, 0)].x % 4, 0);
        assert_eq!(placements[&(42, 1)].x % 4, 0);
    }

    #[test]
    fn packer_retains_extruded_filter_gutter() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };

        let (pages, placements) = pack_frames_into_pages(vec![frame(42, 0, 5, 5)], &options).unwrap();

        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page.record.used_width, 16);
        assert_eq!(page.record.used_height, 16);
        let placed = placements[&(42, 0)];
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
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 0,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };

        let (pages, placements) = pack_frames_into_pages(vec![sparse_frame(42, 0)], &options).unwrap();
        let placement = placements[&(42, 0)];

        assert_eq!(pages.len(), 1);
        assert_eq!(placement.width, 3);
        assert_eq!(placement.height, 3);
        assert_eq!(placement.center_x, 3);
        assert_eq!(placement.center_y, 3);
        assert_eq!(pages[0].record.used_width % 4, 0);
        assert_eq!(pages[0].record.used_height % 4, 0);
    }

    #[test]
    fn packer_uses_smallest_effective_bucket() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 2048,
            atlas_height: 2048,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };

        let (pages, placements) = pack_frames_into_pages(vec![frame(42, 0, 16, 16)], &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].record.atlas_width, 512);
        assert_eq!(pages[0].record.atlas_height, 512);
        assert_eq!(placements[&(42, 0)].page_index, 0);
    }

    /// Server body 146 draws with the sequence and frames of body 124 (its tileart placeholder);
    /// the sequence stored id (an equipment wear-check id) plays no part.
    const SEQUENCE_BODY: u32 = 146;
    const SEQUENCE_ID: u32 = 124;
    const FRAMES_BODY: u32 = 124;
    const SEQUENCE_STORED_ID: u32 = 704;
    const NO_SEQUENCE_BODY: u32 = 42;
    const HUMAN_MALE: u32 = 400;
    const HUMAN_FEMALE: u32 = 401;
    const MALE_GHOST: u32 = 402;
    /// Placeholder whose appearance has no frames.
    const FRAMELESS_APPEARANCE_BODY: u32 = 433;
    const FALLBACK_BODY: u32 = uocf::enhanced::mobile_body::FALLBACK_FRAMES_BODY;
    const GHOST_HUE: u16 = uocf::enhanced::mobile_body::GHOST_HUE;

    /// `(body, sequence_id, appearance)` placeholders.
    fn body_map(placeholders: &[(u32, u32, u32)]) -> MobileBodyMap {
        MobileBodyMap {
            definitions: placeholders
                .iter()
                .map(|&(body, sequence_id, appearance)| {
                    (body, MobileBodyDefinition {
                        body,
                        sequence_id,
                        appearance,
                        animation_appearance: None,
                        race: None,
                        gender: None,
                    })
                })
                .collect(),
        }
    }

    fn sample_body_map() -> MobileBodyMap {
        body_map(&[(SEQUENCE_BODY, SEQUENCE_ID, FRAMES_BODY)])
    }
    fn source(source_frame_index: u16, action: u16, frame_number: u16) -> SourceFrameIdentity {
        SourceFrameIdentity { source_frame_index, action, frame_number }
    }

    /// Source frames numbered `numbers` for `action`, appended to `frames` in entry order.
    fn push_sources(frames: &mut Vec<SourceFrameIdentity>, action: u16, numbers: impl IntoIterator<Item = u16>) {
        for number in numbers {
            let index = frames.len() as u16;
            frames.push(source(index, action, number));
        }
    }

    /// `(action, frame_count, alias, fps, skipped frames)` in file order; `stand` is the stand
    /// action of the peace group.
    fn sequence(stored_id: u32, actions: &[(u32, u32, Option<u32>, f32, &[u32])], stand: u32) -> AnimationSequence {
        let mut next_base_frame = uocf::animation_sequence::FIRST_BASE_FRAME;
        let actions = actions
            .iter()
            .map(|&(action, frame_count, alias, fps, skipped)| {
                let base_frame = if alias.is_none() {
                    let base_frame = next_base_frame;
                    next_base_frame += STORED_DIRECTION_COUNT * frame_count;
                    base_frame
                } else {
                    0
                };
                (action, SequenceAction {
                    frame_count,
                    alias,
                    fps,
                    offsets: Default::default(),
                    extra_offsets: Default::default(),
                    frame_effects: BTreeMap::new(),
                    skipped_frames: skipped.iter().copied().collect(),
                    base_frame,
                    kr_extra: None,
                })
            })
            .collect();
        let group = uocf::animation_sequence::ActionGroup {
            parent: None,
            default_action: stand,
            transitions: BTreeMap::new(),
            moves: BTreeMap::from([(
                uocf::animation_sequence::MOVE_STAND,
                uocf::animation_sequence::MoveActions { action: stand, alternate: stand },
            )]),
            typed: BTreeMap::new(),
        };
        AnimationSequence {
            stored_id,
            offsets: Default::default(),
            extra_offsets: Default::default(),
            actions,
            groups: BTreeMap::from([(uocf::animation_sequence::GROUP_PEACE, group)]),
            repeated_actions: Vec::new(),
        }
    }

    /// Sequence 124: plain 3 (1 frame), 4 (2 frames, fps 12, skipped frame 0, stand) and 19
    /// (1 frame); alias 0 -> 4, 1 -> 4, 2 -> 1 (alias of an alias).
    fn sample_sequences() -> BTreeMap<u32, AnimationSequence> {
        BTreeMap::from([(
            SEQUENCE_ID,
            sequence(
                SEQUENCE_STORED_ID,
                &[
                    (0, 0, Some(4), 0.0, &[]),
                    (1, 0, Some(4), 0.0, &[]),
                    (2, 0, Some(1), 0.0, &[]),
                    (3, 1, None, 9.0, &[]),
                    (4, 2, None, 12.0, &[0]),
                    (19, 1, None, 9.0, &[]),
                ],
                4,
            ),
        )])
    }

    fn sample_sequence_frames() -> BTreeMap<u32, Vec<SourceFrameIdentity>> {
        let mut frames = Vec::new();
        push_sources(&mut frames, 3, 1..=5);
        push_sources(&mut frames, 4, 1..=10);
        push_sources(&mut frames, 19, 1..=5);
        BTreeMap::from([(FRAMES_BODY, frames)])
    }

    fn record<'a>(
        layout: &'a AnimationLayout,
        body_id: u32,
        action_id: u16,
        direction: u8,
    ) -> (usize, &'a MobileAnimEcAnimationRecord) {
        layout
            .animations
            .iter()
            .enumerate()
            .find(|(_, record)| {
                record.body_id == body_id && record.action_id == action_id && record.direction == direction
            })
            .expect("animation record exists")
    }

    fn slot_numbers(layout: &AnimationLayout, record: &MobileAnimEcAnimationRecord) -> Vec<(u16, u16)> {
        let start = record.frame_start as usize;
        layout.slots[start..start + usize::from(record.frame_count)]
            .iter()
            .map(|slot| (slot.source_action, slot.source_frame_number))
            .collect()
    }

    #[test]
    fn sequence_layout_emits_stored_mirrored_and_alias_records() {
        let layout = build_animation_layout(&sample_sequence_frames(), &sample_sequences(), &sample_body_map()).unwrap();

        // Body 124 (no placeholder: its own sequence and frames) and body 146 share one set.
        let body_record_count = 6 * usize::from(DIRECTION_COUNT);
        assert_eq!(layout.animations.len(), 2 * body_record_count);
        assert!(layout.animations.windows(2).all(|pair| {
            (pair[0].body_id, pair[0].action_id, pair[0].direction)
                < (pair[1].body_id, pair[1].action_id, pair[1].direction)
        }));
        assert_eq!(layout.stats.sequence_body_count, 2);
        assert_eq!(layout.stats.placeholder_body_count, 1);
        assert_eq!(layout.stats.unmapped_body_count, 1);
        assert_eq!(layout.stats.shared_animation_set_body_count, 1);
        assert_eq!(layout.stats.plain_action_count, 3);
        assert_eq!(layout.stats.alias_action_count, 2 * 3);
        assert_eq!(layout.stats.mirrored_record_count, 2 * 6 * 3);
        assert_eq!(layout.stats.unreferenced_frame_count, 0);
        assert_eq!(
            layout.bodies,
            [
                MobileAnimEcBodyRecord {
                    body_id: FRAMES_BODY,
                    sequence_id: SEQUENCE_ID,
                    frames_body_id: FRAMES_BODY,
                    appearance_body_id: FRAMES_BODY,
                    forced_hue: NO_FORCED_HUE,
                    flags: BODY_FLAG_NO_PLACEHOLDER,
                    sequence_stored_id: SEQUENCE_STORED_ID,
                    race: 0,
                    gender: 0,
                },
                MobileAnimEcBodyRecord {
                    body_id: SEQUENCE_BODY,
                    sequence_id: SEQUENCE_ID,
                    frames_body_id: FRAMES_BODY,
                    appearance_body_id: FRAMES_BODY,
                    forced_hue: NO_FORCED_HUE,
                    flags: 0,
                    sequence_stored_id: SEQUENCE_STORED_ID,
                    race: 0,
                    gender: 0,
                },
            ]
        );
        let (_, shared) = record(&layout, FRAMES_BODY, 4, 4);

        // Stored direction: frames numbered from (direction - 3) * frame_count + 1.
        let (owner_index, south) = record(&layout, SEQUENCE_BODY, 4, 4);
        assert_eq!(shared.frame_start, south.frame_start);
        assert_eq!(south.source_body_id, FRAMES_BODY);
        assert_eq!(south.frame_count, 2);
        assert_eq!(slot_numbers(&layout, south), [(4, 3), (4, 4)]);
        assert!(!south.is_mirrored());
        assert_eq!((south.fps, south.skipped_frame_mask), (12.0, 1));
        assert_ne!(south.flags & ANIMATION_FLAG_STAND_ACTION, 0);
        let start = south.frame_start as usize;
        // The first body laid out (124) owns the shared slots.
        assert_ne!(owner_index, record(&layout, FRAMES_BODY, 4, 4).0);
        let shared_owner_index = record(&layout, FRAMES_BODY, 4, 4).0;
        assert!(layout.slots[start..start + 2]
            .iter()
            .enumerate()
            .all(|(index, slot)| slot.animation_index == Some(shared_owner_index as u32) && slot.frame_index == index as u16));

        // Directions 0..2 mirror 6..4 and share their frames.
        let (_, east) = record(&layout, SEQUENCE_BODY, 4, 2);
        assert!(east.is_mirrored());
        assert_eq!(east.frame_start, south.frame_start);
        let (_, north) = record(&layout, SEQUENCE_BODY, 4, 0);
        let (_, west) = record(&layout, SEQUENCE_BODY, 4, 6);
        assert_eq!((north.frame_start, north.is_mirrored()), (west.frame_start, true));
        assert_eq!(slot_numbers(&layout, west), [(4, 7), (4, 8)]);

        // Aliases point at the resolved action's frames and keep the raw alias.
        let (_, alias) = record(&layout, SEQUENCE_BODY, 0, 7);
        let (_, target) = record(&layout, SEQUENCE_BODY, 4, 7);
        assert_eq!((alias.frame_start, alias.frame_count), (target.frame_start, target.frame_count));
        assert_eq!((alias.alias_action_id, alias.resolved_action_id), (4, 4));
        assert_eq!((alias.fps, alias.skipped_frame_mask), (12.0, 0));
        assert!(alias.is_alias());

        // An alias of an alias falls back to the lowest plain action.
        let (_, chained) = record(&layout, SEQUENCE_BODY, 2, 3);
        assert_eq!((chained.alias_action_id, chained.resolved_action_id), (1, 3));
        assert_eq!(slot_numbers(&layout, chained), [(3, 1)]);

        // Flying action: itself when enabled; 0 is an alias, so the lowest plain action when not.
        let (_, flying) = record(&layout, SEQUENCE_BODY, 19, 3);
        assert_eq!((flying.resolved_action_id, flying.resolved_action_id_no_flying), (19, 3));
        assert_eq!((flying.fps, flying.fps_no_flying), (9.0, 9.0));
        assert_eq!(flying.alias_action_id, NO_ALIAS_ACTION);
    }

    #[test]
    fn sequence_layout_handles_gaps_count_mismatches_and_missing_entries() {
        let sequences = BTreeMap::from([(
            SEQUENCE_ID,
            sequence(SEQUENCE_STORED_ID, &[(23, 2, None, 9.0, &[]), (24, 3, None, 9.0, &[])], 23),
        )]);
        let mut frames = Vec::new();
        // Action 23: number 1 missing, 5 repeated, 11 beyond 5 * 2; action 30 is not in the sequence.
        push_sources(&mut frames, 23, [2, 3, 4, 5, 5, 6, 7, 8, 9, 10, 11]);
        push_sources(&mut frames, 30, [1, 2]);
        let first_number_five = frames[3].source_frame_index;
        let layout =
            build_animation_layout(&BTreeMap::from([(FRAMES_BODY, frames)]), &sequences, &sample_body_map()).unwrap();

        assert_eq!(layout.stats.frame_count_mismatch_action_count, 1);
        assert_eq!(layout.stats.missing_frame_count, 1);
        assert_eq!(layout.stats.plain_action_without_frames_count, 1);
        assert_eq!(layout.stats.unreferenced_frame_count, 4);

        let (_, first_direction) = record(&layout, SEQUENCE_BODY, 23, 3);
        let start = first_direction.frame_start as usize;
        assert_eq!(layout.slots[start].source_frame_index, None);
        assert_eq!(layout.slots[start].source_frame_number, 1);
        assert_eq!(layout.slots[start + 1].source_frame_number, 2);
        let (_, third_direction) = record(&layout, SEQUENCE_BODY, 23, 5);
        assert_eq!(layout.slots[third_direction.frame_start as usize].source_frame_index, Some(first_number_five));
        assert!(layout.slots.iter().all(|slot| slot.animation_index.is_some()));

        let (_, without_frames) = record(&layout, SEQUENCE_BODY, 24, 5);
        assert_eq!(without_frames.frame_count, 0);

        let (_, _, frames) = layout.into_records(&HashMap::new(), &HashMap::new()).unwrap();
        assert_eq!(frames.len(), 10);
        assert_eq!(frames[0].source_frame_index, MISSING_SOURCE_FRAME_INDEX);
        assert_eq!((frames[0].source_action, frames[0].source_frame_number), (23, 1));
        assert_eq!(frames[0].page_index, MISSING_PAGE_INDEX);
    }

    #[test]
    fn no_sequence_layout_derives_directions_or_falls_back_undirected() {
        // Body 600 has a sequence but no frames and no placeholder: no records.
        let frameless_body = 600;
        let sequences = BTreeMap::from([(frameless_body, sequence(601, &[(0, 1, None, 9.0, &[])], 0))]);
        let mut fallback = Vec::new();
        push_sources(&mut fallback, 0, [1, 2, 3, 5, 6, 7, 8, 9, 10]);
        push_sources(&mut fallback, 1, [1, 2, 3]);
        let frames_by_body = BTreeMap::from([(NO_SEQUENCE_BODY, fallback)]);
        let layout = build_animation_layout(&frames_by_body, &sequences, &MobileBodyMap::default()).unwrap();

        assert_eq!(layout.stats.no_sequence_body_count, 1);
        assert_eq!(layout.stats.sequence_without_frames_count, 1);
        assert_eq!(layout.stats.fallback_directional_action_count, 1);
        assert_eq!(layout.stats.fallback_undirected_action_count, 1);
        assert_eq!(layout.stats.missing_frame_count, 1);
        assert_eq!(layout.stats.unreferenced_frame_count, 0);
        assert!(layout.animations.iter().all(|record| record.body_id == NO_SEQUENCE_BODY));
        assert_eq!(layout.bodies.len(), 1);
        assert_eq!(layout.bodies[0].flags, BODY_FLAG_NO_PLACEHOLDER | BODY_FLAG_NO_SEQUENCE);

        let (_, west) = record(&layout, NO_SEQUENCE_BODY, 0, 6);
        let (_, north) = record(&layout, NO_SEQUENCE_BODY, 0, 0);
        assert_eq!(west.flags, ANIMATION_FLAG_NO_SEQUENCE);
        assert_eq!(north.flags, ANIMATION_FLAG_NO_SEQUENCE | ANIMATION_FLAG_MIRRORED);
        assert_eq!(north.frame_start, west.frame_start);
        assert_eq!(slot_numbers(&layout, west), [(0, 7), (0, 8)]);
        assert_eq!((west.fps, west.resolved_action_id), (MISSING_ACTION_FPS, 0));

        for direction in 0..DIRECTION_COUNT {
            let (_, undirected) = record(&layout, NO_SEQUENCE_BODY, 1, direction);
            assert_eq!(undirected.flags, ANIMATION_FLAG_NO_SEQUENCE | ANIMATION_FLAG_UNDIRECTED);
            assert_eq!(slot_numbers(&layout, undirected), [(1, 1), (1, 2), (1, 3)]);
        }
        assert!(layout.slots.iter().all(|slot| slot.animation_index.is_some()));
    }

    #[test]
    fn serialized_package_roundtrips_through_reader() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };
        let mut frames_by_body = sample_sequence_frames();
        frames_by_body.insert(NO_SEQUENCE_BODY, vec![source(0, 0, 1)]);
        let layout = build_animation_layout(&frames_by_body, &sample_sequences(), &sample_body_map()).unwrap();
        let masked_frame = half_hued_frame(NO_SEQUENCE_BODY, 0, 4, 4);
        let expected_mask = masked_frame.hue_mask.clone();
        let (mut pages, placements) = pack_frames_into_pages(vec![masked_frame], &options).unwrap();
        let shadow = ShadowPaletteStats {
            min_inverse: 10,
            mean_inverse: 100,
            max_inverse: 200,
        };
        let shadow_by_source = HashMap::from([((NO_SEQUENCE_BODY, 0), shadow)]);
        let (bodies, animations, frames) = layout.into_records(&placements, &shadow_by_source).unwrap();
        let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
        let animation_manifest = serialize_animation_manifest(&animations).unwrap();
        let frame_manifest = serialize_frame_manifest(&frames).unwrap();
        let body_manifest = serialize_body_manifest(&bodies).unwrap();
        let hue_mask_path = hue_mask_entry_path(0);
        let stored_hue_mask = pages[0].hue_mask.take().expect("page with a masked frame has a hue mask");
        let (items, source_hints) = build_item_metadata(&EcMobileAnimationsKdl {
            items: vec![udd_assets::mobile_anim_ec::EcMobileAnimationItemKdl {
                id: 42,
                name: "Test Body".to_string(),
                item_type: 3,
                layer: 0,
                male: Some(true),
                female: None,
                gargoyle: None,
                animations: vec![udd_assets::mobile_anim_ec::EcMobileAnimationSourceHintKdl {
                    action_id: 0,
                    uop: "AnimationFrame1.uop".to_string(),
                    block: 2,
                    file: 7,
                }],
            }],
        }).unwrap();
        let item_manifest = serialize_item_manifest(&items).unwrap();
        let source_hint_manifest = serialize_source_hint_manifest(&source_hints).unwrap();
        let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
        let stored_page = pages[0].pixels.clone();

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice(), DataType::Metadata),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice(), DataType::Metadata),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice(), DataType::Metadata),
            (ITEM_MANIFEST_ENTRY_PATH, item_manifest.as_slice(), DataType::Metadata),
            (SOURCE_HINT_MANIFEST_ENTRY_PATH, source_hint_manifest.as_slice(), DataType::Metadata),
            (BODY_MANIFEST_ENTRY_PATH, body_manifest.as_slice(), DataType::Metadata),
            (page_path.as_str(), stored_page.as_slice(), DataType::Texture),
            (hue_mask_path.as_str(), stored_hue_mask.as_slice(), DataType::Texture),
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

        let package = MobileAnimEcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(package.pages().len(), 1);
        assert!(package.pages()[0].has_hue_mask());
        assert_eq!(package.animations(), animations.as_slice());
        assert_eq!(package.frames(), frames.as_slice());
        assert_eq!(package.bodies(), bodies.as_slice());
        let serpent = package.body(SEQUENCE_BODY).unwrap();
        assert_eq!((serpent.sequence_id, serpent.frames_body_id), (SEQUENCE_ID, FRAMES_BODY));
        let undirected = package.animation(NO_SEQUENCE_BODY, 0, 0).unwrap();
        assert_eq!(undirected.frame_count, 1);
        let undirected_frame = &package.animation_frames(undirected)[0];
        assert_eq!(undirected_frame.center_y, -1);
        assert_eq!(undirected_frame.hue_mode, FRAME_HUE_MASKED);
        assert_eq!(undirected_frame.shadow, shadow);
        assert_eq!(package.read_frame_hue_mask(undirected_frame).unwrap(), Some(expected_mask));

        // Client resolution: missing actions use the lowest plain action; flying disabled
        // uses the no-flying resolution and fps.
        let missing = package.resolve_animation(SEQUENCE_BODY, 50, 3, true).unwrap();
        assert_eq!((missing.record.action_id, missing.fps), (3, 9.0));
        let grounded = package.resolve_animation(SEQUENCE_BODY, 19, 1, false).unwrap();
        assert_eq!((grounded.record.action_id, grounded.record.direction), (3, 1));
        assert!(grounded.record.is_mirrored());
        let flying = package.resolve_animation(SEQUENCE_BODY, 19, 1, true).unwrap();
        assert_eq!(flying.record.action_id, 19);

        assert_eq!(package.item(42).unwrap().name, "Test Body");
        assert_eq!(package.item_source_hints(package.item(42).unwrap())[0].file_index, 7);
        assert_eq!(package.read_page_bytes(0).unwrap().len(), stored_page.len());
    }

    #[test]
    fn animation_records_preserve_unpacked_empty_frames() {
        let source_frames = vec![
            frame(NO_SEQUENCE_BODY, 0, 4, 4),
            DecodedMobileAnimEcFrame {
                body_id: NO_SEQUENCE_BODY,
                source_frame_index: 1,
                width: 0,
                height: 0,
                center_x: 0,
                center_y: 0,
                rgba: Vec::new(),
                hue_mask: Vec::new(),
            },
        ];
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };
        let (_, placements) = pack_frames_into_pages(source_frames, &options).unwrap();
        let layout = build_animation_layout(
            &BTreeMap::from([(NO_SEQUENCE_BODY, vec![source(0, 0, 1), source(1, 0, 2)])]),
            &BTreeMap::new(),
            &MobileBodyMap::default(),
        )
        .unwrap();
        let (_, animations, frames) = layout.into_records(&placements, &HashMap::new()).unwrap();

        assert_eq!(animations[0].frame_count, 2);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].source_frame_index, 1);
        assert_eq!(frames[1].page_index, MISSING_PAGE_INDEX);
        assert_eq!(frames[1].page_frame_index, MISSING_PAGE_FRAME_INDEX);
    }

    #[test]
    fn appending_duplicate_body_frames_preserves_all_frames() {
        let mut decoded_by_body = BTreeMap::new();

        append_decoded_body_frames(&mut decoded_by_body, 42, vec![frame(42, 0, 4, 4)]).unwrap();
        append_decoded_body_frames(
            &mut decoded_by_body,
            42,
            vec![frame(42, 0, 5, 5), frame(42, 1, 6, 6)],
        )
        .unwrap();

        let frames = &decoded_by_body[&42];
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].source_frame_index, 0);
        assert_eq!(frames[1].source_frame_index, 1);
        assert_eq!(frames[2].source_frame_index, 2);
        assert_eq!(frames[1].width, 5);
        assert_eq!(frames[2].width, 6);
    }

    #[test]
    fn placeholder_layout_shares_frames_falls_back_to_body_zero_and_flags_ghosts() {
        let human = sequence(HUMAN_MALE, &[(4, 1, None, 9.0, &[])], 4);
        let sequences = BTreeMap::from([(HUMAN_MALE, human)]);
        let frames_by_body = BTreeMap::from([
            (FALLBACK_BODY, vec![source(0, 4, 1)]),
            (HUMAN_MALE, (0..5).map(|index| source(index, 4, index + 1)).collect()),
            (HUMAN_FEMALE, (0..5).map(|index| source(index, 4, index + 1)).collect()),
        ]);
        let map = body_map(&[
            (HUMAN_MALE, HUMAN_MALE, HUMAN_MALE),
            (HUMAN_FEMALE, HUMAN_MALE, HUMAN_FEMALE),
            (MALE_GHOST, HUMAN_MALE, HUMAN_MALE),
            (FRAMELESS_APPEARANCE_BODY, HUMAN_MALE, FRAMELESS_APPEARANCE_BODY),
        ]);
        let layout = build_animation_layout(&frames_by_body, &sequences, &map).unwrap();

        assert_eq!(layout.stats.placeholder_body_count, 4);
        assert_eq!(layout.stats.unmapped_body_count, 1);
        assert_eq!(layout.stats.frames_fallback_body_count, 1);
        assert_eq!(layout.stats.ghost_body_count, 1);
        assert_eq!(layout.stats.shared_animation_set_body_count, 1);

        let body = |body_id| layout.bodies.iter().find(|body| body.body_id == body_id).unwrap();
        let ghost = body(MALE_GHOST);
        assert_eq!((ghost.frames_body_id, ghost.forced_hue), (HUMAN_MALE, GHOST_HUE));
        assert_eq!(ghost.flags, BODY_FLAG_GHOST);
        let female = body(HUMAN_FEMALE);
        assert_eq!((female.sequence_id, female.frames_body_id), (HUMAN_MALE, HUMAN_FEMALE));
        let fallback = body(FRAMELESS_APPEARANCE_BODY);
        assert_eq!((fallback.frames_body_id, fallback.appearance_body_id), (FALLBACK_BODY, FRAMELESS_APPEARANCE_BODY));
        assert_eq!(fallback.flags, BODY_FLAG_FRAMES_FALLBACK);
        // Body 0 has frames but neither a sequence nor a placeholder.
        assert_eq!(body(FALLBACK_BODY).flags, BODY_FLAG_NO_PLACEHOLDER | BODY_FLAG_NO_SEQUENCE);

        // The ghost reuses the frame records (and so the pixels) of body 400.
        let (_, male) = record(&layout, HUMAN_MALE, 4, 3);
        let (_, ghost_record) = record(&layout, MALE_GHOST, 4, 3);
        let (_, female_record) = record(&layout, HUMAN_FEMALE, 4, 3);
        assert_eq!((ghost_record.frame_start, ghost_record.source_body_id), (male.frame_start, HUMAN_MALE));
        assert_ne!(female_record.frame_start, male.frame_start);
        assert_eq!(female_record.source_body_id, HUMAN_FEMALE);
        let (_, fallback_record) = record(&layout, FRAMELESS_APPEARANCE_BODY, 4, 3);
        assert_eq!(fallback_record.source_body_id, FALLBACK_BODY);
        assert_eq!(slot_numbers(&layout, fallback_record), [(4, 1)]);
        assert!(layout.slots.iter().all(|slot| slot.animation_index.is_some()));
    }

    #[test]
    fn placeholder_race_and_gender_reach_the_body_record() {
        let elf_race = 2;
        let female_gender = 1;
        let mut body_map = sample_body_map();
        let definition = body_map.definitions.get_mut(&SEQUENCE_BODY).unwrap();
        definition.race = Some(elf_race);
        definition.gender = Some(female_gender);
        let layout = build_animation_layout(&sample_sequence_frames(), &sample_sequences(), &body_map).unwrap();
        let body = layout.bodies.iter().find(|body| body.body_id == SEQUENCE_BODY).unwrap();
        assert_eq!((body.race(), body.gender()), (Some(elf_race as u8), Some(female_gender as u8)));
        assert_eq!(body.sequence_stored_id, SEQUENCE_STORED_ID);

        body_map.definitions.get_mut(&SEQUENCE_BODY).unwrap().race = Some(u32::from(u8::MAX) + 1);
        assert!(build_animation_layout(&sample_sequence_frames(), &sample_sequences(), &body_map).is_err());
    }

    #[test]
    fn frame_hue_mode_follows_coded_pixels() {
        let opaque = frame(NO_SEQUENCE_BODY, 0, 2, 1);
        let window = FrameWindow::full(2, 1);
        assert_eq!(frame_window_hue_mode(&opaque.rgba, &[], 2, window), FRAME_HUE_NONE);
        assert_eq!(frame_window_hue_mode(&opaque.rgba, &[0, 0], 2, window), FRAME_HUE_NONE);
        assert_eq!(
            frame_window_hue_mode(&opaque.rgba, &[HUE_MASK_HUEABLE, HUE_MASK_HUEABLE], 2, window),
            FRAME_HUE_ALL
        );
        assert_eq!(frame_window_hue_mode(&opaque.rgba, &[HUE_MASK_HUEABLE, 0], 2, window), FRAME_HUE_MASKED);

        // Transparent pixels do not count; the window limits the pixels looked at.
        let mut half_transparent = opaque.rgba.clone();
        half_transparent[RGBA_CHANNELS + RGBA_ALPHA_CHANNEL] = 0;
        assert_eq!(frame_window_hue_mode(&half_transparent, &[HUE_MASK_HUEABLE, 0], 2, window), FRAME_HUE_ALL);
        let first_pixel = FrameWindow { left: 0, top: 0, width: 1, height: 1 };
        assert_eq!(frame_window_hue_mode(&opaque.rgba, &[HUE_MASK_HUEABLE, 0], 2, first_pixel), FRAME_HUE_ALL);

        assert_eq!(resample_hue_mask_nearest(&[HUE_MASK_HUEABLE, 0], 2, 1, 4, 2), [
            HUE_MASK_HUEABLE, HUE_MASK_HUEABLE, 0, 0,
            HUE_MASK_HUEABLE, HUE_MASK_HUEABLE, 0, 0,
        ]);
    }

    #[test]
    fn page_hue_mask_holds_masked_frames_with_extruded_gutters() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks: crate::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            upscale_passes: Vec::new(),
            upscale_profile: None,
            metadata_path: None,
            allow_missing_metadata: false,
        };
        let (pages, placements) = pack_frames_into_pages(vec![half_hued_frame(NO_SEQUENCE_BODY, 0, 4, 4)], &options).unwrap();
        let page = &pages[0];
        assert_eq!(page.record.flags, PAGE_FLAG_HUE_MASK);
        let mask = page.hue_mask.as_ref().unwrap();
        let used_width = page.record.used_width as usize;
        assert_eq!(mask.len(), used_width * page.record.used_height as usize);
        let placement = &placements[&(NO_SEQUENCE_BODY, 0)];
        assert_eq!(placement.hue_mode, FRAME_HUE_MASKED);
        let (x, y) = (usize::from(placement.x), usize::from(placement.y));
        assert_eq!(mask[y * used_width + x], HUE_MASK_HUEABLE);
        assert_eq!(mask[y * used_width + x + 3], 0);
        if x > 0 {
            assert_eq!(mask[y * used_width + x - 1], HUE_MASK_HUEABLE, "left gutter repeats the edge");
        }

        // Pages without masked frames carry no mask.
        let (plain_pages, plain_placements) = pack_frames_into_pages(vec![frame(NO_SEQUENCE_BODY, 0, 4, 4)], &options).unwrap();
        assert!(plain_pages[0].hue_mask.is_none());
        assert_eq!(plain_pages[0].record.flags, 0);
        assert_eq!(plain_placements[&(NO_SEQUENCE_BODY, 0)].hue_mode, FRAME_HUE_NONE);
    }
}
