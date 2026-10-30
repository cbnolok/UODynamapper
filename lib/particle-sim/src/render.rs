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
//!
//! NIF effects ([`crate::nif`]) also fill [`RenderFrame::mesh_draws`] (triangle meshes with their world
//! transform, material, texture stages and render state) and [`RenderFrame::batch_states`] (the render state
//! of their sprite batches that [`DrawBatch`] does not carry). Both carry a scene `order`: draw batches and mesh
//! draws interleaved by it reproduce the scene graph's draw order. [`expand_mesh_draw`] lights and transforms a
//! mesh draw into a triangle list for software rasterizers.

use std::ops::Range;
use std::sync::Arc;

use crate::math::{Mat3, Vec3};
use crate::nif::desc::{
    CompareFunction, CullMode, LightingMode, NifVertexColor, TextureAddressing, TextureApplyMode, TextureSlot,
    VertexColorSource,
};

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
    /// Triangle meshes (NIF effects).
    pub mesh_draws: Vec<MeshDraw>,
    /// Extra render state of some batches (NIF effects); batches without an entry use the defaults of
    /// [`BatchState`].
    pub batch_states: Vec<BatchState>,
}

impl RenderFrame {
    pub fn clear(&mut self) {
        self.batches.clear();
        self.sprites.clear();
        self.meshes.clear();
        self.mesh_draws.clear();
        self.batch_states.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty() && self.meshes.is_empty() && self.mesh_draws.is_empty()
    }

    /// The extra state of batch `batch`, if any.
    pub fn batch_state(&self, batch: usize) -> Option<&BatchState> {
        self.batch_states.iter().find(|state| state.batch == batch)
    }
}

/// Triangle geometry (`NiTriShapeData`), in the mesh's local space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshGeometry {
    pub positions: Vec<[f32; 3]>,
    /// Per vertex, or empty.
    pub normals: Vec<[f32; 3]>,
    /// RGBA per vertex, or empty.
    pub colors: Vec<[f32; 4]>,
    /// Texture coordinate sets, each with one UV per vertex.
    pub uv_sets: Vec<Vec<[f32; 2]>>,
    /// Vertex indices; front faces are counter-clockwise as seen by the viewer (`NiStencilProperty::DRAW_CCW`).
    pub triangles: Vec<[u16; 3]>,
}

/// An affine texture coordinate transform: `u' = rows[0] · (u, v, 1)`, `v' = rows[1] · (u, v, 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub rows: [[f32; 3]; 2],
}

impl UvTransform {
    pub const IDENTITY: Self = Self { rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] };

    pub fn apply(&self, [u, v]: [f32; 2]) -> [f32; 2] {
        let [a, b] = self.rows;
        [a[0] * u + a[1] * v + a[2], b[0] * u + b[1] * v + b[2]]
    }
}

impl Default for UvTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Alpha test: a fragment is kept when `alpha <function> reference`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlphaTest {
    pub function: CompareFunction,
    /// Reference alpha in [0, 1].
    pub reference: f32,
}

/// Render state of a sprite batch that [`DrawBatch`] does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchState {
    /// Index in [`RenderFrame::batches`].
    pub batch: usize,
    /// Scene draw order (see [`MeshDraw::order`]).
    pub order: u32,
    pub alpha_test: Option<AlphaTest>,
    /// Depth comparison when [`DepthState::test`] is on.
    pub depth_function: CompareFunction,
    /// Transform of the base map's sprite texture coordinates (after the [`UvRect`] mapping).
    pub uv_transform: UvTransform,
    pub apply_mode: TextureApplyMode,
    /// Every texture stage of the batch ([`DrawBatch::texture`] is the base map's), each with its own transform
    /// of the sprite texture coordinates; combine the samples with [`combine_textures`].
    pub textures: Vec<TextureStage>,
}

