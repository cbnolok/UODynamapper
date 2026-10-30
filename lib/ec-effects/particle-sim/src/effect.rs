//! The effect layer: a particle system placed by a `ParticleEffect` definition (EffectDefinition key 0).
//!
//! The definition places the effect's scene node relative to what it is attached to and passes its duration
//! and loop flag to the system ([`ParticleSystemInstance::start`]). The placement rules are the client's
//! `UOParticleEffectChaos::Init` (0x0041aff0) and its Nif counterpart (confirmed): `offset_x`, `offset_y` and
//! `camera_offset` are a tile offset (dx, dy, dz) converted to world space as `PlayEffectAtTile` (0x00421130)
//! converts a tile position.

use crate::math::{Mat3, Vec3};
use crate::render::{CameraFrame, RenderFrame};
use crate::system::{NodeTransform, ParticleSystemInstance};

/// Factor of `camera_offset` along the client's camera down vector: `k = -camera_offset * 0.12856` (UOSA.exe
/// 0x00d050a0), so a positive `camera_offset` moves the node up on screen (confirmed).
pub const CAMERA_OFFSET_FACTOR: f32 = 0.12856;

/// World vector of one tile step east (+x): `PlayEffectAtTile` 0x00421130 and both `Init`s add the tile x to the
/// world x (confirmed).
pub const TILE_X_WORLD_AXIS: Vec3 = Vec3::X;
/// World vector of one tile step south (+y): `PlayEffectAtTile` 0x00421130 and both `Init`s subtract the tile y
/// from the world z (confirmed).
pub const TILE_Y_WORLD_AXIS: Vec3 = Vec3::new(0.0, 0.0, -1.0);

/// Yaw of the client's world camera: `UOWorldViewer::CreateCamera` 0x00471580 starts from
/// `MakeZRotation(-π/2)` (π/2 at UOSA.exe 0x00dc1fd0).
const CLIENT_CAMERA_YAW: f32 = -std::f32::consts::FRAC_PI_2;
/// The pitch bias of `MakeXRotation(0.001 - π/2 * 0.5)` (0.001 at UOSA.exe 0x00c9857c).
const CLIENT_CAMERA_PITCH_BIAS: f32 = 0.001;
/// The pitch factor of `MakeXRotation(0.001 - π/2 * 0.5)` (0.5 at UOSA.exe 0x00c95604).
const CLIENT_CAMERA_PITCH_FACTOR: f32 = 0.5;
/// The final `MakeZRotation(-0.05)` (UOSA.exe 0x00d051c0).
const CLIENT_CAMERA_ROLL: f32 = -0.05;

