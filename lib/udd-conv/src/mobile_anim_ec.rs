//! Build-time support for `mobile_anim_ec.uddp`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::AnimationFrame;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

use crate::bc7::{
    bc7_encode_progress_units, encode_for_vram_with_bc7_rdo_lambda_and_progress,
    preferred_bc7_encoder_backend, ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::{find_first_dir_matching, find_first_existing_file};
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
const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 1;
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
    let metadata_path = find_ec_mobile_animations_kdl(source_dirs)
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

    let decoded_by_body = decode_animationframe_packages(&animationframe_paths)?;
    let decoded_frames = decoded_by_body
        .values()
        .flat_map(|frames| frames.iter().cloned())
        .collect::<Vec<_>>();
    let (pages, placements) = pack_frames_into_pages(decoded_frames, options)?;
    let packed_frame_count = placements.len() as u32;
    let sequences = load_animation_sequences(&client_dir, decoded_by_body.keys().copied().collect::<Vec<_>>())?;
    let (animations, frames) = build_animation_records(&decoded_by_body, &placements, &sequences)?;

    let page_manifest = serialize_page_manifest(&pages, options)?;
    let animation_manifest = serialize_animation_manifest(&animations)?;
    let frame_manifest = serialize_frame_manifest(&frames)?;
    let item_manifest = serialize_item_manifest(&items)?;
    let source_hint_manifest = serialize_source_hint_manifest(&source_hints)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
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

    let page_count = pages.len() as u32;
    let page_stats = summarize_mobile_anim_pages(&pages);
    encode_and_add_mobile_anim_pages(&mut package, pages, options)?;

    build_and_write_package(&mut package, out_file)?;

    Ok(MobileAnimEcBuildSummary {
        body_count: decoded_by_body.len() as u32,
        animation_count: animations.len() as u32,
        frame_count: frames.len() as u32,
        packed_frame_count,
        page_count,
        used_page_pixel_count: page_stats.used_page_pixel_count,
        filled_pixel_count: page_stats.filled_pixel_count,
        empty_pixel_count: page_stats.empty_pixel_count,
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
        let used_pixels = page.record.used_width as u64 * page.record.used_height as u64;
        let filled_pixels = page
            .pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[3] != 0)
            .count() as u64;
        stats.used_page_pixel_count += used_pixels;
        stats.filled_pixel_count += filled_pixels;
        stats.empty_pixel_count += used_pixels.saturating_sub(filled_pixels);
    }
    stats
}

fn validate_options(options: &MobileAnimEcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("EC mobile animation atlas dimensions must be non-zero");
    }
    Ok(())
}

