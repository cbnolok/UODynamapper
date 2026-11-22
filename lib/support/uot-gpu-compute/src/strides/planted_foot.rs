//! The planted-foot stride measurement, on the CPU: follows the ground contact of a walking body's planted foot from
//! frame to frame, deterministically (no model, no GPU), and gives the stride in side tiles (see [`super`]).
//!
//! The frames sit on their anchor, so the body stays put and a planted foot slides backward at the body's speed,
//! rigidly, while the swinging foot moves forward and the body above stays. For each pair of frames (the last with the
//! first too), every opaque pixel (alpha over one half) of the ground band (the lowest [`BAND_FRACTION`] of the
//! silhouette, measured across the travel) looks for where its 5x5 patch went in the next frame, among the shifts
//! backward along the screen x of the travel (with a little room up and down on the side facing, and within the
//! [`MIN_ANGLE_DEG`]..[`MAX_ANGLE_DEG`] cone of a diagonal one, whose feet move up or down the screen as they go
//! back); a pixel whose best patch matches (mean RGBA difference under [`MATCH_TOLERANCE`]) votes for its horizontal
//! shift. The pair's travel is the backward shift most pixels vote for, when at least [`MIN_VOTES`] do: the planted
//! foot, the largest region moving rigidly backward (the still body and the forward swing vote for no backward
//! shift), refined to a fraction of a pixel by the votes' mean over it and its neighbours within
//! [`REFINE_RADIUS_PX`] (whole pixels would step a ten-frame clip's stride by a quarter tile). The clip's travel per
//! pair is the median of the pairs that found one (at least [`MIN_PAIR_FRACTION`] of them), times the frames, over
//! the screen x of a side tile along the facing.
//!
//! Only the horizontal shift is read: a diagonal facing's feet move along a shallower slope than its ground's 45
//! degrees, and their screen x over the 22 px of a diagonal tile, divided by the square root of two, gives the same
//! side stride the side facing's x over its 44 px gives. Chosen on a Python prototype against strides counted on the
//! frames, then tuned in Rust over the band, the tolerance, the votes and the per-pair reduction
//! (`docs/agent_notes/mobile_animation_motion.md`, "Planted-foot measurement").

use super::{percentile, travel_direction, Measurement, StrideClip, FACING_COUNT, HALF_TILE_PX, PERCENT};

/// A pixel of a frame is opaque above this alpha.
const OPAQUE_ALPHA: u8 = 127;
/// The ground band: this fraction of the silhouette's extent across the travel, at least [`MIN_BAND_PX`].
const BAND_FRACTION: f32 = 0.15;
const MIN_BAND_PX: f32 = 4.0;
/// The ground band of a diagonal facing runs along a line this many degrees off the screen's horizontal (rising
/// toward the facing's back), the slope its feet line up along; the side facing's runs level.
const DIAGONAL_GROUND_DEG: f32 = 30.0;
/// The largest backward shift a pair is searched for, in pixels, and the forward one (the still body's pixels find
/// their match at no shift and a pixel's forward neighbour, so they vote for neither).
const MAX_SHIFT_PX: i32 = 60;
const MIN_SHIFT_PX: i32 = -1;
/// The vertical room around a shift: up and down on the side facing, about the cone of a diagonal one.
const VERTICAL_SLACK_PX: f32 = 1.5;
/// The cone a diagonal facing's planted foot moves in, in degrees above the screen's horizontal toward the back.
const MIN_ANGLE_DEG: f32 = 10.0;
const MAX_ANGLE_DEG: f32 = 60.0;
/// The patch a pixel is matched by: the square of this radius around it.
const PATCH_RADIUS: i32 = 2;
/// A patch matches under this mean difference per RGBA channel (0..255).
const MATCH_TOLERANCE: f32 = 20.0;
/// The smallest backward shift voted for, and the votes a pair's travel needs.
const MIN_VOTE_SHIFT_PX: i32 = 1;
const MIN_VOTES: usize = 4;
/// The shifts either side of the most voted one whose votes refine it.
const REFINE_RADIUS_PX: usize = 1;
/// The pairs of a clip that must find a travel.
const MIN_PAIR_FRACTION: f32 = 0.3;
const RGBA_BYTES: usize = 4;

