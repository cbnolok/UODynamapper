//! Build-time support for `mobile_anim_ec.uddp`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::{AnimationFrame, FrameEntry};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::bc7::{
    encode_for_vram_with_bc7_rdo_lambda_and_progress, preferred_bc7_encoder_backend, ImageExtent,
    RawImageFormat, VramTextureEncoding,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::{find_first_dir_matching, find_first_existing_file};
use crate::upscale::{apply_filter_passes, UpscaleFilter};
use crate::{resolve_packing_axis, AtlasPackingMode};
use udd_assets::mobile_anim_ec::{
    EcMobileAnimationsKdl,
    page_entry_path, MobileAnimEcAnimationRecord, MobileAnimEcFrameRecord,
    MobileAnimEcItemRecord, MobileAnimEcPageRecord, MobileAnimEcSourceHintRecord,
    ANIMATION_MANIFEST_ENTRY_PATH, FRAME_MANIFEST_ENTRY_PATH, ITEM_MANIFEST_ENTRY_PATH,
    MISSING_PAGE_FRAME_INDEX, MISSING_PAGE_INDEX, PAGE_MANIFEST_ENTRY_PATH,
    SOURCE_HINT_MANIFEST_ENTRY_PATH,
};
use udd_assets::tex_art_cc::PagePixelFormat;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 4;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const ITEM_MANIFEST_MAGIC: [u8; 4] = *b"MEIT";
const SOURCE_HINT_MANIFEST_MAGIC: [u8; 4] = *b"MESH";
const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 2;
const PLANNED_SOURCE_CACHE_LIMIT: usize = 256;
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
}

pub struct MobileAnimEcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub pixel_format: PagePixelFormat,
    pub bc7_rdo_lambda: f32,
    pub upscale_passes: Vec<UpscaleFilter>,
    pub metadata_path: Option<PathBuf>,
    pub tables_dir: Option<PathBuf>,
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
            upscale_passes: Vec::new(),
            metadata_path: None,
            tables_dir: None,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PlannedMobileAnimEcSource {
    path: PathBuf,
    file_hash: u64,
}

#[derive(Debug, Clone)]
struct PlannedMobileAnimEcFrame {
    body_id: u32,
    source_frame_index: u16,
    source_entry_index: u16,
    width: u16,
    height: u16,
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
}

#[derive(Debug, Clone)]
pub struct BuiltMobileAnimEcPage {
    pub record: MobileAnimEcPageRecord,
    pub pixels: Vec<u8>,
}

