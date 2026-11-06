//! `facet00.mul`-`facet05.mul`, the Classic client's world map images (one pixel per tile, RGB555, run-length
//! encoded), read as the 2024 client reads them, strictly, and written back byte for byte; exported to and imported
//! from RGBA images.
//!
//! Traced in the 2024 Classic client (`UOClassicMapImage::CourseGump_DrawFacetImage`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-map-image`): the map item gump (`CCourseGump`, a map with its
//! course pins) reads file id `0x38 + facet` of the client's file table, the memory
//! mapped `facetNN.mul`. It skips the 4-byte header without reading it and walks the rows from the top: a `u32`
//! byte count, then runs of a `u8` pixel count and a `u16` colour, until it reaches the map's tile rectangle (clamped
//! to the facet's wrap size). It samples one pixel per tile, nearest, scaled to the gump. A colour with bit 15 set
//! is drawn black. Only the 7.0.x clients (7.0.20 and later) name the files; UO Gold's course gump shades the
//! `multimap.rle` outline instead.
//!
//! The file: `u16` width, `u16` height, then `height` rows of a `u32` byte count and its runs. Every shipped run
//! has a nonzero count, and a run never continues the previous run's colour below 255 pixels (the encoder is greedy,
//! [`encode_row`]). Rows are kept as read, so any file the client reads writes back byte for byte; an edited row is
//! encoded greedily.
//!
//! Strict where the client would draw something else than the file means ([`FacetImageFile::from_bytes`]): a row
//! whose runs do not add up to the width (the client would read past it into the next row, or never draw the rest),
//! a byte count that ends inside a run or past the file, and bytes after the last row (the client never reads them)
//! are errors. [`FacetImageFile::check_facet_size`] refuses a size other than the facet's: the client ignores the
//! header and draws tile (x, y) from column x of row y.
//!
//! As an RGBA image ([`FacetImageFile::to_rgba`]): each 5-bit channel times 8, opaque; a pixel with bit 15 set is
//! transparent, its colour bits kept in the RGB channels, so the image imports back to the same file
//! ([`FacetImageFile::replace_image`]). The import refuses a partly transparent pixel and a channel that is not a
//! multiple of 8, which RGB555 cannot store.

crate::eyre_imports!();
use std::path::Path;

use image::RgbaImage;

use crate::classic::map::CLASSIC_FACET_HEIGHTS;
use crate::client_exe::classic_facets::CLASSIC_STOCK_WIDTHS;
use crate::utils::output_file::write_through_temp;

/// Bytes of the width and height header.
pub const FACET_IMAGE_HEADER_BYTES: usize = 2 * size_of::<u16>();
/// Bytes of a row's byte count.
pub const ROW_LENGTH_BYTES: usize = size_of::<u32>();
/// Bytes of a run: the pixel count and the colour.
pub const RUN_BYTES: usize = size_of::<u8>() + size_of::<u16>();
/// The most pixels one run holds.
pub const MAX_RUN_PIXELS: u8 = u8::MAX;
/// Set in a colour the client draws black.
pub const UNDRAWN_BIT: u16 = 0x8000;
/// The facets that have an image (Felucca to Ter Mur).
pub const FACET_IMAGE_COUNT: u32 = 6;
const RGBA_CHANNELS: usize = 4;
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MASK: u16 = (1 << CHANNEL_BITS) - 1;
/// An 8-bit channel per 5-bit step.
const CHANNEL_STEP: u8 = 1 << (u8::BITS - CHANNEL_BITS);
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
const GREEN_SHIFT: u32 = CHANNEL_BITS;
const OPAQUE: u8 = u8::MAX;
const TRANSPARENT: u8 = 0;

/// The file name of a facet's image: `facet00.mul` for Felucca.
pub fn facet_image_file_name(facet: u32) -> String {
    format!("facet{facet:02}.mul")
}

/// The size of a facet's image in the shipped clients: the facet's size in tiles.
pub fn facet_image_size(facet: u32) -> Option<(u16, u16)> {
    let index = usize::try_from(facet).ok()?;
    let width = u16::try_from(*CLASSIC_STOCK_WIDTHS.get(index)?).ok()?;
    let height = u16::try_from(*CLASSIC_FACET_HEIGHTS.get(index)?).ok()?;
    Some((width, height))
}

/// The colour the client draws for a stored one: bit 15 set draws black.
pub fn drawn_colour(stored: u16) -> u16 {
    if stored & UNDRAWN_BIT != 0 { 0 } else { stored }
}

/// One run: `count` pixels of `colour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourRun {
    pub count: u8,
    pub colour: u16,
}

