//! The three BC7 modes the mobile animation pages use (4, 5 and 6: one subset each), as fields: unpacked from a
//! block, packed back bit for bit, decoded as the format reference decodes them, and re-solved for given pixels
//! (indices picked for fixed endpoints, or endpoints fitted to fixed indices). The lossy conditioning experiments
//! (`lossy.rs`) edit blocks through it; the tests check the round trip and the decode against `udd_assets::bc7`.

/// Texels of a block.
pub const TEXELS: usize = 16;
const CHANNELS: usize = 4;
const ALPHA: usize = 3;
const BLOCK_BITS: u32 = 128;
/// The interpolation weights of 2-, 3- and 4-bit indices, out of `WEIGHT_ONE`.
const WEIGHTS_2: [u32; 4] = [0, 21, 43, 64];
const WEIGHTS_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const WEIGHT_ONE: u32 = 64;
const WEIGHT_ROUND: u32 = 32;
const WEIGHT_SHIFT: u32 = 6;
pub const MODE_4: u8 = 4;
pub const MODE_5: u8 = 5;
pub const MODE_6: u8 = 6;
/// Endpoint value bits of the colour and alpha channels, per mode 4, 5, 6.
const COLOUR_BITS: [u32; 3] = [5, 7, 7];
const ALPHA_BITS: [u32; 3] = [6, 8, 7];
const ROTATION_BITS: u32 = 2;

pub type Rgba = [u8; CHANNELS];

/// One block of mode 4, 5 or 6 as its fields. Endpoint values are as stored (before the p-bit and the expansion to
/// 8 bits); the indices are one per texel, the anchor texel's included. Mode 6 has one index set, `colour_index`,
/// for all four channels, and its alpha endpoints share the p-bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub mode: u8,
    /// Modes 4 and 5: the channel swapped with alpha (0 none, 1 red, 2 green, 3 blue).
    pub rotation: u8,
    /// Mode 4: 1 when the colour takes the 3-bit indices.
    pub index_mode: u8,
    pub colour: [[u8; 3]; 2],
    pub alpha: [u8; 2],
    pub pbits: [u8; 2],
    pub colour_index: [u8; TEXELS],
    pub alpha_index: [u8; TEXELS],
}

struct Reader {
    bits: u128,
    at: u32,
}

impl Reader {
    fn take(&mut self, count: u32) -> u8 {
        let value = ((self.bits >> self.at) & ((1u128 << count) - 1)) as u8;
        self.at += count;
        value
    }
}

struct Writer {
    bits: u128,
    at: u32,
}

impl Writer {
    fn put(&mut self, value: u8, count: u32) {
        debug_assert!(u32::from(value) < (1 << count), "{value} does not fit {count} bits");
        self.bits |= u128::from(value) << self.at;
        self.at += count;
    }
}

fn slot(mode: u8) -> usize {
    usize::from(mode - MODE_4)
}

fn expand(value: u8, bits: u32) -> u8 {
    if bits >= 8 {
        return value;
    }
    let shifted = u32::from(value) << (8 - bits);
    (shifted | (shifted >> bits)) as u8
}

fn weights(bits: u32) -> &'static [u32] {
    match bits {
        2 => &WEIGHTS_2,
        3 => &WEIGHTS_3,
        _ => &WEIGHTS_4,
    }
}

fn interpolate(low: u8, high: u8, weight: u32) -> u8 {
    (((WEIGHT_ONE - weight) * u32::from(low) + weight * u32::from(high) + WEIGHT_ROUND) >> WEIGHT_SHIFT) as u8
}

fn rotate(pixel: &mut Rgba, rotation: u8) {
    if rotation > 0 {
        pixel.swap(usize::from(rotation - 1), ALPHA);
    }
}

impl Block {
    /// The mode of a block's first byte: its lowest set bit.
    pub fn mode_of(bytes: &[u8; 16]) -> Option<u8> {
        (bytes[0] != 0).then(|| bytes[0].trailing_zeros() as u8)
    }

    /// The index bits of the colour and the alpha set.
    pub fn index_bits(&self) -> (u32, u32) {
        match self.mode {
            MODE_4 if self.index_mode == 0 => (2, 3),
            MODE_4 => (3, 2),
            MODE_5 => (2, 2),
            _ => (4, 0),
        }
    }