impl Default for BatchState {
    fn default() -> Self {
        Self {
            batch: 0,
            order: 0,
            alpha_test: None,
            depth_function: CompareFunction::LessEqual,
            uv_transform: UvTransform::IDENTITY,
            apply_mode: TextureApplyMode::Modulate,
            textures: Vec::new(),
        }
    }
}

/// Material colours of a draw (`NiMaterialProperty`, animated).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialColors {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
    pub alpha: f32,
}

/// The default `NiMaterialProperty` colours (ambient and diffuse 0.5, specular and emissive 0, alpha 1).
pub const DEFAULT_MATERIAL_COLORS: MaterialColors =
    MaterialColors { ambient: [0.5; 3], diffuse: [0.5; 3], specular: [0.0; 3], emissive: [0.0; 3], alpha: 1.0 };

/// One texture of a draw.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureStage {
    pub slot: TextureSlot,
    /// Texture name (a file name of the client's packages, or an embedded texture name of the effect).
    pub texture: Arc<str>,
    pub uv_set: u32,
    pub uv_transform: UvTransform,
    pub addressing: TextureAddressing,
}

/// The lights an effect is lit by. The client's effect lighting environment is not established: the default is
/// a white ambient light and no diffuse light, so the usual `E_A_D` materials show `emissive + 0.5 * ambient`.
/// Diffuse light is applied without direction (as if facing every surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLighting {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
}

impl Default for SceneLighting {
    fn default() -> Self {
        Self { ambient: [1.0; 3], diffuse: [0.0; 3] }
    }
}

/// A triangle mesh to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshDraw {
    /// Scene draw order: the depth-first position of the drawable in the effect's scene graph (sprite batches
    /// share the numbering through [`BatchState::order`]).
    pub order: u32,
    pub name: Arc<str>,
    pub geometry: Arc<MeshGeometry>,
    /// Morphed positions replacing [`MeshGeometry::positions`], when the mesh is morphed.
    pub positions: Option<Arc<[[f32; 3]]>>,
    /// World transform: `p' = translation + scale * rotation * p`.
    pub rotation: Mat3,
    pub translation: Vec3,
    pub scale: f32,
    pub material: MaterialColors,
    pub vertex_color: NifVertexColor,
    pub apply_mode: TextureApplyMode,
    /// Texture stages; the base map, when present, is the one of slot [`TextureSlot::Base`].
    pub textures: Vec<TextureStage>,
    /// `None`: opaque (blending off).
    pub blend: Option<BlendState>,
    pub alpha_test: Option<AlphaTest>,
    pub depth: DepthState,
    pub depth_function: CompareFunction,
    pub cull: CullMode,
}

impl MeshDraw {
    pub fn base_texture(&self) -> Option<&TextureStage> {
        self.textures.iter().find(|stage| stage.slot == TextureSlot::Base)
    }

    /// Whether lighting is off because the base texture replaces the colour (`NiDX9LightManager::SetState`).
    pub fn texture_replaces_color(&self) -> bool {
        self.apply_mode == TextureApplyMode::Replace && self.base_texture().is_some()
    }

    /// Local vertex positions (morphed when morphed).
    pub fn local_positions(&self) -> &[[f32; 3]] {
        self.positions.as_deref().unwrap_or(&self.geometry.positions)
    }
}

