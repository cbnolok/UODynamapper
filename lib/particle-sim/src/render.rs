//! Engine-neutral render output.
//!
//! A system writes one [`RenderFrame`] per frame: draw batches of camera-facing sprites and a list of mesh
//! instances. Nothing here depends on a graphics API; a consumer either
//! - uploads [`SpriteInstance`]s and expands each into a quad on the GPU (the formulas are those of
//!   [`expand_sprite`]), or
//! - calls [`expand_frame`] to get 4 [`QuadVertex`]es per sprite (draw them with [`QUAD_INDICES`]).
//!
//! Positions are in the client's world space (Z is up, probable). Batches are drawn in order, and sprites
//! within a batch in order (the client sorts nothing: oldest particle first, one batch per sequence).
//!
//! Quad layout (Chaos `ParticleTriShape::BuildVertices` 0x008c4c30): vertex k has base UV (0,1), (0,0), (1,0),
//! (1,1), mapped into the sprite's [`UvRect`]; triangles are (0, 2, 1) and (0, 2, 3). Colour is per sprite
//! (all four vertices share it), already clamped to [0, 1], and modulates the texture (unlit).

use std::ops::Range;
use std::sync::Arc;

use crate::math::{Mat3, Vec3};

/// Half extent of a quad relative to the particle size (UOSA.exe constant 0.5 at 0x00c95604).
pub const QUAD_HALF_EXTENT: f32 = 0.5;
/// Vertices per sprite quad.
pub const QUAD_VERTICES: usize = 4;
/// Index pattern of one quad, relative to its first vertex.
pub const QUAD_INDICES: [u32; 6] = [0, 2, 1, 0, 2, 3];

/// Alpha blending factor: Gamebryo `NiAlphaProperty::AlphaFunction`, the values of the client's `ALPHA_*`
/// names. The D3D mapping is the usual one (`One` = D3DBLEND_ONE, ...; probable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BlendFactor {
    One = 0,
    Zero = 1,
    SrcColor = 2,
    InvSrcColor = 3,
    DestColor = 4,
    InvDestColor = 5,
    SrcAlpha = 6,
    InvSrcAlpha = 7,
    DestAlpha = 8,
    InvDestAlpha = 9,
    /// `(f, f, f, 1)` with `f = min(src.a, 1 - dst.a)`.
    SrcAlphaSat = 10,
}

impl BlendFactor {
    pub const ALL: [Self; 11] = [
        Self::One,
        Self::Zero,
        Self::SrcColor,
        Self::InvSrcColor,
        Self::DestColor,
        Self::InvDestColor,
        Self::SrcAlpha,
        Self::InvSrcAlpha,
        Self::DestAlpha,
        Self::InvDestAlpha,
        Self::SrcAlphaSat,
    ];

    /// The factor of a Gamebryo value.
    pub fn from_value(value: u8) -> Option<Self> {
        Self::ALL.get(usize::from(value)).copied()
    }

    /// The client's script name (`ALPHA_ONE`, ...).
    pub fn script_name(self) -> &'static str {
        match self {
            Self::One => "ALPHA_ONE",
            Self::Zero => "ALPHA_ZERO",
            Self::SrcColor => "ALPHA_SRCCOLOR",
            Self::InvSrcColor => "ALPHA_INVSRCCOLOR",
            Self::DestColor => "ALPHA_DESTCOLOR",
            Self::InvDestColor => "ALPHA_INVDESTCOLOR",
            Self::SrcAlpha => "ALPHA_SRCALPHA",
            Self::InvSrcAlpha => "ALPHA_INVSRCALPHA",
            Self::DestAlpha => "ALPHA_DESTALPHA",
            Self::InvDestAlpha => "ALPHA_INVDESTALPHA",
            Self::SrcAlphaSat => "ALPHA_SRCALPHASAT",
        }
    }
}

/// Source and destination blend factors: `out = src * src_factor + dst * dst_factor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendState {
    pub src: BlendFactor,
    pub dst: BlendFactor,
}

/// Depth buffer use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthState {
    /// Less-or-equal test against the scene depth.
    pub test: bool,
    pub write: bool,
}

/// Chaos quads: depth test on, depth write off (`NiZBufferProperty` 0xd on `ParticleSystemRoot` and each shape).
pub const PARTICLE_DEPTH_STATE: DepthState = DepthState { test: true, write: false };

