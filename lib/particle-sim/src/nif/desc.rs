//! The compiled, engine-neutral description of a Gamebryo NIF effect (`UOParticleEffectNif`).
//!
//! [`NifEffectDesc`] keeps what the client's runtime reads from a `.nif` scene, as plain data: vectors and
//! fixed-size fields, objects referenced by index, no Gamebryo types. A converter stores it in a package and a
//! renderer loads it without parsing the NIF. Layers:
//! - [`NifNode`]: the scene graph (every `NiAVObject`), parent first, with local transforms, flags, billboard
//!   modes and what each node draws;
//! - [`NifMesh`]: triangle geometry (`NiTriShape`), with morph targets;
//! - [`NifParticleSystem`]: `NiParticleSystem` / `NiMeshParticleSystem` with its data capacity and its modifiers
//!   in update order;
//! - render state: [`NifRenderState`] per drawable; animatable material and texturing properties are shared
//!   tables ([`NifEffectDesc::materials`], [`NifEffectDesc::texturings`]) that controllers target by index;
//! - [`NifController`]: every time controller, with its timing and its interpolator's keys (key type, times,
//!   tangents and TCB parameters as stored);
//! - [`NifTexture`]: textures by name, referenced by file or embedded (`NiPixelData` decoded to RGBA8);
//! - [`NifLight`]: the scene's lights (`NiLight`), with the nodes whose effect list holds them
//!   ([`NifNode::lights`]) and the nodes they skip.
//!
//! Units and conventions are Gamebryo's: seconds, radians, row-major rotation matrices applied to column
//! vectors (`p' = T + s * R * p`), quaternions as (w, x, y, z).
//!
//! Layout changes bump [`NIF_DESC_VERSION`].

use crate::render::{AlphaTest, BlendFactor, BlendState, MeshGeometry};

/// Version of the [`NifEffectDesc`] layout.
pub const NIF_DESC_VERSION: u32 = 2;

/// Name of an embedded texture: this prefix and the index of the texture in [`NifEffectDesc::textures`].
pub const EMBEDDED_TEXTURE_PREFIX: &str = "#embedded:";

/// Gamebryo's "invalid" float of an interpolator pose (`NiFloatInterpolator`, `NiQuatTransform::INVALID_SCALE`:
/// `-FLT_MAX`). Poses equal to it are stored as `None`.
pub const INVALID_POSE_FLOAT: f32 = -f32::MAX;

/// A compiled NIF effect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NifEffectDesc {
    /// The file name, when known.
    pub name: String,
    /// Every scene object; [`Self::root`] and its descendants form the scene, the others are referenced only by
    /// modifiers (mesh-particle prototypes).
    pub nodes: Vec<NifNode>,
    /// The scene root (header root 0); `None` when the file has no `NiAVObject` root.
    pub root: Option<usize>,
    pub meshes: Vec<NifMesh>,
    pub particle_systems: Vec<NifParticleSystem>,
    pub materials: Vec<NifMaterial>,
    pub texturings: Vec<NifTexturing>,
    pub textures: Vec<NifTexture>,
    pub lights: Vec<NifLight>,
    pub controllers: Vec<NifController>,
}

impl NifEffectDesc {
    /// The largest controller stop time (`hi_key_time`), the length of the authored animation; 0 without
    /// controllers.
    pub fn animation_length(&self) -> f32 {
        self.controllers.iter().map(|controller| controller.timing.stop_time).fold(0.0, f32::max)
    }

    /// The texture of a name produced by [`NifTexture::name`].
    pub fn texture(&self, name: &str) -> Option<&NifTexture> {
        self.textures.iter().find(|texture| texture.name().eq_ignore_ascii_case(name))
    }

    /// Particle capacity summed over every system.
    pub fn particle_capacity(&self) -> usize {
        self.particle_systems.iter().map(|system| usize::from(system.capacity)).sum()
    }
}

/// A local transform: `p' = translation + scale * rotation * p`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifTransform {
    pub translation: [f32; 3],
    /// Rows of the rotation matrix.
    pub rotation: [[f32; 3]; 3],
    pub scale: f32,
}

