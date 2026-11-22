//! The clips of the mobiles drawn: the frames of a body's action in one direction, decoded on the background pool and
//! placed in the frame atlas. A clip is asked for every frame it is drawn; one not resident yet starts decoding and
//! draws once placed.

use std::collections::HashMap;
use std::sync::Arc;
use bevy::prelude::*;
use udd_assets::animation::mobile_anim_cc::MobileAnimCcPackage;
use udd_assets::animation::mobile_anim_ec::MobileAnimEcPackage;
use udd_assets::animation::mobile_strides::{MobileAnimClips, StoredFramePixels};
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use super::frame_atlas::{AtlasFormat, AtlasRect, MobileFrameAtlas};
use super::placement::FramePlacement;
use super::rife_in_betweens::InBetweenKey;

/// The Classic client's time per frame of a MUL body's animation (ClassicUO `CHARACTER_ANIMATION_DELAY`); UOP bodies
/// take their sequence's fps (`MobileAnimCcAnimationRecord::fps`).
pub const CLASSIC_FRAME_SECS: f32 = 0.08;

/// Which client's frames a clip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimSource {
    Classic,
    Enhanced,
}

/// A clip: the frames of `action` of `body` facing `direction`, as the client resolves them (0 north, clockwise).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipKey {
    pub source: AnimSource,
    pub body: u32,
    pub action: u16,
    pub direction: u8,
}

/// What a frame atlas entry holds: a clip's frames, a clip's frames with their RIFE in-betweens (rife_in_betweens.rs),
/// or the 2x frames of either (upscales.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AtlasEntry {
    Clip(ClipKey),
    InBetweens(InBetweenKey),
    Upscaled(InBetweenKey),
}

/// A frame as read: its placement and its pixels as its package stores them (`FrameUpload::pixels`).
pub struct DecodedFrame {
    pub placement: FramePlacement,
    pub pixels: Arc<[u8]>,
}

/// A clip as read, before it has room in the atlas of its format.
pub struct DecodedClip {
    pub format: AtlasFormat,
    pub frames: Vec<DecodedFrame>,
    pub mirrored: bool,
    pub frame_secs: f32,
    pub stored: u32,
}

/// A frame of a resident clip.
#[derive(Clone, Copy, Debug)]
pub struct ClipFrame {
    pub rect: AtlasRect,
    pub placement: FramePlacement,
}

/// The point of a clip shown at a time: the frame `index`, the frame after it, `next` (the first after the last, as
/// the clips loop), and the fraction `t` (0..1) of the way from the one to the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePick {
    pub index: usize,
    pub next: usize,
    pub t: f32,
}

/// A clip whose frames are in the atlas.
#[derive(Clone, Debug)]
pub struct Clip {
    pub frames: Vec<ClipFrame>,
    pub mirrored: bool,
    pub frame_secs: f32,
    /// Which stored frames the clip shows, among those of its body and action: a Classic clip's stored direction, an
    /// Enhanced clip's first frame record. A mirrored direction shares its twin's.
    pub stored: u32,
    /// The texels per pixel of the frames' placements: 1, or 2 for an upscaled sequence (upscales.rs).
    pub texel_scale: u32,
}

impl Clip {
    /// The point of the clip `elapsed_secs` into it, which loops.
    pub fn frame_at(&self, elapsed_secs: f32) -> Option<FramePick> {
        self.pick(elapsed_secs / self.frame_secs.max(f32::EPSILON))
    }

    /// The time the clip takes to show its frames once.
    pub fn cycle_secs(&self) -> f32 {
        self.frame_secs * self.frames.len() as f32
    }

    /// The point of the clip `cycles` of its cycle into it (the fraction of a cycle past the whole ones).
    pub fn frame_at_cycles(&self, cycles: f32) -> Option<FramePick> {
        self.pick(cycles * self.frames.len() as f32)
    }

    /// Marks the pages of the clip's frames in use at `now`: one, or several when its frames lie in pieces.
    pub fn touch_pages(&self, atlas: &mut MobileFrameAtlas<AtlasEntry>, now: f64) {
        for frame in &self.frames {
            atlas.touch(frame.rect, now);
        }
    }

    /// The point `frames` frames into the clip (whole frames, and the fraction of the next).
    fn pick(&self, frames: f32) -> Option<FramePick> {
        if self.frames.is_empty() {
            return None;
        }
        let frames = frames.max(0.0);
        let index = frames as usize % self.frames.len();
        Some(FramePick { index, next: (index + 1) % self.frames.len(), t: frames.fract() })
    }
}

enum ClipState {
    Decoding(Task<Option<DecodedClip>>),
    Resident(Clip),
    /// The package has no such clip, or its frames do not fit the atlas (or one of them a page).
    Missing,
}

/// The packages the clips decode from.
#[derive(Clone, Default)]
pub struct AnimPackages {
    pub classic: Option<Arc<MobileAnimCcPackage>>,
    pub enhanced: Option<Arc<MobileAnimEcPackage>>,
}