/// How a sprite becomes a quad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpriteKind {
    /// Camera-facing square of side `size`, aligned with the camera right and up axes.
    Billboard,
    /// Camera-facing square spun by `rotation` radians about the view direction.
    RotatedBillboard,
    /// A quad from the particle position to `position + direction * length`, `size` wide, facing the camera.
    Axial,
}

/// A rectangle of texture coordinates: `u0, v0` map to base UV (0, 0), `u1, v1` to (1, 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl UvRect {
    pub const FULL: Self = Self { u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0 };
}

/// One particle to draw as a quad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteInstance {
    /// World position (for axial sprites, the trailing end).
    pub position: Vec3,
    /// Quad side (billboards) or width (axial). May be negative: the quad is then mirrored, as in the client.
    pub size: f32,
    /// Spin in radians ([`SpriteKind::RotatedBillboard`] only; 0 otherwise).
    pub rotation: f32,
    /// RGBA in [0, 1].
    pub color: [f32; 4],
    pub uv: UvRect,
    /// Axial: unit direction of motion (zero for billboards).
    pub direction: Vec3,
    /// Axial: length along `direction`, node scale applied (0 for billboards).
    pub length: f32,
}

/// Texture and render state shared by a run of sprites.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawBatch {
    /// Texture file name (base name with extension, as in the script); empty when the script names none.
    pub texture: Arc<str>,
    pub blend: BlendState,
    pub depth: DepthState,
    /// Draw both faces (axial quads); otherwise back faces are culled.
    pub two_sided: bool,
    pub kind: SpriteKind,
    /// The sprites of the batch in [`RenderFrame::sprites`]; their vertices are `4 * start .. 4 * end` in the
    /// output of [`expand_frame`].
    pub sprites: Range<usize>,
}

/// A particle drawn as a mesh (Chaos MESH sequences): a clone of the `.nif` scene per particle.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshInstance {
    /// Mesh file name (`.nif`, as in the script).
    pub mesh: Arc<str>,
    pub translation: Vec3,
    /// Uniform scale (the particle size).
    pub scale: f32,
    /// Rotation of the effect node.
    pub node_rotation: Mat3,
    /// Particle rotation in radians, to compose as Euler angles after `node_rotation`, when the sequence
    /// rotates its particles. The client's Euler convention (0x0066b640) is not established.
    pub euler: Option<Vec3>,
    /// Time for the mesh controllers: the particle age in seconds (the client sub-steps them at 1/30 s).
    pub animation_time: f32,
    /// Particle colour, RGBA in [0, 1]; whether the client applies it to the mesh is not established.
    pub color: [f32; 4],
}

/// Everything to draw for one frame. Reuse it across frames: [`Self::clear`] keeps the allocations.
#[derive(Debug, Clone, Default)]
pub struct RenderFrame {
    pub batches: Vec<DrawBatch>,
    pub sprites: Vec<SpriteInstance>,
    pub meshes: Vec<MeshInstance>,
}

impl RenderFrame {
    pub fn clear(&mut self) {
        self.batches.clear();
        self.sprites.clear();
        self.meshes.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty() && self.meshes.is_empty()
    }
}

/// The camera axes and position (the client's global camera `DAT_00e321d0`: world rotation columns 0, 1, 2
/// are the view direction, up and right).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFrame {
    pub position: Vec3,
    /// Unit view direction.
    pub direction: Vec3,
    /// Unit up axis.
    pub up: Vec3,
    /// Unit right axis.
    pub right: Vec3,
}

