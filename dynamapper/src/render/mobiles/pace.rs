//! How fast a mobile's walk and run cycles play. The clients play a clip at its own frame time, whatever the speed of
//! the steps, so a big Enhanced creature walking a tile in 0.4 s skates. Matched to the ground, a cycle lasts the time
//! the steps take to cover the clip's stride: how far the body's planted feet travel in one cycle of the clip, measured
//! offline on the frames (`udd-cli tool strides measure`) and stored in the animation packages with the hash of the
//! frames measured, or, for the clips the measurement does not follow, counted by hand (`runtime_specs/mobiles/
//! Strides.kdl`, [`CuratedStrides`]).

use std::collections::hash_map::{Entry, HashMap};
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use bevy::prelude::*;
use color_eyre::eyre;
use serde::{Deserialize, Serialize};
use udd_assets::animation::mobile_strides::MobileAnimClips;
use uot_kdl::ast::Node;
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::settings::runtime_specs::SpecFiles;
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
/// ([`CuratedStrides`]) rather than measured into the package.
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

/// The folder of the curated strides under `runtime_specs`, and the file's name: the clips whose stride stays
/// hand-configured because the planted-foot measurement (`udd-cli tool strides measure`) does not follow them, in place
/// of the package's, each with its reason as a comment (`docs/agent_notes/mobile_animation_motion.md`, "Curated
/// strides"). A curated stride still needs the package's stride record (one without a stride counts), and is used only
/// while the clip's frames hash as that record says. The file is hand-written and read strictly (`uot_kdl::strict`):
/// every error names its line. It is read at startup, and again when the user's copy under the cache dir, which takes
/// the bundled file's place (`settings::runtime_specs`), is saved.
pub const STRIDES_SPECS_FOLDER: &str = "mobiles";
pub const STRIDES_FILE_NAME: &str = "Strides.kdl";
/// What the log calls the file.
const STRIDES_WHAT: &str = "the curated strides";
/// How often the user's copy is checked for a save.
const STRIDES_FILE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// The strides the file accepts, in side tiles: the curated ones are 0.52 to 6.9, the tool's widest measured 9.2.
pub const CURATED_STRIDE_TILES: RangeInclusive<f32> = 0.1..=12.0;
/// The action ids the file accepts lie below this: the clients' tables number the movements below it in both
/// layouts (`uocf`'s `MAX_ACTION_ID_FALLBACK`).
const CURATED_ACTION_LIMIT: u16 = 100;

const STRIDE_NODE: &str = "stride";
const CLIENT_TIMING_NODE: &str = "client-timing";
const PACKAGE_PROPERTY: &str = "package";
const ACTION_PROPERTY: &str = "action";
const TILES_PROPERTY: &str = "tiles";
/// The `package` values: the Classic and the Enhanced animation package.
const CLASSIC_PACKAGE: &str = "cc";
const ENHANCED_PACKAGE: &str = "ec";
const STRIDES_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(STRIDE_NODE, &[PACKAGE_PROPERTY, ACTION_PROPERTY, TILES_PROPERTY]),
    NodeSchema::leaf(CLIENT_TIMING_NODE, &[PACKAGE_PROPERTY, ACTION_PROPERTY]),
];

/// A clip's hand-configured pace: `pace` for `action` of each of the server `bodies` from `source`.
#[derive(Clone, Debug, PartialEq)]
struct CuratedStride {
    source: AnimSource,
    bodies: Vec<u32>,
    action: u16,
    pace: CuratedPace,
    /// The entry's 1-based line, for the log and the error of a clip listed twice.
    line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CuratedPace {
    /// The stride, in side tiles.
    Tiles(f32),
    /// The clip keeps the clients' timing: it covers no ground to match.
    ClientTiming,
}

/// The curated strides, as read from `source_name`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CuratedStrides {
    source_name: String,
    entries: Vec<CuratedStride>,
}

impl CuratedStrides {
    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        let mut entries: Vec<CuratedStride> = Vec::new();
        for node in &document.nodes {
            let entry = strict::node_schema(node, None, STRIDES_SCHEMA)
                .and_then(|schema| decode_curated_stride(node, schema))
                .map_err(|error| strict::in_file(source_name, error))?;
            let listed = entries.iter()
                .filter(|earlier| earlier.source == entry.source && earlier.action == entry.action)
                .find_map(|earlier| entry.bodies.iter().find(|body| earlier.bodies.contains(body))
                    .map(|body| (*body, earlier.line)));
            if let Some((body, line)) = listed {
                let error = strict::error_at(node.node_name.span(), format!(
                    "action {} of body {body} from package \"{}\" is listed already, on line {line}", entry.action,
                    package_name(entry.source)));
                return Err(strict::in_file(source_name, error));
            }
            entries.push(entry);
        }
        Ok(Self { source_name: source_name.to_string(), entries })
    }

    /// The entry for `action` of `body` from `source`, if there is one.
    fn get(&self, source: AnimSource, body: u32, action: u16) -> Option<&CuratedStride> {
        self.entries.iter()
            .find(|curated| curated.source == source && curated.action == action && curated.bodies.contains(&body))
    }
}

