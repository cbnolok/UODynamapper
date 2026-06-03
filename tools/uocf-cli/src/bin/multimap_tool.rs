use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, WrapErr};
use uocf::classic::multimap_rle;

/// Classic Client multimap.rle converter.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Decode multimap.rle to a BMP or PNG image.
    RleToImage {
        /// Input multimap.rle file.
        #[arg(long)]
        input: PathBuf,
        /// Output .bmp or .png file.
        #[arg(long)]
        output: PathBuf,
    },
    /// Encode a BMP or PNG image to multimap.rle.
    ImageToRle {
        /// Input .bmp or .png file.
        #[arg(long)]
        input: PathBuf,
        /// Output multimap.rle file.
        #[arg(long)]
        output: PathBuf,
    },
    /// Convert a DDS world/facet render to treasure-map style multimap.rle.
    DdsToRle {
        /// Input .dds file.
        #[arg(long)]
        input: PathBuf,
        /// Output multimap.rle file.
        #[arg(long)]
        output: PathBuf,
        /// Source crop X coordinate.
        #[arg(long, default_value_t = 0)]
        source_x: u32,
        /// Source crop Y coordinate.
        #[arg(long, default_value_t = 0)]
        source_y: u32,
        /// Source crop width. Defaults to output-width * 2, clamped to the DDS.
        #[arg(long)]
        source_width: Option<u32>,
        /// Source crop height. Defaults to output-height * 2, clamped to the DDS.
        #[arg(long)]
        source_height: Option<u32>,
        /// Output multimap width.
        #[arg(long, default_value_t = multimap_rle::DEFAULT_WIDTH)]
        output_width: u32,
        /// Output multimap height.
        #[arg(long, default_value_t = multimap_rle::DEFAULT_HEIGHT)]
        output_height: u32,
        /// Minimum edge strength that becomes black ink.
        #[arg(long, default_value_t = 28)]
        edge_threshold: u16,
        /// Extra ink dilation radius in output pixels.
        #[arg(long, default_value_t = 0)]
        line_radius: u32,
    },
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let _ = udd_logging::install_paris_logger();

    match Cli::parse().command {
        Commands::RleToImage { input, output } => {
            let image = multimap_rle::load_rle(&input)?;
            multimap_rle::save_bitmap_or_png(&output, &image)?;
            println!(
                "Decoded {}x{} multimap RLE '{}' to '{}'.",
                image.width,
                image.height,
                input.display(),
                output.display()
            );
        }
        Commands::ImageToRle { input, output } => {
            let image = multimap_rle::load_bitmap_or_png(&input)?;
            multimap_rle::save_rle(&output, &image)?;
            println!(
                "Encoded {}x{} multimap image '{}' to '{}'.",
                image.width,
                image.height,
                input.display(),
                output.display()
            );
        }
        Commands::DdsToRle {
            input,
            output,
            source_x,
            source_y,
            source_width,
            source_height,
            output_width,
            output_height,
            edge_threshold,
            line_radius,
        } => {
            let decoded = load_dds_rgba(&input)?;
            let crop = multimap_default_rect(
                decoded.width,
                decoded.height,
                source_x,
                source_y,
                source_width,
                source_height,
                output_width,
                output_height,
            )?;
            let samples = resample_rgb(&decoded, crop, output_width, output_height)?;
            let pixels = sketch_multimap_pixels(
                &samples,
                output_width,
                output_height,
                edge_threshold,
                line_radius,
            )?;
            let image = multimap_rle::MultimapRleImage::new(output_width, output_height, pixels)?;
            multimap_rle::save_rle(&output, &image)?;
            println!(
                "Converted DDS '{}' crop {}x{}+{},{} to {}x{} multimap RLE '{}'.",
                input.display(),
                crop.width,
                crop.height,
                crop.x,
                crop.y,
                output_width,
                output_height,
                output.display()
            );
        }
    }

    Ok(())
}

