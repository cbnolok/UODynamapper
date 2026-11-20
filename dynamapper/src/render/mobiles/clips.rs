//! The clips of the mobiles drawn: the frames of a body's action in one direction, decoded on the background pool and
//! placed in the frame atlas. A clip is asked for every frame it is drawn; one not resident yet starts decoding and
//! draws once placed.

use std::collections::HashMap;
use std::sync::Arc;
use bevy::prelude::*;
use udd_assets::animation::mobile_anim_cc::{classic_stored_direction, MobileAnimCcPackage};
use udd_assets::animation::mobile_anim_ec::MobileAnimEcPackage;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use super::frame_atlas::{AtlasRect, MobileFrameAtlas};
use super::placement::FramePlacement;

/// The Classic client's time per frame of a mobile's animation (ClassicUO `CHARACTER_ANIMATION_DELAY`).
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

/// A frame as decoded: its placement and its straight-alpha RGBA pixels.
pub struct DecodedFrame {
    pub placement: FramePlacement,
    pub rgba: Arc<[u8]>,
}

/// A clip as decoded, before it has room in the atlas.
pub struct DecodedClip {
    pub frames: Vec<DecodedFrame>,
    pub mirrored: bool,
    pub frame_secs: f32,
}

/// A frame of a resident clip.
#[derive(Clone, Copy, Debug)]
pub struct ClipFrame {
    pub rect: AtlasRect,
    pub placement: FramePlacement,
}

/// A clip whose frames are in the atlas.
#[derive(Clone, Debug)]
pub struct Clip {
    pub frames: Vec<ClipFrame>,
    pub mirrored: bool,
    pub frame_secs: f32,
}

impl Clip {
    /// The frame shown `elapsed_secs` into the clip, which loops.
    pub fn frame_at(&self, elapsed_secs: f32) -> Option<&ClipFrame> {
        if self.frames.is_empty() {
            return None;
        }
        let index = (elapsed_secs / self.frame_secs.max(f32::EPSILON)) as usize % self.frames.len();
        self.frames.get(index)
    }
}

enum ClipState {
    Decoding(Task<Option<DecodedClip>>),
    Resident(Clip),
    /// The package has no such clip, or its frames do not fit a page.
    Missing,
}

/// The packages the clips decode from.
#[derive(Clone, Default)]
pub struct AnimPackages {
    pub classic: Option<Arc<MobileAnimCcPackage>>,
    pub enhanced: Option<Arc<MobileAnimEcPackage>>,
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

    /// Places the decoded clips in `atlas`, and forgets the clips of the pages it emptied for them or for idleness.
    pub fn place_decoded(&mut self, atlas: &mut MobileFrameAtlas<ClipKey>, now: f64) {
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
                .map(|frame| (frame.placement.width as u32, frame.placement.height as u32, frame.rgba.clone()))
                .collect::<Vec<_>>();
            *state = match atlas.place_clip(*key, &frames, now) {
                Some((rects, page_evicted)) => {
                    evicted.extend(page_evicted);
                    ClipState::Resident(Clip {
                        frames: rects.into_iter().zip(&decoded.frames)
                            .map(|(rect, frame)| ClipFrame { rect, placement: frame.placement })
                            .collect(),
                        mirrored: decoded.mirrored,
                        frame_secs: decoded.frame_secs,
                    })
                }
                None => {
                    warn!("The frames of mobile clip {key:?} do not fit a page of the frame atlas.");
                    ClipState::Missing
                }
            };
        }
        for key in evicted {
            if matches!(self.clips.get(&key), Some(ClipState::Resident(_))) {
                self.clips.remove(&key);
            }
        }
    }

    /// Forgets every clip, as when the packages change.
    pub fn clear(&mut self) {
        self.clips.clear();
    }
}

fn decode_clip(packages: &AnimPackages, key: ClipKey) -> Option<DecodedClip> {
    match key.source {
        AnimSource::Classic => decode_classic_clip(packages.classic.as_deref()?, key),
        AnimSource::Enhanced => decode_enhanced_clip(packages.enhanced.as_deref()?, key),
    }
}

/// A Classic clip: the client stores five directions and mirrors the other three (`classic_stored_direction`). A
/// frame's anchor is its centre x from the left and its centre y from the bottom (ClassicUO: the frame's top at the
/// anchor minus its height and centre y).
fn decode_classic_clip(package: &MobileAnimCcPackage, key: ClipKey) -> Option<DecodedClip> {
    let body = u16::try_from(key.body).ok()?;
    let (stored_direction, mirrored) = classic_stored_direction(key.direction)?;
    let animation = package.server_body_animation(body, key.action, stored_direction)?;
    let frames = package.animation_frames(animation).iter().filter(|frame| frame.width > 0 && frame.height > 0)
        .map(|frame| {
            let rgba = package.read_frame_rgba(frame)
                .map_err(|error| warn!("Mobile clip {key:?}: {error:#}")).ok()?;
            Some(DecodedFrame {
                placement: FramePlacement {
                    width: f32::from(frame.width),
                    height: f32::from(frame.height),
                    anchor_x: f32::from(frame.center_x),
                    anchor_y: f32::from(frame.height) + f32::from(frame.center_y),
                },
                rgba: Arc::from(rgba),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!frames.is_empty()).then_some(DecodedClip { frames, mirrored, frame_secs: CLASSIC_FRAME_SECS })
}

/// An Enhanced clip: the package holds every direction, a mirrored one sharing the frames of its stored twin. A
/// frame's anchor is its centre from its top-left corner; the frames shown are those the sequence does not skip.
fn decode_enhanced_clip(package: &MobileAnimEcPackage, key: ClipKey) -> Option<DecodedClip> {
    const FLYING_ANIMATIONS: bool = true;
    let resolved = package.resolve_animation(key.body, key.action, key.direction, FLYING_ANIMATIONS)?;
    let record = resolved.record;
    let frames = package.animation_frames(record).iter().enumerate()
        .filter(|(index, frame)| !record.is_frame_skipped(*index as u32) && frame.width > 0 && frame.height > 0)
        .map(|(_, frame)| {
            let rgba = package.read_frame_rgba(frame)
                .map_err(|error| warn!("Mobile clip {key:?}: {error:#}")).ok()?;
            Some(DecodedFrame {
                placement: FramePlacement {
                    width: f32::from(frame.width),
                    height: f32::from(frame.height),
                    anchor_x: f32::from(frame.center_x),
                    anchor_y: f32::from(frame.center_y),
                },
                rgba: Arc::from(rgba),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let frame_secs = if resolved.fps > 0.0 { resolved.fps.recip() } else { CLASSIC_FRAME_SECS };
    (!frames.is_empty()).then_some(DecodedClip { frames, mirrored: record.is_mirrored(), frame_secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_loops_its_frames() {
        let frame = ClipFrame {
            rect: AtlasRect { layer: 0, x: 0, y: 0, width: 1, height: 1 },
            placement: FramePlacement { width: 1.0, height: 1.0, anchor_x: 0.0, anchor_y: 0.0 },
        };
        let clip = Clip { frames: vec![frame; 4], mirrored: false, frame_secs: CLASSIC_FRAME_SECS };
        let index_at = |secs: f32| {
            let shown = clip.frame_at(secs).unwrap() as *const ClipFrame;
            clip.frames.iter().position(|frame| std::ptr::eq(frame, shown)).unwrap()
        };
        assert_eq!(index_at(0.0), 0);
        assert_eq!(index_at(CLASSIC_FRAME_SECS * 1.5), 1);
        assert_eq!(index_at(CLASSIC_FRAME_SECS * 5.5), 1);
    }
}