/// Gamebryo `NiMatrix3::MakeZRotation`.
fn make_z_rotation(angle: f32) -> Mat3 {
    let (s, c) = angle.sin_cos();
    Mat3::from_rows([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
}

/// Gamebryo `NiMatrix3::MakeXRotation`.
fn make_x_rotation(angle: f32) -> Mat3 {
    let (s, c) = angle.sin_cos();
    Mat3::from_rows([[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]])
}

/// The world rotation of the client's camera, `Z(-π/2) · X(0.001 - π/4) · Z(-0.05)`, as
/// `UOWorldViewer::CreateCamera` 0x00471580 builds it with `NiMatrix3` products (0x00668f10) and stores it in
/// the camera (+0x2c). Columns: view direction, up, right (Gamebryo camera axes).
pub fn client_world_camera_rotation() -> Mat3 {
    let pitch = CLIENT_CAMERA_PITCH_BIAS - std::f32::consts::FRAC_PI_2 * CLIENT_CAMERA_PITCH_FACTOR;
    make_z_rotation(CLIENT_CAMERA_YAW).mul_mat3(&make_x_rotation(pitch)).mul_mat3(&make_z_rotation(CLIENT_CAMERA_ROLL))
}

/// A world vector of the client expressed in the frame where the client's camera is `camera`: its components
/// along the client's world camera axes put on `camera`'s axes.
fn client_world_vector_seen_by(world: Vec3, camera: &CameraFrame) -> Vec3 {
    let rotation = client_world_camera_rotation();
    camera.direction * world.dot(rotation.col(0))
        + camera.up * world.dot(rotation.col(1))
        + camera.right * world.dot(rotation.col(2))
}

/// The fields of a `ParticleEffect` definition that the particle system uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleEffectParams {
    /// Tiles east: moves the node by `offset_x` along [`EffectPlacement::tile_x_axis`] (world +x, confirmed).
    pub offset_x: f32,
    /// Tiles south: moves the node by `offset_y` along [`EffectPlacement::tile_y_axis`] (world -z, confirmed).
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
    /// The node offset from the anchor in the placement's frame: `UOParticleEffectNif::Init`
    /// 0x0041aa6a..0x0041aaf9 and `UOParticleEffectChaos::Init` 0x0041b119..0x0041b1ba compute the world offset
    /// `(offset_x, 0, -offset_y) + k · camera_down` with `k = -0.0 - camera_offset * 0.12856`, which `SetPosition`
    /// (Chaos 0x0041b500) adds to the anchor (confirmed).
    pub fn node_offset(&self, placement: &EffectPlacement) -> Vec3 {
        placement.tile_x_axis * self.offset_x
            + placement.tile_y_axis * self.offset_y
            + placement.camera_down * (-self.camera_offset * CAMERA_OFFSET_FACTOR)
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
    /// The world vector of one tile east in this frame ([`TILE_X_WORLD_AXIS`] in world space).
    pub tile_x_axis: Vec3,
    /// The world vector of one tile south in this frame ([`TILE_Y_WORLD_AXIS`] in world space).
    pub tile_y_axis: Vec3,
    /// The node rotation. The client uses one global matrix for every effect (`DAT_00e37224`+0x30); identity
    /// is the neutral choice outside the client.
    pub node_rotation: Mat3,
    /// World velocity of the node; `None` derives it from the movement.
    pub velocity: Option<Vec3>,
}

impl Default for EffectPlacement {
    fn default() -> Self {
        Self {
            anchor: Vec3::ZERO,
            camera_down: Vec3::ZERO,
            tile_x_axis: TILE_X_WORLD_AXIS,
            tile_y_axis: TILE_Y_WORLD_AXIS,
            node_rotation: Mat3::IDENTITY,
            velocity: None,
        }
    }
}

impl EffectPlacement {
    pub fn at(anchor: Vec3) -> Self {
        Self { anchor, ..Self::default() }
    }

    /// At `anchor`, in the frame where the client's camera is `camera` (an effect's local frame,
    /// [`CameraFrame::client_view`]): the camera down vector and the tile axes of the client's world camera
    /// ([`client_world_camera_rotation`]) put on `camera`'s axes.
    pub fn seen_by(anchor: Vec3, camera: &CameraFrame) -> Self {
        Self {
            anchor,
            camera_down: -camera.up,
            tile_x_axis: client_world_vector_seen_by(TILE_X_WORLD_AXIS, camera),
            tile_y_axis: client_world_vector_seen_by(TILE_Y_WORLD_AXIS, camera),
            ..Self::default()
        }
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
            position: placement.anchor + self.params.node_offset(placement),
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
        let position = anchor + params.node_offset(&placement);
        let view = OrthoView::client_view(anchor, DISTANCE, PIXELS_PER_UNIT, SIDE, SIDE);
        let (anchor_x, anchor_y) = view.to_screen(anchor);
        let (x, y) = view.to_screen(position);
        assert_eq!(x, anchor_x, "no horizontal move");
        let rise = CAMERA_OFFSET * CAMERA_OFFSET_FACTOR * PIXELS_PER_UNIT;
        assert!((anchor_y - y - rise).abs() < PIXEL_EPSILON, "moved up by camera_offset * factor (canvas y points down)");
        assert_eq!(view.depth(position), view.depth(anchor), "no move in depth");
    }

    /// Screen right and down of one tile east on the client's view, in world units: the right and `-up`
    /// components of world +x in the camera of `UOWorldViewer::CreateCamera` 0x00471580.
    const TILE_EAST_ON_SCREEN: (f32, f32) = (0.70640, 0.70693);
    /// Screen right and down of one tile south: the right and `-up` components of world -z.
    const TILE_SOUTH_ON_SCREEN: (f32, f32) = (-0.70781, 0.70552);
    /// Tolerance of the tile steps on screen, in world units.
    const TILE_STEP_EPSILON: f32 = 1e-4;

    #[test]
    fn positive_tile_offsets_move_the_node_east_and_south_on_the_client_view() {
        let anchor = Vec3::new(1.0, 2.0, 3.0);
        let placement = EffectPlacement::seen_by(anchor, &default_camera());
        let view = OrthoView::client_view(anchor, DISTANCE, PIXELS_PER_UNIT, SIDE, SIDE);
        let (anchor_x, anchor_y) = view.to_screen(anchor);
        let cases = [
            (ParticleEffectParams { offset_x: 1.0, ..Default::default() }, TILE_EAST_ON_SCREEN),
            (ParticleEffectParams { offset_y: 1.0, ..Default::default() }, TILE_SOUTH_ON_SCREEN),
        ];
        for (params, (right, down)) in cases {
            let (x, y) = view.to_screen(anchor + params.node_offset(&placement));
            assert!(((x - anchor_x) / PIXELS_PER_UNIT - right).abs() < TILE_STEP_EPSILON, "{params:?}: right");
            assert!(((y - anchor_y) / PIXELS_PER_UNIT - down).abs() < TILE_STEP_EPSILON, "{params:?}: down");
        }
    }

    #[test]
    fn offsets_place_the_node() {
        let params = ParticleEffectParams { offset_x: 2.0, offset_y: 3.0, camera_offset: 10.0, scale: -2.0, ..Default::default() };
        let placement = EffectPlacement { camera_down: Vec3::new(0.0, 1.0, 0.0), ..Default::default() };
        let offset = params.node_offset(&placement);
        assert_eq!(offset.x, 2.0);
        assert_eq!(offset.z, -3.0);
        assert!((offset.y - -10.0 * CAMERA_OFFSET_FACTOR).abs() < 1e-6);
        assert_eq!(params.node_scale(), 2.0);
    }
}
