//! Engine-neutral render output.
//!
//! A system writes one [`RenderFrame`] per frame: draw batches of camera-facing sprites and a list of mesh
//! instances. Nothing here depends on a graphics API; a consumer either
//! - uploads [`SpriteInstance`]s and expands each into a quad on the GPU (the formulas are those of
//!   [`expand_sprite`]), or
//! - calls [`expand_frame`] to get 4 [`QuadVertex`]es per sprite (draw them with [`QUAD_INDICES`]).
//!
//! Positions are in the effect's space. The client turns every effect node to its camera, so there X is camera
//! right, Y the view direction and Z camera up (confirmed: the effect rotation of `g_UOWorldViewer` 0x00e37224;
//! `uosa_effects.md`, "Effect camera and lighting"). Sprites within a batch are drawn in order (oldest particle
//! first, one batch per sequence). Every batch and mesh draw carries a [`SortKey`]: the client's alpha accumulator
//! draws the unsorted drawables first, in scene order, then the sorted ones far to near by their bound centre
//! (`uosa_effects.md`, "Draw order"; [`crate::raster::Canvas::draw_group`]).
//!
//! Quad layout (Chaos `ParticleTriShape::BuildVertices` 0x008c4c30): vertex k has base UV (0,1), (0,0), (1,0),
//! (1,1), mapped into the sprite's [`UvRect`]; triangles are (0, 2, 1) and (0, 2, 3). Colour is per sprite
//! (all four vertices share it), already clamped to [0, 1], and modulates the texture (unlit).
//!
//! NIF effects ([`crate::nif`]) also fill [`RenderFrame::mesh_draws`] (triangle meshes with their world
//! transform, material, texture stages and render state) and [`RenderFrame::batch_states`] (the render state
//! of their sprite batches that [`DrawBatch`] does not carry). Both carry a scene `order`: draw batches and mesh
//! draws interleaved by it reproduce the scene graph's draw order. [`expand_mesh_draw`] lights and transforms a
//! mesh draw into a triangle list for software rasterizers, and [`combine_textures`] is the client's pixel colour
//! (`NiStandardMaterial`).

use std::ops::Range;
use std::sync::Arc;

use crate::math::{Mat3, Vec3};
use crate::nif::desc::{
    CompareFunction, CullMode, LightAttenuation, LightingMode, NifVertexColor, TextureAddressing, TextureApplyMode,
    TextureCoordGen, TextureFilter, TextureSlot, VertexColorSource,
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

/// Where a drawable goes in the draw order of the effect render group (`NiAlphaAccumulator`, UOSA.exe
/// `FinishAccumulating` 0x006d6990; `uosa_effects.md`, "Draw order").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SortKey {
    /// Drawn with the sorted drawables (alpha blending on, no `NoSorter` flag, not under an `NiSortAdjustNode`
    /// with sorting off: `NiAlphaAccumulator::RegisterObjectArray`, `NiAlphaAccumulator.cpp:33-66`); otherwise
    /// drawn first, in scene order.
    pub sorted: bool,
    /// World centre of the drawable's bound: the sorted drawables are drawn far to near by its depth along the
    /// camera direction (`NiAlphaAccumulator.cpp:68-110`, `SortByClosestPoint` off).
    pub center: Vec3,
}

impl SortKey {
    /// An unsorted drawable (blending off) at `center`.
    pub fn unsorted(center: Vec3) -> Self {
        Self { sorted: false, center }
    }

    /// A sorted drawable at `center`.
    pub fn sorted(center: Vec3) -> Self {
        Self { sorted: true, center }
    }
}

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
    /// The batch's place in the draw order: its whole set of sprites is one drawable.
    pub sort: SortKey,
}

/// A particle drawn as a mesh (Chaos MESH sequences): a clone of the `.nif` scene per particle. A Chaos effect
/// that can load its mesh NIFs draws the clones' own [`MeshDraw`]s and batches instead
/// ([`crate::chaos::MeshSource`]); these instances are the placements of the others.
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
    /// Particle colour, RGBA in [0, 1]. The client does not apply it to the clone (`uosa_chaos.md`, "MESH
    /// particles").
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

    /// Appends `other`'s draws after this frame's, its scene orders shifted by `order`; returns the next free
    /// order number (`order` past the highest appended one).
    pub fn append(&mut self, other: &RenderFrame, order: u32) -> u32 {
        let sprites = self.sprites.len();
        let batches = self.batches.len();
        self.sprites.extend_from_slice(&other.sprites);
        self.batches.extend(other.batches.iter().map(|batch| DrawBatch {
            sprites: batch.sprites.start + sprites..batch.sprites.end + sprites,
            ..batch.clone()
        }));
        self.batch_states.extend(other.batch_states.iter().map(|state| BatchState {
            batch: state.batch + batches,
            order: state.order + order,
            ..state.clone()
        }));
        self.meshes.extend_from_slice(&other.meshes);
        self.mesh_draws.extend(other.mesh_draws.iter().map(|draw| MeshDraw { order: draw.order + order, ..draw.clone() }));
        let orders = other.mesh_draws.iter().map(|draw| draw.order).chain(other.batch_states.iter().map(|state| state.order));
        orders.max().map_or(order, |last| order + last + 1)
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
    /// The map's sampler filters (`NiTexturingProperty::FilterMode`).
    pub filter: TextureFilter,
}

