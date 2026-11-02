//! Custom animations added to the MUL files survive conversion, whatever tool wrote them and wherever it put them. The
//! client folder here is synthetic, so no real client is needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use udd_assets::MobileAnimCcPackage;
use udd_conv::mobile_anim_cc::{
    animation_source_index, convert_anim_mul_to_mobile_anim_cc_uddp, MobileAnimCcAtlasOptions,
    MobileAnimCcBuildSummary,
};
use udd_conv::{CompressionFlag, PagePixelFormat};

const PALETTE_ENTRIES: usize = 256;
const RUN_END_MARKER: u32 = 0x7FFF_7FFF;
const RUN_OFFSET_MASK: u32 = 0x3FF;
const EMPTY_LOOKUP: u32 = u32::MAX;
/// Past the 148,810 slots of the shipped `anim.idx`.
const LONG_IDX_BODY: u16 = 1100;
const OPAQUE: u8 = u8::MAX;
const ANIM_MUL_FILE: u8 = 0;

/// One run, at image coordinates of the frame's declared rectangle, of one palette index.
struct Run {
    x: i32,
    y: i32,
    len: usize,
    index: u8,
}

struct Frame {
    center_x: i16,
    center_y: i16,
    width: u16,
    height: u16,
    runs: Vec<Run>,
}

fn entry(palette: &[u16; PALETTE_ENTRIES], frames: &[Frame]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in palette {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    let frame_bytes: Vec<Vec<u8>> = frames.iter().map(frame_bytes).collect();
    bytes.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    // Offsets count from the end of the palette: the count, the table, then the frames.
    let mut offset = (size_of::<u32>() * (frames.len() + 1)) as u32;
    for frame in &frame_bytes {
        bytes.extend_from_slice(&offset.to_le_bytes());
        offset += frame.len() as u32;
    }
    bytes.extend(frame_bytes.into_iter().flatten());
    bytes
}

fn frame_bytes(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&frame.center_x.to_le_bytes());
    bytes.extend_from_slice(&frame.center_y.to_le_bytes());
    bytes.extend_from_slice(&frame.width.to_le_bytes());
    bytes.extend_from_slice(&frame.height.to_le_bytes());
    for run in &frame.runs {
        let x_offset = run.x - i32::from(frame.center_x);
        let y_offset = run.y - i32::from(frame.center_y) - i32::from(frame.height);
        let header = ((x_offset as u32 & RUN_OFFSET_MASK) << 22)
            | ((y_offset as u32 & RUN_OFFSET_MASK) << 12)
            | run.len as u32;
        bytes.extend_from_slice(&header.to_le_bytes());
        bytes.extend(std::iter::repeat_n(run.index, run.len));
    }
    bytes.extend_from_slice(&RUN_END_MARKER.to_le_bytes());
    bytes
}

/// A solid frame of one palette index, anchored at its top-left corner.
fn solid_frame(width: u16, height: u16, index: u8) -> Frame {
    Frame {
        center_x: 0,
        center_y: -(height as i16),
        width,
        height,
        runs: (0..i32::from(height)).map(|y| Run { x: 0, y, len: usize::from(width), index }).collect(),
    }
}

fn palette_with(index: u8, word: u16) -> [u16; PALETTE_ENTRIES] {
    let mut palette = [0x7FFFu16; PALETTE_ENTRIES];
    palette[usize::from(index)] = word;
    palette
}