/// A `facetNN.mul` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetImageFile {
    width: u16,
    height: u16,
    rows: Vec<Vec<ColourRun>>,
}

/// Runs of one row of pixels: the longest runs, as the shipped files hold them.
pub fn encode_row(pixels: &[u16]) -> Vec<ColourRun> {
    let mut runs: Vec<ColourRun> = Vec::new();
    for &colour in pixels {
        match runs.last_mut() {
            Some(run) if run.colour == colour && run.count < MAX_RUN_PIXELS => run.count += 1,
            _ => runs.push(ColourRun { count: 1, colour }),
        }
    }
    runs
}

fn decode_row(runs: &[ColourRun], out: &mut Vec<u16>) {
    for run in runs {
        out.extend(std::iter::repeat_n(run.colour, usize::from(run.count)));
    }
}

impl FacetImageFile {
    /// An image of `width` x `height` pixels, row by row.
    pub fn from_pixels(width: u16, height: u16, pixels: &[u16]) -> eyre::Result<Self> {
        let row_len = usize::from(width);
        eyre::ensure!(
            pixels.len() == row_len * usize::from(height),
            "{} pixels for a {width} x {height} image",
            pixels.len()
        );
        let rows = if row_len == 0 { vec![Vec::new(); usize::from(height)] } else { pixels.chunks_exact(row_len).map(encode_row).collect() };
        Ok(Self { width, height, rows })
    }

