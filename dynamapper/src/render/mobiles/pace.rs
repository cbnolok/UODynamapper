//! How fast a mobile's walk and run cycles play. The clients play a clip at its own frame time, whatever the speed of
//! the steps, so a big Enhanced creature walking a tile in 0.4 s skates. Matched to the ground, a cycle lasts the time
//! the steps take to cover the clip's stride: how far the body's planted feet travel in one cycle of the clip, measured
//! offline on the frames (`udd-cli tool strides measure`) and stored in the animation packages with the hash of the
//! frames measured, or, for the clips the measurement does not follow, counted by hand ([`CURATED_STRIDES`]).

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
/// long plays no faster than three times and no slower than a third of the clip's own speed. The planted foot
/// measures within about 0.2 tiles of the hand counts, so the bounds only catch an outlier; three times leaves room for
/// the clips that legitimately play that fast: a Classic monster runs its walk at a run's steps (0.37 of its own
/// cycle for the orc on a side facing), and the Enhanced human run plays in 0.35 of its own.
const MIN_OWN_CYCLE_FRACTION: f32 = 1.0 / 3.0;
const MAX_OWN_CYCLE_FRACTION: f32 = 3.0;

/// How far a clip's planted feet travel in one cycle, in side tiles, and whether it was counted by hand
/// ([`CURATED_STRIDES`]) rather than measured into the package.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stride {
    pub tiles: f32,
    pub by_hand: bool,
}

/// A clip paced to the ground: its stride, the time a step takes and the side tiles it covers
/// ([`step_side_tiles`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundStride {
    pub stride: Stride,
    pub step_secs: f32,
    pub step_tiles: f32,
}

/// The side tiles (`uot_gpu_compute::strides`: the 44 screen px of a step on the screen's horizontal, a stride's unit)
/// a step toward `facing` covers: a whole one on the facings that step along the screen's horizontal or vertical (the
/// odd ones), and one over the square root of two on those that step along its diagonals (north, east, south, west:
/// 31.1 px). The art's feet travel as far on the screen a cycle in every facing, so a cycle along a diagonal takes the
/// square root of two times as many steps.
pub fn step_side_tiles(facing: u8) -> f32 {
    if facing % 2 == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 }
}

impl GroundStride {
    /// The seconds of one cycle of a clip whose own cycle lasts `own_cycle_secs`: the clamp bounds a measurement,
    /// not a stride measured by hand.
    pub fn cycle_secs(self, own_cycle_secs: f32) -> f32 {
        let secs = self.stride.tiles / self.step_tiles * self.step_secs;
        if self.stride.by_hand {
            secs
        } else {
            secs.clamp(MIN_OWN_CYCLE_FRACTION * own_cycle_secs, MAX_OWN_CYCLE_FRACTION * own_cycle_secs)
        }
    }
}

