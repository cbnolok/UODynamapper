//! Formats for the Classic mobile animation and art pages, whose alpha is binary (docs/agent_notes/
//! atlas_page_compression.md, "Formats for the Classic pages"), with the Enhanced mobile pages for contrast. Every
//! variant encodes the same lossless pages sampled as `bc7_gpu` samples them and is measured on:
//! - the blocks' (or texels') size after Zstd 9 and 19, each page compressed alone;
//! - RGB PSNR over the opaque texels (alpha 255) only;
//! - the texels whose alpha lands on the other side of one half (the renderer's cutoff), the transparent texels
//!   decoded with some alpha (the blended pass draws them), and the opaque ones decoded below 255;
//! - the edges under the renderer's straight-alpha bilinear tap (client_bilinear_rect): every 2x2 window with a
//!   transparent texel whose source alpha averages at least one half, its RGB against the lossless page's tap and
//!   against the premultiplied tap (sum of alpha x RGB over sum of alpha), which a transparent texel lends no colour.
//!
//! Usage:
//! - `classic_formats <uddp dir> <mobile_cc|art_cc|mobile_ec> <pages> [variant,...]`: the variants' rows; all of
//!   them by default (`VARIANTS`).
//! - `classic_formats texels <uddp dir> <set> <pages>`: the pages' alpha values and their transparent texels' RGB.
//! - `classic_formats palettes <uddp dir> <pages>`: the Classic mobile package's per-animation palettes (every page)
//!   and the sampled pages as palette indices (R8, 0 transparent), against their texels' Zstd.
//! - `classic_formats split <uddp dir> <set> <pages>`: the GPU's UltraFast on the page against two dispatches, the
//!   alpha profile on the blocks with alpha and the opaque profile on the packed all-opaque ones, timed.
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::time::Instant;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage, TexArtCcPackage};
use uot_gpu_compute::block_encode::{Bc7Quality, MobileBlockFormat};
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const ZSTD_LEVELS: [i32; 2] = [9, 19];
const PEAK_SQUARED: f64 = 255.0 * 255.0;
const MIB: f64 = 1048576.0;
const OPAQUE: u8 = u8::MAX;
/// The renderer's alpha cutoff (statics_draw.rs `alpha_cutoff: 0.5`) on 8-bit alpha.
const CUTOFF: u8 = 128;
const BLOCK_EDGE: usize = 4;
const BLOCK_TEXELS: usize = BLOCK_EDGE * BLOCK_EDGE;
const BC7_BLOCK_BYTES: usize = 16;
/// The packed band's width in blocks (page_encode's, 1024 texels).
const PACKED_BLOCKS_PER_ROW: usize = 256;
/// Palette indices in an R8 texel; index 0 is transparent.
const PALETTE_SLOTS: usize = 255;
/// Bytes of a palette colour stored as 1555.
const PALETTE_COLOUR_BYTES: usize = 2;
const BEST_OF: usize = 2;
/// The bits a colour widened from five bits leaves at zero.
const LOW_THREE_BITS: u8 = 7;

const VARIANTS: [&str; 20] = ["lossless", "lossless-1555", "cpu", "cpu-prefill", "cpu-prefill-mean",
    "gpu-ultrafast", "gpu-ultrafast-prefill", "gpu-ultrafast-prefill-mean", "gpu-veryfast", "gpu-fast", "gpu-basic",
    "gpu-slow", "gpu-ultrafast+opaque-ultrafast", "gpu-ultrafast+opaque-basic", "bc1-cpu", "bc1-gpu", "bc2", "bc3",
    "bc3-prefill", "gpu-ultrafast-prefill+opaque-ultrafast"];

struct Page { index: u32, width: u32, height: u32, rgba: Vec<u8> }

fn pick(total: usize, count: usize) -> Vec<usize> {
    let count = count.min(total);
    (0..count).map(|k| k * total / count).collect()
}

fn crop(index: u32, atlas_width: u32, width: u32, height: u32, format: PagePixelFormat, rgba: Vec<u8>) -> Page {
    assert_eq!(format, PagePixelFormat::Rgba8888, "a lossless source (pack it with --raw)");
    let used = width as usize * height as usize * 4;
    let rgba = if rgba.len() == used { rgba } else {
        rgba.chunks_exact(atlas_width as usize * 4).take(height as usize)
            .flat_map(|row| row[..width as usize * 4].to_vec()).collect()
    };
    Page { index, width, height, rgba }
}

