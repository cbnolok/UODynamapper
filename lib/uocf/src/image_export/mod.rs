//! Images written as BMP, PNG or WebP ([`ExportImage::encode`]), each in its source's own colour format where the
//! file format has one, and image files read back for the Classic encoders ([`classic_import_rgba`]).
//!
//! | Source | BMP | PNG | WebP |
//! |---|---|---|---|
//! | Classic art, gumps, land tiles, texmaps ([`ExportImage::Classic16`]) | 16-bit X1R5G5B5, the stored words bit for bit ([`bmp`]); colour 0 stays 0 | RGBA, colour 0 transparent (a land tile: outside its diamond; a texmap: never), channels widened by [`widen_channel`] | lossless, the PNG's pixels |
//! | Classic animation frames ([`ExportImage::Paletted`]) | 8-bit with the frame's palette | 8-bit paletted, index 0 transparent | lossless RGBA: the `image` crate's WebP encoder has no palette mode |
//! | Enhanced images, decoded textures ([`ExportImage::Rgba`]) | 32-bit with an alpha mask (`BITMAPV4HEADER`) | RGBA as stored | lossless RGBA |
//!
//! The widening `(c << 3) | (c >> 2)` gives 0 and 255 for the darkest and brightest 5-bit values, and is reversible:
//! [`classic_import_rgba`] takes a channel it made back to `c << 3`, the value the Classic encoders read as `c`
//! without rounding, and reads a 16-bit BMP word for word, so an exported Classic image imports to the same colours.

pub mod bmp;

use std::path::Path;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use crate::classic::anim_frames::{ClassicFrame, ClassicPalette};
use crate::classic::art::{LAND_DIMENSION, RGBA_BYTES_PER_PIXEL};
use crate::classic::art_write::{land_rows, ArtColors};
use crate::classic::gump_write::{GumpColors, OPAQUE_BLACK};
use crate::classic::texmap_write::TexmapRecord;
use crate::utils::output_file::write_through_temp;

crate::eyre_imports!();

const CHANNEL_BITS: u32 = 5;
const CHANNEL_MASK: u16 = (1 << CHANNEL_BITS) - 1;
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
const GREEN_SHIFT: u32 = CHANNEL_BITS;
/// Bit 15 of a Classic colour, above its three channels: no colour of its own.
const FLAG_BIT: u16 = 1 << (3 * CHANNEL_BITS);
/// A 5-bit channel moves up by this many bits to 8 bits...
const WIDEN_SHIFT: u32 = u8::BITS - CHANNEL_BITS;
/// ...and its top bits repeat in the low ones.
const REPEAT_SHIFT: u32 = CHANNEL_BITS - WIDEN_SHIFT;
/// The low bits of an 8-bit channel a shift alone leaves 0.
const WIDENED_LOW_BITS: u8 = (1 << WIDEN_SHIFT) - 1;
const OPAQUE_ALPHA: u8 = u8::MAX;
const ALPHA_CHANNEL: usize = 3;
const RGB_CHANNELS: usize = 3;
const CLEAR_PIXEL: [u8; RGBA_BYTES_PER_PIXEL] = [0; RGBA_BYTES_PER_PIXEL];
/// The palette index of a pixel no run draws: transparent in the exported file. No shipped frame draws it.
pub const TRANSPARENT_INDEX: u8 = 0;
/// The PNG `tRNS` chunk of a frame: index 0 transparent, the indices past it opaque.
const FRAME_PNG_TRANSPARENCY: [u8; 1] = [0];
/// Hex digits of an id in a file name (`art_0x0E75`).
const ID_HEX_DIGITS: usize = 4;
/// Digits of a frame number in a file name (`anim_400_a0_d3_f01`).
const FRAME_NUMBER_DIGITS: usize = 2;

/// An 8-bit channel of a 5-bit one: `(c << 3) | (c >> 2)`, 0 to 0 and 31 to 255.
pub fn widen_channel(channel: u16) -> u8 {
    let channel = channel & CHANNEL_MASK;
    ((channel << WIDEN_SHIFT) | (channel >> REPEAT_SHIFT)) as u8
}

/// The 8-bit channels of a 15-bit colour, by `widen` (bit 15 ignored).
fn rgb_of(color: u16, widen: fn(u16) -> u8) -> [u8; RGB_CHANNELS] {
    [widen(color >> RED_SHIFT), widen(color >> GREEN_SHIFT), widen(color)]
}

