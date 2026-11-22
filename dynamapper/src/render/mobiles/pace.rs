//! How fast a mobile's walk and run cycles play. The clients play a clip at its own frame time, whatever the speed of
//! the steps, so a big Enhanced creature walking a tile in 0.4 s skates. Matched to the ground, a cycle lasts the time
//! the steps take to cover the clip's stride: how far the body's planted feet travel in one cycle of the clip, measured
//! offline on the frames (`udd-cli tool strides measure`) and stored in the animation packages with the hash of the
//! frames measured, or measured by hand for the human clips, which the flow measures short ([`HAND_STRIDES`]).

use std::collections::hash_map::{Entry, HashMap};
use std::sync::Arc;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use udd_assets::animation::mobile_strides::MobileAnimClips;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use super::clips::{AnimPackages, AnimSource};

/// How a mobile's walk and run cycles are paced. Kept in the app-written world rendering settings as
/// `world_rendering.mobile_walk_pace`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileWalkPace {
    /// At the clip's own frame time, as the clients play it.
    ClientTiming,
    /// One cycle in the time the steps take to cover the clip's measured stride.
    #[default]
    MatchTheGround,
}

impl MobileWalkPace {
    pub const ALL: [Self; 2] = [Self::ClientTiming, Self::MatchTheGround];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClientTiming => "Client timing",
            Self::MatchTheGround => "Match the ground",
        }
    }
}

/// The bounds of a cycle paced to the ground, as fractions of the clip's own cycle: a stride measured too short or too
/// long (small creatures give implausibly short ones) plays no faster than twice and no slower than half the clip's
/// own speed.
const MIN_OWN_CYCLE_FRACTION: f32 = 0.5;
const MAX_OWN_CYCLE_FRACTION: f32 = 2.0;

/// How far a clip's planted feet travel in one cycle, in tiles, and whether it was measured by hand
/// ([`HAND_STRIDES`]) rather than by the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stride {
    pub tiles: f32,
    pub by_hand: bool,
}

/// A clip paced to the ground: its stride and the time a step takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundStride {
    pub stride: Stride,
    pub step_secs: f32,
}

impl GroundStride {
    /// The seconds of one cycle of a clip whose own cycle lasts `own_cycle_secs`: the clamp bounds a flow
    /// measurement, not a stride measured by hand.
    pub fn cycle_secs(self, own_cycle_secs: f32) -> f32 {
        let secs = self.stride.tiles * self.step_secs;
        if self.stride.by_hand {
            secs
        } else {
            secs.clamp(MIN_OWN_CYCLE_FRACTION * own_cycle_secs, MAX_OWN_CYCLE_FRACTION * own_cycle_secs)
        }
    }
}

/// Strides measured by hand for the walk and run clips named, which take the place of the flow's (`udd-cli tool
/// strides measure`). The flow under-measures the human clips: the Classic walk and run measure 0.78 and 1.40 tiles
/// where the planted foot travels about 1.3 and 2.15, and the Enhanced walk and run 1.16 and 1.24 where it travels
/// about 1.4 and 1.75. Paced by the flow, the legs moved a fifth to a third faster than the ground. Each was
/// measured on the south-west facing (44 px a tile) by following the planted foot's ground contact from frame to
/// frame, on 2026-11-22 (`docs/agent_notes/mobile_animation_motion.md`, "Walk pace"). The elves and ghosts have the
/// humans' art and measure the same. A hand stride still needs the package's stride record, and is used only while
/// the clip's frames hash as that record says.
const HAND_STRIDES: [HandStride; 5] = [
    HandStride { source: AnimSource::Classic, bodies: &HUMAN_BODIES, action: CLASSIC_PEOPLE_WALK, tiles: 1.3 },
    HandStride { source: AnimSource::Classic, bodies: &HUMAN_BODIES, action: CLASSIC_PEOPLE_RUN, tiles: 2.15 },
    HandStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_WALK, tiles: 1.4 },
    HandStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_RUN, tiles: 1.75 },
    HandStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_WAR_WALK,
        tiles: 1.3 },
];
/// The male and female humans, their ghosts, and the elves, in both sources.
const HUMAN_BODIES: [u32; 6] = [400, 401, 402, 403, 605, 606];
/// The Classic people layout's walk and run (unarmed).
const CLASSIC_PEOPLE_WALK: u16 = 0;
const CLASSIC_PEOPLE_RUN: u16 = 2;
/// The Enhanced humans' peace walk and run, and their war walk.
const ENHANCED_HUMAN_WALK: u16 = 1;
const ENHANCED_HUMAN_RUN: u16 = 3;
const ENHANCED_HUMAN_WAR_WALK: u16 = 15;

/// A stride measured by hand: `tiles` a cycle for `action` of each of `bodies` from `source`.
struct HandStride {
    source: AnimSource,
    bodies: &'static [u32],
    action: u16,
    tiles: f32,
}