/// Measures the clips of a movement in several facings (`None` where a facing has no frames), in side tiles.
pub fn measure_planted_foot<const FACINGS: usize>(clips: &[Option<StrideClip>; FACINGS]) -> Measurement<FACINGS> {
    let facings = std::array::from_fn(|index| clips[index].as_ref().and_then(clip_side_tiles));
    Measurement::of_facings(facings)
}

/// The side stride of one facing's clip; none when it travels straight up or down the screen, or when too few pairs
/// find a planted foot.
pub fn clip_side_tiles(clip: &StrideClip) -> Option<f32> {
    let facing = FacingClip::of(clip)?;
    let count = facing.canvas.frames.len();
    let travels: Vec<f32> = (0..count).filter_map(|from| facing.pair_travel(from, None)).collect();
    facing.side_tiles(&travels)
}

/// The side stride of a clip of `frames` frames from the travels (in screen px) of the pairs taken: their median times
/// the frames, over `side_tile_x`, the screen x of a side tile along the facing ([`ClipInspection::side_tile_x`]);
/// none without a travel. The measurement takes every pair that found one; a stride counted on the pairs seen to
/// follow the planted foot takes those alone.
pub fn side_tiles_of_pairs(travels: &[f32], frames: usize, side_tile_x: f32) -> Option<f32> {
    if travels.is_empty() {
        return None;
    }
    let mut travels = travels.to_vec();
    Some(percentile(&mut travels, PERCENT / 2.0) * frames as f32 / side_tile_x)
}

/// What the measurement sees of one facing's clip, pair by pair (`udd-cli tool strides inspect`): to check by eye
/// which pairs follow a planted foot and which a wing, a tail or a robe, and to count a stride on the pairs that do
/// ([`side_tiles_of_pairs`]).
pub struct ClipInspection {
    /// The canvas the frames sit on, each on its anchor.
    pub width: i32,
    pub height: i32,
    /// Each frame's straight-alpha RGBA rows on the canvas.
    pub frames: Vec<Vec<u8>>,
    /// Whether each canvas pixel lies in the ground band.
    pub band: Vec<bool>,
    /// The screen x of a side tile along the facing.
    pub side_tile_x: f32,
    /// Each pair of frames, the last with the first too.
    pub pairs: Vec<PairInspection>,
    /// The facing's side stride, as [`clip_side_tiles`] gives it.
    pub stride: Option<f32>,
}

/// A pair of frames as the measurement sees it.
pub struct PairInspection {
    pub from: usize,
    pub to: usize,
    /// The planted foot's backward travel in screen px; none when too few pixels vote.
    pub travel_px: Option<f32>,
    /// The opaque ground band pixels of the first frame.
    pub band_pixels: usize,
    /// The canvas pixels of the first frame whose votes refined the travel (the most voted shift and its neighbours
    /// within [`REFINE_RADIUS_PX`]): what the measurement followed.
    pub voters: Vec<usize>,
}

/// [`ClipInspection`] of a facing's clip; none where [`clip_side_tiles`] cannot measure it.
pub fn inspect_clip(clip: &StrideClip) -> Option<ClipInspection> {
    let facing = FacingClip::of(clip)?;
    let count = facing.canvas.frames.len();
    let pairs: Vec<PairInspection> = (0..count).map(|from| {
        let mut voters = Vec::new();
        let travel_px = facing.pair_travel(from, Some(&mut voters));
        let frame = &facing.canvas.frames[from];
        let band_pixels = (0..facing.band.len()).filter(|&at| facing.band[at] && Canvas::opaque(frame, at)).count();
        PairInspection { from, to: (from + 1) % count, travel_px, band_pixels, voters }
    }).collect();
    let travels: Vec<f32> = pairs.iter().filter_map(|pair| pair.travel_px).collect();
    let stride = facing.side_tiles(&travels);
    let FacingClip { canvas, band, side_tile_x, .. } = facing;
    Some(ClipInspection { width: canvas.width, height: canvas.height, frames: canvas.frames, band, side_tile_x,
        pairs, stride })
}

/// A facing's clip on its canvas, set up for the pairs' search.
struct FacingClip {
    canvas: Canvas,
    band: Vec<bool>,
    shifts: Vec<(i32, i32)>,
    backward_x: i32,
    side_tile_x: f32,
}