/// `bc7_gpu`'s sample of a set's pages.
fn sample_pages(dir: &str, set: &str, count: usize) -> Vec<Page> {
    match set {
        "mobile_cc" => {
            let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
            pick(package.pages().len(), count).into_par_iter().map(|i| {
                let r = package.pages()[i];
                crop(r.page_index, r.atlas_width, r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(r.page_index).unwrap())
            }).collect()
        }
        "mobile_ec" => {
            let package = MobileAnimEcPackage::load(format!("{dir}/mobile_anim_ec.uddp")).unwrap();
            pick(package.pages().len(), count).into_par_iter().map(|i| {
                let r = package.pages()[i];
                crop(r.page_index, r.atlas_width, r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(r.page_index).unwrap())
            }).collect()
        }
        "art_cc" => {
            let package = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
            pick(package.pages().len(), count).into_par_iter().map(|i| {
                let r = &package.pages()[i];
                crop(i as u32, package.atlas_width(), r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(i as u32).unwrap())
            }).collect()
        }
        other => panic!("unknown page set {other}"),
    }
}

fn extent(page: &Page) -> ImageExtent {
    ImageExtent::new(page.width, page.height).unwrap()
}

fn texel(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let at = (y * width + x) * 4;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

/// The page with every transparent texel of a block holding an opaque one given the colour of its nearest opaque
/// texel (4-neighbour distance over the page); its alpha stays, and the wholly transparent blocks stay as they were.
fn prefill(page: &Page) -> Vec<u8> {
    let (width, height) = (page.width as usize, page.height as usize);
    let mut nearest: Vec<u32> = vec![u32::MAX; width * height];
    let mut queue = VecDeque::new();
    for i in 0..width * height {
        if page.rgba[i * 4 + 3] >= CUTOFF {
            nearest[i] = i as u32;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % width, i / width);
        let mut visit = |j: usize| {
            if nearest[j] == u32::MAX {
                nearest[j] = nearest[i];
                queue.push_back(j);
            }
        };
        if x > 0 { visit(i - 1); }
        if x + 1 < width { visit(i + 1); }
        if y > 0 { visit(i - width); }
        if y + 1 < height { visit(i + width); }
    }
    let mut rgba = page.rgba.clone();
    let blocks_wide = width.div_ceil(BLOCK_EDGE);
    let mut mixed = vec![false; blocks_wide * height.div_ceil(BLOCK_EDGE)];
    for i in 0..width * height {
        if page.rgba[i * 4 + 3] >= CUTOFF {
            mixed[(i / width / BLOCK_EDGE) * blocks_wide + (i % width) / BLOCK_EDGE] = true;
        }
    }
    for i in 0..width * height {
        let block = (i / width / BLOCK_EDGE) * blocks_wide + (i % width) / BLOCK_EDGE;
        if page.rgba[i * 4 + 3] < CUTOFF && mixed[block] && nearest[i] != u32::MAX {
            let from = nearest[i] as usize * 4;
            rgba.copy_within(from..from + 3, i * 4);
        }
    }
    rgba
}

/// The page with every transparent texel of a block holding an opaque one given the mean colour of the block's opaque
/// texels: they then pull the block's endpoints nowhere, as a "don't care" texel would.
fn prefill_mean(page: &Page) -> Vec<u8> {
    let (width, height) = (page.width as usize, page.height as usize);
    let mut rgba = page.rgba.clone();
    for by in (0..height).step_by(BLOCK_EDGE) {
        for bx in (0..width).step_by(BLOCK_EDGE) {
            let texels: Vec<usize> = (by..(by + BLOCK_EDGE).min(height))
                .flat_map(|y| (bx..(bx + BLOCK_EDGE).min(width)).map(move |x| y * width + x)).collect();
            let opaque: Vec<usize> = texels.iter().copied().filter(|&i| page.rgba[i * 4 + 3] >= CUTOFF).collect();
            if opaque.is_empty() || opaque.len() == texels.len() { continue; }
            let mean: [u8; 3] = std::array::from_fn(|c| (opaque.iter().map(|&i| u32::from(page.rgba[i * 4 + c]))
                .sum::<u32>() as f64 / opaque.len() as f64).round() as u8);
            for &i in &texels {
                if page.rgba[i * 4 + 3] < CUTOFF { rgba[i * 4..i * 4 + 3].copy_from_slice(&mean); }
            }
        }
    }
    rgba
}

/// The page's texels as 1555 (one bit of alpha, five of each colour; a transparent texel is 0), little-endian. The
/// converter widens five bits to eight as `c << 3` (uocf `rgba8888_word_from_bgra5551`).
fn to_1555(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|t| {
        let value: u16 = if t[3] < CUTOFF { 0 } else {
            0x8000 | (u16::from(t[0] >> 3) << 10) | (u16::from(t[1] >> 3) << 5) | u16::from(t[2] >> 3)
        };
        value.to_le_bytes()
    }).collect()
}

