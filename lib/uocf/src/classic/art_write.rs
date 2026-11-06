//! Writing Classic art ([`ArtColors`]): the `art.mul` records and the `artLegacyMUL.uop` entries (the same bytes),
//! encoded as every shipped one is.
//!
//! The art id says what a record holds, as in the client (`UOClassicArt::GetArtRecord`, see
//! `scripts/dev-helpers/ec-research/ghidra-classic-art/`): an id below [`STATIC_ART_ID_BASE`] is a land tile, any other
//! a static (item id + 0x4000). [`ArtKind::of_id`].
//!
//! The shipped convention, measured on every record of the Classic (`artLegacyMUL.uop`), Classic fp, 7.0.20,
//! Mondain's Legacy and UO Gold (`art.mul`) clients:
//! - **Land:** 2,048 bytes: the 1,012 colors of the 44x44 diamond, rows of 2, 4, .. 44 then 44, .. 2 pixels, then
//!   24 bytes after them, zero in the Classic clients and old memory in most Mondain's Legacy and UO Gold ones. Color 0 is opaque
//!   black: a land diamond has no transparent pixel.
//! - **Static:** a `u32` of the byte length of the rows (the client does not read it), `u16` width and height, a
//!   `u16` offset per row in 2-byte units from the end of that table, then the rows. A row is `(x offset, run)`
//!   pairs, each followed by `run` colors, the offset counted from the end of the previous run; `(0, 0)` ends it.
//!   Runs are the opaque spans: none touches the next, none holds color 0 (transparent). Each row is padded to a
//!   multiple of 4 bytes with one `u16` after its `(0, 0)`, zero or old memory (often a color of the record).
//!   Rows are back to back in order, except that a row equal to an earlier one (most often an empty row) points
//!   at it. Nothing follows the last row.
//!
//! [`ArtColors::encode`] writes that with zero padding, so a decoded shipped record encodes back byte for byte,
//! except for the padding words that held old memory. Colors are 15-bit RGB; no shipped record sets bit 15.
//!
//! From RGBA ([`ArtColors::from_rgba`]), the gump rules: alpha 0 is transparent, 255 opaque, other alphas refused;
//! channels round to the nearest 5-bit value. A static stores opaque black as [`OPAQUE_BLACK`] (0 is transparent).
//! A land tile is a 44x44 image whose diamond is opaque and whose corners are transparent.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::classic::art::{ART_LEGACY_UOP_MAX_ID_EXCLUSIVE, LAND_DIAMOND_PIXEL_COUNT, LAND_DIMENSION, RGBA_BYTES_PER_PIXEL, STATIC_TILE_ID_BASE};
use crate::classic::gump_write::{
    OPAQUE_ALPHA, OPAQUE_BLACK, RgbaConversion, TRANSPARENT, TRANSPARENT_ALPHA, is_rounded_opaque, refuse_partial_alpha,
    rgb555_of_rgba,
};
use crate::uop_container::hash::hash_file_name_single;
use crate::utils::color::color_lut;

crate::eyre_imports!();

/// The first static art id: lower ids are land tiles.
pub const STATIC_ART_ID_BASE: u32 = STATIC_TILE_ID_BASE;
/// The length of every shipped land record.
pub const LAND_RECORD_BYTES: usize = 2048;
const COLOR_BYTES: usize = size_of::<u16>();
const LAND_HALF_ROWS: usize = LAND_DIMENSION / 2;
/// The diamond's rows grow and shrink by one pixel on each side.
const LAND_ROW_STEP: usize = 2;
const STATIC_LENGTH_BYTES: usize = size_of::<u32>();
const STATIC_HEADER_BYTES: usize = STATIC_LENGTH_BYTES + 2 * size_of::<u16>();
const RUN_PAIR_WORDS: usize = 2;
/// Each static row is padded to a multiple of 4 bytes.
const ROW_ALIGNMENT_WORDS: usize = size_of::<u32>() / COLOR_BYTES;

/// The `artLegacyMUL.uop` entry name of art `art_id` (statics are item id + 0x4000).
pub fn uop_entry_name(art_id: u32) -> String {
    format!("build/artlegacymul/{art_id:08}.tga")
}

/// The art id whose `artLegacyMUL.uop` entry name ([`uop_entry_name`]) hashes to `hash`.
pub fn art_id_of_uop_hash(hash: u64) -> Option<u32> {
    static IDS: OnceLock<HashMap<u64, u32>> = OnceLock::new();
    IDS.get_or_init(|| (0..ART_LEGACY_UOP_MAX_ID_EXCLUSIVE).map(|id| (hash_file_name_single(&uop_entry_name(id)), id)).collect())
        .get(&hash)
        .copied()
}

