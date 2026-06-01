use std::hint::black_box;
use std::time::{Duration, Instant};

const WIDTH: usize = 128;
const HEIGHT: usize = 128;
const DST_WIDTH: usize = 160;
const DST_HEIGHT: usize = 144;
const DST_X: usize = 16;
const DST_Y: usize = 8;

struct BlitCase {
    name: &'static str,
    src: Vec<u8>,
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

    println!("mobile_anim_blit benchmark");
    println!("profile={} min_duration_ms={}", profile_name(), min_duration.as_millis());

    let cases = [
        blit_case("solid_alpha", AlphaPattern::Solid),
        blit_case("checker_alpha", AlphaPattern::Checker),
        blit_case("sparse_alpha", AlphaPattern::Sparse),
    ];
    for case in &cases {
        let result = bench_blit(case, min_duration);
        print_result(case.name, result);
    }
}

fn bench_blit(case: &BlitCase, min_duration: Duration) -> BenchResult {
    let mut dst = vec![0u8; DST_WIDTH * DST_HEIGHT * 4];
    let mut iterations = 0u64;
    let mut checksum_acc = 0u64;
    let start = Instant::now();
    while iterations == 0 || start.elapsed() < min_duration {
        dst.fill(0);
        let filled_pixel_count = blit_rgba_frame(
            black_box(&mut dst),
            black_box(DST_WIDTH),
            black_box(DST_X),
            black_box(DST_Y),
            black_box(WIDTH),
            black_box(HEIGHT),
            black_box(&case.src),
        );
        checksum_acc = checksum_acc
            .wrapping_add(checksum(black_box(&dst)))
            .wrapping_add(black_box(filled_pixel_count));
        iterations += 1;
    }

    BenchResult {
        iterations,
        elapsed: start.elapsed(),
        checksum: checksum_acc,
    }
}

fn blit_rgba_frame(
    dst: &mut [u8],
    dst_width: usize,
    dst_x: usize,
    dst_y: usize,
    frame_width: usize,
    frame_height: usize,
    src: &[u8],
) -> u64 {
    let dst_stride = dst_width * 4;
    let src_stride = frame_width * 4;
    let mut filled_pixel_count = 0u64;
    for row in 0..frame_height {
        let src_start = row * src_stride;
        let dst_start = ((dst_y + row) * dst_stride) + dst_x * 4;
        let dst_end = dst_start + src_stride;
        let src_row = &src[src_start..src_start + src_stride];
        filled_pixel_count += count_nonzero_alpha(src_row);
        dst[dst_start..dst_end].copy_from_slice(src_row);
    }
    filled_pixel_count
}

fn count_nonzero_alpha(row: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut index = 3usize;
    while index < row.len() {
        count += u64::from(row[index] != 0);
        index += 4;
    }
    count
}

#[derive(Clone, Copy)]
enum AlphaPattern {
    Solid,
    Checker,
    Sparse,
}

fn blit_case(name: &'static str, pattern: AlphaPattern) -> BlitCase {
    let mut src = vec![0u8; WIDTH * HEIGHT * 4];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let pixel = (y * WIDTH + x) * 4;
            src[pixel] = x as u8;
            src[pixel + 1] = y as u8;
            src[pixel + 2] = (x ^ y) as u8;
            src[pixel + 3] = match pattern {
                AlphaPattern::Solid => 255,
                AlphaPattern::Checker => {
                    if (x + y) & 1 == 0 { 255 } else { 0 }
                }
                AlphaPattern::Sparse => {
                    if (x.wrapping_mul(17) + y.wrapping_mul(31)) & 15 == 0 { 255 } else { 0 }
                }
            };
        }
    }
    BlitCase { name, src }
}

fn print_result(case_name: &str, result: BenchResult) {
    let elapsed_ns = result.elapsed.as_nanos() as f64;
    let ns_per_iter = elapsed_ns / result.iterations as f64;
    let iters_per_sec = 1_000_000_000.0 / ns_per_iter;
    println!(
        "{case_name:20} blit {:>12.2} blit/s {:>10.1} ns/blit iterations={} checksum={:016x}",
        iters_per_sec,
        ns_per_iter,
        result.iterations,
        result.checksum
    );
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