struct PackedMobileAnimEcPages {
    records: Vec<MobileAnimEcPageRecord>,
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

#[derive(Debug, Clone, Copy, Default)]
struct MobileAnimPageStats {
    used_page_pixel_count: u64,
    filled_pixel_count: u64,
    empty_pixel_count: u64,
}

pub fn convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<MobileAnimEcBuildSummary> {
    validate_options(options)?;
    let client_dir = find_first_dir_matching(source_dirs, &[&["AnimationFrame1.uop"], &["animationframe1.uop"]])
        .ok_or_else(|| eyre::eyre!(
            "no EC animation sources found in any provided path: expected AnimationFrame1.uop"
        ))?;
    let animationframe_paths = discover_animationframe_paths(&client_dir);
    if animationframe_paths.is_empty() {
        eyre::bail!("missing EC AnimationFrame1.uop..AnimationFrame6.uop in {}", client_dir.display());
    }
    let metadata_path = find_ec_mobile_animations_kdl(
        source_dirs,
        options.metadata_path.as_deref(),
        options.tables_dir.as_deref(),
    )
        .context("missing EcMobileAnimations.kdl for EC mobile animation metadata")?;
    let metadata = EcMobileAnimationsKdl::load(&metadata_path)?;
    let (items, source_hints) = build_item_metadata(&metadata)?;

    info!(
        "Converting EC mobile animations from {} UOP files to {}",
        animationframe_paths.len(),
        out_file.display()
    );
    println!("Using EC animation source dir: {}", client_dir.display());
    println!("Using EC mobile animation metadata: {}", metadata_path.display());

    let mut planned_by_body = plan_animationframe_packages(&animationframe_paths)?;
    apply_planned_upscale(&mut planned_by_body, options)?;
    let planned_frames = planned_by_body
        .values()
        .flat_map(|frames| frames.iter().cloned())
        .collect::<Vec<_>>();
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    let (packed_pages, placements) =
        pack_planned_frames_into_package(&mut package, planned_frames, options)?;
    let packed_frame_count = placements.len() as u32;
    let sequences = load_animation_sequences(&client_dir, planned_by_body.keys().copied().collect::<Vec<_>>())?;
    let planned_frame_indices_by_body = planned_frame_indices_by_body(&planned_by_body);
    let (animations, frames) =
        build_animation_records_for_body_frames(&planned_frame_indices_by_body, &placements, &sequences)?;

    let page_manifest = serialize_page_record_manifest(&packed_pages.records, options)?;
    let animation_manifest = serialize_animation_manifest(&animations)?;
    let frame_manifest = serialize_frame_manifest(&frames)?;
    let item_manifest = serialize_item_manifest(&items)?;
    let source_hint_manifest = serialize_source_hint_manifest(&source_hints)?;

    for (path, data) in [
        (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice()),
        (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice()),
        (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice()),
        (ITEM_MANIFEST_ENTRY_PATH, item_manifest.as_slice()),
        (SOURCE_HINT_MANIFEST_ENTRY_PATH, source_hint_manifest.as_slice()),
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

    build_and_write_package(&mut package, out_file)?;

    Ok(MobileAnimEcBuildSummary {
        body_count: planned_by_body.len() as u32,
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
    })
}

fn summarize_mobile_anim_pages(pages: &[BuiltMobileAnimEcPage]) -> MobileAnimPageStats {
    let mut stats = MobileAnimPageStats::default();
    for page in pages {
        stats.add_page(page);
    }
    stats
}

impl MobileAnimPageStats {
    fn add_page(&mut self, page: &BuiltMobileAnimEcPage) {
        let used_pixels = page.record.used_width as u64 * page.record.used_height as u64;
        let filled_pixels = page
            .pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[3] != 0)
            .count() as u64;
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

fn mobile_anim_page_progress_message(options: &MobileAnimEcAtlasOptions) -> &'static str {
    if options.pixel_format == PagePixelFormat::Bc7 {
        if options.bc7_rdo_lambda > 0.0 && options.bc7_rdo_lambda.is_finite() {
            "BC7-compressing EC mobile animation atlas pages; RDO pass follows"
        } else {
            "BC7-compressing EC mobile animation atlas pages"
        }
    } else if options.compression == CompressionFlag::JpegXl {
        "registering EC mobile animation atlas pages for JPEG XL package compression"
    } else {
        "registering uncompressed EC mobile animation atlas pages"
    }
}

fn encode_and_add_mobile_anim_page_chunk(
    package: &mut UddpBuilder,
    pages: &[BuiltMobileAnimEcPage],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<()> {
    let encode_pb = if options.pixel_format == PagePixelFormat::Bc7 {
        let pb = ProgressBar::new_spinner();
        pb.set_message(mobile_anim_page_progress_message(options));
        pb.enable_steady_tick(Duration::from_millis(100));
        Some(pb)
    } else {
        None
    };
    let encoded_pages = encode_mobile_anim_page_chunk(pages, options, encode_pb.as_ref())?;
    if let Some(pb) = encode_pb {
        pb.finish_and_clear();
    }
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
    Ok(())
}

fn encode_mobile_anim_page_chunk(
    pages: &[BuiltMobileAnimEcPage],
    options: &MobileAnimEcAtlasOptions,
    pb: Option<&ProgressBar>,
) -> eyre::Result<Vec<(String, Vec<u8>, u32, u32)>> {
    if options.pixel_format == PagePixelFormat::Bc7 {
        let encoding = VramTextureEncoding::Bc7(preferred_bc7_encoder_backend());
        let encoded_pages = pages
            .par_iter()
            .map(|page| {
                let extent = ImageExtent::new(page.record.used_width, page.record.used_height)
                    .map_err(|e| eyre::eyre!("{e}"))?;
                let encoded = encode_for_vram_with_bc7_rdo_lambda_and_progress(
                    &page.pixels,
                    extent,
                    RawImageFormat::Rgba8888,
                    encoding,
                    options.bc7_rdo_lambda,
                    |units| {
                        if let Some(pb) = pb {
                            pb.inc(units as u64);
                        }
                    },
                )
                .map_err(|e| {
                    eyre::eyre!("BC7 encode EC mobile animation page {}: {e}", page.record.page_index)
                })?
                .into_bytes()
                .to_vec();
                Ok((
                    page_entry_path(page.record.page_index, PagePixelFormat::Bc7),
                    encoded,
                    page.record.used_width,
                    page.record.used_height,
                ))
            })
            .collect::<Vec<eyre::Result<(String, Vec<u8>, u32, u32)>>>();

        let mut resolved = Vec::with_capacity(encoded_pages.len());
        for page in encoded_pages {
            resolved.push(page?);
        }
        Ok(resolved)
    } else {
        Ok(pages
            .iter()
            .map(|page| {
                (
                    page_entry_path(page.record.page_index, PagePixelFormat::Rgba8888),
                    page.pixels.clone(),
                    page.record.used_width,
                    page.record.used_height,
                )
            })
            .collect())
    }
}

fn discover_animationframe_paths(client_dir: &Path) -> Vec<PathBuf> {
    EC_ANIMATIONFRAME_FILES
        .iter()
        .map(|name| client_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn find_ec_mobile_animations_kdl(
    source_dirs: &[PathBuf],
    explicit_path: Option<&Path>,
    tables_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = explicit_path {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    if let Some(dir) = tables_dir {
        let path = if dir.is_file() {
            dir.to_path_buf()
        } else {
            dir.join("EcMobileAnimations.kdl")
        };
        if path.is_file() {
            return Some(path);
        }
    }
    find_first_existing_file(source_dirs, &[
        "EcMobileAnimations.kdl",
        "cc_ec_convtables/EcMobileAnimations.kdl",
        "dynamapper/assets/cc_ec_convtables/EcMobileAnimations.kdl",
    ])
    .or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| find_first_existing_file(&[cwd], &[
                "EcMobileAnimations.kdl",
                "cc_ec_convtables/EcMobileAnimations.kdl",
                "dynamapper/assets/cc_ec_convtables/EcMobileAnimations.kdl",
            ]))
    })
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
    if options.upscale_passes.is_empty() {
        return Ok(());
    }

    let scale = options
        .upscale_passes
        .iter()
        .copied()
        .filter(|filter| !matches!(filter, UpscaleFilter::None))
        .fold(1u32, |scale, filter| scale.saturating_mul(filter.scale_factor()));
    if scale == 1 {
        return Ok(());
    }

    for frames in planned_by_body.values_mut() {
        for frame in frames {
            frame.width = scale_u16(frame.width, scale, "EC mobile animation frame width")?;
            frame.height = scale_u16(frame.height, scale, "EC mobile animation frame height")?;
            frame.center_x = scale_i16(frame.center_x, scale, "EC mobile animation frame center_x")?;
            frame.center_y = scale_i16(frame.center_y, scale, "EC mobile animation frame center_y")?;
        }
    }
    Ok(())
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
    let pb = ProgressBar::new(file_hashes.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("extracting EC mobile animations from {file_name}"));

    let mut planned_by_body = BTreeMap::<u32, Vec<PlannedMobileAnimEcFrame>>::new();
    for file_hash in file_hashes {
        pb.inc(1);
        let Ok(Some(data)) = package.unpack_file_by_hash(file_hash) else { continue; };
        let Ok(animation) = AnimationFrame::load(&data) else { continue; };
        let mut frames = Vec::with_capacity(animation.frames.len());
        for (index, entry) in animation.frames.iter().enumerate() {
            if index > u16::MAX as usize {
                continue;
            }
            frames.push(planned_frame_from_entry(
                &animation,
                entry,
                index as u16,
                path.to_path_buf(),
                file_hash,
            ));
        }
        append_planned_body_frames(&mut planned_by_body, animation.animation_id, frames)?;
    }
    pb.finish_with_message(format!("EC mobile animations extracted from {file_name}"));
    Ok(planned_by_body)
}

fn planned_frame_from_entry(
    animation: &AnimationFrame,
    entry: &FrameEntry,
    source_entry_index: u16,
    path: PathBuf,
    file_hash: u64,
) -> PlannedMobileAnimEcFrame {
    let width = (entry.end_coords_x - entry.init_coords_x).abs() as u16;
    let height = (entry.end_coords_y - entry.init_coords_y).abs() as u16;
    PlannedMobileAnimEcFrame {
        body_id: animation.animation_id,
        source_frame_index: source_entry_index,
        source_entry_index,
        width,
        height,
        center_x: if width == 0 || height == 0 {
            0
        } else {
            animation.init_coords_x - entry.init_coords_x
        },
        center_y: if width == 0 || height == 0 {
            0
        } else {
            animation.init_coords_y - entry.init_coords_y
        },
        source: PlannedMobileAnimEcSource { path, file_hash },
    }
}

fn decode_planned_animation_source(source: &PlannedMobileAnimEcSource) -> eyre::Result<AnimationFrame> {
    let package = UopPackage::load_with_mode(&source.path, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", source.path.display()))?;
    let data = package
        .unpack_file_by_hash(source.file_hash)?
        .ok_or_else(|| eyre::eyre!(
            "EC AnimationFrame payload {:016x} not found in {}",
            source.file_hash,
            source.path.display()
        ))?;
    AnimationFrame::load(&data)
}

fn cached_decode_planned_animation_source(
    source: &PlannedMobileAnimEcSource,
    cache: &mut HashMap<PlannedMobileAnimEcSource, Arc<AnimationFrame>>,
    order: &mut VecDeque<PlannedMobileAnimEcSource>,
) -> eyre::Result<Arc<AnimationFrame>> {
    if !cache.contains_key(source) {
        let decoded = Arc::new(decode_planned_animation_source(source)?);
        cache.insert(source.clone(), decoded);
        order.push_back(source.clone());
        while cache.len() > PLANNED_SOURCE_CACHE_LIMIT {
            let Some(oldest) = order.pop_front() else { break; };
            if &oldest == source {
                order.push_back(oldest);
                break;
            }
            cache.remove(&oldest);
        }
    }

    Ok(Arc::clone(
        cache
            .get(source)
            .expect("planned EC mobile animation source was just cached"),
    ))
}

fn planned_frame_indices_by_body(
    planned_by_body: &BTreeMap<u32, Vec<PlannedMobileAnimEcFrame>>,
) -> BTreeMap<u32, Vec<u16>> {
    planned_by_body
        .iter()
        .map(|(&body_id, frames)| {
            (
                body_id,
                frames
                    .iter()
                    .map(|frame| frame.source_frame_index)
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn load_animation_sequences(
    client_dir: &Path,
    body_ids: Vec<u32>,
) -> eyre::Result<HashMap<u32, AnimationSequence>> {
    let sequence_path = ["AnimationSequence.uop", "animationsequence.uop"]
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file());
    let Some(sequence_path) = sequence_path else {
        return Ok(HashMap::new());
    };
    let package = UopPackage::load_with_mode(&sequence_path, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", sequence_path.display()))?;
    let mut sequences = HashMap::new();
    let pb = ProgressBar::new(body_ids.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("loading EC mobile animation sequences");
    pb.enable_steady_tick(Duration::from_millis(100));
    for body_id in body_ids {
        pb.inc(1);
        for path in [
            format!("data/animationsequence/{body_id:06}.bin"),
            format!("build/animationsequence/{body_id:08}.bin"),
        ] {
            let hash = hash_file_name_single(&path);
            let Some(data) = package.unpack_file_by_hash(hash)? else {
                continue;
            };
            if let Ok(sequence) = AnimationSequence::parse_ec(&data, body_id) {
                sequences.insert(body_id, sequence);
                break;
            }
        }
    }
    pb.finish_with_message(format!("EC mobile animation sequences loaded ({})", sequences.len()));
    Ok(sequences)
}

fn build_animation_records(
    decoded_by_body: &BTreeMap<u32, Vec<DecodedMobileAnimEcFrame>>,
    placements: &HashMap<(u32, u16), FramePlacement>,
    sequences: &HashMap<u32, AnimationSequence>,
) -> eyre::Result<(Vec<MobileAnimEcAnimationRecord>, Vec<MobileAnimEcFrameRecord>)> {
    let source_frame_indices_by_body = decoded_by_body
        .iter()
        .map(|(&body_id, frames)| {
            (
                body_id,
                frames
                    .iter()
                    .map(|frame| frame.source_frame_index)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    build_animation_records_for_body_frames(&source_frame_indices_by_body, placements, sequences)
}

fn build_animation_records_for_body_frames(
    source_frame_indices_by_body: &BTreeMap<u32, Vec<u16>>,
    placements: &HashMap<(u32, u16), FramePlacement>,
    sequences: &HashMap<u32, AnimationSequence>,
) -> eyre::Result<(Vec<MobileAnimEcAnimationRecord>, Vec<MobileAnimEcFrameRecord>)> {
    let mut animations = Vec::new();
    let mut frames = Vec::new();

    for (&body_id, source_frame_indices) in source_frame_indices_by_body {
        if let Some(sequence) = sequences.get(&body_id) {
            let mut action_ids = sequence.actions.keys().copied().collect::<Vec<_>>();
            action_ids.sort_unstable();
            for action_id in action_ids {
                let action = &sequence.actions[&action_id];
                for (direction, direction_data) in action.directions.iter().enumerate() {
                    if direction > u8::MAX as usize || direction_data.frame_indices.len() > u16::MAX as usize {
                        continue;
                    }
                    let animation_index = animations.len() as u32;
                    let frame_start = frames.len() as u32;
                    for (frame_index, &source_frame_index) in direction_data.frame_indices.iter().enumerate() {
                        frames.push(frame_record_from_placement(
                            animation_index,
                            frame_index as u16,
                            source_frame_index,
                            placements.get(&(body_id, source_frame_index)),
                        ));
                    }
                    animations.push(MobileAnimEcAnimationRecord {
                        body_id,
                        action_id,
                        direction: direction as u8,
                        frame_start,
                        frame_count: (frames.len() as u32 - frame_start) as u16,
                        flags: 0,
                    });
                }
            }
        } else {
            if source_frame_indices.len() > u16::MAX as usize {
                continue;
            }
            let animation_index = animations.len() as u32;
            let frame_start = frames.len() as u32;
            for &source_frame_index in source_frame_indices {
                frames.push(frame_record_from_placement(
                    animation_index,
                    source_frame_index,
                    source_frame_index,
                    placements.get(&(body_id, source_frame_index)),
                ));
            }
            animations.push(MobileAnimEcAnimationRecord {
                body_id,
                action_id: 0,
                direction: 0,
                frame_start,
                frame_count: (frames.len() as u32 - frame_start) as u16,
                flags: 0,
            });
        }
    }

    Ok((animations, frames))
}

fn frame_record_from_placement(
    animation_index: u32,
    frame_index: u16,
    source_frame_index: u16,
    placement: Option<&FramePlacement>,
) -> MobileAnimEcFrameRecord {
    if let Some(placement) = placement {
        MobileAnimEcFrameRecord {
            animation_index,
            frame_index,
            source_frame_index,
            page_index: placement.page_index,
            page_frame_index: placement.page_frame_index,
            x: placement.x,
            y: placement.y,
            width: placement.width,
            height: placement.height,
            center_x: placement.center_x,
            center_y: placement.center_y,
        }
    } else {
        MobileAnimEcFrameRecord {
            animation_index,
            frame_index,
            source_frame_index,
            page_index: MISSING_PAGE_INDEX,
            page_frame_index: MISSING_PAGE_FRAME_INDEX,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            center_x: 0,
            center_y: 0,
        }
    }
}

pub(crate) fn pack_frames_into_pages(
    frames: Vec<DecodedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(Vec<BuiltMobileAnimEcPage>, HashMap<(u32, u16), FramePlacement>)> {
    let mut remaining = frames
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
        let (page, unplaced) =
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
    package: &mut UddpBuilder,
    frames: Vec<PlannedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
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

    remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
    let mut records = Vec::new();
    let mut stats = MobileAnimPageStats::default();
    let mut placements = HashMap::new();
    let mut page_index = 0u32;
    let mut pending_pages = Vec::new();
    let mut page_pixels = Vec::new();
    let chunk_size = rayon::current_num_threads().max(1);
    let mut next_frame = 0usize;
    let mut decoded_source_cache = HashMap::<PlannedMobileAnimEcSource, Arc<AnimationFrame>>::new();
    let mut decoded_source_order = VecDeque::<PlannedMobileAnimEcSource>::new();

    while next_frame < remaining.len() {
        pb.set_message(format!("creating EC mobile animation atlas page {}", page_index + 1));
        let (page_size, prefix_len) = select_planned_page_bucket(&remaining[next_frame..], options)?;
        if prefix_len == 0 {
            eyre::bail!(
                "could not fit any EC mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        let page_frames = remaining[next_frame..next_frame + prefix_len].to_vec();
        next_frame += prefix_len;
        let (page, unplaced) = build_planned_page(
            page_index,
            page_size,
            page_frames,
            &mut placements,
            options,
            &mut page_pixels,
            &mut decoded_source_cache,
            &mut decoded_source_order,
        )?;
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
        stats.add_page(&page);
        records.push(page.record);
        pending_pages.push(page);
        if pending_pages.len() >= chunk_size {
            encode_and_add_mobile_anim_page_chunk(package, &pending_pages, options)?;
            pending_pages.clear();
        }
        page_index += 1;
    }

    if !pending_pages.is_empty() {
        encode_and_add_mobile_anim_page_chunk(package, &pending_pages, options)?;
    }

    pb.finish_with_message(format!("EC mobile animation atlas pages created ({})", records.len()));
    Ok((PackedMobileAnimEcPages { records, stats }, placements))
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
            let prefix_len = max_fitting_page_prefix_len(frames, page_size, options)?;
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area = page_prefix_alloc_area(&frames[..prefix_len], page_size, options)?;
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
            let prefix_len = max_fitting_planned_page_prefix_len(frames, page_size, options)?;
            if prefix_len == 0 {
                return Ok(None);
            }
            let alloc_area =
                planned_page_prefix_alloc_area(&frames[..prefix_len], page_size, options)?;
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

fn max_fitting_page_prefix_len(
    frames: &[DecodedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<usize> {
    let mut best = 0usize;
    let mut high = 1usize;
    while high <= frames.len() {
        if page_prefix_fits(&frames[..high], page_size, options)? {
            best = high;
            if high == frames.len() {
                return Ok(best);
            }
            high = high.saturating_mul(2).min(frames.len());
        } else {
            break;
        }
    }
    if best == 0 {
        return Ok(0);
    }

    let mut low = best + 1;
    let mut high = high.saturating_sub(1);
    while low <= high {
        let mid = low + (high - low) / 2;
        if page_prefix_fits(&frames[..mid], page_size, options)? {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }
    Ok(best)
}

fn max_fitting_planned_page_prefix_len(
    frames: &[PlannedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<usize> {
    let mut best = 0usize;
    let mut high = 1usize;
    while high <= frames.len() {
        if planned_page_prefix_fits(&frames[..high], page_size, options)? {
            best = high;
            if high == frames.len() {
                return Ok(best);
            }
            high = high.saturating_mul(2).min(frames.len());
        } else {
            break;
        }
    }
    if best == 0 {
        return Ok(0);
    }

    let mut low = best + 1;
    let mut high = high.saturating_sub(1);
    while low <= high {
        let mid = low + (high - low) / 2;
        if planned_page_prefix_fits(&frames[..mid], page_size, options)? {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }
    Ok(best)
}

fn page_prefix_fits(
    frames: &[DecodedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<bool> {
    let mut to_pack = frames.iter().collect::<Vec<_>>();
    sort_frame_refs_within_page(&mut to_pack, page_size, options);
    let mut allocator = AtlasAllocator::new(size2(
        page_size.width as i32,
        page_size.height as i32,
    ));
    for frame in to_pack {
        let Ok((width_axis, height_axis)) = packing_axes(frame, page_size, options) else {
            return Ok(false);
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

fn planned_page_prefix_fits(
    frames: &[PlannedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<bool> {
    let mut to_pack = frames.iter().collect::<Vec<_>>();
    sort_planned_frame_refs_within_page(&mut to_pack, page_size, options);
    let mut allocator = AtlasAllocator::new(size2(
        page_size.width as i32,
        page_size.height as i32,
    ));
    for frame in to_pack {
        let Ok((width_axis, height_axis)) = planned_packing_axes(frame, page_size, options) else {
            return Ok(false);
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

fn page_prefix_alloc_area(
    frames: &[DecodedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<u64> {
    frames.iter().try_fold(0u64, |total, frame| {
        let (width_axis, height_axis) = packing_axes(frame, page_size, options)?;
        Ok(total + width_axis.alloc_extent as u64 * height_axis.alloc_extent as u64)
    })
}

fn planned_page_prefix_alloc_area(
    frames: &[PlannedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<u64> {
    frames.iter().try_fold(0u64, |total, frame| {
        let (width_axis, height_axis) = planned_packing_axes(frame, page_size, options)?;
        Ok(total + width_axis.alloc_extent as u64 * height_axis.alloc_extent as u64)
    })
}

fn build_page(
    page_index: u32,
    page_size: AtlasPageSize,
    mut frames: Vec<DecodedMobileAnimEcFrame>,
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    options: &MobileAnimEcAtlasOptions,
    pixels: &mut Vec<u8>,
) -> eyre::Result<(BuiltMobileAnimEcPage, Vec<DecodedMobileAnimEcFrame>)> {
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

    sort_frames_within_page(&mut frames, page_size, options);

    for frame in frames {
        let (width_axis, height_axis) = packing_axes(&frame, page_size, options)?;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            blit_rgba_frame(
                pixels,
                page_size.width,
                inner_x as u32,
                inner_y as u32,
                frame.width as u32,
                frame.height as u32,
                &frame.rgba,
            )?;
            used_width = used_width.max(inner_x as u32 + width_axis.used_extent);
            used_height = used_height.max(inner_y as u32 + height_axis.used_extent);
            placements.insert((frame.body_id, frame.source_frame_index), FramePlacement {
                page_index,
                page_frame_index,
                x: inner_x as u16,
                y: inner_y as u16,
                width: frame.width,
                height: frame.height,
                center_x: frame.center_x,
                center_y: frame.center_y,
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
            },
            pixels,
        },
        leftovers,
    ))
}

fn build_planned_page(
    page_index: u32,
    page_size: AtlasPageSize,
    mut frames: Vec<PlannedMobileAnimEcFrame>,
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    options: &MobileAnimEcAtlasOptions,
    pixels: &mut Vec<u8>,
    decoded_sources: &mut HashMap<PlannedMobileAnimEcSource, Arc<AnimationFrame>>,
    decoded_source_order: &mut VecDeque<PlannedMobileAnimEcSource>,
) -> eyre::Result<(BuiltMobileAnimEcPage, Vec<PlannedMobileAnimEcFrame>)> {
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
    let mut pending_blits = Vec::new();

    struct PendingPlannedBlit {
        body_id: u32,
        source_frame_index: u16,
        inner_x: u32,
        inner_y: u32,
        expected_width: u16,
        expected_height: u16,
        animation: Arc<AnimationFrame>,
        source_entry: FrameEntry,
    }

    struct PreparedPlannedBlit {
        body_id: u32,
        source_frame_index: u16,
        inner_x: u32,
        inner_y: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    sort_planned_frames_within_page(&mut frames, page_size, options);

    for frame in frames {
        let (width_axis, height_axis) = planned_packing_axes(&frame, page_size, options)?;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let animation = cached_decode_planned_animation_source(
                &frame.source,
                decoded_sources,
                decoded_source_order,
            )?;
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
            pending_blits.push(PendingPlannedBlit {
                body_id: frame.body_id,
                source_frame_index: frame.source_frame_index,
                inner_x: inner_x as u32,
                inner_y: inner_y as u32,
                expected_width: frame.width,
                expected_height: frame.height,
                animation,
                source_entry,
            });
            used_width = used_width.max(inner_x as u32 + width_axis.used_extent);
            used_height = used_height.max(inner_y as u32 + height_axis.used_extent);
            placements.insert((frame.body_id, frame.source_frame_index), FramePlacement {
                page_index,
                page_frame_index,
                x: inner_x as u16,
                y: inner_y as u16,
                width: frame.width,
                height: frame.height,
                center_x: frame.center_x,
                center_y: frame.center_y,
            });
            page_frame_index += 1;
        } else {
            leftovers.push(frame);
        }
    }

    let prepared_blits = pending_blits
        .into_par_iter()
        .map(|pending| -> eyre::Result<PreparedPlannedBlit> {
            let decoded = pending.animation.decode_frame(&pending.source_entry)?;
            let (width, height, rgba, _, _) = apply_filter_passes(
                decoded.width as u32,
                decoded.height as u32,
                &decoded.data,
                &options.upscale_passes,
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
            Ok(PreparedPlannedBlit {
                body_id: pending.body_id,
                source_frame_index: pending.source_frame_index,
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
        blit_rgba_frame(
            pixels,
            page_size.width,
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
    }

    let pixels = crate::tex_art_cc::crop_rgba_page(
        pixels,
        page_size.width,
        used_width,
        used_height,
    );

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
            },
            pixels,
        },
        leftovers,
    ))
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

fn sort_frames_within_page(
    frames: &mut [DecodedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) {
    frames.sort_by(|left, right| {
        compare_frames_for_page(left, right, page_size, options)
    });
}

fn sort_frame_refs_within_page<'a>(
    frames: &mut [&'a DecodedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) {
    frames.sort_by(|left, right| compare_frames_for_page(left, right, page_size, options));
}

fn sort_planned_frames_within_page(
    frames: &mut [PlannedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) {
    frames.sort_by(|left, right| compare_planned_frames_for_page(left, right, page_size, options));
}

fn sort_planned_frame_refs_within_page<'a>(
    frames: &mut [&'a PlannedMobileAnimEcFrame],
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) {
    frames.sort_by(|left, right| compare_planned_frames_for_page(left, right, page_size, options));
}

fn compare_frames_for_page(
    left: &DecodedMobileAnimEcFrame,
    right: &DecodedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> std::cmp::Ordering {
    let left_area = sort_area(left, page_size, options);
    let right_area = sort_area(right, page_size, options);
    right_area
        .cmp(&left_area)
        .then_with(|| (left.body_id, left.source_frame_index).cmp(&(right.body_id, right.source_frame_index)))
}

fn sort_area(
    frame: &DecodedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> u32 {
    packing_axes(frame, page_size, options)
        .map(|(width_axis, height_axis)| width_axis.alloc_extent * height_axis.alloc_extent)
        .unwrap_or(0)
}

fn compare_planned_frames_for_page(
    left: &PlannedMobileAnimEcFrame,
    right: &PlannedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> std::cmp::Ordering {
    let left_area = planned_sort_area(left, page_size, options);
    let right_area = planned_sort_area(right, page_size, options);
    right_area
        .cmp(&left_area)
        .then_with(|| (left.body_id, left.source_frame_index).cmp(&(right.body_id, right.source_frame_index)))
}

fn planned_sort_area(
    frame: &PlannedMobileAnimEcFrame,
    page_size: AtlasPageSize,
    options: &MobileAnimEcAtlasOptions,
) -> u32 {
    planned_packing_axes(frame, page_size, options)
        .map(|(width_axis, height_axis)| width_axis.alloc_extent * height_axis.alloc_extent)
        .unwrap_or(0)
}

fn blit_rgba_frame(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    frame_width: u32,
    frame_height: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let expected_len = frame_width as usize * frame_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for EC mobile animation frame {}x{}: expected {}, got {}",
            frame_width,
            frame_height,
            expected_len,
            src.len()
        );
    }
    let dst_stride = dst_width as usize * 4;
    let src_stride = frame_width as usize * 4;
    for row in 0..frame_height as usize {
        let src_start = row * src_stride;
        let dst_start = ((dst_y as usize + row) * dst_stride) + dst_x as usize * 4;
        let dst_end = dst_start + src_stride;
        dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + src_stride]);
    }
    Ok(())
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
    let mut bytes = Vec::with_capacity(26 + records.len() * 24);
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
    }
    Ok(bytes)
}

pub fn serialize_animation_manifest(
    animations: &[MobileAnimEcAnimationRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + animations.len() * 15);
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
    }
    Ok(bytes)
}

pub fn serialize_frame_manifest(frames: &[MobileAnimEcFrameRecord]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + frames.len() * 26);
    bytes.extend_from_slice(&FRAME_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(frames.len() as u32)?;
    for frame in frames {
        bytes.write_u32::<LittleEndian>(frame.animation_index)?;
        bytes.write_u16::<LittleEndian>(frame.frame_index)?;
        bytes.write_u16::<LittleEndian>(frame.source_frame_index)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::MobileAnimEcPackage;
    use udd_container::UddpReader;

    fn frame(body_id: u32, source_frame_index: u16, width: u16, height: u16) -> DecodedMobileAnimEcFrame {
        DecodedMobileAnimEcFrame {
            body_id,
            source_frame_index,
            width,
            height,
            center_x: 1,
            center_y: -1,
            rgba: vec![255u8; width as usize * height as usize * 4],
        }
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
            upscale_passes: Vec::new(),
            metadata_path: None,
            tables_dir: None,
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
    fn packer_uses_smallest_effective_bucket() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 2048,
            atlas_height: 2048,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            upscale_passes: Vec::new(),
            metadata_path: None,
            tables_dir: None,
        };

        let (pages, placements) = pack_frames_into_pages(vec![frame(42, 0, 16, 16)], &options).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].record.atlas_width, 512);
        assert_eq!(pages[0].record.atlas_height, 512);
        assert_eq!(placements[&(42, 0)].page_index, 0);
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
            upscale_passes: Vec::new(),
            metadata_path: None,
            tables_dir: None,
        };
        let (pages, placements) = pack_frames_into_pages(vec![frame(42, 0, 4, 4)], &options).unwrap();
        let (animations, frames) = build_animation_records(
            &BTreeMap::from([(42, vec![frame(42, 0, 4, 4)])]),
            &placements,
            &HashMap::new(),
        )
        .unwrap();
        let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
        let animation_manifest = serialize_animation_manifest(&animations).unwrap();
        let frame_manifest = serialize_frame_manifest(&frames).unwrap();
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

        let package = MobileAnimEcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(package.pages().len(), 1);
        assert_eq!(package.animation(42, 0, 0).unwrap().frame_count, 1);
        assert_eq!(package.animation_frames(package.animation(42, 0, 0).unwrap())[0].center_y, -1);
        assert_eq!(package.item(42).unwrap().name, "Test Body");
        assert_eq!(package.item_source_hints(package.item(42).unwrap())[0].file_index, 7);
        assert_eq!(package.read_page_bytes(0).unwrap().len(), stored_page.len());
    }

    #[test]
    fn animation_records_preserve_unpacked_empty_frames() {
        let source_frames = BTreeMap::from([(42, vec![
            frame(42, 0, 4, 4),
            DecodedMobileAnimEcFrame {
                body_id: 42,
                source_frame_index: 1,
                width: 0,
                height: 0,
                center_x: 0,
                center_y: 0,
                rgba: Vec::new(),
            },
        ])]);
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            upscale_passes: Vec::new(),
            metadata_path: None,
            tables_dir: None,
        };
        let (_, placements) = pack_frames_into_pages(source_frames[&42].clone(), &options).unwrap();
        let (animations, frames) = build_animation_records(&source_frames, &placements, &HashMap::new()).unwrap();

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
}
