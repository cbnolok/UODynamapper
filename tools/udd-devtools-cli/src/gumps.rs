//! Gump package quality, exact pixel comparison and storage statistics.
//! gumps <lossless> <lossy> <csv>: every single gump, composited over grey on a canvas of at least MIN_SIDE;
//!   --linear blends in linear light, otherwise in sRGB values.
//! same-pixels <a> <b>: every gump (single and atlas) of both packages decodes to the same RGBA.
//! sizes <package>: single gumps per pixel format with their pixel bytes (their VRAM as uploaded blocks), and the
//!   atlas pages' RGBA8888 bytes.
use color_eyre::eyre;
use clap::{Args, Subcommand};
use rayon::prelude::*;
use ssimulacra2::{compute_frame_ssimulacra2, ColorPrimaries, LinearRgb, Rgb, TransferCharacteristic};
use std::collections::BTreeMap;
use udd_assets::gumps::{GumpPixelFormat, GumpsPackage};

const BACKGROUND: f32 = 128.0 / 255.0;
const MIN_SIDE: u32 = 64;
const RGBA: usize = 4;
const ALPHA: usize = 3;

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

/// Blends over grey in sRGB values, or in linear light when requested (as the bench's mosaics did).
fn canvas(width: u32, height: u32, rgba: &[u8], linear: bool) -> LinearRgb {
    let (canvas_width, canvas_height) = (width.max(MIN_SIDE), height.max(MIN_SIDE));
    let background = if linear { srgb_to_linear(BACKGROUND) } else { BACKGROUND };
    let mut data = vec![[background; 3]; (canvas_width * canvas_height) as usize];
    for y in 0..height {
        for x in 0..width {
            let pixel = &rgba[((y * width + x) as usize) * RGBA..][..RGBA];
            let alpha = f32::from(pixel[ALPHA]) / 255.0;
            let target = &mut data[(y * canvas_width + x) as usize];
            for channel in 0..3 {
                let value = f32::from(pixel[channel]) / 255.0;
                let value = if linear { srgb_to_linear(value) } else { value };
                target[channel] = background * (1.0 - alpha) + value * alpha;
            }
        }
    }
    if linear {
        return LinearRgb::new(data, canvas_width as usize, canvas_height as usize).expect("canvas");
    }
    let rgb = Rgb::new(data, canvas_width as usize, canvas_height as usize, TransferCharacteristic::SRGB, ColorPrimaries::BT709)
        .expect("canvas");
    LinearRgb::try_from(rgb).expect("linear canvas")
}

fn gumps(lossless: &str, lossy: &str, out: &str, linear: bool) -> eyre::Result<()> {
    let reference = GumpsPackage::load(lossless)?;
    let test = GumpsPackage::load(lossy)?;
    let ids = reference
        .available_gump_ids()
        .into_iter()
        .filter(|&id| reference.atlas_slot(id).is_none())
        .collect::<Vec<_>>();
    let rows = ids
        .par_iter()
        .map(|&id| -> eyre::Result<Option<String>> {
            let (width, height, a) = reference.read_gump_rgba(id)?;
            let (test_width, test_height, b) = test.read_gump_rgba(id)?;
            eyre::ensure!((width, height) == (test_width, test_height), "gump {id} size differs");
            if width == 0 || height == 0 {
                return Ok(None);
            }
            let soft = a.chunks_exact(RGBA).any(|p| p[ALPHA] != 0 && p[ALPHA] != 255);
            let alpha_changed = a.chunks_exact(RGBA).zip(b.chunks_exact(RGBA)).filter(|(p, q)| p[ALPHA] != q[ALPHA]).count();
            let value = compute_frame_ssimulacra2(canvas(width, height, &a, linear), canvas(width, height, &b, linear))?;
            Ok(Some(format!("{id},{width},{height},{},{alpha_changed},{value:.4}\n", u8::from(soft))))
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    let mut csv = String::from("id,width,height,soft_alpha,alpha_changed,ssimulacra2\n");
    rows.into_iter().flatten().for_each(|row| csv.push_str(&row));
    std::fs::write(out, csv)?;
    Ok(())
}

fn same_pixels(a: &str, b: &str) -> eyre::Result<()> {
    let (a, b) = (GumpsPackage::load(a)?, GumpsPackage::load(b)?);
    let (a_ids, b_ids) = (a.available_gump_ids(), b.available_gump_ids());
    eyre::ensure!(a_ids == b_ids, "gump id sets differ: {} vs {}", a_ids.len(), b_ids.len());
    let atlas = a_ids.iter().filter(|&&id| a.atlas_slot(id).is_some()).count();
    let differing = a_ids
        .par_iter()
        .filter(|&&id| {
            let left = a.read_gump_image(id).expect("reference gump");
            let right = b.read_gump_image(id).expect("candidate gump");
            (left.physical_width, left.physical_height, left.upscale_factor)
                != (right.physical_width, right.physical_height, right.upscale_factor)
                || left.canvas_rgba() != right.canvas_rgba()
        })
        .count();
    println!("{} gumps ({atlas} on atlas pages), {differing} differ", a_ids.len());
    eyre::ensure!(differing == 0, "gump pixels differ");
    eyre::ensure!(a.shared_classic_gumps().eq(b.shared_classic_gumps()), "shared Classic gump tables differ");
    Ok(())
}

fn sizes(package: &str) -> eyre::Result<()> {
    let package = GumpsPackage::load(package)?;
    let mut formats: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    for id in package.available_gump_ids() {
        let Some(format) = package.single_gump_pixel_format(id)? else { continue };
        let image = package.read_gump_image(id)?;
        let bytes = format.pixel_byte_len(image.physical_width, image.physical_height) as u64;
        let entry = formats.entry(format.label()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
    }
    for (label, (count, bytes)) in &formats {
        println!("single {label}: {count} gumps, {bytes} pixel bytes ({:.2} MB)", *bytes as f64 / 1e6);
    }
    let page_bytes = package.atlas_pages().iter().map(|page| u64::from(page.used_width) * u64::from(page.used_height) * RGBA as u64).sum::<u64>();
    println!("atlas: {} pages, {page_bytes} RGBA8888 bytes ({:.2} MB)", package.atlas_pages().len(), page_bytes as f64 / 1e6);
    let _ = GumpPixelFormat::Rgba8888;
    Ok(())
}


#[derive(Args, Debug)]
pub struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Score single gumps against a lossless reference, writing a CSV.
    Score {
        reference: String,
        candidate: String,
        output: String,
        /// Blend over grey in linear light instead of sRGB values.
        #[arg(long)]
        linear: bool,
    },
    /// Fail if logical gump pixels or shared-gump tables differ.
    SamePixels { reference: String, candidate: String },
    /// Report per-format pixel storage and atlas storage.
    Sizes { package: String },
}

pub fn run(args: Arguments) -> eyre::Result<()> {
    match args.command {
        Command::Score { reference, candidate, output, linear } => gumps(&reference, &candidate, &output, linear),
        Command::SamePixels { reference, candidate } => same_pixels(&reference, &candidate),
        Command::Sizes { package } => sizes(&package),
    }
}