/// The lit vertex colour of the fixed-function pipeline, as `NiDX9LightManager::SetState` configures it:
/// - lighting mode `E` with vertex source `IGNORE` or `AMB_DIFF`: material emissive and alpha (the ambient and
///   diffuse terms are off);
/// - `E` with source `EMISSIVE`: lighting off, the vertex colour (white without one);
/// - `E_A_D`: `emissive + ambient * La + diffuse * Ld`, where the vertex colour replaces the ambient and diffuse
///   material colours (source `AMB_DIFF`, alpha from the vertex) or the emissive colour (source `EMISSIVE`).
///
/// Without vertex colours the material colours are used (D3D falls back to the material). The result is
/// clamped to [0, 1].
pub fn light_vertex(
    material: &MaterialColors,
    vertex_color: Option<[f32; 4]>,
    mode: NifVertexColor,
    lighting: &SceneLighting,
) -> [f32; 4] {
    let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
    let lit = |emissive: [f32; 3], ambient: [f32; 3], diffuse: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|k| emissive[k] + ambient[k] * lighting.ambient[k] + diffuse[k] * lighting.diffuse[k])
    };
    let (color, alpha) = match (mode.lighting, mode.source) {
        (LightingMode::Emissive, VertexColorSource::Emissive) => match vertex_color {
            Some(vertex) => (rgb(vertex), vertex[3]),
            None => ([1.0; 3], 1.0),
        },
        (LightingMode::Emissive, _) => (material.emissive, material.alpha),
        (LightingMode::EmissiveAmbientDiffuse, VertexColorSource::AmbientDiffuse) => match vertex_color {
            Some(vertex) => (lit(material.emissive, rgb(vertex), rgb(vertex)), vertex[3]),
            None => (lit(material.emissive, material.ambient, material.diffuse), material.alpha),
        },
        (LightingMode::EmissiveAmbientDiffuse, VertexColorSource::Emissive) => {
            let emissive = vertex_color.map_or(material.emissive, rgb);
            (lit(emissive, material.ambient, material.diffuse), material.alpha)
        }
        (LightingMode::EmissiveAmbientDiffuse, VertexColorSource::Ignore) => {
            (lit(material.emissive, material.ambient, material.diffuse), material.alpha)
        }
    };
    [color[0].clamp(0.0, 1.0), color[1].clamp(0.0, 1.0), color[2].clamp(0.0, 1.0), alpha.clamp(0.0, 1.0)]
}

/// The texture stage of the fixed-function pipeline (`NiTexturingProperty::ApplyMode`): `color` is the lit vertex
/// colour, `texel` the base texture sample.
pub fn apply_texture(mode: TextureApplyMode, color: [f32; 4], texel: [f32; 4]) -> [f32; 4] {
    match mode {
        TextureApplyMode::Replace => texel,
        TextureApplyMode::Decal => {
            let a = texel[3];
            [
                color[0] + (texel[0] - color[0]) * a,
                color[1] + (texel[1] - color[1]) * a,
                color[2] + (texel[2] - color[2]) * a,
                color[3],
            ]
        }
        TextureApplyMode::Modulate => std::array::from_fn(|k| color[k] * texel[k]),
    }
}

/// `D3DTOP_MODULATE2X`: the scale of a detail map's modulation.
const DETAIL_MODULATE_SCALE: f32 = 2.0;

/// The texture stages of the fixed-function pipeline as `NiD3DDefaultShader::ConstructPipeline` chains them,
/// folded into one pass: `color` is the lit vertex colour and `samples` the texel of each map of the draw.
/// - base: [`apply_texture`] with the apply mode;
/// - dark: colour × texel (`D3DTOP_MODULATE`; alpha kept);
/// - detail: colour × 2 × texel (`D3DTOP_MODULATE2X`); without a base map it is the first stage and its alpha is
///   modulated the same way;
/// - decals: colour blended toward the texel by its alpha (`D3DTOP_BLENDTEXTUREALPHA`; alpha kept);
/// - glow: colour + texel (`D3DTOP_ADD`; alpha kept). The SDK draws it in an extra pass (source factor of the
///   alpha property, destination `ONE`) unless the draw already blends `ONE`/`ONE`; here it is a stage.
/// - gloss and bump maps need the environment and specular lighting and are ignored.
///
/// Each stage result is clamped to [0, 1], as the D3D stages are.
pub fn combine_textures(mode: TextureApplyMode, color: [f32; 4], samples: &[(TextureSlot, [f32; 4])]) -> [f32; 4] {
    let sample = |slot: TextureSlot| samples.iter().find(|(stage, _)| *stage == slot).map(|(_, texel)| *texel);
    let saturate = |c: [f32; 4]| c.map(|channel| channel.clamp(0.0, 1.0));
    let base = sample(TextureSlot::Base);
    let mut out = base.map_or(color, |texel| apply_texture(mode, color, texel));
    if let Some(texel) = sample(TextureSlot::Dark) {
        out = saturate([out[0] * texel[0], out[1] * texel[1], out[2] * texel[2], out[3]]);
    }
    if let Some(texel) = sample(TextureSlot::Detail) {
        let alpha = if base.is_some() { out[3] } else { out[3] * texel[3] * DETAIL_MODULATE_SCALE };
        out = saturate(std::array::from_fn(|k| {
            if k == ALPHA_CHANNEL { alpha } else { out[k] * texel[k] * DETAIL_MODULATE_SCALE }
        }));
    }
    for (_, texel) in samples.iter().filter(|(slot, _)| matches!(slot, TextureSlot::Decal(_))) {
        out = apply_texture(TextureApplyMode::Decal, out, *texel);
    }
    if let Some(texel) = sample(TextureSlot::Glow) {
        out = saturate([out[0] + texel[0], out[1] + texel[1], out[2] + texel[2], out[3]]);
    }
    out
}

