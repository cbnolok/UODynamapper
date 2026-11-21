//! The in-between frames of the mobiles: between two frames of a clip, a frame composed on the CPU for the time between
//! them, so an animation of 8 to 12 frames a second does not step on a faster display. Each mobile frame drawn has a
//! slot of the frame atlas's in-between page; its frame is composed and uploaded again only when its clip, frames or
//! point between them change. The packages are untouched: the in-betweens come from the frames alone.

use std::sync::Arc;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use super::clips::{Clip, ClipKey, FramePick};
use super::frame_atlas::{AtlasRect, MobileFrameAtlas, BYTES_PER_TEXEL, FRAME_GUTTER, IN_BETWEEN_SLOTS,
    IN_BETWEEN_SLOT_HEIGHT, IN_BETWEEN_SLOT_WIDTH};
use super::placement::FramePlacement;

/// What a mobile shows between two frames of its animation. Kept in the app-written world rendering settings as
/// `world_rendering.mobile_in_between_frames`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileInBetweenFrames {
    /// Each frame until the next, as the clients show them.
    Off,
    /// The two frames blended by the time between them.
    #[default]
    Crossfade,
}

impl MobileInBetweenFrames {
    pub const ALL: [Self; 2] = [Self::Off, Self::Crossfade];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Crossfade => "Crossfade",
        }
    }
}

/// The points between two frames an in-between is composed for: a pick's `t` is floored to one of them, and the
/// first, 0, shows the frame itself. At 64 a step changes a blended channel by at most 4 of 255.
const IN_BETWEEN_T_STEPS: u32 = 64;
/// The fixed-point scale of the blend weight.
const BLEND_WEIGHT_ONE: u32 = 256;

/// One of the two frames an in-between is composed from: its straight-alpha RGBA8 pixels and its placement.
#[derive(Clone, Copy)]
pub struct InBetweenSource<'a> {
    pub rgba: &'a [u8],
    pub placement: FramePlacement,
}

/// The placement holding both frames with their anchors on one point: the anchor of each frame is at the union's.
pub fn union_placement(a: FramePlacement, b: FramePlacement) -> FramePlacement {
    let left = a.anchor_x.max(b.anchor_x);
    let right = (a.width - a.anchor_x).max(b.width - b.anchor_x);
    let up = a.anchor_y.max(b.anchor_y);
    let down = (a.height - a.anchor_y).max(b.height - b.anchor_y);
    FramePlacement { width: left + right, height: up + down, anchor_x: left, anchor_y: up }
}

/// The crossfade of `a` into `b` at `t` (0..1) on `union` (`union_placement`), into `out`, rows of `stride` bytes:
/// premultiplied colour and alpha blended by `t`, then straight again. The bytes are blended as the sRGB atlas holds
/// them, as the clients blend their sprites. A warp of the frames toward each other would replace this function.
pub fn crossfade(a: InBetweenSource, b: InBetweenSource, t: f32, union: FramePlacement, out: &mut [u8], stride: usize) {
    let weight_b = (t.clamp(0.0, 1.0) * BLEND_WEIGHT_ONE as f32).round() as u32;
    let weight_a = BLEND_WEIGHT_ONE - weight_b;
    let (a, b) = (SourceRows::new(a, union), SourceRows::new(b, union));
    let (width, height) = (union.width as usize, union.height as usize);
    for y in 0..height {
        let row = &mut out[y * stride..y * stride + width * TEXEL];
        let (row_a, row_b) = (a.row(y), b.row(y));
        for (x, texel) in row.chunks_exact_mut(TEXEL).enumerate() {
            let (pa, pb) = (a.texel(row_a, x), b.texel(row_b, x));
            let alpha = u32::from(pa[3]) * weight_a + u32::from(pb[3]) * weight_b;
            if alpha == 0 {
                texel.copy_from_slice(&[0; 4]);
                continue;
            }
            if pa[3] == pb[3] {
                // Equal alphas: the colours mix by the weights alone.
                for channel in 0..3 {
                    texel[channel] = ((u32::from(pa[channel]) * weight_a + u32::from(pb[channel]) * weight_b
                        + BLEND_WEIGHT_ONE / 2) / BLEND_WEIGHT_ONE) as u8;
                }
            } else if pa[3] == 0 || pb[3] == 0 {
                // One frame alone: its colour, faded.
                texel[..3].copy_from_slice(&(if pb[3] == 0 { pa } else { pb })[..3]);
            } else {
                for channel in 0..3 {
                    let premultiplied = u32::from(pa[channel]) * u32::from(pa[3]) * weight_a
                        + u32::from(pb[channel]) * u32::from(pb[3]) * weight_b;
                    texel[channel] = ((premultiplied + alpha / 2) / alpha) as u8;
                }
            }
            texel[3] = ((alpha + BLEND_WEIGHT_ONE / 2) / BLEND_WEIGHT_ONE) as u8;
        }
    }
}