impl FacingClip {
    fn of(clip: &StrideClip) -> Option<Self> {
        if clip.facing >= FACING_COUNT || clip.frames.len() < 2 {
            return None;
        }
        let (mut travel_x, travel_y, _) = travel_direction(clip.facing);
        if clip.mirrored {
            travel_x = -travel_x;
        }
        if travel_x.abs() < f32::EPSILON {
            return None;
        }
        let canvas = Canvas::of(clip)?;
        let geometry = Geometry::new(travel_x, travel_y);
        let band = canvas.ground_band(&geometry);
        // The screen x of a side tile along the travel: the side facing's 44 px, a diagonal's 22 px times the square
        // root of two.
        let side_tile_x = 2.0 * HALF_TILE_PX * travel_x.abs();
        Some(Self { canvas, band, shifts: geometry.shifts(), backward_x: geometry.backward_x, side_tile_x })
    }

    /// The travel from frame `from` to the next (the last to the first), the pixels whose votes refined it put in
    /// `voters` when given.
    fn pair_travel(&self, from: usize, voters: Option<&mut Vec<usize>>) -> Option<f32> {
        let to = (from + 1) % self.canvas.frames.len();
        self.canvas.pair_travel(from, to, &self.band, &self.shifts, self.backward_x, voters)
    }

    /// The side stride from the travels of the pairs that found one: none when they are too few.
    fn side_tiles(&self, travels: &[f32]) -> Option<f32> {
        let count = self.canvas.frames.len();
        if (travels.len() as f32) < MIN_PAIR_FRACTION * count as f32 {
            return None;
        }
        side_tiles_of_pairs(travels, count, self.side_tile_x)
    }
}

/// A facing's directions in the stored frames.
struct Geometry {
    /// +1 or -1: the screen x the planted foot moves toward.
    backward_x: i32,
    /// +1, -1 or 0 (the side facing): the screen y it moves toward.
    backward_y: i32,
    /// The direction across the travel toward the ground, the band's measure.
    across: (f32, f32),
}

impl Geometry {
    fn new(travel_x: f32, travel_y: f32) -> Self {
        let backward_x = if travel_x > 0.0 { -1 } else { 1 };
        if travel_y.abs() < f32::EPSILON {
            return Self { backward_x, backward_y: 0, across: (0.0, 1.0) };
        }
        let backward_y = if travel_y > 0.0 { -1 } else { 1 };
        // The ground line rises toward the back: its normal leans toward the front's opposite side.
        let angle = DIAGONAL_GROUND_DEG.to_radians();
        let lean = -backward_y as f32 * backward_x as f32;
        Self { backward_x, backward_y, across: (lean * angle.sin(), angle.cos()) }
    }

    /// The shifts searched, as (backward x in pixels, screen y), the vertical outer: the order ties resolve in.
    fn shifts(&self) -> Vec<(i32, i32)> {
        let (low, high) = (MIN_ANGLE_DEG.to_radians().tan(), MAX_ANGLE_DEG.to_radians().tan());
        let mut shifts = Vec::new();
        for dy in -MAX_SHIFT_PX..=MAX_SHIFT_PX {
            for back in MIN_SHIFT_PX..=MAX_SHIFT_PX {
                let keep = if self.backward_y == 0 {
                    dy.abs() as f32 <= VERTICAL_SLACK_PX
                } else {
                    let along = (dy * self.backward_y) as f32;
                    along >= low * back as f32 - VERTICAL_SLACK_PX && along <= high * back as f32 + VERTICAL_SLACK_PX
                };
                if keep {
                    shifts.push((back, dy));
                }
            }
        }
        shifts
    }
}

/// A clip's frames on one canvas, each on its anchor, with room around for the shifts and the patches.
struct Canvas {
    width: i32,
    height: i32,
    /// Straight-alpha RGBA rows of each frame, transparent off the frame.
    frames: Vec<Vec<u8>>,
}