/// Index of the alpha channel of an RGBA colour.
const ALPHA_CHANNEL: usize = 3;

/// The vertex indices of the triangles [`expand_mesh_draw`] emits (triangles with an index out of range are
/// skipped).
fn drawn_triangles(draw: &MeshDraw) -> impl Iterator<Item = [usize; TRIANGLE_VERTICES]> + '_ {
    let count = draw.local_positions().len();
    draw.geometry
        .triangles
        .iter()
        .map(|triangle| triangle.map(usize::from))
        .filter(move |triangle| triangle.iter().all(|&index| index < count))
}

/// Appends the texture coordinates of one texture stage for the vertices [`expand_mesh_draw`] emits (same order
/// and count): the stage's UV set (zeros when the mesh lacks it) with the stage's transform.
pub fn expand_mesh_stage_uvs(draw: &MeshDraw, stage: &TextureStage, out: &mut Vec<[f32; 2]>) {
    let uvs = draw.geometry.uv_sets.get(stage.uv_set as usize);
    for triangle in drawn_triangles(draw) {
        for index in triangle {
            out.push(uvs.and_then(|set| set.get(index)).map_or([0.0; 2], |&uv| stage.uv_transform.apply(uv)));
        }
    }
}

/// Appends the triangles of a mesh draw as a triangle list (3 vertices per triangle, in index order): world
/// positions, base-map UVs with the map's transform, and lit colours ([`light_vertex`]; white when the texture
/// replaces the colour). The texture is not applied; combine a sample with [`apply_texture`], or the samples of
/// every stage ([`expand_mesh_stage_uvs`]) with [`combine_textures`].
pub fn expand_mesh_draw(draw: &MeshDraw, lighting: &SceneLighting, out: &mut Vec<QuadVertex>) {
    let positions = draw.local_positions();
    let geometry = &draw.geometry;
    let base = draw.base_texture();
    let uvs = base.and_then(|stage| geometry.uv_sets.get(stage.uv_set as usize));
    let uv_transform = base.map_or(UvTransform::IDENTITY, |stage| stage.uv_transform);
    let replaces = draw.texture_replaces_color();
    let vertex = |index: usize| -> QuadVertex {
        let local = Vec3::from_array(positions[index]);
        let position = draw.translation + draw.rotation.mul_vec3(local) * draw.scale;
        let uv = uvs.and_then(|set| set.get(index)).map_or([0.0; 2], |&uv| uv_transform.apply(uv));
        let color = if replaces {
            [1.0; 4]
        } else {
            light_vertex(&draw.material, geometry.colors.get(index).copied(), draw.vertex_color, lighting)
        };
        QuadVertex { position, uv, color }
    };
    out.reserve(geometry.triangles.len() * TRIANGLE_VERTICES);
    for triangle in drawn_triangles(draw) {
        out.extend(triangle.map(vertex));
    }
}