const TEXEL: usize = BYTES_PER_TEXEL as usize;

/// A frame's rows as they lie on the union: `x` and `y` texels from its top-left corner.
struct SourceRows<'a> {
    rgba: &'a [u8],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl<'a> SourceRows<'a> {
    fn new(source: InBetweenSource<'a>, union: FramePlacement) -> Self {
        let frame = source.placement;
        Self {
            rgba: source.rgba,
            x: (union.anchor_x - frame.anchor_x) as usize,
            y: (union.anchor_y - frame.anchor_y) as usize,
            width: frame.width as usize,
            height: frame.height as usize,
        }
    }

    /// The frame's row at the union's row `y`, if it has one there.
    fn row(&self, y: usize) -> Option<&'a [u8]> {
        let row = y.checked_sub(self.y).filter(|&row| row < self.height)?;
        self.rgba.get(row * self.width * TEXEL..(row + 1) * self.width * TEXEL)
    }

    /// The texel of `row` at the union's column `x`, transparent outside the frame.
    fn texel(&self, row: Option<&[u8]>, x: usize) -> [u8; 4] {
        let column = x.checked_sub(self.x).filter(|&column| column < self.width);
        match (row, column) {
            (Some(row), Some(column)) => {
                let at = column * TEXEL;
                [row[at], row[at + 1], row[at + 2], row[at + 3]]
            }
            _ => [0; 4],
        }
    }
}

/// What a slot shows: the in-between of a clip's frames `from` and `to` at step `t_step` of `IN_BETWEEN_T_STEPS`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct InBetweenKey {
    clip: ClipKey,
    from: usize,
    to: usize,
    t_step: u32,
}

#[derive(Default)]
struct InBetweenSlot {
    shown: Option<(InBetweenKey, AtlasRect, FramePlacement)>,
    /// The upload buffers, reused once the render world has dropped them.
    buffers: Vec<Arc<[u8]>>,
}

impl InBetweenSlot {
    /// The index of a buffer of at least `len` bytes no upload still holds.
    fn free_buffer(&mut self, len: usize) -> usize {
        if let Some(index) = self.buffers.iter().position(|buffer| Arc::strong_count(buffer) == 1 && buffer.len() >= len) {
            return index;
        }
        let capacity = len.next_power_of_two()
            .min((IN_BETWEEN_SLOT_WIDTH * IN_BETWEEN_SLOT_HEIGHT * BYTES_PER_TEXEL) as usize).max(len);
        let buffer = Arc::from(vec![0u8; capacity]);
        // A free buffer too small gives way; else one more, while the others are still uploading.
        match self.buffers.iter().position(|buffer| Arc::strong_count(buffer) == 1) {
            Some(index) => {
                self.buffers[index] = buffer;
                index
            }
            None => {
                self.buffers.push(buffer);
                self.buffers.len() - 1
            }
        }
    }
}

/// The in-between slots of the mobile frames drawn.
#[derive(Resource)]
pub struct MobileInBetweens {
    slots: Vec<InBetweenSlot>,
}

impl Default for MobileInBetweens {
    fn default() -> Self {
        Self { slots: (0..IN_BETWEEN_SLOTS).map(|_| InBetweenSlot::default()).collect() }
    }
}

