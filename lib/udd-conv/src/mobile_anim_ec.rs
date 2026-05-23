//! Build-time support for `mobile_anim_ec.uddp`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::AnimationFrame;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_dir_matching;
use crate::{resolve_packing_axis, AtlasPackingMode};
use udd_assets::mobile_anim_ec::{
    page_entry_path, MobileAnimEcAnimationRecord, MobileAnimEcFrameRecord,
    MobileAnimEcPageRecord, ANIMATION_MANIFEST_ENTRY_PATH, FRAME_MANIFEST_ENTRY_PATH,
    MISSING_PAGE_FRAME_INDEX, MISSING_PAGE_INDEX, PAGE_MANIFEST_ENTRY_PATH,
};
use udd_assets::tex_art_cc::PagePixelFormat;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 4;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcBuildSummary {
    pub body_count: u32,
    pub animation_count: u32,
    pub frame_count: u32,
    pub packed_frame_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

pub struct MobileAnimEcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub pixel_format: PagePixelFormat,
}

impl Default for MobileAnimEcAtlasOptions {
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

pub fn convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &MobileAnimEcAtlasOptions,
) -> eyre::Result<MobileAnimEcBuildSummary> {
    validate_options(options)?;
    let client_dir = find_first_dir_matching(source_dirs, &[&["AnimationFrame.uop"], &["animationframe.uop"]])
        .ok_or_else(|| eyre::eyre!(
            "no EC animation sources found in any provided path: expected AnimationFrame.uop"
        ))?;
    let animationframe_path = ["AnimationFrame.uop", "animationframe.uop"]
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file())
        .context("missing AnimationFrame.uop")?;

    info!(
        "Converting EC mobile animations from {} to {}",
        animationframe_path.display(),
        out_file.display()
    );
    println!("Using EC animation source dir: {}", client_dir.display());

    let animationframe_package = UopPackage::load(&animationframe_path)
        .wrap_err_with(|| format!("load {}", animationframe_path.display()))?;
    let decoded_by_body = decode_animationframe_package(&animationframe_package)?;
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

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    for (path, data) in [
        (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice()),
        (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice()),
        (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice()),
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

    Ok(MobileAnimEcBuildSummary {
        body_count: decoded_by_body.len() as u32,
        animation_count: animations.len() as u32,
        frame_count: frames.len() as u32,
        packed_frame_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &MobileAnimEcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("EC mobile animation atlas dimensions must be non-zero");
    }
    if options.pixel_format != PagePixelFormat::Rgba8888 {
        eyre::bail!("mobile_anim_ec currently supports RGBA8888 pages only");
    }
    Ok(())
}

fn decode_animationframe_package(
    package: &UopPackage,
) -> eyre::Result<BTreeMap<u32, Vec<DecodedMobileAnimEcFrame>>> {
    let files = package.iter_files().filter(|file| file.has_size()).collect::<Vec<_>>();
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding EC mobile animations ({eta})")
        .unwrap()
        .progress_chars("#>-"));

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
        decoded_by_body.insert(animation.animation_id, frames);
    }
    pb.finish_with_message("EC mobile animations decoded");
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
    for body_id in body_ids {
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
    remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
    let mut pages = Vec::new();
    let mut placements = HashMap::new();
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page_frames, leftovers) = take_page_frame_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_frames, &mut placements, options)?;
        if page.record.frame_count == 0 {
            eyre::bail!(
                "could not fit any EC mobile animation frame into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }
        pages.push(page);
        remaining = leftovers;
        remaining.extend(unplaced);
        remaining.sort_by_key(|frame| (frame.body_id, frame.source_frame_index));
        page_index += 1;
    }

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
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| (left.body_id, left.source_frame_index).cmp(&(right.body_id, right.source_frame_index)))
    });
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
        };
        let (_, placements) = pack_frames_into_pages(source_frames[&42].clone(), &options).unwrap();
        let (animations, frames) = build_animation_records(&source_frames, &placements, &HashMap::new()).unwrap();

        assert_eq!(animations[0].frame_count, 2);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].source_frame_index, 1);
        assert_eq!(frames[1].page_index, MISSING_PAGE_INDEX);
        assert_eq!(frames[1].page_frame_index, MISSING_PAGE_FRAME_INDEX);
    }
}
