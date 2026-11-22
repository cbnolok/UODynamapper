//! The GPU BC7 RDO against a CPU walk of the same search (every candidate tried, no bound, the codecs' BC7 decoder),
//! and its output across runs, batches and concurrent callers. The GPU tests are ignored by default; run them with
//! `cargo test -p uot-gpu-compute --release bc7_rdo -- --ignored`.

use super::*;
use uot_texture_codecs::bc7::{
    decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat,
};

const BLOCK_SIDE: usize = 4;
const RGBA: usize = 4;
const INVALID_MODE: u32 = 8;
/// bc7_rdo.wgsl's MS_ERROR_SCALE: the squared error of a block over its texels' channels.
const MS_ERROR_SCALE: f32 = 64.0;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
/// The random fixture's blocks: one chunk of each mode in turn, with near copies of earlier blocks.
const RANDOM_BLOCKS_X: usize = 16;
const RANDOM_BLOCKS_Y: usize = 32;
/// How far back a random block's near copy reaches, and how many of its bytes change.
const NEAR_COPY_REACH: u64 = 8;
const NEAR_COPY_MAX_CHANGES: u64 = 3;
/// The noise on the random fixture's texels.
const TEXEL_NOISE: u64 = 7;
/// One block in this many of the random fixture is invalid (no mode), and one in this many is exact (no error).
const INVALID_EVERY: u64 = 23;
const EXACT_EVERY: u64 = 11;
/// The sprite fixtures: pixels wide and high.
const SPRITE_SMALL: (u32, u32) = (128, 256);
const SPRITE_WIDE: (u32, u32) = (256, 384);
/// The sprite fixture's tile side in pixels (repeated, so that blocks match).
const SPRITE_TILE: usize = 24;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// A page: blocks, texels in block order, blocks wide and high.
#[derive(Clone)]
struct Page {
    blocks: Vec<[u8; BLOCK_BYTES]>,
    rgba_blocks: Vec<[u8; 4]>,
    blocks_x: usize,
    blocks_y: usize,
}

fn decode_block(block: &[u8; BLOCK_BYTES]) -> Vec<u8> {
    let extent = ImageExtent::new(BLOCK_SIDE as u32, BLOCK_SIDE as u32).expect("a block's extent");
    decode_bc7_to_rgba8888(block, extent).expect("a block decodes")
}

fn block_mode(block: &[u8; BLOCK_BYTES]) -> u32 {
    if block[0] == 0 { INVALID_MODE } else { block[0].trailing_zeros() }
}

/// Blocks of random bits under each mode's header, many of them near copies of an earlier block of their mode, and
/// texels near their decoding (some exact, some blocks invalid).
fn random_page() -> Page {
    let mut rng = Rng(SEED);
    let count = RANDOM_BLOCKS_X * RANDOM_BLOCKS_Y;
    let mut blocks: Vec<[u8; BLOCK_BYTES]> = Vec::with_capacity(count);
    let mut rgba_blocks = Vec::with_capacity(count * BLOCK_TEXELS);
    for index in 0..count {
        let invalid = rng.below(INVALID_EVERY) == 0;
        let mode = rng.below(u64::from(INVALID_MODE)) as u32;
        let source = (rng.below(2) == 0 && index > 0)
            .then(|| index - 1 - rng.below(NEAR_COPY_REACH.min(index as u64)) as usize)
            .filter(|&source| block_mode(&blocks[source]) != INVALID_MODE);
        let mut block = match source {
            Some(source) => {
                let mut block = blocks[source];
                for _ in 0..=rng.below(NEAR_COPY_MAX_CHANGES) {
                    let byte = 1 + rng.below(BLOCK_BYTES as u64 - 1) as usize;
                    block[byte] = rng.next() as u8;
                }
                block
            }
            None => {
                let mut block: [u8; BLOCK_BYTES] = std::array::from_fn(|_| rng.next() as u8);
                // The mode's header: its bit set, the bits below clear.
                block[0] = ((u32::from(block[0]) >> (mode + 1) << (mode + 1)) | 1 << mode) as u8;
                block
            }
        };
        if invalid {
            block[0] = 0;
        }
        let exact = rng.below(EXACT_EVERY) == 0;
        let decoded = if invalid { vec![0; BLOCK_TEXELS * RGBA] } else { decode_block(&block) };
        for texel in decoded.chunks_exact(RGBA) {
            rgba_blocks.push(std::array::from_fn(|channel| if exact { texel[channel] } else {
                let noise = rng.below(2 * TEXEL_NOISE + 1) as i32 - TEXEL_NOISE as i32;
                (i32::from(texel[channel]) + noise).clamp(0, 255) as u8
            }));
        }
        blocks.push(block);
    }
    Page { blocks, rgba_blocks, blocks_x: RANDOM_BLOCKS_X, blocks_y: RANDOM_BLOCKS_Y }
}