    /// Reads a file, strictly (module docs).
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        eyre::ensure!(bytes.len() >= FACET_IMAGE_HEADER_BYTES, "{} bytes: shorter than the width and height", bytes.len());
        let width = u16::from_le_bytes([bytes[0], bytes[1]]);
        let height = u16::from_le_bytes([bytes[2], bytes[3]]);
        let mut at = FACET_IMAGE_HEADER_BYTES;
        let mut rows = Vec::with_capacity(usize::from(height));
        for y in 0..height {
            let Some(length) = bytes.get(at..at + ROW_LENGTH_BYTES) else {
                eyre::bail!("row {y} at byte {at}: the file ends before its byte count (the image is {width} x {height})");
            };
            let length = u32::from_le_bytes(length.try_into().expect("four bytes")) as usize;
            let start = at + ROW_LENGTH_BYTES;
            let Some(body) = bytes.get(start..start.saturating_add(length)) else {
                eyre::bail!("row {y} at byte {at}: {length} bytes, past the end of the file");
            };
            eyre::ensure!(
                length % RUN_BYTES == 0,
                "row {y} at byte {at}: {length} bytes end inside a run (a run is {RUN_BYTES} bytes); the client would read the next row's byte count as pixels"
            );
            let runs: Vec<ColourRun> = body
                .chunks_exact(RUN_BYTES)
                .map(|run| ColourRun { count: run[0], colour: u16::from_le_bytes([run[1], run[2]]) })
                .collect();
            let pixels: usize = runs.iter().map(|run| usize::from(run.count)).sum();
            eyre::ensure!(
                pixels == usize::from(width),
                "row {y} at byte {at}: {pixels} pixels in a {width}-pixel row; {}",
                if pixels < usize::from(width) {
                    "the client would read past the row into the next one"
                } else {
                    "the client never draws the pixels past the width, and the next rows are shifted for an editor"
                }
            );
            rows.push(runs);
            at = start + length;
        }
        eyre::ensure!(
            at == bytes.len(),
            "{} bytes after the last row, at byte {at}: the client never reads them",
            bytes.len() - at
        );
        Ok(Self { width, height, rows })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| path.display().to_string())?;
        Self::from_bytes(&bytes).wrap_err_with(|| path.display().to_string())
    }

    /// Reads `facetNN.mul` of `facet` and checks its size ([`Self::check_facet_size`]).
    pub fn load_for_facet(path: &Path, facet: u32) -> eyre::Result<Self> {
        let file = Self::load(path)?;
        file.check_facet_size(facet).wrap_err_with(|| path.display().to_string())?;
        Ok(file)
    }

    /// Refuses a size other than the facet's: the client ignores the header and draws tile (x, y) from column x
    /// of row y.
    pub fn check_facet_size(&self, facet: u32) -> eyre::Result<()> {
        let Some((width, height)) = facet_image_size(facet) else {
            eyre::bail!("facet {facet} has no image (facets 0 to {} do)", FACET_IMAGE_COUNT - 1);
        };
        self.check_size(u32::from(width), u32::from(height)).wrap_err_with(|| format!("facet {facet}"))
    }

    /// Refuses a size other than `width` x `height` tiles, the size of the map the client draws the image for (a
    /// client whose size table was patched draws a map of another size than the stock one).
    pub fn check_size(&self, width: u32, height: u32) -> eyre::Result<()> {
        eyre::ensure!(
            (u32::from(self.width), u32::from(self.height)) == (width, height),
            "a {} x {} image for a map of {width} x {height} tiles: the client ignores the header and draws tile \
             (x, y) from column x of row y, so it would show the wrong tiles",
            self.width,
            self.height
        );
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let runs: usize = self.rows.iter().map(Vec::len).sum();
        let mut bytes = Vec::with_capacity(FACET_IMAGE_HEADER_BYTES + self.rows.len() * ROW_LENGTH_BYTES + runs * RUN_BYTES);
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        for row in &self.rows {
            let length = u32::try_from(row.len() * RUN_BYTES).expect("a row of at most 65,535 runs");
            bytes.extend_from_slice(&length.to_le_bytes());
            for run in row {
                bytes.push(run.count);
                bytes.extend_from_slice(&run.colour.to_le_bytes());
            }
        }
        bytes
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| temp.display().to_string()))
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// The runs of each row, as read.
    pub fn rows(&self) -> &[Vec<ColourRun>] {
        &self.rows
    }

    /// The stored colours, row by row (bit 15 included).
    pub fn pixels(&self) -> Vec<u16> {
        let mut pixels = Vec::with_capacity(usize::from(self.width) * usize::from(self.height));
        for row in &self.rows {
            decode_row(row, &mut pixels);
        }
        pixels
    }

    /// Replaces the pixels (same size); a row whose pixels are unchanged keeps its runs, a changed row is encoded
    /// greedily. Returns the rows changed.
    pub fn set_pixels(&mut self, pixels: &[u16]) -> eyre::Result<usize> {
        let row_len = usize::from(self.width);
        eyre::ensure!(
            pixels.len() == row_len * usize::from(self.height),
            "{} pixels for a {} x {} image",
            pixels.len(),
            self.width,
            self.height
        );
        if row_len == 0 {
            return Ok(0);
        }
        let mut changed = 0;
        let mut old = Vec::with_capacity(row_len);
        for (row, new) in self.rows.iter_mut().zip(pixels.chunks_exact(row_len)) {
            old.clear();
            decode_row(row, &mut old);
            if old != new {
                *row = encode_row(new);
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// The image as RGBA (module docs: bit 15 transparent, its colour kept).
    pub fn to_rgba(&self) -> RgbaImage {
        let pixels = self.pixels();
        let mut rgba = Vec::with_capacity(pixels.len() * RGBA_CHANNELS);
        for colour in pixels {
            let channel = |shift: u32| ((colour >> shift) & CHANNEL_MASK) as u8 * CHANNEL_STEP;
            let alpha = if colour & UNDRAWN_BIT != 0 { TRANSPARENT } else { OPAQUE };
            rgba.extend_from_slice(&[channel(RED_SHIFT), channel(GREEN_SHIFT), channel(0), alpha]);
        }
        RgbaImage::from_raw(u32::from(self.width), u32::from(self.height), rgba).expect("width x height pixels")
    }

    /// The stored colours of an RGBA image, refusing what RGB555 cannot store (module docs).
    pub fn colours_of_rgba(image: &RgbaImage) -> eyre::Result<Vec<u16>> {
        let width = image.width();
        let mut colours = Vec::with_capacity(image.as_raw().len() / RGBA_CHANNELS);
        let (mut off_grid, mut partial) = (None, None);
        let (mut off_grid_count, mut partial_count) = (0usize, 0usize);
        for (index, pixel) in image.as_raw().chunks_exact(RGBA_CHANNELS).enumerate() {
            let at = (index as u32 % width, index as u32 / width);
            if pixel[3] != OPAQUE && pixel[3] != TRANSPARENT {
                partial_count += 1;
                partial.get_or_insert((at, pixel[3]));
            }
            if pixel[..3].iter().any(|&channel| channel % CHANNEL_STEP != 0) {
                off_grid_count += 1;
                off_grid.get_or_insert((at, [pixel[0], pixel[1], pixel[2]]));
            }
            let channel = |value: u8| u16::from(value / CHANNEL_STEP);
            let undrawn = if pixel[3] == TRANSPARENT { UNDRAWN_BIT } else { 0 };
            colours.push(undrawn | channel(pixel[0]) << RED_SHIFT | channel(pixel[1]) << GREEN_SHIFT | channel(pixel[2]));
        }
        if let Some(((x, y), alpha)) = partial {
            eyre::bail!(
                "{partial_count} pixels are partly transparent, the first at x {x}, y {y} (alpha {alpha}): a facet image pixel \
                 is opaque, or transparent for a colour with bit 15 set (drawn black)"
            );
        }
        if let Some(((x, y), rgb)) = off_grid {
            eyre::bail!(
                "{off_grid_count} pixels have a channel that is not a multiple of {CHANNEL_STEP}, the first at x {x}, y {y} \
                 ({rgb:?}): RGB555 cannot store it"
            );
        }
        Ok(colours)
    }

    /// Replaces the image with an RGBA one of the same size ([`Self::colours_of_rgba`], [`Self::set_pixels`]).
    pub fn replace_image(&mut self, image: &RgbaImage) -> eyre::Result<usize> {
        eyre::ensure!(
            (image.width(), image.height()) == (u32::from(self.width), u32::from(self.height)),
            "a {} x {} image for a {} x {} facet image",
            image.width(),
            image.height(),
            self.width,
            self.height
        );
        let colours = Self::colours_of_rgba(image)?;
        self.set_pixels(&colours)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u16 = 300;
    const HEIGHT: u16 = 2;
    const BLUE: u16 = 0x001f;
    const RED: u16 = 0x7c00;

    fn sample() -> FacetImageFile {
        let mut pixels = vec![BLUE; usize::from(WIDTH) * usize::from(HEIGHT)];
        pixels[usize::from(WIDTH) + 1] = RED | UNDRAWN_BIT;
        FacetImageFile::from_pixels(WIDTH, HEIGHT, &pixels).unwrap()
    }

    #[test]
    fn a_file_reads_back_byte_for_byte_and_its_image_imports_to_the_same_file() {
        let file = sample();
        assert_eq!(file.rows()[0], vec![ColourRun { count: MAX_RUN_PIXELS, colour: BLUE }, ColourRun { count: 45, colour: BLUE }]);
        let bytes = file.to_bytes();
        let back = FacetImageFile::from_bytes(&bytes).unwrap();
        assert_eq!(back, file);
        let image = back.to_rgba();
        assert_eq!(image.get_pixel(1, 1).0, [31 * 8, 0, 0, TRANSPARENT]);
        let mut edited = back.clone();
        assert_eq!(edited.replace_image(&image).unwrap(), 0);
        assert_eq!(edited.to_bytes(), bytes);
        assert_eq!(drawn_colour(RED | UNDRAWN_BIT), 0);
    }

    #[test]
    fn rows_the_client_reads_otherwise_are_errors() {
        // A run split where the encoder would not split it, and a zero run, are kept as read.
        let mut bytes = vec![2, 0, 1, 0, 9, 0, 0, 0, 1, 0x1f, 0, 0, 0, 0, 1, 0x1f, 0];
        let file = FacetImageFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.to_bytes(), bytes);
        bytes.push(0);
        assert!(FacetImageFile::from_bytes(&bytes).unwrap_err().to_string().contains("never reads"));
        let short = [2, 0, 1, 0, 3, 0, 0, 0, 1, 0x1f, 0];
        assert!(FacetImageFile::from_bytes(&short).unwrap_err().to_string().contains("past the row"));
        let inside = [1, 0, 1, 0, 4, 0, 0, 0, 1, 0x1f, 0, 0];
        assert!(FacetImageFile::from_bytes(&inside).unwrap_err().to_string().contains("inside a run"));
        let truncated = [1, 0, 1, 0, 6, 0, 0, 0, 1, 0x1f, 0];
        assert!(FacetImageFile::from_bytes(&truncated).unwrap_err().to_string().contains("past the end"));
        assert!(format!("{:#}", sample().check_facet_size(0).unwrap_err()).contains("wrong tiles"));
    }

    #[test]
    fn an_edited_image_changes_only_its_rows_and_what_rgb555_cannot_store_is_refused() {
        let mut file = sample();
        let mut image = file.to_rgba();
        image.put_pixel(5, 0, image::Rgba([8, 16, 24, OPAQUE]));
        assert_eq!(file.replace_image(&image).unwrap(), 1);
        assert_eq!(file.pixels()[5], 1 << RED_SHIFT | 2 << GREEN_SHIFT | 3);
        assert_eq!(FacetImageFile::from_bytes(&file.to_bytes()).unwrap(), file);
        image.put_pixel(6, 0, image::Rgba([9, 0, 0, OPAQUE]));
        assert!(file.replace_image(&image).unwrap_err().to_string().contains("multiple of 8"));
        image.put_pixel(6, 0, image::Rgba([8, 0, 0, 128]));
        assert!(file.replace_image(&image).unwrap_err().to_string().contains("partly transparent"));
        assert!(file.replace_image(&RgbaImage::new(1, 1)).is_err());
    }
}
