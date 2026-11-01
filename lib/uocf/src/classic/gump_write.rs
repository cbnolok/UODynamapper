//! Writing Classic gumps ([`GumpColors`]): the row RLE of `gumpart.mul` and, after its width/height header, of the
//! `gumpartLegacyMUL.uop` entries, encoded as every shipped gump is.
//!
//! The shipped convention, measured on every gump that decodes (Classic 4,422 UOP entries; 7.0.20 3,164, Mondain's
//! Legacy 2,600 and UO Gold 2,416 `gumpart.mul` slots): the lookup table (one `u32` per row, in 4-byte units from
//! the start of the rows), then the rows in order, back to back, none shared; each row the fewest
//! `(color, run)` pairs (no two neighbours of one color); nothing after the last row. [`GumpColors::encode_rows`]
//! writes that, so a decoded shipped gump encodes back byte for byte.
//!
//! Colors are 15-bit RGB, 0 transparent. The only bit-15 color shipped is in one Mondain's Legacy and one UO Gold
//! gump; [`GumpColors`] keeps it. From RGBA ([`GumpColors::from_rgba`]): alpha 0 is transparent, 255 opaque, and
//! other alphas are refused (the format has none); channels round to the nearest 5-bit value, and opaque black,
//! which would be transparent, is stored as `0x0001`, the near black the shipped gumps use most (2,498 Classic
//! gumps).
//!
//! The 34 `gumpartLegacyMUL.uop` entries that do not decode are 8 zero bytes: a 0x0 header and no rows. They are
//! kept as they are; a gump replaces them only through the generic entry editor's kind check.

use crate::classic::gump::RGBA_BYTES_PER_PIXEL;

crate::eyre_imports!();

pub const TRANSPARENT: u16 = 0;
/// What opaque black is stored as: color 0 is transparent.
pub const OPAQUE_BLACK: u16 = 0x0001;
const OPAQUE_ALPHA: u8 = u8::MAX;
const TRANSPARENT_ALPHA: u8 = 0;
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MAX: u16 = (1 << CHANNEL_BITS) - 1;
/// An 8-bit channel per 5-bit step, as the decoder expands them.
const CHANNEL_STEP: u16 = 1 << (u8::BITS - CHANNEL_BITS);
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
const GREEN_SHIFT: u32 = CHANNEL_BITS;
const LOOKUP_ENTRY_BYTES: usize = size_of::<u32>();
const RLE_PAIR_BYTES: usize = 2 * size_of::<u16>();
const UOP_DIMENSION_BYTES: usize = size_of::<u32>();
/// The `gumpidx.mul` extra field: the width in the high 16 bits, the height in the low.
const INDEX_EXTRA_WIDTH_SHIFT: u32 = u16::BITS;

/// A gump as 15-bit colors, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GumpColors {
    pub width: u16,
    pub height: u16,
    /// `width * height` colors, 0 transparent.
    pub colors: Vec<u16>,
}

/// What [`GumpColors::from_rgba`] changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RgbaConversion {
    /// Opaque pixels whose channels were not on the 5-bit grid, rounded to it.
    pub rounded: usize,
    /// Opaque black pixels, stored as [`OPAQUE_BLACK`].
    pub black: usize,
}

impl GumpColors {
    /// Reads the rows of a gump (a `gumpart.mul` slot, or a UOP entry after its header), as strictly as
    /// [`crate::classic::gump::decode_gump_from_raw`].
    pub fn decode_rows(rows: &[u8], width: u16, height: u16) -> eyre::Result<Self> {
        if width == 0 || height == 0 {
            eyre::bail!("a gump of {width}x{height} has no pixels");
        }
        let (width_px, height_px) = (usize::from(width), usize::from(height));
        if rows.len() < height_px * LOOKUP_ENTRY_BYTES {
            eyre::bail!("the lookup table of {height} rows extends past the {} bytes", rows.len());
        }
        let read_u32 = |offset: usize| u32::from_le_bytes(rows[offset..offset + LOOKUP_ENTRY_BYTES].try_into().expect("4 bytes"));
        let mut colors = Vec::with_capacity(width_px * height_px);
        for y in 0..height_px {
            let mut offset = usize::try_from(read_u32(y * LOOKUP_ENTRY_BYTES))?
                .checked_mul(RLE_PAIR_BYTES)
                .ok_or_else(|| eyre!("row {y}: the lookup offset overflows"))?;
            let mut x = 0;
            while x < width_px {
                let pair = rows.get(offset..offset + RLE_PAIR_BYTES).ok_or_else(|| eyre!("row {y}: the runs end past the data at x {x}"))?;
                let color = u16::from_le_bytes([pair[0], pair[1]]);
                let run = usize::from(u16::from_le_bytes([pair[2], pair[3]]));
                if run == 0 || x + run > width_px {
                    eyre::bail!("row {y}: a run of {run} at x {x} in a width of {width}");
                }
                colors.extend(std::iter::repeat_n(color, run));
                x += run;
                offset += RLE_PAIR_BYTES;
            }
        }
        Ok(Self { width, height, colors })
    }

