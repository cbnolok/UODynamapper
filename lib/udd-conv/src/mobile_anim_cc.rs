//! Build-time support for `mobile_anim_cc.uddp`.
//!
//! This package targets classic `anim*.mul` / `anim*.idx` mobile-animation
//! sources plus Classic `AnimationFrame*.uop` packages when present. The atlas
//! packer uses 4-pixel-aligned content extents so the same metadata remains
//! valid for both RGBA8888 and BC7 page payloads.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::classic::anim::{AnimFrame, AnimMap, MAX_ANIM_FILES};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;
use uocf::uop_container::package::UopPackage;

use crate::bc7::{
    bc7_encode_progress_units, encode_for_vram_with_bc7_rdo_lambda_and_progress,
    preferred_bc7_encoder_backend, ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_dir_matching;
use crate::{resolve_packing_axis, AtlasPackingMode};
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
const MOBILE_ANIM_CC_METADATA_VERSION: u32 = 2;
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
    pub atlas_width: u32,
    pub atlas_height: u32,
}

pub struct MobileAnimCcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub pixel_format: PagePixelFormat,
    pub bc7_rdo_lambda: f32,
}

impl Default for MobileAnimCcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
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
    Decoded(Vec<AnimFrame>),
}

#[derive(Debug, Clone)]
pub struct BuiltMobileAnimPage {
    pub record: MobileAnimCcPageRecord,
    pub pixels: Vec<u8>,
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
    validate_options(options)?;

    let client_dir = find_first_dir_matching(source_dirs, &[&["anim.idx", "anim.mul"]])
        .ok_or_else(|| eyre::eyre!(
            "no classic animation sources found in any provided path: expected anim.mul/anim.idx"
        ))?;

    info!(
        "Converting CC mobile animations from classic MUL format to {}",
        out_file.display()
    );
    println!("Using CC animation source dir: {}", client_dir.display());

    let anim_map = AnimMap::load(&client_dir)
        .wrap_err_with(|| format!("load animation sources from {}", client_dir.display()))?;
    let (decoded_frames, mut animation_records, mut frame_records) =
        decode_present_animations(&client_dir, &anim_map)?;
    let packed_frame_count = decoded_frames.len() as u32;
    let pages = pack_frames_into_pages(decoded_frames, &mut frame_records, options)?;
    let body_resolve_records = build_body_resolve_records(&client_dir)?;
    let body_type_records = build_body_type_records(&client_dir)?;

    let page_manifest = serialize_page_manifest(&pages, options)?;
    let animation_manifest = serialize_animation_manifest(&animation_records)?;
    let frame_manifest = serialize_frame_manifest(&frame_records)?;
    let body_resolve_manifest = serialize_body_resolve_manifest(&body_resolve_records)?;
    let body_type_manifest = serialize_body_type_manifest(&body_type_records)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
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

    let encoded_pages = encode_mobile_anim_pages(&pages, options)?;
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

    build_and_write_package(&mut package, out_file)?;