fn from_1555(bytes: &[u8]) -> Vec<u8> {
    let expand = |five: u16| (five << 3) as u8;
    bytes.chunks_exact(2).flat_map(|pair| {
        let value = u16::from_le_bytes([pair[0], pair[1]]);
        if value & 0x8000 == 0 { [0; 4] } else {
            [expand((value >> 10) & 31), expand((value >> 5) & 31), expand(value & 31), OPAQUE]
        }
    }).collect()
}

fn dds_options() -> dds::EncodeOptions {
    let mut options = dds::EncodeOptions::default();
    options.quality = dds::CompressionQuality::High;
    options.dithering = dds::Dithering::None;
    options.parallel = false;
    options
}

fn dds_encode(page: &Page, rgba: &[u8], format: dds::Format) -> Vec<u8> {
    let size = dds::Size::new(page.width, page.height);
    let image = dds::ImageView::new(rgba, size, dds::ColorFormat::RGBA_U8).unwrap();
    let mut blocks = Vec::new();
    dds::encode(&mut blocks, image, format, None, &dds_options()).unwrap();
    blocks
}

fn dds_decode(page: &Page, blocks: &[u8], format: dds::Format) -> Vec<u8> {
    let mut rgba = vec![0u8; page.rgba.len()];
    let image = dds::ImageViewMut::new(&mut rgba, dds::Size::new(page.width, page.height), dds::ColorFormat::RGBA_U8)
        .unwrap();
    dds::decode(&mut Cursor::new(blocks), image, format, &dds::DecodeOptions::default()).unwrap();
    rgba
}

fn quality(name: &str) -> Bc7Quality {
    *Bc7Quality::ALL.iter().find(|quality| format!("{quality:?}").eq_ignore_ascii_case(name)).expect("a profile")
}

/// Whether block `index` (row-major, `blocks_wide` to a row) of `rgba` has all its texels at alpha 255. A partial
/// block repeats the page's edge texels, as the encoders do.
fn block_texels(rgba: &[u8], width: usize, height: usize, index: usize) -> [[u8; 4]; BLOCK_TEXELS] {
    let blocks_wide = width.div_ceil(BLOCK_EDGE);
    let (bx, by) = (index % blocks_wide * BLOCK_EDGE, index / blocks_wide * BLOCK_EDGE);
    std::array::from_fn(|k| texel(rgba, width, (bx + k % BLOCK_EDGE).min(width - 1),
        (by + k / BLOCK_EDGE).min(height - 1)))
}

fn all_opaque(texels: &[[u8; 4]; BLOCK_TEXELS]) -> bool {
    texels.iter().all(|t| t[3] == OPAQUE)
}

fn one_colour(texels: &[[u8; 4]; BLOCK_TEXELS]) -> bool {
    texels.iter().all(|t| *t == texels[0])
}

/// The GPU's alpha-profile blocks, with the all-opaque blocks of more than one colour taken from the opaque profile.
fn with_opaque_blocks(gpu: &PageEncoderGpu, page: &Page, rgba: &[u8], alpha: Bc7Quality, opaque: Bc7Quality)
    -> Vec<u8> {
    let mut blocks = gpu.encode(rgba, page.width, page.height, alpha).unwrap().concat();
    let opaque_blocks = gpu.encode_blocks(rgba, page.width, page.height, MobileBlockFormat::Bc7Opaque(opaque)).unwrap();
    let (width, height) = (page.width as usize, page.height as usize);
    for index in 0..blocks.len() / BC7_BLOCK_BYTES {
        let texels = block_texels(rgba, width, height, index);
        if all_opaque(&texels) && !one_colour(&texels) {
            let range = index * BC7_BLOCK_BYTES..(index + 1) * BC7_BLOCK_BYTES;
            blocks[range.clone()].copy_from_slice(&opaque_blocks[range]);
        }
    }
    blocks
}

