//! The effect layer: a particle system placed by a `ParticleEffect` definition (EffectDefinition key 0).
//!
//! The definition places the effect's scene node relative to what it is attached to and passes its duration
//! and loop flag to the system ([`ParticleSystemInstance::start`]). The placement rules are the client's
//! `UOParticleEffectChaos::Init` (0x0041aff0) and its Nif counterpart; `camera_offset` is confirmed, the axes of
//! `offset_x` and `offset_y` are probable.

use crate::math::{Mat3, Vec3};
use crate::render::{CameraFrame, RenderFrame};
use crate::system::{NodeTransform, ParticleSystemInstance};

/// Factor of `camera_offset` along the client's camera down vector: `k = -camera_offset * 0.12856` (UOSA.exe
/// 0x00d050a0), so a positive `camera_offset` moves the node up on screen (confirmed).
pub const CAMERA_OFFSET_FACTOR: f32 = 0.12856;

/// The fields of a `ParticleEffect` definition that the particle system uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleEffectParams {
    /// Added to the node x (probable).
    pub offset_x: f32,
    /// Subtracted from the node z: a vertical offset (probable).
    pub offset_y: f32,
    /// Moves the node by `-camera_offset * CAMERA_OFFSET_FACTOR` along the client's camera down vector, i.e. by
    /// `camera_offset * CAMERA_OFFSET_FACTOR` along camera up (confirmed).
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
    /// The node offset from the anchor, given the client's camera down vector ([`EffectPlacement::camera_down`]):
    /// `UOParticleEffectNif::Init` 0x0041aa6a..0x0041aaf9 and `UOParticleEffectChaos::Init` 0x0041b119..0x0041b1ba
    /// compute `(offset_x, 0, -offset_y) + k · camera_down`, which `SetPosition` adds to the anchor.
    pub fn node_offset(&self, camera_down: Vec3) -> Vec3 {
        Vec3::new(self.offset_x, 0.0, -self.offset_y) + camera_down * (-self.camera_offset * CAMERA_OFFSET_FACTOR)
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
    /// The client's camera down vector (-up), along which `camera_offset` moves the node: `g_UOWorldViewer`
    /// 0x00e37224 +0x24, written by `UOWorldViewer::CreateCamera` 0x00471580 as `-(R · (0, 1, 0))` from the
    /// camera rotation `R` (0x004718ac..0x004718de).
    pub camera_down: Vec3,
    /// The node rotation. The client uses one global matrix for every effect (`DAT_00e37224`+0x30); identity
    /// is the neutral choice outside the client.
    pub node_rotation: Mat3,
    /// World velocity of the node; `None` derives it from the movement.
    pub velocity: Option<Vec3>,
}

impl Default for EffectPlacement {
    fn default() -> Self {
        Self { anchor: Vec3::ZERO, camera_down: Vec3::ZERO, node_rotation: Mat3::IDENTITY, velocity: None }
    }
}

impl EffectPlacement {
    pub fn at(anchor: Vec3) -> Self {
        Self { anchor, ..Self::default() }
    }

    /// At `anchor`, with the camera down vector of the client's `camera` (in an effect's local frame,
    /// [`CameraFrame::client_view`]).
    pub fn seen_by(anchor: Vec3, camera: &CameraFrame) -> Self {
        Self { anchor, camera_down: -camera.up, ..Self::default() }
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
            position: placement.anchor + self.params.node_offset(placement.camera_down),
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
    use crate::nif::default_camera;
    use crate::raster::OrthoView;

    /// Canvas side of the client-view test.
    const SIDE: usize = 64;
    /// Camera distance of the client-view test.
    const DISTANCE: f32 = 100.0;
    /// Pixels per world unit of the client-view test.
    const PIXELS_PER_UNIT: f32 = 10.0;
    /// A positive `camera_offset`.
    const CAMERA_OFFSET: f32 = 10.0;
    /// Tolerance of the projected offset, in pixels.
    const PIXEL_EPSILON: f32 = 1e-3;

    #[test]
    fn a_positive_camera_offset_moves_the_node_up_on_the_client_view() {
        let anchor = Vec3::new(1.0, 2.0, 3.0);
        let placement = EffectPlacement::seen_by(anchor, &default_camera());
        assert_eq!(placement.camera_down, -Vec3::Z, "the client's camera down is local -z");
        let params = ParticleEffectParams { camera_offset: CAMERA_OFFSET, ..Default::default() };
        let position = anchor + params.node_offset(placement.camera_down);
        let view = OrthoView::client_view(anchor, DISTANCE, PIXELS_PER_UNIT, SIDE, SIDE);
        let (anchor_x, anchor_y) = view.to_screen(anchor);
        let (x, y) = view.to_screen(position);
        assert_eq!(x, anchor_x, "no horizontal move");
        let rise = CAMERA_OFFSET * CAMERA_OFFSET_FACTOR * PIXELS_PER_UNIT;
        assert!((anchor_y - y - rise).abs() < PIXEL_EPSILON, "moved up by camera_offset * factor (canvas y points down)");
        assert_eq!(view.depth(position), view.depth(anchor), "no move in depth");
    }

    #[test]
    fn offsets_place_the_node() {
        let params = ParticleEffectParams { offset_x: 2.0, offset_y: 3.0, camera_offset: 10.0, scale: -2.0, ..Default::default() };
        let camera_down = Vec3::new(0.0, 1.0, 0.0);
        let offset = params.node_offset(camera_down);
        assert_eq!(offset.x, 2.0);
        assert_eq!(offset.z, -3.0);
        assert!((offset.y - -10.0 * CAMERA_OFFSET_FACTOR).abs() < 1e-6);
        assert_eq!(params.node_scale(), 2.0);
    }
}