impl AnimPackages {
    /// The atlas format of `source`'s frames: its package's page format.
    pub fn format(&self, source: AnimSource) -> Option<AtlasFormat> {
        let format = match source {
            AnimSource::Classic => self.classic.as_deref()?.page_pixel_format(),
            AnimSource::Enhanced => self.enhanced.as_deref()?.page_pixel_format(),
        };
        Some(AtlasFormat::of(format))
    }

    /// Whether a package loaded has frames of `format`.
    pub fn has_format(&self, format: AtlasFormat) -> bool {
        [AnimSource::Classic, AnimSource::Enhanced].into_iter().any(|source| self.format(source) == Some(format))
    }

    /// The atlas formats of the packages loaded, Classic's first; RGBA8 without any.
    pub fn formats(&self) -> Vec<AtlasFormat> {
        let formats: Vec<_> = [AnimSource::Classic, AnimSource::Enhanced].into_iter()
            .filter_map(|source| self.format(source)).collect();
        if formats.is_empty() { vec![AtlasFormat::Rgba8] } else { formats }
    }
}

/// The clips asked for, by key.
#[derive(Resource, Default)]
pub struct MobileClips {
    clips: HashMap<ClipKey, ClipState>,
}

impl MobileClips {
    /// The clip of `key` if resident; otherwise starts its decode, and gives none until it is placed.
    pub fn get_or_load(&mut self, key: ClipKey, packages: &AnimPackages) -> Option<&Clip> {
        let state = self.clips.entry(key).or_insert_with(|| {
            let packages = packages.clone();
            ClipState::Decoding(background_pool().spawn(Priority::Visible, "mobile clip decode",
                move || decode_clip(&packages, key)))
        });
        match state {
            ClipState::Resident(clip) => Some(clip),
            _ => None,
        }
    }

    /// Whether the clip of `key` is decoding.
    pub fn loading(&self, key: ClipKey) -> bool {
        matches!(self.clips.get(&key), Some(ClipState::Decoding(_)))
    }

    /// The clip of `key` if resident.
    pub fn resident(&self, key: ClipKey) -> Option<&Clip> {
        match self.clips.get(&key) {
            Some(ClipState::Resident(clip)) => Some(clip),
            _ => None,
        }
    }

    /// Places the decoded clips in `atlas`, and forgets the clips of the pages it emptied for them or for idleness.
    /// The other entries of those pages come back, for their owner to forget.
    pub fn place_decoded(&mut self, atlas: &mut MobileFrameAtlas<AtlasEntry>, now: f64) -> Vec<AtlasEntry> {
        let mut evicted = atlas.release_idle(now);
        for (key, state) in self.clips.iter_mut() {
            let ClipState::Decoding(task) = state else { continue };
            let decoded = match task.poll() {
                TaskPoll::Pending => continue,
                TaskPoll::Ready(Some(decoded)) => decoded,
                TaskPoll::Ready(None) | TaskPoll::Failed => {
                    *state = ClipState::Missing;
                    continue;
                }
            };
            let frames = decoded.frames.iter()
                .map(|frame| (frame.placement.width as u32, frame.placement.height as u32, frame.pixels.clone()))
                .collect::<Vec<_>>();
            *state = match atlas.place_clip(AtlasEntry::Clip(*key), decoded.format, &frames, now) {
                Some((rects, page_evicted)) => {
                    evicted.extend(page_evicted);
                    ClipState::Resident(Clip {
                        frames: rects.into_iter().zip(&decoded.frames)
                            .map(|(rect, frame)| ClipFrame { rect, placement: frame.placement })
                            .collect(),
                        mirrored: decoded.mirrored,
                        frame_secs: decoded.frame_secs,
                        stored: decoded.stored,
                        texel_scale: 1,
                    })
                }
                None => {
                    warn!("The frames of mobile clip {key:?} do not fit the frame atlas.");
                    ClipState::Missing
                }
            };
        }
        self.forget(evicted)
    }

    /// Forgets the resident clips of `evicted`, the entries of pages the atlas emptied, and gives back its other
    /// entries.
    pub fn forget(&mut self, evicted: Vec<AtlasEntry>) -> Vec<AtlasEntry> {
        evicted.into_iter().filter(|entry| {
            let AtlasEntry::Clip(key) = entry else { return true };
            if matches!(self.clips.get(key), Some(ClipState::Resident(_))) {
                self.clips.remove(key);
            }
            false
        }).collect()
    }

    /// Forgets every clip, as when the packages change.
    pub fn clear(&mut self) {
        self.clips.clear();
    }
}