/// A variant's encoded bytes and decoded texels for one page.
fn run_variant(name: &str, page: &Page, gpu: &PageEncoderGpu) -> (Vec<u8>, Vec<u8>) {
    let bc7 = |blocks: Vec<u8>| {
        let decoded = decode_bc7_to_rgba8888(&blocks, extent(page)).unwrap();
        (blocks, decoded)
    };
    let cpu = |rgba: &[u8]| encode_to_bc7(rgba, extent(page), RawImageFormat::Rgba8888,
        Bc7EncoderBackend::AnalyticalWide).unwrap().into_blocks();
    let dds_pair = |rgba: &[u8], format| { let blocks = dds_encode(page, rgba, format);
        let decoded = dds_decode(page, &blocks, format); (blocks, decoded) };
    match name {
        "lossless" => (page.rgba.clone(), page.rgba.clone()),
        "lossless-1555" => { let bytes = to_1555(&page.rgba); let decoded = from_1555(&bytes); (bytes, decoded) }
        "cpu" => bc7(cpu(&page.rgba)),
        "cpu-prefill" => bc7(cpu(&prefill(page))),
        "cpu-prefill-mean" => bc7(cpu(&prefill_mean(page))),
        "gpu-ultrafast-prefill-mean" => bc7(gpu.encode(&prefill_mean(page), page.width, page.height,
            Bc7Quality::UltraFast).unwrap().concat()),
        "gpu-ultrafast-prefill" => bc7(gpu.encode(&prefill(page), page.width, page.height, Bc7Quality::UltraFast)
            .unwrap().concat()),
        "gpu-ultrafast+opaque-ultrafast" => bc7(with_opaque_blocks(gpu, page, &page.rgba, Bc7Quality::UltraFast,
            Bc7Quality::UltraFast)),
        "gpu-ultrafast+opaque-basic" => bc7(with_opaque_blocks(gpu, page, &page.rgba, Bc7Quality::UltraFast,
            Bc7Quality::Basic)),
        "gpu-ultrafast-prefill+opaque-ultrafast" => bc7(with_opaque_blocks(gpu, page, &prefill(page),
            Bc7Quality::UltraFast, Bc7Quality::UltraFast)),
        "bc1-cpu" => dds_pair(&page.rgba, dds::Format::BC1_UNORM),
        "bc1-gpu" => {
            let blocks = gpu.encode_bc1(&page.rgba, page.width, page.height).unwrap().concat();
            let decoded = dds_decode(page, &blocks, dds::Format::BC1_UNORM);
            (blocks, decoded)
        }
        "bc2" => dds_pair(&page.rgba, dds::Format::BC2_UNORM),
        "bc3" => dds_pair(&page.rgba, dds::Format::BC3_UNORM),
        "bc3-prefill" => dds_pair(&prefill(page), dds::Format::BC3_UNORM),
        gpu_profile if gpu_profile.starts_with("gpu-") => bc7(gpu.encode(&page.rgba, page.width, page.height,
            quality(&gpu_profile["gpu-".len()..])).unwrap().concat()),
        other => panic!("unknown variant {other}"),
    }
}

/// Sums over a variant's pages.
#[derive(Default, Clone, Copy)]
struct Measure {
    zstd: [usize; 2],
    raw: usize,
    opaque_squared: f64,
    opaque_count: f64,
    cutoff_flips: usize,
    ghosts: usize,
    dimmed: usize,
    edge_squared_lossless: f64,
    edge_squared_premultiplied: f64,
    edge_count: f64,
    edge_flips: usize,
}

impl Measure {
    fn add(mut self, other: Self) -> Self {
        for k in 0..ZSTD_LEVELS.len() { self.zstd[k] += other.zstd[k]; }
        self.raw += other.raw;
        self.opaque_squared += other.opaque_squared;
        self.opaque_count += other.opaque_count;
        self.cutoff_flips += other.cutoff_flips;
        self.ghosts += other.ghosts;
        self.dimmed += other.dimmed;
        self.edge_squared_lossless += other.edge_squared_lossless;
        self.edge_squared_premultiplied += other.edge_squared_premultiplied;
        self.edge_count += other.edge_count;
        self.edge_flips += other.edge_flips;
        self
    }
}

/// The straight-alpha bilinear tap at the corner of a 2x2 window, RGBA in 0..255.
fn straight_tap(window: &[[u8; 4]; 4]) -> [f64; 4] {
    std::array::from_fn(|c| window.iter().map(|t| f64::from(t[c])).sum::<f64>() / 4.0)
}

/// The premultiplied tap's RGB: a transparent texel lends no colour.
fn premultiplied_rgb(window: &[[u8; 4]; 4]) -> [f64; 3] {
    let alpha: f64 = window.iter().map(|t| f64::from(t[3])).sum();
    std::array::from_fn(|c| window.iter().map(|t| f64::from(t[c]) * f64::from(t[3])).sum::<f64>() / alpha)
}