/// The stride measured by hand for `action` of `body` from `source`, if there is one.
fn hand_stride(source: AnimSource, body: u32, action: u16) -> Option<f32> {
    HAND_STRIDES.iter()
        .find(|hand| hand.source == source && hand.action == action && hand.bodies.contains(&body))
        .map(|hand| hand.tiles)
}

/// The point of a mobile's cycle, in cycles, advanced by the time passing over the cycle's length then: a cycle that
/// changes length moves the frames on from where they are, without a jump.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MobileCycleClock {
    /// 0..1.
    cycles: f32,
    /// The seconds into the clip at the last advance (`MobileMotion::clip_secs`).
    clip_secs: f32,
}

impl MobileCycleClock {
    /// The point `clip_secs` into the clip when a cycle lasts `cycle_secs`; a clip that started again (`clip_secs`
    /// went back) starts the cycle again.
    pub fn advance(&mut self, clip_secs: f32, cycle_secs: f32) -> f32 {
        let cycle_secs = cycle_secs.max(f32::EPSILON);
        self.cycles = if clip_secs < self.clip_secs {
            clip_secs / cycle_secs
        } else {
            self.cycles + (clip_secs - self.clip_secs) / cycle_secs
        }
        .rem_euclid(1.0);
        self.clip_secs = clip_secs;
        self.cycles
    }
}

/// The measured strides of the walk and run clips (and an Enhanced mount's mounted moves), by source, server body and
/// the action the client tables give for the movement, from the animation packages (`metadata/strides.bin`). A stride
/// is used only once the clip's frames are checked to hash as those it was measured on: the first time a clip asks,
/// its side facings hash on the background pool, and it keeps the clients' timing until they match.
#[derive(Resource, Default)]
pub struct MobileStrides {
    packages: AnimPackages,
    checks: HashMap<(AnimSource, u32, u16), StrideCheck>,
}

/// Where the check of a clip's frames against its stride stands.
enum StrideCheck {
    Hashing { stride: Stride, clip_hash: u64, task: Task<Option<u64>> },
    Matches(Stride),
    /// The frames are not those measured, or do not read: the clip keeps the clients' timing.
    Differs,
}

impl StrideCheck {
    /// The check once the frames hash `hash_now` (`None` without frames, or when they do not read).
    fn settle(key: (AnimSource, u32, u16), stride: Stride, clip_hash: u64, hash_now: Option<u64>) -> Self {
        if hash_now == Some(clip_hash) {
            return Self::Matches(stride);
        }
        let (source, body, action) = key;
        debug!("The {source:?} frames of action {action} of body {body} are not those its stride was measured on \
            (hash {hash_now:x?}, measured {clip_hash:016x}): the clip keeps the clients' timing until the strides are \
            measured again.");
        Self::Differs
    }
}

impl MobileStrides {
    /// The strides of `packages`, none checked yet.
    pub fn new(packages: &AnimPackages) -> Self {
        Self { packages: packages.clone(), checks: HashMap::new() }
    }

    /// The stride of `action` of server `body` from `source` once its frames are checked.
    pub fn get(&mut self, source: AnimSource, body: u32, action: u16) -> Option<Stride> {
        let key = (source, body, action);
        let check = match self.checks.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let check = match source {
                    AnimSource::Classic => start_check(self.packages.classic.as_ref()?, source, body, action),
                    AnimSource::Enhanced => start_check(self.packages.enhanced.as_ref()?, source, body, action),
                }?;
                entry.insert(check)
            }
        };
        if let StrideCheck::Hashing { stride, clip_hash, task } = check {
            let hash_now = match task.poll() {
                TaskPoll::Pending => return None,
                TaskPoll::Ready(hash_now) => hash_now,
                TaskPoll::Failed => None,
            };
            *check = StrideCheck::settle(key, *stride, *clip_hash, hash_now);
        }
        match check {
            StrideCheck::Matches(stride) => Some(*stride),
            _ => None,
        }
    }
}

