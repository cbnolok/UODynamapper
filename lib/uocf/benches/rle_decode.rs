use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use uocf::classic::anim::AnimMap;
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::gump::decode_gump_from_raw;
use uocf::enhanced::animationframe::AnimationFrame;

const GUMP_WIDTH: u16 = 128;
const GUMP_HEIGHT: u16 = 128;
const ANIM_WIDTH: u16 = 128;
const ANIM_HEIGHT: u16 = 128;
const SMALL_ANIM_WIDTH: u16 = 32;
const SMALL_ANIM_HEIGHT: u16 = 32;
const MANY_ANIM_FRAMES: u32 = 32;
const EC_ANIM_WIDTH: u16 = 128;
const EC_ANIM_HEIGHT: u16 = 128;

struct GumpCase {
    name: &'static str,
    width: u16,
    height: u16,
    payload: Vec<u8>,
}

struct AnimCase {
    name: &'static str,
    map: AnimMap,
    cleanup_dir: PathBuf,
}

struct EcAnimCase {
    name: &'static str,
    animation: AnimationFrame,
}

struct CcAnimCase {
    name: &'static str,
    payload: Vec<u8>,
}

impl Drop for AnimCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.cleanup_dir);
    }
}

struct BenchResult {
    iterations: u64,
    elapsed: Duration,
    checksum: u64,
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let quick = args.iter().any(|arg| arg == "--quick");
    let min_duration = if quick {
        Duration::from_millis(120)
    } else {
        Duration::from_millis(700)
    };

    println!("rle_decode benchmark");
    println!("profile={} min_duration_ms={}", profile_name(), min_duration.as_millis());

    let gump_cases = [
        gump_case("gump_solid_rows", GumpRunPattern::SolidRows),
        gump_case("gump_alternating_pixels", GumpRunPattern::AlternatingPixels),
        gump_case("gump_mixed_short_runs", GumpRunPattern::MixedShortRuns),
        gump_case("gump_transparent_rows", GumpRunPattern::TransparentRows),
    ];
    for case in &gump_cases {
        let result = bench_gump(case, min_duration);
        print_result(case.name, "decode", result);
    }

    let anim_cases = [
        anim_case("anim_solid_rows", AnimRunPattern::SolidRows),
        anim_case("anim_single_pixel_runs", AnimRunPattern::SinglePixelRuns),
        anim_case_with_frames(
            "anim_many_small_frames",
            SMALL_ANIM_WIDTH,
            SMALL_ANIM_HEIGHT,
            MANY_ANIM_FRAMES,
            AnimRunPattern::SolidRows,
        ),
    ];
    for case in &anim_cases {
        let result = bench_anim(case, min_duration);
        print_result(case.name, "decode", result);
    }
    let result = bench_anim_metadata(&anim_cases[2], min_duration);
    print_result(anim_cases[2].name, "metadata", result);

    let cc_anim_cases = [
        cc_anim_case(
            "cc_anim_solid_rows",
            ANIM_WIDTH,
            ANIM_HEIGHT,
            1,
            AnimRunPattern::SolidRows,
        ),
        cc_anim_case(
            "cc_anim_many_small_frames",
            SMALL_ANIM_WIDTH,
            SMALL_ANIM_HEIGHT,
            MANY_ANIM_FRAMES,
            AnimRunPattern::SolidRows,
        ),
    ];
    for case in &cc_anim_cases {
        let result = bench_cc_anim(case, min_duration);
        print_result(case.name, "parse", result);
    }
    let result = bench_cc_anim_metadata(&cc_anim_cases[1], min_duration);
    print_result(cc_anim_cases[1].name, "metadata", result);

    let ec_anim_cases = [
        ec_anim_case("ec_anim_solid_runs", EcAnimRunPattern::SolidRuns),
        ec_anim_case("ec_anim_skip_solid_pairs", EcAnimRunPattern::SkipSolidPairs),
        ec_anim_case("ec_anim_blend_pairs", EcAnimRunPattern::BlendPairs),
    ];
    for case in &ec_anim_cases {
        let result = bench_ec_anim(case, min_duration);
        print_result(case.name, "decode", result);
    }
}