impl NifTransform {
    pub const IDENTITY: Self =
        Self { translation: [0.0; 3], rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], scale: 1.0 };
}

impl Default for NifTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// `NiAVObject` flag bit 0, `APP_CULLED`: the object and its subtree are not drawn (set and cleared by
/// `NiVisController`).
pub const AV_FLAG_APP_CULLED: u16 = 0x1;

/// `NiBillboardNode::FaceMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BillboardMode {
    AlwaysFaceCamera,
    RotateAboutUp,
    RigidFaceCamera,
    AlwaysFaceCenter,
    RigidFaceCenter,
}

impl BillboardMode {
    /// In `FaceMode` order.
    pub const ALL: [Self; 5] =
        [Self::AlwaysFaceCamera, Self::RotateAboutUp, Self::RigidFaceCamera, Self::AlwaysFaceCenter, Self::RigidFaceCenter];

    /// The mode of a raw `BillboardFlags::face_mode` value.
    pub fn from_raw(value: u16) -> Option<Self> {
        Self::ALL.get(usize::from(value)).copied()
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum NifNodeKind {
    /// `NiNode` and subclasses without behaviour of their own here (`NiSortAdjustNode`, ...).
    Node,
    /// `NiBillboardNode`: its world rotation turns to face the camera every frame.
    Billboard(BillboardMode),
    /// `NiTriShape`: draws [`NifEffectDesc::meshes`] entry `mesh`.
    Mesh { mesh: usize },
    /// `NiParticleSystem` / `NiMeshParticleSystem`: [`NifEffectDesc::particle_systems`] entry `system`.
    ParticleSystem { system: usize },
    /// `NiLight`: [`NifEffectDesc::lights`] entry `light`; its world transform places the light.
    Light { light: usize },
    /// Anything else (texture effects, unknown geometry): kept for the hierarchy, not drawn.
    Other { type_name: String },
}

/// One scene object (`NiAVObject`).
#[derive(Debug, Clone, PartialEq)]
pub struct NifNode {
    pub name: String,
    /// Index of the parent in [`NifEffectDesc::nodes`]; parents come before their children.
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub transform: NifTransform,
    /// `NiAVObject` flags ([`AV_FLAG_APP_CULLED`]).
    pub flags: u16,
    pub kind: NifNodeKind,
    /// `NiStringExtraData` values (3ds Max user properties such as `zMode10`, `billboard`).
    pub extra_strings: Vec<String>,
    /// Whether the node is the root or a descendant of it.
    pub in_scene: bool,
    /// [`NifEffectDesc::lights`] entries of the node's effect list (`NiNode::m_kEffectList`; `NiNode::LinkObject`
    /// makes the node an affected node of each): they light this node's subtree. Texture effects are not kept.
    pub lights: Vec<usize>,
}

/// Triangle geometry with its render state.
#[derive(Debug, Clone, PartialEq)]
pub struct NifMesh {
    /// The `NiTriShape` node.
    pub node: usize,
    pub geometry: MeshGeometry,
    pub morph: Option<NifMorph>,
    pub render_state: NifRenderState,
}

/// `NiMorphData`: per-vertex target positions blended by weights (`NiGeomMorpherController`).
#[derive(Debug, Clone, PartialEq)]
pub struct NifMorph {
    /// Relative targets: target 0 is the base, always weighted 1, the others are offsets.
    pub relative_targets: bool,
    pub targets: Vec<NifMorphTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NifMorphTarget {
    pub name: String,
    /// The weight before any controller update (`legacy_weight`).
    pub initial_weight: f32,
    pub vertices: Vec<[f32; 3]>,
}

/// Static render state of a drawable, accumulated from the root down (the nearest property of each type
/// wins), plus the animatable material and texturing tables by index.
#[derive(Debug, Clone, PartialEq)]
pub struct NifRenderState {
    pub material: Option<usize>,
    pub texturing: Option<usize>,
    pub alpha: NifAlpha,
    pub z_buffer: NifZBuffer,
    pub vertex_color: NifVertexColor,
    /// `NiStencilProperty` draw mode.
    pub cull: CullMode,
    /// `NiSpecularProperty` enable (the default is off).
    pub specular: bool,
}

impl Default for NifRenderState {
    fn default() -> Self {
        Self {
            material: None,
            texturing: None,
            alpha: NifAlpha::default(),
            z_buffer: NifZBuffer::default(),
            vertex_color: NifVertexColor::default(),
            cull: CullMode::Back,
            specular: false,
        }
    }
}

/// Comparison of a test (`NiAlphaProperty::TestFunction`, `NiZBufferProperty::TestFunction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareFunction {
    Always,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Never,
}

impl CompareFunction {
    pub const ALL: [Self; 8] = [
        Self::Always,
        Self::Less,
        Self::Equal,
        Self::LessEqual,
        Self::Greater,
        Self::NotEqual,
        Self::GreaterEqual,
        Self::Never,
    ];

    pub fn from_raw(value: u16) -> Option<Self> {
        Self::ALL.get(usize::from(value)).copied()
    }

    /// Whether `value` passes against `reference`.
    pub fn passes(self, value: f32, reference: f32) -> bool {
        match self {
            Self::Always => true,
            Self::Less => value < reference,
            Self::Equal => value == reference,
            Self::LessEqual => value <= reference,
            Self::Greater => value > reference,
            Self::NotEqual => value != reference,
            Self::GreaterEqual => value >= reference,
            Self::Never => false,
        }
    }
}

/// Face culling (`NiStencilProperty::DrawMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    /// `DRAW_CCW_OR_BOTH` / `DRAW_CCW` (the default): back faces culled.
    Back,
    /// `DRAW_CW`: front faces culled.
    Front,
    /// `DRAW_BOTH`: two-sided.
    None,
}

/// `NiAlphaProperty` (defaults: blending off, SRCALPHA / INVSRCALPHA, testing off, ALWAYS, reference 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifAlpha {
    pub blend: bool,
    pub src: BlendFactor,
    pub dst: BlendFactor,
    pub test: bool,
    pub test_function: CompareFunction,
    /// Reference value 0..=255.
    pub test_reference: u8,
    /// `NoSorter` flag (draw order hint; unused here).
    pub no_sorter: bool,
}