    /// Reads a `gumpartLegacyMUL.uop` entry: the `u32` width and height, then the rows.
    pub fn decode_uop(payload: &[u8]) -> eyre::Result<Self> {
        let header = payload.get(..2 * UOP_DIMENSION_BYTES).ok_or_else(|| eyre!("{} bytes, shorter than the width/height header", payload.len()))?;
        let dimension = |at: usize| u32::from_le_bytes(header[at..at + UOP_DIMENSION_BYTES].try_into().expect("4 bytes"));
        let (width, height) = (dimension(0), dimension(UOP_DIMENSION_BYTES));
        let (Ok(width16), Ok(height16)) = (u16::try_from(width), u16::try_from(height)) else {
            eyre::bail!("a gump of {width}x{height} is larger than the format holds");
        };
        Self::decode_rows(&payload[2 * UOP_DIMENSION_BYTES..], width16, height16)
    }

    /// The colors of an RGBA image: alpha 0 transparent, 255 opaque, other alphas refused.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> eyre::Result<(Self, RgbaConversion)> {
        let (Ok(width16), Ok(height16)) = (u16::try_from(width), u16::try_from(height)) else {
            eyre::bail!("an image of {width}x{height} is larger than a gump holds ({} per side)", u16::MAX);
        };
        if width == 0 || height == 0 {
            eyre::bail!("an image of {width}x{height} has no pixels");
        }
        let pixels = usize::from(width16) * usize::from(height16);
        if rgba.len() != pixels * RGBA_BYTES_PER_PIXEL {
            eyre::bail!("{} RGBA bytes for {width}x{height}", rgba.len());
        }
        let partial: Vec<usize> = rgba
            .chunks_exact(RGBA_BYTES_PER_PIXEL)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] != OPAQUE_ALPHA && pixel[3] != TRANSPARENT_ALPHA)
            .map(|(index, _)| index)
            .collect();
        if let Some(&first) = partial.first() {
            let (x, y) = (first % usize::from(width16), first / usize::from(width16));
            eyre::bail!(
                "{} pixels are partly transparent, the first at x {x}, y {y} (alpha {}); a gump pixel is opaque or transparent",
                partial.len(),
                rgba[first * RGBA_BYTES_PER_PIXEL + 3]
            );
        }
        let mut conversion = RgbaConversion::default();
        let colors = rgba
            .chunks_exact(RGBA_BYTES_PER_PIXEL)
            .map(|pixel| {
                if pixel[3] == TRANSPARENT_ALPHA {
                    return TRANSPARENT;
                }
                let channel = |value: u8| ((u16::from(value) + CHANNEL_STEP / 2) / CHANNEL_STEP).min(CHANNEL_MAX);
                let (red, green, blue) = (channel(pixel[0]), channel(pixel[1]), channel(pixel[2]));
                match red << RED_SHIFT | green << GREEN_SHIFT | blue {
                    TRANSPARENT => {
                        conversion.black += 1;
                        OPAQUE_BLACK
                    }
                    color => color,
                }
            })
            .collect();
        conversion.rounded = rgba
            .chunks_exact(RGBA_BYTES_PER_PIXEL)
            .filter(|pixel| pixel[3] == OPAQUE_ALPHA && pixel[..3].iter().any(|&value| u16::from(value) % CHANNEL_STEP != 0))
            .count();
        Ok((Self { width: width16, height: height16, colors }, conversion))
    }

    /// The rows as the shipped gumps store them: the lookup table, then each row's fewest runs, back to back.
    pub fn encode_rows(&self) -> eyre::Result<Vec<u8>> {
        let width = usize::from(self.width);
        if self.colors.len() != width * usize::from(self.height) || width == 0 {
            eyre::bail!("{} colors for {}x{}", self.colors.len(), self.width, self.height);
        }
        let mut lookup = Vec::with_capacity(usize::from(self.height) * LOOKUP_ENTRY_BYTES);
        let mut runs = Vec::new();
        let lookup_units = usize::from(self.height) * LOOKUP_ENTRY_BYTES / RLE_PAIR_BYTES;
        for row in self.colors.chunks_exact(width) {
            let start = u32::try_from(lookup_units + runs.len() / RLE_PAIR_BYTES).wrap_err("the gump is too large for its lookup offsets")?;
            lookup.extend_from_slice(&start.to_le_bytes());
            for run in row.chunk_by(|a, b| a == b) {
                // A row is at most u16::MAX wide, so a run fits.
                runs.extend_from_slice(&run[0].to_le_bytes());
                runs.extend_from_slice(&(run.len() as u16).to_le_bytes());
            }
        }
        lookup.extend_from_slice(&runs);
        Ok(lookup)
    }

    /// A `gumpartLegacyMUL.uop` entry: the `u32` width and height, then [`Self::encode_rows`].
    pub fn encode_uop(&self) -> eyre::Result<Vec<u8>> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&u32::from(self.width).to_le_bytes());
        payload.extend_from_slice(&u32::from(self.height).to_le_bytes());
        payload.extend_from_slice(&self.encode_rows()?);
        Ok(payload)
    }

    /// The `gumpidx.mul` extra field of the slot.
    pub fn index_extra(&self) -> u32 {
        u32::from(self.width) << INDEX_EXTRA_WIDTH_SHIFT | u32::from(self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::gump::{decode_gump_from_raw, decode_uop_gump};

    const RED: u16 = 0x7C00;
    const BLUE: u16 = 0x001F;

    #[test]
    fn encodes_the_fewest_runs_back_to_back() {
        let gump = GumpColors { width: 4, height: 2, colors: vec![RED, TRANSPARENT, TRANSPARENT, RED, BLUE, BLUE, BLUE, BLUE] };
        let rows = gump.encode_rows().unwrap();
        #[rustfmt::skip]
        let expected = [
            2, 0, 0, 0, 5, 0, 0, 0,
            0x00, 0x7C, 1, 0, 0, 0, 2, 0, 0x00, 0x7C, 1, 0,
            0x1F, 0, 4, 0,
        ];
        assert_eq!(rows, expected);
        assert_eq!(GumpColors::decode_rows(&rows, 4, 2).unwrap(), gump);
        assert_eq!(decode_gump_from_raw(&rows, 4, 2).unwrap()[..4], [248, 0, 0, 255]);
        let payload = gump.encode_uop().unwrap();
        assert_eq!(GumpColors::decode_uop(&payload).unwrap(), gump);
        assert_eq!(decode_uop_gump(&payload).unwrap().0, 4);
        assert_eq!(gump.index_extra(), 0x0004_0002);
    }

    #[test]
    fn reads_rgba_strictly() {
        let rgba = [248, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 255, 250, 3, 0, 255];
        let (gump, conversion) = GumpColors::from_rgba(2, 2, &rgba).unwrap();
        assert_eq!(gump.colors, [RED, TRANSPARENT, OPAQUE_BLACK, RED]);
        assert_eq!(conversion, RgbaConversion { rounded: 1, black: 1 });
        let error = GumpColors::from_rgba(2, 1, &[0, 0, 0, 128, 0, 0, 0, 255]).unwrap_err().to_string();
        assert!(error.contains("x 0, y 0 (alpha 128)"), "{error}");
        assert!(GumpColors::from_rgba(1, 1, &[0; 3]).is_err());
        assert!(GumpColors::decode_rows(&[1, 0, 0, 0, 0, 0, 2, 0], 1, 1).is_err(), "a run past the width");
    }
}
