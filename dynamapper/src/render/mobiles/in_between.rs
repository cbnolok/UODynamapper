//! The in-between frames of the mobiles: between two frames of a clip, both frames on one card, crossfaded by the time
//! between them, so an animation of 8 to 12 frames a second does not step on a faster display. The CPU picks the two
//! frames and the weight of the next for each mobile frame drawn (`layer_frames`); the art sprite shader samples both
//! and blends them (art/mobile_frames.wesl), so an in-between costs no pixels composed or uploaded at any refresh
//! rate. The packages are untouched: the in-betweens come from the frames alone. The RIFE modes add frames made by
//! the RIFE network between each two (rife_in_betweens.rs), and crossfade the clip of both alike.

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use super::clips::{Clip, ClipFrame, FramePick};
use super::frame_atlas::MOBILE_ATLAS_PAGE_SIZE;
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
    /// One frame made by RIFE between each two, at the middle; the shorter frames crossfade.
    RifeX2,
    /// Three frames made by RIFE between each two, at the quarters; the shorter frames crossfade.
    RifeX4,
}

impl MobileInBetweenFrames {
    pub const ALL: [Self; 4] = [Self::Off, Self::Crossfade, Self::RifeX2, Self::RifeX4];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Crossfade => "Crossfade",
            Self::RifeX2 => "RIFE x2",
            Self::RifeX4 => "RIFE x4",
        }
    }

    /// The frames a RIFE mode shows for each of a clip's, its own and the in-betweens after it.
    pub fn rife_factor(self) -> Option<u32> {
        match self {
            Self::Off | Self::Crossfade => None,
            Self::RifeX2 => Some(2),
            Self::RifeX4 => Some(4),
        }
    }
}

/// The middle part of the time between two frames that blends them: before it the first frame shows as it is, after
/// it the next one, and the blend eases in and out (smoothstep) across it. A blend over the whole time leaves a moving
/// leg as two half-transparent copies for long, so thin and washed out; a shorter one keeps that state brief. Chosen
/// by eye on the Enhanced horse's walk.
const CROSSFADE_SPAN: f32 = 0.5;

/// The blend weight of the next frame at `t` (0..1) between two frames: 0 to 1 across `CROSSFADE_SPAN`, eased.
fn crossfade_weight(t: f32) -> f32 {
    let start = (1.0 - CROSSFADE_SPAN) / 2.0;
    let x = ((t - start) / CROSSFADE_SPAN).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The placement holding both frames with their anchors on one point: the anchor of each frame is at the union's.
pub fn union_placement(a: FramePlacement, b: FramePlacement) -> FramePlacement {
    let left = a.anchor_x.max(b.anchor_x);
    let right = (a.width - a.anchor_x).max(b.width - b.anchor_x);
    let up = a.anchor_y.max(b.anchor_y);
    let down = (a.height - a.anchor_y).max(b.height - b.anchor_y);
    FramePlacement { width: left + right, height: up + down, anchor_x: left, anchor_y: up }
}

/// The crossfade of a mobile frame drawn, as the art sprite shader reads it after the frame's sprite instance
/// (`MobileSpriteInstance`, art/mobile_frames.wesl), in the 0..1 coordinates of an atlas page. While two frames blend,
/// the sprite's card holds both (`union_placement`) and its `uv_min`..`uv_max` span the card in the coordinates of the
/// first frame; the next frame lies `next_uv_offset` from them, on the same page, as a clip's frames all do. Each frame
/// shows inside its own rectangle only. A weight of 0 draws the sprite's frame alone, as a static's.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FrameCrossfade {
    /// The first frame's rectangle.
    pub frame_uv_min: [f32; 2],
    pub frame_uv_max: [f32; 2],
    /// The next frame's rectangle.
    pub next_uv_min: [f32; 2],
    pub next_uv_max: [f32; 2],
    /// The next frame's coordinates less the first frame's at the same point of the card.
    pub next_uv_offset: [f32; 2],
    /// The next frame's blend weight, 0..1 (`crossfade_weight`).
    pub next_weight: f32,
    pub _pad: f32,
}

/// What a mobile frame drawn shows: its card's placement, the card's span on its atlas page and the crossfade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerFrames {
    pub layer: u32,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    pub placement: FramePlacement,
    pub crossfade: FrameCrossfade,
}