fn measure(page: &Page, encoded: &[u8], decoded: &[u8]) -> Measure {
    let mut m = Measure { raw: encoded.len(), ..Measure::default() };
    for (k, level) in ZSTD_LEVELS.iter().enumerate() {
        m.zstd[k] = zstd::bulk::compress(encoded, *level).unwrap().len();
    }
    for (s, d) in page.rgba.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        if s[3] == OPAQUE {
            for c in 0..3 { m.opaque_squared += (f64::from(s[c]) - f64::from(d[c])).powi(2); }
            m.opaque_count += 3.0;
            if d[3] < OPAQUE { m.dimmed += 1; }
        }
        if (s[3] >= CUTOFF) != (d[3] >= CUTOFF) { m.cutoff_flips += 1; }
        if s[3] == 0 && d[3] > 0 { m.ghosts += 1; }
    }
    let (width, height) = (page.width as usize, page.height as usize);
    for y in 0..height.saturating_sub(1) {
        for x in 0..width.saturating_sub(1) {
            let window = |rgba: &[u8]| [texel(rgba, width, x, y), texel(rgba, width, x + 1, y),
                texel(rgba, width, x, y + 1), texel(rgba, width, x + 1, y + 1)];
            let source = window(&page.rgba);
            if source.iter().all(|t| t[3] >= CUTOFF) { continue; }
            let source_tap = straight_tap(&source);
            if source_tap[3] < f64::from(CUTOFF) { continue; }
            let decoded_tap = straight_tap(&window(decoded));
            let reference = premultiplied_rgb(&source);
            for c in 0..3 {
                m.edge_squared_lossless += (decoded_tap[c] - source_tap[c]).powi(2);
                m.edge_squared_premultiplied += (decoded_tap[c] - reference[c]).powi(2);
            }
            m.edge_count += 3.0;
            if decoded_tap[3] < f64::from(CUTOFF) { m.edge_flips += 1; }
        }
    }
    m
}

fn psnr(squared: f64, count: f64) -> f64 {
    if squared == 0.0 { f64::INFINITY } else { 10.0 * (PEAK_SQUARED / (squared / count)).log10() }
}

fn variants_rows(dir: &str, set: &str, count: usize, names: Vec<String>) {
    let pages = sample_pages(dir, set, count);
    let texels: f64 = pages.iter().map(|page| f64::from(page.width) * f64::from(page.height)).sum();
    println!("{set}: {} pages, {:.1} Mpx", pages.len(), texels / 1e6);
    let gpu = PageEncoderGpu::new().unwrap();
    println!("| Variant | Zstd 9 | Zstd 19 | bpp (raw) | Opaque RGB PSNR | Cutoff flips | Ghosts | Dimmed | \
        Edge vs lossless | Edge vs premultiplied | Edge flips | Wall |");
    let mut lossless_zstd = None;
    for name in names {
        // A first run compiles the GPU's pipelines; the second is timed.
        if name.starts_with("gpu") || name == "bc1-gpu" {
            let _ = run_variant(&name, &pages[0], &gpu);
        }
        let started = Instant::now();
        let encoded: Vec<(Vec<u8>, Vec<u8>)> = pages.par_iter().map(|page| run_variant(&name, page, &gpu)).collect();
        let wall = started.elapsed().as_secs_f64();
        let m = pages.par_iter().zip(&encoded).map(|(page, (bytes, decoded))| measure(page, bytes, decoded))
            .reduce(Measure::default, Measure::add);
        let base = *lossless_zstd.get_or_insert(m.zstd);
        let versus = |k: usize| format!("{:.2} MiB ({:.1} %)", m.zstd[k] as f64 / MIB,
            m.zstd[k] as f64 / base[k] as f64 * 100.0);
        println!("| {name} | {} | {} | {:.1} | {:.2} dB | {} | {} | {} | {:.2} dB | {:.2} dB | {} | {wall:.2} s |",
            versus(0), versus(1), m.raw as f64 * 8.0 / texels, psnr(m.opaque_squared, m.opaque_count),
            m.cutoff_flips, m.ghosts, m.dimmed, psnr(m.edge_squared_lossless, m.edge_count),
            psnr(m.edge_squared_premultiplied, m.edge_count), m.edge_flips);
    }
}