impl Canvas {
    fn of(clip: &StrideClip) -> Option<Self> {
        let frames = &clip.frames;
        let margin = MAX_SHIFT_PX + PATCH_RADIUS + 1;
        let left = frames.iter().map(|frame| -frame.anchor_x).min()? - margin;
        let top = frames.iter().map(|frame| -frame.anchor_y).min()? - margin;
        let right = frames.iter().map(|frame| i32::from(frame.width) - frame.anchor_x).max()? + margin;
        let bottom = frames.iter().map(|frame| i32::from(frame.height) - frame.anchor_y).max()? + margin;
        let (width, height) = (right - left, bottom - top);
        let frames = frames.iter().map(|frame| {
            let mut canvas = vec![0u8; (width * height) as usize * RGBA_BYTES];
            let row_bytes = usize::from(frame.width) * RGBA_BYTES;
            let x0 = (-frame.anchor_x - left) as usize;
            for y in 0..usize::from(frame.height) {
                let at = ((-frame.anchor_y - top) as usize + y) * width as usize + x0;
                if let Some(row) = frame.rgba.get(y * row_bytes..(y + 1) * row_bytes) {
                    canvas[at * RGBA_BYTES..][..row_bytes].copy_from_slice(row);
                }
            }
            canvas
        }).collect();
        Some(Self { width, height, frames })
    }

    fn opaque(frame: &[u8], at: usize) -> bool {
        frame[at * RGBA_BYTES + 3] > OPAQUE_ALPHA
    }

    /// Whether each pixel lies in the ground band: within the band of the furthest any frame's silhouette reaches
    /// across the travel toward the ground.
    fn ground_band(&self, geometry: &Geometry) -> Vec<bool> {
        let measure = |at: usize| {
            let (x, y) = ((at as i32 % self.width) as f32, (at as i32 / self.width) as f32);
            x * geometry.across.0 + y * geometry.across.1
        };
        let pixels = (self.width * self.height) as usize;
        let (mut low, mut ground) = (f32::INFINITY, f32::NEG_INFINITY);
        for frame in &self.frames {
            for at in (0..pixels).filter(|&at| Self::opaque(frame, at)) {
                let across = measure(at);
                low = low.min(across);
                ground = ground.max(across);
            }
        }
        let band = MIN_BAND_PX.max(BAND_FRACTION * (ground - low));
        (0..pixels).map(|at| measure(at) >= ground - band).collect()
    }

    /// The mean RGBA difference of the patch around `(x, y)` in `from` and around it shifted by `(dx, dy)` in `to`;
    /// stops early, as `None`, past `limit`.
    fn patch_difference(&self, from: &[u8], to: &[u8], x: i32, y: i32, dx: i32, dy: i32, limit: f32) -> Option<f32> {
        let side = 2 * PATCH_RADIUS + 1;
        let scale = (side * side) as f32 * RGBA_BYTES as f32;
        let limit = (limit * scale) as u32;
        let mut sum = 0u32;
        for py in y - PATCH_RADIUS..=y + PATCH_RADIUS {
            for px in x - PATCH_RADIUS..=x + PATCH_RADIUS {
                let (tx, ty) = (px + dx, py + dy);
                let a = &from[(py * self.width + px) as usize * RGBA_BYTES..][..RGBA_BYTES];
                let inside = tx >= 0 && ty >= 0 && tx < self.width && ty < self.height;
                for channel in 0..RGBA_BYTES {
                    let b = if inside { to[(ty * self.width + tx) as usize * RGBA_BYTES + channel] } else { 0 };
                    sum += u32::from(a[channel].abs_diff(b));
                }
            }
            if sum >= limit {
                return None;
            }
        }
        Some(sum as f32 / scale)
    }

