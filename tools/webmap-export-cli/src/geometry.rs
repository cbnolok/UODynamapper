//! The isometric projection at client scale, the tile pyramid and the frame grid.
//!
//! Base-level pixels: the top vertex of map tile `(x, y)` at height `z` is at
//! `px = (x − y + H) · 22`, `py = (x + y) · 22 − 4·z + z_top`, for a facet of `W × H` tiles. The tile's diamond
//! spans `[px − 22, px + 22)` horizontally and `[py, py + 44)` vertically. The bounding box starts at 0 on both
//! axes; Leaflet's pixel coordinates at the base zoom are these pixels.

/// Half the width of a tile diamond at client scale (44 px).
pub const TILE_HALF_PX: i64 = 22;
/// Screen pixels per unit of height.
pub const Z_STEP_PX: i64 = 4;
/// The highest and lowest map and statics heights.
pub const Z_MAX: i64 = 127;
pub const Z_MIN: i64 = -128;
/// The top and bottom margins are rounded up to this.
pub const MARGIN_ALIGN_PX: i64 = 64;

/// The exported tile size (the user's choice, 2026-11-01).
pub const TILE_SIZE_PX: u32 = 512;
/// Base tiles kept per frame side: a frame then also yields whole tiles at three lower levels.
pub const FRAME_KEEP_TILES: u32 = 8;
/// Pixels rendered around the kept area and cropped, for screen-space effects at the frame edge.
pub const FRAME_MARGIN_PX: u32 = 32;

/// Tile size, kept tiles per frame and frame margin. The exporter uses [`TileGrid::EXPORT`]; tests use smaller
/// grids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGrid {
    pub tile_size_px: u32,
    pub frame_keep_tiles: u32,
    pub frame_margin_px: u32,
}

impl TileGrid {
    pub const EXPORT: Self = Self {
        tile_size_px: TILE_SIZE_PX,
        frame_keep_tiles: FRAME_KEEP_TILES,
        frame_margin_px: FRAME_MARGIN_PX,
    };

    /// Kept pixels per frame side.
    pub const fn frame_keep_px(self) -> u32 {
        self.tile_size_px * self.frame_keep_tiles
    }

    /// Rendered pixels per frame side.
    pub const fn frame_size_px(self) -> u32 {
        self.frame_keep_px() + 2 * self.frame_margin_px
    }

    /// Levels cut directly from a frame: the base level and every level down to one tile per frame.
    pub fn frame_levels(self) -> u32 {
        self.frame_keep_tiles.ilog2() + 1
    }

    pub fn validate(self) -> Result<(), String> {
        if self.tile_size_px == 0 || !self.tile_size_px.is_power_of_two() {
            return Err(format!("tile size {} is not a power of two", self.tile_size_px));
        }
        if self.frame_keep_tiles == 0 || !self.frame_keep_tiles.is_power_of_two() {
            return Err(format!("kept tiles per frame {} is not a power of two", self.frame_keep_tiles));
        }
        Ok(())
    }
}

fn align_up(value: i64, align: i64) -> i64 {
    (value + align - 1).div_euclid(align) * align
}

/// One frame of the grid: it keeps base-level pixels `[i·K, (i+1)·K) × [j·K, (j+1)·K)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameIndex {
    pub i: u32,
    pub j: u32,
}

impl FrameIndex {
    pub fn id(self) -> String {
        format!("f{:04}_{:04}", self.i, self.j)
    }
}

/// A rectangle in base-level pixels, `[left, left + width) × [top, top + height)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
}

/// A facet's projection and pyramid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width_tiles: u32,
    pub height_tiles: u32,
    pub z_top_px: u32,
    pub z_bottom_px: u32,
    pub grid: TileGrid,
}

impl Layout {
    /// `max_sprite_height_px`: the tallest static sprite, which can rise above its tile's top vertex.
    pub fn new(width_tiles: u32, height_tiles: u32, max_sprite_height_px: u32, grid: TileGrid) -> Result<Self, String> {
        grid.validate()?;
        if width_tiles == 0 || height_tiles == 0 {
            return Err(format!("facet size {width_tiles}x{height_tiles} is empty"));
        }
        let z_top = align_up(Z_STEP_PX * Z_MAX + i64::from(max_sprite_height_px), MARGIN_ALIGN_PX);
        let z_bottom = align_up(-Z_STEP_PX * Z_MIN, MARGIN_ALIGN_PX);
        let layout = Self {
            width_tiles,
            height_tiles,
            z_top_px: z_top as u32,
            z_bottom_px: z_bottom as u32,
            grid,
        };
        if layout.base_level() + 1 < grid.frame_levels() {
            return Err(format!(
                "facet {width_tiles}x{height_tiles} has {} zoom levels, fewer than the {} cut from each frame",
                layout.base_level() + 1,
                grid.frame_levels()
            ));
        }
        Ok(layout)
    }