    fn colour_bits(&self) -> u32 {
        COLOUR_BITS[slot(self.mode)]
    }

    fn alpha_bits(&self) -> u32 {
        ALPHA_BITS[slot(self.mode)]
    }

    pub fn unpack(bytes: &[u8; 16]) -> Option<Self> {
        let mode = Self::mode_of(bytes)?;
        if !(MODE_4..=MODE_6).contains(&mode) {
            return None;
        }
        let mut reader = Reader { bits: u128::from_le_bytes(*bytes), at: u32::from(mode) + 1 };
        let mut block = Self { mode, rotation: 0, index_mode: 0, colour: [[0; 3]; 2], alpha: [0; 2], pbits: [0; 2],
            colour_index: [0; TEXELS], alpha_index: [0; TEXELS] };
        if mode != MODE_6 {
            block.rotation = reader.take(ROTATION_BITS);
        }
        if mode == MODE_4 {
            block.index_mode = reader.take(1);
        }
        let (colour_bits, alpha_bits) = (block.colour_bits(), block.alpha_bits());
        for channel in 0..3 {
            for endpoint in 0..2 {
                block.colour[endpoint][channel] = reader.take(colour_bits);
            }
        }
        for endpoint in 0..2 {
            block.alpha[endpoint] = reader.take(alpha_bits);
        }
        if mode == MODE_6 {
            block.pbits = [reader.take(1), reader.take(1)];
        }
        let (colour_index_bits, alpha_index_bits) = block.index_bits();
        // Mode 4 stores the 2-bit set first, whichever channels it serves.
        let alpha_first = mode == MODE_4 && block.index_mode == 1;
        let read_set = |reader: &mut Reader, set: &mut [u8; TEXELS], bits: u32| {
            for (texel, index) in set.iter_mut().enumerate() {
                *index = reader.take(if texel == 0 { bits - 1 } else { bits });
            }
        };
        if alpha_first {
            read_set(&mut reader, &mut block.alpha_index, alpha_index_bits);
            read_set(&mut reader, &mut block.colour_index, colour_index_bits);
        } else {
            read_set(&mut reader, &mut block.colour_index, colour_index_bits);
            if alpha_index_bits > 0 {
                read_set(&mut reader, &mut block.alpha_index, alpha_index_bits);
            }
        }
        debug_assert_eq!(reader.at, BLOCK_BITS);
        Some(block)
    }

    /// The block's bytes; the anchor indices must have their top bit clear (see [`Self::fix_anchors`]).
    pub fn pack(&self) -> [u8; 16] {
        let mut writer = Writer { bits: 1u128 << self.mode, at: u32::from(self.mode) + 1 };
        if self.mode != MODE_6 {
            writer.put(self.rotation, ROTATION_BITS);
        }
        if self.mode == MODE_4 {
            writer.put(self.index_mode, 1);
        }
        let (colour_bits, alpha_bits) = (self.colour_bits(), self.alpha_bits());
        for channel in 0..3 {
            for endpoint in 0..2 {
                writer.put(self.colour[endpoint][channel], colour_bits);
            }
        }
        for endpoint in 0..2 {
            writer.put(self.alpha[endpoint], alpha_bits);
        }
        if self.mode == MODE_6 {
            writer.put(self.pbits[0], 1);
            writer.put(self.pbits[1], 1);
        }
        let (colour_index_bits, alpha_index_bits) = self.index_bits();
        let write_set = |writer: &mut Writer, set: &[u8; TEXELS], bits: u32| {
            for (texel, &index) in set.iter().enumerate() {
                writer.put(index, if texel == 0 { bits - 1 } else { bits });
            }
        };
        if self.mode == MODE_4 && self.index_mode == 1 {
            write_set(&mut writer, &self.alpha_index, alpha_index_bits);
            write_set(&mut writer, &self.colour_index, colour_index_bits);
        } else {
            write_set(&mut writer, &self.colour_index, colour_index_bits);
            if alpha_index_bits > 0 {
                write_set(&mut writer, &self.alpha_index, alpha_index_bits);
            }
        }
        debug_assert_eq!(writer.at, BLOCK_BITS);
        writer.bits.to_le_bytes()
    }