impl Default for NifAlpha {
    fn default() -> Self {
        Self {
            blend: false,
            src: BlendFactor::SrcAlpha,
            dst: BlendFactor::InvSrcAlpha,
            test: false,
            test_function: CompareFunction::Always,
            test_reference: 0,
            no_sorter: false,
        }
    }
}

/// Largest `NiAlphaProperty` test reference.
const ALPHA_TEST_REFERENCE_MAX: f32 = 255.0;

impl NifAlpha {
    /// The blend factors when blending is on.
    pub fn blend_state(&self) -> Option<BlendState> {
        self.blend.then_some(BlendState { src: self.src, dst: self.dst })
    }

    /// The alpha test when testing is on.
    pub fn alpha_test(&self) -> Option<AlphaTest> {
        self.test.then_some(AlphaTest {
            function: self.test_function,
            reference: f32::from(self.test_reference) / ALPHA_TEST_REFERENCE_MAX,
        })
    }
}

/// `NiZBufferProperty` (default: test and write on, LESSEQUAL).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifZBuffer {
    pub test: bool,
    pub write: bool,
    pub function: CompareFunction,
}

impl Default for NifZBuffer {
    fn default() -> Self {
        Self { test: true, write: true, function: CompareFunction::LessEqual }
    }
}

/// `NiVertexColorProperty::SourceVertexMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexColorSource {
    Ignore,
    Emissive,
    AmbientDiffuse,
}

/// `NiVertexColorProperty::LightingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LightingMode {
    /// Emissive only.
    Emissive,
    /// Emissive, ambient and diffuse.
    EmissiveAmbientDiffuse,
}

/// `NiVertexColorProperty` (default: IGNORE, E_A_D).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifVertexColor {
    pub source: VertexColorSource,
    pub lighting: LightingMode,
}