/// The lights an effect is lit by. Diffuse light is applied without direction (as if facing every surface). The
/// effect's own lights ([`EffectLight`], carried by each draw) add to it.
///
/// The default, [`CLIENT_EFFECT_LIGHTING`], is the client's: no light at all (confirmed). Effect NIFs hang under
/// render group node 7 (`g_UOEffectRenderGroups` 0x00e32e28), a parentless node with an empty effect list; no
/// game code attaches a light to it (`NiDynamicEffect::AttachAffectedNode` 0x006a1900 is called only by NIF
/// streaming, cloning and the UI's `NifDisplay`), and the world's day/night light does not reach it. With no
/// light the renderer's ambient uniform is 0 (`SCM_DEF_AMBIENTLIGHT` sums the attached `NiAmbientLight`s) and
/// so is the diffuse sum, so both of the client's pipelines (`NiStandardMaterial` on shader model 2 hardware,
/// chosen by `NiDX9Renderer` init at 0x00755612, and the legacy fixed-function one) draw `saturate(emissive)`:
/// `E_A_D` materials show only their emissive colour. Lights inside an effect NIF light its own subtree (one
/// shipped file, `00005540_shadowstrike.nif`, has two white `NiDirectionalLight`s on its root): the NIF effect
/// runtime puts them on its draws ([`MeshDraw::lights`]) and they add to this.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLighting {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
}

/// The client's effect lighting: no light (see [`SceneLighting`]).
pub const CLIENT_EFFECT_LIGHTING: SceneLighting = SceneLighting { ambient: [0.0; 3], diffuse: [0.0; 3] };

impl Default for SceneLighting {
    fn default() -> Self {
        CLIENT_EFFECT_LIGHTING
    }
}

/// `NiSpotLight` inner cone cosine: NIF 20.0.0.4 stores no inner angle (streamed since 20.2.0.5), so it is the
/// constructor's 0 degrees and its cosine 1 (`NiSpotLight::SetInnerSpotAngle`).
pub const SPOT_INNER_COS: f32 = 1.0;

/// The type and placement of an [`EffectLight`], in world space. A direction is the one light travels along (the
/// light's world rotation column 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectLightKind {
    Ambient,
    Directional { direction: Vec3 },
    Point { position: Vec3, attenuation: LightAttenuation },
    /// `cos_outer`: cosine of the outer half angle.
    Spot { position: Vec3, direction: Vec3, attenuation: LightAttenuation, cos_outer: f32, exponent: f32 },
}

/// A light of the effect's own scene (an `NiLight` of an effect NIF) that is on, with its colours multiplied by
/// its dimmer (`SCM_OBJ_AMBIENT` / `DIFFUSE`). Specular light is not applied: it needs an `NiSpecularProperty`,
/// which no shipped effect with lights has.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLight {
    pub kind: EffectLightKind,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
}

/// The light reaching a vertex: `base` plus the effect lights, as the client's per-vertex `NiStandardMaterial`
/// light fragment sums them (`StandardMaterialNodeLibraryFragments.xml`, fragment `Light`):
/// - an ambient light adds its ambient colour (`SCM_DEF_AMBIENTLIGHT`);
/// - another light adds `diffuse × max(0, N·L) × attenuation` to the diffuse light and `ambient × attenuation` to
///   the ambient light, with `L = -direction` for a directional light (attenuation 1) and the unit vector to the
///   light otherwise; point and spot lights attenuate by `1 / max(1, c + l d + q d²)`, spot lights also by
///   `saturate((cos α - cos_outer) / (cos_inner - cos_outer))^exponent` (0 when that is 0), `α` the angle between
///   the light direction and the light-to-vertex vector.
///
/// `normal` is the unit world normal; geometry without normals is lit by ambient lights only
/// (`NiStandardMaterial` sets the light count to 0 without normals).
pub fn accumulate_lights(base: &SceneLighting, lights: &[EffectLight], position: Vec3, normal: Option<Vec3>) -> SceneLighting {
    let mut out = *base;
    for light in lights {
        let (to_light, attenuation) = match light.kind {
            EffectLightKind::Ambient => (Vec3::ZERO, 1.0),
            _ if normal.is_none() => continue,
            EffectLightKind::Directional { direction } => (-direction, 1.0),
            EffectLightKind::Point { position: light_position, attenuation } => {
                let (to_light, distance) = toward(light_position, position);
                (to_light, distance_attenuation(attenuation, distance))
            }
            EffectLightKind::Spot { position: light_position, direction, attenuation, cos_outer, exponent } => {
                let (to_light, distance) = toward(light_position, position);
                let cone = spot_attenuation((-to_light).dot(direction), cos_outer, exponent);
                (to_light, distance_attenuation(attenuation, distance) * cone)
            }
        };
        // An ambient light has no diffuse term (its zero vector gives intensity 0).
        let intensity = normal.map_or(0.0, |normal| to_light.dot(normal).max(0.0));
        for k in 0..3 {
            out.diffuse[k] += light.diffuse[k] * intensity * attenuation;
            out.ambient[k] += light.ambient[k] * attenuation;
        }
    }
    out
}

