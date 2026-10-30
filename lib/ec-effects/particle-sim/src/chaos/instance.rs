//! `UOParticleEffectChaos` (vftable 0x00c9e114): the effect that plays a Chaos script.

use std::sync::Arc;

use super::runtime::ChaosEmitter;
use super::{ChaosSystem, EFFECT_FADE_SECONDS, MeshSource};
use crate::render::RenderFrame;
use crate::rng::{Mt19937, Rng};
use crate::system::{NodeTransform, ParticleSystemInstance, SECONDS_PER_MS};

/// A playing Chaos effect: an emitter, its RNG and its clock.
///
/// - The clock starts at 0 and advances by `dt_ms * 0.001` seconds per update, as the client's
///   `ParticleSystemManager` time does (one global clock there; one per instance here).
/// - The client draws from one global MT19937 shared by every effect, so its sequence of draws depends on what
///   else plays. Here each instance owns its RNG: a fixed seed replays the same effect.
/// - [`ParticleSystemInstance::is_finished`] follows the client (emission ended, the fade timer reached 5 s or
///   the tick countdown ran out); particles outlive it. [`ParticleSystemInstance::is_drained`] tells when the
///   last particle died.
#[derive(Debug, Clone)]
pub struct ChaosEffectInstance<R: Rng = Mt19937> {
    emitter: ChaosEmitter,
    rng: R,
    /// Manager time, seconds.
    time: f32,
    /// +0x20
    elapsed: f32,
    /// +0x24
    active: bool,
    /// +0x25
    stopping: bool,
    /// +0x2c
    fade: f32,
    /// +0x3c: updates left, `None` for no limit (the client's -1).
    lifetime_ticks: Option<u32>,
}

impl ChaosEffectInstance<Mt19937> {
    /// An instance with its own MT19937 seeded with `seed`.
    pub fn with_seed(system: Arc<ChaosSystem>, seed: u32) -> Self {
        Self::new(system, Mt19937::new(seed))
    }
}

impl<R: Rng> ChaosEffectInstance<R> {
    pub fn new(system: Arc<ChaosSystem>, rng: R) -> Self {
        Self {
            emitter: ChaosEmitter::new(system),
            rng,
            time: 0.0,
            elapsed: 0.0,
            active: false,
            stopping: false,
            fade: 0.0,
            lifetime_ticks: None,
        }
    }

    /// Resolves the `.nif` of the MESH sequences ([`ChaosEmitter::set_mesh_source`]).
    pub fn set_mesh_source(&mut self, source: &MeshSource) {
        self.emitter.set_mesh_source(source);
    }

    pub fn emitter(&self) -> &ChaosEmitter {
        &self.emitter
    }

    /// The instance clock in seconds.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Seconds since Start.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub fn rng_mut(&mut self) -> &mut R {
        &mut self.rng
    }
}

impl<R: Rng + Send + Sync> ParticleSystemInstance for ChaosEffectInstance<R> {
    /// `Start` (0x0041b210): `Emitter::Start(0, duration, !looping)`.
    fn start(&mut self, duration_s: f32, looping: bool) {
        self.fade = 0.0;
        self.elapsed = 0.0;
        self.stopping = false;
        self.emitter.start(&mut self.rng, duration_s, !looping);
        self.active = true;
    }

    /// `Stop` (0x0041b290).
    fn stop(&mut self) {
        self.active = false;
        self.stopping = true;
        self.emitter.stop();
    }

    /// `ParticleSystemManager::Update` for this emitter, then the effect's `Update` (0x0041b370).
    fn update(&mut self, dt_ms: u32, node: &NodeTransform) {
        let dt = dt_ms as f32 * SECONDS_PER_MS;
        self.time += dt;
        self.emitter.update(&mut self.rng, self.time, dt, node);
        self.elapsed += dt;
        if let Some(ticks) = &mut self.lifetime_ticks {
            *ticks = ticks.saturating_sub(1);
        }
        if self.stopping {
            self.fade += dt;
        }
    }

    /// `IsFinished` (0x0041b2c0; probable in detail).
    fn is_finished(&self) -> bool {
        self.lifetime_ticks == Some(0)
            || self.fade >= EFFECT_FADE_SECONDS
            || (self.active && !self.emitter.is_active())
    }

    fn particle_count(&self) -> usize {
        self.emitter.particle_count()
    }

    fn is_drained(&self) -> bool {
        !self.emitter.is_active() && self.emitter.particle_count() == 0
    }

    fn set_lifetime_ticks(&mut self, ticks: Option<u32>) {
        self.lifetime_ticks = ticks;
    }

    fn render(&self, out: &mut RenderFrame) {
        self.emitter.render(out);
    }
}