struct DecodedRgba {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct SourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn load_dds_rgba(path: &Path) -> eyre::Result<DecodedRgba> {
    let bytes = fs::read(path).wrap_err_with(|| format!("Failed to read DDS {}", path.display()))?;
    let mut decoder = dds::Decoder::new(Cursor::new(bytes)).wrap_err("Failed reading DDS header")?;
    let size = decoder.main_size();
    let rgba_len = dds::ColorFormat::RGBA_U8
        .buffer_size(size)
        .ok_or_else(|| eyre::eyre!("DDS image is too large to decode"))?;
    let mut rgba = vec![0u8; rgba_len];
    let image = dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8)
        .ok_or_else(|| eyre::eyre!("Could not create DDS decode buffer"))?;
    decoder.read_surface(image).wrap_err("Failed decoding DDS image")?;
    Ok(DecodedRgba {
        width: size.width,
        height: size.height,
        rgba,
    })
}

fn multimap_default_rect(
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
    width: Option<u32>,
    height: Option<u32>,
    output_width: u32,
    output_height: u32,
) -> eyre::Result<SourceRect> {
    let remaining_width = image_width
        .checked_sub(x)
        .ok_or_else(|| eyre::eyre!("source-x {} is outside DDS width {}", x, image_width))?;
    let remaining_height = image_height
        .checked_sub(y)
        .ok_or_else(|| eyre::eyre!("source-y {} is outside DDS height {}", y, image_height))?;
    let default_width = output_width.saturating_mul(2).min(remaining_width);
    let default_height = output_height.saturating_mul(2).min(remaining_height);
    checked_rect(
        image_width,
        image_height,
        x,
        y,
        width.unwrap_or(default_width),
        height.unwrap_or(default_height),
    )
}

fn checked_rect(
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> eyre::Result<SourceRect> {
    if width == 0 || height == 0 {
        eyre::bail!("source crop dimensions must be non-zero, got {}x{}", width, height);
    }
    let end_x = x
        .checked_add(width)
        .ok_or_else(|| eyre::eyre!("source crop X range overflows"))?;
    let end_y = y
        .checked_add(height)
        .ok_or_else(|| eyre::eyre!("source crop Y range overflows"))?;
    if end_x > image_width || end_y > image_height {
        eyre::bail!(
            "source crop {}x{}+{},{} exceeds DDS dimensions {}x{}",
            width,
            height,
            x,
            y,
            image_width,
            image_height
        );
    }
    Ok(SourceRect {
        x,
        y,
        width,
        height,
    })
}

fn resample_rgb(
    image: &DecodedRgba,
    rect: SourceRect,
    output_width: u32,
    output_height: u32,
) -> eyre::Result<Vec<[u8; 3]>> {
    let output_pixels = output_len(output_width, output_height)?;
    let mut output = Vec::with_capacity(output_pixels);
    for y in 0..output_height {
        let (source_y0, source_y1) = source_span(rect.y, rect.height, y, output_height);
        for x in 0..output_width {
            let (source_x0, source_x1) = source_span(rect.x, rect.width, x, output_width);
            let [r, g, b, _] = average_rgba(image, source_x0, source_y0, source_x1, source_y1);
            output.push([r, g, b]);
        }
    }
    Ok(output)
}

fn source_span(offset: u32, length: u32, output: u32, output_length: u32) -> (u32, u32) {
    let start = offset + ((u64::from(output) * u64::from(length)) / u64::from(output_length)) as u32;
    let end = offset
        + (((u64::from(output + 1) * u64::from(length)) + u64::from(output_length - 1))
            / u64::from(output_length)) as u32;
    (start, end.max(start + 1))
}

fn average_rgba(image: &DecodedRgba, x0: u32, y0: u32, x1: u32, y1: u32) -> [u8; 4] {
    let mut r = 0u64;
    let mut g = 0u64;
    let mut b = 0u64;
    let mut a = 0u64;
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let index = ((y * image.width + x) as usize) * 4;
            r += u64::from(image.rgba[index]);
            g += u64::from(image.rgba[index + 1]);
            b += u64::from(image.rgba[index + 2]);
            a += u64::from(image.rgba[index + 3]);
            count += 1;
        }
    }
    [
        (r / count) as u8,
        (g / count) as u8,
        (b / count) as u8,
        (a / count) as u8,
    ]
}

