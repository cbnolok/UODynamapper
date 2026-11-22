//! The GPU BC7 page encoder (uot_gpu_compute::page_encode) against the packers' CPU encoder (AnalyticalWide) on the
//! same lossless pages: wall time with the pages encoded at once from the rayon threads (the packers' way), Zstd 9
//! size of the blocks, RGB PSNR over the texels with alpha > 0 and alpha PSNR, per GPU profile. Times the encoder's
//! creation first; each profile's pipeline compiles in its first run (cold when MESA_SHADER_CACHE_DIR points at an
//! empty folder), which the faster of its two runs leaves out. Checks that a second GPU run gives the same blocks.
//! Usage: bc7_gpu <uddp dir> <mobile_cc|mobile_ec|art_cc|art_ec|land_ec|dumped> <pages> [profiles, comma-separated]
//! (`dumped`: raw pages in the directory, see `sample_pages`; `BC7_GPU_ZSTD` sets the Zstd level), or one of the
//! modes `compare`, `stats`, `variants`, `fields` and `profile` (their functions).
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage, TexArtCcPackage, TexArtEcPackage, TexLandEcPackage};
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const ZSTD_LEVEL: i32 = 9;
/// Overrides [`ZSTD_LEVEL`]: the packages' Zstd 19, say.
const ZSTD_LEVEL_VARIABLE: &str = "BC7_GPU_ZSTD";

fn zstd_level() -> i32 {
    std::env::var(ZSTD_LEVEL_VARIABLE).ok().and_then(|level| level.parse().ok()).unwrap_or(ZSTD_LEVEL)
}
const PEAK_SQUARED: f64 = 255.0 * 255.0;
const MIB: f64 = 1048576.0;

struct Page { width: u32, height: u32, rgba: Vec<u8> }

fn pick(total: usize, count: usize) -> Vec<usize> {
    let count = count.min(total);
    (0..count).map(|k| k * total / count).collect()
}

/// A page's used rectangle from its decoded texels: `rgba` is either that rectangle or the whole `atlas_width`-wide
/// atlas.
fn page(atlas_width: u32, width: u32, height: u32, format: PagePixelFormat, rgba: Vec<u8>) -> Page {
    assert_eq!(format, PagePixelFormat::Rgba8888, "a lossless source (pack it with --raw)");
    let used = width as usize * height as usize * 4;
    let rgba = if rgba.len() == used { rgba } else {
        rgba.chunks_exact(atlas_width as usize * 4).take(height as usize)
            .flat_map(|row| row[..width as usize * 4].to_vec()).collect()
    };
    Page { width, height, rgba }
}

fn sample_pages(dir: &str, set: &str, count: usize) -> Vec<Page> {
    match set {
        "mobile_cc" => {
            let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
            pick(package.pages().len(), count).into_iter().map(|i| {
                let r = package.pages()[i];
                page(r.atlas_width, r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(r.page_index).unwrap())
            }).collect()
        }
        "mobile_ec" => {
            let package = MobileAnimEcPackage::load(format!("{dir}/mobile_anim_ec.uddp")).unwrap();
            pick(package.pages().len(), count).into_iter().map(|i| {
                let r = package.pages()[i];
                page(r.atlas_width, r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(r.page_index).unwrap())
            }).collect()
        }
        "art_cc" => {
            let package = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
            pick(package.pages().len(), count).into_iter().map(|i| {
                let r = &package.pages()[i];
                page(package.atlas_width(), r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(i as u32).unwrap())
            }).collect()
        }
        "art_ec" => {
            let package = TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap();
            pick(package.pages().len(), count).into_iter().map(|i| {
                let r = &package.pages()[i];
                page(package.atlas_width(), r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(i as u32).unwrap())
            }).collect()
        }
        "land_ec" => {
            let package = TexLandEcPackage::load(format!("{dir}/tex_land_ec.uddp")).unwrap();
            pick(package.pages().len(), count).into_iter().map(|i| {
                let r = &package.pages()[i];
                page(package.atlas_width(), r.used_width, r.used_height, r.pixel_format,
                    package.read_page_rgba(i as u32).unwrap())
            }).collect()
        }
        // Pages as a packer hands them to its BC7 encoder, `<width>x<height>_<n>.rgba` (RGBA8 rows) in `dir`: a BC7
        // pack lays its pages out for BC7 (slots on whole blocks), unlike the lossless packs the other sets read.
        "dumped" => {
            let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "rgba")).collect();
            files.sort();
            pick(files.len(), count).into_iter().map(|i| {
                let name = files[i].file_stem().unwrap().to_str().unwrap().to_owned();
                let (width, height) = name.split('_').next().unwrap().split_once('x').unwrap();
                Page { width: width.parse().unwrap(), height: height.parse().unwrap(),
                    rgba: std::fs::read(&files[i]).unwrap() }
            }).collect()
        }
        other => panic!("unknown page set {other}"),
    }
}