/// A sprite-like page: repeated tiles of soft gradients with transparent gaps and noise, CPU-encoded.
fn sprite_page((width, height): (u32, u32), seed: u64) -> Page {
    let mut rng = Rng(SEED ^ seed);
    let mut pixels = vec![0u8; width as usize * height as usize * RGBA];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let (tx, ty) = (x % SPRITE_TILE, y % SPRITE_TILE);
            let inside = (tx as i32 - 12).pow(2) + (ty as i32 - 12).pow(2) < 100 + (x / SPRITE_TILE % 3) as i32 * 20;
            if !inside {
                continue;
            }
            let noise = rng.below(4) as usize;
            let texel = &mut pixels[(y * width as usize + x) * RGBA..][..RGBA];
            texel[0] = (tx * 10 + noise) as u8;
            texel[1] = (ty * 9 + (y / SPRITE_TILE) * 20) as u8;
            texel[2] = (200 - tx * 4 + noise) as u8;
            texel[3] = if tx == 2 || ty == 2 { 128 } else { 255 };
        }
    }
    let extent = ImageExtent::new(width, height).expect("a sprite page's extent");
    let encoded = encode_to_bc7(&pixels, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::Analytical)
        .expect("the sprite page encodes");
    let blocks = encoded.blocks().chunks_exact(BLOCK_BYTES).map(|block| block.try_into().unwrap()).collect();
    let (blocks_x, blocks_y) = (width as usize / BLOCK_SIDE, height as usize / BLOCK_SIDE);
    let mut rgba_blocks = Vec::with_capacity(blocks_x * blocks_y * BLOCK_TEXELS);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            for y in 0..BLOCK_SIDE {
                for x in 0..BLOCK_SIDE {
                    let at = ((by * BLOCK_SIDE + y) * width as usize + bx * BLOCK_SIDE + x) * RGBA;
                    rgba_blocks.push(pixels[at..at + RGBA].try_into().unwrap());
                }
            }
        }
    }
    Page { blocks, rgba_blocks, blocks_x, blocks_y }
}

fn params(lambda: f32, lookback_blocks: usize, skip_zero_mse_blocks: bool) -> Bc7RdoParams {
    Bc7RdoParams {
        lambda, lookback_window_size: lookback_blocks * BLOCK_BYTES, skip_zero_mse_blocks, ..Default::default()
    }
}

fn block_error(block: &[u8; BLOCK_BYTES], source: &[[u8; 4]]) -> u32 {
    decode_block(block).chunks_exact(RGBA).zip(source)
        .map(|(decoded, texel)| (0..RGBA).map(|c| (i32::from(decoded[c]) - i32::from(texel[c])).pow(2) as u32)
            .sum::<u32>())
        .sum()
}

