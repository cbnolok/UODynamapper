use image_postprocess::bc7_analytical::{
    pack_bc7_rgba, Pixel, FLAG_PBIT_OPT, FLAG_PBIT_OPT_M6, FLAG_USE_2SUBSETS, FLAG_USE_3SUBSETS,
    FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6,
};
use image_postprocess::bc7_analytical_wide::pack_bc7_rgba_blocks_wide;
use image_postprocess::bc7_rdo::{
    reduce_entropy_bc7, reduce_entropy_bc7_with_stats, Bc7RdoParams, Bc7RdoStats,
};
use std::hint::black_box;
use std::time::{Duration, Instant};

const FLAGS: u32 = FLAG_USE_DUAL_PLANE
    | FLAG_USE_TRIVIAL_M6
    | FLAG_PBIT_OPT_M6
    | FLAG_USE_2SUBSETS
    | FLAG_USE_3SUBSETS
    | FLAG_PBIT_OPT;

struct Case {
    name: &'static str,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone)]
struct RdoCase {
    name: &'static str,
    params: Bc7RdoParams,
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let quick = args.iter().any(|arg| arg == "--quick");
    let rdo_stats = args.iter().any(|arg| arg == "--rdo-stats");
    let rdo_case_filter = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--rdo-case="));
    let min_duration = if quick {
        Duration::from_millis(80)
    } else {
        Duration::from_millis(700)
    };
    let cases = [
        opaque_case("opaque_uo_land", 128, 128),
        alpha_case("alpha_mobile_art", 128, 128),
        mixed_case("mixed_atlas_edges", 130, 126),
    ];