impl Default for NifVertexColor {
    fn default() -> Self {
        Self { source: VertexColorSource::Ignore, lighting: LightingMode::EmissiveAmbientDiffuse }
    }
}

/// `NiMaterialProperty` (defaults: ambient and diffuse 0.5, specular and emissive 0, shine 4, alpha 1).
#[derive(Debug, Clone, PartialEq)]
pub struct NifMaterial {
    pub name: String,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
    pub shine: f32,
    pub alpha: f32,
}

impl Default for NifMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            ambient: [0.5; 3],
            diffuse: [0.5; 3],
            specular: [0.0; 3],
            emissive: [0.0; 3],
            shine: 4.0,
            alpha: 1.0,
        }
    }
}

/// `NiTexturingProperty::ApplyMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureApplyMode {
    /// The texture replaces the lit colour (lighting is off, `NiDX9LightManager::SetState`).
    Replace,
    /// `rgb = lerp(color, texture, texture.a)`, alpha from the colour.
    Decal,
    /// `texture * color` (the usual mode).
    Modulate,
}

/// A texture slot of `NiTexturingProperty`, in the property's own order (also the `map_index` of
/// `NiTextureTransformController` for 20.0.0.4 files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSlot {
    Base,
    Dark,
    Detail,
    Gloss,
    Glow,
    Bump,
    /// Decal map `n`.
    Decal(u32),
}

impl TextureSlot {
    /// The fixed slots, in map index order; decals follow.
    pub const FIXED: [Self; 6] = [Self::Base, Self::Dark, Self::Detail, Self::Gloss, Self::Glow, Self::Bump];

    /// The slot of a `NiTextureTransformController::map_index`.
    pub fn from_map_index(index: u32) -> Self {
        let fixed = Self::FIXED.len() as u32;
        if index < fixed { Self::FIXED[index as usize] } else { Self::Decal(index - fixed) }
    }
}

/// Texture coordinate addressing (`TexClampMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureAddressing {
    pub wrap_s: bool,
    pub wrap_t: bool,
}

/// `NiTextureTransform::TransformMethod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureTransformMethod {
    MayaDeprecated,
    Max,
    Maya,
}

/// `NiTextureTransform` (defaults: translate 0, rotate 0, scale 1, centre (0.5, 0.5), MAYA).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NifTextureTransform {
    pub translate: [f32; 2],
    pub scale: [f32; 2],
    pub rotate: f32,
    pub method: TextureTransformMethod,
    pub center: [f32; 2],
}

impl Default for NifTextureTransform {
    fn default() -> Self {
        Self {
            translate: [0.0; 2],
            scale: [1.0; 2],
            rotate: 0.0,
            method: TextureTransformMethod::Maya,
            center: [0.5; 2],
        }
    }
}

/// One map of a texturing property.
#[derive(Debug, Clone, PartialEq)]
pub struct NifTextureMap {
    pub slot: TextureSlot,
    /// Index in [`NifEffectDesc::textures`]; `None` when the slot names no texture.
    pub texture: Option<usize>,
    pub addressing: TextureAddressing,
    /// `TexFilterMode` raw value (0 nearest, 1 bilinear, 2 trilinear, ...).
    pub filter: u32,
    /// UV set of the geometry.
    pub uv_set: u32,
    pub transform: Option<NifTextureTransform>,
}

/// `NiTexturingProperty`.
#[derive(Debug, Clone, PartialEq)]
pub struct NifTexturing {
    pub apply_mode: TextureApplyMode,
    pub maps: Vec<NifTextureMap>,
}

impl NifTexturing {
    pub fn map(&self, slot: TextureSlot) -> Option<&NifTextureMap> {
        self.maps.iter().find(|map| map.slot == slot)
    }
}

/// Where a texture comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum NifTextureSource {
    /// A file of the client's packages, by base name as stored (`03002012_flare.tga`).
    External { file_name: String },
    /// `NiPixelData` inside the NIF, decoded to RGBA8 rows (top row first), mip level 0.
    Embedded { width: u32, height: u32, rgba: Vec<u8> },
    /// Embedded pixel data in a format not decoded here.
    Undecoded { format: u32 },
}