/// The `package` value naming `source`.
fn package_name(source: AnimSource) -> &'static str {
    match source {
        AnimSource::Classic => CLASSIC_PACKAGE,
        AnimSource::Enhanced => ENHANCED_PACKAGE,
    }
}

fn decode_curated_stride(node: &Node<LineSpan>, schema: &NodeSchema) -> eyre::Result<CuratedStride> {
    strict::check_properties(node, schema.properties)?;
    strict::reject_children(node)?;
    let package = strict::required_property(node, PACKAGE_PROPERTY)?;
    let source = match strict::value_string(package, PACKAGE_PROPERTY)?.as_str() {
        CLASSIC_PACKAGE => AnimSource::Classic,
        ENHANCED_PACKAGE => AnimSource::Enhanced,
        other => return Err(strict::error_at(package.literal.span(), format!(
            "{PACKAGE_PROPERTY}: expected \"{CLASSIC_PACKAGE}\" or \"{ENHANCED_PACKAGE}\", found \"{other}\""))),
    };
    let action_value = strict::required_property(node, ACTION_PROPERTY)?;
    let action = strict::value_u16(action_value, ACTION_PROPERTY)?;
    if action >= CURATED_ACTION_LIMIT {
        return Err(strict::error_at(action_value.literal.span(),
            format!("{ACTION_PROPERTY}: {action} is not below {CURATED_ACTION_LIMIT}")));
    }
    let pace = if schema.name == STRIDE_NODE {
        let value = strict::required_property(node, TILES_PROPERTY)?;
        let tiles = strict::value_f32(value, TILES_PROPERTY)?;
        if !CURATED_STRIDE_TILES.contains(&tiles) {
            return Err(strict::error_at(value.literal.span(), format!("{TILES_PROPERTY}: {tiles} is outside {}..={}",
                CURATED_STRIDE_TILES.start(), CURATED_STRIDE_TILES.end())));
        }
        CuratedPace::Tiles(tiles)
    } else {
        CuratedPace::ClientTiming
    };
    if node.arguments.is_empty() {
        return Err(strict::error_at(node.node_name.span(),
            format!("`{}` takes the body ids it applies to, found none", schema.name)));
    }
    let mut bodies = Vec::with_capacity(node.arguments.len());
    for argument in &node.arguments {
        let body = u32::from(strict::value_u16(argument, "the body id")?);
        if bodies.contains(&body) {
            return Err(strict::error_at(argument.literal.span(), format!("body {body} is listed twice")));
        }
        bodies.push(body);
    }
    Ok(CuratedStride { source, bodies, action, pace, line: node.node_name.span().0.line + 1 })
}

/// The modification time of `path`; `None` when it does not exist.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
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
/// its side facings hash on the background pool, and it keeps the clients' timing until they match. The curated
/// strides take the package's place for their clips.
#[derive(Resource, Default)]
pub struct MobileStrides {
    packages: AnimPackages,
    curated: CuratedStrides,
    /// The curated strides' files, and the user copy's modification time when it was last read (`None`: there was
    /// none).
    curated_files: SpecFiles,
    curated_user_modified: Option<SystemTime>,
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
    /// No packages yet, and the curated strides read at startup: the bundled file is required, a missing or invalid
    /// one stops the app; a user copy that does not read leaves it in force, with a warning.
    pub fn load() -> Self {
        let curated_files = SpecFiles::of(STRIDES_SPECS_FOLDER, STRIDES_FILE_NAME);
        let curated_user_modified = modified(&curated_files.user);
        let loaded = curated_files.load(CuratedStrides::decode)
            .unwrap_or_else(|error| crate::settings::exit_on_startup_config_error(&error));
        loaded.report(STRIDES_WHAT);
        Self { curated: loaded.value, curated_files, curated_user_modified, ..default() }
    }

    /// The strides of `packages`, none checked yet.
    pub fn set_packages(&mut self, packages: &AnimPackages) {
        self.packages = packages.clone();
        self.checks.clear();
    }

    /// Reads the curated strides again; the clips are checked again when they changed.
    fn reload_curated(&mut self) {
        self.curated_user_modified = modified(&self.curated_files.user);
        match self.curated_files.load(CuratedStrides::decode) {
            Ok(loaded) => {
                loaded.report(STRIDES_WHAT);
                if loaded.value != self.curated {
                    self.curated = loaded.value;
                    self.checks.clear();
                }
            }
            // The bundled file no longer reads: what is in force stays.
            Err(error) => {
                let message = format!("{error:#}: the curated strides in force stay");
                console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
                crate::ingame_sysmessage_logger::warning(message);
            }
        }
    }