    /// The 8-bit endpoints: colour then alpha, per endpoint, before the rotation.
    fn endpoints(&self) -> [Rgba; 2] {
        let (colour_bits, alpha_bits) = (self.colour_bits(), self.alpha_bits());
        std::array::from_fn(|endpoint| {
            if self.mode == MODE_6 {
                let p = self.pbits[endpoint];
                let c = self.colour[endpoint];
                [c[0] << 1 | p, c[1] << 1 | p, c[2] << 1 | p, self.alpha[endpoint] << 1 | p]
            } else {
                let c = self.colour[endpoint];
                [expand(c[0], colour_bits), expand(c[1], colour_bits), expand(c[2], colour_bits),
                    expand(self.alpha[endpoint], alpha_bits)]
            }
        })
    }

    pub fn decode(&self) -> [Rgba; TEXELS] {
        let [low, high] = self.endpoints();
        let (colour_index_bits, alpha_index_bits) = self.index_bits();
        let (colour_weights, alpha_weights) = (weights(colour_index_bits), weights(alpha_index_bits.max(2)));
        std::array::from_fn(|texel| {
            let colour_weight = colour_weights[usize::from(self.colour_index[texel])];
            let alpha_weight = if self.mode == MODE_6 { colour_weight } else {
                alpha_weights[usize::from(self.alpha_index[texel])]
            };
            let mut pixel = [
                interpolate(low[0], high[0], colour_weight),
                interpolate(low[1], high[1], colour_weight),
                interpolate(low[2], high[2], colour_weight),
                interpolate(low[ALPHA], high[ALPHA], alpha_weight),
            ];
            rotate(&mut pixel, self.rotation);
            pixel
        })
    }

    /// Swaps a set's endpoints and inverts its indices where the anchor index has its top bit set, which the format
    /// cannot store; the decode is unchanged.
    pub fn fix_anchors(&mut self) {
        let (colour_index_bits, alpha_index_bits) = self.index_bits();
        let colour_top = (1u8 << colour_index_bits) - 1;
        if self.colour_index[0] > colour_top / 2 {
            self.colour.swap(0, 1);
            self.colour_index.iter_mut().for_each(|index| *index = colour_top - *index);
            if self.mode == MODE_6 {
                self.alpha.swap(0, 1);
                self.pbits.swap(0, 1);
            }
        }
        if alpha_index_bits > 0 {
            let alpha_top = (1u8 << alpha_index_bits) - 1;
            if self.alpha_index[0] > alpha_top / 2 {
                self.alpha.swap(0, 1);
                self.alpha_index.iter_mut().for_each(|index| *index = alpha_top - *index);
            }
        }
    }

    /// Picks each texel's indices for the block's endpoints, nearest in squared error to `pixels` (the colour and
    /// alpha sets apart in modes 4 and 5, all four channels together in mode 6), then fixes the anchors.
    pub fn pick_indices(&mut self, pixels: &[Rgba; TEXELS]) {
        let [low, high] = self.endpoints();
        let (colour_index_bits, alpha_index_bits) = self.index_bits();
        let colour_weights = weights(colour_index_bits);
        let colour_channels = if self.mode == MODE_6 { CHANNELS } else { 3 };
        let palette: Vec<Rgba> = colour_weights.iter().map(|&weight| std::array::from_fn(|channel| {
            interpolate(low[channel], high[channel], weight)
        })).collect();
        for texel in 0..TEXELS {
            let mut pixel = pixels[texel];
            rotate(&mut pixel, self.rotation);
            let error = |entry: &Rgba, channels: std::ops::Range<usize>| -> u32 {
                channels.map(|channel| (i32::from(entry[channel]) - i32::from(pixel[channel])).unsigned_abs().pow(2)).sum()
            };
            self.colour_index[texel] = (0..palette.len()).min_by_key(|&at| error(&palette[at], 0..colour_channels))
                .unwrap_or(0) as u8;
            if alpha_index_bits > 0 {
                let alpha_weights = weights(alpha_index_bits);
                self.alpha_index[texel] = (0..alpha_weights.len()).min_by_key(|&at| {
                    (i32::from(interpolate(low[ALPHA], high[ALPHA], alpha_weights[at])) - i32::from(pixel[ALPHA]))
                        .unsigned_abs()
                }).unwrap_or(0) as u8;
            }
        }
        self.fix_anchors();
    }

