//! What every particle system kind offers to the effect layer.

use crate::math::{Mat3, Vec3};
use crate::render::RenderFrame;

/// Seconds per millisecond: the client advances particle time by `ms * 0.001` (UOSA.exe constant 0.001 at
/// 0x00c9857c).
pub const SECONDS_PER_MS: f32 = 0.001;
/// Milliseconds per second.
pub const MS_PER_SECOND: f32 = 1000.0;

/// The client's frame step: `round(dt * 1000)` whole milliseconds (`UOEffects::UpdateActiveEffects`).
pub fn frame_dt_to_ms(dt_seconds: f32) -> u32 {
    (dt_seconds * MS_PER_SECOND).round().max(0.0) as u32
}

/// The transform of the scene node an effect is attached to, read by the system every update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeTransform {
    /// World position.
    pub position: Vec3,
    /// World rotation.
    pub rotation: Mat3,
    /// Uniform local scale.
    pub scale: f32,
    /// World velocity. `None` derives it from the position change over the frame, as the client's node does.
    pub velocity: Option<Vec3>,
}

impl Default for NodeTransform {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Mat3::IDENTITY, scale: 1.0, velocity: None }
    }
}

impl NodeTransform {
    pub fn at(position: Vec3) -> Self {
        Self { position, ..Self::default() }
    }
}

/// A running particle system of any kind (Chaos script; Gamebryo NiPSys later), driven by
/// [`crate::effect::EffectInstance`].
///
/// Time is whole milliseconds, as in the client. Each kind reproduces the start, stop and finish rules of its
/// client effect class.
pub trait ParticleSystemInstance: Send + Sync {
    /// Starts emitting. `duration_s` and `looping` are the effect definition's fields; their meaning depends on
    /// the kind.
    fn start(&mut self, duration_s: f32, looping: bool);

    /// Stops emitting. Particles already alive keep simulating until they die, so keep updating and drawing
    /// the instance until [`Self::is_drained`].
    fn stop(&mut self);

    /// Advances the simulation by `dt_ms` with the node at `node`.
    fn update(&mut self, dt_ms: u32, node: &NodeTransform);

    /// The client's `IsFinished` for this kind of effect: when the client would release the effect.
    fn is_finished(&self) -> bool;

    /// Live particles.
    fn particle_count(&self) -> usize;

    /// Whether nothing is left to simulate or draw: the instance can be dropped.
    fn is_drained(&self) -> bool;

    /// Limits the effect to `ticks` more updates (the client's packet duration, a count of frames); `None`
    /// removes the limit.
    fn set_lifetime_ticks(&mut self, ticks: Option<u32>);

    /// Appends the current particles to `out`.
    fn render(&self, out: &mut RenderFrame);
}