/// A 5-bit channel shifted up alone, as the Classic decoders widen it.
fn shift_channel(channel: u16) -> u8 {
    ((channel & CHANNEL_MASK) << WIDEN_SHIFT) as u8
}

fn opaque(rgb: [u8; RGB_CHANNELS]) -> [u8; RGBA_BYTES_PER_PIXEL] {
    [rgb[0], rgb[1], rgb[2], OPAQUE_ALPHA]
}

/// The file format of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageFileFormat {
    Bmp,
    #[default]
    Png,
    WebP,
}

impl ImageFileFormat {
    pub const ALL: [Self; 3] = [Self::Bmp, Self::Png, Self::WebP];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Bmp => "bmp",
            Self::Png => "png",
            Self::WebP => "webp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bmp => "BMP",
            Self::Png => "PNG",
            Self::WebP => "WebP",
        }
    }
}

/// Which pixels of colour 0 a Classic 15-bit image draws (black).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicZero {
    /// None: art statics and gumps, where colour 0 is transparent.
    Transparent,
    /// Those inside the diamond of a 44x44 land tile; the corners are transparent.
    LandDiamond,
    /// All: texmaps are opaque.
    Opaque,
}

/// An image to export, in its source's colour format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportImage {
    /// Classic 15-bit colours row by row (bit 15 kept).
    Classic16 { width: u32, height: u32, colors: Vec<u16>, zero: ClassicZero },
    /// A Classic animation frame: indices into its 256 15-bit colours, [`TRANSPARENT_INDEX`] for no pixel.
    Paletted { width: u32, height: u32, palette: Box<ClassicPalette>, indices: Vec<u8> },
    /// Straight-alpha RGBA.
    Rgba { width: u32, height: u32, rgba: Vec<u8> },
}

/// Whether each pixel of a 44x44 land tile is inside its diamond.
fn land_diamond_mask() -> Vec<bool> {
    let mut inside = vec![false; LAND_DIMENSION * LAND_DIMENSION];
    for (row_x, y, row_width) in land_rows() {
        inside[y * LAND_DIMENSION + row_x..][..row_width].fill(true);
    }
    inside
}

/// The RGBA of 15-bit `colors` drawn by the rule `zero`, channels widened by `widen`.
fn classic_rgba(width: u32, height: u32, colors: &[u16], zero: ClassicZero, widen: fn(u16) -> u8) -> Vec<u8> {
    let land = zero == ClassicZero::LandDiamond && (width, height) == (LAND_DIMENSION as u32, LAND_DIMENSION as u32);
    let inside = land.then(land_diamond_mask);
    colors
        .iter()
        .enumerate()
        .flat_map(|(index, &color)| {
            let drawn = color != 0
                || match zero {
                    ClassicZero::Transparent => false,
                    ClassicZero::Opaque => true,
                    ClassicZero::LandDiamond => inside.as_ref().is_some_and(|inside| inside[index]),
                };
            if drawn { opaque(rgb_of(color, widen)) } else { CLEAR_PIXEL }
        })
        .collect()
}

impl ExportImage {
    /// Classic art as stored: a land tile on its 44x44 square, a static with colour 0 transparent.
    pub fn art(art: &ArtColors) -> Self {
        match art {
            ArtColors::Land(colors) => {
                let mut grid = vec![0; LAND_DIMENSION * LAND_DIMENSION];
                let positions = land_rows().flat_map(|(row_x, y, row_width)| (row_x..row_x + row_width).map(move |x| y * LAND_DIMENSION + x));
                for (index, &color) in positions.zip(colors) {
                    grid[index] = color;
                }
                let side = LAND_DIMENSION as u32;
                Self::Classic16 { width: side, height: side, colors: grid, zero: ClassicZero::LandDiamond }
            }
            ArtColors::Static { width, height, colors } => {
                Self::Classic16 { width: u32::from(*width), height: u32::from(*height), colors: colors.clone(), zero: ClassicZero::Transparent }
            }
        }
    }

    pub fn gump(gump: &GumpColors) -> Self {
        Self::Classic16 { width: u32::from(gump.width), height: u32::from(gump.height), colors: gump.colors.clone(), zero: ClassicZero::Transparent }
    }

