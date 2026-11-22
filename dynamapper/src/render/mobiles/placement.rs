//! Where a mobile's frame goes: an upright card like a static's, anchored at the near vertex of its cell
//! (`apply_cc_static_world_anchor_translation`), with the frame's anchor on the cell's centre as the Classic client
//! draws a mobile (ClassicUO `MobileView`: the anchor at the tile's top-left plus 22, 22).

use crate::render::statics::static_placement::{StaticBillboardBounds, CC_TILE_PIXEL_WIDTH, CC_WORLD_XZ_PER_PIXEL,
    CC_WORLD_Y_PER_PIXEL};

/// The pixels from a cell's near vertex up to its centre on screen: half the tile's 44-pixel diamond.
pub(crate) const CELL_CENTRE_ABOVE_NEAR_VERTEX_PX: f32 = CC_TILE_PIXEL_WIDTH * 0.5;

/// A frame as placed on screen: its size and its anchor, in pixels from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlacement {
    pub width: f32,
    pub height: f32,
    pub anchor_x: f32,
    pub anchor_y: f32,
}

/// The card of `frame` for a mobile at a cell's near vertex, its anchor moved by `offset_px` (x right, y down, as
/// screen pixels; a rider's offset on its mount). A mirrored frame turns about its anchor: its pixel x lands at
/// `anchor - 1 - x`, as the clients mirror the stored directions. Mirrored, the card's x bounds come in reverse, which
/// flips the image on the card without flipping its coordinates in the atlas.
pub(crate) fn mobile_card_bounds(frame: FramePlacement, mirrored: bool, offset_px: (f32, f32), pixel_ratio: f32)
    -> StaticBillboardBounds {
    let xz_per_px = CC_WORLD_XZ_PER_PIXEL * pixel_ratio;
    let y_per_px = CC_WORLD_Y_PER_PIXEL * pixel_ratio;
    let (offset_x, offset_y) = (offset_px.0 / pixel_ratio, offset_px.1 / pixel_ratio);
    let (left, right) = if mirrored {
        (frame.anchor_x - frame.width, frame.anchor_x)
    } else {
        (-frame.anchor_x, frame.width - frame.anchor_x)
    };
    let centre_up = CELL_CENTRE_ABOVE_NEAR_VERTEX_PX / pixel_ratio - offset_y;
    let top = centre_up + frame.anchor_y;
    let (near_x, far_x) = if mirrored { (right, left) } else { (left, right) };
    StaticBillboardBounds {
        local_min_x: (near_x + offset_x) * xz_per_px,
        local_max_x: (far_x + offset_x) * xz_per_px,
        local_min_y: (top - frame.height) * y_per_px,
        local_max_y: top * y_per_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: FramePlacement = FramePlacement { width: 30.0, height: 60.0, anchor_x: 10.0, anchor_y: 55.0 };

    fn in_pixels(bounds: StaticBillboardBounds) -> [f32; 4] {
        [bounds.local_min_x / CC_WORLD_XZ_PER_PIXEL, bounds.local_max_x / CC_WORLD_XZ_PER_PIXEL,
            bounds.local_min_y / CC_WORLD_Y_PER_PIXEL, bounds.local_max_y / CC_WORLD_Y_PER_PIXEL]
            .map(|value| (value * 1000.0).round() / 1000.0)
    }

    #[test]
    fn the_anchor_sits_on_the_cell_centre() {
        // Left of the anchor by its x; the top 55 pixels over the centre, 22 over the near vertex.
        assert_eq!(in_pixels(mobile_card_bounds(FRAME, false, (0.0, 0.0), 1.0)), [-10.0, 20.0, 17.0, 77.0]);
    }

    #[test]
    fn a_mirrored_frame_turns_about_its_anchor() {
        // Pixel x lands at 9 - x: the frame spans -20..10, its first column on the right.
        assert_eq!(in_pixels(mobile_card_bounds(FRAME, true, (0.0, 0.0), 1.0)), [10.0, -20.0, 17.0, 77.0]);
    }

    #[test]
    fn an_offset_moves_the_anchor_right_and_down() {
        assert_eq!(in_pixels(mobile_card_bounds(FRAME, false, (3.0, 5.0), 1.0)), [-7.0, 23.0, 12.0, 72.0]);
    }
}