/// Land tile or static, by art id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtKind {
    Land,
    Static,
}

impl ArtKind {
    /// As the client: an id below [`STATIC_ART_ID_BASE`] is a land tile.
    pub fn of_id(art_id: u32) -> Self {
        if art_id < STATIC_ART_ID_BASE { Self::Land } else { Self::Static }
    }
}

/// An art record as 15-bit colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtColors {
    /// The [`LAND_DIAMOND_PIXEL_COUNT`] diamond colors in record order; 0 is black.
    Land(Vec<u16>),
    /// `width * height` colors row by row; 0 is transparent.
    Static { width: u16, height: u16, colors: Vec<u16> },
}

/// The `(x, y, width)` of each land diamond row, in record order.
pub(crate) fn land_rows() -> impl Iterator<Item = (usize, usize, usize)> {
    let growing = (0..LAND_HALF_ROWS).map(|row| (LAND_HALF_ROWS - 1 - row, row, LAND_ROW_STEP * (row + 1)));
    let shrinking = (0..LAND_HALF_ROWS).map(|row| (row, LAND_HALF_ROWS + row, LAND_DIMENSION - LAND_ROW_STEP * row));
    growing.chain(shrinking)
}

fn word(bytes: &[u8], word_index: usize) -> Option<u16> {
    let at = word_index * COLOR_BYTES;
    bytes.get(at..at + COLOR_BYTES).map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
}

impl ArtColors {
    /// Reads a record of `kind` as strictly as the decoders of [`crate::classic::art`].
    pub fn decode(kind: ArtKind, record: &[u8]) -> eyre::Result<Self> {
        match kind {
            ArtKind::Land => Self::decode_land(record),
            ArtKind::Static => Self::decode_static(record),
        }
    }

    fn decode_land(record: &[u8]) -> eyre::Result<Self> {
        if record.len() < LAND_DIAMOND_PIXEL_COUNT * COLOR_BYTES {
            eyre::bail!("{} bytes, shorter than a land diamond ({} bytes)", record.len(), LAND_DIAMOND_PIXEL_COUNT * COLOR_BYTES);
        }
        Ok(Self::Land((0..LAND_DIAMOND_PIXEL_COUNT).map(|index| word(record, index).expect("checked length")).collect()))
    }

    fn decode_static(record: &[u8]) -> eyre::Result<Self> {
        let header = record.get(..STATIC_HEADER_BYTES).ok_or_else(|| eyre!("{} bytes, shorter than a static header", record.len()))?;
        let dimension = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
        let (width, height) = (dimension(STATIC_LENGTH_BYTES), dimension(STATIC_LENGTH_BYTES + COLOR_BYTES));
        if width == 0 || height == 0 {
            eyre::bail!("a static of {width}x{height} has no pixels");
        }
        let (width_px, height_px) = (usize::from(width), usize::from(height));
        let rows_start = STATIC_HEADER_BYTES / COLOR_BYTES + height_px;
        if rows_start * COLOR_BYTES > record.len() {
            eyre::bail!("the lookup table of {height} rows extends past the {} bytes", record.len());
        }
        let mut colors = vec![TRANSPARENT; width_px * height_px];
        for y in 0..height_px {
            let mut at = rows_start + usize::from(word(record, STATIC_HEADER_BYTES / COLOR_BYTES + y).expect("checked length"));
            let mut x = 0usize;
            loop {
                let (Some(offset), Some(run)) = (word(record, at), word(record, at + 1)) else {
                    eyre::bail!("row {y}: the runs end past the data");
                };
                at += RUN_PAIR_WORDS;
                if offset == 0 && run == 0 {
                    break;
                }
                x += usize::from(offset);
                let end = x + usize::from(run);
                if end > width_px {
                    eyre::bail!("row {y}: a run of {run} at x {x} in a width of {width}");
                }
                for target in &mut colors[y * width_px + x..y * width_px + end] {
                    *target = word(record, at).ok_or_else(|| eyre!("row {y}: the colors end past the data"))?;
                    at += 1;
                }
                x = end;
            }
        }
        Ok(Self::Static { width, height, colors })
    }

