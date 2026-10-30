//! The effect layer: a particle system placed by a `ParticleEffect` definition (EffectDefinition key 0).
//!
//! The definition places the effect's scene node relative to what it is attached to and passes its duration
//! and loop flag to the system ([`ParticleSystemInstance::start`]). The placement rules are the client's
//! `UOParticleEffectChaos::Init` (0x0041aff0) and its Nif counterpart; those on the offsets are probable.

use crate::math::{Mat3, Vec3};
use crate::render::RenderFrame;
use crate::system::{NodeTransform, ParticleSystemInstance};

/// Factor of `camera_offset` along the camera vector: `k = -camera_offset * 0.12856` (UOSA.exe 0x00d050a0;
/// probable).
pub const CAMERA_OFFSET_FACTOR: f32 = 0.12856;

/// The fields of a `ParticleEffect` definition that the particle system uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleEffectParams {
    /// Added to the node x (probable).
    pub offset_x: f32,
    /// Subtracted from the node z: a vertical offset (probable).
    pub offset_y: f32,
    /// Moves the node by `-camera_offset * CAMERA_OFFSET_FACTOR` along the camera vector (probable).
    pub camera_offset: f32,
    /// Node scale; the client uses its absolute value.
    pub scale: f32,
    /// Seconds; for Chaos, the emission time of a looping effect (≤ 0: forever).
    pub duration_s: f32,
    pub looping: bool,
}

impl Default for ParticleEffectParams {
    fn default() -> Self {
        Self { offset_x: 0.0, offset_y: 0.0, camera_offset: 0.0, scale: 1.0, duration_s: 0.0, looping: false }
    }
}

#[cfg(feature = "uocf")]
impl From<&uocf::enhanced::effect_definition::ParticleEffect> for ParticleEffectParams {
    fn from(effect: &uocf::enhanced::effect_definition::ParticleEffect) -> Self {
        Self {
            offset_x: effect.offset_x,
            offset_y: effect.offset_y,
            camera_offset: effect.camera_offset,
            scale: effect.scale,
            duration_s: effect.duration,
            looping: effect.looping,
        }
    }
}

impl ParticleEffectParams {
    /// The node offset from the anchor, given the camera vector (`DAT_00e37224`+0x24).
    pub fn node_offset(&self, camera_vector: Vec3) -> Vec3 {
        Vec3::new(self.offset_x, 0.0, -self.offset_y) + camera_vector * (-self.camera_offset * CAMERA_OFFSET_FACTOR)
    }

    /// The node scale (`fabs(scale)`).
    pub fn node_scale(&self) -> f32 {
        self.scale.abs()
    }
}

/// Where the effect is this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectPlacement {
    /// World position of what the effect is attached to (object, tile, projectile).
    pub anchor: Vec3,
    /// The client's camera vector, for `camera_offset`.
    pub camera_vector: Vec3,
    /// The node rotation. The client uses one global matrix for every effect (`DAT_00e37224`+0x30); identity
    /// is the neutral choice outside the client.
    pub node_rotation: Mat3,
    /// World velocity of the node; `None` derives it from the movement.
    pub velocity: Option<Vec3>,
}

impl Default for EffectPlacement {
    fn default() -> Self {
        Self { anchor: Vec3::ZERO, camera_vector: Vec3::ZERO, node_rotation: Mat3::IDENTITY, velocity: None }
    }
}

impl EffectPlacement {
    pub fn at(anchor: Vec3) -> Self {
        Self { anchor, ..Self::default() }
    }
}

/// A particle system of any kind, placed and started by a definition.
pub struct EffectInstance {
    pub params: ParticleEffectParams,
    system: Box<dyn ParticleSystemInstance>,
}

impl EffectInstance {
    pub fn new(params: ParticleEffectParams, system: Box<dyn ParticleSystemInstance>) -> Self {
        Self { params, system }
    }

    pub fn system(&self) -> &dyn ParticleSystemInstance {
        self.system.as_ref()
    }

    pub fn system_mut(&mut self) -> &mut dyn ParticleSystemInstance {
        self.system.as_mut()
    }

    /// The node transform for `placement`.
    pub fn node(&self, placement: &EffectPlacement) -> NodeTransform {
        NodeTransform {
            position: placement.anchor + self.params.node_offset(placement.camera_vector),
            rotation: placement.node_rotation,
            scale: self.params.node_scale(),
            velocity: placement.velocity,
        }
    }

    /// Starts the system with the definition's duration and loop flag.
    pub fn start(&mut self) {
        self.system.start(self.params.duration_s, self.params.looping);
    }

    pub fn stop(&mut self) {
        self.system.stop();
    }

    pub fn update(&mut self, dt_ms: u32, placement: &EffectPlacement) {
        let node = self.node(placement);
        self.system.update(dt_ms, &node);
    }

    pub fn is_finished(&self) -> bool {
        self.system.is_finished()
    }

    pub fn is_drained(&self) -> bool {
        self.system.is_drained()
    }

    pub fn particle_count(&self) -> usize {
        self.system.particle_count()
    }

    pub fn render(&self, out: &mut RenderFrame) {
        self.system.render(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_place_the_node() {
        let params = ParticleEffectParams { offset_x: 2.0, offset_y: 3.0, camera_offset: 10.0, scale: -2.0, ..Default::default() };
        let camera_vector = Vec3::new(0.0, 1.0, 0.0);
        let offset = params.node_offset(camera_vector);
        assert_eq!(offset.x, 2.0);
        assert_eq!(offset.z, -3.0);
        assert!((offset.y - -10.0 * CAMERA_OFFSET_FACTOR).abs() < 1e-6);
        assert_eq!(params.node_scale(), 2.0);
    }
}