/// Squared errors and counts: RGB over the texels with alpha > 0, and alpha over all.
fn errors(source: &[u8], decoded: &[u8]) -> [f64; 4] {
    let (mut rgb, mut rgb_count, mut alpha) = (0.0, 0.0, 0.0);
    for (s, d) in source.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        if s[3] > 0 {
            for c in 0..3 { rgb += (s[c] as f64 - d[c] as f64).powi(2); }
            rgb_count += 3.0;
        }
        alpha += (s[3] as f64 - d[3] as f64).powi(2);
    }
    [rgb, rgb_count, alpha, (source.len() / 4) as f64]
}

fn psnr(squared: f64, count: f64) -> f64 {
    10.0 * (PEAK_SQUARED / (squared / count)).log10()
}

fn print(label: &str, pages: &[Page], blocks: &[Vec<u8>], wall: f64, cpu_zstd: Option<usize>) -> usize {
    let rows: Vec<(usize, [f64; 4])> = pages.par_iter().zip(blocks).map(|(page, blocks)| {
        let extent = ImageExtent::new(page.width, page.height).unwrap();
        let decoded = decode_bc7_to_rgba8888(blocks, extent).unwrap();
        (zstd::bulk::compress(blocks, zstd_level()).unwrap().len(), errors(&page.rgba, &decoded))
    }).collect();
    let zstd: usize = rows.iter().map(|row| row.0).sum();
    let sum = |i: usize| rows.iter().map(|row| row.1[i]).sum::<f64>();
    let versus = cpu_zstd.map(|cpu| format!(" ({:+5.1} %)", (zstd as f64 / cpu as f64 - 1.0) * 100.0))
        .unwrap_or_default();
    println!("  {label:18} wall {wall:7.2} s  zstd {:8.2} MiB{versus:10}  RGB PSNR {:5.2} dB  alpha PSNR {:5.2} dB",
        zstd as f64 / MIB, psnr(sum(0), sum(1)), psnr(sum(2), sum(3)));
    zstd
}

/// A package's slots by id: their page and rectangle (x, y, width, height).
type Slots = std::collections::HashMap<u32, (u32, [usize; 4])>;

/// The slots and decoded pages (with their used widths) of a `set` package in `dir`.
fn slots_and_pages(dir: &str, set: &str) -> (Slots, Vec<(usize, Vec<u8>)>) {
    macro_rules! read {
        ($package:expr) => {{
            let package = $package;
            let slots = package.slots().iter().filter(|slot| slot.width > 0 && slot.height > 0)
                .map(|slot| (slot.art_id, (slot.page_index,
                    [slot.x as usize, slot.y as usize, slot.width as usize, slot.height as usize]))).collect();
            let pages = (0..package.pages().len()).map(|index| {
                (package.pages()[index].used_width as usize, package.read_page_rgba(index as u32).unwrap())
            }).collect();
            (slots, pages)
        }};
    }
    match set {
        "art_cc" => read!(TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap()),
        "art_ec" => read!(TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap()),
        "land_ec" => read!(TexLandEcPackage::load(format!("{dir}/tex_land_ec.uddp")).unwrap()),
        other => panic!("compare takes art_cc, art_ec or land_ec, not {other}"),
    }
}