    /// The record bytes, as the shipped records are (see the module docs), with zero padding.
    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        match self {
            Self::Land(colors) => {
                if colors.len() != LAND_DIAMOND_PIXEL_COUNT {
                    eyre::bail!("{} land colors, not {LAND_DIAMOND_PIXEL_COUNT}", colors.len());
                }
                let mut record: Vec<u8> = colors.iter().flat_map(|color| color.to_le_bytes()).collect();
                record.resize(LAND_RECORD_BYTES, 0);
                Ok(record)
            }
            Self::Static { width, height, colors } => encode_static(*width, *height, colors),
        }
    }

    /// The colors of a `width x height` RGBA image as art of `kind` (see the module docs for the rules).
    pub fn from_rgba(kind: ArtKind, width: u32, height: u32, rgba: &[u8]) -> eyre::Result<(Self, RgbaConversion)> {
        let (Ok(width16), Ok(height16)) = (u16::try_from(width), u16::try_from(height)) else {
            eyre::bail!("an image of {width}x{height} is larger than art holds ({} per side)", u16::MAX);
        };
        if width == 0 || height == 0 {
            eyre::bail!("an image of {width}x{height} has no pixels");
        }
        let width_px = usize::from(width16);
        if rgba.len() != width_px * usize::from(height16) * RGBA_BYTES_PER_PIXEL {
            eyre::bail!("{} RGBA bytes for {width}x{height}", rgba.len());
        }
        refuse_partial_alpha(width_px, rgba, if kind == ArtKind::Land { "land tile" } else { "static" })?;
        let mut conversion = RgbaConversion::default();
        let pixel = |x: usize, y: usize| &rgba[(y * width_px + x) * RGBA_BYTES_PER_PIXEL..][..RGBA_BYTES_PER_PIXEL];
        match kind {
            ArtKind::Land => {
                if width_px != LAND_DIMENSION || usize::from(height16) != LAND_DIMENSION {
                    eyre::bail!("a land tile is {LAND_DIMENSION}x{LAND_DIMENSION}, not {width}x{height}");
                }
                let mut inside = vec![false; LAND_DIMENSION * LAND_DIMENSION];
                let mut colors = Vec::with_capacity(LAND_DIAMOND_PIXEL_COUNT);
                for (row_x, y, row_width) in land_rows() {
                    for x in row_x..row_x + row_width {
                        let value = pixel(x, y);
                        if value[3] != OPAQUE_ALPHA {
                            eyre::bail!("the land diamond pixel at x {x}, y {y} is transparent; the diamond is opaque");
                        }
                        conversion.rounded += usize::from(is_rounded_opaque(value));
                        inside[y * LAND_DIMENSION + x] = true;
                        colors.push(rgb555_of_rgba(value));
                    }
                }
                if let Some(outside) = (0..inside.len()).find(|&index| !inside[index] && rgba[index * RGBA_BYTES_PER_PIXEL + 3] != TRANSPARENT_ALPHA) {
                    let (x, y) = (outside % LAND_DIMENSION, outside / LAND_DIMENSION);
                    eyre::bail!("the pixel at x {x}, y {y} is outside the land diamond, so it must be transparent");
                }
                Ok((Self::Land(colors), conversion))
            }
            ArtKind::Static => {
                let colors = rgba
                    .chunks_exact(RGBA_BYTES_PER_PIXEL)
                    .map(|value| {
                        if value[3] == TRANSPARENT_ALPHA {
                            return TRANSPARENT;
                        }
                        conversion.rounded += usize::from(is_rounded_opaque(value));
                        match rgb555_of_rgba(value) {
                            TRANSPARENT => {
                                conversion.black += 1;
                                OPAQUE_BLACK
                            }
                            color => color,
                        }
                    })
                    .collect();
                Ok((Self::Static { width: width16, height: height16, colors }, conversion))
            }
        }
    }

    /// The RGBA image, as [`crate::classic::art`] decodes the record: `(width, height, rgba)`.
    pub fn to_rgba(&self) -> (u16, u16, Vec<u8>) {
        let lut = color_lut();
        let opaque = |color: u16| lut[usize::from(color)].to_le_bytes();
        match self {
            Self::Land(colors) => {
                let mut rgba = vec![0; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL];
                let positions = land_rows().flat_map(|(row_x, y, row_width)| (row_x..row_x + row_width).map(move |x| y * LAND_DIMENSION + x));
                for (index, &color) in positions.zip(colors) {
                    rgba[index * RGBA_BYTES_PER_PIXEL..][..RGBA_BYTES_PER_PIXEL].copy_from_slice(&opaque(color));
                }
                (LAND_DIMENSION as u16, LAND_DIMENSION as u16, rgba)
            }
            Self::Static { width, height, colors } => {
                let rgba = colors.iter().flat_map(|&color| if color == TRANSPARENT { [0; RGBA_BYTES_PER_PIXEL] } else { opaque(color) }).collect();
                (*width, *height, rgba)
            }
        }
    }
}