    pub fn bbox_width_px(&self) -> u64 {
        (u64::from(self.width_tiles) + u64::from(self.height_tiles)) * TILE_HALF_PX as u64
    }

    pub fn bbox_height_px(&self) -> u64 {
        self.bbox_width_px() + u64::from(self.z_top_px) + u64::from(self.z_bottom_px)
    }

    /// The highest zoom: the smallest level whose tiles cover the bounding box.
    pub fn base_level(&self) -> u32 {
        let side = self.bbox_width_px().max(self.bbox_height_px());
        let tiles = side.div_ceil(u64::from(self.grid.tile_size_px));
        tiles.next_power_of_two().ilog2()
    }

    /// The lowest level cut from frames; lower levels come from its lossless copy.
    pub fn intermediate_level(&self) -> u32 {
        self.base_level() + 1 - self.grid.frame_levels()
    }

    /// Tiles per side at `level` that the bounding box touches.
    pub fn tiles_per_side(&self, level: u32) -> (u32, u32) {
        let scale = 1u64 << (self.base_level() - level);
        let span = u64::from(self.grid.tile_size_px) * scale;
        (self.bbox_width_px().div_ceil(span) as u32, self.bbox_height_px().div_ceil(span) as u32)
    }

    /// The top vertex of tile `(x, y)` at height `z`, in base-level pixels.
    pub fn project(&self, x: i64, y: i64, z: i64) -> (i64, i64) {
        (
            (x - y + i64::from(self.height_tiles)) * TILE_HALF_PX,
            (x + y) * TILE_HALF_PX - Z_STEP_PX * z + i64::from(self.z_top_px),
        )
    }

    /// The tile whose z = 0 diamond holds base-level pixel `(px, py)`; the inverse of [`Self::project`].
    pub fn ground_tile_at(&self, px: i64, py: i64) -> (i64, i64) {
        // u = x − y and v = x + y, both in units of 22 px; floor((u + v) / 2) on the doubled values stays exact.
        let u2 = px - i64::from(self.height_tiles) * TILE_HALF_PX;
        let v2 = py - i64::from(self.z_top_px);
        ((u2 + v2).div_euclid(2 * TILE_HALF_PX), (v2 - u2).div_euclid(2 * TILE_HALF_PX))
    }

    pub fn frames_per_side(&self) -> (u32, u32) {
        let keep = u64::from(self.grid.frame_keep_px());
        (self.bbox_width_px().div_ceil(keep) as u32, self.bbox_height_px().div_ceil(keep) as u32)
    }

    /// The rendered rectangle of a frame: the kept area plus the margin.
    pub fn frame_rect(&self, frame: FrameIndex) -> PixelRect {
        let keep = i64::from(self.grid.frame_keep_px());
        let margin = i64::from(self.grid.frame_margin_px);
        let size = i64::from(self.grid.frame_size_px());
        PixelRect {
            left: i64::from(frame.i) * keep - margin,
            top: i64::from(frame.j) * keep - margin,
            width: size,
            height: size,
        }
    }

    /// The facet's z = 0 ground diamond, clockwise from the top vertex.
    fn ground_diamond(&self) -> [(i64, i64); 4] {
        let (w, h) = (i64::from(self.width_tiles), i64::from(self.height_tiles));
        [self.project(0, 0, 0), self.project(w, 0, 0), self.project(w, h, 0), self.project(0, h, 0)]
    }

    /// Whether anything of the facet can be drawn inside `rect`: a tile at height z, or a sprite on it, appears up
    /// to `z_top` above its ground position and down to `z_bottom` below it.
    pub fn rect_touches_map(&self, rect: PixelRect) -> bool {
        let top = rect.top - i64::from(self.z_bottom_px);
        let bottom = rect.top + rect.height + i64::from(self.z_top_px);
        let (left, right) = (rect.left, rect.left + rect.width);
        let corners = [(left, top), (right, top), (right, bottom), (left, bottom)];
        let diamond = self.ground_diamond();
        // Separating axes: the rectangle's sides and the diamond's 45° sides.
        let axes: [fn((i64, i64)) -> i64; 4] = [|p| p.0, |p| p.1, |p| p.0 + p.1, |p| p.0 - p.1];
        axes.iter().all(|axis| {
            let (a_min, a_max) = min_max(corners.iter().map(|&p| axis(p)));
            let (b_min, b_max) = min_max(diamond.iter().map(|&p| axis(p)));
            a_min < b_max && b_min < a_max
        })
    }