/// The pages' alpha values, their transparent texels' RGB, and whether the opaque texels are widened 1555.
fn texels_report(dir: &str, set: &str, count: usize) {
    let pages = sample_pages(dir, set, count);
    let mut alpha = [0usize; 256];
    let mut transparent_rgb: HashMap<[u8; 3], usize> = HashMap::new();
    let (mut not_1555, mut opaque) = (0usize, 0usize);
    let five_bit = |v: u8| v & LOW_THREE_BITS == 0;
    let (mut blocks, mut transparent_blocks, mut mixed_blocks, mut opaque_blocks) = (0usize, 0usize, 0usize, 0usize);
    for page in &pages {
        for t in page.rgba.chunks_exact(4) {
            alpha[t[3] as usize] += 1;
            if t[3] == 0 { *transparent_rgb.entry([t[0], t[1], t[2]]).or_default() += 1; }
            if t[3] == OPAQUE {
                opaque += 1;
                if !(five_bit(t[0]) && five_bit(t[1]) && five_bit(t[2])) { not_1555 += 1; }
            }
        }
        let (width, height) = (page.width as usize, page.height as usize);
        for index in 0..width.div_ceil(BLOCK_EDGE) * height.div_ceil(BLOCK_EDGE) {
            let texels = block_texels(&page.rgba, width, height, index);
            blocks += 1;
            if texels.iter().all(|t| t[3] == 0) { transparent_blocks += 1; }
            else if all_opaque(&texels) { opaque_blocks += 1; }
            else if texels.iter().any(|t| t[3] >= CUTOFF) { mixed_blocks += 1; }
        }
    }
    let total: usize = alpha.iter().sum();
    println!("{set}: {} pages, {total} texels: alpha 0 {:.2} %, 255 {:.2} %, other {:.4} %", pages.len(),
        alpha[0] as f64 / total as f64 * 100.0, alpha[255] as f64 / total as f64 * 100.0,
        (total - alpha[0] - alpha[255]) as f64 / total as f64 * 100.0);
    let mut rgb: Vec<_> = transparent_rgb.into_iter().collect();
    rgb.sort_by(|a, b| b.1.cmp(&a.1));
    println!("  transparent texels' RGB, most common: {:?}", &rgb[..rgb.len().min(5)]);
    println!("  opaque texels not widened from 1555: {not_1555} of {opaque}");
    println!("  blocks {blocks}: transparent {:.1} %, all opaque {:.1} %, mixed with binary-or-soft alpha {:.1} %",
        transparent_blocks as f64 / blocks as f64 * 100.0, opaque_blocks as f64 / blocks as f64 * 100.0,
        mixed_blocks as f64 / blocks as f64 * 100.0);
}