/// `NiSourceTexture`.
#[derive(Debug, Clone, PartialEq)]
pub struct NifTexture {
    /// Index of the texture in [`NifEffectDesc::textures`].
    pub index: usize,
    pub source: NifTextureSource,
}

impl NifTexture {
    /// The name render output uses: the file name of an external texture, otherwise
    /// [`EMBEDDED_TEXTURE_PREFIX`] and the index.
    pub fn name(&self) -> String {
        match &self.source {
            NifTextureSource::External { file_name } => file_name.clone(),
            _ => format!("{EMBEDDED_TEXTURE_PREFIX}{}", self.index),
        }
    }
}

/// A particle system (`NiParticleSystem`, `NiMeshParticleSystem`).
#[derive(Debug, Clone, PartialEq)]
pub struct NifParticleSystem {
    /// The particle system node.
    pub node: usize,
    /// Particles are simulated in world space (the system's own transform is ignored); otherwise in the
    /// system's local space.
    pub world_space: bool,
    /// Most particles alive at once: the vertex count of `NiPSysData`.
    pub capacity: u16,
    /// Which per-particle arrays `NiPSysData` has (modifiers write only those that exist).
    pub has_colors: bool,
    pub has_radii: bool,
    pub has_sizes: bool,
    pub has_rotation_angles: bool,
    pub has_rotation_axes: bool,
    /// Modifiers in update order (stable-sorted by [`NifModifier::order`]).
    pub modifiers: Vec<NifModifier>,
    pub render_state: NifRenderState,
    /// `NiMeshParticleSystem`: each particle draws a mesh subtree.
    pub mesh_particles: bool,
}

impl NifParticleSystem {
    pub fn modifier_index(&self, name: &str) -> Option<usize> {
        self.modifiers.iter().position(|modifier| modifier.name == name)
    }
}

/// `NiPSysModifier`.
#[derive(Debug, Clone, PartialEq)]
pub struct NifModifier {
    pub name: String,
    /// `NiPSysModifier::m_uiOrder` (`ORDER_EMITTER` = 1000, ...).
    pub order: u32,
    pub active: bool,
    pub kind: NifModifierKind,
}

/// The modifiers the client uses, with their stored parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum NifModifierKind {
    Emitter(NifEmitter),
    AgeDeath {
        spawn_on_death: bool,
        /// The spawn modifier: (particle system index, modifier index).
        spawn_modifier: Option<(usize, usize)>,
    },
    Spawn(NifSpawn),
    Position,
    BoundUpdate,
    GrowFade {
        grow_time: f32,
        grow_generation: u16,
        fade_time: f32,
        fade_generation: u16,
    },
    Color {
        /// `NiColorData` keys; `None` when the modifier has no data.
        colors: Option<Curve<[f32; 4]>>,
    },
    Rotation {
        initial_speed: f32,
        initial_speed_var: f32,
        initial_angle: f32,
        initial_angle_var: f32,
        random_speed_sign: bool,
        random_initial_axis: bool,
        initial_axis: [f32; 3],
    },
    Gravity {
        /// The node whose transform places the field; the modifier does nothing without it.
        object: Option<usize>,
        axis: [f32; 3],
        decay: f32,
        strength: f32,
        /// `ForceType::FORCE_SPHERICAL`; planar otherwise.
        spherical: bool,
        turbulence: f32,
        turbulence_scale: f32,
    },
    Drag {
        object: Option<usize>,
        axis: [f32; 3],
        percentage: f32,
        range: f32,
        falloff: f32,
    },
    Bomb {
        object: Option<usize>,
        axis: [f32; 3],
        decay: f32,
        delta_v: f32,
        decay_type: BombDecay,
        symmetry: BombSymmetry,
    },
    MeshUpdate {
        /// Mesh prototype nodes, chosen by particle generation.
        meshes: Vec<usize>,
    },
    /// A modifier this simulator does not run.
    Unsupported { type_name: String },
}

/// `NiPSysBombModifier::DecayType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BombDecay {
    None,
    Linear,
    Exponential,
}