fn sketch_multimap_pixels(
    samples: &[[u8; 3]],
    width: u32,
    height: u32,
    edge_threshold: u16,
    line_radius: u32,
) -> eyre::Result<Vec<u8>> {
    let pixel_count = output_len(width, height)?;
    if samples.len() != pixel_count {
        eyre::bail!(
            "sample buffer has {} pixels, expected {} for {}x{}",
            samples.len(),
            pixel_count,
            width,
            height
        );
    }

    let mut edges = vec![false; pixel_count];
    for y in 0..height {
        for x in 0..width {
            let edge = edge_strength(samples, width, height, x, y);
            edges[(y * width + x) as usize] = edge >= edge_threshold;
        }
    }

    let mut pixels = vec![multimap_rle::WHITE_PIXEL; pixel_count];
    for y in 0..height {
        for x in 0..width {
            if !edges[(y * width + x) as usize] {
                continue;
            }
            let min_x = x.saturating_sub(line_radius);
            let max_x = (x + line_radius).min(width - 1);
            let min_y = y.saturating_sub(line_radius);
            let max_y = (y + line_radius).min(height - 1);
            for ink_y in min_y..=max_y {
                for ink_x in min_x..=max_x {
                    pixels[(ink_y * width + ink_x) as usize] = multimap_rle::BLACK_PIXEL;
                }
            }
        }
    }
    Ok(pixels)
}

fn edge_strength(samples: &[[u8; 3]], width: u32, height: u32, x: u32, y: u32) -> u16 {
    let left = x.saturating_sub(1);
    let right = (x + 1).min(width - 1);
    let top = y.saturating_sub(1);
    let bottom = (y + 1).min(height - 1);

    let tl = luma(samples[(top * width + left) as usize]);
    let tc = luma(samples[(top * width + x) as usize]);
    let tr = luma(samples[(top * width + right) as usize]);
    let ml = luma(samples[(y * width + left) as usize]);
    let mr = luma(samples[(y * width + right) as usize]);
    let bl = luma(samples[(bottom * width + left) as usize]);
    let bc = luma(samples[(bottom * width + x) as usize]);
    let br = luma(samples[(bottom * width + right) as usize]);

    let gx = -tl + tr - 2 * ml + 2 * mr - bl + br;
    let gy = -tl - 2 * tc - tr + bl + 2 * bc + br;
    let luma_edge = ((gx.abs() + gy.abs()) / 8) as u16;
    luma_edge.max(color_edge(samples, width, height, x, y))
}

fn color_edge(samples: &[[u8; 3]], width: u32, height: u32, x: u32, y: u32) -> u16 {
    let here = samples[(y * width + x) as usize];
    let mut strongest = 0u16;
    if x + 1 < width {
        strongest = strongest.max(color_distance(here, samples[(y * width + x + 1) as usize]));
    }
    if y + 1 < height {
        strongest = strongest.max(color_distance(here, samples[((y + 1) * width + x) as usize]));
    }
    strongest
}

fn color_distance(a: [u8; 3], b: [u8; 3]) -> u16 {
    let dr = i16::from(a[0]) - i16::from(b[0]);
    let dg = i16::from(a[1]) - i16::from(b[1]);
    let db = i16::from(a[2]) - i16::from(b[2]);
    ((dr.abs() + dg.abs() + db.abs()) / 3) as u16
}

fn luma(rgb: [u8; 3]) -> i32 {
    (i32::from(rgb[0]) * 299 + i32::from(rgb[1]) * 587 + i32::from(rgb[2]) * 114) / 1000
}

fn output_len(width: u32, height: u32) -> eyre::Result<usize> {
    if width == 0 || height == 0 {
        eyre::bail!("output dimensions must be non-zero, got {}x{}", width, height);
    }
    width
        .checked_mul(height)
        .and_then(|pixels| usize::try_from(pixels).ok())
        .ok_or_else(|| eyre::eyre!("output dimensions are too large: {}x{}", width, height))
}