    pub fn texmap(texmap: &TexmapRecord) -> Self {
        let (width, height) = texmap.size.dimensions();
        Self::Classic16 { width, height, colors: texmap.colors.clone(), zero: ClassicZero::Opaque }
    }

    /// A Classic animation frame with its palette; RGBA when a run draws [`TRANSPARENT_INDEX`] (no shipped frame
    /// does), which the palette formats would make transparent.
    pub fn classic_frame(frame: &ClassicFrame, palette: &ClassicPalette) -> Self {
        let (width, height) = (u32::from(frame.width), u32::from(frame.height));
        if frame.used_indices().any(|index| index == TRANSPARENT_INDEX) {
            let rgba = frame
                .pixels
                .iter()
                .flat_map(|pixel| pixel.map_or(CLEAR_PIXEL, |index| opaque(rgb_of(palette[usize::from(index)], widen_channel))))
                .collect();
            return Self::Rgba { width, height, rgba };
        }
        let indices = frame.pixels.iter().map(|pixel| pixel.unwrap_or(TRANSPARENT_INDEX)).collect();
        Self::Paletted { width, height, palette: Box::new(*palette), indices }
    }

    /// A decoded Classic animation frame (straight-alpha RGBA, channels widened from `palette`'s colours) back on
    /// `palette`: each drawn pixel takes the first index past [`TRANSPARENT_INDEX`] of its colour, the same colour as
    /// stored when the palette repeats one. RGBA when a drawn colour has no such index.
    pub fn classic_frame_rgba(width: u32, height: u32, rgba: &[u8], palette: &ClassicPalette) -> Self {
        let colour_of = |pixel: &[u8]| pixel[..RGB_CHANNELS].iter().fold(0u16, |colour, &channel| colour << CHANNEL_BITS | u16::from(channel >> WIDEN_SHIFT));
        let mut index_of = std::collections::HashMap::new();
        for (index, &colour) in palette.iter().enumerate().skip(usize::from(TRANSPARENT_INDEX) + 1) {
            index_of.entry(colour & !FLAG_BIT).or_insert(index as u8);
        }
        let indices: Option<Vec<u8>> = rgba
            .chunks_exact(RGBA_BYTES_PER_PIXEL)
            .map(|pixel| if pixel[ALPHA_CHANNEL] == 0 { Some(TRANSPARENT_INDEX) } else { index_of.get(&colour_of(pixel)).copied() })
            .collect();
        match indices {
            Some(indices) => Self::Paletted { width, height, palette: Box::new(*palette), indices },
            None => Self::Rgba { width, height, rgba: rgba.to_vec() },
        }
    }