fn encode_and_add_mobile_anim_pages(
    package: &mut UddpBuilder,
    pages: Vec<BuiltMobileAnimEcPage>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<()> {
    let use_bc7 = options.pixel_format == PagePixelFormat::Bc7;
    let progress_len = if use_bc7 {
        pages
            .iter()
            .map(|page| {
                let extent = ImageExtent::new(page.record.used_width, page.record.used_height)
                    .map_err(|e| eyre::eyre!("{e}"))?;
                Ok(bc7_encode_progress_units(extent, options.bc7_rdo_lambda) as u64)
            })
            .sum::<eyre::Result<u64>>()?
    } else {
        pages.len() as u64
    };
    let progress_message = if use_bc7 {
        if options.bc7_rdo_lambda > 0.0 && options.bc7_rdo_lambda.is_finite() {
            "BC7-compressing EC mobile animation atlas pages; RDO pass follows"
        } else {
            "BC7-compressing EC mobile animation atlas pages"
        }
    } else if options.compression == CompressionFlag::JpegXl {
        "registering EC mobile animation atlas pages for JPEG XL package compression"
    } else {
        "registering uncompressed EC mobile animation atlas pages"
    };

    let pb = ProgressBar::new(progress_len);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(progress_message);

    let chunk_size = rayon::current_num_threads().max(1);
    let mut pages = pages.into_iter();
    loop {
        let chunk = pages.by_ref().take(chunk_size).collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        let encoded_pages = encode_mobile_anim_page_chunk(&chunk, options, &pb)?;
        for (page_path, stored_page, width, height) in &encoded_pages {
            package.add_file(AddFileRequest {
                data_type: DataType::Texture as u8,
                compression: options.compression,
                width: *width,
                height: *height,
                virtual_path: Some(page_path),
                path_hash64: None,
                id: None,
                data: stored_page,
            })?;
        }
    }
    let finish_message = if use_bc7 {
        if options.bc7_rdo_lambda > 0.0 && options.bc7_rdo_lambda.is_finite() {
            "EC mobile animation atlas pages BC7-compressed with RDO"
        } else {
            "EC mobile animation atlas pages BC7-compressed"
        }
    } else if options.compression == CompressionFlag::JpegXl {
        "EC mobile animation atlas pages registered for JPEG XL package compression"
    } else {
        "EC mobile animation atlas pages registered uncompressed"
    };
    pb.finish_with_message(finish_message);
    Ok(())
}

fn encode_mobile_anim_page_chunk(
    pages: &[BuiltMobileAnimEcPage],
    options: &MobileAnimEcAtlasOptions,
    pb: &ProgressBar,
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
                    |units| pb.inc(units as u64),
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
                pb.inc(1);
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

fn find_ec_mobile_animations_kdl(source_dirs: &[PathBuf]) -> Option<PathBuf> {
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

fn decode_animationframe_packages(
    paths: &[PathBuf],
) -> eyre::Result<BTreeMap<u32, Vec<DecodedMobileAnimEcFrame>>> {
    let mut decoded_by_body = BTreeMap::<u32, Vec<DecodedMobileAnimEcFrame>>::new();
    for path in paths {
        let package = UopPackage::load(path)
            .wrap_err_with(|| format!("load {}", path.display()))?;
        let decoded = decode_animationframe_package(&package, path)?;
        for (body_id, frames) in decoded {
            append_decoded_body_frames(&mut decoded_by_body, body_id, frames)?;
        }
    }
    Ok(decoded_by_body)
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

fn decode_animationframe_package(
    package: &UopPackage,
    path: &Path,
) -> eyre::Result<BTreeMap<u32, Vec<DecodedMobileAnimEcFrame>>> {
    let files = package.iter_files().filter(|file| file.has_size()).collect::<Vec<_>>();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("AnimationFrame UOP");
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("extracting EC mobile animations from {file_name}"));

    let mut decoded_by_body = BTreeMap::<u32, Vec<DecodedMobileAnimEcFrame>>::new();
    for file in files {
        pb.inc(1);
        let Ok(data) = file.unpack() else { continue; };
        let Ok(animation) = AnimationFrame::load(&data) else { continue; };
        let mut frames = Vec::with_capacity(animation.frames.len());
        for (index, entry) in animation.frames.iter().enumerate() {
            if index > u16::MAX as usize {
                continue;
            }
            let decoded = animation.decode_frame(entry)?;
            frames.push(DecodedMobileAnimEcFrame {
                body_id: animation.animation_id,
                source_frame_index: index as u16,
                width: decoded.width,
                height: decoded.height,
                center_x: decoded.center_x,
                center_y: decoded.center_y,
                rgba: decoded.data,
            });
        }
        append_decoded_body_frames(&mut decoded_by_body, animation.animation_id, frames)?;
    }
    pb.finish_with_message(format!("EC mobile animations extracted from {file_name}"));
    Ok(decoded_by_body)
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
    let package = UopPackage::load(&sequence_path)
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
            let Some(file) = package.get_file_by_hash(hash) else {
                continue;
            };
            let data = file.unpack()?;
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
    let mut animations = Vec::new();
    let mut frames = Vec::new();

    for (&body_id, source_frames) in decoded_by_body {
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
            if source_frames.len() > u16::MAX as usize {
                continue;
            }
            let animation_index = animations.len() as u32;
            let frame_start = frames.len() as u32;
            for frame in source_frames {
                frames.push(frame_record_from_placement(
                    animation_index,
                    frame.source_frame_index,
                    frame.source_frame_index,
                    placements.get(&(body_id, frame.source_frame_index)),
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

    while !remaining.is_empty() {
        pb.set_message(format!("creating EC mobile animation atlas page {}", page_index + 1));
        let (page_frames, leftovers) = take_page_frame_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_frames, &mut placements, options)?;
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

fn take_page_frame_prefix(
    frames: Vec<DecodedMobileAnimEcFrame>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(Vec<DecodedMobileAnimEcFrame>, Vec<DecodedMobileAnimEcFrame>)> {
    let prefix_len = max_fitting_page_prefix_len(&frames, options)?;
    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any EC mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }
    let mut leftovers = frames;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((selected, leftovers))
}

fn max_fitting_page_prefix_len(
    frames: &[DecodedMobileAnimEcFrame],
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<usize> {
    let mut low = 1usize;
    let mut high = frames.len();
    let mut best = 0usize;
    while low <= high {
        let mid = low + (high - low) / 2;
        if page_prefix_fits(&frames[..mid], options)? {
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
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<bool> {
    let mut to_pack = frames.iter().collect::<Vec<_>>();
    sort_frame_refs_within_page(&mut to_pack, options);
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    for frame in to_pack {
        let (width_axis, height_axis) = packing_axes(frame, options)?;
        if allocator
            .allocate(size2(width_axis.alloc_extent as i32, height_axis.alloc_extent as i32))
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn build_page(
    page_index: u32,
    mut frames: Vec<DecodedMobileAnimEcFrame>,
    placements: &mut HashMap<(u32, u16), FramePlacement>,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(BuiltMobileAnimEcPage, Vec<DecodedMobileAnimEcFrame>)> {
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let mut page_frame_index = 0u16;

    sort_frames_within_page(&mut frames, options);

    for frame in frames {
        let (width_axis, height_axis) = packing_axes(&frame, options)?;
        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            blit_rgba_frame(
                &mut pixels,
                options.atlas_width,
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
        &pixels,
        options.atlas_width,
        used_width,
        used_height,
    );

    Ok((
        BuiltMobileAnimEcPage {
            record: MobileAnimEcPageRecord {
                page_index,
                frame_count: page_frame_index as u32,
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
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<(crate::PackingAxis, crate::PackingAxis)> {
    let width_axis = resolve_packing_axis(
        frame.width as u32,
        options.atlas_width,
        options.gutter,
        AtlasPackingMode::Bc7Oriented,
        false,
    );
    let height_axis = resolve_packing_axis(
        frame.height as u32,
        options.atlas_height,
        options.gutter,
        AtlasPackingMode::Bc7Oriented,
        false,
    );
    match (width_axis, height_axis) {
        (Some(width_axis), Some(height_axis)) => Ok((width_axis, height_axis)),
        _ => eyre::bail!(
            "EC mobile animation frame body {} frame {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
            frame.body_id,
            frame.source_frame_index,
            frame.width,
            frame.height,
            options.atlas_width,
            options.atlas_height,
            options.gutter
        ),
    }
}

fn sort_frames_within_page(frames: &mut [DecodedMobileAnimEcFrame], options: &MobileAnimEcAtlasOptions) {
    frames.sort_by(|left, right| {
        compare_frames_for_page(left, right, options)
    });
}

fn sort_frame_refs_within_page<'a>(
    frames: &mut [&'a DecodedMobileAnimEcFrame],
    options: &MobileAnimEcAtlasOptions,
) {
    frames.sort_by(|left, right| compare_frames_for_page(left, right, options));
}

fn compare_frames_for_page(
    left: &DecodedMobileAnimEcFrame,
    right: &DecodedMobileAnimEcFrame,
    options: &MobileAnimEcAtlasOptions,
) -> std::cmp::Ordering {
    let left_area = sort_area(left, options);
    let right_area = sort_area(right, options);
    right_area
        .cmp(&left_area)
        .then_with(|| (left.body_id, left.source_frame_index).cmp(&(right.body_id, right.source_frame_index)))
}

fn sort_area(frame: &DecodedMobileAnimEcFrame, options: &MobileAnimEcAtlasOptions) -> u32 {
    let (width_axis, height_axis) =
        packing_axes(frame, options).unwrap_or_else(|_| unreachable!("validated before placement"));
    width_axis.alloc_extent * height_axis.alloc_extent
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
    let mut bytes = Vec::with_capacity(26 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(options.pixel_format as u8);
    bytes.push(AtlasPackingMode::Bc7Oriented as u8);
    bytes.write_u32::<LittleEndian>(pages.len() as u32)?;
    for page in pages {
        bytes.write_u32::<LittleEndian>(page.record.page_index)?;
        bytes.write_u32::<LittleEndian>(page.record.frame_count)?;
        bytes.write_u32::<LittleEndian>(page.record.used_width)?;
        bytes.write_u32::<LittleEndian>(page.record.used_height)?;
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
    fn serialized_package_roundtrips_through_reader() {
        let options = MobileAnimEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
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
