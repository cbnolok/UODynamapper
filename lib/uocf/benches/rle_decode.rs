use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use uocf::classic::anim::AnimMap;
use uocf::classic::gump::decode_gump_from_raw;

const GUMP_WIDTH: u16 = 128;
const GUMP_HEIGHT: u16 = 128;
const ANIM_WIDTH: u16 = 128;
const ANIM_HEIGHT: u16 = 128;

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
    ];
    for case in &anim_cases {
        let result = bench_anim(case, min_duration);
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
    let dir = unique_temp_dir(name);
    fs::create_dir_all(&dir).expect("create animation benchmark directory");
    let payload = build_anim_payload(ANIM_WIDTH, ANIM_HEIGHT, pattern);
    write_anim_pair(&dir, &payload).expect("write animation benchmark files");
    let map = AnimMap::load(&dir).expect("load animation benchmark files");
    AnimCase {
        name,
        map,
        cleanup_dir: dir,
    }
}

fn build_anim_payload(width: u16, height: u16, pattern: AnimRunPattern) -> Vec<u8> {
    let mut payload = vec![0u8; 256 * 2];
    payload[2..4].copy_from_slice(&0x7C00u16.to_le_bytes());
    payload[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&8u32.to_le_bytes());
    payload.extend_from_slice(&0i16.to_le_bytes());
    payload.extend_from_slice(&0i16.to_le_bytes());
    payload.extend_from_slice(&width.to_le_bytes());
    payload.extend_from_slice(&height.to_le_bytes());

    for y in 0..height {
        match pattern {
            AnimRunPattern::SolidRows => {
                append_anim_run(
                    &mut payload,
                    0,
                    y,
                    width,
                    height,
                    std::iter::repeat(1).take(width as usize),
                );
            }
            AnimRunPattern::SinglePixelRuns => {
                for x in 0..width {
                    let index = if (x + y) & 1 == 0 { 1 } else { 2 };
                    append_anim_run(&mut payload, x, y, 1, height, std::iter::once(index));
                }
            }
        }
    }

    payload.extend_from_slice(&0x7FFF7FFFu32.to_le_bytes());
    payload
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