/// The clip of `key` as its package gives it (`MobileAnimClips::stored_clip`: the client's frames, anchors and
/// mirroring, the pixels as stored, RGBA8 or BC7 blocks), timed by its fps.
pub(super) fn decode_clip(packages: &AnimPackages, key: ClipKey) -> Option<DecodedClip> {
    let format = packages.format(key.source)?;
    let clip = match key.source {
        AnimSource::Classic => packages.classic.as_deref()?.stored_clip(key.body, key.action, key.direction),
        AnimSource::Enhanced => packages.enhanced.as_deref()?.stored_clip(key.body, key.action, key.direction),
    };
    let clip = clip.map_err(|error| warn!("Mobile clip {key:?}: {error:#}")).ok()??;
    Some(DecodedClip {
        format,
        frames: clip.frames.into_iter().map(|frame| DecodedFrame {
            placement: FramePlacement {
                width: f32::from(frame.width),
                height: f32::from(frame.height),
                anchor_x: frame.anchor_x as f32,
                anchor_y: frame.anchor_y as f32,
            },
            pixels: Arc::from(match frame.pixels {
                StoredFramePixels::Rgba8(rgba) => rgba,
                StoredFramePixels::Bc7Blocks(blocks) => blocks,
            }),
        }).collect(),
        mirrored: clip.mirrored,
        frame_secs: frame_secs_at(clip.fps),
        stored: clip.stored,
    })
}

/// The time per frame at `fps` frames a second; `CLASSIC_FRAME_SECS` without an fps (Classic MUL bodies, and
/// animations no sequence times).
fn frame_secs_at(fps: f32) -> f32 {
    if fps > 0.0 { fps.recip() } else { CLASSIC_FRAME_SECS }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_frame() -> ClipFrame {
        ClipFrame {
            rect: AtlasRect { format: AtlasFormat::Rgba8, layer: 0, x: 0, y: 0, width: 1, height: 1 },
            placement: FramePlacement { width: 1.0, height: 1.0, anchor_x: 0.0, anchor_y: 0.0 },
        }
    }

    #[test]
    fn a_classic_clip_takes_the_fps_of_its_uop_sequence_and_the_classic_delay_without_one() {
        use udd_assets::animation::mobile_anim_cc::NO_SEQUENCE_FPS;
        const UOP_SEQUENCE_FPS: f32 = 12.0;
        assert!((frame_secs_at(UOP_SEQUENCE_FPS) - 1.0 / UOP_SEQUENCE_FPS).abs() < f32::EPSILON);
        assert_eq!(frame_secs_at(NO_SEQUENCE_FPS), CLASSIC_FRAME_SECS);
    }

    #[test]
    fn a_clip_loops_its_frames() {
        let frame = test_frame();
        let clip = Clip { frames: vec![frame; 4], mirrored: false, frame_secs: CLASSIC_FRAME_SECS, stored: 0, texel_scale: 1 };
        let index_at = |secs: f32| clip.frame_at(secs).unwrap().index;
        assert_eq!(index_at(0.0), 0);
        assert_eq!(index_at(CLASSIC_FRAME_SECS * 1.5), 1);
        assert_eq!(index_at(CLASSIC_FRAME_SECS * 5.5), 1);
    }

    #[test]
    fn a_clip_spreads_its_frames_over_a_cycle() {
        let frame = test_frame();
        let clip = Clip { frames: vec![frame; 10], mirrored: false, frame_secs: CLASSIC_FRAME_SECS, stored: 0, texel_scale: 1 };
        assert!((clip.cycle_secs() - CLASSIC_FRAME_SECS * 10.0).abs() < f32::EPSILON);
        let index_at = |cycles: f32| clip.frame_at_cycles(cycles).unwrap().index;
        assert_eq!(index_at(0.0), 0);
        assert_eq!(index_at(0.55), 5);
        assert_eq!(index_at(0.95), 9);
    }

    #[test]
    fn a_pick_names_the_next_frame_and_the_way_to_it() {
        let clip = Clip { frames: vec![test_frame(); 4], mirrored: false, frame_secs: CLASSIC_FRAME_SECS, stored: 0, texel_scale: 1 };
        let pick = clip.frame_at(CLASSIC_FRAME_SECS * 1.25).unwrap();
        assert_eq!((pick.index, pick.next), (1, 2));
        assert!((pick.t - 0.25).abs() < 1e-4, "{pick:?}");
        // The last frame goes on to the first, as the clip loops.
        let pick = clip.frame_at(CLASSIC_FRAME_SECS * 3.5).unwrap();
        assert_eq!((pick.index, pick.next), (3, 0));
        assert!((pick.t - 0.5).abs() < 1e-4, "{pick:?}");
        assert_eq!(clip.frame_at(0.0).unwrap(), FramePick { index: 0, next: 1, t: 0.0 });

        let pick = clip.frame_at_cycles(0.6).unwrap();
        assert_eq!((pick.index, pick.next), (2, 3));
        assert!((pick.t - 0.4).abs() < 1e-4, "{pick:?}");

        let single = Clip { frames: vec![test_frame()], mirrored: false, frame_secs: CLASSIC_FRAME_SECS, stored: 0, texel_scale: 1 };
        let pick = single.frame_at(CLASSIC_FRAME_SECS * 2.5).unwrap();
        assert_eq!((pick.index, pick.next), (0, 0));
    }
}