    pub fn width(&self) -> u32 {
        match self {
            Self::Classic16 { width, .. } | Self::Paletted { width, .. } | Self::Rgba { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Classic16 { height, .. } | Self::Paletted { height, .. } | Self::Rgba { height, .. } => *height,
        }
    }

    /// Recolours a Classic image by `hue` (a Classic hue, 15-bit colour to 15-bit colour): every drawn colour but
    /// black, or every palette colour. A colour hued to 0 where 0 is transparent becomes [`OPAQUE_BLACK`]. An
    /// RGBA image is left as it is.
    pub fn hue_classic_colors(&mut self, hue: impl Fn(u16) -> u16) {
        match self {
            Self::Classic16 { colors, zero, .. } => {
                for color in colors.iter_mut().filter(|color| **color != 0) {
                    *color = match hue(*color) {
                        0 if *zero == ClassicZero::Transparent => OPAQUE_BLACK,
                        hued => hued,
                    };
                }
            }
            Self::Paletted { palette, .. } => {
                for color in palette.iter_mut() {
                    *color = hue(*color);
                }
            }
            Self::Rgba { .. } => {}
        }
    }

    /// The straight-alpha RGBA of the PNG and WebP files.
    pub fn to_rgba(&self) -> Vec<u8> {
        match self {
            Self::Classic16 { width, height, colors, zero } => classic_rgba(*width, *height, colors, *zero, widen_channel),
            Self::Paletted { palette, indices, .. } => indices
                .iter()
                .flat_map(|&index| {
                    if index == TRANSPARENT_INDEX { CLEAR_PIXEL } else { opaque(rgb_of(palette[usize::from(index)], widen_channel)) }
                })
                .collect(),
            Self::Rgba { rgba, .. } => rgba.clone(),
        }
    }

    fn check_size(&self) -> eyre::Result<()> {
        let (width, height) = (self.width(), self.height());
        eyre::ensure!(width > 0 && height > 0, "a {width}x{height} image has no pixels");
        let pixels = width as usize * height as usize;
        let (found, expected) = match self {
            Self::Classic16 { colors, .. } => (colors.len(), pixels),
            Self::Paletted { indices, .. } => (indices.len(), pixels),
            Self::Rgba { rgba, .. } => (rgba.len(), pixels * RGBA_BYTES_PER_PIXEL),
        };
        eyre::ensure!(found == expected, "{found} values for a {width}x{height} image, not {expected}");
        Ok(())
    }

    /// The file bytes of the image in `format` (see the module docs).
    pub fn encode(&self, format: ImageFileFormat) -> eyre::Result<Vec<u8>> {
        self.check_size()?;
        let (width, height) = (self.width(), self.height());
        let mut out = Vec::new();
        match (format, self) {
            (ImageFileFormat::Bmp, Self::Classic16 { colors, .. }) => return bmp::encode_rgb555(width, height, colors),
            (ImageFileFormat::Bmp, Self::Paletted { palette, indices, .. }) => {
                let colors: Vec<[u8; RGB_CHANNELS]> = palette.iter().map(|&color| rgb_of(color, widen_channel)).collect();
                image::codecs::bmp::BmpEncoder::new(&mut out).encode_with_palette(indices, width, height, ExtendedColorType::L8, Some(&colors))?;
            }
            (ImageFileFormat::Bmp, Self::Rgba { rgba, .. }) => {
                image::codecs::bmp::BmpEncoder::new(&mut out).encode(rgba, width, height, ExtendedColorType::Rgba8)?;
            }
            (ImageFileFormat::Png, Self::Paletted { palette, indices, .. }) => {
                let colors: Vec<u8> = palette.iter().flat_map(|&color| rgb_of(color, widen_channel)).collect();
                let mut encoder = png::Encoder::new(&mut out, width, height);
                encoder.set_color(png::ColorType::Indexed);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_palette(colors);
                encoder.set_trns(FRAME_PNG_TRANSPARENCY.to_vec());
                let mut writer = encoder.write_header()?;
                writer.write_image_data(indices)?;
                writer.finish()?;
            }
            (ImageFileFormat::Png, _) => {
                image::codecs::png::PngEncoder::new(&mut out).write_image(&self.to_rgba(), width, height, ExtendedColorType::Rgba8)?;
            }
            (ImageFileFormat::WebP, _) => {
                image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(&self.to_rgba(), width, height, ExtendedColorType::Rgba8)?;
            }
        }
        Ok(out)
    }

    /// Writes the image to `path` in `format`, through a temporary file in its folder.
    pub fn write(&self, path: &Path, format: ImageFileFormat) -> eyre::Result<()> {
        let bytes = self.encode(format)?;
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }
}

/// An image file read for the Classic encoders (`ArtColors::from_rgba`, `GumpColors::from_rgba`). A 16-bit BMP is
/// read word for word: colour 0 transparent (black inside the diamond of a 44x44 `land_tile`), every other colour
/// opaque with its channels shifted up as the decoders widen them. Any other image goes through the `image` crate,
/// and a channel [`widen_channel`] makes is taken back to its shift alone. Either way an exported Classic image
/// imports to the colours it was exported from, without rounding.
pub fn classic_import_rgba(bytes: &[u8], land_tile: bool) -> eyre::Result<RgbaImage> {
    if let Some((width, height, words)) = bmp::decode_rgb555(bytes)? {
        let zero = if land_tile { ClassicZero::LandDiamond } else { ClassicZero::Transparent };
        let rgba = classic_rgba(width, height, &words, zero, shift_channel);
        return RgbaImage::from_raw(width, height, rgba).ok_or_else(|| eyre!("{width}x{height} pixels do not fit an image"));
    }
    let mut image = image::load_from_memory(bytes).wrap_err("the file does not decode as an image")?.into_rgba8();
    for pixel in image.pixels_mut() {
        for channel in &mut pixel.0[..ALPHA_CHANNEL] {
            if widen_channel(u16::from(*channel >> WIDEN_SHIFT)) == *channel {
                *channel &= !WIDENED_LOW_BITS;
            }
        }
    }
    Ok(image)
}

/// `art_0x0E75`: the file stem of `id` of the kind `prefix` names.
pub fn id_file_stem(prefix: &str, id: u32) -> String {
    format!("{prefix}_0x{id:0digits$X}", digits = ID_HEX_DIGITS)
}

/// `anim_400_a0_d3_f01`: the file stem of a frame of `body`, `action` and `direction`, numbered from 1.
pub fn animation_frame_stem(body: u32, action: u16, direction: u8, frame_number: usize) -> String {
    format!("anim_{body}_a{action}_d{direction}_f{frame_number:0digits$}", digits = FRAME_NUMBER_DIGITS)
}

/// `value` with only ASCII letters, digits, `-` and `_` (whitespace becomes `_`); `fallback` when nothing is left.
pub fn sanitize_file_stem(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::art::LAND_DIAMOND_PIXEL_COUNT;
    use crate::classic::art_write::ArtKind;
    use crate::classic::gump_write::RgbaConversion;
    use crate::classic::land_texture::LandTextureSize;

    /// An odd width, so BMP rows are padded.
    const WIDTH: u16 = 37;
    const HEIGHT: u16 = 23;
    /// Every this many pixels is colour 0.
    const ZERO_EVERY: usize = 7;
    const PALETTE_WIDTH: u16 = 9;
    const PALETTE_HEIGHT: u16 = 5;
    /// Every this many frame pixels is not drawn.
    const UNDRAWN_EVERY: usize = 4;
    const BMP_PIXELS_OFFSET_AT: usize = 10;
    const BMP_INFO_SIZE_AT: usize = 14;
    const BMP_ROW_ALIGNMENT: usize = 4;
    const BGRA: usize = 4;

    /// 15-bit colours with every 5-bit value in each channel.
    fn colour(index: usize) -> u16 {
        let channel = |value: usize| (value % (CHANNEL_MASK as usize + 1)) as u16;
        channel(index) << RED_SHIFT | channel(index / 3 + 11) << GREEN_SHIFT | channel(index * 7 + 5)
    }

    fn colours(count: usize) -> Vec<u16> {
        (0..count).map(|index| if index % ZERO_EVERY == 0 { 0 } else { colour(index) }).collect()
    }

    fn frame() -> (ClassicFrame, ClassicPalette) {
        let mut palette = [0; crate::classic::anim_frames::PALETTE_COLOURS];
        for (index, color) in palette.iter_mut().enumerate().skip(1) {
            *color = colour(index);
        }
        let count = usize::from(PALETTE_WIDTH) * usize::from(PALETTE_HEIGHT);
        let pixels = (0..count).map(|index| (index % UNDRAWN_EVERY != 0).then_some((index * 29 % 255 + 1) as u8)).collect();
        (ClassicFrame { center_x: 0, center_y: 0, width: PALETTE_WIDTH, height: PALETTE_HEIGHT, pixels }, palette)
    }

    fn import(image: &ExportImage, format: ImageFileFormat, land_tile: bool) -> RgbaImage {
        classic_import_rgba(&image.encode(format).unwrap(), land_tile).unwrap()
    }

    #[test]
    fn classic_art_and_gumps_import_back_to_their_colours() {
        let pixels = usize::from(WIDTH) * usize::from(HEIGHT);
        let statics = ArtColors::Static { width: WIDTH, height: HEIGHT, colors: colours(pixels) };
        let land = ArtColors::Land(colours(LAND_DIAMOND_PIXEL_COUNT));
        let gump = GumpColors { width: WIDTH, height: HEIGHT, colors: colours(pixels) };
        for format in ImageFileFormat::ALL {
            let rgba = import(&ExportImage::art(&statics), format, false);
            let (back, conversion) = ArtColors::from_rgba(ArtKind::Static, rgba.width(), rgba.height(), rgba.as_raw()).unwrap();
            assert_eq!((back, conversion), (statics.clone(), RgbaConversion::default()), "{format:?} static");

            let rgba = import(&ExportImage::art(&land), format, true);
            let (back, conversion) = ArtColors::from_rgba(ArtKind::Land, rgba.width(), rgba.height(), rgba.as_raw()).unwrap();
            assert_eq!((back, conversion), (land.clone(), RgbaConversion::default()), "{format:?} land tile");

            let rgba = import(&ExportImage::gump(&gump), format, false);
            let (back, conversion) = GumpColors::from_rgba(rgba.width(), rgba.height(), rgba.as_raw()).unwrap();
            assert_eq!((back, conversion), (gump.clone(), RgbaConversion::default()), "{format:?} gump");
        }
        // The 16-bit BMP keeps a gump's bit 15 too.
        let flagged = GumpColors { colors: gump.colors.iter().map(|&color| color | FLAG_BIT).collect(), ..gump };
        let bmp = ExportImage::gump(&flagged).encode(ImageFileFormat::Bmp).unwrap();
        assert_eq!(bmp::decode_rgb555(&bmp).unwrap().unwrap().2, flagged.colors);
    }

    #[test]
    fn texmaps_import_back_to_their_colours() {
        let (width, height) = LandTextureSize::Small.dimensions();
        let texmap = TexmapRecord { size: LandTextureSize::Small, colors: colours(width as usize * height as usize) };
        let image = ExportImage::texmap(&texmap);
        let bmp = image.encode(ImageFileFormat::Bmp).unwrap();
        assert_eq!(bmp::decode_rgb555(&bmp).unwrap(), Some((width, height, texmap.colors.clone())));
        for format in [ImageFileFormat::Png, ImageFileFormat::WebP] {
            let rgba = image::load_from_memory(&image.encode(format).unwrap()).unwrap().into_rgba8();
            assert!(rgba.pixels().all(|pixel| pixel.0[ALPHA_CHANNEL] == OPAQUE_ALPHA), "{format:?}: a texmap is opaque");
            assert_eq!(TexmapRecord::from_rgba(width, height, rgba.as_raw()).unwrap(), texmap, "{format:?}");
        }
    }

    /// The palette and indices of an 8-bit BMP, read by hand.
    fn bmp_palette_and_indices(bytes: &[u8], width: usize, height: usize) -> (Vec<[u8; RGB_CHANNELS]>, Vec<u8>) {
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let palette_at = BMP_INFO_SIZE_AT + u32_at(BMP_INFO_SIZE_AT);
        let pixels_at = u32_at(BMP_PIXELS_OFFSET_AT);
        let palette = bytes[palette_at..pixels_at].chunks_exact(BGRA).map(|bgra| [bgra[2], bgra[1], bgra[0]]).collect();
        let row_bytes = width.next_multiple_of(BMP_ROW_ALIGNMENT);
        let indices = (0..height).rev().flat_map(|row| bytes[pixels_at + row * row_bytes..][..width].to_vec()).collect();
        (palette, indices)
    }

    #[test]
    fn animation_frames_export_their_palette_and_indices() {
        let (frame, palette) = frame();
        let image = ExportImage::classic_frame(&frame, &palette);
        let ExportImage::Paletted { indices, .. } = &image else { panic!("a frame not drawing index 0 is paletted") };
        assert_eq!(*indices, frame.pixels.iter().map(|pixel| pixel.unwrap_or(TRANSPARENT_INDEX)).collect::<Vec<_>>());
        let narrowed = |rgb: &[u8]| rgb.iter().fold(0u16, |color, &channel| color << CHANNEL_BITS | u16::from(channel >> WIDEN_SHIFT));
        let stored: Vec<u16> = palette.iter().map(|&color| color & !FLAG_BIT).collect();

        let png_bytes = image.encode(ImageFileFormat::Png).unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut decoded).unwrap();
        let info = reader.info();
        assert_eq!(info.color_type, png::ColorType::Indexed);
        assert_eq!(info.trns.as_deref(), Some(&FRAME_PNG_TRANSPARENCY[..]));
        assert_eq!(info.palette.as_ref().unwrap().chunks_exact(RGB_CHANNELS).map(narrowed).collect::<Vec<_>>(), stored);
        assert_eq!(&decoded, indices);

        let bmp_bytes = image.encode(ImageFileFormat::Bmp).unwrap();
        let (bmp_palette, bmp_indices) = bmp_palette_and_indices(&bmp_bytes, usize::from(frame.width), usize::from(frame.height));
        assert_eq!(bmp_palette.iter().map(|rgb| narrowed(rgb)).collect::<Vec<_>>(), stored);
        assert_eq!(&bmp_indices, indices);

        // The decoded RGBA goes back on the palette to the same colours.
        let decoded = ExportImage::classic_frame_rgba(image.width(), image.height(), &image.to_rgba(), &palette);
        assert!(matches!(decoded, ExportImage::Paletted { .. }));
        assert_eq!(decoded.to_rgba(), image.to_rgba());
        // A colour only index 0 holds stays RGBA.
        let mut only_zero = palette;
        only_zero[0] = colour(1) ^ CHANNEL_MASK;
        only_zero.iter_mut().skip(1).for_each(|color| *color = colour(1));
        let drawn = opaque(rgb_of(only_zero[0], widen_channel));
        assert!(matches!(ExportImage::classic_frame_rgba(1, 1, &drawn, &only_zero), ExportImage::Rgba { .. }));

        // A run drawing index 0 cannot be paletted with index 0 transparent.
        let drawn_zero = ClassicFrame { pixels: vec![Some(TRANSPARENT_INDEX); frame.pixels.len()], ..frame };
        assert!(matches!(ExportImage::classic_frame(&drawn_zero, &palette), ExportImage::Rgba { .. }));
    }

