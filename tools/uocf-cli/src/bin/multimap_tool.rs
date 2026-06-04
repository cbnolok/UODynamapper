use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use image::{ColorType, ImageFormat};
use uocf::classic::multimap_render::{
    render_multimap, render_source_image, DecodedRgba, ImageRenderOptions, MultimapRenderOptions,
    MultimapStyle,
};
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
    /// Decode a DDS image to a lossless BMP or PNG image.
    DdsToImage {
        /// Input .dds file.
        #[arg(long)]
        input: PathBuf,
        /// Output .bmp or .png file.
        #[arg(long)]
        output: PathBuf,
        /// Source crop X coordinate.
        #[arg(long, default_value_t = 0)]
        source_x: u32,
        /// Source crop Y coordinate.
        #[arg(long, default_value_t = 0)]
        source_y: u32,
        /// Source crop width. Defaults to the remaining source width.
        #[arg(long)]
        source_width: Option<u32>,
        /// Source crop height. Defaults to the remaining source height.
        #[arg(long)]
        source_height: Option<u32>,
        /// Output image width. Defaults to the source crop width.
        #[arg(long)]
        output_width: Option<u32>,
        /// Output image height. Defaults to the source crop height.
        #[arg(long)]
        output_height: Option<u32>,
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
        /// Rendering style used to turn the aerial image into monochrome ink.
        #[arg(long, value_enum, default_value_t = CliMultimapStyle::Classic)]
        style: CliMultimapStyle,
    },
    /// Decode a BC7+Zstd KTX2 image to a lossless BMP or PNG image.
    Ktx2ToImage {
        /// Input .ktx2 file.
        #[arg(long)]
        input: PathBuf,
        /// Output .bmp or .png file.
        #[arg(long)]
        output: PathBuf,
        /// Source crop X coordinate.
        #[arg(long, default_value_t = 0)]
        source_x: u32,
        /// Source crop Y coordinate.
        #[arg(long, default_value_t = 0)]
        source_y: u32,
        /// Source crop width. Defaults to the remaining source width.
        #[arg(long)]
        source_width: Option<u32>,
        /// Source crop height. Defaults to the remaining source height.
        #[arg(long)]
        source_height: Option<u32>,
        /// Output image width. Defaults to the source crop width.
        #[arg(long)]
        output_width: Option<u32>,
        /// Output image height. Defaults to the source crop height.
        #[arg(long)]
        output_height: Option<u32>,
    },
    /// Convert a BC7+Zstd KTX2 world/facet render to treasure-map style multimap.rle.
    Ktx2ToRle {
        /// Input .ktx2 file.
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
        /// Source crop width. Defaults to output-width * 2, clamped to the source.
        #[arg(long)]
        source_width: Option<u32>,
        /// Source crop height. Defaults to output-height * 2, clamped to the source.
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
        /// Rendering style used to turn the aerial image into monochrome ink.
        #[arg(long, value_enum, default_value_t = CliMultimapStyle::Classic)]
        style: CliMultimapStyle,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliMultimapStyle {
    /// First-pass edge extraction from the aerial render.
    Edge,
    /// Symbolic hand-drawn style closer to Classic Client multimap.rle.
    Classic,
}

impl From<CliMultimapStyle> for MultimapStyle {
    fn from(value: CliMultimapStyle) -> Self {
        match value {
            CliMultimapStyle::Edge => Self::Edge,
            CliMultimapStyle::Classic => Self::Classic,
        }
    }
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
        Commands::DdsToImage {
            input,
            output,
            source_x,
            source_y,
            source_width,
            source_height,
            output_width,
            output_height,
        } => {
            let decoded = load_dds_rgba(&input)?;
            convert_source_to_image(
                "DDS",
                &input,
                &output,
                decoded,
                source_x,
                source_y,
                source_width,
                source_height,
                output_width,
                output_height,
            )?;
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
            style,
        } => {
            let decoded = load_dds_rgba(&input)?;
            convert_source_to_rle(
                "DDS",
                &input,
                &output,
                decoded,
                source_x,
                source_y,
                source_width,
                source_height,
                output_width,
                output_height,
                edge_threshold,
                line_radius,
                style,
            )?;
        }
        Commands::Ktx2ToImage {
            input,
            output,
            source_x,
            source_y,
            source_width,
            source_height,
            output_width,
            output_height,
        } => {
            let decoded = load_ktx2_rgba(&input)?;
            convert_source_to_image(
                "KTX2",
                &input,
                &output,
                decoded,
                source_x,
                source_y,
                source_width,
                source_height,
                output_width,
                output_height,
            )?;
        }
        Commands::Ktx2ToRle {
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
            style,
        } => {
            let decoded = load_ktx2_rgba(&input)?;
            convert_source_to_rle(
                "KTX2",
                &input,
                &output,
                decoded,
                source_x,
                source_y,
                source_width,
                source_height,
                output_width,
                output_height,
                edge_threshold,
                line_radius,
                style,
            )?;
        }
    }

    Ok(())
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
    DecodedRgba::new(size.width, size.height, rgba)
}