    /// Fits the colour endpoints (and in mode 6 the alpha endpoints and p-bits) to `pixels` by least squares for the
    /// block's colour indices, rounds them to the mode's precision, then picks the indices again for the rounded
    /// endpoints. Modes 4 and 5 keep their alpha endpoints. Leaves the block as it is when its indices are all equal
    /// (no fit: the endpoints are then the pixels' mean).
    pub fn fit_endpoints(&mut self, pixels: &[Rgba; TEXELS]) {
        let (colour_index_bits, _) = self.index_bits();
        let colour_weights = weights(colour_index_bits);
        let channels = if self.mode == MODE_6 { CHANNELS } else { 3 };
        let rotated: [Rgba; TEXELS] = std::array::from_fn(|texel| {
            let mut pixel = pixels[texel];
            rotate(&mut pixel, self.rotation);
            pixel
        });
        // Normal equations of value = low (1 - t) + high t.
        let (mut aa, mut ab, mut bb) = (0f64, 0f64, 0f64);
        let mut ax = [0f64; CHANNELS];
        let mut bx = [0f64; CHANNELS];
        for texel in 0..TEXELS {
            let t = f64::from(colour_weights[usize::from(self.colour_index[texel])]) / f64::from(WEIGHT_ONE);
            let (a, b) = (1.0 - t, t);
            aa += a * a;
            ab += a * b;
            bb += b * b;
            for channel in 0..channels {
                let x = f64::from(rotated[texel][channel]);
                ax[channel] += a * x;
                bx[channel] += b * x;
            }
        }
        let determinant = aa * bb - ab * ab;
        if determinant.abs() < f64::EPSILON {
            return;
        }
        let fitted: [[f64; CHANNELS]; 2] = [
            std::array::from_fn(|channel| ((bb * ax[channel] - ab * bx[channel]) / determinant).clamp(0.0, 255.0)),
            std::array::from_fn(|channel| ((aa * bx[channel] - ab * ax[channel]) / determinant).clamp(0.0, 255.0)),
        ];
        let colour_bits = self.colour_bits();
        if self.mode == MODE_6 {
            // Each endpoint's p-bit: the one whose rounding is nearer.
            for endpoint in 0..2 {
                let best = (0..2u8).min_by(|&p, &q| {
                    let cost = |pbit: u8| -> f64 {
                        (0..CHANNELS).map(|channel| {
                            let value = quantize_with_pbit(fitted[endpoint][channel], pbit);
                            (f64::from(value << 1 | pbit) - fitted[endpoint][channel]).powi(2)
                        }).sum()
                    };
                    cost(p).total_cmp(&cost(q))
                }).unwrap_or(0);
                self.pbits[endpoint] = best;
                for channel in 0..3 {
                    self.colour[endpoint][channel] = quantize_with_pbit(fitted[endpoint][channel], best);
                }
                self.alpha[endpoint] = quantize_with_pbit(fitted[endpoint][ALPHA], best);
            }
        } else {
            for endpoint in 0..2 {
                for channel in 0..3 {
                    self.colour[endpoint][channel] = quantize(fitted[endpoint][channel], colour_bits);
                }
            }
        }
        self.pick_indices(pixels);
    }

    /// Rounds every colour endpoint value to a multiple of `1 << drop` of its stored precision, then picks the indices
    /// again. Alpha endpoints and p-bits keep their values, so the silhouette and coverage edges stay where they are.
    pub fn coarsen_endpoints(&mut self, drop: u32, pixels: &[Rgba; TEXELS]) {
        if drop == 0 {
            return;
        }
        let colour_bits = self.colour_bits();
        let snap = |value: u8, bits: u32| -> u8 {
            let step = 1u32 << drop.min(bits - 1);
            let top = (1u32 << bits) - 1;
            let rounded = ((u32::from(value) + step / 2) / step) * step;
            rounded.min(top - top % step) as u8
        };
        for endpoint in 0..2 {
            for channel in 0..3 {
                self.colour[endpoint][channel] = snap(self.colour[endpoint][channel], colour_bits);
            }
        }
        self.pick_indices(pixels);
    }
}