fn slot_texels(pages: &[(usize, Vec<u8>)], (page, [x, y, width, height]): (u32, [usize; 4])) -> Vec<u8> {
    let (used_width, rgba) = &pages[page as usize];
    (y..y + height).flat_map(|row| rgba[(row * used_width + x) * 4..][..width * 4].to_vec()).collect()
}

/// `compare <set> <lossless dir> <BC7 dir>`: the RGB and alpha PSNR of a BC7 package's images (decoded by the
/// package) against the same images of a lossless one packed from the same sources, matched by id: the two lay out
/// their pages differently.
fn compare(mut args: impl Iterator<Item = String>) {
    let set = args.next().unwrap();
    let (lossless_slots, lossless_pages) = slots_and_pages(&args.next().unwrap(), &set);
    let lossy_dir = args.next().unwrap();
    let (lossy_slots, lossy_pages) = slots_and_pages(&lossy_dir, &set);
    let (mut sums, mut images, mut unmatched) = ([0.0; 4], 0, 0);
    for (id, slot) in &lossless_slots {
        match lossy_slots.get(id) {
            Some(lossy) if lossy.1[2..] == slot.1[2..] => {
                let row = errors(&slot_texels(&lossless_pages, *slot), &slot_texels(&lossy_pages, *lossy));
                for (sum, value) in sums.iter_mut().zip(row) { *sum += value; }
                images += 1;
            }
            _ => unmatched += 1,
        }
    }
    println!("{set} {lossy_dir}: {images} images ({unmatched} unmatched), RGB PSNR {:5.2} dB, alpha PSNR {:5.2} dB",
        psnr(sums[0], sums[1]), psnr(sums[2], sums[3]));
}

/// The kinds of source block `stats` sorts the blocks into.
const KINDS: [&str; 4] = ["transparent", "solid", "opaque", "soft alpha"];

/// A block's kind: every texel's alpha 0, every texel the same, every alpha 255, or other.
fn kind(texels: &[[u8; 4]; 16]) -> usize {
    if texels.iter().all(|texel| texel[3] == 0) { 0 }
    else if texels.iter().all(|texel| *texel == texels[0]) { 1 }
    else if texels.iter().all(|texel| texel[3] == 255) { 2 }
    else { 3 }
}

/// The 16 texels of block `index` of a page, its edge texels repeated past its right and bottom edges.
fn block_texels(page: &Page, index: usize) -> [[u8; 4]; 16] {
    let blocks_x = page.width.div_ceil(4) as usize;
    let (bx, by) = (index % blocks_x, index / blocks_x);
    std::array::from_fn(|i| {
        let x = (bx * 4 + i % 4).min(page.width as usize - 1);
        let y = (by * 4 + i / 4).min(page.height as usize - 1);
        page.rgba[(y * page.width as usize + x) * 4..][..4].try_into().unwrap()
    })
}

/// A BC7 block's mode (8 for a block of no mode).
fn mode(block: &[u8]) -> usize {
    (block[0].trailing_zeros() as usize).min(8)
}