    Ok(MobileAnimCcBuildSummary {
        animation_count: animation_records.len() as u32,
        frame_count: frame_records.len() as u32,
        packed_frame_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
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

fn encode_mobile_anim_pages(
    pages: &[BuiltMobileAnimPage],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<(String, Vec<u8>, u32, u32)>> {
    let use_bc7 = options.pixel_format == PagePixelFormat::Bc7;
    let progress_len = if use_bc7 {
        let extent = ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?;
        pages.len() as u64 * bc7_encode_progress_units(extent, options.bc7_rdo_lambda) as u64
    } else {
        pages.len() as u64
    };
    let progress_message = if use_bc7 {
        "compressing mobile animation BC7 atlas blocks"
    } else {
        "encoding mobile animation atlas pages"
    };

    let pb = ProgressBar::new(progress_len);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(progress_message);

    let mut encoded_pages = Vec::with_capacity(pages.len());
    if use_bc7 {
        let extent = ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?;
        let encoding = VramTextureEncoding::Bc7(preferred_bc7_encoder_backend());
        for page in pages {
            let encoded = encode_for_vram_with_bc7_rdo_lambda_and_progress(
                &page.pixels,
                extent,
                RawImageFormat::Rgba8888,
                encoding,
                options.bc7_rdo_lambda,
                |units| pb.inc(units as u64),
            )
            .map_err(|e| eyre::eyre!("BC7 encode mobile animation page {}: {e}", page.record.page_index))?
            .into_bytes()
            .to_vec();
            encoded_pages.push((
                page_entry_path(page.record.page_index, PagePixelFormat::Bc7),
                encoded,
                options.atlas_width,
                options.atlas_height,
            ));
        }
    } else {
        for page in pages {
            pb.inc(1);
            encoded_pages.push((
                page_entry_path(page.record.page_index, PagePixelFormat::Rgba8888),
                crate::tex_art_cc::crop_rgba_page(
                    &page.pixels,
                    options.atlas_width,
                    page.record.used_width,
                    page.record.used_height,
                ),
                page.record.used_width,
                page.record.used_height,
            ));
        }
    }
    pb.finish_with_message("Mobile animation atlas pages encoded");
    Ok(encoded_pages)
}

fn decode_present_animations(
    client_dir: &Path,
    anim_map: &AnimMap,
) -> eyre::Result<(Vec<DecodedMobileAnimFrame>, Vec<MobileAnimCcAnimationRecord>, Vec<MobileAnimCcFrameRecord>)> {
    let mut decoded_frames = Vec::new();
    let mut animation_records = Vec::new();
    let mut frame_records = Vec::new();

    let mut candidates = collect_present_animation_candidates(anim_map);
    let animationframe_paths = discover_classic_animationframe_paths(client_dir);
    let animationframe_candidates =
        decode_classic_animationframe_packages(&animationframe_paths)?;
    candidates.extend(animationframe_candidates);

    let pb = ProgressBar::new(candidates.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("decoding mobile animations");

    let mut active_source = String::new();
    for candidate in candidates {
        pb.inc(1);
        let source_label = candidate_source_label(&candidate);
        if source_label != active_source {
            active_source = source_label;
            pb.set_message(format!("decoding {active_source}"));
        }
        let frames = match candidate.frames {
            PresentAnimationFrames::Mul => {
                let Ok(frames) = anim_map.decode_animation_index(candidate.file_index, candidate.source_index) else {
                    continue;
                };
                frames
            }
            PresentAnimationFrames::Decoded(frames) => frames,
        };
        if frames.is_empty() || frames.len() > u16::MAX as usize {
            continue;
        }

        let animation_index = animation_records.len() as u32;
        let frame_start = frame_records.len() as u32;
        for (frame_index, frame) in frames.iter().enumerate() {
            let global_frame_index = frame_records.len() as u32;
            frame_records.push(empty_frame_record(
                animation_index,
                frame_index as u16,
                frame,
            ));
            if frame.width != 0 && frame.height != 0 && !frame.data.is_empty() {
                decoded_frames.push(DecodedMobileAnimFrame {
                    global_frame_index,
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.data.clone(),
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
            frame_count: frames.len() as u16,
            flags: candidate.flags,
        });
    }
    pb.finish_with_message("Mobile animations decoded");

    Ok((decoded_frames, animation_records, frame_records))
}

fn candidate_source_label(candidate: &PresentAnimationCandidate) -> String {
    match &candidate.frames {
        PresentAnimationFrames::Mul => classic_anim_mul_name(candidate.file_index).to_string(),
        PresentAnimationFrames::Decoded(_) => format!(
            "AnimationFrame{}.uop anim_id {}",
            candidate.file_index + 1,
            candidate.source_index
        ),
    }
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
        let package = UopPackage::load(path)
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
    let files = package.iter_files().filter(|file| file.has_size()).collect::<Vec<_>>();
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("AnimationFrame*.uop");
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("reading {file_name}"));

    let mut candidates = Vec::new();
    for file in files {
        pb.inc(1);
        let Ok(data) = file.unpack() else { continue; };
        let Ok(animation) = AnimationFrameCc::parse(&data) else { continue; };
        if animation.frames.is_empty() || animation.frames.len() > u16::MAX as usize {
            continue;
        }
        let Some((body_id, action_id, direction)) =
            animation_layout_from_source_index(file_index, animation.anim_id)
        else {
            continue;
        };
        candidates.push(PresentAnimationCandidate {
            body_id,
            action_id,
            direction,
            file_index,
            source_index: animation.anim_id,
            flags: 0,
            frames: PresentAnimationFrames::Decoded(animation.frames),
        });
    }
    pb.finish_with_message(format!("{file_name} read"));
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
    frame: &AnimFrame,
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
    let mut remaining = frames;
    let total_frames = remaining.len() as u64;
    remaining.sort_by_key(|frame| frame.global_frame_index);
    let mut pages = Vec::new();
    let mut page_index = 0u32;
    let pb = ProgressBar::new(total_frames);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("packing mobile animation atlas pages");

    while !remaining.is_empty() {
        pb.set_message(format!("packing mobile animation atlas page {page_index}"));
        let (page_frames, leftovers) = take_page_frame_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_frames, frame_records, options)?;
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }

        pb.inc(page.record.frame_count as u64);
        pages.push(page);
        remaining = leftovers;
        remaining.extend(unplaced);
        remaining.sort_by_key(|frame| frame.global_frame_index);
        page_index += 1;
    }
    pb.finish_with_message(format!("Mobile animation atlas pages packed ({page_index} pages)"));

    Ok(pages)
}

fn take_page_frame_prefix(
    frames: Vec<DecodedMobileAnimFrame>,
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(Vec<DecodedMobileAnimFrame>, Vec<DecodedMobileAnimFrame>)> {
    let prefix_len = max_fitting_page_prefix_len(&frames, options)?;
    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any mobile animation frame into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }

    let mut leftovers = frames;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((selected, leftovers))
}

fn max_fitting_page_prefix_len(
    frames: &[DecodedMobileAnimFrame],
    options: &MobileAnimCcAtlasOptions,
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
    frames: &[DecodedMobileAnimFrame],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<bool> {
    let mut to_pack = frames.to_vec();
    sort_frames_within_page(&mut to_pack, options);

    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));

    for frame in &to_pack {
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
    mut frames: Vec<DecodedMobileAnimFrame>,
    frame_records: &mut [MobileAnimCcFrameRecord],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<(BuiltMobileAnimPage, Vec<DecodedMobileAnimFrame>)> {
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

    Ok((
        BuiltMobileAnimPage {
            record: MobileAnimCcPageRecord {
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
    frame: &DecodedMobileAnimFrame,
    options: &MobileAnimCcAtlasOptions,
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
            "mobile animation frame {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
            frame.global_frame_index,
            frame.width,
            frame.height,
            options.atlas_width,
            options.atlas_height,
            options.gutter
        ),
    }
}

fn sort_frames_within_page(frames: &mut [DecodedMobileAnimFrame], options: &MobileAnimCcAtlasOptions) {
    frames.sort_by(|left, right| {
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| left.global_frame_index.cmp(&right.global_frame_index))
    });
}

fn sort_area(frame: &DecodedMobileAnimFrame, options: &MobileAnimCcAtlasOptions) -> u32 {
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
            "invalid RGBA payload length for mobile animation frame {}x{}: expected {}, got {}",
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
    pages: &[BuiltMobileAnimPage],
    options: &MobileAnimCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(26 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(MOBILE_ANIM_CC_METADATA_VERSION)?;
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
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
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
    fn serialized_package_roundtrips_through_reader() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 4,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
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
        let stored_page = crate::tex_art_cc::crop_rgba_page(
            &pages[0].pixels,
            options.atlas_width,
            pages[0].record.used_width,
            pages[0].record.used_height,
        );

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
