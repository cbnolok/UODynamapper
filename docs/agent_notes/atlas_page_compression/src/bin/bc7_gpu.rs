//! The GPU BC7 page encoder (uot_gpu_compute::page_encode) against the packers' CPU encoder (AnalyticalWide) on the
//! same lossless pages: wall time with the pages encoded at once from the rayon threads (the packers' way), Zstd 9
//! size of the blocks, RGB PSNR over the texels with alpha > 0 and alpha PSNR, per GPU profile. Times the encoder's
//! creation first (its pipelines' compile; cold when MESA_SHADER_CACHE_DIR points at an empty folder), and checks
//! that a second GPU run gives the same blocks.
//! Usage: bc7_gpu <uddp dir> <mobile_cc|mobile_ec|art_cc|art_ec|land_ec> <pages> [profiles, comma-separated]
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage, TexArtCcPackage, TexArtEcPackage, TexLandEcPackage};
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::page_encode::Bc7PageEncoderGpu;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const ZSTD_LEVEL: i32 = 9;
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
        (zstd::bulk::compress(blocks, ZSTD_LEVEL).unwrap().len(), errors(&page.rgba, &decoded))
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

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap();
    if dir == "compare" { return compare(args); }
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
    let gpu = Bc7PageEncoderGpu::new().unwrap();
    println!("encoder created (compile) in {:.1} ms on {}", started.elapsed().as_secs_f64() * 1e3, gpu.adapter());

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