fn encode_static(width: u16, height: u16, colors: &[u16]) -> eyre::Result<Vec<u8>> {
    let width_px = usize::from(width);
    if width_px == 0 || height == 0 || colors.len() != width_px * usize::from(height) {
        eyre::bail!("{} colors for a static of {width}x{height}", colors.len());
    }
    let mut lookup = Vec::with_capacity(usize::from(height));
    let mut rows: Vec<u16> = Vec::new();
    let mut written: Vec<(usize, usize)> = Vec::new();
    for row in colors.chunks_exact(width_px) {
        let mut encoded = Vec::new();
        let mut cursor = 0usize;
        let mut x = 0usize;
        while x < width_px {
            if row[x] == TRANSPARENT {
                x += 1;
                continue;
            }
            let run = row[x..].iter().take_while(|&&color| color != TRANSPARENT).count();
            // x and run are below the width, a u16.
            encoded.extend([(x - cursor) as u16, run as u16]);
            encoded.extend_from_slice(&row[x..x + run]);
            x += run;
            cursor = x;
        }
        encoded.extend([0, 0]);
        encoded.resize(encoded.len().next_multiple_of(ROW_ALIGNMENT_WORDS), 0);
        let start = match written.iter().find(|&&(start, len)| rows[start..start + len] == encoded[..]) {
            Some(&(start, _)) => start,
            None => {
                written.push((rows.len(), encoded.len()));
                rows.extend_from_slice(&encoded);
                rows.len() - encoded.len()
            }
        };
        lookup.push(u16::try_from(start).map_err(|_| eyre!("the static's rows are too long for its u16 row offsets"))?);
    }
    let row_bytes: Vec<u8> = rows.iter().flat_map(|value| value.to_le_bytes()).collect();
    let mut record = Vec::with_capacity(STATIC_HEADER_BYTES + lookup.len() * COLOR_BYTES + row_bytes.len());
    record.extend_from_slice(&u32::try_from(row_bytes.len())?.to_le_bytes());
    record.extend_from_slice(&width.to_le_bytes());
    record.extend_from_slice(&height.to_le_bytes());
    record.extend(lookup.iter().flat_map(|offset| offset.to_le_bytes()));
    record.extend_from_slice(&row_bytes);
    Ok(record)
}