/// The unit vector from `position` to `light` and the distance.
fn toward(light: Vec3, position: Vec3) -> (Vec3, f32) {
    let offset = light - position;
    (offset.normalize_or_zero(), offset.length())
}

/// Smallest distance attenuation divisor of the light fragment (`Attenuate = max(1.0, Attenuate)`).
const MIN_ATTENUATION_DIVISOR: f32 = 1.0;

fn distance_attenuation(attenuation: LightAttenuation, distance: f32) -> f32 {
    let LightAttenuation { constant, linear, quadratic } = attenuation;
    1.0 / (constant + linear * distance + quadratic * distance * distance).max(MIN_ATTENUATION_DIVISOR)
}

/// The spot cone factor of the light fragment; a cone without width (outer cosine 1) lights only along its axis.
fn spot_attenuation(cos_alpha: f32, cos_outer: f32, exponent: f32) -> f32 {
    let width = SPOT_INNER_COS - cos_outer;
    let factor = if width > 0.0 {
        ((cos_alpha - cos_outer) / width).clamp(0.0, 1.0)
    } else if cos_alpha >= SPOT_INNER_COS {
        1.0
    } else {
        0.0
    };
    if factor <= 0.0 { 0.0 } else { factor.powf(exponent) }
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
    /// The effect's own lights that reach this mesh (empty for nearly every effect), added per vertex to the
    /// [`SceneLighting`] by [`expand_mesh_draw`].
    pub lights: Vec<EffectLight>,
    /// The environment map reaching this mesh (an `NiTextureEffect`), when the mesh has the normals it needs.
    pub env_map: Option<EnvironmentMap>,
    /// The draw's place in the draw order (bound centre: the model bound's, in world space).
    pub sort: SortKey,
}

/// An environment map (`NiTextureEffect` with type `ENVIRONMENT_MAP`) as `NiStandardMaterial` applies it: texture
/// coordinates generated per vertex from a world-space source vector `s` (`HandleTextureEffectUVSetGen`,
/// `NiStandardMaterial.cpp:4827`) through the effect's world projection, `uv = projection * s + translation`
/// (`ProjectTextureCoordinates` with `SCM_OBJ_WORLDPROJECTIONTRANSFORM`, `NiD3DShaderConstantMap.cpp:3658`);
/// the sample, times the gloss map when there is one, is added to the colour ([`combine_textures`]).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentMap {
    pub texture: Arc<str>,
    pub coord_gen: TextureCoordGen,
    /// `m_kWorldProjMat` (`NiTextureEffect::UpdateProjection`, `NiTextureEffect.cpp:92-144`).
    pub projection: Mat3,
    /// `m_kWorldProjTrans`.
    pub translation: Vec3,
    pub addressing: TextureAddressing,
    pub filter: TextureFilter,
}

impl EnvironmentMap {
    /// The texture coordinates of a vertex at world `position` with unit world `normal`, seen from
    /// `camera_position`:
    /// - world parallel: the position projected;
    /// - world perspective: the position projected, divided by the third component (`tex2Dproj`);
    /// - sphere map: the unit reflection of the camera-to-vertex vector about the normal (`WorldReflect`,
    ///   `reflect(-(CameraPos - WorldPos), N)`, normalized) projected;
    /// - cube maps are looked up by a direction, not a coordinate pair: their maps are not drawn (`None`).
    pub fn uv(&self, position: Vec3, normal: Vec3, camera_position: Vec3) -> Option<[f32; 2]> {
        let source = match self.coord_gen {
            TextureCoordGen::WorldParallel | TextureCoordGen::WorldPerspective => position,
            TextureCoordGen::SphereMap => {
                let incident = position - camera_position;
                (incident - normal * (2.0 * normal.dot(incident))).normalize_or_zero()
            }
            TextureCoordGen::SpecularCubeMap | TextureCoordGen::DiffuseCubeMap => return None,
        };
        let projected = self.projection.mul_vec3(source) + self.translation;
        if self.coord_gen == TextureCoordGen::WorldPerspective {
            return (projected.z != 0.0).then(|| [projected.x / projected.z, projected.y / projected.z]);
        }
        Some([projected.x, projected.y])
    }
}