/// Vertices per triangle.
pub const TRIANGLE_VERTICES: usize = 3;

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

    fn close(actual: [f32; 4], expected: [f32; 4]) -> bool {
        actual.iter().zip(&expected).all(|(a, e)| (a - e).abs() < EPSILON)
    }

    #[test]
    fn texture_stages_combine_like_the_fixed_function_pipeline() {
        let color = [0.5, 0.25, 1.0, 0.5];
        let texel = [0.5, 1.0, 0.5, 0.5];
        // Base map alone: the apply mode.
        let base = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Base, texel)]);
        assert!(close(base, [0.25, 0.25, 0.5, 0.25]));
        // Detail without a base map: colour and alpha modulated by 2 x texel, clamped.
        let detail = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Detail, texel)]);
        assert!(close(detail, [0.5, 0.5, 1.0, 0.5]));
        // Detail after a base map keeps the alpha.
        let both = combine_textures(
            TextureApplyMode::Modulate,
            color,
            &[(TextureSlot::Base, [1.0; 4]), (TextureSlot::Detail, [0.25; 4])],
        );
        assert!(close(both, [0.25, 0.125, 0.5, 0.5]));
        // Dark modulates the colour only; glow adds (clamped); decals blend by the texel alpha.
        let dark = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Dark, texel)]);
        assert!(close(dark, [0.25, 0.25, 0.5, 0.5]));
        let glow = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Glow, texel)]);
        assert!(close(glow, [1.0, 1.0, 1.0, 0.5]));
        let decal = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Decal(0), texel)]);
        assert!(close(decal, [0.5, 0.625, 0.75, 0.5]));
        // Gloss and bump maps are ignored.
        let gloss = combine_textures(TextureApplyMode::Modulate, color, &[(TextureSlot::Gloss, texel)]);
        assert!(close(gloss, color));
    }

    #[test]
    fn stage_uvs_follow_the_expanded_vertices() {
        let geometry = MeshGeometry {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uv_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![[0.5; 2]; 3]],
            // The second triangle has an index out of range and is skipped.
            triangles: vec![[0, 1, 2], [0, 1, 7]],
            ..MeshGeometry::default()
        };
        let stage = |slot, uv_set| TextureStage {
            slot,
            texture: Arc::from("t"),
            uv_set,
            uv_transform: UvTransform::IDENTITY,
            addressing: TextureAddressing { wrap_s: true, wrap_t: true },
        };
        let draw = MeshDraw {
            order: 0,
            name: Arc::from("mesh"),
            geometry: Arc::new(geometry),
            positions: None,
            rotation: Mat3::IDENTITY,
            translation: Vec3::ZERO,
            scale: 1.0,
            material: DEFAULT_MATERIAL_COLORS,
            vertex_color: NifVertexColor::default(),
            apply_mode: TextureApplyMode::Modulate,
            textures: vec![stage(TextureSlot::Base, 0), stage(TextureSlot::Detail, 1), stage(TextureSlot::Glow, 2)],
            blend: None,
            alpha_test: None,
            depth: DepthState { test: true, write: true },
            depth_function: CompareFunction::LessEqual,
            cull: CullMode::Back,
        };
        let mut vertices = Vec::new();
        expand_mesh_draw(&draw, &SceneLighting::default(), &mut vertices);
        assert_eq!(vertices.len(), TRIANGLE_VERTICES);
        let uvs = |stage: &TextureStage| {
            let mut out = Vec::new();
            expand_mesh_stage_uvs(&draw, stage, &mut out);
            out
        };
        let base: Vec<[f32; 2]> = vertices.iter().map(|vertex| vertex.uv).collect();
        assert_eq!(uvs(&draw.textures[0]), base);
        assert_eq!(uvs(&draw.textures[1]), vec![[0.5; 2]; TRIANGLE_VERTICES]);
        // A missing UV set gives zeros.
        assert_eq!(uvs(&draw.textures[2]), vec![[0.0; 2]; TRIANGLE_VERTICES]);
    }
}