/// Writes `anim.idx` and `anim.mul`, one idx row per slot up to the highest used. `orphan` goes in the data file with
/// no idx row, as when a tool rewrites a body and leaves its old bytes behind.
fn write_client(dir: &Path, orphan: &[u8], entries: &[(u32, Vec<u8>)]) {
    let slots = entries.iter().map(|(slot, _)| *slot).max().unwrap() as usize + 1;
    let mut idx: Vec<[u32; 3]> = vec![[EMPTY_LOOKUP, 0, 0]; slots];
    let mut mul = orphan.to_vec();
    for (slot, bytes) in entries {
        idx[*slot as usize] = [mul.len() as u32, bytes.len() as u32, 0];
        mul.extend_from_slice(bytes);
    }
    fs::write(dir.join("anim.mul"), mul).unwrap();
    fs::write(dir.join("anim.idx"), idx.iter().flatten().flat_map(|word| word.to_le_bytes()).collect::<Vec<u8>>())
        .unwrap();
}

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).expect("system time is after unix epoch").as_nanos();
    let dir = std::env::temp_dir().join(format!("uddconv_{test_name}_{timestamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn convert(dir: &Path) -> (MobileAnimCcPackage, MobileAnimCcBuildSummary) {
    let out_file = dir.join("mobile_anim_cc.uddp");
    let options = MobileAnimCcAtlasOptions {
        compression: CompressionFlag::None,
        pixel_format: PagePixelFormat::Rgba8888,
        ..MobileAnimCcAtlasOptions::default()
    };
    let summary = convert_anim_mul_to_mobile_anim_cc_uddp(dir, &out_file, &options).expect("convert");
    (MobileAnimCcPackage::load(&out_file).expect("load package"), summary)
}

fn slot(body: u16, action: u16, direction: u8) -> u32 {
    animation_source_index(ANIM_MUL_FILE, body, action, direction)
}

/// The RGBA of a body's frame 0, with its size and anchor.
fn frame_0(package: &MobileAnimCcPackage, body: u16) -> (u16, u16, i16, i16, Vec<u8>) {
    let animation = package.animation(body, 0, 0).unwrap_or_else(|| panic!("body {body} lost"));
    let frame = package.animation_frames(animation)[0];
    (frame.width, frame.height, frame.center_x, frame.center_y, package.read_frame_rgba(&frame).unwrap())
}

/// Each opaque pixel's offset from the anchor, by the client's formula, in image order.
fn opaque_anchor_offsets((width, height, center_x, center_y, pixels): &(u16, u16, i16, i16, Vec<u8>)) -> Vec<(i32, i32)> {
    let (width, height) = (i32::from(*width), i32::from(*height));
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| pixels[(y * width + x) as usize * 4 + 3] != 0)
        .map(|(x, y)| (x - i32::from(*center_x), y - i32::from(*center_y) - height))
        .collect()
}