fn bench_gump(case: &GumpCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let decoded = decode_gump_from_raw(
            black_box(&case.payload),
            black_box(case.width),
            black_box(case.height),
        )
        .expect("synthetic gump RLE payload should decode");
        checksum_acc = checksum_acc.wrapping_add(checksum(black_box(&decoded)));
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn bench_anim_metadata(case: &AnimCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let frames = case
            .map
            .decode_animation_metadata(black_box(0), black_box(0))
            .expect("synthetic animation metadata should decode");
        let frame_checksum = frames.iter().fold(0u64, |sum, frame| {
            sum.wrapping_add(u64::from(frame.width))
                .wrapping_add(u64::from(frame.height) << 16)
                .wrapping_add((frame.center_x as i64 as u64).rotate_left(17))
                .wrapping_add((frame.center_y as i64 as u64).rotate_left(31))
        });
        checksum_acc = checksum_acc.wrapping_add(black_box(frame_checksum));
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn bench_cc_anim(case: &CcAnimCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let animation = AnimationFrameCc::parse(black_box(&case.payload))
            .expect("synthetic CC AnimationFrame payload should parse");
        let frame_checksum = animation
            .frames
            .iter()
            .fold(0u64, |sum, frame| sum.wrapping_add(checksum(black_box(&frame.data))));
        checksum_acc = checksum_acc.wrapping_add(frame_checksum);
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn bench_cc_anim_metadata(case: &CcAnimCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let animation = AnimationFrameCc::parse_metadata(black_box(&case.payload))
            .expect("synthetic CC AnimationFrame metadata should parse");
        let frame_checksum = animation.frames.iter().fold(0u64, |sum, frame| {
            sum.wrapping_add(u64::from(frame.width))
                .wrapping_add(u64::from(frame.height) << 16)
                .wrapping_add((frame.center_x as i64 as u64).rotate_left(17))
                .wrapping_add((frame.center_y as i64 as u64).rotate_left(31))
        });
        checksum_acc = checksum_acc.wrapping_add(black_box(frame_checksum));
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn bench_anim(case: &AnimCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let frames = case
            .map
            .decode_animation(black_box(0), black_box(0))
            .expect("synthetic animation RLE payload should decode");
        let frame_checksum = frames
            .iter()
            .fold(0u64, |sum, frame| sum.wrapping_add(checksum(black_box(&frame.data))));
        checksum_acc = checksum_acc.wrapping_add(frame_checksum);
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn bench_ec_anim(case: &EcAnimCase, min_duration: Duration) -> BenchResult {
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let frame_entry = case.animation.frames[0];
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        let frame = case
            .animation
            .decode_frame(black_box(&frame_entry))
            .expect("synthetic EC animation RLE payload should decode");
        checksum_acc = checksum_acc.wrapping_add(checksum(black_box(&frame.data)));
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn print_result(case_name: &str, operation: &str, result: BenchResult) {
    let elapsed_ns = result.elapsed.as_nanos() as f64;
    let ns_per_iter = elapsed_ns / result.iterations as f64;
    let iters_per_sec = 1_000_000_000.0 / ns_per_iter;
    println!(
        "{case_name:28} {operation:8} {:>12.2} decode/s {:>10.1} ns/decode iterations={} checksum={:016x}",
        iters_per_sec,
        ns_per_iter,
        result.iterations,
        result.checksum
    );
}

#[derive(Clone, Copy)]
enum GumpRunPattern {
    SolidRows,
    AlternatingPixels,
    MixedShortRuns,
    TransparentRows,
}

fn gump_case(name: &'static str, pattern: GumpRunPattern) -> GumpCase {
    GumpCase {
        name,
        width: GUMP_WIDTH,
        height: GUMP_HEIGHT,
        payload: build_gump_payload(GUMP_WIDTH, GUMP_HEIGHT, pattern),
    }
}

fn build_gump_payload(width: u16, height: u16, pattern: GumpRunPattern) -> Vec<u8> {
    let mut payload = vec![0u8; height as usize * 4];
    let mut rle = Vec::new();
    for y in 0..height {
        let lookup = ((payload.len() + rle.len()) / 4) as u32;
        payload[y as usize * 4..y as usize * 4 + 4].copy_from_slice(&lookup.to_le_bytes());
        append_gump_row(&mut rle, width, y, pattern);
    }
    payload.extend_from_slice(&rle);
    payload
}

fn append_gump_row(out: &mut Vec<u8>, width: u16, y: u16, pattern: GumpRunPattern) {
    match pattern {
        GumpRunPattern::SolidRows => append_gump_run(out, 0x7C00, width),
        GumpRunPattern::TransparentRows => append_gump_run(out, 0, width),
        GumpRunPattern::AlternatingPixels => {
            for x in 0..width {
                let color = if (x + y) & 1 == 0 { 0x7C00 } else { 0 };
                append_gump_run(out, color, 1);
            }
        }
        GumpRunPattern::MixedShortRuns => {
            let mut x = 0u16;
            let mut state = u32::from(y).wrapping_mul(0x9E37_79B9) ^ 0xA5A5_5A5A;
            while x < width {
                state = xorshift32(state);
                let run = ((state & 7) + 1).min(u32::from(width - x)) as u16;
                let color = if (state & 0x100) == 0 { 0x03E0 } else { 0 };
                append_gump_run(out, color, run);
                x += run;
            }
        }
    }
}

fn append_gump_run(out: &mut Vec<u8>, color: u16, run: u16) {
    out.extend_from_slice(&color.to_le_bytes());
    out.extend_from_slice(&run.to_le_bytes());
}

#[derive(Clone, Copy)]
enum AnimRunPattern {
    SolidRows,
    SinglePixelRuns,
}

fn anim_case(name: &'static str, pattern: AnimRunPattern) -> AnimCase {
    anim_case_with_frames(name, ANIM_WIDTH, ANIM_HEIGHT, 1, pattern)
}

fn anim_case_with_frames(
    name: &'static str,
    width: u16,
    height: u16,
    frame_count: u32,
    pattern: AnimRunPattern,
) -> AnimCase {
    let dir = unique_temp_dir(name);
    fs::create_dir_all(&dir).expect("create animation benchmark directory");
    let payload = build_anim_payload(width, height, frame_count, pattern);
    write_anim_pair(&dir, &payload).expect("write animation benchmark files");
    let map = AnimMap::load(&dir).expect("load animation benchmark files");
    AnimCase {
        name,
        map,
        cleanup_dir: dir,
    }
}

fn build_anim_payload(width: u16, height: u16, frame_count: u32, pattern: AnimRunPattern) -> Vec<u8> {
    let mut payload = vec![0u8; 256 * 2];
    payload[2..4].copy_from_slice(&0x7C00u16.to_le_bytes());
    payload[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
    payload.extend_from_slice(&frame_count.to_le_bytes());
    let offsets_start = payload.len();
    payload.resize(offsets_start + frame_count as usize * 4, 0);

    for frame_index in 0..frame_count as usize {
        let offset = (payload.len() - 512) as u32;
        payload[offsets_start + frame_index * 4..offsets_start + frame_index * 4 + 4]
            .copy_from_slice(&offset.to_le_bytes());
        append_anim_frame(&mut payload, width, height, pattern);
    }

    payload
}

fn append_anim_frame(out: &mut Vec<u8>, width: u16, height: u16, pattern: AnimRunPattern) {
    out.extend_from_slice(&0i16.to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());

    append_anim_rle(out, width, height, pattern);
    out.extend_from_slice(&0x7FFF7FFFu32.to_le_bytes());
}

fn append_anim_rle(out: &mut Vec<u8>, width: u16, height: u16, pattern: AnimRunPattern) {
    for y in 0..height {
        match pattern {
            AnimRunPattern::SolidRows => {
                append_anim_run(
                    out,
                    0,
                    y,
                    width,
                    height,
                    std::iter::repeat_n(1, width as usize),
                );
            }
            AnimRunPattern::SinglePixelRuns => {
                for x in 0..width {
                    let index = if (x + y) & 1 == 0 { 1 } else { 2 };
                    append_anim_run(out, x, y, 1, height, std::iter::once(index));
                }
            }
        }
    }
}

fn append_anim_run<I>(out: &mut Vec<u8>, x: u16, y: u16, run: u16, height: u16, indices: I)
where
    I: IntoIterator<Item = u8>,
{
    let y_offset = encode_anim_signed_10(i32::from(y) - i32::from(height));
    let x_offset = encode_anim_signed_10(i32::from(x));
    let header = u32::from(run) | (u32::from(y_offset) << 12) | (u32::from(x_offset) << 22);
    out.extend_from_slice(&header.to_le_bytes());
    for index in indices {
        out.push(index);
    }
}

fn encode_anim_signed_10(value: i32) -> u16 {
    debug_assert!((-512..=511).contains(&value));
    (value & 0x3FF) as u16
}

fn write_anim_pair(dir: &Path, payload: &[u8]) -> std::io::Result<()> {
    let mut idx = fs::File::create(dir.join("anim.idx"))?;
    idx.write_all(&0u32.to_le_bytes())?;
    idx.write_all(&(payload.len() as u32).to_le_bytes())?;
    idx.write_all(&0u32.to_le_bytes())?;
    fs::write(dir.join("anim.mul"), payload)
}

fn cc_anim_case(
    name: &'static str,
    width: u16,
    height: u16,
    frame_count: u32,
    pattern: AnimRunPattern,
) -> CcAnimCase {
    CcAnimCase {
        name,
        payload: build_cc_animation_payload(width, height, frame_count, pattern),
    }
}

fn build_cc_animation_payload(
    width: u16,
    height: u16,
    frame_count: u32,
    pattern: AnimRunPattern,
) -> Vec<u8> {
    let header_size = 40usize;
    let entry_size = 16usize;
    let first_frame_address = header_size as u32;
    let mut payload = Vec::new();

    push_u32(&mut payload, 1);
    push_u32(&mut payload, 1);
    push_u32(&mut payload, 0);
    push_u32(&mut payload, 42);
    push_u64(&mut payload, 0);
    push_u16(&mut payload, 0);
    push_u16(&mut payload, 0);
    push_u32(&mut payload, header_size as u32);
    push_u32(&mut payload, frame_count);
    push_u32(&mut payload, first_frame_address);

    for frame_index in 0..frame_count {
        push_u16(&mut payload, 0);
        push_u16(&mut payload, frame_index as u16);
        push_u64(&mut payload, 0);
        push_u32(&mut payload, 0);
    }

    for frame_index in 0..frame_count as usize {
        let entry_start = header_size + frame_index * entry_size;
        let frame_start = payload.len();
        let pixel_data_offset = (frame_start - entry_start) as u32;
        payload[entry_start + 12..entry_start + 16]
            .copy_from_slice(&pixel_data_offset.to_le_bytes());

        append_cc_palette(&mut payload);
        append_anim_frame(&mut payload, width, height, pattern);
    }

    let decompressed_size = payload.len() as u32;
    payload[8..12].copy_from_slice(&decompressed_size.to_le_bytes());
    payload
}

fn append_cc_palette(out: &mut Vec<u8>) {
    for index in 0..256u16 {
        let color: u16 = match index {
            1 => 0x7C00,
            2 => 0x03E0,
            _ => 0,
        };
        out.extend_from_slice(&color.to_le_bytes());
    }
}

#[derive(Clone, Copy)]
enum EcAnimRunPattern {
    SolidRuns,
    SkipSolidPairs,
    BlendPairs,
}

fn ec_anim_case(name: &'static str, pattern: EcAnimRunPattern) -> EcAnimCase {
    let rle = build_ec_anim_rle(EC_ANIM_WIDTH, EC_ANIM_HEIGHT, pattern);
    let payload = build_ec_animation_payload(EC_ANIM_WIDTH, EC_ANIM_HEIGHT, &rle);
    EcAnimCase {
        name,
        animation: AnimationFrame::load(&payload).expect("synthetic EC animation should load"),
    }
}

fn build_ec_anim_rle(width: u16, height: u16, pattern: EcAnimRunPattern) -> Vec<u8> {
    let pixel_count = width as usize * height as usize;
    let mut rle = Vec::new();
    let mut remaining = pixel_count;
    let mut state = 0x1234_5678u32;

    match pattern {
        EcAnimRunPattern::SolidRuns => {
            while remaining > 0 {
                let count = remaining.min(127);
                append_ec_solid_run(&mut rle, count, &mut state);
                remaining -= count;
            }
        }
        EcAnimRunPattern::SkipSolidPairs => {
            while remaining > 0 {
                rle.push(1);
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    break;
                }
                append_ec_solid_run(&mut rle, 1, &mut state);
                remaining -= 1;
            }
        }
        EcAnimRunPattern::BlendPairs => {
            while remaining > 1 {
                rle.push(129);
                rle.push(0x80);
                rle.push(next_ec_color(&mut state));
                rle.push(next_ec_color(&mut state));
                remaining -= 2;
            }
            if remaining == 1 {
                append_ec_solid_run(&mut rle, 1, &mut state);
            }
        }
    }

    rle
}

fn append_ec_solid_run(out: &mut Vec<u8>, count: usize, state: &mut u32) {
    debug_assert!((1..=127).contains(&count));
    out.push(128 + count as u8);
    out.push(0);
    for _ in 0..count {
        out.push(next_ec_color(state));
    }
}

fn next_ec_color(state: &mut u32) -> u8 {
    *state = xorshift32(*state);
    (*state & 3) as u8
}

fn build_ec_animation_payload(width: u16, height: u16, frame_bytes: &[u8]) -> Vec<u8> {
    let header_size = 40u32;
    let colours_offset = header_size;
    let colours_count = 256u32;
    let frames_offset = colours_offset + colours_count * 4;
    let image_offset = frames_offset + 16;
    let total_size = image_offset + frame_bytes.len() as u32;
    let mut bytes = Vec::with_capacity(total_size as usize);

    bytes.extend_from_slice(b"AMO\x04");
    push_u32(&mut bytes, 4);
    push_u32(&mut bytes, total_size);
    push_u32(&mut bytes, 42);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, width as i16);
    push_i16(&mut bytes, height as i16);
    push_u32(&mut bytes, colours_count);
    push_u32(&mut bytes, colours_offset);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, frames_offset);

    for index in 0..colours_count {
        let colour = match index & 3 {
            0 => [255u8, 0, 0, 255],
            1 => [0u8, 255, 0, 255],
            2 => [0u8, 0, 255, 255],
            _ => [255u8, 255, 255, 255],
        };
        bytes.extend_from_slice(&colour);
    }

    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, width as i16);
    push_i16(&mut bytes, height as i16);
    push_u32(&mut bytes, image_offset - frames_offset);
    bytes.extend_from_slice(frame_bytes);

    bytes
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i16(out: &mut Vec<u8>, value: i16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    dir.push(format!(
        "uocf_rle_decode_bench_{}_{}_{}",
        std::process::id(),
        name,
        timestamp
    ));
    dir
}

fn xorshift32(mut value: u32) -> u32 {
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    value
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}