    #[test]
    fn png_webp_and_32_bit_bmp_decode_to_the_exported_rgba() {
        // Every byte value in every channel, partial alpha and colours under alpha 0 included.
        let pixels = usize::from(WIDTH) * usize::from(HEIGHT);
        let mut state = 0x9E37_79B9u32;
        let rgba: Vec<u8> = (0..pixels * RGBA_BYTES_PER_PIXEL)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect();
        let (frame, palette) = frame();
        let images = [
            ExportImage::Rgba { width: u32::from(WIDTH), height: u32::from(HEIGHT), rgba },
            ExportImage::art(&ArtColors::Land(colours(LAND_DIAMOND_PIXEL_COUNT))),
            ExportImage::gump(&GumpColors { width: WIDTH, height: HEIGHT, colors: colours(pixels) }),
            ExportImage::classic_frame(&frame, &palette),
        ];
        for image in &images {
            for format in [ImageFileFormat::Png, ImageFileFormat::WebP] {
                let decoded = image::load_from_memory(&image.encode(format).unwrap()).unwrap().into_rgba8();
                assert_eq!(decoded.into_raw(), image.to_rgba(), "{format:?}");
            }
        }
        let decoded = image::load_from_memory(&images[0].encode(ImageFileFormat::Bmp).unwrap()).unwrap().into_rgba8();
        assert_eq!(decoded.into_raw(), images[0].to_rgba(), "32-bit BMP");
    }

