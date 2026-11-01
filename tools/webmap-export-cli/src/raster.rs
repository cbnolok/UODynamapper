//! RGB8 images: cropping a spooled frame, cutting tiles and the 2 × 2 sRGB box downsample.

pub const RGB_CHANNELS: usize = 3;
const RGBA_CHANNELS: usize = 4;
/// Pixels averaged per output pixel by [`Rgb::downsample_half`], and the rounding added before dividing.
const BOX_PIXELS: u16 = 4;
const BOX_ROUNDING: u16 = BOX_PIXELS / 2;

/// A tightly packed RGB8 image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgb {
    pub fn black(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; width as usize * height as usize * RGB_CHANNELS] }
    }

    /// The `size × size` square at `(left, top)` of an RGBA8 image `width` pixels wide, alpha dropped.
    pub fn from_rgba_crop(rgba: &[u8], width: u32, left: u32, top: u32, size: u32) -> Self {
        let mut pixels = Vec::with_capacity(size as usize * size as usize * RGB_CHANNELS);
        for row in top..top + size {
            let start = (row as usize * width as usize + left as usize) * RGBA_CHANNELS;
            let line = &rgba[start..start + size as usize * RGBA_CHANNELS];
            pixels.extend(line.chunks_exact(RGBA_CHANNELS).flat_map(|p| [p[0], p[1], p[2]]));
        }
        Self { width: size, height: size, pixels }
    }

    /// The `size × size` square at `(left, top)`.
    pub fn crop(&self, left: u32, top: u32, size: u32) -> Self {
        let mut pixels = Vec::with_capacity(size as usize * size as usize * RGB_CHANNELS);
        for row in top..top + size {
            let start = (row as usize * self.width as usize + left as usize) * RGB_CHANNELS;
            pixels.extend_from_slice(&self.pixels[start..start + size as usize * RGB_CHANNELS]);
        }
        Self { width: size, height: size, pixels }
    }

    /// Copies `tile` into this image at `(left, top)`.
    pub fn paste(&mut self, tile: &Rgb, left: u32, top: u32) {
        let row_bytes = tile.width as usize * RGB_CHANNELS;
        for row in 0..tile.height as usize {
            let dst = ((top as usize + row) * self.width as usize + left as usize) * RGB_CHANNELS;
            self.pixels[dst..dst + row_bytes].copy_from_slice(&tile.pixels[row * row_bytes..(row + 1) * row_bytes]);
        }
    }

    /// Half the size: each output pixel is the rounded average of a 2 × 2 block, per sRGB channel (the map-tile
    /// convention, the user's choice).
    pub fn downsample_half(&self) -> Self {
        let (width, height) = (self.width / 2, self.height / 2);
        let stride = self.width as usize * RGB_CHANNELS;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * RGB_CHANNELS);
        for y in 0..height as usize {
            let (row0, row1) = (2 * y * stride, (2 * y + 1) * stride);
            for x in 0..width as usize {
                let col = 2 * x * RGB_CHANNELS;
                for c in 0..RGB_CHANNELS {
                    let sum = u16::from(self.pixels[row0 + col + c])
                        + u16::from(self.pixels[row0 + col + RGB_CHANNELS + c])
                        + u16::from(self.pixels[row1 + col + c])
                        + u16::from(self.pixels[row1 + col + RGB_CHANNELS + c]);
                    pixels.push(((sum + BOX_ROUNDING) / BOX_PIXELS) as u8);
                }
            }
        }
        Self { width, height, pixels }
    }

    /// All pixels are black: the area outside the facet and the black voids inside it. Such tiles are not
    /// written; the viewer shows its shared black tile for them.
    pub fn is_black(&self) -> bool {
        self.pixels.iter().all(|&v| v == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_rounds_the_block_average() {
        let image = Rgb { width: 2, height: 2, pixels: vec![0, 0, 255, 1, 0, 255, 1, 0, 255, 1, 3, 254] };
        assert_eq!(image.downsample_half().pixels, vec![1, 1, 255]);
    }

    #[test]
    fn crop_and_paste_are_inverse() {
        let image = Rgb { width: 4, height: 4, pixels: (0..48).collect() };
        let tile = image.crop(2, 2, 2);
        assert_eq!(tile.pixels, vec![30, 31, 32, 33, 34, 35, 42, 43, 44, 45, 46, 47]);
        let mut back = Rgb::black(4, 4);
        back.paste(&tile, 2, 2);
        assert_eq!(back.crop(2, 2, 2), tile);
        assert!(!back.is_black() && Rgb::black(2, 2).is_black());
    }

    #[test]
    fn rgba_crop_drops_alpha() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|v| v as u8).collect();
        let image = Rgb::from_rgba_crop(&rgba, 3, 1, 1, 2);
        assert_eq!(image.pixels, vec![16, 17, 18, 20, 21, 22, 28, 29, 30, 32, 33, 34]);
    }
}