impl LayerFrames {
    /// `frame` alone, its card its own.
    fn alone(frame: &ClipFrame) -> Self {
        let (uv_min, uv_max) = frame.rect.uv();
        Self {
            layer: frame.rect.layer,
            uv_min,
            uv_max,
            placement: frame.placement,
            crossfade: FrameCrossfade { frame_uv_min: uv_min.to_array(), frame_uv_max: uv_max.to_array(),
                ..default() },
        }
    }
}

/// What a mobile frame drawn shows for `pick` of `clip`: with `mode` off, a clip of one frame or `t` outside the blend,
/// the frame `pick` stands at alone, else both frames on their union, blended by `crossfade_weight`. None when the
/// clip has no such frame.
pub fn layer_frames(mode: MobileInBetweenFrames, clip: &Clip, pick: FramePick) -> Option<LayerFrames> {
    let frame = clip.frames.get(pick.index)?;
    if mode == MobileInBetweenFrames::Off || clip.frames.len() < 2 {
        return Some(LayerFrames::alone(frame));
    }
    let weight = crossfade_weight(pick.t);
    let next = clip.frames.get(pick.next)?;
    if weight <= 0.0 {
        return Some(LayerFrames::alone(frame));
    }
    if weight >= 1.0 {
        return Some(LayerFrames::alone(next));
    }
    let card = union_placement(frame.placement, next.placement);
    // The card's top-left corner in the texels of a frame's page: the frame's corner less its place on the card.
    let card_origin = |frame: &ClipFrame| Vec2::new(frame.rect.x as f32 - (card.anchor_x - frame.placement.anchor_x),
        frame.rect.y as f32 - (card.anchor_y - frame.placement.anchor_y));
    let page_size = MOBILE_ATLAS_PAGE_SIZE as f32;
    let uv_min = card_origin(frame) / page_size;
    let (frame_uv_min, frame_uv_max) = frame.rect.uv();
    let (next_uv_min, next_uv_max) = next.rect.uv();
    Some(LayerFrames {
        layer: frame.rect.layer,
        uv_min,
        uv_max: uv_min + Vec2::new(card.width, card.height) / page_size,
        placement: card,
        crossfade: FrameCrossfade {
            frame_uv_min: frame_uv_min.to_array(),
            frame_uv_max: frame_uv_max.to_array(),
            next_uv_min: next_uv_min.to_array(),
            next_uv_max: next_uv_max.to_array(),
            next_uv_offset: ((card_origin(next) - card_origin(frame)) / page_size).to_array(),
            next_weight: weight,
            _pad: 0.0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::clips::CLASSIC_FRAME_SECS;
    use super::super::frame_atlas::AtlasRect;

    fn placement(width: f32, height: f32, anchor_x: f32, anchor_y: f32) -> FramePlacement {
        FramePlacement { width, height, anchor_x, anchor_y }
    }

    #[test]
    fn the_union_holds_both_frames_on_their_anchors() {
        // a: 2 left and 1 right of its anchor; b: 0 left and 4 right. Up 3 and 2, down 0 and 2.
        let union = union_placement(placement(3.0, 3.0, 2.0, 3.0), placement(4.0, 4.0, 0.0, 2.0));
        assert_eq!(union, placement(6.0, 5.0, 2.0, 3.0));
    }

    #[test]
    fn the_blend_spans_the_middle_of_the_time_between_frames() {
        let start = (1.0 - CROSSFADE_SPAN) / 2.0;
        assert_eq!(crossfade_weight(0.0), 0.0);
        assert_eq!(crossfade_weight(start), 0.0);
        assert!((crossfade_weight(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(crossfade_weight(1.0 - start), 1.0);
        assert_eq!(crossfade_weight(1.0), 1.0);
    }

    /// Two frames on one page: a, 3 by 3 at (10, 20) anchored at (2, 3); b, 4 by 4 at (40, 20) anchored at (0, 2).
    fn two_frame_clip() -> Clip {
        let frame = |x: u32, width: u32, height: u32, anchor_x: f32, anchor_y: f32| ClipFrame {
            rect: AtlasRect { layer: 3, x, y: 20, width, height },
            placement: placement(width as f32, height as f32, anchor_x, anchor_y),
        };
        Clip { frames: vec![frame(10, 3, 3, 2.0, 3.0), frame(40, 4, 4, 0.0, 2.0)], mirrored: false,
            frame_secs: CLASSIC_FRAME_SECS, stored: 0 }
    }

    #[test]
    fn between_frames_the_card_holds_both_on_their_anchors() {
        let clip = two_frame_clip();
        let pick = FramePick { index: 0, next: 1, t: 0.5 };
        let drawn = layer_frames(MobileInBetweenFrames::Crossfade, &clip, pick).unwrap();
        assert_eq!(drawn.placement, placement(6.0, 5.0, 2.0, 3.0));
        assert!((drawn.crossfade.next_weight - 0.5).abs() < 1e-6);
        assert_eq!(drawn.layer, 3);
        let page_size = MOBILE_ATLAS_PAGE_SIZE as f32;
        let texels = |uv: [f32; 2]| (Vec2::from_array(uv) * page_size).round();
        // The card spans 6 by 5 texels; a lies at its corner, so the card starts at a's corner on a's page.
        assert_eq!(texels(drawn.uv_min.to_array()), Vec2::new(10.0, 20.0));
        assert_eq!(texels(drawn.uv_max.to_array()), Vec2::new(16.0, 25.0));
        // Each anchor lands on the card's: a's at (12, 23) of its page, b's at (40, 22) of its.
        let card_anchor = drawn.uv_min * page_size + Vec2::new(drawn.placement.anchor_x, drawn.placement.anchor_y);
        assert_eq!(card_anchor.round(), Vec2::new(12.0, 23.0));
        assert_eq!((card_anchor + texels(drawn.crossfade.next_uv_offset)).round(), Vec2::new(40.0, 22.0));
        // Each frame shows inside its own rectangle.
        assert_eq!((texels(drawn.crossfade.frame_uv_min), texels(drawn.crossfade.frame_uv_max)),
            (Vec2::new(10.0, 20.0), Vec2::new(13.0, 23.0)));
        assert_eq!((texels(drawn.crossfade.next_uv_min), texels(drawn.crossfade.next_uv_max)),
            (Vec2::new(40.0, 20.0), Vec2::new(44.0, 24.0)));
    }

    #[test]
    fn outside_the_blend_or_off_a_frame_shows_alone() {
        let clip = two_frame_clip();
        let alone = |frame: &ClipFrame| LayerFrames::alone(frame);
        let at = |mode, t: f32| layer_frames(mode, &clip, FramePick { index: 0, next: 1, t }).unwrap();
        // Off, or before the blend, the first frame; after it, the next.
        assert_eq!(at(MobileInBetweenFrames::Off, 0.5), alone(&clip.frames[0]));
        assert_eq!(at(MobileInBetweenFrames::Crossfade, 0.1), alone(&clip.frames[0]));
        assert_eq!(at(MobileInBetweenFrames::Crossfade, 0.9), alone(&clip.frames[1]));
        // Alone, the card is the frame's rectangle and the weight 0.
        let first = at(MobileInBetweenFrames::Off, 0.5);
        assert_eq!((first.uv_min, first.uv_max), clip.frames[0].rect.uv());
        assert_eq!(first.placement, clip.frames[0].placement);
        assert_eq!(first.crossfade.next_weight, 0.0);
        assert_eq!((Vec2::from_array(first.crossfade.frame_uv_min), Vec2::from_array(first.crossfade.frame_uv_max)),
            clip.frames[0].rect.uv());
        // A clip of one frame shows it alone.
        let single = Clip { frames: vec![clip.frames[0]], mirrored: false, frame_secs: CLASSIC_FRAME_SECS, stored: 0 };
        let pick = FramePick { index: 0, next: 0, t: 0.5 };
        assert_eq!(layer_frames(MobileInBetweenFrames::Crossfade, &single, pick).unwrap(), alone(&clip.frames[0]));
    }

    #[test]
    fn the_shader_reads_the_crossfade_as_wgsl_lays_it_out() {
        // Five vec2<f32>, then two f32 (art/art_bindings.wesl).
        const WGSL_FRAME_CROSSFADE_SIZE: usize = 48;
        assert_eq!(std::mem::size_of::<FrameCrossfade>(), WGSL_FRAME_CROSSFADE_SIZE);
    }
}