/// The Classic package's per-animation palettes over every page, and the sampled pages as palette indices.
fn palettes(dir: &str, count: usize) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let mut frames_by_page: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, frame) in package.frames().iter().enumerate() {
        frames_by_page.entry(frame.page_index).or_default().push(i);
    }
    // Every page, a frame's distinct opaque colours to its animation, in the order the frame scans them.
    let colours_per_page: Vec<Vec<(u32, Vec<u32>)>> = package.pages().par_iter().map(|record| {
        let page = crop(record.page_index, record.atlas_width, record.used_width, record.used_height,
            record.pixel_format, package.read_page_rgba(record.page_index).unwrap());
        frames_by_page.get(&record.page_index).map(|frames| frames.iter().map(|&i| {
            let f = &package.frames()[i];
            let (mut colours, mut seen) = (Vec::new(), HashSet::new());
            for y in f.y as usize..(f.y + f.height) as usize {
                for x in f.x as usize..(f.x + f.width) as usize {
                    let t = texel(&page.rgba, page.width as usize, x, y);
                    if t[3] >= CUTOFF && seen.insert(u32::from_le_bytes(t)) { colours.push(u32::from_le_bytes(t)); }
                }
            }
            (f.animation_index, colours)
        }).collect()).unwrap_or_default()
    }).collect();
    let mut palettes: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut seen: HashMap<u32, HashSet<u32>> = HashMap::new();
    for (animation, colours) in colours_per_page.into_iter().flatten() {
        let set = seen.entry(animation).or_default();
        let palette = palettes.entry(animation).or_default();
        for colour in colours { if set.insert(colour) { palette.push(colour); } }
    }
    let mut sizes: Vec<usize> = palettes.values().map(Vec::len).collect();
    sizes.sort_unstable();
    let distinct: HashSet<Vec<u32>> = palettes.values().map(|palette| {
        let mut sorted = palette.clone(); sorted.sort_unstable(); sorted }).collect();
    let mut by_body_action: HashMap<(u16, u16), HashSet<u32>> = HashMap::new();
    for (animation, palette) in &palettes {
        let record = &package.animations()[*animation as usize];
        by_body_action.entry((record.body_id, record.action_id)).or_default().extend(palette.iter().copied());
    }
    let over = sizes.iter().filter(|&&size| size > PALETTE_SLOTS).count();
    let body_action_over = by_body_action.values().filter(|set| set.len() > PALETTE_SLOTS).count();
    println!("mobile_cc: {} pages, {} animations with frames, {} distinct palettes", package.pages().len(),
        palettes.len(), distinct.len());
    println!("  colours per animation: median {}, p99 {}, max {}; over {PALETTE_SLOTS}: {over}", sizes[sizes.len() / 2],
        sizes[sizes.len() * 99 / 100], sizes[sizes.len() - 1]);
    println!("  per body and action (directions sharing one palette): {} palettes, over {PALETTE_SLOTS}: \
        {body_action_over}",
        by_body_action.len());
    let palette_bytes: usize = palettes.values().map(|palette| palette.len() * PALETTE_COLOUR_BYTES).sum();
    let distinct_bytes: usize = distinct.iter().map(|palette| palette.len() * PALETTE_COLOUR_BYTES).sum();
    let mut animations: Vec<&u32> = palettes.keys().collect();
    animations.sort_unstable();
    let blob: Vec<u8> = animations.iter().flat_map(|animation| palettes[animation].iter().flat_map(|&colour| {
        let t = colour.to_le_bytes();
        (0x8000 | (u16::from(t[0] >> 3) << 10) | (u16::from(t[1] >> 3) << 5) | u16::from(t[2] >> 3)).to_le_bytes()
    })).collect();
    println!("  palettes as 1555: {palette_bytes} bytes, {distinct_bytes} deduplicated; all of them at Zstd 19: \
        {} bytes",
        zstd::bulk::compress(&blob, ZSTD_LEVELS[1]).unwrap().len());

    // The sampled pages as R8 indices: a frame's texels index its animation's palette (first-seen order), plus 1.
    let pages = sample_pages(dir, "mobile_cc", count);
    let indexed: Vec<(Vec<u8>, usize)> = pages.par_iter().map(|page| {
        let mut indices = vec![0u8; page.width as usize * page.height as usize];
        let mut overflow = 0;
        let lookup: HashMap<u32, HashMap<u32, usize>> = frames_by_page.get(&page.index).into_iter().flatten()
            .map(|&i| package.frames()[i].animation_index).collect::<HashSet<_>>().into_iter()
            .map(|animation| (animation, palettes[&animation].iter().enumerate().map(|(k, &c)| (c, k)).collect()))
            .collect();
        for &i in frames_by_page.get(&page.index).into_iter().flatten() {
            let f = &package.frames()[i];
            let palette = &lookup[&f.animation_index];
            for y in f.y as usize..(f.y + f.height) as usize {
                for x in f.x as usize..(f.x + f.width) as usize {
                    let t = texel(&page.rgba, page.width as usize, x, y);
                    if t[3] < CUTOFF { continue; }
                    let k = palette[&u32::from_le_bytes(t)];
                    if k < PALETTE_SLOTS {
                        indices[y * page.width as usize + x] = (k + 1) as u8;
                    } else {
                        overflow += 1;
                    }
                }
            }
        }
        (indices, overflow)
    }).collect();
    let texels: usize = indexed.iter().map(|(indices, _)| indices.len()).sum();
    let overflow: usize = indexed.iter().map(|(_, overflow)| overflow).sum();
    for level in ZSTD_LEVELS {
        let index_zstd: usize = indexed.par_iter()
            .map(|(indices, _)| zstd::bulk::compress(indices, level).unwrap().len()).sum();
        println!("  {} sampled pages ({:.1} Mpx) as R8 indices, Zstd {level}: {:.2} MiB ({:.2} bpp); texels past \
            {PALETTE_SLOTS} colours: {overflow}", pages.len(), texels as f64 / 1e6, index_zstd as f64 / MIB,
            index_zstd as f64 * 8.0 / texels as f64);
    }
}