    println!("bc7_encode benchmark");
    println!("profile={} min_duration_ms={}", profile_name(), min_duration.as_millis());
    for case in &cases {
        run_case(case, min_duration, &rdo_cases(), rdo_stats, rdo_case_filter);
    }
}

fn rdo_cases() -> Vec<RdoCase> {
    vec![
        RdoCase {
            name: "rdo_default",
            params: Bc7RdoParams {
                lambda: 0.5,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_low_lambda",
            params: Bc7RdoParams {
                lambda: 0.125,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_high_lambda",
            params: Bc7RdoParams {
                lambda: 2.0,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_large_window",
            params: Bc7RdoParams {
                lambda: 0.5,
                lookback_window_size: 64 * 1024,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_two_matches",
            params: Bc7RdoParams {
                lambda: 0.5,
                try_two_matches: true,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_fixed_b64",
            params: Bc7RdoParams {
                lambda: 0.5,
                lookback_window_size: 64 * 16,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_fixed_b256",
            params: Bc7RdoParams {
                lambda: 0.5,
                lookback_window_size: 256 * 16,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_relative",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_relative_d4",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_offset_delta: 4,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_relative_b64",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_previous_blocks: 64,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_relative_l6",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_min_match_len: 6,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_rel_d4_b64",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_offset_delta: 4,
                relative_movement_max_previous_blocks: 64,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_rel_d4_l6",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_offset_delta: 4,
                relative_movement_min_match_len: 6,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_rel_b64_l6",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_previous_blocks: 64,
                relative_movement_min_match_len: 6,
                ..Bc7RdoParams::default()
            },
        },
        RdoCase {
            name: "rdo_relative_mix",
            params: Bc7RdoParams {
                lambda: 0.5,
                allow_relative_movement: true,
                relative_movement_max_offset_delta: 4,
                relative_movement_max_previous_blocks: 64,
                relative_movement_min_match_len: 6,
                ..Bc7RdoParams::default()
            },
        },
    ]
}

fn run_case(
    case: &Case,
    min_duration: Duration,
    rdo_cases: &[RdoCase],
    rdo_stats: bool,
    rdo_case_filter: Option<&str>,
) {
    let blocks = case.width.div_ceil(4) as usize * case.height.div_ceil(4) as usize;
    let scalar = bench_scalar(case, min_duration);
    let wide = bench_wide(case, min_duration);

    println!(
        "{:<18} blocks={:<5} scalar={:>10.2} blk/s wide={:>10.2} blk/s speedup={:>5.2}x checksums={:016x}/{:016x}",
        case.name,
        blocks,
        scalar.blocks_per_second,
        wide.blocks_per_second,
        wide.blocks_per_second / scalar.blocks_per_second,
        scalar.checksum,
        wide.checksum
    );
    for rdo_case in rdo_cases {
        if rdo_case_filter.is_some_and(|filter| filter != rdo_case.name) {
            continue;
        }
        let rdo = bench_rdo(case, min_duration, &rdo_case.params, rdo_stats);
        println!(
            "  {:<16} rdo={:>10.2} blk/s checksum={:016x}",
            rdo_case.name,
            rdo.bench.blocks_per_second,
            rdo.bench.checksum
        );
        if let Some(stats) = rdo.stats {
            println!(
                "    stats candidates={} rate_skips={} hash_skips={} rel_offset_skips={} rel_prev_hits={} rel_len_skips={} orig_blocks={} decodes={} modes={:?} fused_m0={} fused_m1={} fused_m2={} fused_m3={} fused_m4={} fused_m5={} fused_m6={} fused_m7={} unsupported_modes={} bounded_exits={} accepted={} modified={}",
                stats.candidate_checks,
                stats.rate_skips,
                stats.hash_skips,
                stats.relative_offset_skips,
                stats.relative_previous_block_limit_hits,
                stats.relative_length_skips,
                stats.original_block_skips,
                stats.decode_trials,
                stats.decode_mode_trials,
                stats.fused_mode0_trials,
                stats.fused_mode1_trials,
                stats.fused_mode2_trials,
                stats.fused_mode3_trials,
                stats.fused_mode4_trials,
                stats.fused_mode5_trials,
                stats.fused_mode6_trials,
                stats.fused_mode7_trials,
                stats.unsupported_mode_trials,
                stats.bounded_error_exits,
                stats.accepted_matches,
                stats.modified_blocks
            );
        }
    }
}

fn bench_scalar(case: &Case, min_duration: Duration) -> BenchResult {
    let blocks_x = case.width.div_ceil(4) as usize;
    let blocks_y = case.height.div_ceil(4) as usize;
    let mut out = vec![0u8; blocks_x * blocks_y * 16];
    let mut iterations = 0u64;
    let start = Instant::now();

    while iterations == 0 || start.elapsed() < min_duration {
        encode_scalar_image(&mut out, &case.rgba, case.width, case.height);
        black_box(&out);
        iterations += 1;
    }

    BenchResult::new(blocks_x * blocks_y, iterations, start.elapsed(), checksum(&out))
}

fn bench_wide(case: &Case, min_duration: Duration) -> BenchResult {
    let blocks_x = case.width.div_ceil(4) as usize;
    let blocks_y = case.height.div_ceil(4) as usize;
    let mut out = vec![0u8; blocks_x * blocks_y * 16];
    let mut iterations = 0u64;
    let start = Instant::now();

    while iterations == 0 || start.elapsed() < min_duration {
        pack_bc7_rgba_blocks_wide(&mut out, &case.rgba, case.width, case.height, FLAGS);
        black_box(&out);
        iterations += 1;
    }

    BenchResult::new(blocks_x * blocks_y, iterations, start.elapsed(), checksum(&out))
}

fn bench_rdo(case: &Case, min_duration: Duration, params: &Bc7RdoParams, collect_stats: bool) -> RdoBenchResult {
    let blocks_x = case.width.div_ceil(4) as usize;
    let blocks_y = case.height.div_ceil(4) as usize;
    let mut encoded = vec![0u8; blocks_x * blocks_y * 16];
    pack_bc7_rgba_blocks_wide(&mut encoded, &case.rgba, case.width, case.height, FLAGS);
    let baseline = encoded
        .chunks_exact(16)
        .map(|chunk| chunk.try_into().expect("BC7 blocks are 16 bytes"))
        .collect::<Vec<[u8; 16]>>();
    let rgba_blocks = rgba_to_block_order(&case.rgba, case.width, case.height);
    let mut out = baseline.clone();
    let mut modified = 0u32;
    let mut stats = Bc7RdoStats::default();
    let mut iterations = 0u64;
    let start = Instant::now();

    while iterations == 0 || start.elapsed() < min_duration {
        out.clone_from(&baseline);
        if collect_stats {
            stats = Bc7RdoStats::default();
            modified = reduce_entropy_bc7_with_stats(&mut out, &rgba_blocks, blocks_x, blocks_y, params, &mut stats);
        } else {
            modified = reduce_entropy_bc7(&mut out, &rgba_blocks, blocks_x, blocks_y, params);
        }
        black_box(&out);
        iterations += 1;
    }

    let flat = out.iter().flatten().copied().collect::<Vec<_>>();
    RdoBenchResult {
        bench: BenchResult::new(
            blocks_x * blocks_y,
            iterations,
            start.elapsed(),
            checksum(&flat) ^ modified as u64,
        ),
        stats: collect_stats.then_some(stats),
    }
}

fn encode_scalar_image(out: &mut [u8], rgba: &[u8], width: u32, height: u32) {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let width = width as usize;
    let height = height as usize;

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut pixels = [[0u8; 4]; 16];
            for row in 0..4 {
                let src_y = (block_y * 4 + row).min(height - 1);
                for col in 0..4 {
                    let src_x = (block_x * 4 + col).min(width - 1);
                    let src = (src_y * width + src_x) * 4;
                    pixels[row * 4 + col].copy_from_slice(&rgba[src..src + 4]);
                }
            }

            let block_index = block_y * blocks_x + block_x;
            let block: &mut [u8; 16] = out[block_index * 16..(block_index + 1) * 16]
                .as_mut()
                .try_into()
                .expect("BC7 output is addressed in 16-byte blocks");
            let pixels: &[Pixel; 16] = &pixels;
            pack_bc7_rgba(block, pixels, FLAGS);
        }
    }
}

fn rgba_to_block_order(rgba: &[u8], width: u32, height: u32) -> Vec<[u8; 4]> {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let width = width as usize;
    let height = height as usize;
    let mut pixels = Vec::with_capacity(blocks_x * blocks_y * 16);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            for row in 0..4 {
                let src_y = (block_y * 4 + row).min(height - 1);
                for col in 0..4 {
                    let src_x = (block_x * 4 + col).min(width - 1);
                    let src = (src_y * width + src_x) * 4;
                    pixels.push(rgba[src..src + 4].try_into().expect("RGBA pixels have 4 bytes"));
                }
            }
        }
    }

    pixels
}

fn opaque_case(name: &'static str, width: u32, height: u32) -> Case {
    image_case(name, width, height, |x, y| {
        let r = ((x * 5 + y * 3 + ((x / 8) ^ (y / 8)) * 17) & 255) as u8;
        let g = ((x * 2 + y * 7 + ((x / 4) * 13)) & 255) as u8;
        let b = ((x * 11 + y * 5 + 91) & 255) as u8;
        [r, g, b, 255]
    })
}

fn alpha_case(name: &'static str, width: u32, height: u32) -> Case {
    image_case(name, width, height, |x, y| {
        let r = ((x * 9 + y * 2 + 31) & 255) as u8;
        let g = ((x * 4 + y * 11 + 67) & 255) as u8;
        let b = ((x * 3 + y * 5 + (x ^ y) * 3) & 255) as u8;
        let a = (((x / 3) * 19 + (y / 5) * 23 + x + y) & 255) as u8;
        [r, g, b, a]
    })
}

fn mixed_case(name: &'static str, width: u32, height: u32) -> Case {
    image_case(name, width, height, |x, y| {
        if ((x / 16) + (y / 16)) & 1 == 0 {
            let v = ((x * 13 + y * 7) & 255) as u8;
            [v, v.saturating_add(32), v / 2, 255]
        } else {
            [
                ((x * 5 + y * 17) & 255) as u8,
                ((x * 29 + y * 3) & 255) as u8,
                ((x * 7 + y * 11) & 255) as u8,
                ((x * 2 + y * 31) & 255) as u8,
            ]
        }
    })
}

fn image_case(
    name: &'static str,
    width: u32,
    height: u32,
    mut pixel: impl FnMut(u32, u32) -> [u8; 4],
) -> Case {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&pixel(x, y));
        }
    }
    Case { name, width, height, rgba }
}

struct BenchResult {
    blocks_per_second: f64,
    checksum: u64,
}

struct RdoBenchResult {
    bench: BenchResult,
    stats: Option<Bc7RdoStats>,
}

impl BenchResult {
    fn new(blocks_per_iteration: usize, iterations: u64, elapsed: Duration, checksum: u64) -> Self {
        let blocks = blocks_per_iteration as f64 * iterations as f64;
        Self {
            blocks_per_second: blocks / elapsed.as_secs_f64(),
            checksum,
        }
    }
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    })
}

fn profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}