impl MeshDraw {
    pub fn base_texture(&self) -> Option<&TextureStage> {
        self.textures.iter().find(|stage| stage.slot == TextureSlot::Base)
    }

    /// Whether lighting and the vertex and material colours are off because the textures replace the colour
    /// (`NiStandardMaterial`: apply mode `REPLACE`, `NiStandardMaterial.cpp:949`, `:3244`).
    pub fn texture_replaces_color(&self) -> bool {
        self.apply_mode == TextureApplyMode::Replace
    }

    /// Local vertex positions (morphed when morphed).
    pub fn local_positions(&self) -> &[[f32; 3]] {
        self.positions.as_deref().unwrap_or(&self.geometry.positions)
    }
}

/// The per-vertex colour `Kd` of `NiStandardMaterial` (per-vertex lighting, `HandleVertexLightingAndMaterials`):
/// - lighting mode `E`: the emissive colour, which is the vertex colour with source `EMISSIVE` and the material
///   emissive otherwise (the ambient and diffuse terms are off);
/// - `E_A_D`: `emissive + ambient * La + diffuse * Ld`, where the vertex colour replaces the ambient and diffuse
///   material colours (source `AMB_DIFF`) or the emissive colour (source `EMISSIVE`).
///
/// The alpha is the vertex alpha for (`E`, `EMISSIVE`) and (`E_A_D`, `AMB_DIFF`), the material alpha otherwise.
/// Without vertex colours the material colours are used. Nothing is clamped: the client's material does not
/// saturate the shading (`m_bSaturateShading` off), and only the pixel alpha is saturated ([`combine_textures`]).
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
            None => (material.emissive, material.alpha),
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
    [color[0], color[1], color[2], alpha]
}

/// Scale of a detail map's modulation: `NiStandardMaterial::HandleDetailMap` multiplies by `2 * texel`
/// (`NiStandardMaterial.cpp:4296`).
const DETAIL_MODULATE_SCALE: f32 = 2.0;