fn quantize(value: f64, bits: u32) -> u8 {
    // The stored value whose expansion is nearest.
    let top = (1u32 << bits) - 1;
    let guess = (value * f64::from(top) / 255.0).round() as i64;
    (guess - 1..=guess + 1).filter(|&v| v >= 0 && v <= i64::from(top))
        .min_by(|&p, &q| {
            let error = |v: i64| (f64::from(expand(v as u8, bits)) - value).abs();
            error(p).total_cmp(&error(q))
        })
        .unwrap_or(0) as u8
}

fn quantize_with_pbit(value: f64, pbit: u8) -> u8 {
    (((value - f64::from(pbit)) / 2.0).round()).clamp(0.0, 127.0) as u8
}

/// Squared error of a decoded block against its pixels, all four channels.
#[cfg(test)]
fn error(decoded: &[Rgba; TEXELS], pixels: &[Rgba; TEXELS]) -> u32 {
    decoded.iter().zip(pixels).map(|(a, b)| {
        (0..CHANNELS).map(|channel| (i32::from(a[channel]) - i32::from(b[channel])).unsigned_abs().pow(2)).sum::<u32>()
    }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One level of error per channel and texel, squared and summed, twice over.
    const FIT_TOLERANCE: u32 = 2 * (TEXELS * CHANNELS) as u32;

    fn sample_block(mode: u8, seed: u64) -> Block {
        let mut state = seed;
        let mut next = |bits: u32| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) & ((1 << bits) - 1)) as u8
        };
        let mut block = Block { mode, rotation: 0, index_mode: 0, colour: [[0; 3]; 2], alpha: [0; 2], pbits: [0; 2],
            colour_index: [0; TEXELS], alpha_index: [0; TEXELS] };
        if mode != MODE_6 {
            block.rotation = next(ROTATION_BITS);
        }
        if mode == MODE_4 {
            block.index_mode = next(1);
        }
        for endpoint in 0..2 {
            for channel in 0..3 {
                block.colour[endpoint][channel] = next(block.colour_bits());
            }
            block.alpha[endpoint] = next(block.alpha_bits());
        }
        if mode == MODE_6 {
            block.pbits = [next(1), next(1)];
        }
        let (colour_index_bits, alpha_index_bits) = block.index_bits();
        for texel in 0..TEXELS {
            block.colour_index[texel] = next(colour_index_bits);
            if alpha_index_bits > 0 {
                block.alpha_index[texel] = next(alpha_index_bits);
            }
        }
        block.fix_anchors();
        block
    }

    #[test]
    fn blocks_round_trip_and_decode_as_the_reference_does() {
        for mode in MODE_4..=MODE_6 {
            for seed in 0..500 {
                let block = sample_block(mode, seed);
                let bytes = block.pack();
                assert_eq!(Block::unpack(&bytes), Some(block), "mode {mode} seed {seed}");
                let extent = udd_assets::bc7::ImageExtent::new(4, 4).unwrap();
                let reference = udd_assets::bc7::decode_bc7_to_rgba8888(&bytes, extent).unwrap();
                let decoded: Vec<u8> = block.decode().iter().flatten().copied().collect();
                assert_eq!(decoded, reference, "mode {mode} seed {seed}");
            }
        }
    }

    #[test]
    fn fixing_anchors_keeps_the_decode() {
        for mode in MODE_4..=MODE_6 {
            for seed in 0..200 {
                let mut block = sample_block(mode, seed);
                block.colour_index[0] = (1 << block.index_bits().0) - 1;
                let before = block.decode();
                block.fix_anchors();
                assert_eq!(block.decode(), before);
                assert_eq!(Block::unpack(&block.pack()), Some(block));
            }
        }
    }

    #[test]
    fn picked_indices_never_lose_to_the_original_ones() {
        for mode in MODE_4..=MODE_6 {
            for seed in 0..200 {
                let block = sample_block(mode, seed);
                let pixels = block.decode();
                let mut picked = block;
                picked.pick_indices(&pixels);
                // Mode 6 picks all channels at once and modes 4 and 5 each set apart, so the error of each set can
                // only fall; ties may choose other indices.
                assert_eq!(error(&picked.decode(), &pixels), 0, "mode {mode} seed {seed}");
                // Fitting the endpoints to the block's own decode finds them again, up to the rounding of the
                // interpolated values.
                let mut fitted = block;
                fitted.fit_endpoints(&pixels);
                assert!(error(&fitted.decode(), &pixels) <= FIT_TOLERANCE, "mode {mode} seed {seed}");
            }
        }
    }
}