    /// The backward shift most ground pixels of frame `from` vote for into frame `to`, in pixels, refined by its
    /// neighbours' votes; the pixels whose votes refined it put in `voters` when given.
    fn pair_travel(&self, from: usize, to: usize, band: &[bool], shifts: &[(i32, i32)], backward_x: i32,
        mut voters: Option<&mut Vec<usize>>) -> Option<f32> {
        let mut votes_of = voters.as_ref().map(|_| Vec::new());
        let (first, second) = (&self.frames[from], &self.frames[to]);
        let mut votes = vec![0usize; (MAX_SHIFT_PX + 1) as usize];
        for at in (0..band.len()).filter(|&at| band[at] && Self::opaque(first, at)) {
            let (x, y) = (at as i32 % self.width, at as i32 / self.width);
            let mut best: Option<(f32, i32)> = None;
            for &(back, dy) in shifts {
                let limit = best.map_or(MATCH_TOLERANCE, |(difference, _)| difference);
                if let Some(difference) = self.patch_difference(first, second, x, y, back * backward_x, dy, limit) {
                    if difference < limit {
                        best = Some((difference, back));
                    }
                }
            }
            if let Some((_, back)) = best.filter(|&(_, back)| back >= MIN_VOTE_SHIFT_PX) {
                votes[back as usize] += 1;
                if let Some(votes_of) = votes_of.as_mut() {
                    votes_of.push((at, back as usize));
                }
            }
        }
        // The most voted shift; the shorter of a tie.
        let (back, &count) = votes.iter().enumerate().rev().max_by_key(|&(_, count)| *count)?;
        if count < MIN_VOTES {
            return None;
        }
        let near = back.saturating_sub(REFINE_RADIUS_PX)..=(back + REFINE_RADIUS_PX).min(votes.len() - 1);
        let (sum, weight) = near.clone().fold((0usize, 0usize), |(sum, weight), shift| {
            (sum + shift * votes[shift], weight + votes[shift])
        });
        if let (Some(voters), Some(votes_of)) = (voters.as_deref_mut(), votes_of) {
            voters.extend(votes_of.into_iter().filter(|(_, shift)| near.contains(shift)).map(|(at, _)| at));
        }
        Some(sum as f32 / weight as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::super::StrideFrame;
    use super::*;

    /// A synthetic walk of `frames` frames of [`SIZE`] px, the anchor at the bottom centre: a textured body block
    /// standing still over a textured foot that slides `step` (screen x and y) a frame over the first `planted`
    /// frames and swings back to where it started over the rest.
    fn walk(frames: usize, planted: usize, step: (i32, i32)) -> Vec<Vec<u8>> {
        let swing = (frames - planted) as i32;
        let reach = (step.0 * planted as i32, step.1 * planted as i32);
        let start = (FOOT_CENTRE.0 - reach.0 / 2, FOOT_CENTRE.1 - reach.1 / 2);
        (0..frames).map(|index| {
            let mut rgba = vec![0u8; (SIZE * SIZE) as usize * RGBA_BYTES];
            let mut paint = |x: i32, y: i32, seed: i32| {
                if (0..SIZE).contains(&x) && (0..SIZE).contains(&y) {
                    let texel = &mut rgba[(y * SIZE + x) as usize * RGBA_BYTES..][..RGBA_BYTES];
                    let shade = ((seed * 37 + 11) % 97) as u8;
                    texel.copy_from_slice(&[60 + shade, 40 + shade / 2, 200 - shade, 255]);
                }
            };
            for y in BODY_ROWS {
                for x in BODY_COLUMNS {
                    paint(x, y, x * 7 + y * 13);
                }
            }
            let index = index as i32;
            let offset = if index < planted as i32 {
                (step.0 * index, step.1 * index)
            } else {
                let back = index - planted as i32;
                (reach.0 - reach.0 * back / swing, reach.1 - reach.1 * back / swing)
            };
            for y in 0..FOOT_SIZE.1 {
                for x in 0..FOOT_SIZE.0 {
                    paint(start.0 + offset.0 + x, start.1 + offset.1 + y, x * 5 + y * 17);
                }
            }
            rgba
        }).collect()
    }

    const SIZE: i32 = 120;
    const BODY_ROWS: std::ops::Range<i32> = 30..90;
    const BODY_COLUMNS: std::ops::Range<i32> = 45..75;
    /// The top-left corner of the foot halfway through its slide, and its size.
    const FOOT_CENTRE: (i32, i32) = (52, 96);
    const FOOT_SIZE: (i32, i32) = (16, 10);

    fn clip(frames: &[Vec<u8>], facing: u8) -> StrideClip<'_> {
        StrideClip { facing, mirrored: false, frames: frames.iter().map(|rgba| StrideFrame {
            width: SIZE as u16, height: SIZE as u16, anchor_x: SIZE / 2, anchor_y: SIZE - 1, rgba }).collect() }
    }

    #[test]
    fn the_side_facing_reads_the_planted_foots_slide() {
        // South-west: the foot slides right 6 px a frame over 8 of 10 frames, the stride 6 * 10 / 44 side tiles
        // (the median of the planted pairs, whatever the swing's).
        let frames = walk(10, 8, (6, 0));
        let tiles = clip_side_tiles(&clip(&frames, 5)).expect("a stride");
        assert!((tiles - 60.0 / 44.0).abs() < 0.02, "{tiles}");
    }

    #[test]
    fn a_diagonal_facing_reads_the_screen_x_over_a_diagonal_side_tile() {
        // South: walking down-left, the foot slides right 5 px and up 3 a frame: its x over 22 * sqrt 2 px.
        let frames = walk(10, 8, (5, -3));
        let tiles = clip_side_tiles(&clip(&frames, 4)).expect("a stride");
        assert!((tiles - 50.0 / (22.0 * 2f32.sqrt())).abs() < 0.03, "{tiles}");
        // Mirrored, the stored frames of the west facing walk the other way: the same frames read as north-east.
        let mirrored = walk(10, 8, (-5, 3));
        let mut west = clip(&mirrored, 6);
        west.mirrored = true;
        let tiles = clip_side_tiles(&west).expect("a stride");
        assert!((tiles - 50.0 / (22.0 * 2f32.sqrt())).abs() < 0.03, "{tiles}");
    }

    #[test]
    fn a_body_without_planted_feet_gets_no_stride() {
        // A foot that never moves gives no backward vote.
        let still = walk(10, 0, (0, 0));
        assert_eq!(clip_side_tiles(&clip(&still, 5)), None);
        let measurement = measure_planted_foot(&[Some(clip(&still, 4)), Some(clip(&still, 5)), None]);
        assert_eq!(measurement.stride, Err("too few facings"));
    }

    #[test]
    fn the_facings_give_one_side_stride() {
        let side = walk(10, 8, (6, 0));
        let south = walk(10, 8, (6, -3));
        let measurement = measure_planted_foot(&[Some(clip(&south, 4)), Some(clip(&side, 5)), None]);
        let stride = measurement.stride.expect("a stride");
        // The side's 60 / 44 and the south's 60 / 31.1: the median of two is their mean.
        assert!((stride - (60.0 / 44.0 + 60.0 / (22.0 * 2f32.sqrt())) / 2.0).abs() < 0.03, "{stride}");
    }

    /// The inspection sees what the measurement does: the same stride, the planted pairs' 6 px followed on the foot
    /// (the voters lie in its rows, under the body), and a stride counted on the planted pairs alone.
    #[test]
    fn the_inspection_follows_the_planted_foot_as_the_measurement_does() {
        const PLANTED: usize = 8;
        let frames = walk(10, PLANTED, (6, 0));
        let side = clip(&frames, 5);
        let inspection = inspect_clip(&side).expect("an inspection");
        assert_eq!(inspection.stride, clip_side_tiles(&side));
        assert_eq!(inspection.pairs.len(), frames.len());
        let planted = &inspection.pairs[..PLANTED - 1];
        for pair in planted {
            let travel = pair.travel_px.expect("a planted pair");
            assert!((travel - 6.0).abs() < 0.5, "pair {}: {travel}", pair.from);
            assert!(!pair.voters.is_empty() && pair.voters.len() <= pair.band_pixels, "pair {}", pair.from);
            // The frames sit the canvas margin down (their anchors all alike).
            let frame_row = |at: usize| at as i32 / inspection.width - (MAX_SHIFT_PX + PATCH_RADIUS + 1);
            assert!(pair.voters.iter().all(|&at| frame_row(at) >= BODY_ROWS.end), "pair {}", pair.from);
        }
        let travels: Vec<f32> = planted.iter().filter_map(|pair| pair.travel_px).collect();
        let tiles = side_tiles_of_pairs(&travels, frames.len(), inspection.side_tile_x).expect("a stride");
        assert!((tiles - 60.0 / 44.0).abs() < 0.02, "{tiles}");
        assert_eq!(side_tiles_of_pairs(&[], frames.len(), inspection.side_tile_x), None);
    }
}