#[test]
fn custom_mul_animations_survive_conversion() {
    const ADDED_BODY: u16 = 150;
    const REPLACED_BODY: u16 = 1;
    const INDEX_ZERO_BODY: u16 = 151;
    const OVERSIZED_BODY: u16 = 152;
    const OVERSIZED_WIDTH: u16 = 1500;
    const OVERSIZED_HEIGHT: u16 = 900;
    const RED: u16 = 0x7C00;
    const GREEN: u16 = 0x03E0;
    const BLACK: u16 = 0;
    const BYTES_PER_PIXEL: usize = 4;
    let dir = temp_dir("custom_mul");

    let old_art = entry(&palette_with(1, GREEN), &[solid_frame(3, 3, 1)]);
    let oversized = Frame {
        center_x: 512,
        center_y: -388,
        width: OVERSIZED_WIDTH,
        height: OVERSIZED_HEIGHT,
        runs: vec![
            Run { x: 0, y: 0, len: usize::from(OVERSIZED_WIDTH), index: 1 },
            Run { x: 0, y: i32::from(OVERSIZED_HEIGHT) - 1, len: usize::from(OVERSIZED_WIDTH), index: 1 },
        ],
    };
    write_client(
        &dir,
        &old_art,
        &[
            (slot(REPLACED_BODY, 0, 0), entry(&palette_with(1, RED), &[solid_frame(2, 2, 1)])),
            (slot(ADDED_BODY, 0, 0), entry(&palette_with(1, RED), &[solid_frame(4, 3, 1)])),
            (slot(INDEX_ZERO_BODY, 0, 0), entry(&palette_with(0, BLACK), &[solid_frame(2, 1, 0)])),
            (slot(OVERSIZED_BODY, 0, 0), entry(&palette_with(1, RED), &[oversized])),
            (slot(LONG_IDX_BODY, 0, 0), entry(&palette_with(1, RED), &[solid_frame(1, 1, 1)])),
        ],
    );

    let (package, summary) = convert(&dir);

    assert_eq!(summary.animation_count, 5, "the orphaned old art has no idx row, so it isn't an animation");
    assert_eq!(summary.skipped_animation_count, 0);

    let red = [0xF8, 0, 0, OPAQUE];
    let (width, height, .., pixels) = frame_0(&package, REPLACED_BODY);
    assert_eq!((width, height), (2, 2), "the idx points at the replacement, not at the old bytes");
    assert_eq!(pixels[..BYTES_PER_PIXEL], red);

    let (width, height, .., pixels) = frame_0(&package, ADDED_BODY);
    assert_eq!((width, height), (4, 3));
    assert_eq!(pixels[..BYTES_PER_PIXEL], red);

    let (_, _, .., pixels) = frame_0(&package, INDEX_ZERO_BODY);
    assert_eq!(pixels[..BYTES_PER_PIXEL], [0, 0, 0, OPAQUE], "the client draws index 0 and a word of 0 as black");

    let (width, height, ..) = frame_0(&package, OVERSIZED_BODY);
    assert_eq!((width, height), (OVERSIZED_WIDTH, OVERSIZED_HEIGHT));

    frame_0(&package, LONG_IDX_BODY);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn runs_outside_the_declared_size_are_kept_at_their_anchor_offsets() {
    const BODY: u16 = 150;
    let dir = temp_dir("custom_mul_runs");
    // Declared 2x2 anchored at (0, -2), so image pixel (x, y) has offsets (x, y). The runs reach one row above and
    // three columns right of the declared rectangle.
    let frame = Frame {
        center_x: 0,
        center_y: -2,
        width: 2,
        height: 2,
        runs: vec![Run { x: 0, y: -1, len: 1, index: 1 }, Run { x: 4, y: 1, len: 1, index: 1 }],
    };
    write_client(&dir, &[], &[(slot(BODY, 0, 0), entry(&palette_with(1, 0x7C00), &[frame]))]);

    let (package, _) = convert(&dir);

    let frame = frame_0(&package, BODY);
    assert_eq!((frame.0, frame.1), (5, 3));
    assert_eq!(opaque_anchor_offsets(&frame), vec![(0, -1), (4, 1)]);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn trimming_keeps_pixels_at_their_anchor_offsets() {
    const BODY: u16 = 150;
    let dir = temp_dir("custom_mul_trim");
    // Declared 6x8 anchored at (2, -5), so image pixel (x, y) has offsets (x - 2, y - 3). Transparent space lies on all
    // four sides, and more of it above than below.
    let frame = Frame {
        center_x: 2,
        center_y: -5,
        width: 6,
        height: 8,
        runs: vec![Run { x: 1, y: 4, len: 1, index: 1 }, Run { x: 3, y: 6, len: 1, index: 1 }],
    };
    write_client(&dir, &[], &[(slot(BODY, 0, 0), entry(&palette_with(1, 0x7C00), &[frame]))]);

    let (package, _) = convert(&dir);

    assert_eq!(opaque_anchor_offsets(&frame_0(&package, BODY)), vec![(-1, 1), (1, 3)]);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unreadable_entries_are_counted_and_the_rest_is_kept() {
    const GOOD_BODY: u16 = 150;
    const TOO_MANY_FRAMES_BODY: u16 = 151;
    const NO_FRAMES_BODY: u16 = 152;
    const UNREADABLE_FRAME_COUNT: u32 = 5000;
    let dir = temp_dir("custom_mul_broken");
    let mut too_many_frames = entry(&palette_with(1, 0x7C00), &[]);
    too_many_frames.truncate(PALETTE_ENTRIES * size_of::<u16>());
    too_many_frames.extend_from_slice(&UNREADABLE_FRAME_COUNT.to_le_bytes());
    write_client(
        &dir,
        &[],
        &[
            (slot(GOOD_BODY, 0, 0), entry(&palette_with(1, 0x7C00), &[solid_frame(2, 2, 1)])),
            (slot(TOO_MANY_FRAMES_BODY, 0, 0), too_many_frames),
            (slot(NO_FRAMES_BODY, 0, 0), entry(&palette_with(1, 0x7C00), &[])),
        ],
    );

    let (package, summary) = convert(&dir);

    assert_eq!(summary.animation_count, 1);
    assert_eq!(summary.skipped_animation_count, 2);
    frame_0(&package, GOOD_BODY);

    fs::remove_dir_all(&dir).unwrap();
}