    #[test]
    fn widening_spans_the_byte_and_land_corners_stay_clear() {
        assert_eq!((widen_channel(0), widen_channel(1), widen_channel(16), widen_channel(CHANNEL_MASK)), (0, 8, 132, u8::MAX));
        let land = ExportImage::art(&ArtColors::Land(vec![0; LAND_DIAMOND_PIXEL_COUNT])).to_rgba();
        let alpha = |x: usize, y: usize| land[(y * LAND_DIMENSION + x) * RGBA_BYTES_PER_PIXEL + ALPHA_CHANNEL];
        let middle = LAND_DIMENSION / 2;
        assert_eq!((alpha(0, 0), alpha(middle, middle)), (0, OPAQUE_ALPHA), "a corner is clear, black inside is drawn");
    }

    #[test]
    fn a_classic_hue_recolours_and_keeps_transparency() {
        let mut image = ExportImage::gump(&GumpColors { width: 2, height: 1, colors: vec![0, colour(3)] });
        image.hue_classic_colors(|_| 0);
        assert_eq!(image, ExportImage::gump(&GumpColors { width: 2, height: 1, colors: vec![0, OPAQUE_BLACK] }));
    }

    #[test]
    fn file_stems_name_the_id_and_the_frame() {
        assert_eq!(id_file_stem("art", 0x0E75), "art_0x0E75");
        assert_eq!(animation_frame_stem(400, 0, 3, 1), "anim_400_a0_d3_f01");
        assert_eq!(sanitize_file_stem("body 400/action:2", "frame"), "body_400action2");
        assert_eq!(sanitize_file_stem("///", "frame"), "frame");
    }
}
