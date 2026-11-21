//! How fast a mobile's walk and run cycles play. The clients play a clip at its own frame time, whatever the speed of
//! the steps, so a big Enhanced creature walking a tile in 0.4 s skates. Matched to the ground, a cycle lasts the time
//! the steps take to cover the clip's stride: how far the body's planted feet travel in one cycle of the clip, measured
//! offline on the frames (`measure`, the table in `assets/data/mobile_strides.txt`).

#[cfg(test)]
mod measure;

use std::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use super::clips::AnimSource;

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

/// A clip paced to the ground: its stride, in tiles per cycle, and the time a step takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundStride {
    pub tiles: f32,
    pub step_secs: f32,
}

impl GroundStride {
    /// The seconds of one cycle of a clip whose own cycle lasts `own_cycle_secs`.
    pub fn cycle_secs(self, own_cycle_secs: f32) -> f32 {
        (self.tiles * self.step_secs)
            .clamp(MIN_OWN_CYCLE_FRACTION * own_cycle_secs, MAX_OWN_CYCLE_FRACTION * own_cycle_secs)
    }
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

/// The bundled table of strides, written by `measure`.
const BUNDLED_STRIDES: &str = include_str!("../../../assets/data/mobile_strides.txt");
const BUNDLED_STRIDES_NAME: &str = "assets/data/mobile_strides.txt";
/// The table's names of the sources.
const CLASSIC_TOKEN: &str = "cc";
const ENHANCED_TOKEN: &str = "ec";
const COMMENT_PREFIX: char = '#';

/// The measured strides, in tiles per cycle, of the walk and run clips (and an Enhanced mount's mounted moves) by
/// source, server body and the action the client tables give for the movement.
#[derive(Resource, Debug, Default)]
pub struct MobileStrides {
    strides: HashMap<(AnimSource, u32, u16), f32>,
}

impl MobileStrides {
    /// Reads a table: a line per clip, `source body action tiles` (source `cc` or `ec`), `#` comments and blank lines
    /// aside. Errors name the line.
    pub fn parse(name: &str, text: &str) -> Result<Self, String> {
        let mut strides = HashMap::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(COMMENT_PREFIX) {
                continue;
            }
            let error = |what: &str| format!("{name}:{}: {what}: {line:?}", index + 1);
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [source, body, action, tiles] = fields[..] else {
                return Err(error("expected `source body action tiles`"));
            };
            let source = match source {
                CLASSIC_TOKEN => AnimSource::Classic,
                ENHANCED_TOKEN => AnimSource::Enhanced,
                _ => return Err(error(&format!("the source is `{CLASSIC_TOKEN}` or `{ENHANCED_TOKEN}`"))),
            };
            let body = body.parse().map_err(|_| error("the body is not a number"))?;
            let action = action.parse().map_err(|_| error("the action is not a number"))?;
            let tiles: f32 = tiles.parse().map_err(|_| error("the stride is not a number"))?;
            if !(tiles.is_finite() && tiles > 0.0) {
                return Err(error("the stride is not positive"));
            }
            if strides.insert((source, body, action), tiles).is_some() {
                return Err(error("a second stride for this clip"));
            }
        }
        Ok(Self { strides })
    }

    /// The bundled table; an unreadable one is logged and leaves every clip at its own timing.
    pub fn bundled() -> Self {
        Self::parse(BUNDLED_STRIDES_NAME, BUNDLED_STRIDES).unwrap_or_else(|error| {
            error!("The mobiles' strides do not read, so their walks keep the clients' timing: {error}");
            Self::default()
        })
    }

    /// The stride of `action` of server `body` from `source`, in tiles per cycle.
    pub fn get(&self, source: AnimSource, body: u32, action: u16) -> Option<f32> {
        self.strides.get(&(source, body, action)).copied()
    }
}

/// The table line of a stride, as [`MobileStrides::parse`] reads it.
#[cfg(test)]
fn stride_line(source: AnimSource, body: u32, action: u16, tiles: f32) -> String {
    let source = match source {
        AnimSource::Classic => CLASSIC_TOKEN,
        AnimSource::Enhanced => ENHANCED_TOKEN,
    };
    format!("{source} {body} {action} {tiles:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ground_cycle_lasts_the_stride_in_steps_within_half_and_twice_the_clip_own() {
        const OWN_CYCLE_SECS: f32 = 1.0;
        let cycle = |tiles: f32, step_secs: f32| GroundStride { tiles, step_secs }.cycle_secs(OWN_CYCLE_SECS);
        assert!((cycle(2.0, 0.4) - 0.8).abs() < 1e-6);
        assert!((cycle(0.5, 0.4) - MIN_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
        assert!((cycle(8.0, 0.4) - MAX_OWN_CYCLE_FRACTION * OWN_CYCLE_SECS).abs() < 1e-6);
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

    #[test]
    fn a_stride_table_reads_its_lines_and_refuses_bad_ones() {
        let text = format!("# a comment\n\n{}\n{}\n", stride_line(AnimSource::Classic, 200, 0, 1.854),
            stride_line(AnimSource::Enhanced, 200, 9, 2.5));
        let table = MobileStrides::parse("test", &text).unwrap();
        assert_eq!(table.strides.len(), 2);
        assert_eq!(table.get(AnimSource::Classic, 200, 0), Some(1.85));
        assert_eq!(table.get(AnimSource::Enhanced, 200, 9), Some(2.5));
        assert_eq!(table.get(AnimSource::Enhanced, 200, 0), None);
        for bad in ["kr 200 0 1.0", "cc 200 0", "cc x 0 1.0", "cc 200 0 -1.0", "cc 200 0 1.0\ncc 200 0 2.0"] {
            let error = MobileStrides::parse("test", bad).unwrap_err();
            assert!(error.starts_with("test:"), "{error}");
        }
    }

    #[test]
    fn the_bundled_stride_table_reads() {
        let table = MobileStrides::parse(BUNDLED_STRIDES_NAME, BUNDLED_STRIDES).unwrap();
        assert!(!table.strides.is_empty());
    }
}