impl MobileInBetweens {
    /// The frame to draw as the mobile frame of `slot` for `pick` of `clip` (`key`), as its atlas rectangle and its
    /// placement: an in-between when `mode` asks for one and `pick` lies between two frames, composed and staged in
    /// `atlas` unless the slot shows it already. None draws the picked frame: with `mode` off, a clip of one frame,
    /// `t` under the first step, or frames whose union does not fit a slot.
    pub fn frame(&mut self, slot: usize, mode: MobileInBetweenFrames, key: ClipKey, clip: &Clip, pick: FramePick,
        atlas: &mut MobileFrameAtlas<ClipKey>) -> Option<(AtlasRect, FramePlacement)> {
        if mode == MobileInBetweenFrames::Off || clip.frames.len() < 2 {
            return None;
        }
        let t_step = (pick.t * IN_BETWEEN_T_STEPS as f32) as u32;
        if t_step == 0 {
            return None;
        }
        let slot_index = u32::try_from(slot).ok()?;
        let slot = self.slots.get_mut(slot)?;
        let wanted = InBetweenKey { clip: key, from: pick.index, to: pick.next, t_step };
        if let Some((shown, rect, placement)) = slot.shown
            && shown == wanted {
            return Some((rect, placement));
        }
        let (from, to) = (clip.frames.get(pick.index)?, clip.frames.get(pick.next)?);
        let union = union_placement(from.placement, to.placement);
        let (width, height) = (union.width as u32, union.height as u32);
        let padded = AtlasRect::in_between_slot(slot_index, width + 2 * FRAME_GUTTER, height + 2 * FRAME_GUTTER)?;
        let stride = (padded.width * BYTES_PER_TEXEL) as usize;
        let len = stride * padded.height as usize;
        let index = slot.free_buffer(len);
        let buffer = Arc::get_mut(&mut slot.buffers[index]).expect("no upload holds the buffer");
        // The gutter stays transparent, so a filtered tap at the edge reads nothing an earlier in-between left.
        buffer[..len].fill(0);
        let first = FRAME_GUTTER as usize * stride + (FRAME_GUTTER * BYTES_PER_TEXEL) as usize;
        crossfade(InBetweenSource { rgba: &from.rgba, placement: from.placement },
            InBetweenSource { rgba: &to.rgba, placement: to.placement },
            t_step as f32 / IN_BETWEEN_T_STEPS as f32, union, &mut buffer[first..], stride);
        atlas.stage_in_between(padded, slot.buffers[index].clone());
        let rect = AtlasRect { x: padded.x + FRAME_GUTTER, y: padded.y + FRAME_GUTTER, width, height, ..padded };
        slot.shown = Some((wanted, rect, union));
        Some((rect, union))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::frame_atlas::IN_BETWEEN_LAYER;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn placement(width: f32, height: f32, anchor_x: f32, anchor_y: f32) -> FramePlacement {
        FramePlacement { width, height, anchor_x, anchor_y }
    }

    fn compose(a: InBetweenSource, b: InBetweenSource, t: f32) -> (FramePlacement, Vec<[u8; 4]>) {
        let union = union_placement(a.placement, b.placement);
        let stride = union.width as usize * BYTES_PER_TEXEL as usize;
        let mut out = vec![0u8; stride * union.height as usize];
        crossfade(a, b, t, union, &mut out, stride);
        (union, out.chunks_exact(4).map(|texel| [texel[0], texel[1], texel[2], texel[3]]).collect())
    }

    #[test]
    fn the_union_holds_both_frames_on_their_anchors() {
        // a: 2 left and 1 right of its anchor; b: 0 left and 4 right. Up 3 and 2, down 0 and 2.
        let union = union_placement(placement(3.0, 3.0, 2.0, 3.0), placement(4.0, 4.0, 0.0, 2.0));
        assert_eq!(union, placement(6.0, 5.0, 2.0, 3.0));
    }

    #[test]
    fn a_crossfade_blends_premultiplied_and_keeps_each_frames_colour_alone() {
        // Two 2 by 1 frames, b one texel to the right of a: the middle texel overlaps.
        let a_pixels = [RED, RED].concat();
        let b_pixels = [BLUE, BLUE].concat();
        let a = InBetweenSource { rgba: &a_pixels, placement: placement(2.0, 1.0, 0.0, 1.0) };
        let b = InBetweenSource { rgba: &b_pixels, placement: placement(2.0, 1.0, -1.0, 1.0) };
        let (union, texels) = compose(a, b, 0.25);
        assert_eq!(union, placement(3.0, 1.0, 0.0, 1.0));
        // a alone: its colour, its alpha faded by t; both: the colours mixed by t, opaque; b alone: faded in by t.
        assert_eq!(texels, vec![[255, 0, 0, 191], [191, 0, 64, 255], [0, 0, 255, 64]]);
        // At 0 the first frame shows as it is.
        let (_, texels) = compose(a, b, 0.0);
        assert_eq!(texels, vec![RED, RED, [0, 0, 0, 0]]);
    }

    #[test]
    fn a_slot_composes_and_uploads_only_when_its_in_between_changes() {
        use super::super::clips::{AnimSource, ClipFrame, CLASSIC_FRAME_SECS};
        let mut images = Assets::<Image>::default();
        let mut atlas = MobileFrameAtlas::<ClipKey>::new(&mut images);
        let frame = |colour: [u8; 4], width: u32, height: u32| ClipFrame {
            rect: AtlasRect { layer: 0, x: 0, y: 0, width, height },
            placement: placement(width as f32, height as f32, 0.0, height as f32),
            rgba: Arc::from(colour.repeat((width * height) as usize)),
        };
        let clip = Clip { frames: vec![frame(RED, 2, 2), frame(BLUE, 3, 1)], mirrored: false,
            frame_secs: CLASSIC_FRAME_SECS };
        let key = ClipKey { source: AnimSource::Classic, body: 400, action: 0, direction: 0 };
        let mut in_betweens = MobileInBetweens::default();
        let frame_at = |in_betweens: &mut MobileInBetweens, atlas: &mut MobileFrameAtlas<ClipKey>, mode, t: f32| {
            in_betweens.frame(1, mode, key, &clip, FramePick { index: 0, next: 1, t }, atlas)
        };

        let (rect, union) = frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Crossfade, 0.5).unwrap();
        assert_eq!(union, placement(3.0, 2.0, 0.0, 2.0));
        assert_eq!((rect.layer, rect.x, rect.y, rect.width, rect.height),
            (IN_BETWEEN_LAYER, FRAME_GUTTER, IN_BETWEEN_SLOT_HEIGHT + FRAME_GUTTER, 3, 2));
        // The same point again, or one within its step, reuses the upload.
        assert_eq!(frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Crossfade, 0.5).unwrap().0, rect);
        assert_eq!(frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Crossfade, 0.501).unwrap().0, rect);
        assert_eq!(atlas.staged_uploads(), 1);
        frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Crossfade, 0.75).unwrap();
        assert_eq!(atlas.staged_uploads(), 2);
        // Off, or right on a frame, the frame shows as it is.
        assert!(frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Off, 0.5).is_none());
        assert!(frame_at(&mut in_betweens, &mut atlas, MobileInBetweenFrames::Crossfade, 0.001).is_none());
        // A clip of one frame shows it as it is.
        let single = Clip { frames: vec![frame(RED, 2, 2)], mirrored: false, frame_secs: CLASSIC_FRAME_SECS };
        let pick = FramePick { index: 0, next: 0, t: 0.5 };
        assert!(in_betweens.frame(0, MobileInBetweenFrames::Crossfade, key, &single, pick, &mut atlas).is_none());
        assert_eq!(atlas.staged_uploads(), 2);
    }

    #[test]
    fn a_transparent_texel_lends_no_colour() {
        let a_pixels = [0, 255, 0, 0];
        let b_pixels = BLUE;
        let a = InBetweenSource { rgba: &a_pixels, placement: placement(1.0, 1.0, 0.0, 0.0) };
        let b = InBetweenSource { rgba: &b_pixels, placement: placement(1.0, 1.0, 0.0, 0.0) };
        let (_, texels) = compose(a, b, 0.5);
        assert_eq!(texels, vec![[0, 0, 255, 128]]);
    }
}
