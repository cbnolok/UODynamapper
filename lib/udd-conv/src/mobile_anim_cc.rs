//! Build-time support for `mobile_anim_cc.uddp`.
//!
//! This first package version targets the classic `anim*.mul` / `anim*.idx`
//! mobile-animation sources. The atlas packer uses 4-pixel-aligned content
//! extents so the same metadata remains valid when page payloads gain BC7
//! support later.

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
use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;

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
}

impl Default for MobileAnimCcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
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
        decode_present_animations(&anim_map)?;
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

    for page in &pages {
        let page_path = page_entry_path(page.record.page_index, options.pixel_format);
        let stored_page = crate::tex_art_cc::crop_rgba_page(
            &page.pixels,
            options.atlas_width,
            page.record.used_width,
            page.record.used_height,
        );
        package.add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width: 0,
            height: 0,
            virtual_path: Some(&page_path),
            path_hash64: None,
            id: None,
            data: &stored_page,
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
    if options.pixel_format != PagePixelFormat::Rgba8888 {
        eyre::bail!("mobile_anim_cc currently supports RGBA8888 pages only");
    }
    Ok(())
}

fn decode_present_animations(
    anim_map: &AnimMap,
) -> eyre::Result<(Vec<DecodedMobileAnimFrame>, Vec<MobileAnimCcAnimationRecord>, Vec<MobileAnimCcFrameRecord>)> {
    let mut decoded_frames = Vec::new();
    let mut animation_records = Vec::new();
    let mut frame_records = Vec::new();
    let total_candidate_count = candidate_count(anim_map);
    let pb = ProgressBar::new(total_candidate_count as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding mobile animations ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    for file_index in 0..MAX_ANIM_FILES {
        let Some(source_index_count) = anim_map.source_index_count(file_index) else {
            continue;
        };
        let body_count = body_count_for_source(file_index, source_index_count);
        for body_id in 0..body_count {
            let action_count = action_count_for_body(file_index, body_id);
            for action_id in 0..action_count {
                for direction in 0..5u8 {
                    pb.inc(1);
                    let source_index = animation_source_index(file_index, body_id, action_id, direction);
                    if source_index as usize >= source_index_count
                        || !anim_map.has_anim(file_index, source_index)
                    {
                        continue;
                    }
                    let Ok(frames) = anim_map.decode_animation_index(file_index, source_index) else {
                        continue;
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
                        body_id: body_id as u16,
                        action_id: action_id as u16,
                        direction,
                        file_index,
                        source_index,
                        frame_start,
                        frame_count: frames.len() as u16,
                        flags: 0,
                    });
                }
            }
        }
    }
    pb.finish_with_message("Mobile animations decoded");

    Ok((decoded_frames, animation_records, frame_records))
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

fn candidate_count(anim_map: &AnimMap) -> usize {
    let mut count = 0usize;
    for file_index in 0..MAX_ANIM_FILES {
        if let Some(source_index_count) = anim_map.source_index_count(file_index) {
            let body_count = body_count_for_source(file_index, source_index_count);
            for body_id in 0..body_count {
                count += action_count_for_body(file_index, body_id) as usize * 5;
            }
        }
    }
    count
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
                22000 + ((body - 200) * 65)
            } else {
                35000 + ((body - 400) * 175)
            }
        }
    };

    index += action_id as u32 * 5;
    index += direction.min(4) as u32;
    index
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
    remaining.sort_by_key(|frame| frame.global_frame_index);
    let mut pages = Vec::new();
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page_frames, leftovers) = take_page_frame_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_frames, frame_records, options)?;
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }

        pages.push(page);
        remaining = leftovers;
        remaining.extend(unplaced);
        remaining.sort_by_key(|frame| frame.global_frame_index);
        page_index += 1;
    }

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
        assert_eq!(animation_source_index(0, 0, 2, 3), 13);
    }

    #[test]
    fn packer_uses_four_pixel_aligned_extents() {
        let options = MobileAnimCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 0,
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Rgba8888,
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