/// `stats <dir> <set> <pages> [profile]`: where the GPU's blocks lose to the CPU's under Zstd. Sorts the source blocks
/// by kind ([`KINDS`]) and gives, per kind, the blocks, their distinct encodings and modes on each encoder, and the
/// Zstd size of the GPU's blocks with that kind's blocks taken from the CPU.
fn stats(mut args: impl Iterator<Item = String>) {
    let dir = args.next().unwrap();
    let set = args.next().unwrap();
    let count: usize = args.next().unwrap().parse().unwrap();
    let quality = args.next().map(|name| *Bc7Quality::ALL.iter()
        .find(|quality| format!("{quality:?}").eq_ignore_ascii_case(&name)).expect("a profile"))
        .unwrap_or(Bc7Quality::UltraFast);
    let pages = sample_pages(&dir, &set, count);
    let gpu = PageEncoderGpu::new().unwrap();
    let encoded: Vec<(Vec<u8>, Vec<u8>, Vec<usize>)> = pages.par_iter().map(|page| {
        let extent = ImageExtent::new(page.width, page.height).unwrap();
        let cpu = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap().into_blocks();
        let gpu = gpu.encode(&page.rgba, page.width, page.height, quality).unwrap().concat();
        let kinds = (0..cpu.len() / 16).map(|index| kind(&block_texels(page, index))).collect();
        (cpu, gpu, kinds)
    }).collect();
    let zstd = |pick: &(dyn Fn(usize, usize) -> bool + Sync)| -> usize {
        encoded.par_iter().map(|(cpu, gpu, kinds)| {
            let mixed: Vec<u8> = kinds.iter().enumerate().flat_map(|(index, &kind)| {
                let blocks = if pick(index, kind) { cpu } else { gpu };
                blocks[index * 16..][..16].to_vec()
            }).collect();
            zstd::bulk::compress(&mixed, zstd_level()).unwrap().len()
        }).sum()
    };
    let cpu_size = zstd(&|_, _| true);
    let gpu_size = zstd(&|_, _| false);
    println!("{set} {quality:?}: {} pages; Zstd {}: CPU {cpu_size}, GPU {gpu_size} ({:+.2} %)", pages.len(), zstd_level(),
        (gpu_size as f64 / cpu_size as f64 - 1.0) * 100.0);
    let total: usize = encoded.iter().map(|page| page.2.len()).sum();
    for (k, name) in KINDS.iter().enumerate() {
        let mut modes = [[0usize; 9]; 2];
        let mut distinct = [std::collections::HashSet::new(), std::collections::HashSet::new()];
        let mut blocks = 0;
        for (cpu, gpu, kinds) in &encoded {
            for (index, _) in kinds.iter().enumerate().filter(|(_, kind)| **kind == k) {
                blocks += 1;
                for (side, encoded) in [cpu, gpu].into_iter().enumerate() {
                    let block = &encoded[index * 16..][..16];
                    modes[side][mode(block)] += 1;
                    distinct[side].insert(block.to_vec());
                }
            }
        }
        let swapped = zstd(&|_, kind| kind == k);
        println!("  {name:12} {blocks:9} blocks ({:4.1} %); distinct CPU {} GPU {}; GPU with the CPU's: {swapped} \
            ({:+.2} % of the GPU's)", blocks as f64 * 100.0 / total as f64, distinct[0].len(), distinct[1].len(),
            (swapped as f64 / gpu_size as f64 - 1.0) * 100.0);
        for (side, label) in ["CPU", "GPU"].iter().enumerate() {
            let histogram: Vec<String> = modes[side].iter().enumerate().filter(|(_, n)| **n > 0)
                .map(|(mode, n)| format!("m{mode} {:.1} %", *n as f64 * 100.0 / blocks.max(1) as f64)).collect();
            println!("    {label} modes: {}", histogram.join(", "));
        }
    }
}