/// The GPU's search on the CPU: every candidate of every block, in the GPU's order and with its prices and state.
fn reference_walk(page: &Page, params: &Bc7RdoParams) -> Vec<[u8; BLOCK_BYTES]> {
    let settings = WalkSettings::new(params).unwrap();
    let rates = settings.rates();
    let shapes = candidate_shapes();
    let scales = block_error_scales(&page.rgba_blocks, page.blocks_x, page.blocks_y, params);
    let continue_rates = (settings.lookback_blocks + 1) * RATE_ROW;
    let rep0_rates = continue_rates + RATE_ROW;
    let literal_rate = BLOCK_BYTES as f32 * LITERAL_BITS * settings.lambda;
    let mut blocks = page.blocks.clone();
    let chunk_blocks = settings.chunk_blocks(page.blocks_x);
    for first in (0..blocks.len()).step_by(chunk_blocks) {
        let end = (first + chunk_blocks).min(blocks.len());
        let (mut rep0, mut continues) = (NO_REP0 as usize, NO_BLOCK as usize);
        for i in first..end {
            let orig = blocks[i];
            let mode = block_mode(&orig);
            if mode == INVALID_MODE {
                continue;
            }
            let source = &page.rgba_blocks[i * BLOCK_TEXELS..][..BLOCK_TEXELS];
            let cur_err = block_error(&orig, source);
            if cur_err == 0 && settings.skip_zero_error {
                continue;
            }
            let scale = scales[i];
            let cur_ms = cur_err as f32 / MS_ERROR_SCALE;
            let thresh = settings.thresh_ratio_squared * cur_ms.max(1.0);
            let mut best_t = cur_ms * scale + literal_rate;
            let mut best = None;
            for k in 0..settings.lookback_blocks.min(i - first) * CANDIDATES_PER_BLOCK {
                let delta = k / CANDIDATES_PER_BLOCK + 1;
                let prev = blocks[i - delta];
                if block_mode(&prev) != mode {
                    continue;
                }
                let shape = shapes[k % CANDIDATES_PER_BLOCK];
                let (len, ofs) = ((shape & 0xFF) as usize, (shape >> 8) as usize);
                let mut rate = rates[delta * RATE_ROW + len];
                if delta == rep0 {
                    rate = rates[rep0_rates + len];
                }
                if ofs == 0 && i - delta == continues {
                    rate = rates[continue_rates + len];
                }
                let mut trial = orig;
                trial[ofs..ofs + len].copy_from_slice(&prev[ofs..ofs + len]);
                let err = if trial == orig {
                    cur_err
                } else {
                    if ofs == 0 && block_mode(&trial) != mode {
                        continue;
                    }
                    block_error(&trial, source)
                };
                let ms = err as f32 / MS_ERROR_SCALE;
                if ms < thresh {
                    let t = ms * scale + rate;
                    if t < best_t {
                        best_t = t;
                        best = Some((trial, delta, len, ofs));
                    }
                }
            }
            match best {
                Some((trial, delta, len, ofs)) => {
                    blocks[i] = trial;
                    rep0 = delta;
                    continues = if ofs + len == BLOCK_BYTES { i - delta + 1 } else { NO_BLOCK as usize };
                }
                None => continues = NO_BLOCK as usize,
            }
        }
    }
    blocks
}

fn gpu_walk(gpu: &Bc7RdoGpu, pages: &[Page], params: &Bc7RdoParams) -> Vec<Vec<[u8; BLOCK_BYTES]>> {
    let mut reduced: Vec<Page> = pages.to_vec();
    let mut batch: Vec<Bc7RdoPage> = reduced.iter_mut().map(|page| Bc7RdoPage {
        blocks: &mut page.blocks, rgba_blocks: &page.rgba_blocks, blocks_x: page.blocks_x, blocks_y: page.blocks_y,
    }).collect();
    gpu.reduce_pages(&mut batch, params).expect("the GPU RDO runs");
    reduced.into_iter().map(|page| page.blocks).collect()
}

fn changed(before: &Page, after: &[[u8; BLOCK_BYTES]]) -> usize {
    before.blocks.iter().zip(after).filter(|(a, b)| a != b).count()
}

#[test]
fn candidates_run_longest_first_then_by_offset() {
    let shapes = candidate_shapes();
    assert_eq!(shapes.len(), CANDIDATES_PER_BLOCK);
    assert_eq!(shapes[0], BLOCK_BYTES as u32);
    assert_eq!(shapes[1], 15);
    assert_eq!(shapes[2], 15 | 1 << 8);
    assert_eq!(*shapes.last().unwrap(), MIN_MATCH_LEN as u32 | ((BLOCK_BYTES - MIN_MATCH_LEN) as u32) << 8);
}