fn load_ktx2_rgba(path: &Path) -> eyre::Result<DecodedRgba> {
    let (width, height, rgba) = udd_image_codecs::ktx2::decode_ktx2_bc7_zstd_to_rgba8888(path)?;
    DecodedRgba::new(width, height, rgba)
}

fn convert_source_to_image(
    label: &str,
    input: &Path,
    output: &Path,
    decoded: DecodedRgba,
    source_x: u32,
    source_y: u32,
    source_width: Option<u32>,
    source_height: Option<u32>,
    output_width: Option<u32>,
    output_height: Option<u32>,
) -> eyre::Result<()> {
    let rendered = render_source_image(
        &decoded,
        ImageRenderOptions {
            source_x,
            source_y,
            source_width,
            source_height,
            output_width,
            output_height,
        },
    )?;
    save_rgba_image(output, rendered.width, rendered.height, &rendered.rgba)?;
    println!(
        "Decoded {} '{}' crop {}x{}+{},{} to {}x{} image '{}'.",
        label,
        input.display(),
        rendered.crop.width,
        rendered.crop.height,
        rendered.crop.x,
        rendered.crop.y,
        rendered.width,
        rendered.height,
        output.display()
    );
    Ok(())
}

fn convert_source_to_rle(
    label: &str,
    input: &Path,
    output: &Path,
    decoded: DecodedRgba,
    source_x: u32,
    source_y: u32,
    source_width: Option<u32>,
    source_height: Option<u32>,
    output_width: u32,
    output_height: u32,
    edge_threshold: u16,
    line_radius: u32,
    style: CliMultimapStyle,
) -> eyre::Result<()> {
    let rendered = render_multimap(
        &decoded,
        MultimapRenderOptions {
            source_x,
            source_y,
            source_width,
            source_height,
            output_width,
            output_height,
            edge_threshold,
            line_radius,
            style: style.into(),
        },
    )?;
    multimap_rle::save_rle(output, &rendered.image)?;
    println!(
        "Converted {} '{}' crop {}x{}+{},{} to {}x{} {:?} multimap RLE '{}'.",
        label,
        input.display(),
        rendered.crop.width,
        rendered.crop.height,
        rendered.crop.x,
        rendered.crop.y,
        output_width,
        output_height,
        style,
        output.display()
    );
    Ok(())
}

fn save_rgba_image(path: &Path, width: u32, height: u32, rgba: &[u8]) -> eyre::Result<()> {
    let format = output_format(path)?;
    image::save_buffer_with_format(path, rgba, width, height, ColorType::Rgba8, format)
        .wrap_err_with(|| format!("Failed to write image {}", path.display()))
}

fn output_format(path: &Path) -> eyre::Result<ImageFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("bmp") => Ok(ImageFormat::Bmp),
        Some("png") => Ok(ImageFormat::Png),
        _ => eyre::bail!("output image extension must be .bmp or .png: {}", path.display()),
    }
}