/// `NiPSysBombModifier::SymmType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BombSymmetry {
    Spherical,
    Cylindrical,
    Planar,
}

/// `NiPSysEmitter` parameters and shape.
#[derive(Debug, Clone, PartialEq)]
pub struct NifEmitter {
    pub speed: f32,
    pub speed_var: f32,
    pub declination: f32,
    pub declination_var: f32,
    pub planar_angle: f32,
    pub planar_angle_var: f32,
    pub initial_color: [f32; 4],
    pub initial_radius: f32,
    pub radius_var: f32,
    pub life_span: f32,
    pub life_span_var: f32,
    pub shape: EmitterShape,
}

/// Where particles start.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitterShape {
    /// `NiPSysBoxEmitter`, centred on `object`.
    Box { object: Option<usize>, width: f32, height: f32, depth: f32 },
    /// `NiPSysSphereEmitter`.
    Sphere { object: Option<usize>, radius: f32 },
    /// `NiPSysCylinderEmitter` (axis along local z).
    Cylinder { object: Option<usize>, radius: f32, height: f32 },
    /// `NiPSysMeshEmitter`: emits from the vertices, faces or edges of meshes.
    Mesh {
        /// [`NifEffectDesc::meshes`] indices.
        meshes: Vec<usize>,
        velocity: MeshEmitterVelocity,
        emission: MeshEmission,
        emit_axis: [f32; 3],
    },
}

/// `NiPSysMeshEmitter::InitialVelocityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshEmitterVelocity {
    Normals,
    Random,
    Direction,
}

/// `NiPSysMeshEmitter::MeshEmissionType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshEmission {
    Vertices,
    FaceCenter,
    EdgeCenter,
    FaceSurface,
    EdgeSurface,
}

/// `NiPSysSpawnModifier`.
#[derive(Debug, Clone, PartialEq)]
pub struct NifSpawn {
    pub num_spawn_generations: u16,
    pub percentage_spawned: f32,
    pub min_num_to_spawn: u16,
    pub max_num_to_spawn: u16,
    pub spawn_speed_chaos: f32,
    pub spawn_dir_chaos: f32,
    pub life_span: f32,
    pub life_span_var: f32,
}

/// Gamebryo key types (`NiAnimationKey::KeyType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// `LINKEY`.
    Linear,
    /// `BEZKEY` ("quadratic" in the exporters; Hermite with stored tangents).
    Bezier,
    /// `TCBKEY`: Kochanek-Bartels.
    Tcb,
    /// `STEPKEY`: holds each value until the next key.
    Step,
}

/// Tension, continuity, bias of a TCB key.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tcb {
    pub tension: f32,
    pub continuity: f32,
    pub bias: f32,
}

/// One key: tangents are used by [`KeyKind::Bezier`], `tcb` by [`KeyKind::Tcb`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key<T> {
    pub time: f32,
    pub value: T,
    pub in_tangent: T,
    pub out_tangent: T,
    pub tcb: Tcb,
}

/// A keyed curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve<T> {
    pub kind: KeyKind,
    /// Sorted by time.
    pub keys: Vec<Key<T>>,
}

/// One rotation key (quaternion w, x, y, z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotKey {
    pub time: f32,
    pub value: [f32; 4],
    pub tcb: Tcb,
}

/// Rotation keys of `NiTransformData`.
#[derive(Debug, Clone, PartialEq)]
pub enum RotationCurve {
    Quaternion { kind: KeyKind, keys: Vec<RotKey> },
    /// `EULERKEY`: one float curve per axis (x, y, z), radians, composed with `FromAngleAxesXYZ`.
    Euler { axes: [Option<Curve<f32>>; 3] },
}

/// A boolean key (`NiBoolData`: always stepped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoolKey {
    pub time: f32,
    pub value: bool,
}

/// `NiFloatInterpolator`: keys, else the pose.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FloatInterp {
    pub pose: Option<f32>,
    pub keys: Option<Curve<f32>>,
}

/// `NiPoint3Interpolator`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Point3Interp {
    pub pose: Option<[f32; 3]>,
    pub keys: Option<Curve<[f32; 3]>>,
}