/// The clips whose stride stays hand-configured because the planted-foot measurement (`udd-cli tool strides
/// measure`) does not follow them, in place of the package's. Each was counted on 2026-11-22 on the south-west
/// facing (44 px a side tile), from the planted foot's ground contact frame to frame on the frame strips of `udd-cli
/// tool strides inspect --strip` (its `--pair` count on the pairs where the foot is clear, against a hand count of
/// the foot's x), or keeps the clients' timing where no foot touches the ground; the reason says why the tool is not
/// trusted (`docs/agent_notes/mobile_animation_motion.md`, "Curated strides"). The elves and ghosts have the humans'
/// art, and the bodies of an entry share their frames. A curated stride still needs the package's stride record (one
/// without a stride counts), and is used only while the clip's frames hash as that record says.
const CURATED_STRIDES: [CuratedStride; 15] = [
    CuratedStride { source: AnimSource::Classic, bodies: &HUMAN_BODIES, action: CLASSIC_PEOPLE_WALK,
        pace: CuratedPace::Tiles(1.3),
        reason: "counted by hand before the tool, which reads 1.34 but 1.68 on the west facing" },
    CuratedStride { source: AnimSource::Classic, bodies: &HUMAN_BODIES, action: CLASSIC_PEOPLE_RUN,
        pace: CuratedPace::Tiles(2.15),
        reason: "half the pairs lose the foot and the facings read 1.96 to 2.43 (2.10)" },
    CuratedStride { source: AnimSource::Classic, bodies: &CLASSIC_HORSE_BODIES, action: CLASSIC_ACTION_WALK,
        pace: CuratedPace::Tiles(1.05),
        reason: "the trot's diagonal hooves swap within a pair: 3 of 5 pairs follow another hoof (1.38 to 1.73)" },
    CuratedStride { source: AnimSource::Classic, bodies: &CLASSIC_HORSE_BODIES, action: CLASSIC_ACTION_ANIMAL_RUN,
        pace: CuratedPace::Tiles(2.25),
        reason: "five-frame gallop: pairs match another hoof, the facings read 2.3 to 5.2, 2.4 or 4.1 by package" },
    CuratedStride { source: AnimSource::Classic, bodies: &CLASSIC_WOLF_BODIES, action: CLASSIC_ACTION_WALK,
        pace: CuratedPace::Tiles(0.7),
        reason: "five frames, 3 px paws: four-vote pairs read 8 and 11 px, the facings 0.69 to 1.11" },
    CuratedStride { source: AnimSource::Classic, bodies: &CLASSIC_DOG_BODIES, action: CLASSIC_ACTION_WALK,
        pace: CuratedPace::Tiles(0.52),
        reason: "2 px paws: the south and west facings read 0.64 on two and three of five pairs" },
    CuratedStride { source: AnimSource::Classic, bodies: &CLASSIC_FLYING_BODIES, action: CLASSIC_ACTION_FLY,
        pace: CuratedPace::ClientTiming, reason: "flight: no foot touches the ground, the tool follows a wing tip" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_WALK,
        pace: CuratedPace::Tiles(1.4), reason: "counted by hand before the tool; the tool agrees within 0.05" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_RUN,
        pace: CuratedPace::Tiles(1.75),
        reason: "half the pairs show no planted foot and theirs spread 3 to 12 px: the tool reads 1.83 to 2.03" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &HUMAN_BODIES, action: ENHANCED_HUMAN_WAR_WALK,
        pace: CuratedPace::Tiles(1.3), reason: "counted by hand before the tool; the tool reads 1.20" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &ENHANCED_DAEMON_BODIES, action: ENHANCED_ACTION_RUN,
        pace: CuratedPace::Tiles(6.8),
        reason: "the wings sweep through the ground band: the west facing follows them (1.65), the rest 5.2 to 5.8" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &ENHANCED_KIRIN_BODIES, action: ENHANCED_ACTION_RUN,
        pace: CuratedPace::Tiles(6.9),
        reason: "the hooves touch down for one pair, the lift-off pairs read 2 to 8 px: 4.2 to 7.8" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &ENHANCED_KIRIN_BODIES, action: ENHANCED_ACTION_MOUNTED_RUN,
        pace: CuratedPace::Tiles(6.9),
        reason: "the run's frames ridden: the hooves touch down for one pair, the lift-off pairs read 2 to 8 px" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &ENHANCED_OSTARD_BODIES, action: ENHANCED_ACTION_RUN,
        pace: CuratedPace::Tiles(6.2),
        reason: "two of ten pairs on the south-west facing: the stride is the mean of 5.9 and 9.2" },
    CuratedStride { source: AnimSource::Enhanced, bodies: &ENHANCED_FLYING_BODIES, action: ENHANCED_ACTION_RUN,
        pace: CuratedPace::ClientTiming, reason: "flight: no foot touches the ground, the tool follows a wing tip" },
];
/// The male and female humans, their ghosts, and the elves, in both sources.
const HUMAN_BODIES: [u32; 6] = [400, 401, 402, 403, 605, 606];
/// The Classic horses and the nightmare (one set of frames).
const CLASSIC_HORSE_BODIES: [u32; 5] = [177, 200, 204, 226, 228];
/// The Classic timber, dire, grey, white and plain wolves and the hell hound (one set of frames).
const CLASSIC_WOLF_BODIES: [u32; 6] = [23, 25, 27, 34, 98, 225];
const CLASSIC_DOG_BODIES: [u32; 1] = [217];
/// The Classic bodies whose run is their flight: the eagle, the bird, the daemons, the dragons and wyrms, the harpy,
/// the drakes and the wyvern.
const CLASSIC_FLYING_BODIES: [u32; 12] = [5, 6, 9, 10, 12, 30, 49, 59, 60, 61, 62, 106];
const ENHANCED_DAEMON_BODIES: [u32; 2] = [9, 10];
const ENHANCED_KIRIN_BODIES: [u32; 1] = [132];
/// The desert, frenzied and forest ostards.
const ENHANCED_OSTARD_BODIES: [u32; 3] = [210, 218, 219];
/// The Enhanced bodies whose run is a flight: the dragons, the harpy, the white and shadow wyrms, the drakes and the
/// wyvern.
const ENHANCED_FLYING_BODIES: [u32; 8] = [12, 30, 49, 59, 60, 61, 62, 106];
/// The Classic people layout's walk and run (unarmed).
const CLASSIC_PEOPLE_WALK: u16 = 0;
const CLASSIC_PEOPLE_RUN: u16 = 2;
/// The Classic animal and monster layouts' walk, the animals' run and the monsters' flight.
const CLASSIC_ACTION_WALK: u16 = 0;
const CLASSIC_ACTION_ANIMAL_RUN: u16 = 1;
const CLASSIC_ACTION_FLY: u16 = 19;
/// The Enhanced humans' peace walk and run, and their war walk.
const ENHANCED_HUMAN_WALK: u16 = 1;
const ENHANCED_HUMAN_RUN: u16 = 3;
const ENHANCED_HUMAN_WAR_WALK: u16 = 15;
/// The Enhanced creatures' peace run, and a ridden kirin's run.
const ENHANCED_ACTION_RUN: u16 = 24;
const ENHANCED_ACTION_MOUNTED_RUN: u16 = 30;