/// `variants <dir> <set> <pages>`: the Zstd size and PSNR of the GPU's UltraFast blocks with some kinds of blocks
/// encoded otherwise: the CPU's blocks where the 16 texels are one colour (its canonical solid blocks), and the opaque
/// profile's blocks where every alpha is 255.
fn variants(mut args: impl Iterator<Item = String>) {
    let dir = args.next().unwrap();
    let set = args.next().unwrap();
    let count: usize = args.next().unwrap().parse().unwrap();
    let pages = sample_pages(&dir, &set, count);
    let gpu = PageEncoderGpu::new().unwrap();
    let opaque_format = uot_gpu_compute::block_encode::MobileBlockFormat::Bc7Opaque(Bc7Quality::UltraFast);
    // Per page: CPU, GPU, GPU opaque profile, and per block (uniform, opaque).
    let encoded: Vec<[Vec<u8>; 3]> = pages.par_iter().map(|page| {
        let extent = ImageExtent::new(page.width, page.height).unwrap();
        let cpu = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap().into_blocks();
        let ultra = gpu.encode(&page.rgba, page.width, page.height, Bc7Quality::UltraFast).unwrap().concat();
        let opaque = gpu.encode_blocks(&page.rgba, page.width, page.height, opaque_format).unwrap();
        [cpu, ultra, opaque]
    }).collect();
    let flags: Vec<Vec<(bool, bool)>> = pages.par_iter().zip(&encoded).map(|(page, blocks)| {
        (0..blocks[0].len() / 16).map(|index| {
            let texels = block_texels(page, index);
            (texels.iter().all(|texel| *texel == texels[0]), texels.iter().all(|texel| texel[3] == 255))
        }).collect()
    }).collect();
    // A variant: which encoding (0 CPU, 1 GPU, 2 opaque profile) each block takes, from its (uniform, opaque).
    let variant = |label: &str, pick: &(dyn Fn(bool, bool) -> usize + Sync)| {
        let rows: Vec<(usize, [f64; 4])> = (0..pages.len()).into_par_iter().map(|p| {
            let page = &pages[p];
            let blocks: Vec<u8> = flags[p].iter().enumerate()
                .flat_map(|(index, &(uniform, opaque))| encoded[p][pick(uniform, opaque)][index * 16..][..16].to_vec())
                .collect();
            let decoded = decode_bc7_to_rgba8888(&blocks, ImageExtent::new(page.width, page.height).unwrap()).unwrap();
            (zstd::bulk::compress(&blocks, zstd_level()).unwrap().len(), errors(&page.rgba, &decoded))
        }).collect();
        let zstd: usize = rows.iter().map(|row| row.0).sum();
        let sum = |i: usize| rows.iter().map(|row| row.1[i]).sum::<f64>();
        println!("  {label:36} zstd {zstd:9}  RGB PSNR {:6.3} dB  alpha PSNR {:6.2} dB", psnr(sum(0), sum(1)),
            psnr(sum(2), sum(3)));
    };
    println!("{set}: {} pages", pages.len());
    variant("CPU", &|_, _| 0);
    variant("GPU UltraFast", &|_, _| 1);
    variant("GPU, CPU's uniform blocks", &|uniform, _| if uniform { 0 } else { 1 });
    variant("GPU, opaque profile on opaque blocks", &|_, opaque| if opaque { 2 } else { 1 });
    variant("GPU, both", &|uniform, opaque| if uniform { 0 } else if opaque { 2 } else { 1 });
}

/// Bits `from..from + count` of a block.
fn bits(block: &[u8], from: usize, count: usize) -> u32 {
    (0..count).map(|i| (u32::from(block[(from + i) / 8] >> ((from + i) % 8)) & 1) << i).sum()
}

