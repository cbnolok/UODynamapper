//! Premultiplied colour for block formats (the user's rule, 2026-11-05: block formats store premultiplied colour, so a
//! filtered edge doesn't darken), and back to straight colour for consumers that key or blend on it.
//!
//! Both directions work per channel with one formula, so a texel whose R, G and B are equal stays so: a grey test
//! on the straight colour still passes after a round trip.

const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;
const MAX_CHANNEL: u32 = u8::MAX as u32;
/// Added before dividing by 255, so the product rounds to the nearest value (255 is odd, so there is no tie).
const ROUND_HALF_UNDER_MAX: u32 = MAX_CHANNEL / 2;

/// Multiplies each texel's colour by its alpha, rounding to the nearest value.
pub fn premultiply_rgba8888(rgba: &mut [u8]) {
    for texel in rgba.chunks_exact_mut(RGBA_CHANNELS) {
        let alpha = u32::from(texel[ALPHA_CHANNEL]);
        for channel in &mut texel[..ALPHA_CHANNEL] {
            *channel = ((u32::from(*channel) * alpha + ROUND_HALF_UNDER_MAX) / MAX_CHANNEL) as u8;
        }
    }
}

/// Divides each texel's colour by its alpha, rounding to the nearest value. A transparent texel becomes (0, 0, 0, 0);
/// colour above its alpha, which a lossy block can decode to, saturates at 255.
pub fn unpremultiply_rgba8888(rgba: &mut [u8]) {
    for texel in rgba.chunks_exact_mut(RGBA_CHANNELS) {
        let alpha = u32::from(texel[ALPHA_CHANNEL]);
        if alpha == 0 {
            texel[..ALPHA_CHANNEL].fill(0);
            continue;
        }
        for channel in &mut texel[..ALPHA_CHANNEL] {
            *channel = ((u32::from(*channel) * MAX_CHANNEL + alpha / 2) / alpha).min(MAX_CHANNEL) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every grey at every alpha stays a grey through premultiplying and back, and opaque texels don't change.
    #[test]
    fn exact_greys_stay_greys() {
        for alpha in 0..=u8::MAX {
            for grey in 0..=u8::MAX {
                let mut texel = [grey, grey, grey, alpha];
                premultiply_rgba8888(&mut texel);
                assert!(texel[0] == texel[1] && texel[0] == texel[2], "premultiplied grey {grey} at alpha {alpha}");
                unpremultiply_rgba8888(&mut texel);
                assert!(texel[0] == texel[1] && texel[0] == texel[2], "straight grey {grey} at alpha {alpha}");
                assert_eq!(texel[ALPHA_CHANNEL], alpha);
                if alpha == u8::MAX {
                    assert_eq!(texel, [grey, grey, grey, alpha]);
                }
            }
        }
    }

    /// Straight colour from premultiplied colour premultiplies back to the same bytes, so an upload that premultiplies
    /// again (egui's `from_rgba_unmultiplied`) loses nothing.
    #[test]
    fn premultiplied_colour_survives_a_round_trip_through_straight_colour() {
        for alpha in 1..=u8::MAX {
            for premultiplied in 0..=alpha {
                let mut texel = [premultiplied, premultiplied, premultiplied, alpha];
                unpremultiply_rgba8888(&mut texel);
                premultiply_rgba8888(&mut texel);
                assert_eq!(texel, [premultiplied, premultiplied, premultiplied, alpha], "alpha {alpha}");
            }
        }
    }

    #[test]
    fn transparent_texels_and_colour_above_alpha() {
        const ALPHA: u8 = 100;
        let mut texels = [7, 8, 9, 0, 200, 50, 0, ALPHA];
        unpremultiply_rgba8888(&mut texels);
        assert_eq!(texels, [0, 0, 0, 0, u8::MAX, 128, 0, ALPHA]);
    }
}