    /// Frames whose kept area can show part of the facet, optionally only those whose kept area meets the
    /// projection of the tile region `[x0, x1) × [y0, y1)`.
    pub fn frames(&self, region: Option<[u32; 4]>) -> Vec<FrameIndex> {
        let region_rect = region.map(|[x0, y0, x1, y1]| self.region_rect(x0, y0, x1, y1));
        let (frames_x, frames_y) = self.frames_per_side();
        let keep = i64::from(self.grid.frame_keep_px());
        let mut frames = Vec::new();
        for j in 0..frames_y {
            for i in 0..frames_x {
                let kept = PixelRect { left: i64::from(i) * keep, top: i64::from(j) * keep, width: keep, height: keep };
                let in_region = region_rect.is_none_or(|r| {
                    kept.left < r.left + r.width && r.left < kept.left + kept.width
                        && kept.top < r.top + r.height && r.top < kept.top + kept.height
                });
                if in_region && self.rect_touches_map(kept) {
                    frames.push(FrameIndex { i, j });
                }
            }
        }
        frames
    }

    /// The pixel bounding rectangle of the ground projection of a tile region.
    pub fn region_rect(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> PixelRect {
        let corners = [
            self.project(i64::from(x0), i64::from(y0), 0),
            self.project(i64::from(x1), i64::from(y0), 0),
            self.project(i64::from(x1), i64::from(y1), 0),
            self.project(i64::from(x0), i64::from(y1), 0),
        ];
        let (left, right) = min_max(corners.iter().map(|p| p.0));
        let (top, bottom) = min_max(corners.iter().map(|p| p.1));
        PixelRect { left, top, width: right - left, height: bottom - top }
    }
}

fn min_max(values: impl Iterator<Item = i64>) -> (i64, i64) {
    values.fold((i64::MAX, i64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRITANNIA: (u32, u32) = (7168, 4096);
    const NO_SPRITES: u32 = 0;

    fn britannia() -> Layout {
        Layout::new(BRITANNIA.0, BRITANNIA.1, NO_SPRITES, TileGrid::EXPORT).unwrap()
    }

    #[test]
    fn britannia_has_ten_levels_and_frames_cover_the_box() {
        let layout = britannia();
        assert_eq!(layout.z_top_px, 512);
        assert_eq!(layout.z_bottom_px, 512);
        assert_eq!(layout.bbox_width_px(), 247_808);
        assert_eq!(layout.base_level(), 9);
        assert_eq!(layout.intermediate_level(), 6);
        assert_eq!(TileGrid::EXPORT.frame_size_px(), 4160);
        let (fx, fy) = layout.frames_per_side();
        assert!(u64::from(fx) * 4096 >= layout.bbox_width_px() && u64::from(fy) * 4096 >= layout.bbox_height_px());
        let frames = layout.frames(None);
        // The diamond covers 2·W·H / (W + H)² = 46 % of the box, plus the frames along its edges; the corners
        // are skipped.
        let all = (fx * fy) as usize;
        assert!(frames.len() * 5 > all * 2 && frames.len() * 5 < all * 3, "{} of {all}", frames.len());
    }

    #[test]
    fn sprite_height_raises_the_top_margin() {
        let layout = Layout::new(16, 16, 200, TileGrid::EXPORT).err();
        // 16x16 tiles fit in fewer levels than a frame yields.
        assert!(layout.is_some());
        let layout = Layout::new(BRITANNIA.0, BRITANNIA.1, 200, TileGrid::EXPORT).unwrap();
        assert_eq!(layout.z_top_px, 768);
    }

    #[test]
    fn projection_and_inverse_agree_over_every_pixel_of_a_diamond() {
        let layout = britannia();
        for &(x, y) in &[(0, 0), (7167, 0), (0, 4095), (7167, 4095), (1496, 1640), (5000, 3000)] {
            let (px, py) = layout.project(x, y, 0);
            assert!(px >= 0 && py >= 0);
            assert!(px as u64 <= layout.bbox_width_px() && (py as u64) < layout.bbox_height_px());
            for dy in 0..2 * TILE_HALF_PX {
                let half = dy.min(2 * TILE_HALF_PX - 1 - dy);
                for dx in -half..=half {
                    assert_eq!(layout.ground_tile_at(px + dx, py + dy), (x, y), "pixel {dx},{dy} of tile {x},{y}");
                }
            }
        }
    }

    #[test]
    fn raised_tiles_move_up_four_pixels_per_unit() {
        let layout = britannia();
        let (px0, py0) = layout.project(100, 200, 0);
        assert_eq!(layout.project(100, 200, 10), (px0, py0 - 40));
        assert_eq!(layout.project(100, 200, -128), (px0, py0 + 512));
    }

    #[test]
    fn region_selects_only_nearby_frames() {
        let layout = britannia();
        let frames = layout.frames(Some([1400, 1600, 1500, 1700]));
        assert!(!frames.is_empty() && frames.len() <= 4, "{frames:?}");
        let (px, py) = layout.project(1450, 1650, 0);
        assert!(frames.contains(&FrameIndex { i: (px / 4096) as u32, j: (py / 4096) as u32 }));
    }

    #[test]
    fn tiles_per_side_halve_per_level() {
        let layout = britannia();
        assert_eq!(layout.tiles_per_side(9), (484, 486));
        assert_eq!(layout.tiles_per_side(8), (242, 243));
        assert_eq!(layout.tiles_per_side(0), (1, 1));
    }
}