/// The check of the stride for `action` of `body` from `source`, the one measured by hand or the one `package`
/// has, its hashing started; none without the package's stride record.
fn start_check<P: MobileAnimClips + Send + Sync + 'static>(package: &Arc<P>, source: AnimSource, body: u32,
    action: u16) -> Option<StrideCheck> {
    let record = package.stride_record(body, action)?;
    let stride = match hand_stride(source, body, action) {
        Some(tiles) => Stride { tiles, by_hand: true },
        None => Stride { tiles: record.stride_tiles?, by_hand: false },
    };
    let package = package.clone();
    let task = background_pool().spawn(Priority::Visible, "mobile stride check", move || {
        package.stride_clip_hash(body, action)
            .map_err(|error| warn!("The frames of action {action} of body {body} do not read: {error:#}"))
            .ok().flatten()
    });
    Some(StrideCheck::Hashing { stride, clip_hash: record.clip_hash, task })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ground_cycle_lasts_the_stride_in_steps_within_half_and_twice_the_clip_own() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        let cycle = |tiles: f32, step_secs: f32|
            GroundStride { stride: Stride { tiles, by_hand: false }, step_secs }.cycle_secs(OWN_CYCLE_SECS);
        assert!((cycle(2.0, 0.4) - 0.8).abs() < 1e-6);
        assert!((cycle(0.5, 0.4) - MIN_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
        assert!((cycle(8.0, 0.4) - MAX_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
    }

    /// The Enhanced human run, 9 frames at 9 fps: its hand stride paces a cycle in 0.35 s at a run's steps, a third
    /// of its own, which the clamp would hold at half.
    #[test]
    fn a_hand_stride_paces_the_cycle_outside_the_clamp() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        const RUN_STEP_SECS: f32 = 0.2;
        const TILES: f32 = 1.75;
        let cycle = |by_hand| GroundStride { stride: Stride { tiles: TILES, by_hand }, step_secs: RUN_STEP_SECS }
            .cycle_secs(OWN_CYCLE_SECS);
        assert!((cycle(true) - TILES * RUN_STEP_SECS).abs() < 1e-6);
        assert!((cycle(false) - MIN_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
    }

    #[test]
    fn the_human_walk_and_run_have_hand_strides_in_both_sources() {
        for (source, body, action, tiles) in [
            (AnimSource::Classic, 400, CLASSIC_PEOPLE_WALK, 1.3),
            (AnimSource::Classic, 606, CLASSIC_PEOPLE_RUN, 2.15),
            (AnimSource::Enhanced, 401, ENHANCED_HUMAN_WALK, 1.4),
            (AnimSource::Enhanced, 402, ENHANCED_HUMAN_RUN, 1.75),
            (AnimSource::Enhanced, 400, ENHANCED_HUMAN_WAR_WALK, 1.3),
        ] {
            assert_eq!(hand_stride(source, body, action), Some(tiles), "{source:?} body {body} action {action}");
        }
        // Not another body, nor the other source's action numbers.
        assert_eq!(hand_stride(AnimSource::Classic, 17, CLASSIC_PEOPLE_WALK), None);
        assert_eq!(hand_stride(AnimSource::Enhanced, 400, CLASSIC_PEOPLE_RUN), None);
        assert_eq!(hand_stride(AnimSource::Classic, 400, ENHANCED_HUMAN_WALK), None);
    }

    #[test]
    fn the_cycle_clock_moves_on_without_a_jump_when_the_cycle_changes() {
        let mut clock = MobileCycleClock::default();
        assert!((clock.advance(0.25, 1.0) - 0.25).abs() < 1e-6);
        // Twice as long a cycle: the next quarter second moves it an eighth on from where it was.
        assert!((clock.advance(0.5, 2.0) - 0.375).abs() < 1e-6);
        // It loops.
        assert!((clock.advance(1.5, 1.0) - 0.375).abs() < 1e-6);
        // A clip that starts again starts the cycle again.
        assert!((clock.advance(0.1, 1.0) - 0.1).abs() < 1e-6);
    }

    /// A stride is used only for the frames it was measured on: a clip whose frames hash otherwise, or do not read,
    /// keeps the clients' timing, and so does one still hashing, until it matches.
    #[test]
    fn a_stride_is_used_only_while_the_frames_hash_as_measured() {
        const KEY: (AnimSource, u32, u16) = (AnimSource::Enhanced, 200, 0);
        const TILES: f32 = 1.5;
        const STRIDE: Stride = Stride { tiles: TILES, by_hand: false };
        const MEASURED: u64 = 0x1234;
        const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        let (source, body, action) = KEY;
        let mut strides = MobileStrides::default();
        for (hash_now, expected) in [(Some(MEASURED), Some(STRIDE)), (Some(MEASURED + 1), None), (None, None)] {
            let (hashed, hashing) = std::sync::mpsc::channel::<()>();
            let task = background_pool().spawn(Priority::Visible, "test stride check", move || {
                hashing.recv_timeout(CHECK_TIMEOUT).ok()?;
                hash_now
            });
            strides.checks.insert(KEY, StrideCheck::Hashing { stride: STRIDE, clip_hash: MEASURED, task });
            assert_eq!(strides.get(source, body, action), None, "no stride while hashing");
            hashed.send(()).unwrap();
            let started = std::time::Instant::now();
            while !matches!(strides.checks.get(&KEY), Some(StrideCheck::Hashing { task, .. }) if task.is_finished()) {
                assert!(started.elapsed() < CHECK_TIMEOUT, "the check never ran");
                std::thread::yield_now();
            }
            assert_eq!(strides.get(source, body, action), expected, "frames hashing {hash_now:x?}");
            assert_eq!(strides.get(source, body, action), expected, "settled");
        }
        // Without the packages, no stride.
        assert_eq!(strides.get(AnimSource::Classic, body, action), None);
    }
}
