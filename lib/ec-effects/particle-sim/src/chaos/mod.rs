//! The Chaos engine: UOSA.exe's text-scripted particle system for effects named `*.ems`.
//!
//! Layers:
//! - [`ChaosSystemDesc`] ([`desc`]): the compiled script as plain data. Produced from a uocf syntax tree by
//!   [`compile::compile_chaos_script`] (feature `uocf`), which applies the client's compile rules and reports
//!   what the client would reject ([`ChaosIssue`]).
//! - [`ChaosSystem`]: a description prepared for running: event times drawn, events sorted and fade links
//!   resolved. The client does this once per compiled (cached) script, so every instance of a script shares
//!   it; share it with an `Arc`.
//! - [`ChaosEmitter`] ([`runtime`]): the client's `Emitter` (start, update, stop, particles), with the
//!   per-sequence bounds and MESH clones (`mesh`, resolved through a [`MeshSource`]) of its `ParticleSet`.
//! - [`ChaosEffectInstance`] ([`instance`]): `UOParticleEffectChaos`, the effect around an emitter, with its own
//!   RNG and clock; a [`crate::system::ParticleSystemInstance`].
//!
//! Semantics follow the reverse engineering in `scripts/dev-helpers/ec-research/ghidra-effects/results/
//! uosa_chaos.md` (addresses are UOSA.exe virtual addresses).

#[cfg(feature = "uocf")]
pub mod compile;
pub mod desc;
mod instance;
mod issues;
pub mod lexer_check;
mod mesh;
mod runtime;

#[cfg(all(test, feature = "uocf"))]
mod tests;

use std::sync::Arc;

pub use desc::*;
pub use instance::ChaosEffectInstance;
pub use issues::{ChaosIssue, ChaosIssueKind, ChaosIssueSeverity, SourceLocation};
pub use mesh::{MESH_CLONE_SEED, MESH_SUBSTEP_S, MeshSource, euler_xyz};
pub use runtime::{
    BOUND_INTERVAL_S, BOUND_RADIUS_MARGIN, ChaosEmitter, ChaosParticle, GRAVITY_AXIS, MAX_EVENT_TIMER_REWINDS,
};

use crate::rng::Rng;

/// Shortest emission interval: 60 emission steps per second per sequence (UOSA.exe `DAT_00d05310`).
pub const MIN_EMIT_INTERVAL_S: f32 = 1.0 / 60.0;
/// Particles of a sequence are sub-stepped at least this often while it emits within a long frame
/// (`DAT_00d05210`).
pub const SUBSTEP_INTERVAL_S: f32 = 1.0 / 30.0;
/// ROTATION values are degrees in scripts and radians at run time.
pub const DEGREES_TO_RADIANS: f32 = std::f32::consts::PI / 180.0;
/// Seconds `UOParticleEffectChaos` keeps updating after Stop before it reports finished (0x00c98590).
pub const EFFECT_FADE_SECONDS: f32 = 5.0;
/// Mask of the client's integer ranges (`SampleIntRange`, `atol & 0xffff`).
pub const INT_RANGE_MASK: u32 = 0xffff;

/// `ChaosScript::SampleFloatRange` (0x008a2050): `min` when the bounds are equal (no draw), a uniform value in
/// `[min, max]` when `min < max`, and **`rand01()` itself** when `min > max` (the bounds are ignored).
pub fn sample_range(rng: &mut (impl Rng + ?Sized), min: f32, max: f32) -> f32 {
    if min == max {
        return min;
    }
    let r = rng.rand01();
    if min < max { (max - min) * r + min } else { r }
}

/// `SampleVectorRange` (0x008aebe0): x, y, z sampled in that order with [`sample_range`].
pub fn sample_components<const N: usize>(rng: &mut (impl Rng + ?Sized), min: &[f32], max: &[f32]) -> [f32; N] {
    std::array::from_fn(|index| sample_range(rng, min[index], max[index]))
}

/// `SampleIntRange` (0x0089cfe0): `max` is exclusive (except when `rand01()` is exactly 1), and the result is
/// 0 when `min > max`.
pub fn sample_int_range(rng: &mut (impl Rng + ?Sized), range: IntRange) -> u32 {
    if range.min == range.max {
        return range.min;
    }
    let r = rng.rand01();
    let value = if range.max > range.min { range.min as f32 + (range.max - range.min) as f32 * r } else { r };
    (value as i32 as u32) & INT_RANGE_MASK
}

/// An event ready to run: its time drawn and its next fade linked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PreparedEvent {
    pub kind: ChaosEventKind,
    pub fade: bool,
    /// The actual time, shared by every particle of the sequence.
    pub time: f32,
    pub value: ChaosEventValue,
    /// Index of the first later event of the same kind with `fade` set.
    pub next_fade: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedSequence {
    /// Sorted by time.
    pub events: Vec<PreparedEvent>,
    pub resource: Arc<str>,
}

/// A [`ChaosSystemDesc`] prepared for running: what the client's script cache holds.
#[derive(Debug, Clone)]
pub struct ChaosSystem {
    desc: ChaosSystemDesc,
    sequences: Vec<PreparedSequence>,
}

impl ChaosSystem {
    /// Draws the event times and orders the events, as the client does at the closing brace of each sequence
    /// (`ResolveEventTimes` 0x008b79e0, `SortEvents` 0x008b8cc0, fade links 0x008b7990):
    /// - each event's time is sampled once, in script order, sequence by sequence;
    /// - the events are sorted by time. The client uses MSVC's introsort, which falls back to a stable insertion
    ///   sort below 32 elements; every client sequence is that short, so a stable sort matches;
    /// - each event links the first later event of the same kind that fades.
    pub fn prepare(desc: ChaosSystemDesc, rng: &mut (impl Rng + ?Sized)) -> Self {
        let sequences = desc
            .sequences
            .iter()
            .map(|sequence| {
                let mut events: Vec<PreparedEvent> = sequence
                    .events
                    .iter()
                    .map(|event| PreparedEvent {
                        kind: event.kind,
                        fade: event.fade,
                        time: sample_range(rng, event.time.min, event.time.max),
                        value: event.value,
                        next_fade: None,
                    })
                    .collect();
                events.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
                for index in 0..events.len() {
                    let kind = events[index].kind;
                    events[index].next_fade =
                        (index + 1..events.len()).find(|&later| events[later].kind == kind && events[later].fade);
                }
                PreparedSequence { events, resource: Arc::from(sequence.resource.as_str()) }
            })
            .collect();
        Self { desc, sequences }
    }

    pub fn desc(&self) -> &ChaosSystemDesc {
        &self.desc
    }

    /// The drawn event times of a sequence, in run order.
    pub fn event_times(&self, sequence: usize) -> Vec<f32> {
        self.sequences
            .get(sequence)
            .map(|prepared| prepared.events.iter().map(|event| event.time).collect())
            .unwrap_or_default()
    }

    pub(crate) fn prepared(&self, sequence: usize) -> &PreparedSequence {
        &self.sequences[sequence]
    }
}