/// `fields <dir> <set> <pages>`: the fields of the mode 6 blocks of the opaque blocks (every alpha 255) on the CPU and
/// the GPU: their alpha endpoints and p-bits, and the order-0 entropy of each field.
fn fields(mut args: impl Iterator<Item = String>) {
    let dir = args.next().unwrap();
    let set = args.next().unwrap();
    let count: usize = args.next().unwrap().parse().unwrap();
    let pages = sample_pages(&dir, &set, count);
    let gpu = PageEncoderGpu::new().unwrap();
    // Mode 6: 7 mode bits, R0 R1 G0 G1 B0 B1 A0 A1 of 7 bits, P0, P1, then 63 index bits.
    const FIELDS: [(&str, usize, usize); 12] = [("R0", 7, 7), ("R1", 14, 7), ("G0", 21, 7), ("G1", 28, 7),
        ("B0", 35, 7), ("B1", 42, 7), ("A0", 49, 7), ("A1", 56, 7), ("P0", 63, 1), ("P1", 64, 1),
        ("idx lo", 65, 31), ("idx hi", 96, 32)];
    let mut histograms = vec![[std::collections::HashMap::<u32, usize>::new(), Default::default()]; FIELDS.len()];
    let mut alpha = [std::collections::HashMap::<(u32, u32, u32, u32), usize>::new(), Default::default()];
    let mut both = 0usize;
    let mut same_endpoints = 0usize;
    for page in &pages {
        let extent = ImageExtent::new(page.width, page.height).unwrap();
        let cpu = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap().into_blocks();
        let ultra = gpu.encode(&page.rgba, page.width, page.height, Bc7Quality::UltraFast).unwrap().concat();
        for index in 0..cpu.len() / 16 {
            let texels = block_texels(page, index);
            if !texels.iter().all(|texel| texel[3] == 255) || texels.iter().all(|texel| *texel == texels[0]) {
                continue;
            }
            let blocks = [&cpu[index * 16..][..16], &ultra[index * 16..][..16]];
            if blocks.iter().any(|block| mode(block) != 6) { continue; }
            both += 1;
            if bits(blocks[0], 7, 42) == bits(blocks[1], 7, 42) { same_endpoints += 1; }
            for (side, block) in blocks.iter().enumerate() {
                for (f, &(_, from, n)) in FIELDS.iter().enumerate() {
                    *histograms[f][side].entry(bits(block, from, n)).or_default() += 1;
                }
                *alpha[side].entry((bits(block, 49, 7), bits(block, 56, 7), bits(block, 63, 1), bits(block, 64, 1)))
                    .or_default() += 1;
            }
        }
    }
    let entropy = |histogram: &std::collections::HashMap<u32, usize>| -> f64 {
        let total: usize = histogram.values().sum();
        histogram.values().map(|&n| { let p = n as f64 / total as f64; -p * p.log2() }).sum()
    };
    println!("{set}: {both} opaque blocks in mode 6 on both, {same_endpoints} with the same colour endpoints");
    for (f, &(name, _, n)) in FIELDS.iter().enumerate() {
        println!("  {name:7} ({n:2} bits): entropy CPU {:6.3}, GPU {:6.3}", entropy(&histograms[f][0]),
            entropy(&histograms[f][1]));
    }
    for (side, label) in ["CPU", "GPU"].iter().enumerate() {
        let mut top: Vec<_> = alpha[side].iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        let shown: Vec<String> = top.iter().take(6).map(|((a0, a1, p0, p1), n)| {
            format!("A {a0}/{a1} P {p0}/{p1}: {:.1} %", **n as f64 * 100.0 / both as f64)
        }).collect();
        println!("  {label} alpha and p-bits: {}", shown.join(", "));
    }
}