/// One quad vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadVertex {
    pub position: Vec3,
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// The 4 vertices of a sprite.
///
/// - Billboard: `v0 = p - R·h - U·h`, `v1 = p - R·h + U·h`, `v2 = p + R·h + U·h`, `v3 = p + R·h - U·h` with
///   `h = size / 2`.
/// - Rotated billboard: the same with R and U turned by `rotation` about the view direction (the client builds
///   `MakeRotation(-rotation, D) × camera_rotation`; the sign is probable).
/// - Axial: `S = h · cross(view, direction)` with `view = normalize(camera - p)` (not normalized again, so the
///   quad narrows when seen along the motion), `front = p + direction · length`;
///   `v0 = front - S`, `v1 = front + S`, `v2 = p + S`, `v3 = p - S`.
pub fn expand_sprite(kind: SpriteKind, sprite: &SpriteInstance, camera: &CameraFrame) -> [QuadVertex; QUAD_VERTICES] {
    let half = sprite.size * QUAD_HALF_EXTENT;
    let p = sprite.position;
    let corners = match kind {
        SpriteKind::Billboard | SpriteKind::RotatedBillboard => {
            let (right, up) = if kind == SpriteKind::RotatedBillboard {
                (
                    camera.right.rotated_about(camera.direction, sprite.rotation),
                    camera.up.rotated_about(camera.direction, sprite.rotation),
                )
            } else {
                (camera.right, camera.up)
            };
            let r = right * half;
            let u = up * half;
            [p - r - u, p - r + u, p + r + u, p + r - u]
        }
        SpriteKind::Axial => {
            let view = (camera.position - p).normalize_or_zero();
            let side = view.cross(sprite.direction) * half;
            let front = p + sprite.direction * sprite.length;
            [front - side, front + side, p + side, p - side]
        }
    };
    let UvRect { u0, v0, u1, v1 } = sprite.uv;
    let uvs = [[u0, v1], [u0, v0], [u1, v0], [u1, v1]];
    std::array::from_fn(|k| QuadVertex { position: corners[k], uv: uvs[k], color: sprite.color })
}

/// Appends the vertices of every sprite of `frame`, batch by batch (4 per sprite, in sprite order).
pub fn expand_frame(frame: &RenderFrame, camera: &CameraFrame, out: &mut Vec<QuadVertex>) {
    out.reserve(frame.sprites.len() * QUAD_VERTICES);
    for batch in &frame.batches {
        for sprite in &frame.sprites[batch.sprites.clone()] {
            out.extend_from_slice(&expand_sprite(batch.kind, sprite, camera));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn camera_looking_along_y() -> CameraFrame {
        CameraFrame {
            position: Vec3::new(0.0, -10.0, 0.0),
            direction: Vec3::Y,
            up: Vec3::Z,
            right: Vec3::X,
        }
    }

    fn sprite() -> SpriteInstance {
        SpriteInstance {
            position: Vec3::ZERO,
            size: 1.0,
            rotation: 0.0,
            color: [1.0; 4],
            uv: UvRect::FULL,
            direction: Vec3::ZERO,
            length: 0.0,
        }
    }

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!((actual - expected).length() < EPSILON, "{actual:?} != {expected:?}");
    }

    #[test]
    fn billboard_quads_face_the_camera() {
        let quad = expand_sprite(SpriteKind::Billboard, &sprite(), &camera_looking_along_y());
        assert_close(quad[0].position, Vec3::new(-0.5, 0.0, -0.5));
        assert_close(quad[1].position, Vec3::new(-0.5, 0.0, 0.5));
        assert_close(quad[2].position, Vec3::new(0.5, 0.0, 0.5));
        assert_close(quad[3].position, Vec3::new(0.5, 0.0, -0.5));
        assert_eq!(quad.map(|vertex| vertex.uv), [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
    }

    #[test]
    fn axial_quads_run_from_the_front_end_to_the_particle() {
        let axial = SpriteInstance { direction: Vec3::X, length: 2.0, ..sprite() };
        let quad = expand_sprite(SpriteKind::Axial, &axial, &camera_looking_along_y());
        assert_close(quad[0].position, Vec3::new(2.0, 0.0, -0.5));
        assert_close(quad[1].position, Vec3::new(2.0, 0.0, 0.5));
        assert_close(quad[2].position, Vec3::new(0.0, 0.0, 0.5));
        assert_close(quad[3].position, Vec3::new(0.0, 0.0, -0.5));
    }

    #[test]
    fn rotated_billboards_spin_about_the_view_direction() {
        let rotated = SpriteInstance { rotation: std::f32::consts::FRAC_PI_2, ..sprite() };
        let quad = expand_sprite(SpriteKind::RotatedBillboard, &rotated, &camera_looking_along_y());
        // A quarter turn about +Y maps right (+X) to -Z and up (+Z) to +X.
        assert_close(quad[0].position, Vec3::new(-0.5, 0.0, 0.5));
        assert_close(quad[2].position, Vec3::new(0.5, 0.0, -0.5));
    }
}
