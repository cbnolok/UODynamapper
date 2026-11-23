//! A frame's canvas upscaled in tiles, on a workspace smaller than the canvas: the network runs over windows of the
//! canvas each the workspace's size, and keeps of each only its core, which lies at least the network's reach
//! ([`UPSCALE_RECEPTIVE_RADIUS`]) inside every window edge that is not an edge of the canvas. Each texel kept then
//! reads the same inputs it would on the whole canvas, its zero padding at the canvas's edges included, so the cores
//! put together are the whole canvas's upscale: the same per-pixel arithmetic, so equal to it but for the rounding of
//! sums the units may order differently (the GPU test `tiles_stitch_into_the_whole_canvas` finds them equal).
//!
//! The caller sets each tile's frame with [`UpscaleWorkspace::set_frame_at`](super::UpscaleWorkspace::set_frame_at)
//! at [`UpscaleTile::frame_origin`], runs the frame's dispatches, and copies the core's part of the 2x output
//! ([`UpscaleTile::output_core`]) to where the core lies at 2x on the frame's canvas. The tiles may cover a region of
//! the canvas only, a part of the frame stored apart.

use super::{UPSCALE_BORDER, UPSCALE_OUTPUT_SCALE};

/// The network's reach: the canvas pixels on each side of a pixel its output reads, one per 3x3 convolution on the way
/// from the input to the upsampler (conv_1, the three of each SPAB block, conv_2 and the upsampler; conv_cat is
/// pointwise, the SPAB attention per pixel). The alpha's bicubic reaches 2 pixels, within it.
pub const UPSCALE_RECEPTIVE_RADIUS: u32 = super::CONV_SIDE_LAYERS * (super::CONV_SIDE / 2);
const BICUBIC_REACH: u32 = 2;
const _: () = assert!(BICUBIC_REACH <= UPSCALE_RECEPTIVE_RADIUS);

/// The smallest workspace side that tiles: a core of a pixel between two windows' margins of
/// [`UPSCALE_RECEPTIVE_RADIUS`].
pub const UPSCALE_MIN_TILE_SIDE: u32 = 2 * UPSCALE_RECEPTIVE_RADIUS + 1;

/// A span of canvas pixels, `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSpan {
    pub start: u32,
    pub end: u32,
}

impl TileSpan {
    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// One pass of the network: a window of the canvas the workspace's size, and the part of it kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpscaleTile {
    /// The window's top left on the frame's canvas, whose frame starts at ([`UPSCALE_BORDER`], [`UPSCALE_BORDER`]).
    pub window: [u32; 2],
    /// The pixels kept, on the frame's canvas, per axis.
    pub core: [TileSpan; 2],
}

impl UpscaleTile {
    /// Where the frame's top left lies on the workspace's canvas for this window: what
    /// [`UpscaleWorkspace::set_frame_at`](super::UpscaleWorkspace::set_frame_at) takes. Negative when the window
    /// starts inside the frame.
    pub fn frame_origin(&self) -> [i32; 2] {
        self.window.map(|start| UPSCALE_BORDER as i32 - start as i32)
    }

    /// The core in the workspace's 2x output: its top left texel and size. At 2x on the frame's canvas it starts at
    /// twice the core's start.
    pub fn output_core(&self) -> ([u32; 2], [u32; 2]) {
        let origin = [0, 1].map(|axis| (self.core[axis].start - self.window[axis]) * UPSCALE_OUTPUT_SCALE);
        (origin, self.core.map(|span| span.len() * UPSCALE_OUTPUT_SCALE))
    }
}

/// The tiles that upscale `region` (per axis) of a frame's canvas of `canvas` pixels on a workspace of `tile` pixels,
/// the fewest along each axis, row by row: their cores cover the region once. An axis the workspace spans whole
/// takes one window from the canvas's start, the canvas past its end transparent as it is past the border (a frame's
/// canvas rounded up). Errs when the region is empty or leaves the canvas, or a side to tile is under
/// [`UPSCALE_MIN_TILE_SIDE`].
pub fn upscale_tiles(canvas: [u32; 2], tile: [u32; 2], region: [TileSpan; 2]) -> Result<Vec<UpscaleTile>, String> {
    let mut axes = [Vec::new(), Vec::new()];
    for axis in 0..2 {
        if region[axis].is_empty() || region[axis].end > canvas[axis] {
            return Err(format!("upscale tiles: the region {region:?} is empty or leaves the canvas {canvas:?}"));
        }
        if canvas[axis] > tile[axis] && tile[axis] < UPSCALE_MIN_TILE_SIDE {
            return Err(format!("upscale tiles: a {tile:?} workspace is under the {UPSCALE_MIN_TILE_SIDE} pixels a \
                side that tiles"));
        }
        axes[axis] = axis_tiles(canvas[axis], tile[axis], region[axis]);
    }
    Ok(axes[1].iter().flat_map(|&(window_y, core_y)| axes[0].iter().map(move |&(window_x, core_x)| UpscaleTile {
        window: [window_x, window_y],
        core: [core_x, core_y],
    })).collect())
}