#[test]
fn chunks_are_whole_rows_of_the_minimum_and_the_lookback() {
    let settings = WalkSettings::new(&params(1.0, 64, false)).unwrap();
    assert_eq!(settings.chunk_blocks(512), 2048);
    assert_eq!(settings.chunk_blocks(300), 2100);
    assert_eq!(settings.chunk_blocks(4096), 4096);
    let long = WalkSettings::new(&params(1.0, 4096, false)).unwrap();
    assert_eq!(long.chunk_blocks(1000), 5000);
}

#[test]
fn rates_follow_the_cpu_price_model() {
    let settings = WalkSettings::new(&params(2.0, 8, false)).unwrap();
    let rates = settings.rates();
    let full = (compute_dist_cost_estimate(16) + compute_match_len_cost(16)) as f32 * 2.0;
    assert_eq!(rates[RATE_ROW + BLOCK_BYTES], full);
    let continue_row = 9 * RATE_ROW;
    assert_eq!(rates[continue_row + BLOCK_BYTES], MATCH_CONTINUE_BITS * 2.0);
    assert_eq!(rates[continue_row + RATE_ROW + 3], (13.0 * LITERAL_BITS + MATCH_REP0_BITS) * 2.0);
}

#[test]
fn unsupported_searches_are_errors() {
    assert!(WalkSettings::new(&Bc7RdoParams { try_two_matches: true, ..params(1.0, 64, false) }).is_err());
    assert!(WalkSettings::new(&Bc7RdoParams { allow_relative_movement: true, ..params(1.0, 64, false) }).is_err());
}

#[test]
#[ignore = "requires a GPU adapter; run explicitly (in release) for GPU BC7 RDO changes"]
fn gpu_matches_the_reference_walk() {
    let gpu = Bc7RdoGpu::new().expect("a GPU");
    println!("adapter: {}", gpu.adapter());
    let random = random_page();
    let modes = random.blocks.iter().fold([0usize; 9], |mut counts, block| {
        counts[block_mode(block) as usize] += 1;
        counts
    });
    assert!(modes.iter().all(|&count| count > 0), "every mode and invalid blocks: {modes:?}");
    let small = sprite_page(SPRITE_SMALL, 1);
    let wide = sprite_page(SPRITE_WIDE, 2);
    let cases = [
        (&random, params(1.0, 64, false)),
        (&random, params(4.0, 8, true)),
        (&small, params(1.0, 8, true)),
        (&small, params(2.0, 64, false)),
        (&wide, params(1.0, 8, true)),
    ];
    for (page, params) in cases {
        let expected = reference_walk(page, &params);
        let actual = gpu_walk(&gpu, std::slice::from_ref(page), &params).remove(0);
        let mismatches: Vec<usize> = (0..expected.len()).filter(|&i| expected[i] != actual[i]).collect();
        println!("λ {} look-back {}: {} of {} blocks changed, {} mismatches", params.lambda,
            params.lookback_window_size / BLOCK_BYTES, changed(page, &expected), expected.len(), mismatches.len());
        assert!(changed(page, &expected) > 0, "the fixture exercises matches");
        assert!(mismatches.is_empty(), "first mismatching blocks {:?}", &mismatches[..mismatches.len().min(8)]);
    }
}

#[test]
#[ignore = "requires a GPU adapter; run explicitly (in release) for GPU BC7 RDO changes"]
fn gpu_output_is_the_same_across_runs_batches_and_callers() {
    let gpu = Bc7RdoGpu::new().expect("a GPU");
    let pages = [sprite_page(SPRITE_WIDE, 3), random_page(), sprite_page(SPRITE_SMALL, 4)];
    let params = params(1.0, 64, true);
    let alone: Vec<_> = pages.iter().map(|page| gpu_walk(&gpu, std::slice::from_ref(page), &params).remove(0))
        .collect();
    assert_eq!(gpu_walk(&gpu, &pages, &params), alone, "a batch of pages");
    assert_eq!(gpu_walk(&gpu, &pages, &params), alone, "a second run");
    let concurrent: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = pages.iter().map(|page| scope.spawn(|| {
            let mut blocks = page.blocks.clone();
            gpu.reduce_entropy(&mut blocks, &page.rgba_blocks, page.blocks_x, page.blocks_y, &params)
                .expect("the GPU RDO runs");
            blocks
        })).collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect()
    });
    assert_eq!(concurrent, alone, "concurrent callers");
}