/// The pixel colour of `NiStandardMaterial` (shader model 2, the client's pipeline): `kd` is the interpolated
/// vertex colour ([`light_vertex`]), `samples` the texel of each map of the draw, `env` the environment map
/// sample ([`EnvironmentMap`]).
/// - the texture colour: `tex = dark × base × 2·detail` (missing maps count as 1; `HandleBaseMap`,
///   `HandleDarkMap`, `HandleDetailMap`), then each decal blends it toward its texel by the texel alpha
///   (`HandleDecalMap`); no map contributes alpha but the base map;
/// - apply mode `MODULATE` (and `DECAL`, which the material treats as `MODULATE`,
///   `NiStandardMaterialDescriptor.cpp:863-872`): `rgb = Kd.rgb × tex`, `a = Kd.a × base.a`;
///   `REPLACE`: `rgb = tex`, `a = base.a` (`HandleColorAccumulation`, `NiStandardMaterial.cpp:3224`);
/// - the environment map, times the gloss map's colour when there is one, adds to the colour (`HandleEnvMap`
///   `:4374`, `HandleGlossMap` `:4308`), then the glow map's colour adds (`HandleGlowMap` `:4344`), both in the
///   same pass (`HandlePostLightTextureApplication` `:5099`);
/// - bump maps are ignored.
///
/// Nothing is saturated in between (the client's material saturates neither shading nor textures); only the
/// alpha is saturated here. The colour may exceed 1: the render target clamps it ([`crate::raster::Canvas::blend`]).
pub fn combine_textures(
    mode: TextureApplyMode,
    kd: [f32; 4],
    samples: &[(TextureSlot, [f32; 4])],
    env: Option<[f32; 4]>,
) -> [f32; 4] {
    let sample = |slot: TextureSlot| samples.iter().find(|(stage, _)| *stage == slot).map(|(_, texel)| *texel);
    let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
    let base = sample(TextureSlot::Base);
    let mut tex = base.map_or([1.0; 3], rgb);
    if let Some(dark) = sample(TextureSlot::Dark) {
        tex = std::array::from_fn(|k| tex[k] * dark[k]);
    }
    if let Some(detail) = sample(TextureSlot::Detail) {
        tex = std::array::from_fn(|k| tex[k] * DETAIL_MODULATE_SCALE * detail[k]);
    }
    for (_, decal) in samples.iter().filter(|(slot, _)| matches!(slot, TextureSlot::Decal(_))) {
        let a = decal[ALPHA_CHANNEL];
        tex = std::array::from_fn(|k| tex[k] + (decal[k] - tex[k]) * a);
    }
    let base_alpha = base.map_or(1.0, |texel| texel[ALPHA_CHANNEL]);
    let (mut color, alpha) = match mode {
        TextureApplyMode::Replace => (tex, base_alpha),
        TextureApplyMode::Modulate | TextureApplyMode::Decal => {
            (std::array::from_fn(|k| kd[k] * tex[k]), kd[ALPHA_CHANNEL] * base_alpha)
        }
    };
    if let Some(env) = env {
        let gloss = sample(TextureSlot::Gloss).map_or([1.0; 3], rgb);
        color = std::array::from_fn(|k| color[k] + env[k] * gloss[k]);
    }
    if let Some(glow) = sample(TextureSlot::Glow) {
        color = std::array::from_fn(|k| color[k] + glow[k]);
    }
    [color[0], color[1], color[2], alpha.clamp(0.0, 1.0)]
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

/// Appends the environment map coordinates ([`EnvironmentMap::uv`]) for the vertices [`expand_mesh_draw`] emits
/// (same order and count), seen from `camera_position`; nothing when the draw has no environment map, the map is
/// a cube map, or the mesh has no normals.
pub fn expand_mesh_env_uvs(draw: &MeshDraw, camera_position: Vec3, out: &mut Vec<[f32; 2]>) {
    let Some(env) = &draw.env_map else { return };
    let normals = &draw.geometry.normals;
    let positions = draw.local_positions();
    if normals.len() < positions.len() {
        return;
    }
    let start = out.len();
    for triangle in drawn_triangles(draw) {
        for index in triangle {
            let position = draw.translation + draw.rotation.mul_vec3(Vec3::from_array(positions[index])) * draw.scale;
            let normal = draw.rotation.mul_vec3(Vec3::from_array(normals[index])).normalize_or_zero();
            let Some(uv) = env.uv(position, normal, camera_position) else {
                out.truncate(start);
                return;
            };
            out.push(uv);
        }
    }
}

/// Appends the triangles of a mesh draw as a triangle list (3 vertices per triangle, in index order): world
/// positions, base-map UVs with the map's transform, and vertex colours `Kd` ([`light_vertex`] with `lighting`
/// plus the draw's own [`MeshDraw::lights`] at the vertex ([`accumulate_lights`], world normals from the mesh
/// normals); white when the textures replace the colour). The textures are not applied: combine the samples of
/// every stage ([`expand_mesh_stage_uvs`]) and the environment map ([`expand_mesh_env_uvs`]) with
/// [`combine_textures`].
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
        } else if draw.lights.is_empty() {
            light_vertex(&draw.material, geometry.colors.get(index).copied(), draw.vertex_color, lighting)
        } else {
            let normal = geometry
                .normals
                .get(index)
                .map(|&normal| draw.rotation.mul_vec3(Vec3::from_array(normal)).normalize_or_zero());
            let lit = accumulate_lights(lighting, &draw.lights, position, normal);
            light_vertex(&draw.material, geometry.colors.get(index).copied(), draw.vertex_color, &lit)
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

impl CameraFrame {
    /// The client's view of an effect, in the effect's local frame: every NIF and Chaos effect node takes the
    /// effect rotation `C · P` (`g_UOWorldViewer` 0x00e37224 +0x30, `UOWorldViewer::Init` 0x0046fa70), so
    /// locally X is camera right, Y the view direction (into the screen) and Z camera up, and the world camera is
    /// orthographic (`uosa_effects.md`, "Effect camera and lighting"). The camera stands `distance` behind the
    /// origin; only axial sprites and face-centre billboards see its position.
    pub fn client_view(distance: f32) -> Self {
        Self { position: Vec3::Y * -distance, direction: Vec3::Y, up: Vec3::Z, right: Vec3::X }
    }
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
///   `MakeRotation(-rotation, D) × camera_rotation`; `NiMatrix3::MakeRotation` turns clockwise
///   (`NiMatrix3.cpp:113-138`), so R and U turn counterclockwise by `rotation` about D: confirmed).
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

    /// Distance of the test camera behind the origin.
    const TEST_CAMERA_DISTANCE: f32 = 10.0;

    fn camera_looking_along_y() -> CameraFrame {
        CameraFrame::client_view(TEST_CAMERA_DISTANCE)
    }

    #[test]
    fn the_client_view_looks_along_local_y_with_z_up() {
        let camera = camera_looking_along_y();
        assert_eq!((camera.direction, camera.up, camera.right), (Vec3::Y, Vec3::Z, Vec3::X));
        assert_close(camera.position, Vec3::new(0.0, -TEST_CAMERA_DISTANCE, 0.0));
        assert_close(camera.direction.cross(camera.up), camera.right);
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
    fn texture_stages_combine_like_the_standard_material() {
        let modulate = |kd, samples: &[(TextureSlot, [f32; 4])]| combine_textures(TextureApplyMode::Modulate, kd, samples, None);
        let color = [0.5, 0.25, 1.0, 0.5];
        let texel = [0.5, 1.0, 0.5, 0.5];
        // Base map alone: Kd x base, alpha Kd.a x base.a.
        assert!(close(modulate(color, &[(TextureSlot::Base, texel)]), [0.25, 0.25, 0.5, 0.25]));
        // Detail: 2 x texel on the colour, never on the alpha (even without a base map), nothing clamped.
        let detail = [0.75, 0.75, 0.75, 0.25];
        assert!(close(modulate(color, &[(TextureSlot::Detail, detail)]), [0.75, 0.375, 1.5, 0.5]));
        let both = modulate(color, &[(TextureSlot::Base, [1.0; 4]), (TextureSlot::Detail, [0.25; 4])]);
        assert!(close(both, [0.25, 0.125, 0.5, 0.5]));
        // Dark modulates the colour only.
        assert!(close(modulate(color, &[(TextureSlot::Dark, texel)]), [0.25, 0.25, 0.5, 0.5]));
        // Glow adds after the Kd multiply, unclamped.
        assert!(close(modulate(color, &[(TextureSlot::Glow, texel)]), [1.0, 1.25, 1.5, 0.5]));
        // A decal blends the texture colour toward its texel by its alpha, before the Kd multiply:
        // Kd x lerp(1, texel, 0.5).
        assert!(close(modulate(color, &[(TextureSlot::Decal(0), texel)]), [0.375, 0.25, 0.75, 0.5]));
        let decal_over_base = modulate(color, &[(TextureSlot::Base, [0.0, 0.0, 0.0, 1.0]), (TextureSlot::Decal(1), [1.0; 4])]);
        assert!(close(decal_over_base, [0.5, 0.25, 1.0, 0.5]));
        // DECAL is MODULATE for the material.
        let decal_mode = combine_textures(TextureApplyMode::Decal, color, &[(TextureSlot::Base, texel)], None);
        assert!(close(decal_mode, [0.25, 0.25, 0.5, 0.25]));
        // REPLACE: the texture colour and the base alpha; glow still adds.
        let replace = combine_textures(
            TextureApplyMode::Replace,
            color,
            &[(TextureSlot::Base, texel), (TextureSlot::Glow, [0.25; 4])],
            None,
        );
        assert!(close(replace, [0.75, 1.25, 0.75, 0.5]));
        // The environment map adds, times the gloss map.
        let env = combine_textures(
            TextureApplyMode::Modulate,
            color,
            &[(TextureSlot::Base, [1.0; 4]), (TextureSlot::Gloss, [0.5; 4])],
            Some([0.25; 4]),
        );
        assert!(close(env, [0.625, 0.375, 1.125, 0.5]));
        // Gloss without an environment map and bump maps change nothing.
        assert!(close(modulate(color, &[(TextureSlot::Gloss, texel)]), color));
        // Only the final alpha is saturated.
        assert!(close(modulate([2.0, 2.0, 2.0, 2.0], &[(TextureSlot::Base, [1.0, 1.0, 1.0, 0.75])]), [2.0, 2.0, 2.0, 1.0]));
    }

    #[test]
    fn sphere_environment_maps_reflect_the_view_about_the_normal() {
        let env = EnvironmentMap {
            texture: Arc::from("env"),
            coord_gen: TextureCoordGen::SphereMap,
            // The sphere map projection of an effect at the identity (`NiTextureEffect::UpdateProjection`):
            // u = 0.5 + 0.5 r.z, v = 0.5 - 0.5 r.y.
            projection: Mat3::from_rows([[0.0, 0.0, 0.5], [0.0, -0.5, 0.0], [0.0, 0.0, 0.0]]),
            translation: Vec3::new(0.5, 0.5, 0.0),
            addressing: TextureAddressing { wrap_s: true, wrap_t: true },
            filter: TextureFilter::BILERP,
        };
        // The effect's model space looks along +X (the Gamebryo camera axis): the sphere map ignores x.
        let camera = Vec3::new(-10.0, 0.0, 0.0);
        // Seen head-on, the reflection points back at the camera (-X): the centre of the map.
        let uv = env.uv(Vec3::ZERO, -Vec3::X, camera).unwrap();
        assert!((uv[0] - 0.5).abs() < EPSILON && (uv[1] - 0.5).abs() < EPSILON, "{uv:?}");
        // A normal tilted 45 degrees toward +Z reflects the view along +Z: u = 1.
        let tilted = Vec3::new(-1.0, 0.0, 1.0).normalize_or_zero();
        let uv = env.uv(Vec3::ZERO, tilted, camera).unwrap();
        assert!((uv[0] - 1.0).abs() < EPSILON && (uv[1] - 0.5).abs() < EPSILON, "{uv:?}");
        // Tilted toward +Y: v = 0.
        let tilted = Vec3::new(-1.0, 1.0, 0.0).normalize_or_zero();
        let uv = env.uv(Vec3::ZERO, tilted, camera).unwrap();
        assert!((uv[0] - 0.5).abs() < EPSILON && uv[1].abs() < EPSILON, "{uv:?}");
        // Cube maps are not generated.
        assert!(EnvironmentMap { coord_gen: TextureCoordGen::SpecularCubeMap, ..env }.uv(Vec3::ZERO, -Vec3::X, camera).is_none());
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
            filter: TextureFilter::BILERP,
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
            lights: Vec::new(),
            env_map: None,
            sort: SortKey::unsorted(Vec3::ZERO),
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

    fn close3(actual: [f32; 3], expected: [f32; 3]) -> bool {
        actual.iter().zip(&expected).all(|(a, e)| (a - e).abs() < EPSILON)
    }

    fn white(kind: EffectLightKind) -> EffectLight {
        EffectLight { kind, ambient: [0.25; 3], diffuse: [1.0; 3] }
    }

    #[test]
    fn effect_lights_add_by_normal_distance_and_cone() {
        let base = SceneLighting { ambient: [0.1; 3], diffuse: [0.0; 3] };
        let up = Some(Vec3::Z);
        // A directional light shining down lights an upward normal fully, plus its ambient colour.
        let down = white(EffectLightKind::Directional { direction: -Vec3::Z });
        let lit = accumulate_lights(&base, &[down], Vec3::ZERO, up);
        assert!(close3(lit.diffuse, [1.0; 3]) && close3(lit.ambient, [0.35; 3]));
        // Facing away: no diffuse light, the ambient term stays.
        let lit = accumulate_lights(&base, &[down], Vec3::ZERO, Some(-Vec3::Z));
        assert!(close3(lit.diffuse, [0.0; 3]) && close3(lit.ambient, [0.35; 3]));
        // Without normals only ambient lights count.
        let ambient = white(EffectLightKind::Ambient);
        let lit = accumulate_lights(&base, &[down, ambient], Vec3::ZERO, None);
        assert!(close3(lit.diffuse, [0.0; 3]) && close3(lit.ambient, [0.35; 3]));
        // A point light 2 above with linear attenuation 1: 1 / max(1, 2) = 0.5.
        let attenuation = LightAttenuation::default();
        let point = white(EffectLightKind::Point { position: Vec3::Z * 2.0, attenuation });
        let lit = accumulate_lights(&base, &[point], Vec3::ZERO, up);
        assert!(close3(lit.diffuse, [0.5; 3]));
        // Close lights are not brightened beyond the colour (the divisor is at least 1).
        let near = white(EffectLightKind::Point { position: Vec3::Z * 0.5, attenuation });
        assert!(close3(accumulate_lights(&base, &[near], Vec3::ZERO, up).diffuse, [1.0; 3]));
        // A 60-degree spot cone pointing down: full on its axis, (cos α - cos 60) / (1 - cos 60) off it, 0 outside.
        let spot = |position: Vec3| {
            let light = white(EffectLightKind::Spot {
                position,
                direction: -Vec3::Z,
                attenuation: LightAttenuation { constant: 1.0, linear: 0.0, quadratic: 0.0 },
                cos_outer: 0.5,
                exponent: 1.0,
            });
            accumulate_lights(&SceneLighting::default(), &[light], Vec3::ZERO, up).ambient[0] / 0.25
        };
        assert!((spot(Vec3::Z) - 1.0).abs() < EPSILON);
        let off_axis = Vec3::new(1.0, 0.0, 3.0_f32.sqrt());
        let cos_alpha = 3.0_f32.sqrt() / 2.0;
        assert!((spot(off_axis) - (cos_alpha - 0.5) / 0.5).abs() < EPSILON);
        assert_eq!(spot(Vec3::new(1.0, 0.0, 0.1)), 0.0);
    }

    #[test]
    fn vertex_colours_feed_kd_by_lighting_mode_and_source() {
        use crate::nif::desc::{LightingMode, VertexColorSource};
        let material = MaterialColors {
            ambient: [0.5; 3],
            diffuse: [0.25; 3],
            emissive: [0.125, 0.25, 0.5],
            alpha: 0.75,
            ..DEFAULT_MATERIAL_COLORS
        };
        let lighting = SceneLighting { ambient: [1.0; 3], diffuse: [2.0; 3] };
        let vertex = Some([2.0, 1.0, 0.5, 0.25]);
        let mode = |lighting, source| NifVertexColor { lighting, source };
        let kd = |vertex, lighting_mode, source| light_vertex(&material, vertex, mode(lighting_mode, source), &lighting);
        // E with source EMISSIVE: the vertex colour and alpha, unclamped; without vertex colours the material's.
        assert!(close(kd(vertex, LightingMode::Emissive, VertexColorSource::Emissive), [2.0, 1.0, 0.5, 0.25]));
        assert!(close(kd(None, LightingMode::Emissive, VertexColorSource::Emissive), [0.125, 0.25, 0.5, 0.75]));
        // E with another source: the material emissive and alpha.
        assert!(close(kd(vertex, LightingMode::Emissive, VertexColorSource::AmbientDiffuse), [0.125, 0.25, 0.5, 0.75]));
        // E_A_D with AMB_DIFF: the vertex colour lights as ambient and diffuse; its alpha.
        let lit = |c: f32, e: f32| e + c * 1.0 + c * 2.0;
        let expected = [lit(2.0, 0.125), lit(1.0, 0.25), lit(0.5, 0.5), 0.25];
        assert!(close(kd(vertex, LightingMode::EmissiveAmbientDiffuse, VertexColorSource::AmbientDiffuse), expected));
        // E_A_D with EMISSIVE: the vertex colour is the emissive term; the material alpha.
        let expected = [2.0 + 0.5 + 0.5, 1.0 + 0.5 + 0.5, 0.5 + 0.5 + 0.5, 0.75];
        assert!(close(kd(vertex, LightingMode::EmissiveAmbientDiffuse, VertexColorSource::Emissive), expected));
        // E_A_D without lights and emissive: black, so a texture shows only through glow (`0 x tex + glow`).
        let dark = MaterialColors { emissive: [0.0; 3], ..material };
        let unlit = light_vertex(&dark, None, mode(LightingMode::EmissiveAmbientDiffuse, VertexColorSource::Ignore), &CLIENT_EFFECT_LIGHTING);
        let pixel = combine_textures(TextureApplyMode::Modulate, unlit, &[(TextureSlot::Base, [1.0; 4]), (TextureSlot::Glow, [0.25; 4])], None);
        assert!(close(pixel, [0.25, 0.25, 0.25, 0.75]));
    }

    #[test]
    fn mesh_draw_lights_follow_the_world_normals() {
        let geometry = MeshGeometry {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            triangles: vec![[0, 1, 2]],
            ..MeshGeometry::default()
        };
        let mut draw = MeshDraw {
            order: 0,
            name: Arc::from("mesh"),
            geometry: Arc::new(geometry),
            positions: None,
            // Turned upside down: the world normals point down.
            rotation: Mat3::from_rows([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]]),
            translation: Vec3::ZERO,
            scale: 1.0,
            material: MaterialColors { diffuse: [0.5, 0.25, 1.0], emissive: [0.1; 3], ..DEFAULT_MATERIAL_COLORS },
            vertex_color: NifVertexColor::default(),
            apply_mode: TextureApplyMode::Modulate,
            textures: Vec::new(),
            blend: None,
            alpha_test: None,
            depth: DepthState { test: true, write: true },
            depth_function: CompareFunction::LessEqual,
            cull: CullMode::None,
            lights: Vec::new(),
            env_map: None,
            sort: SortKey::unsorted(Vec3::ZERO),
        };
        let colors = |draw: &MeshDraw| {
            let mut vertices = Vec::new();
            expand_mesh_draw(draw, &SceneLighting::default(), &mut vertices);
            vertices.iter().map(|vertex| vertex.color).collect::<Vec<_>>()
        };
        let unlit = [0.1, 0.1, 0.1, 1.0];
        assert!(colors(&draw).iter().all(|&color| close(color, unlit)));
        let light = |direction| EffectLight {
            kind: EffectLightKind::Directional { direction },
            ambient: [0.0; 3],
            diffuse: [1.0; 3],
        };
        // A light shining up reaches the downward normals: emissive + diffuse, unclamped (the target clamps).
        draw.lights = vec![light(Vec3::Z)];
        assert!(colors(&draw).iter().all(|&color| close(color, [0.6, 0.35, 1.1, 1.0])));
        // One shining down does not.
        draw.lights = vec![light(-Vec3::Z)];
        assert!(colors(&draw).iter().all(|&color| close(color, unlit)));
    }
}