/// The windows' starts and cores along one axis: each window starts [`UPSCALE_RECEPTIVE_RADIUS`] before its core (or
/// at the canvas's start, or as late as it can before the canvas's end), and its core reaches that far inside the
/// window's far edge, or to the region's end, or where the window meets the canvas's end.
fn axis_tiles(canvas: u32, tile: u32, region: TileSpan) -> Vec<(u32, TileSpan)> {
    if canvas <= tile {
        return vec![(0, region)];
    }
    let mut tiles = Vec::new();
    let mut core_start = region.start;
    while core_start < region.end {
        let window = core_start.saturating_sub(UPSCALE_RECEPTIVE_RADIUS).min(canvas - tile);
        let core_end = if window + tile >= canvas {
            region.end
        } else {
            (window + tile - UPSCALE_RECEPTIVE_RADIUS).min(region.end)
        };
        tiles.push((window, TileSpan { start: core_start, end: core_end }));
        core_start = core_end;
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILE: u32 = 256;

    fn span(start: u32, end: u32) -> TileSpan {
        TileSpan { start, end }
    }

    /// Checks the cores cover the region once, in order, each window on the canvas, each core inside its window by
    /// the reach wherever the window's edge is not the canvas's.
    fn check_axis(canvas: u32, tile: u32, region: TileSpan) -> usize {
        let tiles = axis_tiles(canvas, tile, region);
        let mut next = region.start;
        for &(window, core) in &tiles {
            assert_eq!(core.start, next, "{tiles:?}");
            assert!(!core.is_empty(), "{tiles:?}");
            assert!(window + tile.min(canvas) <= canvas, "{tiles:?}");
            assert!(window <= core.start && core.end <= window + tile, "{tiles:?}");
            if window > 0 {
                assert!(core.start >= window + UPSCALE_RECEPTIVE_RADIUS, "{tiles:?}");
            }
            if window + tile < canvas {
                assert!(core.end + UPSCALE_RECEPTIVE_RADIUS <= window + tile, "{tiles:?}");
            }
            next = core.end;
        }
        assert_eq!(next, region.end, "{tiles:?}");
        tiles.len()
    }

    #[test]
    fn the_reach_counts_the_networks_3x3_convolutions() {
        // conv_1, 6 blocks of 3, conv_2 and the upsampler.
        assert_eq!(UPSCALE_RECEPTIVE_RADIUS, 21);
    }

    #[test]
    fn a_canvas_the_workspace_holds_is_one_tile() {
        assert_eq!(upscale_tiles([200, 100], [TILE; 2], [span(0, 196), span(0, 97)]).unwrap(), vec![UpscaleTile {
            window: [0, 0], core: [span(0, 196), span(0, 97)],
        }]);
    }

    #[test]
    fn cores_cover_the_region_once_inside_their_windows() {
        // The largest frames (640x480 with their border) and the sizes around a tile.
        for (canvas, end) in [(644, 644), (484, 484), (512, 508), (TILE + 8, TILE + 1), (TILE + 8, TILE + 8),
            (2 * TILE, 2 * TILE - 3), (1000, 999)] {
            check_axis(canvas, TILE, span(0, end));
        }
        for tile in [UPSCALE_MIN_TILE_SIDE, UPSCALE_MIN_TILE_SIDE + 1, 64, 65] {
            for canvas in tile + 1..tile * 4 {
                for region in [span(0, canvas), span(0, canvas - 1), span(canvas / 3, canvas),
                    span(1, canvas / 2 + 1), span(canvas - 1, canvas)] {
                    check_axis(canvas, tile, region);
                }
            }
        }
    }

    #[test]
    fn the_largest_frames_take_three_by_three_tiles() {
        // A 640x480 frame: a 644x484 canvas with its border. The first core ends at 235, a middle one is 214 long, and
        // the last window keeps from 449 (644 wide) or 253 (484 high) on: three each way.
        assert_eq!(check_axis(644, TILE, span(0, 644)), 3);
        assert_eq!(check_axis(484, TILE, span(0, 484)), 3);
        assert_eq!(upscale_tiles([644, 484], [TILE; 2], [span(0, 644), span(0, 484)]).unwrap().len(), 9);
        // A Classic dragon's widest frame, 508x317: 3 by 2, or 2 by 2 for its left half.
        assert_eq!(upscale_tiles([512, 321], [TILE; 2], [span(0, 512), span(0, 321)]).unwrap().len(), 6);
        assert_eq!(upscale_tiles([512, 321], [TILE; 2], [span(0, 256), span(0, 321)]).unwrap().len(), 4);
    }

    #[test]
    fn a_tile_knows_its_frame_and_its_output() {
        let tile = UpscaleTile { window: [213, 0], core: [span(234, 448), span(0, 50)] };
        assert_eq!(tile.frame_origin(), [UPSCALE_BORDER as i32 - 213, UPSCALE_BORDER as i32]);
        assert_eq!(tile.output_core(), ([42, 0], [428, 100]));
    }

    #[test]
    fn too_small_a_workspace_or_a_region_off_the_canvas_is_refused() {
        let small = UPSCALE_MIN_TILE_SIDE - 1;
        assert!(upscale_tiles([300, 40], [small, 64], [span(0, 300), span(0, 40)]).is_err());
        // An axis the workspace spans whole needs no margin.
        assert!(upscale_tiles([40, 300], [small, 300], [span(0, 40), span(0, 300)]).is_ok());
        assert!(upscale_tiles([300, 40], [TILE; 2], [span(0, 301), span(0, 40)]).is_err());
        assert!(upscale_tiles([300, 40], [TILE; 2], [span(9, 9), span(0, 40)]).is_err());
    }
}