/// The record of art `art_id` from an RGBA image (land or static by the id), and what the conversion changed.
pub fn encode_art_from_rgba(art_id: u32, width: u32, height: u32, rgba: &[u8]) -> eyre::Result<(Vec<u8>, RgbaConversion)> {
    let (colors, conversion) = ArtColors::from_rgba(ArtKind::of_id(art_id), width, height, rgba)?;
    Ok((colors.encode()?, conversion))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::art::{check_art_payload, decode_land_tile_from_raw, decode_static_tile_from_raw};

    const RED: u16 = 0x7C00;
    const BLUE: u16 = 0x001F;

    #[test]
    fn art_ids_come_back_from_entry_hashes() {
        for id in [0, STATIC_ART_ID_BASE - 1, STATIC_ART_ID_BASE, ART_LEGACY_UOP_MAX_ID_EXCLUSIVE - 1] {
            assert_eq!(art_id_of_uop_hash(hash_file_name_single(&uop_entry_name(id))), Some(id));
        }
        assert_eq!(art_id_of_uop_hash(hash_file_name_single("build/gumpartlegacymul/00000001.tga")), None);
    }

    #[test]
    fn kind_follows_the_art_id() {
        assert_eq!(ArtKind::of_id(0), ArtKind::Land);
        assert_eq!(ArtKind::of_id(STATIC_ART_ID_BASE - 1), ArtKind::Land);
        assert_eq!(ArtKind::of_id(STATIC_ART_ID_BASE), ArtKind::Static);
    }

    #[test]
    fn encodes_static_runs_shared_rows_and_padding() {
        // Rows: red _ blue ; empty ; red _ blue (shared with row 0).
        let art = ArtColors::Static { width: 3, height: 3, colors: vec![RED, TRANSPARENT, BLUE, TRANSPARENT, TRANSPARENT, TRANSPARENT, RED, TRANSPARENT, BLUE] };
        let record = art.encode().unwrap();
        // Rows: [0,1,red, 1,1,blue, 0,0] (8 words, 4-byte aligned), then [0,0]; row 2 points at row 0.
        #[rustfmt::skip]
        let expected: Vec<u16> = vec![
            20, 0, 3, 3,
            0, 8, 0,
            0, 1, RED, 1, 1, BLUE, 0, 0,
            0, 0,
        ];
        let expected: Vec<u8> = expected.iter().flat_map(|value| value.to_le_bytes()).collect();
        assert_eq!(record, expected);
        assert_eq!(ArtColors::decode(ArtKind::Static, &record).unwrap(), art);
        let (width, height, rgba) = decode_static_tile_from_raw(&record).unwrap();
        assert_eq!((width, height), (3, 3));
        assert_eq!(art.to_rgba().2, rgba);

        let odd = ArtColors::Static { width: 1, height: 1, colors: vec![RED] };
        let record = odd.encode().unwrap();
        // [0,1,red,0,0] is 10 bytes, padded to 12.
        assert_eq!(&record[..4], &12u32.to_le_bytes());
        assert_eq!(record.len(), STATIC_HEADER_BYTES + COLOR_BYTES + 12);
        assert_eq!(&record[record.len() - 2..], &[0, 0]);
        assert_eq!(ArtColors::decode(ArtKind::Static, &record).unwrap(), odd);

        // Every row is padded: [0,1,red,0,0,pad] twice, distinct rows.
        let two = ArtColors::Static { width: 1, height: 2, colors: vec![RED, BLUE] };
        let record = two.encode().unwrap();
        assert_eq!(&record[STATIC_HEADER_BYTES..STATIC_HEADER_BYTES + 4], &[0, 0, 6, 0]);
        assert_eq!(ArtColors::decode(ArtKind::Static, &record).unwrap(), two);
    }

    #[test]
    fn land_round_trips_through_rgba() {
        let colors: Vec<u16> = (0..LAND_DIAMOND_PIXEL_COUNT as u16).collect();
        let land = ArtColors::Land(colors);
        let record = land.encode().unwrap();
        assert_eq!(record.len(), LAND_RECORD_BYTES);
        check_art_payload(&record).unwrap();
        let mut decoded = [0u8; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL];
        decode_land_tile_from_raw(&record, &mut decoded).unwrap();
        let (width, height, rgba) = land.to_rgba();
        assert_eq!((usize::from(width), usize::from(height)), (LAND_DIMENSION, LAND_DIMENSION));
        assert_eq!(rgba, decoded);
        let (again, conversion) = ArtColors::from_rgba(ArtKind::Land, width.into(), height.into(), &rgba).unwrap();
        assert_eq!(again, land);
        assert_eq!(conversion, RgbaConversion::default());
        assert_eq!(ArtColors::decode(ArtKind::Land, &record).unwrap(), land);
    }

    #[test]
    fn reads_rgba_strictly() {
        let rgba = [248, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 255, 250, 3, 0, 255];
        let (art, conversion) = ArtColors::from_rgba(ArtKind::Static, 2, 2, &rgba).unwrap();
        assert_eq!(art, ArtColors::Static { width: 2, height: 2, colors: vec![RED, TRANSPARENT, OPAQUE_BLACK, RED] });
        assert_eq!(conversion, RgbaConversion { rounded: 1, black: 1 });
        let error = ArtColors::from_rgba(ArtKind::Static, 2, 1, &[0, 0, 0, 128, 0, 0, 0, 255]).unwrap_err().to_string();
        assert!(error.contains("x 0, y 0 (alpha 128)"), "{error}");

        let side = LAND_DIMENSION as u32;
        let mut land = ArtColors::Land(vec![0; LAND_DIAMOND_PIXEL_COUNT]).to_rgba().2;
        assert!(ArtColors::from_rgba(ArtKind::Land, side, side, &land).is_ok(), "black is a land color");
        assert!(ArtColors::from_rgba(ArtKind::Land, side - 1, side, &land[..land.len() - LAND_DIMENSION * RGBA_BYTES_PER_PIXEL]).is_err());
        land[3] = u8::MAX;
        let error = ArtColors::from_rgba(ArtKind::Land, side, side, &land).unwrap_err().to_string();
        assert!(error.contains("x 0, y 0 is outside"), "{error}");
        land[3] = 0;
        let diamond_top = (LAND_HALF_ROWS - 1) * RGBA_BYTES_PER_PIXEL;
        land[diamond_top + 3] = 0;
        let error = ArtColors::from_rgba(ArtKind::Land, side, side, &land).unwrap_err().to_string();
        assert!(error.contains("x 21, y 0 is transparent"), "{error}");
    }
}