    /// The stride of `action` of server `body` from `source` once its frames are checked.
    pub fn get(&mut self, source: AnimSource, body: u32, action: u16) -> Option<Stride> {
        let key = (source, body, action);
        let check = match self.checks.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let curated = &self.curated;
                let check = match source {
                    AnimSource::Classic => start_check(self.packages.classic.as_ref()?, curated, source, body, action),
                    AnimSource::Enhanced => {
                        start_check(self.packages.enhanced.as_ref()?, curated, source, body, action)
                    }
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

/// Reads the curated strides again when the user's copy is saved, created or deleted.
pub fn sys_watch_curated_strides(mut strides: ResMut<MobileStrides>, time: Res<Time<Real>>,
    mut timer: Local<Option<Timer>>) {
    let timer = timer.get_or_insert_with(|| Timer::new(STRIDES_FILE_CHECK_INTERVAL, TimerMode::Repeating));
    let checked = timer.tick(time.delta()).just_finished();
    if checked && modified(&strides.curated_files.user) != strides.curated_user_modified {
        strides.reload_curated();
    }
}

/// The check of the stride for `action` of `body` from `source`, the one `curated` has or the one `package` has, its
/// hashing started; none without the package's stride record. A clip curated to the clients' timing needs no check.
fn start_check<P: MobileAnimClips + Send + Sync + 'static>(package: &Arc<P>, curated: &CuratedStrides,
    source: AnimSource, body: u32, action: u16) -> Option<StrideCheck> {
    let curated_entry = curated.get(source, body, action);
    if let Some(entry) = curated_entry {
        debug!("The {source:?} stride of action {action} of body {body} is curated, {:?}, on line {} of {}",
            entry.pace, entry.line, curated.source_name);
    }
    let curated = curated_entry.map(|curated| curated.pace);
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

    const STRIDES_FILE: &str = "Strides.kdl";
    /// The entries of the bundled file.
    const BUNDLED_CURATED_STRIDES: usize = 17;
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

    fn decode_strides(text: &str) -> CuratedStrides {
        CuratedStrides::decode(STRIDES_FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    fn bundled_text() -> &'static str {
        include_str!("../../../assets/runtime_specs/mobiles/Strides.kdl")
    }

    #[test]
    fn the_curated_clips_have_their_hand_strides_in_both_sources() {
        let curated = decode_strides(bundled_text());
        let curated_pace = |source, body, action| curated.get(source, body, action).map(|curated| curated.pace);
        for (source, body, action, pace) in [
            (AnimSource::Classic, 400, CLASSIC_PEOPLE_WALK, CuratedPace::Tiles(1.3)),
            (AnimSource::Classic, 606, CLASSIC_PEOPLE_RUN, CuratedPace::Tiles(2.15)),
            (AnimSource::Classic, 200, CLASSIC_ACTION_WALK, CuratedPace::Tiles(1.05)),
            (AnimSource::Classic, 177, CLASSIC_ACTION_ANIMAL_RUN, CuratedPace::Tiles(2.25)),
            (AnimSource::Classic, 225, CLASSIC_ACTION_WALK, CuratedPace::Tiles(0.7)),
            (AnimSource::Classic, 217, CLASSIC_ACTION_WALK, CuratedPace::Tiles(0.52)),
            (AnimSource::Classic, 233, CLASSIC_ACTION_WALK, CuratedPace::Tiles(1.0)),
            (AnimSource::Classic, 12, CLASSIC_ACTION_FLY, CuratedPace::ClientTiming),
            (AnimSource::Enhanced, 401, ENHANCED_HUMAN_WALK, CuratedPace::Tiles(1.4)),
            (AnimSource::Enhanced, 402, ENHANCED_HUMAN_RUN, CuratedPace::Tiles(1.75)),
            (AnimSource::Enhanced, 400, ENHANCED_HUMAN_WAR_WALK, CuratedPace::Tiles(1.3)),
            (AnimSource::Enhanced, 10, ENHANCED_ACTION_RUN, CuratedPace::Tiles(6.8)),
            (AnimSource::Enhanced, 132, ENHANCED_ACTION_MOUNTED_RUN, CuratedPace::Tiles(6.9)),
            (AnimSource::Enhanced, 219, ENHANCED_ACTION_RUN, CuratedPace::Tiles(6.2)),
            (AnimSource::Enhanced, 138, ENHANCED_ACTION_RUN, CuratedPace::Tiles(1.9)),
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

    /// Every entry of the bundled file names its bodies and a stride, has its reason in a comment on the line above,
    /// overlaps no other, and paces the cycle on the clamp-free path: its stride's own seconds, outside the bounds a
    /// measured one would be held to.
    #[test]
    fn every_curated_stride_is_unique_explained_and_unclamped() {
        const RUN_STEP_SECS: f32 = 0.2;
        /// How far outside the clamp the clip's own cycle is put, on either side.
        const OUTSIDE_THE_CLAMP: f32 = 2.0;
        let text = bundled_text();
        let lines: Vec<&str> = text.lines().collect();
        let curated = decode_strides(text);
        assert_eq!(curated.entries.len(), BUNDLED_CURATED_STRIDES);
        for (index, curated_stride) in curated.entries.iter().enumerate() {
            let name = format!("{:?} action {} {:?}", curated_stride.source, curated_stride.action,
                curated_stride.bodies);
            assert!(!curated_stride.bodies.is_empty(), "{name}");
            // `line` is 1-based: the line above it is at `line - 2`.
            assert!(lines[curated_stride.line - 2].starts_with("//"), "{name} has no reason above it");
            for other in &curated.entries[index + 1..] {
                assert!(other.source != curated_stride.source || other.action != curated_stride.action
                    || !other.bodies.iter().any(|body| curated_stride.bodies.contains(body)), "{name} overlaps");
            }
            let CuratedPace::Tiles(tiles) = curated_stride.pace else { continue };
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
    fn strides_and_client_timings_read_by_package_action_and_body() {
        let curated = decode_strides("// A reason.\nstride 400 401 package=\"cc\" action=0 tiles=1.3\n\
            client-timing 12 package=\"ec\" action=24\nstride 400 package=\"ec\" action=0 tiles=2\n");
        assert_eq!(curated.get(AnimSource::Classic, 401, 0).map(|entry| (entry.pace, entry.line)),
            Some((CuratedPace::Tiles(1.3), 2)));
        assert_eq!(curated.get(AnimSource::Enhanced, 12, 24).map(|entry| entry.pace), Some(CuratedPace::ClientTiming));
        assert_eq!(curated.get(AnimSource::Enhanced, 400, 0).map(|entry| entry.pace), Some(CuratedPace::Tiles(2.0)));
        assert_eq!(curated.get(AnimSource::Classic, 12, 24), None);
        assert!(decode_strides("").entries.is_empty());
    }

    /// The error starts with the file and the line, and says what is wrong.
    fn assert_strides_error(text: &str, line: usize, needle: &str) {
        let error = format!("{:#}", CuratedStrides::decode(STRIDES_FILE, text).expect_err("the file should not read"));
        assert!(error.starts_with(&format!("{STRIDES_FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn strides_errors_name_the_line() {
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=1\nwalk 2 package=\"cc\" action=0\n", 2,
            "unknown node `walk` at the top level: expected `stride` or `client-timing`");
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=1 reason=\"x\"\n", 1,
            "unknown property `reason` of `stride`");
        assert_strides_error("client-timing 1 package=\"cc\" action=19 tiles=1\n", 1,
            "unknown property `tiles` of `client-timing`");
        assert_strides_error("stride 1 action=0 tiles=1\n", 1, "`stride`: missing package=");
        assert_strides_error("stride 1 package=\"cc\" tiles=1\n", 1, "`stride`: missing action=");
        assert_strides_error("stride 1 package=\"cc\" action=0\n", 1, "`stride`: missing tiles=");
        assert_strides_error("stride 1 package=\"kr\" action=0 tiles=1\n", 1,
            "package: expected \"cc\" or \"ec\", found \"kr\"");
        assert_strides_error("stride 1 package=\"cc\" action=100 tiles=1\n", 1, "action: 100 is not below 100");
        assert_strides_error("stride 1 package=\"cc\" action=-1 tiles=1\n", 1, "action: not a u16");
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=0\n", 1, "tiles: 0 is outside 0.1..=12");
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=13\n", 1, "tiles: 13 is outside 0.1..=12");
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=\"far\"\n", 1, "tiles: expected a number");
        assert_strides_error("stride package=\"cc\" action=0 tiles=1\n", 1,
            "`stride` takes the body ids it applies to, found none");
        assert_strides_error("stride 70000 package=\"cc\" action=0 tiles=1\n", 1, "the body id: not a u16");
        assert_strides_error("stride 1 2 1 package=\"cc\" action=0 tiles=1\n", 1, "body 1 is listed twice");
        assert_strides_error("stride 1 package=\"cc\" action=0 tiles=1 { stride 2; }\n", 1,
            "`stride` takes no children");
        assert_strides_error("stride 1 2 package=\"cc\" action=0 tiles=1\n\nclient-timing 3 2 package=\"cc\" action=0\n",
            3, "action 0 of body 2 from package \"cc\" is listed already, on line 1");
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