/// `NiBoolInterpolator` (`NiBoolTimelineInterpolator` behaves the same here).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoolInterp {
    pub pose: Option<bool>,
    pub keys: Vec<BoolKey>,
}

/// `NiTransformInterpolator`: each channel from its keys, else its pose, else the target keeps its value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransformInterp {
    pub translation_pose: Option<[f32; 3]>,
    pub rotation_pose: Option<[f32; 4]>,
    pub scale_pose: Option<f32>,
    pub translation: Option<Curve<[f32; 3]>>,
    pub rotation: Option<RotationCurve>,
    pub scale: Option<Curve<f32>>,
}

/// `NiPathInterpolator`: a position along a path curve, by a percentage curve.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathInterp {
    pub path: Option<Curve<[f32; 3]>>,
    pub percent: Option<Curve<f32>>,
    /// `NiPathInterpolator` flags (follow, bank, constant velocity, ...).
    pub flags: u16,
    pub follow_axis: i16,
}

/// An interpolator of a single-interpolator controller.
#[derive(Debug, Clone, PartialEq)]
pub enum NifInterpolator {
    Float(FloatInterp),
    Point3(Point3Interp),
    Bool(BoolInterp),
    Transform(TransformInterp),
    Path(PathInterp),
}

/// `NiTimeController::CycleType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CycleType {
    Loop,
    Reverse,
    Clamp,
}

/// `NiTimeController` timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControllerTiming {
    pub cycle: CycleType,
    pub active: bool,
    /// `APP_INIT`: time counts from the controller's first update (otherwise `APP_TIME`, absolute).
    pub app_init: bool,
    /// Reversed playback (`DIRECTION` flag).
    pub reverse_direction: bool,
    pub frequency: f32,
    pub phase: f32,
    pub start_time: f32,
    pub stop_time: f32,
}

/// `NiPointLight` / `NiSpotLight` distance attenuation: `1 / max(1, constant + linear d + quadratic d²)`
/// (`NiStandardMaterial` light fragment); the SDK constructor sets constant 0, linear 1, quadratic 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightAttenuation {
    pub constant: f32,
    pub linear: f32,
    pub quadratic: f32,
}

impl Default for LightAttenuation {
    fn default() -> Self {
        Self { constant: 0.0, linear: 1.0, quadratic: 0.0 }
    }
}

/// The type of a light and its own fields.
///
/// Directions are those of the light's world rotation column 0 (model +X): `NiDirectionalLight::UpdateWorldData`
/// and `NiSpotLight::UpdateWorldData` (`m_kWorld.m_Rotate.GetCol(0, m_kWorldDir)`); light travels along it.
#[derive(Debug, Clone, PartialEq)]
pub enum NifLightKind {
    /// `NiAmbientLight`: only its ambient colour counts (`SCM_DEF_AMBIENTLIGHT`).
    Ambient,
    /// `NiDirectionalLight`.
    Directional,
    /// `NiPointLight`, at the node's world position.
    Point { attenuation: LightAttenuation },
    /// `NiSpotLight`: a point light lit inside a cone around its direction. `outer_angle` is the half angle in
    /// degrees (`m_fOuterSpotAngle`); NIF 20.0.0.4 stores no inner angle (streamed since 20.2.0.5), so it is 0.
    Spot { attenuation: LightAttenuation, outer_angle: f32, exponent: f32 },
}

/// One light (`NiLight`). Colours are multiplied by the dimmer when applied (`NiDX9LightManager`,
/// `SCM_OBJ_AMBIENT` / `DIFFUSE` / `SPECULAR`).
#[derive(Debug, Clone, PartialEq)]
pub struct NifLight {
    /// The light's node in [`NifEffectDesc::nodes`].
    pub node: usize,
    pub kind: NifLightKind,
    /// `NiDynamicEffect` switch (`m_bOn`).
    pub on: bool,
    pub dimmer: f32,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    /// Nodes of `m_kUnaffectedNodeList`: `NiNode::PushLocalEffects` drops the light at them, for their subtree.
    pub unaffected_nodes: Vec<usize>,
}