/// `profile <dir> <set> <pages> [profile] [packed]`: where a page's encode time goes, one page at a time on one thread,
/// in bands of at most `PROFILE_BAND_BLOCKS` blocks: the band's texels made on the CPU, their upload, the kernel
/// (timestamp queries) and the readback, summed over the pages. The bands are whole block rows of the page, or with
/// `packed` its blocks of more than one colour side by side, 1024 to a row, as `page_encode` sends them since
/// 2026-11-22 (the CPU making the others' solid blocks, counted in the texels).
fn profile(mut args: impl Iterator<Item = String>) {
    use uot_gpu_compute::block_encode::{padded_size, MobileBlockEncoder, MobileBlockFormat};
    use wgpu::util::DeviceExt;
    const PROFILE_BAND_BLOCKS: u32 = 1 << 17;
    const RUNS: usize = 3;
    let dir = args.next().unwrap();
    let set = args.next().unwrap();
    let count: usize = args.next().unwrap().parse().unwrap();
    let quality = args.next().map(|name| *Bc7Quality::ALL.iter()
        .find(|quality| format!("{quality:?}").eq_ignore_ascii_case(&name)).expect("a profile"))
        .unwrap_or(Bc7Quality::UltraFast);
    let packed = args.next().is_some_and(|arg| arg == "packed");
    const PACKED_ROW_BLOCKS: usize = 1024;
    let format = MobileBlockFormat::Bc7(quality);
    let pages = sample_pages(&dir, &set, count);
    // A page's bands: their texels, width and height.
    let bands = |page: &Page| -> Vec<(Vec<u8>, u32, u32)> {
        let (padded_width, padded_height) = padded_size(page.width, page.height);
        if packed {
            let blocks = (padded_width / 4 * padded_height / 4) as usize;
            let mut solid = 0;
            let others: Vec<[[u8; 4]; 16]> = (0..blocks).map(|index| block_texels(page, index))
                .filter(|texels| {
                    let uniform = texels.iter().all(|texel| *texel == texels[0]);
                    if uniform {
                        let mut block = [0u8; 16];
                        uot_texture_codecs::bc7::analytical::pack_bc7_rgba(&mut block, texels, 0);
                        solid += usize::from(block[0]);
                    }
                    !uniform
                }).collect();
            std::hint::black_box(solid);
            return others.chunks(PROFILE_BAND_BLOCKS as usize).map(|band| {
                let columns = PACKED_ROW_BLOCKS.min(band.len());
                let rows = band.len().div_ceil(columns);
                let row_bytes = columns * 16;
                let mut texels = vec![0u8; rows * 4 * row_bytes];
                for (index, block) in band.iter().enumerate() {
                    let (column, row) = (index % columns, index / columns);
                    for (texel_row, block_row) in block.chunks_exact(4).enumerate() {
                        texels[(row * 4 + texel_row) * row_bytes + column * 16..][..16]
                            .copy_from_slice(block_row.as_flattened());
                    }
                }
                (texels, (columns * 4) as u32, (rows * 4) as u32)
            }).collect();
        }
        let blocks_x = padded_width / 4;
        let band_height = (PROFILE_BAND_BLOCKS / blocks_x).max(1) * 4;
        (0..padded_height).step_by(band_height as usize).map(|top| {
            let height = band_height.min(padded_height - top);
            let mut texels = Vec::with_capacity((padded_width * height * 4) as usize);
            for y in top..top + height {
                let row = &page.rgba[(y.min(page.height - 1) * page.width * 4) as usize..][..(page.width * 4) as usize];
                texels.extend_from_slice(row);
                for _ in page.width..padded_width { texels.extend_from_slice(&row[row.len() - 4..]); }
            }
            (texels, padded_width, height)
        }).collect()
    };
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.unwrap();
        adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TIMESTAMP_QUERY, required_limits: adapter.limits(), ..Default::default()
        }).await.unwrap()
    });
    let period = f64::from(queue.get_timestamp_period());
    let mut encoder = MobileBlockEncoder::lazy(&device);
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None, ty: wgpu::QueryType::Timestamp, count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 16, mapped_at_creation: false,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    });
    let read = |buffer: &wgpu::Buffer| -> Vec<u8> {
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = slice.get_mapped_range().unwrap().to_vec();
        buffer.unmap();
        bytes
    };
    let mut best = [f64::MAX; 5];
    for run in 0..RUNS {
        // Texels, upload, kernel, readback, wall.
        let mut sums = [0.0f64; 5];
        let wall = Instant::now();
        for page in &pages {
            let started = Instant::now();
            let page_bands = bands(page);
            sums[0] += started.elapsed().as_secs_f64();
            for (texels, padded_width, height) in page_bands {
                let started = Instant::now();
                let texture = device.create_texture_with_data(&queue, &wgpu::TextureDescriptor {
                    label: None, size: wgpu::Extent3d { width: padded_width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }, wgpu::util::TextureDataOrder::LayerMajor, &texels);
                queue.submit([]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                sums[1] += started.elapsed().as_secs_f64();
                let view = texture.create_view(&Default::default());
                let bytes = format.encoded_bytes(padded_width, height);
                let output = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None, size: bytes, mapped_at_creation: false,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                });
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None, size: bytes, mapped_at_creation: false,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                });
                let stamps = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None, size: 16, mapped_at_creation: false,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                });
                let mut commands = device.create_command_encoder(&Default::default());
                {
                    let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: None, timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                            query_set: &queries, beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        }),
                    });
                    encoder.add_frame(format, &view, padded_width, height, &output, 0);
                    encoder.record(&mut pass);
                }
                commands.resolve_query_set(&queries, 0..2, &resolve, 0);
                commands.copy_buffer_to_buffer(&resolve, 0, &stamps, 0, 16);
                queue.submit([commands.finish()]);
                let ticks: Vec<u64> = bytemuck::cast_slice(&read(&stamps)).to_vec();
                sums[2] += (ticks[1] - ticks[0]) as f64 * period * 1e-9;
                let started = Instant::now();
                let mut commands = device.create_command_encoder(&Default::default());
                commands.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
                queue.submit([commands.finish()]);
                let blocks = read(&readback);
                assert_eq!(blocks.len() as u64, bytes);
                sums[3] += started.elapsed().as_secs_f64();
            }
        }
        sums[4] = wall.elapsed().as_secs_f64();
        println!("  run {run}: texels {:.3} s, upload {:.3} s, kernel {:.3} s, readback {:.3} s, wall {:.3} s",
            sums[0], sums[1], sums[2], sums[3], sums[4]);
        if run > 0 { for (best, sum) in best.iter_mut().zip(sums) { *best = best.min(sum); } }
    }
    let texels: f64 = pages.iter().map(|page| f64::from(page.width) * f64::from(page.height)).sum();
    println!("{set} {quality:?}{}, {} pages, {:.1} Mpx, one thread, best of {}: texels {:.3} s, upload {:.3} s, kernel \
        {:.3} s ({:.0} Mpx/s), readback {:.3} s, wall {:.3} s", if packed { " packed" } else { "" }, pages.len(), texels / 1e6, RUNS - 1, best[0], best[1],
        best[2], texels / 1e6 / best[2], best[3], best[4]);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap();
    if dir == "compare" { return compare(args); }
    if dir == "stats" { return stats(args); }
    if dir == "variants" { return variants(args); }
    if dir == "fields" { return fields(args); }
    if dir == "profile" { return profile(args); }
    let set = args.next().unwrap();
    let count: usize = args.next().unwrap().parse().unwrap();
    let profiles: Vec<Bc7Quality> = match args.next() {
        None => Bc7Quality::ALL.to_vec(),
        Some(list) => list.split(',').map(|name| *Bc7Quality::ALL.iter()
            .find(|quality| format!("{quality:?}").eq_ignore_ascii_case(name)).expect("a profile")).collect(),
    };
    let pages = sample_pages(&dir, &set, count);
    let texels: f64 = pages.iter().map(|page| f64::from(page.width) * f64::from(page.height)).sum();
    println!("{set}: {} pages, {:.1} Mpx, {} threads", pages.len(), texels / 1e6, rayon::current_num_threads());

    let started = Instant::now();
    let gpu = PageEncoderGpu::new().unwrap();
    println!("encoder created in {:.1} ms on {}", started.elapsed().as_secs_f64() * 1e3, gpu.adapter());

    let started = Instant::now();
    let cpu: Vec<Vec<u8>> = pages.par_iter().map(|page| {
        let extent = ImageExtent::new(page.width, page.height).unwrap();
        encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide).unwrap()
            .into_blocks()
    }).collect();
    let cpu_zstd = print("CPU AnalyticalWide", &pages, &cpu, started.elapsed().as_secs_f64(), None);

    for quality in profiles {
        let run = || {
            let started = Instant::now();
            let blocks: Vec<Vec<u8>> = pages.par_iter().map(|page| {
                gpu.encode(&page.rgba, page.width, page.height, quality).unwrap().concat()
            }).collect();
            (blocks, started.elapsed().as_secs_f64())
        };
        let (first, first_wall) = run();
        let (second, second_wall) = run();
        print(&format!("GPU {quality:?}"), &pages, &first, first_wall.min(second_wall), Some(cpu_zstd));
        println!("    runs {first_wall:.2} s and {second_wall:.2} s; same blocks: {}", first == second);
    }
}