/// The page's blocks with alpha (the all-opaque ones made transparent, so page_encode skips them as solid) and its
/// all-opaque blocks of more than one colour packed into a band, [`PACKED_BLOCKS_PER_ROW`] to a row.
fn split_page(page: &Page) -> (Vec<u8>, Vec<u8>, u32, u32) {
    let (width, height) = (page.width as usize, page.height as usize);
    let mut with_alpha = page.rgba.clone();
    let mut packed: Vec<[[u8; 4]; BLOCK_TEXELS]> = Vec::new();
    let blocks_wide = width.div_ceil(BLOCK_EDGE);
    for index in 0..blocks_wide * height.div_ceil(BLOCK_EDGE) {
        let texels = block_texels(&page.rgba, width, height, index);
        if !all_opaque(&texels) || one_colour(&texels) { continue; }
        packed.push(texels);
        let (bx, by) = (index % blocks_wide * BLOCK_EDGE, index / blocks_wide * BLOCK_EDGE);
        for y in by..(by + BLOCK_EDGE).min(height) {
            for x in bx..(bx + BLOCK_EDGE).min(width) { with_alpha[(y * width + x) * 4..][..4].fill(0); }
        }
    }
    let rows = packed.len().div_ceil(PACKED_BLOCKS_PER_ROW).max(1);
    let band_width = PACKED_BLOCKS_PER_ROW * BLOCK_EDGE;
    let mut band = vec![0u8; band_width * rows * BLOCK_EDGE * 4];
    for (k, texels) in packed.iter().enumerate() {
        let (bx, by) = (k % PACKED_BLOCKS_PER_ROW * BLOCK_EDGE, k / PACKED_BLOCKS_PER_ROW * BLOCK_EDGE);
        for (t, value) in texels.iter().enumerate() {
            let at = ((by + t / BLOCK_EDGE) * band_width + bx + t % BLOCK_EDGE) * 4;
            band[at..at + 4].copy_from_slice(value);
        }
    }
    (with_alpha, band, band_width as u32, (rows * BLOCK_EDGE) as u32)
}

/// The GPU's UltraFast on whole pages against the two dispatches, one page at a time and from the rayon threads.
fn split(dir: &str, set: &str, count: usize) {
    let pages = sample_pages(dir, set, count);
    let gpu = PageEncoderGpu::new().unwrap();
    let splits: Vec<_> = pages.par_iter().map(split_page).collect();
    let packed_blocks: usize = splits.iter().map(|(_, _, w, h)| (*w as usize * *h as usize) / BLOCK_TEXELS).sum();
    println!("{set}: {} pages, {packed_blocks} all-opaque blocks of more than one colour packed (with padding)",
        pages.len());
    let whole = |page: &Page| { gpu.encode(&page.rgba, page.width, page.height, Bc7Quality::UltraFast).unwrap(); };
    let two = |page: &Page, (with_alpha, band, band_width, band_height): &(Vec<u8>, Vec<u8>, u32, u32), opaque| {
        gpu.encode(with_alpha, page.width, page.height, Bc7Quality::UltraFast).unwrap();
        gpu.encode_blocks(band, *band_width, *band_height, MobileBlockFormat::Bc7Opaque(opaque)).unwrap();
    };
    for opaque in [Bc7Quality::UltraFast, Bc7Quality::Basic] {
        two(&pages[0], &splits[0], opaque);
    }
    whole(&pages[0]);
    let best = |run: &dyn Fn()| (0..BEST_OF).map(|_| { let started = Instant::now(); run();
        started.elapsed().as_secs_f64() }).fold(f64::INFINITY, f64::min);
    println!("  one thread, whole pages:  {:.3} s", best(&|| pages.iter().for_each(whole)));
    println!("  20 threads, whole pages:  {:.3} s", best(&|| pages.par_iter().for_each(whole)));
    for opaque in [Bc7Quality::UltraFast, Bc7Quality::Basic] {
        println!("  one thread, two dispatches (opaque {opaque:?}): {:.3} s",
            best(&|| pages.iter().zip(&splits).for_each(|(page, split)| two(page, split, opaque))));
        println!("  20 threads, two dispatches (opaque {opaque:?}): {:.3} s",
            best(&|| pages.par_iter().zip(&splits).for_each(|(page, split)| two(page, split, opaque))));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap();
    match first.as_str() {
        "texels" => {
            let (dir, set, count) = (args.next().unwrap(), args.next().unwrap(), args.next().unwrap().parse().unwrap());
            texels_report(&dir, &set, count);
        }
        "palettes" => {
            let (dir, count) = (args.next().unwrap(), args.next().unwrap().parse().unwrap());
            palettes(&dir, count);
        }
        "split" => {
            let (dir, set, count) = (args.next().unwrap(), args.next().unwrap(), args.next().unwrap().parse().unwrap());
            split(&dir, &set, count);
        }
        dir => {
            let set = args.next().unwrap();
            let count: usize = args.next().unwrap().parse().unwrap();
            let names = args.next().map(|list| list.split(',').map(str::to_owned).collect())
                .unwrap_or_else(|| VARIANTS.iter().map(|name| name.to_string()).collect());
            variants_rows(dir, &set, count, names);
        }
    }
}