/// A clip's hand-configured pace: `pace` for `action` of each of `bodies` from `source`, and why the measurement is
/// not trusted for it.
struct CuratedStride {
    source: AnimSource,
    bodies: &'static [u32],
    action: u16,
    pace: CuratedPace,
    reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CuratedPace {
    /// The stride, in side tiles.
    Tiles(f32),
    /// The clip keeps the clients' timing: it covers no ground to match.
    ClientTiming,
}

/// The curated entry for `action` of `body` from `source`, if there is one.
fn curated_stride(source: AnimSource, body: u32, action: u16) -> Option<&'static CuratedStride> {
    CURATED_STRIDES.iter()
        .find(|curated| curated.source == source && curated.action == action && curated.bodies.contains(&body))
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
    /// Curated to keep the clients' timing ([`CuratedPace::ClientTiming`]).
    ClientTiming,
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

/// The check of the stride for `action` of `body` from `source`, the curated one or the one `package` has, its
/// hashing started; none without the package's stride record. A clip curated to the clients' timing needs no check.
fn start_check<P: MobileAnimClips + Send + Sync + 'static>(package: &Arc<P>, source: AnimSource, body: u32,
    action: u16) -> Option<StrideCheck> {
    let curated = curated_stride(source, body, action);
    if let Some(curated) = curated {
        debug!("The {source:?} stride of action {action} of body {body} is curated, {:?}: {}", curated.pace,
            curated.reason);
    }
    let curated = curated.map(|curated| curated.pace);
    if curated == Some(CuratedPace::ClientTiming) {
        return Some(StrideCheck::ClientTiming);
    }
    let record = package.stride_record(body, action)?;
    let stride = match curated {
        Some(CuratedPace::Tiles(tiles)) => Stride { tiles, by_hand: true },
        _ => Stride { tiles: record.stride_tiles?, by_hand: false },
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
    fn a_ground_cycle_lasts_the_stride_in_steps_within_a_third_and_three_times_the_clip_own() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        let cycle = |tiles: f32, step_secs: f32| GroundStride { stride: Stride { tiles, by_hand: false }, step_secs,
            step_tiles: 1.0 }.cycle_secs(OWN_CYCLE_SECS);
        assert!((cycle(2.0, 0.4) - 0.8).abs() < 1e-6);
        assert!((cycle(0.5, 0.4) - MIN_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
        assert!((cycle(8.0, 0.4) - MAX_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
    }

    /// A hand stride paces a cycle shorter than the clamp lets a measured one: 1.5 tiles at a run's steps last 0.3 s,
    /// under a third of a one-second clip's own.
    #[test]
    fn a_hand_stride_paces_the_cycle_outside_the_clamp() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        const RUN_STEP_SECS: f32 = 0.2;
        const TILES: f32 = 1.5;
        let cycle = |by_hand| GroundStride { stride: Stride { tiles: TILES, by_hand }, step_secs: RUN_STEP_SECS,
            step_tiles: 1.0 }.cycle_secs(OWN_CYCLE_SECS);
        assert!((cycle(true) - TILES * RUN_STEP_SECS).abs() < 1e-6);
        assert!((cycle(false) - MIN_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
    }

    /// A step along a screen diagonal covers less of the stride: the cycle takes the square root of two times as long.
    #[test]
    fn a_diagonal_step_covers_a_side_stride_in_more_steps() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        const WALK_STEP_SECS: f32 = 0.4;
        let cycle = |facing| GroundStride { stride: Stride { tiles: 1.0, by_hand: false }, step_secs: WALK_STEP_SECS,
            step_tiles: step_side_tiles(facing) }.cycle_secs(OWN_CYCLE_SECS);
        for facing in [1, 3, 5, 7] {
            assert!((cycle(facing) - WALK_STEP_SECS).abs() < 1e-6, "facing {facing}");
        }
        for facing in [0, 2, 4, 6] {
            assert!((cycle(facing) - WALK_STEP_SECS * 2f32.sqrt()).abs() < 1e-6, "facing {facing}");
        }
    }

    #[test]
    fn the_curated_clips_have_their_hand_strides_in_both_sources() {
        for (source, body, action, pace) in [
            (AnimSource::Classic, 400, CLASSIC_PEOPLE_WALK, CuratedPace::Tiles(1.3)),
            (AnimSource::Classic, 606, CLASSIC_PEOPLE_RUN, CuratedPace::Tiles(2.15)),
            (AnimSource::Classic, 200, CLASSIC_ACTION_WALK, CuratedPace::Tiles(1.05)),
            (AnimSource::Classic, 177, CLASSIC_ACTION_ANIMAL_RUN, CuratedPace::Tiles(2.25)),
            (AnimSource::Classic, 225, CLASSIC_ACTION_WALK, CuratedPace::Tiles(0.7)),
            (AnimSource::Classic, 217, CLASSIC_ACTION_WALK, CuratedPace::Tiles(0.52)),
            (AnimSource::Classic, 12, CLASSIC_ACTION_FLY, CuratedPace::ClientTiming),
            (AnimSource::Enhanced, 401, ENHANCED_HUMAN_WALK, CuratedPace::Tiles(1.4)),
            (AnimSource::Enhanced, 402, ENHANCED_HUMAN_RUN, CuratedPace::Tiles(1.75)),
            (AnimSource::Enhanced, 400, ENHANCED_HUMAN_WAR_WALK, CuratedPace::Tiles(1.3)),
            (AnimSource::Enhanced, 10, ENHANCED_ACTION_RUN, CuratedPace::Tiles(6.8)),
            (AnimSource::Enhanced, 132, ENHANCED_ACTION_MOUNTED_RUN, CuratedPace::Tiles(6.9)),
            (AnimSource::Enhanced, 219, ENHANCED_ACTION_RUN, CuratedPace::Tiles(6.2)),
            (AnimSource::Enhanced, 59, ENHANCED_ACTION_RUN, CuratedPace::ClientTiming),
        ] {
            assert_eq!(curated_pace(source, body, action), Some(pace), "{source:?} body {body} action {action}");
        }
        // Not another body, nor the other source's action numbers, nor a ridden ostard, which the tool follows.
        assert_eq!(curated_pace(AnimSource::Classic, 17, CLASSIC_PEOPLE_WALK), None);
        assert_eq!(curated_pace(AnimSource::Enhanced, 400, CLASSIC_PEOPLE_RUN), None);
        assert_eq!(curated_pace(AnimSource::Classic, 400, ENHANCED_HUMAN_WALK), None);
        assert_eq!(curated_pace(AnimSource::Enhanced, 210, ENHANCED_ACTION_MOUNTED_RUN), None);
    }

    fn curated_pace(source: AnimSource, body: u32, action: u16) -> Option<CuratedPace> {
        curated_stride(source, body, action).map(|curated| curated.pace)
    }

    /// Every entry names its bodies, a reason and a stride, overlaps no other, and paces the cycle on the clamp-free
    /// path: its stride's own seconds, outside the bounds a measured one would be held to.
    #[test]
    fn every_curated_stride_is_unique_explained_and_unclamped() {
        const RUN_STEP_SECS: f32 = 0.2;
        /// How far outside the clamp the clip's own cycle is put, on either side.
        const OUTSIDE_THE_CLAMP: f32 = 2.0;
        for (index, curated) in CURATED_STRIDES.iter().enumerate() {
            let name = format!("{:?} action {} {:?}", curated.source, curated.action, curated.bodies);
            assert!(!curated.bodies.is_empty() && !curated.reason.is_empty(), "{name}");
            for other in &CURATED_STRIDES[index + 1..] {
                assert!(other.source != curated.source || other.action != curated.action
                    || !other.bodies.iter().any(|body| curated.bodies.contains(body)), "{name} overlaps");
            }
            let CuratedPace::Tiles(tiles) = curated.pace else { continue };
            assert!(tiles > 0.0 && tiles.is_finite(), "{name}");
            let stride = GroundStride { stride: Stride { tiles, by_hand: true }, step_secs: RUN_STEP_SECS,
                step_tiles: 1.0 };
            let secs = tiles * RUN_STEP_SECS;
            // Own cycles whose bounds both lie above, then both below, the stride's seconds.
            for own in [secs * MAX_OWN_CYCLE_FRACTION * OUTSIDE_THE_CLAMP,
                secs * MIN_OWN_CYCLE_FRACTION / OUTSIDE_THE_CLAMP] {
                assert!((stride.cycle_secs(own) - secs).abs() < 1e-6, "{name} own {own}");
            }
        }
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