/// What a controller drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerTarget {
    Node(usize),
    /// A light by [`NifEffectDesc::lights`] index (its colour and dimmer controllers; transform and visibility
    /// controllers of a light target its node).
    Light(usize),
    Material(usize),
    Texturing(usize),
    /// A particle system by [`NifEffectDesc::particle_systems`] index.
    ParticleSystem(usize),
}

/// `NiMaterialColorController` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialColorField {
    Ambient,
    Diffuse,
    Specular,
    Emissive,
}

/// `NiTextureTransformController::TransformMember`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureTransformMember {
    TranslateU,
    TranslateV,
    Rotate,
    ScaleU,
    ScaleV,
}

/// Emitter and modifier parameters that float controllers set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierParam {
    /// `NiPSysEmitterSpeedCtlr`.
    EmitterSpeed,
    /// `NiPSysEmitterLifeSpanCtlr`.
    EmitterLifeSpan,
    /// `NiPSysEmitterInitialRadiusCtlr`.
    EmitterInitialRadius,
    /// `NiPSysGravityStrengthCtlr`.
    GravityStrength,
    /// `NiPSysInitialRotSpeedCtlr`.
    InitialRotationSpeed,
    /// `NiPSysInitialRotSpeedVarCtlr`.
    InitialRotationSpeedVar,
}

/// The controllers the client uses.
#[derive(Debug, Clone, PartialEq)]
pub enum NifControllerKind {
    /// `NiTransformController` (transform or path interpolator) on a node.
    Transform(NifInterpolator),
    /// `NiVisController`: `APP_CULLED` is the negated value.
    Visibility(BoolInterp),
    /// `NiAlphaController` on a material.
    Alpha(FloatInterp),
    /// `NiMaterialColorController`.
    MaterialColor { field: MaterialColorField, interp: Point3Interp },
    /// `NiLightColorController::UpdateValue`: the value, clamped to [0, 1], becomes the ambient colour (with
    /// `ambient`; diffuse and specular turn black) or the diffuse and specular colours (ambient turns black).
    LightColor { ambient: bool, interp: Point3Interp },
    /// `NiLightDimmerController`: the light's dimmer.
    LightDimmer(FloatInterp),
    /// `NiTextureTransformController`.
    TextureTransform { slot: TextureSlot, member: TextureTransformMember, interp: FloatInterp },
    /// `NiGeomMorpherController`: one weight interpolator per target of mesh `mesh`.
    Morph { mesh: usize, weights: Vec<Option<FloatInterp>> },
    /// `NiPSysEmitterCtlr`: birth rate and emitter-active curves of emitter `modifier`.
    Emitter { modifier: Option<usize>, birth_rate: Option<FloatInterp>, active: Option<BoolInterp> },
    /// A float parameter controller (`NiPSysEmitterSpeedCtlr`, ...).
    ModifierFloat { modifier: Option<usize>, param: ModifierParam, interp: Option<FloatInterp> },
    /// `NiPSysModifierActiveCtlr`.
    ModifierActive { modifier: Option<usize>, interp: Option<BoolInterp> },
    /// `NiPSysUpdateCtlr`: advances the particle system.
    ParticleUpdate,
    /// `NiPSysResetOnLoopCtlr`: clears the particles when its time loops.
    ResetOnLoop,
    /// A controller this simulator does not run.
    Unsupported { type_name: String },
}

/// One time controller.
///
/// [`NifEffectDesc::controllers`] lists them in update order: grouped by [`Self::owner`], each owner's own
/// controllers in list order, then the controllers of its properties in property order
/// (`NiAVObject::UpdateObjectControllers`, `UpdatePropertyControllers`).
#[derive(Debug, Clone, PartialEq)]
pub struct NifController {
    /// The scene node whose update runs the controller (the target node, the first scene node holding the
    /// target property, the particle system node); `None` when no scene node does (the controller never runs).
    pub owner: Option<usize>,
    pub target: ControllerTarget,
    pub timing: ControllerTiming,
    pub kind: NifControllerKind,
}
