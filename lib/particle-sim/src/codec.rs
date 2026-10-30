//! Binary codec of the compiled descriptions: [`ChaosSystemDesc`] and [`NifEffectDesc`] to bytes and back.
//!
//! A package (`effects_ec.uddp`) stores one blob per system; the runtime decodes it and hands the owned desc to
//! [`crate::ChaosSystem::prepare`] / [`crate::NifEffect::prepare`]. Standard library only, no feature needed.
//!
//! Blob: a 12-byte header, then the body.
//!
//! ```text
//! [u8; 4] magic           CHAOS_BLOB_MAGIC ("PSCH") or NIF_BLOB_MAGIC ("PSNF")
//! u32     desc_version    CHAOS_DESC_VERSION / NIF_DESC_VERSION of the producer
//! u32     codec_version   DESC_CODEC_VERSION of the producer
//! ```
//!
//! Body rules (little-endian):
//! - `u8 u16 u32 i16 i32 f32` as is, f32 bit-exact; `bool` is a u8 0 or 1 (anything else is rejected);
//! - `String`: u32 byte length, UTF-8 bytes; `Vec<T>`: u32 count, items; `[T; N]`: N items, rows in order;
//! - an index (`usize`) is a u32; an optional index is a u32 with [`NONE_INDEX`] for `None`; another
//!   `Option<T>` is a u8 tag (0 `None`, 1 `Some`) and `T`;
//! - an enum is a u8 tag and its variant's fields; the tags are the tables of this module, never Rust
//!   declaration order (the `#[repr(u8)]` enums `BlendFactor`, `ChaosParticleType`, `ChaosEventKind` keep their
//!   client values); `TextureSlot` is its u32 map index (Base 0 .. Bump 5, `Decal(n)` 6 + n);
//! - struct fields in declaration order;
//! - a key ([`Key`], [`RotKey`]) stores only what its curve's [`KeyKind`] interpolation reads: time and value,
//!   plus the tangents for `Bezier` and the TCB parameters for `Tcb`. The others decode as zero, which is what the
//!   NIF compiler leaves in them (Gamebryo stores them only for those key kinds), so `decode(encode(d)) == d` for
//!   every compiled desc.
//!
//! The decoder rejects another magic or version, truncated data, trailing bytes, unknown tags and every index
//! out of range (the checks of [`validate_nif_desc`]), so a corrupt blob fails here instead of panicking the
//! simulation. Encoders destructure every struct without `..`: adding a desc field breaks this module's build
//! until the field is encoded (and the desc version bumped).

use std::fmt;

use crate::chaos::desc::{
    CHAOS_DESC_VERSION, ChaosCelSetup, ChaosEventDesc, ChaosEventKind, ChaosEventValue, ChaosParticleType,
    ChaosSequenceDesc, ChaosSequenceFlags, ChaosSystemDesc, IntRange, Range, VecRange,
};
use crate::nif::desc::{
    BillboardMode, BombDecay, BombSymmetry, BoolInterp, BoolKey, CompareFunction, ControllerTarget, ControllerTiming,
    CullMode, Curve, CycleType, EmitterShape, FloatInterp, Key, KeyKind, LightAttenuation, LightingMode,
    MaterialColorField, MeshEmission, MeshEmitterVelocity, ModifierParam, NIF_DESC_VERSION, NifAlpha, NifController,
    NifControllerKind, NifEffectDesc, NifEmitter, NifInterpolator, NifLight, NifLightKind, NifMaterial, NifMesh,
    NifModifier, NifModifierKind, NifMorph,
    NifMorphTarget, NifNode, NifNodeKind, NifParticleSystem, NifRenderState, NifSpawn, NifTexture, NifTextureEffect,
    NifTextureMap, NifTextureSource, NifTextureTransform, NifTexturing, NifTransform, NifVertexColor, NifZBuffer,
    PathInterp, Point3Interp, RotKey, RotationCurve, Tcb, TextureAddressing, TextureApplyMode, TextureCoordGen,
    TextureEffectKind, TextureSlot, SortingMode, embedded_level_size,
    TextureTransformMember, TextureTransformMethod, TransformInterp, VertexColorSource,
};
use crate::render::{BlendFactor, MeshGeometry};

/// Version of the body rules and tag tables of this module (3: texture effect node kind and table, embedded
/// texture levels, bound update skip).
pub const DESC_CODEC_VERSION: u32 = 3;
/// Magic of a [`ChaosSystemDesc`] blob.
pub const CHAOS_BLOB_MAGIC: [u8; 4] = *b"PSCH";
/// Magic of a [`NifEffectDesc`] blob.
pub const NIF_BLOB_MAGIC: [u8; 4] = *b"PSNF";
/// Bytes of the blob header: magic, desc version, codec version.
pub const BLOB_HEADER_SIZE: usize = 12;
/// The stored value of a `None` optional index.
pub const NONE_INDEX: u32 = u32::MAX;

/// `Option<T>` tag: no value.
const OPTION_NONE: u8 = 0;
/// `Option<T>` tag: a value follows.
const OPTION_SOME: u8 = 1;
/// Stored `false`.
const BOOL_FALSE: u8 = 0;
/// Stored `true`.
const BOOL_TRUE: u8 = 1;

/// `NifNodeKind` tags.
const NODE_KIND_NODE: u8 = 0;
const NODE_KIND_BILLBOARD: u8 = 1;
const NODE_KIND_MESH: u8 = 2;
const NODE_KIND_PARTICLE_SYSTEM: u8 = 3;
const NODE_KIND_OTHER: u8 = 4;
const NODE_KIND_LIGHT: u8 = 5;
const NODE_KIND_TEXTURE_EFFECT: u8 = 6;

/// `NifLightKind` tags.
const LIGHT_AMBIENT: u8 = 0;
const LIGHT_DIRECTIONAL: u8 = 1;
const LIGHT_POINT: u8 = 2;
const LIGHT_SPOT: u8 = 3;

/// `NifTextureSource` tags.
const TEXTURE_SOURCE_EXTERNAL: u8 = 0;
const TEXTURE_SOURCE_EMBEDDED: u8 = 1;
const TEXTURE_SOURCE_UNDECODED: u8 = 2;

/// `NifModifierKind` tags.
const MODIFIER_EMITTER: u8 = 0;
const MODIFIER_AGE_DEATH: u8 = 1;
const MODIFIER_SPAWN: u8 = 2;
const MODIFIER_POSITION: u8 = 3;
const MODIFIER_BOUND_UPDATE: u8 = 4;
const MODIFIER_GROW_FADE: u8 = 5;
const MODIFIER_COLOR: u8 = 6;
const MODIFIER_ROTATION: u8 = 7;
const MODIFIER_GRAVITY: u8 = 8;
const MODIFIER_DRAG: u8 = 9;
const MODIFIER_BOMB: u8 = 10;
const MODIFIER_MESH_UPDATE: u8 = 11;
const MODIFIER_UNSUPPORTED: u8 = 12;

/// `EmitterShape` tags.
const SHAPE_BOX: u8 = 0;
const SHAPE_SPHERE: u8 = 1;
const SHAPE_CYLINDER: u8 = 2;
const SHAPE_MESH: u8 = 3;

/// `RotationCurve` tags.
const ROTATION_QUATERNION: u8 = 0;
const ROTATION_EULER: u8 = 1;

/// `NifInterpolator` tags.
const INTERP_FLOAT: u8 = 0;
const INTERP_POINT3: u8 = 1;
const INTERP_BOOL: u8 = 2;
const INTERP_TRANSFORM: u8 = 3;
const INTERP_PATH: u8 = 4;

/// `ControllerTarget` tags.
const TARGET_NODE: u8 = 0;
const TARGET_MATERIAL: u8 = 1;
const TARGET_TEXTURING: u8 = 2;
const TARGET_PARTICLE_SYSTEM: u8 = 3;
const TARGET_LIGHT: u8 = 4;

/// `NifControllerKind` tags.
const CONTROLLER_TRANSFORM: u8 = 0;
const CONTROLLER_VISIBILITY: u8 = 1;
const CONTROLLER_ALPHA: u8 = 2;
const CONTROLLER_MATERIAL_COLOR: u8 = 3;
const CONTROLLER_TEXTURE_TRANSFORM: u8 = 4;
const CONTROLLER_MORPH: u8 = 5;
const CONTROLLER_EMITTER: u8 = 6;
const CONTROLLER_MODIFIER_FLOAT: u8 = 7;
const CONTROLLER_MODIFIER_ACTIVE: u8 = 8;
const CONTROLLER_PARTICLE_UPDATE: u8 = 9;
const CONTROLLER_RESET_ON_LOOP: u8 = 10;
const CONTROLLER_UNSUPPORTED: u8 = 11;
const CONTROLLER_LIGHT_COLOR: u8 = 12;
const CONTROLLER_LIGHT_DIMMER: u8 = 13;

/// Why a blob does not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescDecodeError {
    /// The blob does not start with the expected magic.
    BadMagic { expected: [u8; 4], found: [u8; 4] },
    /// The desc version of the blob is not this build's.
    DescVersion { expected: u32, found: u32 },
    /// The codec version of the blob is not this build's.
    CodecVersion { expected: u32, found: u32 },
    /// The data ends inside a value.
    UnexpectedEnd { needed: usize, left: usize },
    /// Bytes are left after the body.
    TrailingBytes(usize),
    /// A count is larger than the bytes left could hold.
    CountTooLarge { what: &'static str, count: u32, left: usize },
    /// An enum, option or bool tag has no meaning.
    InvalidTag { what: &'static str, tag: u32 },
    /// A string is not UTF-8.
    InvalidUtf8,
    /// An index does not point into its table.
    IndexOutOfRange { what: &'static str, index: usize, len: usize },
    /// A structural rule of the desc is broken.
    Invalid(String),
}

impl fmt::Display for DescDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic { expected, found } => write!(
                f,
                "bad blob magic {:?}, expected {:?}",
                String::from_utf8_lossy(found),
                String::from_utf8_lossy(expected)
            ),
            Self::DescVersion { expected, found } => write!(f, "desc version {found}, this build reads {expected}"),
            Self::CodecVersion { expected, found } => write!(f, "codec version {found}, this build reads {expected}"),
            Self::UnexpectedEnd { needed, left } => write!(f, "truncated blob: {needed} bytes needed, {left} left"),
            Self::TrailingBytes(count) => write!(f, "{count} trailing bytes after the desc"),
            Self::CountTooLarge { what, count, left } => {
                write!(f, "{what}: count {count} does not fit the {left} bytes left")
            }
            Self::InvalidTag { what, tag } => write!(f, "{what}: invalid tag {tag}"),
            Self::InvalidUtf8 => write!(f, "string is not UTF-8"),
            Self::IndexOutOfRange { what, index, len } => write!(f, "{what}: index {index} out of range (len {len})"),
            Self::Invalid(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for DescDecodeError {}

type DecodeResult<T> = Result<T, DescDecodeError>;

// region: --- Public API

/// Encodes a Chaos system.
pub fn encode_chaos_desc(desc: &ChaosSystemDesc) -> Vec<u8> {
    let mut writer = Writer::with_header(CHAOS_BLOB_MAGIC, CHAOS_DESC_VERSION);
    desc.encode(&mut writer);
    writer.bytes
}

/// Decodes a blob of [`encode_chaos_desc`].
pub fn decode_chaos_desc(bytes: &[u8]) -> DecodeResult<ChaosSystemDesc> {
    let mut reader = Reader::with_header(bytes, CHAOS_BLOB_MAGIC, CHAOS_DESC_VERSION)?;
    let desc = ChaosSystemDesc::decode(&mut reader)?;
    reader.finish()?;
    if desc.version != CHAOS_DESC_VERSION {
        return Err(DescDecodeError::DescVersion { expected: CHAOS_DESC_VERSION, found: desc.version });
    }
    Ok(desc)
}

/// Encodes a NIF effect.
pub fn encode_nif_desc(desc: &NifEffectDesc) -> Vec<u8> {
    let mut writer = Writer::with_header(NIF_BLOB_MAGIC, NIF_DESC_VERSION);
    desc.encode(&mut writer);
    writer.bytes
}

/// Decodes a blob of [`encode_nif_desc`] and checks it with [`validate_nif_desc`].
pub fn decode_nif_desc(bytes: &[u8]) -> DecodeResult<NifEffectDesc> {
    let mut reader = Reader::with_header(bytes, NIF_BLOB_MAGIC, NIF_DESC_VERSION)?;
    let desc = NifEffectDesc::decode(&mut reader)?;
    reader.finish()?;
    validate_nif_desc(&desc)?;
    Ok(desc)
}

/// Checks every index of a NIF desc against its table, and the geometry against its vertex count.
pub fn validate_nif_desc(desc: &NifEffectDesc) -> DecodeResult<()> {
    let nodes = desc.nodes.len();
    let meshes = desc.meshes.len();
    let systems = desc.particle_systems.len();
    let materials = desc.materials.len();
    let texturings = desc.texturings.len();
    let textures = desc.textures.len();
    let lights = desc.lights.len();
    let texture_effects = desc.texture_effects.len();
    if let Some(root) = desc.root {
        check_index("scene root", root, nodes)?;
    }
    for node in &desc.nodes {
        if let Some(parent) = node.parent {
            check_index("node parent", parent, nodes)?;
        }
        for &child in &node.children {
            check_index("node child", child, nodes)?;
        }
        for &light in &node.lights {
            check_index("node light", light, lights)?;
        }
        for &effect in &node.texture_effects {
            check_index("node texture effect", effect, texture_effects)?;
        }
        match node.kind {
            NifNodeKind::Mesh { mesh } => check_index("node mesh", mesh, meshes)?,
            NifNodeKind::ParticleSystem { system } => check_index("node particle system", system, systems)?,
            NifNodeKind::Light { light } => check_index("node light kind", light, lights)?,
            NifNodeKind::TextureEffect { effect } => check_index("node texture effect kind", effect, texture_effects)?,
            _ => {}
        }
    }
    for light in &desc.lights {
        check_index("light node", light.node, nodes)?;
        for &node in &light.unaffected_nodes {
            check_index("light unaffected node", node, nodes)?;
        }
    }
    for mesh in &desc.meshes {
        check_index("mesh node", mesh.node, nodes)?;
        validate_geometry(&mesh.geometry)?;
        if let Some(morph) = &mesh.morph {
            for target in &morph.targets {
                check_len("morph target vertices", target.vertices.len(), mesh.geometry.positions.len())?;
            }
        }
        validate_render_state(&mesh.render_state, materials, texturings)?;
    }
    for texturing in &desc.texturings {
        for map in &texturing.maps {
            if let Some(texture) = map.texture {
                check_index("texture map texture", texture, textures)?;
            }
        }
    }
    for (index, texture) in desc.textures.iter().enumerate() {
        if texture.index != index {
            return Err(DescDecodeError::Invalid(format!("texture {index} stores index {}", texture.index)));
        }
        if let NifTextureSource::Embedded { width, height, levels } = &texture.source {
            if levels.is_empty() {
                return Err(DescDecodeError::Invalid(format!("embedded texture {index} has no level")));
            }
            for (level, rgba) in levels.iter().enumerate() {
                let (level_width, level_height) = embedded_level_size(*width, *height, level);
                let expected = u64::from(level_width) * u64::from(level_height) * RGBA_BYTES_PER_PIXEL;
                if rgba.len() as u64 != expected {
                    return Err(DescDecodeError::Invalid(format!(
                        "embedded texture {index} level {level}: {} bytes for {level_width}x{level_height}",
                        rgba.len()
                    )));
                }
            }
        }
    }
    for effect in &desc.texture_effects {
        check_index("texture effect node", effect.node, nodes)?;
        if let Some(texture) = effect.texture {
            check_index("texture effect texture", texture, textures)?;
        }
        for &node in &effect.unaffected_nodes {
            check_index("texture effect unaffected node", node, nodes)?;
        }
    }
    for system in &desc.particle_systems {
        check_index("particle system node", system.node, nodes)?;
        validate_render_state(&system.render_state, materials, texturings)?;
        for modifier in &system.modifiers {
            validate_modifier(desc, &modifier.kind)?;
        }
    }
    for controller in &desc.controllers {
        if let Some(owner) = controller.owner {
            check_index("controller owner", owner, nodes)?;
        }
        let modifiers = match controller.target {
            ControllerTarget::Node(node) => check_index("controller node", node, nodes).map(|()| 0)?,
            ControllerTarget::Light(light) => check_index("controller light", light, lights).map(|()| 0)?,
            ControllerTarget::Material(material) => check_index("controller material", material, materials).map(|()| 0)?,
            ControllerTarget::Texturing(texturing) => {
                check_index("controller texturing", texturing, texturings).map(|()| 0)?
            }
            ControllerTarget::ParticleSystem(system) => {
                check_index("controller particle system", system, systems)?;
                desc.particle_systems[system].modifiers.len()
            }
        };
        let modifier = match &controller.kind {
            NifControllerKind::Morph { mesh, .. } => {
                check_index("morph controller mesh", *mesh, meshes)?;
                None
            }
            NifControllerKind::Emitter { modifier, .. }
            | NifControllerKind::ModifierFloat { modifier, .. }
            | NifControllerKind::ModifierActive { modifier, .. } => *modifier,
            _ => None,
        };
        if let Some(modifier) = modifier {
            check_index("controller modifier", modifier, modifiers)?;
        }
    }
    Ok(())
}

// endregion: --- Public API

/// Bytes per pixel of embedded RGBA8 pixels.
const RGBA_BYTES_PER_PIXEL: u64 = 4;

fn check_index(what: &'static str, index: usize, len: usize) -> DecodeResult<()> {
    if index < len { Ok(()) } else { Err(DescDecodeError::IndexOutOfRange { what, index, len }) }
}

fn check_len(what: &'static str, len: usize, expected: usize) -> DecodeResult<()> {
    if len == expected {
        Ok(())
    } else {
        Err(DescDecodeError::Invalid(format!("{what}: {len} values for {expected} vertices")))
    }
}

fn validate_geometry(geometry: &MeshGeometry) -> DecodeResult<()> {
    let MeshGeometry { positions, normals, colors, uv_sets, triangles } = geometry;
    let vertices = positions.len();
    if !normals.is_empty() {
        check_len("mesh normals", normals.len(), vertices)?;
    }
    if !colors.is_empty() {
        check_len("mesh colours", colors.len(), vertices)?;
    }
    for uvs in uv_sets {
        check_len("mesh uv set", uvs.len(), vertices)?;
    }
    for triangle in triangles {
        for &vertex in triangle {
            check_index("triangle vertex", usize::from(vertex), vertices)?;
        }
    }
    Ok(())
}

fn validate_render_state(state: &NifRenderState, materials: usize, texturings: usize) -> DecodeResult<()> {
    if let Some(material) = state.material {
        check_index("render state material", material, materials)?;
    }
    if let Some(texturing) = state.texturing {
        check_index("render state texturing", texturing, texturings)?;
    }
    Ok(())
}

fn validate_modifier(desc: &NifEffectDesc, kind: &NifModifierKind) -> DecodeResult<()> {
    let nodes = desc.nodes.len();
    match kind {
        NifModifierKind::Emitter(emitter) => match &emitter.shape {
            EmitterShape::Box { object, .. } | EmitterShape::Sphere { object, .. } | EmitterShape::Cylinder { object, .. } => {
                if let Some(object) = object {
                    check_index("emitter object", *object, nodes)?;
                }
            }
            EmitterShape::Mesh { meshes, .. } => {
                for &mesh in meshes {
                    check_index("mesh emitter mesh", mesh, desc.meshes.len())?;
                }
            }
        },
        NifModifierKind::AgeDeath { spawn_modifier: Some((system, modifier)), .. } => {
            check_index("spawn modifier system", *system, desc.particle_systems.len())?;
            check_index("spawn modifier", *modifier, desc.particle_systems[*system].modifiers.len())?;
        }
        NifModifierKind::Gravity { object: Some(object), .. }
        | NifModifierKind::Drag { object: Some(object), .. }
        | NifModifierKind::Bomb { object: Some(object), .. } => check_index("field object", *object, nodes)?,
        NifModifierKind::MeshUpdate { meshes } => {
            for &node in meshes {
                check_index("mesh update prototype", node, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// region: --- Writer and reader

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn with_header(magic: [u8; 4], desc_version: u32) -> Self {
        let mut writer = Self { bytes: Vec::new() };
        writer.bytes.extend_from_slice(&magic);
        writer.u32(desc_version);
        writer.u32(DESC_CODEC_VERSION);
        writer
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// A length or index; descs never come near `u32::MAX` entries.
    fn len(&mut self, value: usize) {
        self.u32(u32::try_from(value).expect("desc counts and indices fit in u32"));
    }

    fn index(&mut self, value: usize) {
        let value = u32::try_from(value).ok().filter(|&value| value != NONE_INDEX);
        self.u32(value.expect("desc indices are below NONE_INDEX"));
    }

    fn opt_index(&mut self, value: Option<usize>) {
        match value {
            Some(value) => self.index(value),
            None => self.u32(NONE_INDEX),
        }
    }

    fn indices(&mut self, values: &[usize]) {
        self.len(values.len());
        for &value in values {
            self.index(value);
        }
    }

    fn tag(&mut self, tag: u8) {
        self.u8(tag);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn with_header(bytes: &'a [u8], magic: [u8; 4], desc_version: u32) -> DecodeResult<Self> {
        let mut reader = Self { bytes, pos: 0 };
        let found: [u8; 4] = reader.take(magic.len())?.try_into().expect("slice of the magic length");
        if found != magic {
            return Err(DescDecodeError::BadMagic { expected: magic, found });
        }
        let found = reader.u32()?;
        if found != desc_version {
            return Err(DescDecodeError::DescVersion { expected: desc_version, found });
        }
        let found = reader.u32()?;
        if found != DESC_CODEC_VERSION {
            return Err(DescDecodeError::CodecVersion { expected: DESC_CODEC_VERSION, found });
        }
        Ok(reader)
    }

    fn left(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, count: usize) -> DecodeResult<&'a [u8]> {
        if count > self.left() {
            return Err(DescDecodeError::UnexpectedEnd { needed: count, left: self.left() });
        }
        let slice = &self.bytes[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("slice of the array length"))
    }

    fn u8(&mut self) -> DecodeResult<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> DecodeResult<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// A count of items of at least one byte each.
    fn count(&mut self, what: &'static str) -> DecodeResult<usize> {
        let count = self.u32()?;
        let left = self.left();
        if count as usize > left {
            return Err(DescDecodeError::CountTooLarge { what, count, left });
        }
        Ok(count as usize)
    }

    fn index(&mut self, what: &'static str) -> DecodeResult<usize> {
        match self.u32()? {
            NONE_INDEX => Err(DescDecodeError::InvalidTag { what, tag: NONE_INDEX }),
            value => Ok(value as usize),
        }
    }

    fn opt_index(&mut self) -> DecodeResult<Option<usize>> {
        Ok(match self.u32()? {
            NONE_INDEX => None,
            value => Some(value as usize),
        })
    }

    fn indices(&mut self, what: &'static str) -> DecodeResult<Vec<usize>> {
        let count = self.count(what)?;
        (0..count).map(|_| self.index(what)).collect()
    }

    fn finish(&self) -> DecodeResult<()> {
        match self.left() {
            0 => Ok(()),
            left => Err(DescDecodeError::TrailingBytes(left)),
        }
    }
}

/// A value with a fixed encoding.
trait Codec: Sized {
    fn encode(&self, writer: &mut Writer);
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self>;
}

macro_rules! le_codec {
    ($($ty:ty),+) => {$(
        impl Codec for $ty {
            fn encode(&self, writer: &mut Writer) {
                writer.bytes.extend_from_slice(&self.to_le_bytes());
            }

            fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
                Ok(<$ty>::from_le_bytes(reader.array()?))
            }
        }
    )+};
}

le_codec!(u8, u16, u32, i16, i32, f32);

impl Codec for bool {
    fn encode(&self, writer: &mut Writer) {
        writer.u8(if *self { BOOL_TRUE } else { BOOL_FALSE });
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        match reader.u8()? {
            BOOL_FALSE => Ok(false),
            BOOL_TRUE => Ok(true),
            tag => Err(DescDecodeError::InvalidTag { what: "bool", tag: u32::from(tag) }),
        }
    }
}

impl Codec for String {
    fn encode(&self, writer: &mut Writer) {
        writer.len(self.len());
        writer.bytes.extend_from_slice(self.as_bytes());
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let len = reader.count("string length")?;
        let bytes = reader.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DescDecodeError::InvalidUtf8)
    }
}

impl<T: Codec + Copy + Default, const N: usize> Codec for [T; N] {
    fn encode(&self, writer: &mut Writer) {
        for item in self {
            item.encode(writer);
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let mut items = [T::default(); N];
        for item in &mut items {
            *item = T::decode(reader)?;
        }
        Ok(items)
    }
}

impl<T: Codec> Codec for Vec<T> {
    fn encode(&self, writer: &mut Writer) {
        writer.len(self.len());
        for item in self {
            item.encode(writer);
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let count = reader.count("vector length")?;
        (0..count).map(|_| T::decode(reader)).collect()
    }
}

impl<T: Codec> Codec for Option<T> {
    fn encode(&self, writer: &mut Writer) {
        match self {
            None => writer.tag(OPTION_NONE),
            Some(value) => {
                writer.tag(OPTION_SOME);
                value.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        match reader.u8()? {
            OPTION_NONE => Ok(None),
            OPTION_SOME => Ok(Some(T::decode(reader)?)),
            tag => Err(DescDecodeError::InvalidTag { what: "option", tag: u32::from(tag) }),
        }
    }
}

/// A fieldless enum stored as a u8 tag; the table lists every variant with its tag.
macro_rules! tagged_enum {
    ($ty:ident, $what:literal, { $($variant:ident => $tag:expr),+ $(,)? }) => {
        impl Codec for $ty {
            fn encode(&self, writer: &mut Writer) {
                writer.tag(match self { $($ty::$variant => $tag,)+ });
            }

            fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
                let tag = reader.u8()?;
                $(if tag == $tag {
                    return Ok($ty::$variant);
                })+
                Err(DescDecodeError::InvalidTag { what: $what, tag: u32::from(tag) })
            }
        }
    };
}

// endregion: --- Writer and reader

// region: --- Chaos

tagged_enum!(BlendFactor, "blend factor", {
    One => BlendFactor::One as u8,
    Zero => BlendFactor::Zero as u8,
    SrcColor => BlendFactor::SrcColor as u8,
    InvSrcColor => BlendFactor::InvSrcColor as u8,
    DestColor => BlendFactor::DestColor as u8,
    InvDestColor => BlendFactor::InvDestColor as u8,
    SrcAlpha => BlendFactor::SrcAlpha as u8,
    InvSrcAlpha => BlendFactor::InvSrcAlpha as u8,
    DestAlpha => BlendFactor::DestAlpha as u8,
    InvDestAlpha => BlendFactor::InvDestAlpha as u8,
    SrcAlphaSat => BlendFactor::SrcAlphaSat as u8,
});

tagged_enum!(ChaosParticleType, "Chaos particle type", {
    None => ChaosParticleType::None as u8,
    Point => ChaosParticleType::Point as u8,
    Mesh => ChaosParticleType::Mesh as u8,
    Billboard => ChaosParticleType::Billboard as u8,
    Emitter => ChaosParticleType::Emitter as u8,
    Axial => ChaosParticleType::Axial as u8,
    Light => ChaosParticleType::Light as u8,
    Test => ChaosParticleType::Test as u8,
});

tagged_enum!(ChaosEventKind, "Chaos event kind", {
    Scale => ChaosEventKind::Scale as u8,
    Color => ChaosEventKind::Color as u8,
    RedColor => ChaosEventKind::RedColor as u8,
    GreenColor => ChaosEventKind::GreenColor as u8,
    BlueColor => ChaosEventKind::BlueColor as u8,
    Alpha => ChaosEventKind::Alpha as u8,
    Velocity => ChaosEventKind::Velocity as u8,
    VelocityX => ChaosEventKind::VelocityX as u8,
    VelocityY => ChaosEventKind::VelocityY as u8,
    VelocityZ => ChaosEventKind::VelocityZ as u8,
    Thrust => ChaosEventKind::Thrust as u8,
    EventTimer => ChaosEventKind::EventTimer as u8,
    Rotation => ChaosEventKind::Rotation as u8,
    Length => ChaosEventKind::Length as u8,
});

impl Codec for Range {
    fn encode(&self, writer: &mut Writer) {
        let Range { min, max } = self;
        min.encode(writer);
        max.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Range { min: f32::decode(reader)?, max: f32::decode(reader)? })
    }
}

impl Codec for VecRange {
    fn encode(&self, writer: &mut Writer) {
        let VecRange { min, max } = self;
        min.encode(writer);
        max.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(VecRange { min: Codec::decode(reader)?, max: Codec::decode(reader)? })
    }
}

impl Codec for IntRange {
    fn encode(&self, writer: &mut Writer) {
        let IntRange { min, max } = self;
        min.encode(writer);
        max.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(IntRange { min: u32::decode(reader)?, max: u32::decode(reader)? })
    }
}

impl Codec for ChaosCelSetup {
    fn encode(&self, writer: &mut Writer) {
        let ChaosCelSetup { frames, u_step, v_step, columns, rows, fps, looping } = self;
        frames.encode(writer);
        u_step.encode(writer);
        v_step.encode(writer);
        columns.encode(writer);
        rows.encode(writer);
        fps.encode(writer);
        looping.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ChaosCelSetup {
            frames: Codec::decode(reader)?,
            u_step: Codec::decode(reader)?,
            v_step: Codec::decode(reader)?,
            columns: Codec::decode(reader)?,
            rows: Codec::decode(reader)?,
            fps: Codec::decode(reader)?,
            looping: Codec::decode(reader)?,
        })
    }
}

impl Codec for ChaosEventDesc {
    fn encode(&self, writer: &mut Writer) {
        let ChaosEventDesc { kind, fade, time, value } = self;
        let ChaosEventValue { min, max } = value;
        kind.encode(writer);
        fade.encode(writer);
        time.encode(writer);
        min.encode(writer);
        max.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ChaosEventDesc {
            kind: Codec::decode(reader)?,
            fade: Codec::decode(reader)?,
            time: Codec::decode(reader)?,
            value: ChaosEventValue { min: Codec::decode(reader)?, max: Codec::decode(reader)? },
        })
    }
}

impl Codec for ChaosSequenceDesc {
    fn encode(&self, writer: &mut Writer) {
        let ChaosSequenceDesc {
            name,
            particle_type,
            resource,
            src_blend,
            dst_blend,
            flags,
            lifetime,
            emit_rate,
            num_particles,
            loops,
            scale_multiplier,
            velocity_multiplier,
            gravity,
            inherited,
            chaos_velocity,
            chaos_location,
            burst,
            emission_boost,
            cel,
            events,
        } = self;
        name.encode(writer);
        particle_type.encode(writer);
        resource.encode(writer);
        src_blend.encode(writer);
        dst_blend.encode(writer);
        flags.0.encode(writer);
        lifetime.encode(writer);
        emit_rate.encode(writer);
        num_particles.encode(writer);
        loops.encode(writer);
        scale_multiplier.encode(writer);
        velocity_multiplier.encode(writer);
        gravity.encode(writer);
        inherited.encode(writer);
        chaos_velocity.encode(writer);
        chaos_location.encode(writer);
        burst.encode(writer);
        emission_boost.encode(writer);
        cel.encode(writer);
        events.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ChaosSequenceDesc {
            name: Codec::decode(reader)?,
            particle_type: Codec::decode(reader)?,
            resource: Codec::decode(reader)?,
            src_blend: Codec::decode(reader)?,
            dst_blend: Codec::decode(reader)?,
            flags: ChaosSequenceFlags(Codec::decode(reader)?),
            lifetime: Codec::decode(reader)?,
            emit_rate: Codec::decode(reader)?,
            num_particles: Codec::decode(reader)?,
            loops: Codec::decode(reader)?,
            scale_multiplier: Codec::decode(reader)?,
            velocity_multiplier: Codec::decode(reader)?,
            gravity: Codec::decode(reader)?,
            inherited: Codec::decode(reader)?,
            chaos_velocity: Codec::decode(reader)?,
            chaos_location: Codec::decode(reader)?,
            burst: Codec::decode(reader)?,
            emission_boost: Codec::decode(reader)?,
            cel: Codec::decode(reader)?,
            events: Codec::decode(reader)?,
        })
    }
}

impl Codec for ChaosSystemDesc {
    fn encode(&self, writer: &mut Writer) {
        let ChaosSystemDesc { version, name, script_version, min_emit_time, sequences } = self;
        version.encode(writer);
        name.encode(writer);
        script_version.encode(writer);
        min_emit_time.encode(writer);
        sequences.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ChaosSystemDesc {
            version: Codec::decode(reader)?,
            name: Codec::decode(reader)?,
            script_version: Codec::decode(reader)?,
            min_emit_time: Codec::decode(reader)?,
            sequences: Codec::decode(reader)?,
        })
    }
}

// endregion: --- Chaos

// region: --- NIF

tagged_enum!(BillboardMode, "billboard mode", {
    AlwaysFaceCamera => 0,
    RotateAboutUp => 1,
    RigidFaceCamera => 2,
    AlwaysFaceCenter => 3,
    RigidFaceCenter => 4,
});

tagged_enum!(CompareFunction, "compare function", {
    Always => 0,
    Less => 1,
    Equal => 2,
    LessEqual => 3,
    Greater => 4,
    NotEqual => 5,
    GreaterEqual => 6,
    Never => 7,
});

tagged_enum!(CullMode, "cull mode", { Back => 0, Front => 1, None => 2 });
tagged_enum!(VertexColorSource, "vertex colour source", { Ignore => 0, Emissive => 1, AmbientDiffuse => 2 });
tagged_enum!(LightingMode, "lighting mode", { Emissive => 0, EmissiveAmbientDiffuse => 1 });
tagged_enum!(TextureApplyMode, "texture apply mode", { Replace => 0, Decal => 1, Modulate => 2 });
tagged_enum!(TextureTransformMethod, "texture transform method", { MayaDeprecated => 0, Max => 1, Maya => 2 });
tagged_enum!(KeyKind, "key kind", { Linear => 0, Bezier => 1, Tcb => 2, Step => 3 });
tagged_enum!(CycleType, "cycle type", { Loop => 0, Reverse => 1, Clamp => 2 });
tagged_enum!(BombDecay, "bomb decay", { None => 0, Linear => 1, Exponential => 2 });
tagged_enum!(BombSymmetry, "bomb symmetry", { Spherical => 0, Cylindrical => 1, Planar => 2 });
tagged_enum!(MeshEmitterVelocity, "mesh emitter velocity", { Normals => 0, Random => 1, Direction => 2 });
tagged_enum!(MeshEmission, "mesh emission", {
    Vertices => 0,
    FaceCenter => 1,
    EdgeCenter => 2,
    FaceSurface => 3,
    EdgeSurface => 4,
});
tagged_enum!(MaterialColorField, "material colour field", { Ambient => 0, Diffuse => 1, Specular => 2, Emissive => 3 });
tagged_enum!(TextureTransformMember, "texture transform member", {
    TranslateU => 0,
    TranslateV => 1,
    Rotate => 2,
    ScaleU => 3,
    ScaleV => 4,
});
tagged_enum!(ModifierParam, "modifier parameter", {
    EmitterSpeed => 0,
    EmitterLifeSpan => 1,
    EmitterInitialRadius => 2,
    GravityStrength => 3,
    InitialRotationSpeed => 4,
    InitialRotationSpeedVar => 5,
});

impl Codec for TextureSlot {
    fn encode(&self, writer: &mut Writer) {
        let fixed = TextureSlot::FIXED.len() as u32;
        let index = match self {
            TextureSlot::Decal(decal) => decal.checked_add(fixed).expect("decal slot fits a u32 map index"),
            slot => TextureSlot::FIXED.iter().position(|fixed| fixed == slot).expect("fixed slot") as u32,
        };
        index.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(TextureSlot::from_map_index(u32::decode(reader)?))
    }
}

impl Codec for Tcb {
    fn encode(&self, writer: &mut Writer) {
        let Tcb { tension, continuity, bias } = self;
        tension.encode(writer);
        continuity.encode(writer);
        bias.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Tcb { tension: Codec::decode(reader)?, continuity: Codec::decode(reader)?, bias: Codec::decode(reader)? })
    }
}

/// Encodes a key with the fields its kind interpolates.
fn encode_key<T: Codec>(kind: KeyKind, key: &Key<T>, writer: &mut Writer) {
    let Key { time, value, in_tangent, out_tangent, tcb } = key;
    time.encode(writer);
    value.encode(writer);
    match kind {
        KeyKind::Bezier => {
            in_tangent.encode(writer);
            out_tangent.encode(writer);
        }
        KeyKind::Tcb => tcb.encode(writer),
        KeyKind::Linear | KeyKind::Step => {}
    }
}

fn decode_key<T: Codec + Default>(kind: KeyKind, reader: &mut Reader<'_>) -> DecodeResult<Key<T>> {
    let time = f32::decode(reader)?;
    let value = T::decode(reader)?;
    let (mut in_tangent, mut out_tangent, mut tcb) = (T::default(), T::default(), Tcb::default());
    match kind {
        KeyKind::Bezier => {
            in_tangent = T::decode(reader)?;
            out_tangent = T::decode(reader)?;
        }
        KeyKind::Tcb => tcb = Tcb::decode(reader)?,
        KeyKind::Linear | KeyKind::Step => {}
    }
    Ok(Key { time, value, in_tangent, out_tangent, tcb })
}

impl<T: Codec + Default> Codec for Curve<T> {
    fn encode(&self, writer: &mut Writer) {
        let Curve { kind, keys } = self;
        kind.encode(writer);
        writer.len(keys.len());
        for key in keys {
            encode_key(*kind, key, writer);
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let kind = KeyKind::decode(reader)?;
        let count = reader.count("curve keys")?;
        let keys = (0..count).map(|_| decode_key(kind, reader)).collect::<DecodeResult<_>>()?;
        Ok(Curve { kind, keys })
    }
}

impl Codec for BoolKey {
    fn encode(&self, writer: &mut Writer) {
        let BoolKey { time, value } = self;
        time.encode(writer);
        value.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(BoolKey { time: Codec::decode(reader)?, value: Codec::decode(reader)? })
    }
}

impl Codec for RotationCurve {
    fn encode(&self, writer: &mut Writer) {
        match self {
            RotationCurve::Quaternion { kind, keys } => {
                writer.tag(ROTATION_QUATERNION);
                kind.encode(writer);
                writer.len(keys.len());
                for key in keys {
                    let RotKey { time, value, tcb } = key;
                    time.encode(writer);
                    value.encode(writer);
                    if *kind == KeyKind::Tcb {
                        tcb.encode(writer);
                    }
                }
            }
            RotationCurve::Euler { axes } => {
                writer.tag(ROTATION_EULER);
                for axis in axes {
                    axis.encode(writer);
                }
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        match reader.u8()? {
            ROTATION_QUATERNION => {
                let kind = KeyKind::decode(reader)?;
                let count = reader.count("rotation keys")?;
                let keys = (0..count)
                    .map(|_| {
                        let time = f32::decode(reader)?;
                        let value = <[f32; 4]>::decode(reader)?;
                        let tcb = if kind == KeyKind::Tcb { Tcb::decode(reader)? } else { Tcb::default() };
                        Ok(RotKey { time, value, tcb })
                    })
                    .collect::<DecodeResult<_>>()?;
                Ok(RotationCurve::Quaternion { kind, keys })
            }
            ROTATION_EULER => Ok(RotationCurve::Euler {
                axes: [Codec::decode(reader)?, Codec::decode(reader)?, Codec::decode(reader)?],
            }),
            tag => Err(DescDecodeError::InvalidTag { what: "rotation curve", tag: u32::from(tag) }),
        }
    }
}

impl Codec for FloatInterp {
    fn encode(&self, writer: &mut Writer) {
        let FloatInterp { pose, keys } = self;
        pose.encode(writer);
        keys.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(FloatInterp { pose: Codec::decode(reader)?, keys: Codec::decode(reader)? })
    }
}

impl Codec for Point3Interp {
    fn encode(&self, writer: &mut Writer) {
        let Point3Interp { pose, keys } = self;
        pose.encode(writer);
        keys.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Point3Interp { pose: Codec::decode(reader)?, keys: Codec::decode(reader)? })
    }
}

impl Codec for BoolInterp {
    fn encode(&self, writer: &mut Writer) {
        let BoolInterp { pose, keys } = self;
        pose.encode(writer);
        keys.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(BoolInterp { pose: Codec::decode(reader)?, keys: Codec::decode(reader)? })
    }
}

impl Codec for TransformInterp {
    fn encode(&self, writer: &mut Writer) {
        let TransformInterp { translation_pose, rotation_pose, scale_pose, translation, rotation, scale } = self;
        translation_pose.encode(writer);
        rotation_pose.encode(writer);
        scale_pose.encode(writer);
        translation.encode(writer);
        rotation.encode(writer);
        scale.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(TransformInterp {
            translation_pose: Codec::decode(reader)?,
            rotation_pose: Codec::decode(reader)?,
            scale_pose: Codec::decode(reader)?,
            translation: Codec::decode(reader)?,
            rotation: Codec::decode(reader)?,
            scale: Codec::decode(reader)?,
        })
    }
}

impl Codec for PathInterp {
    fn encode(&self, writer: &mut Writer) {
        // The arc lengths and the maximum curvature are derived again on decode.
        let PathInterp { path, percent, flags, follow_axis, bank_dir, max_bank_angle, smoothing, .. } = self;
        path.encode(writer);
        percent.encode(writer);
        flags.encode(writer);
        follow_axis.encode(writer);
        bank_dir.encode(writer);
        max_bank_angle.encode(writer);
        smoothing.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(PathInterp::new(
            Codec::decode(reader)?,
            Codec::decode(reader)?,
            Codec::decode(reader)?,
            Codec::decode(reader)?,
            Codec::decode(reader)?,
            Codec::decode(reader)?,
            Codec::decode(reader)?,
        ))
    }
}

impl Codec for NifInterpolator {
    fn encode(&self, writer: &mut Writer) {
        match self {
            NifInterpolator::Float(interp) => {
                writer.tag(INTERP_FLOAT);
                interp.encode(writer);
            }
            NifInterpolator::Point3(interp) => {
                writer.tag(INTERP_POINT3);
                interp.encode(writer);
            }
            NifInterpolator::Bool(interp) => {
                writer.tag(INTERP_BOOL);
                interp.encode(writer);
            }
            NifInterpolator::Transform(interp) => {
                writer.tag(INTERP_TRANSFORM);
                interp.encode(writer);
            }
            NifInterpolator::Path(interp) => {
                writer.tag(INTERP_PATH);
                interp.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            INTERP_FLOAT => NifInterpolator::Float(Codec::decode(reader)?),
            INTERP_POINT3 => NifInterpolator::Point3(Codec::decode(reader)?),
            INTERP_BOOL => NifInterpolator::Bool(Codec::decode(reader)?),
            INTERP_TRANSFORM => NifInterpolator::Transform(Codec::decode(reader)?),
            INTERP_PATH => NifInterpolator::Path(Codec::decode(reader)?),
            tag => return Err(DescDecodeError::InvalidTag { what: "interpolator", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifTransform {
    fn encode(&self, writer: &mut Writer) {
        let NifTransform { translation, rotation, scale } = self;
        translation.encode(writer);
        rotation.encode(writer);
        scale.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifTransform {
            translation: Codec::decode(reader)?,
            rotation: Codec::decode(reader)?,
            scale: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifNodeKind {
    fn encode(&self, writer: &mut Writer) {
        match self {
            NifNodeKind::Node => writer.tag(NODE_KIND_NODE),
            NifNodeKind::Billboard(mode) => {
                writer.tag(NODE_KIND_BILLBOARD);
                mode.encode(writer);
            }
            NifNodeKind::Mesh { mesh } => {
                writer.tag(NODE_KIND_MESH);
                writer.index(*mesh);
            }
            NifNodeKind::ParticleSystem { system } => {
                writer.tag(NODE_KIND_PARTICLE_SYSTEM);
                writer.index(*system);
            }
            NifNodeKind::Light { light } => {
                writer.tag(NODE_KIND_LIGHT);
                writer.index(*light);
            }
            NifNodeKind::TextureEffect { effect } => {
                writer.tag(NODE_KIND_TEXTURE_EFFECT);
                writer.index(*effect);
            }
            NifNodeKind::Other { type_name } => {
                writer.tag(NODE_KIND_OTHER);
                type_name.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            NODE_KIND_NODE => NifNodeKind::Node,
            NODE_KIND_BILLBOARD => NifNodeKind::Billboard(Codec::decode(reader)?),
            NODE_KIND_MESH => NifNodeKind::Mesh { mesh: reader.index("node mesh")? },
            NODE_KIND_PARTICLE_SYSTEM => NifNodeKind::ParticleSystem { system: reader.index("node particle system")? },
            NODE_KIND_LIGHT => NifNodeKind::Light { light: reader.index("node light kind")? },
            NODE_KIND_TEXTURE_EFFECT => NifNodeKind::TextureEffect { effect: reader.index("node texture effect kind")? },
            NODE_KIND_OTHER => NifNodeKind::Other { type_name: Codec::decode(reader)? },
            tag => return Err(DescDecodeError::InvalidTag { what: "node kind", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifNode {
    fn encode(&self, writer: &mut Writer) {
        let NifNode {
            name,
            parent,
            children,
            transform,
            flags,
            kind,
            extra_strings,
            in_scene,
            lights,
            texture_effects,
            sort_adjust,
        } = self;
        name.encode(writer);
        writer.opt_index(*parent);
        writer.indices(children);
        transform.encode(writer);
        flags.encode(writer);
        kind.encode(writer);
        extra_strings.encode(writer);
        in_scene.encode(writer);
        writer.indices(lights);
        writer.indices(texture_effects);
        sort_adjust.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifNode {
            name: Codec::decode(reader)?,
            parent: reader.opt_index()?,
            children: reader.indices("node children")?,
            transform: Codec::decode(reader)?,
            flags: Codec::decode(reader)?,
            kind: Codec::decode(reader)?,
            extra_strings: Codec::decode(reader)?,
            in_scene: Codec::decode(reader)?,
            lights: reader.indices("node lights")?,
            texture_effects: reader.indices("node texture effects")?,
            sort_adjust: Codec::decode(reader)?,
        })
    }
}

impl Codec for LightAttenuation {
    fn encode(&self, writer: &mut Writer) {
        let LightAttenuation { constant, linear, quadratic } = self;
        constant.encode(writer);
        linear.encode(writer);
        quadratic.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(LightAttenuation {
            constant: Codec::decode(reader)?,
            linear: Codec::decode(reader)?,
            quadratic: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifLightKind {
    fn encode(&self, writer: &mut Writer) {
        match self {
            NifLightKind::Ambient => writer.tag(LIGHT_AMBIENT),
            NifLightKind::Directional => writer.tag(LIGHT_DIRECTIONAL),
            NifLightKind::Point { attenuation } => {
                writer.tag(LIGHT_POINT);
                attenuation.encode(writer);
            }
            NifLightKind::Spot { attenuation, outer_angle, exponent } => {
                writer.tag(LIGHT_SPOT);
                attenuation.encode(writer);
                outer_angle.encode(writer);
                exponent.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            LIGHT_AMBIENT => NifLightKind::Ambient,
            LIGHT_DIRECTIONAL => NifLightKind::Directional,
            LIGHT_POINT => NifLightKind::Point { attenuation: Codec::decode(reader)? },
            LIGHT_SPOT => NifLightKind::Spot {
                attenuation: Codec::decode(reader)?,
                outer_angle: Codec::decode(reader)?,
                exponent: Codec::decode(reader)?,
            },
            tag => return Err(DescDecodeError::InvalidTag { what: "light kind", tag: u32::from(tag) }),
        })
    }
}

tagged_enum!(SortingMode, "sorting mode", {
    Inherit => SortingMode::Inherit as u8,
    Off => SortingMode::Off as u8,
});

tagged_enum!(TextureEffectKind, "texture effect kind", {
    ProjectedLight => TextureEffectKind::ProjectedLight as u8,
    ProjectedShadow => TextureEffectKind::ProjectedShadow as u8,
    EnvironmentMap => TextureEffectKind::EnvironmentMap as u8,
    FogMap => TextureEffectKind::FogMap as u8,
});

tagged_enum!(TextureCoordGen, "texture coordinate generation", {
    WorldParallel => TextureCoordGen::WorldParallel as u8,
    WorldPerspective => TextureCoordGen::WorldPerspective as u8,
    SphereMap => TextureCoordGen::SphereMap as u8,
    SpecularCubeMap => TextureCoordGen::SpecularCubeMap as u8,
    DiffuseCubeMap => TextureCoordGen::DiffuseCubeMap as u8,
});

impl Codec for NifTextureEffect {
    fn encode(&self, writer: &mut Writer) {
        let NifTextureEffect {
            node,
            kind,
            coord_gen,
            on,
            texture,
            model_projection,
            model_translation,
            filter,
            addressing,
            unaffected_nodes,
        } = self;
        let TextureAddressing { wrap_s, wrap_t } = addressing;
        writer.index(*node);
        kind.encode(writer);
        coord_gen.encode(writer);
        on.encode(writer);
        writer.opt_index(*texture);
        model_projection.encode(writer);
        model_translation.encode(writer);
        filter.encode(writer);
        wrap_s.encode(writer);
        wrap_t.encode(writer);
        writer.indices(unaffected_nodes);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifTextureEffect {
            node: reader.index("texture effect node")?,
            kind: Codec::decode(reader)?,
            coord_gen: Codec::decode(reader)?,
            on: Codec::decode(reader)?,
            texture: reader.opt_index()?,
            model_projection: Codec::decode(reader)?,
            model_translation: Codec::decode(reader)?,
            filter: Codec::decode(reader)?,
            addressing: TextureAddressing { wrap_s: Codec::decode(reader)?, wrap_t: Codec::decode(reader)? },
            unaffected_nodes: reader.indices("texture effect unaffected nodes")?,
        })
    }
}

impl Codec for NifLight {
    fn encode(&self, writer: &mut Writer) {
        let NifLight { node, kind, on, dimmer, ambient, diffuse, specular, unaffected_nodes } = self;
        writer.index(*node);
        kind.encode(writer);
        on.encode(writer);
        dimmer.encode(writer);
        ambient.encode(writer);
        diffuse.encode(writer);
        specular.encode(writer);
        writer.indices(unaffected_nodes);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifLight {
            node: reader.index("light node")?,
            kind: Codec::decode(reader)?,
            on: Codec::decode(reader)?,
            dimmer: Codec::decode(reader)?,
            ambient: Codec::decode(reader)?,
            diffuse: Codec::decode(reader)?,
            specular: Codec::decode(reader)?,
            unaffected_nodes: reader.indices("light unaffected nodes")?,
        })
    }
}

impl Codec for MeshGeometry {
    fn encode(&self, writer: &mut Writer) {
        let MeshGeometry { positions, normals, colors, uv_sets, triangles } = self;
        positions.encode(writer);
        normals.encode(writer);
        colors.encode(writer);
        uv_sets.encode(writer);
        triangles.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(MeshGeometry {
            positions: Codec::decode(reader)?,
            normals: Codec::decode(reader)?,
            colors: Codec::decode(reader)?,
            uv_sets: Codec::decode(reader)?,
            triangles: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifMorphTarget {
    fn encode(&self, writer: &mut Writer) {
        let NifMorphTarget { name, initial_weight, vertices } = self;
        name.encode(writer);
        initial_weight.encode(writer);
        vertices.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifMorphTarget {
            name: Codec::decode(reader)?,
            initial_weight: Codec::decode(reader)?,
            vertices: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifMorph {
    fn encode(&self, writer: &mut Writer) {
        let NifMorph { relative_targets, targets } = self;
        relative_targets.encode(writer);
        targets.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifMorph { relative_targets: Codec::decode(reader)?, targets: Codec::decode(reader)? })
    }
}

impl Codec for NifAlpha {
    fn encode(&self, writer: &mut Writer) {
        let NifAlpha { blend, src, dst, test, test_function, test_reference, no_sorter } = self;
        blend.encode(writer);
        src.encode(writer);
        dst.encode(writer);
        test.encode(writer);
        test_function.encode(writer);
        test_reference.encode(writer);
        no_sorter.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifAlpha {
            blend: Codec::decode(reader)?,
            src: Codec::decode(reader)?,
            dst: Codec::decode(reader)?,
            test: Codec::decode(reader)?,
            test_function: Codec::decode(reader)?,
            test_reference: Codec::decode(reader)?,
            no_sorter: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifZBuffer {
    fn encode(&self, writer: &mut Writer) {
        let NifZBuffer { test, write, function } = self;
        test.encode(writer);
        write.encode(writer);
        function.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifZBuffer { test: Codec::decode(reader)?, write: Codec::decode(reader)?, function: Codec::decode(reader)? })
    }
}

impl Codec for NifVertexColor {
    fn encode(&self, writer: &mut Writer) {
        let NifVertexColor { source, lighting } = self;
        source.encode(writer);
        lighting.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifVertexColor { source: Codec::decode(reader)?, lighting: Codec::decode(reader)? })
    }
}

impl Codec for NifRenderState {
    fn encode(&self, writer: &mut Writer) {
        let NifRenderState { material, texturing, alpha, z_buffer, vertex_color, cull, specular } = self;
        writer.opt_index(*material);
        writer.opt_index(*texturing);
        alpha.encode(writer);
        z_buffer.encode(writer);
        vertex_color.encode(writer);
        cull.encode(writer);
        specular.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifRenderState {
            material: reader.opt_index()?,
            texturing: reader.opt_index()?,
            alpha: Codec::decode(reader)?,
            z_buffer: Codec::decode(reader)?,
            vertex_color: Codec::decode(reader)?,
            cull: Codec::decode(reader)?,
            specular: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifMesh {
    fn encode(&self, writer: &mut Writer) {
        let NifMesh { node, geometry, bound_center, morph, render_state } = self;
        writer.index(*node);
        geometry.encode(writer);
        bound_center.encode(writer);
        morph.encode(writer);
        render_state.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifMesh {
            node: reader.index("mesh node")?,
            geometry: Codec::decode(reader)?,
            bound_center: Codec::decode(reader)?,
            morph: Codec::decode(reader)?,
            render_state: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifMaterial {
    fn encode(&self, writer: &mut Writer) {
        let NifMaterial { name, ambient, diffuse, specular, emissive, shine, alpha } = self;
        name.encode(writer);
        ambient.encode(writer);
        diffuse.encode(writer);
        specular.encode(writer);
        emissive.encode(writer);
        shine.encode(writer);
        alpha.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifMaterial {
            name: Codec::decode(reader)?,
            ambient: Codec::decode(reader)?,
            diffuse: Codec::decode(reader)?,
            specular: Codec::decode(reader)?,
            emissive: Codec::decode(reader)?,
            shine: Codec::decode(reader)?,
            alpha: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifTextureTransform {
    fn encode(&self, writer: &mut Writer) {
        let NifTextureTransform { translate, scale, rotate, method, center } = self;
        translate.encode(writer);
        scale.encode(writer);
        rotate.encode(writer);
        method.encode(writer);
        center.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifTextureTransform {
            translate: Codec::decode(reader)?,
            scale: Codec::decode(reader)?,
            rotate: Codec::decode(reader)?,
            method: Codec::decode(reader)?,
            center: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifTextureMap {
    fn encode(&self, writer: &mut Writer) {
        let NifTextureMap { slot, texture, addressing, filter, uv_set, transform } = self;
        let TextureAddressing { wrap_s, wrap_t } = addressing;
        slot.encode(writer);
        writer.opt_index(*texture);
        wrap_s.encode(writer);
        wrap_t.encode(writer);
        filter.encode(writer);
        uv_set.encode(writer);
        transform.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifTextureMap {
            slot: Codec::decode(reader)?,
            texture: reader.opt_index()?,
            addressing: TextureAddressing { wrap_s: Codec::decode(reader)?, wrap_t: Codec::decode(reader)? },
            filter: Codec::decode(reader)?,
            uv_set: Codec::decode(reader)?,
            transform: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifTexturing {
    fn encode(&self, writer: &mut Writer) {
        let NifTexturing { apply_mode, maps } = self;
        apply_mode.encode(writer);
        maps.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifTexturing { apply_mode: Codec::decode(reader)?, maps: Codec::decode(reader)? })
    }
}

impl Codec for NifTexture {
    fn encode(&self, writer: &mut Writer) {
        let NifTexture { index, source, mipmapped } = self;
        writer.index(*index);
        mipmapped.encode(writer);
        match source {
            NifTextureSource::External { file_name } => {
                writer.tag(TEXTURE_SOURCE_EXTERNAL);
                file_name.encode(writer);
            }
            NifTextureSource::Embedded { width, height, levels } => {
                writer.tag(TEXTURE_SOURCE_EMBEDDED);
                width.encode(writer);
                height.encode(writer);
                writer.len(levels.len());
                for rgba in levels {
                    writer.len(rgba.len());
                    writer.bytes.extend_from_slice(rgba);
                }
            }
            NifTextureSource::Undecoded { format } => {
                writer.tag(TEXTURE_SOURCE_UNDECODED);
                format.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let index = reader.index("texture index")?;
        let mipmapped = Codec::decode(reader)?;
        let source = match reader.u8()? {
            TEXTURE_SOURCE_EXTERNAL => NifTextureSource::External { file_name: Codec::decode(reader)? },
            TEXTURE_SOURCE_EMBEDDED => {
                let width = u32::decode(reader)?;
                let height = u32::decode(reader)?;
                let count = reader.count("embedded levels")?;
                let mut levels = Vec::with_capacity(count);
                for _ in 0..count {
                    let len = reader.count("embedded pixels")?;
                    levels.push(reader.take(len)?.to_vec());
                }
                NifTextureSource::Embedded { width, height, levels }
            }
            TEXTURE_SOURCE_UNDECODED => NifTextureSource::Undecoded { format: Codec::decode(reader)? },
            tag => return Err(DescDecodeError::InvalidTag { what: "texture source", tag: u32::from(tag) }),
        };
        Ok(NifTexture { index, source, mipmapped })
    }
}

impl Codec for NifSpawn {
    fn encode(&self, writer: &mut Writer) {
        let NifSpawn {
            num_spawn_generations,
            percentage_spawned,
            min_num_to_spawn,
            max_num_to_spawn,
            spawn_speed_chaos,
            spawn_dir_chaos,
            life_span,
            life_span_var,
        } = self;
        num_spawn_generations.encode(writer);
        percentage_spawned.encode(writer);
        min_num_to_spawn.encode(writer);
        max_num_to_spawn.encode(writer);
        spawn_speed_chaos.encode(writer);
        spawn_dir_chaos.encode(writer);
        life_span.encode(writer);
        life_span_var.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifSpawn {
            num_spawn_generations: Codec::decode(reader)?,
            percentage_spawned: Codec::decode(reader)?,
            min_num_to_spawn: Codec::decode(reader)?,
            max_num_to_spawn: Codec::decode(reader)?,
            spawn_speed_chaos: Codec::decode(reader)?,
            spawn_dir_chaos: Codec::decode(reader)?,
            life_span: Codec::decode(reader)?,
            life_span_var: Codec::decode(reader)?,
        })
    }
}

impl Codec for EmitterShape {
    fn encode(&self, writer: &mut Writer) {
        match self {
            EmitterShape::Box { object, width, height, depth } => {
                writer.tag(SHAPE_BOX);
                writer.opt_index(*object);
                width.encode(writer);
                height.encode(writer);
                depth.encode(writer);
            }
            EmitterShape::Sphere { object, radius } => {
                writer.tag(SHAPE_SPHERE);
                writer.opt_index(*object);
                radius.encode(writer);
            }
            EmitterShape::Cylinder { object, radius, height } => {
                writer.tag(SHAPE_CYLINDER);
                writer.opt_index(*object);
                radius.encode(writer);
                height.encode(writer);
            }
            EmitterShape::Mesh { meshes, velocity, emission, emit_axis } => {
                writer.tag(SHAPE_MESH);
                writer.indices(meshes);
                velocity.encode(writer);
                emission.encode(writer);
                emit_axis.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            SHAPE_BOX => EmitterShape::Box {
                object: reader.opt_index()?,
                width: Codec::decode(reader)?,
                height: Codec::decode(reader)?,
                depth: Codec::decode(reader)?,
            },
            SHAPE_SPHERE => EmitterShape::Sphere { object: reader.opt_index()?, radius: Codec::decode(reader)? },
            SHAPE_CYLINDER => EmitterShape::Cylinder {
                object: reader.opt_index()?,
                radius: Codec::decode(reader)?,
                height: Codec::decode(reader)?,
            },
            SHAPE_MESH => EmitterShape::Mesh {
                meshes: reader.indices("mesh emitter meshes")?,
                velocity: Codec::decode(reader)?,
                emission: Codec::decode(reader)?,
                emit_axis: Codec::decode(reader)?,
            },
            tag => return Err(DescDecodeError::InvalidTag { what: "emitter shape", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifEmitter {
    fn encode(&self, writer: &mut Writer) {
        let NifEmitter {
            speed,
            speed_var,
            declination,
            declination_var,
            planar_angle,
            planar_angle_var,
            initial_color,
            initial_radius,
            radius_var,
            life_span,
            life_span_var,
            shape,
        } = self;
        speed.encode(writer);
        speed_var.encode(writer);
        declination.encode(writer);
        declination_var.encode(writer);
        planar_angle.encode(writer);
        planar_angle_var.encode(writer);
        initial_color.encode(writer);
        initial_radius.encode(writer);
        radius_var.encode(writer);
        life_span.encode(writer);
        life_span_var.encode(writer);
        shape.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifEmitter {
            speed: Codec::decode(reader)?,
            speed_var: Codec::decode(reader)?,
            declination: Codec::decode(reader)?,
            declination_var: Codec::decode(reader)?,
            planar_angle: Codec::decode(reader)?,
            planar_angle_var: Codec::decode(reader)?,
            initial_color: Codec::decode(reader)?,
            initial_radius: Codec::decode(reader)?,
            radius_var: Codec::decode(reader)?,
            life_span: Codec::decode(reader)?,
            life_span_var: Codec::decode(reader)?,
            shape: Codec::decode(reader)?,
        })
    }
}

/// An optional (particle system, modifier) pair: two u32, both [`NONE_INDEX`] for `None`.
fn encode_opt_pair(value: Option<(usize, usize)>, writer: &mut Writer) {
    match value {
        Some((system, modifier)) => {
            writer.index(system);
            writer.index(modifier);
        }
        None => {
            writer.u32(NONE_INDEX);
            writer.u32(NONE_INDEX);
        }
    }
}

fn decode_opt_pair(reader: &mut Reader<'_>) -> DecodeResult<Option<(usize, usize)>> {
    match (reader.opt_index()?, reader.opt_index()?) {
        (Some(system), Some(modifier)) => Ok(Some((system, modifier))),
        (None, None) => Ok(None),
        _ => Err(DescDecodeError::Invalid("spawn modifier pair with one NONE half".to_owned())),
    }
}

impl Codec for NifModifierKind {
    fn encode(&self, writer: &mut Writer) {
        match self {
            NifModifierKind::Emitter(emitter) => {
                writer.tag(MODIFIER_EMITTER);
                emitter.encode(writer);
            }
            NifModifierKind::AgeDeath { spawn_on_death, spawn_modifier } => {
                writer.tag(MODIFIER_AGE_DEATH);
                spawn_on_death.encode(writer);
                encode_opt_pair(*spawn_modifier, writer);
            }
            NifModifierKind::Spawn(spawn) => {
                writer.tag(MODIFIER_SPAWN);
                spawn.encode(writer);
            }
            NifModifierKind::Position => writer.tag(MODIFIER_POSITION),
            NifModifierKind::BoundUpdate { skip } => {
                writer.tag(MODIFIER_BOUND_UPDATE);
                skip.encode(writer);
            }
            NifModifierKind::GrowFade { grow_time, grow_generation, fade_time, fade_generation } => {
                writer.tag(MODIFIER_GROW_FADE);
                grow_time.encode(writer);
                grow_generation.encode(writer);
                fade_time.encode(writer);
                fade_generation.encode(writer);
            }
            NifModifierKind::Color { colors } => {
                writer.tag(MODIFIER_COLOR);
                colors.encode(writer);
            }
            NifModifierKind::Rotation {
                initial_speed,
                initial_speed_var,
                initial_angle,
                initial_angle_var,
                random_speed_sign,
                random_initial_axis,
                initial_axis,
            } => {
                writer.tag(MODIFIER_ROTATION);
                initial_speed.encode(writer);
                initial_speed_var.encode(writer);
                initial_angle.encode(writer);
                initial_angle_var.encode(writer);
                random_speed_sign.encode(writer);
                random_initial_axis.encode(writer);
                initial_axis.encode(writer);
            }
            NifModifierKind::Gravity { object, axis, decay, strength, spherical, turbulence, turbulence_scale } => {
                writer.tag(MODIFIER_GRAVITY);
                writer.opt_index(*object);
                axis.encode(writer);
                decay.encode(writer);
                strength.encode(writer);
                spherical.encode(writer);
                turbulence.encode(writer);
                turbulence_scale.encode(writer);
            }
            NifModifierKind::Drag { object, axis, percentage, range, falloff } => {
                writer.tag(MODIFIER_DRAG);
                writer.opt_index(*object);
                axis.encode(writer);
                percentage.encode(writer);
                range.encode(writer);
                falloff.encode(writer);
            }
            NifModifierKind::Bomb { object, axis, decay, delta_v, decay_type, symmetry } => {
                writer.tag(MODIFIER_BOMB);
                writer.opt_index(*object);
                axis.encode(writer);
                decay.encode(writer);
                delta_v.encode(writer);
                decay_type.encode(writer);
                symmetry.encode(writer);
            }
            NifModifierKind::MeshUpdate { meshes } => {
                writer.tag(MODIFIER_MESH_UPDATE);
                writer.indices(meshes);
            }
            NifModifierKind::Unsupported { type_name } => {
                writer.tag(MODIFIER_UNSUPPORTED);
                type_name.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            MODIFIER_EMITTER => NifModifierKind::Emitter(Codec::decode(reader)?),
            MODIFIER_AGE_DEATH => NifModifierKind::AgeDeath {
                spawn_on_death: Codec::decode(reader)?,
                spawn_modifier: decode_opt_pair(reader)?,
            },
            MODIFIER_SPAWN => NifModifierKind::Spawn(Codec::decode(reader)?),
            MODIFIER_POSITION => NifModifierKind::Position,
            MODIFIER_BOUND_UPDATE => NifModifierKind::BoundUpdate { skip: Codec::decode(reader)? },
            MODIFIER_GROW_FADE => NifModifierKind::GrowFade {
                grow_time: Codec::decode(reader)?,
                grow_generation: Codec::decode(reader)?,
                fade_time: Codec::decode(reader)?,
                fade_generation: Codec::decode(reader)?,
            },
            MODIFIER_COLOR => NifModifierKind::Color { colors: Codec::decode(reader)? },
            MODIFIER_ROTATION => NifModifierKind::Rotation {
                initial_speed: Codec::decode(reader)?,
                initial_speed_var: Codec::decode(reader)?,
                initial_angle: Codec::decode(reader)?,
                initial_angle_var: Codec::decode(reader)?,
                random_speed_sign: Codec::decode(reader)?,
                random_initial_axis: Codec::decode(reader)?,
                initial_axis: Codec::decode(reader)?,
            },
            MODIFIER_GRAVITY => NifModifierKind::Gravity {
                object: reader.opt_index()?,
                axis: Codec::decode(reader)?,
                decay: Codec::decode(reader)?,
                strength: Codec::decode(reader)?,
                spherical: Codec::decode(reader)?,
                turbulence: Codec::decode(reader)?,
                turbulence_scale: Codec::decode(reader)?,
            },
            MODIFIER_DRAG => NifModifierKind::Drag {
                object: reader.opt_index()?,
                axis: Codec::decode(reader)?,
                percentage: Codec::decode(reader)?,
                range: Codec::decode(reader)?,
                falloff: Codec::decode(reader)?,
            },
            MODIFIER_BOMB => NifModifierKind::Bomb {
                object: reader.opt_index()?,
                axis: Codec::decode(reader)?,
                decay: Codec::decode(reader)?,
                delta_v: Codec::decode(reader)?,
                decay_type: Codec::decode(reader)?,
                symmetry: Codec::decode(reader)?,
            },
            MODIFIER_MESH_UPDATE => NifModifierKind::MeshUpdate { meshes: reader.indices("mesh update prototypes")? },
            MODIFIER_UNSUPPORTED => NifModifierKind::Unsupported { type_name: Codec::decode(reader)? },
            tag => return Err(DescDecodeError::InvalidTag { what: "modifier kind", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifModifier {
    fn encode(&self, writer: &mut Writer) {
        let NifModifier { name, order, active, kind } = self;
        name.encode(writer);
        order.encode(writer);
        active.encode(writer);
        kind.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifModifier {
            name: Codec::decode(reader)?,
            order: Codec::decode(reader)?,
            active: Codec::decode(reader)?,
            kind: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifParticleSystem {
    fn encode(&self, writer: &mut Writer) {
        let NifParticleSystem {
            node,
            world_space,
            capacity,
            has_colors,
            has_radii,
            has_sizes,
            has_rotation_angles,
            has_rotation_axes,
            modifiers,
            render_state,
            mesh_particles,
            bound_center,
        } = self;
        writer.index(*node);
        world_space.encode(writer);
        capacity.encode(writer);
        has_colors.encode(writer);
        has_radii.encode(writer);
        has_sizes.encode(writer);
        has_rotation_angles.encode(writer);
        has_rotation_axes.encode(writer);
        modifiers.encode(writer);
        render_state.encode(writer);
        mesh_particles.encode(writer);
        bound_center.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifParticleSystem {
            node: reader.index("particle system node")?,
            world_space: Codec::decode(reader)?,
            capacity: Codec::decode(reader)?,
            has_colors: Codec::decode(reader)?,
            has_radii: Codec::decode(reader)?,
            has_sizes: Codec::decode(reader)?,
            has_rotation_angles: Codec::decode(reader)?,
            has_rotation_axes: Codec::decode(reader)?,
            modifiers: Codec::decode(reader)?,
            render_state: Codec::decode(reader)?,
            mesh_particles: Codec::decode(reader)?,
            bound_center: Codec::decode(reader)?,
        })
    }
}

impl Codec for ControllerTiming {
    fn encode(&self, writer: &mut Writer) {
        let ControllerTiming { cycle, active, app_init, reverse_direction, frequency, phase, start_time, stop_time } =
            self;
        cycle.encode(writer);
        active.encode(writer);
        app_init.encode(writer);
        reverse_direction.encode(writer);
        frequency.encode(writer);
        phase.encode(writer);
        start_time.encode(writer);
        stop_time.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ControllerTiming {
            cycle: Codec::decode(reader)?,
            active: Codec::decode(reader)?,
            app_init: Codec::decode(reader)?,
            reverse_direction: Codec::decode(reader)?,
            frequency: Codec::decode(reader)?,
            phase: Codec::decode(reader)?,
            start_time: Codec::decode(reader)?,
            stop_time: Codec::decode(reader)?,
        })
    }
}

impl Codec for ControllerTarget {
    fn encode(&self, writer: &mut Writer) {
        let (tag, index) = match *self {
            ControllerTarget::Node(index) => (TARGET_NODE, index),
            ControllerTarget::Material(index) => (TARGET_MATERIAL, index),
            ControllerTarget::Texturing(index) => (TARGET_TEXTURING, index),
            ControllerTarget::ParticleSystem(index) => (TARGET_PARTICLE_SYSTEM, index),
            ControllerTarget::Light(index) => (TARGET_LIGHT, index),
        };
        writer.tag(tag);
        writer.index(index);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let tag = reader.u8()?;
        let index = reader.index("controller target")?;
        Ok(match tag {
            TARGET_NODE => ControllerTarget::Node(index),
            TARGET_MATERIAL => ControllerTarget::Material(index),
            TARGET_TEXTURING => ControllerTarget::Texturing(index),
            TARGET_PARTICLE_SYSTEM => ControllerTarget::ParticleSystem(index),
            TARGET_LIGHT => ControllerTarget::Light(index),
            tag => return Err(DescDecodeError::InvalidTag { what: "controller target", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifControllerKind {
    fn encode(&self, writer: &mut Writer) {
        match self {
            NifControllerKind::Transform(interp) => {
                writer.tag(CONTROLLER_TRANSFORM);
                interp.encode(writer);
            }
            NifControllerKind::Visibility(interp) => {
                writer.tag(CONTROLLER_VISIBILITY);
                interp.encode(writer);
            }
            NifControllerKind::Alpha(interp) => {
                writer.tag(CONTROLLER_ALPHA);
                interp.encode(writer);
            }
            NifControllerKind::MaterialColor { field, interp } => {
                writer.tag(CONTROLLER_MATERIAL_COLOR);
                field.encode(writer);
                interp.encode(writer);
            }
            NifControllerKind::LightColor { ambient, interp } => {
                writer.tag(CONTROLLER_LIGHT_COLOR);
                ambient.encode(writer);
                interp.encode(writer);
            }
            NifControllerKind::LightDimmer(interp) => {
                writer.tag(CONTROLLER_LIGHT_DIMMER);
                interp.encode(writer);
            }
            NifControllerKind::TextureTransform { slot, member, interp } => {
                writer.tag(CONTROLLER_TEXTURE_TRANSFORM);
                slot.encode(writer);
                member.encode(writer);
                interp.encode(writer);
            }
            NifControllerKind::Morph { mesh, weights } => {
                writer.tag(CONTROLLER_MORPH);
                writer.index(*mesh);
                weights.encode(writer);
            }
            NifControllerKind::Emitter { modifier, birth_rate, active } => {
                writer.tag(CONTROLLER_EMITTER);
                writer.opt_index(*modifier);
                birth_rate.encode(writer);
                active.encode(writer);
            }
            NifControllerKind::ModifierFloat { modifier, param, interp } => {
                writer.tag(CONTROLLER_MODIFIER_FLOAT);
                writer.opt_index(*modifier);
                param.encode(writer);
                interp.encode(writer);
            }
            NifControllerKind::ModifierActive { modifier, interp } => {
                writer.tag(CONTROLLER_MODIFIER_ACTIVE);
                writer.opt_index(*modifier);
                interp.encode(writer);
            }
            NifControllerKind::ParticleUpdate => writer.tag(CONTROLLER_PARTICLE_UPDATE),
            NifControllerKind::ResetOnLoop => writer.tag(CONTROLLER_RESET_ON_LOOP),
            NifControllerKind::Unsupported { type_name } => {
                writer.tag(CONTROLLER_UNSUPPORTED);
                type_name.encode(writer);
            }
        }
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match reader.u8()? {
            CONTROLLER_TRANSFORM => NifControllerKind::Transform(Codec::decode(reader)?),
            CONTROLLER_VISIBILITY => NifControllerKind::Visibility(Codec::decode(reader)?),
            CONTROLLER_ALPHA => NifControllerKind::Alpha(Codec::decode(reader)?),
            CONTROLLER_MATERIAL_COLOR => {
                NifControllerKind::MaterialColor { field: Codec::decode(reader)?, interp: Codec::decode(reader)? }
            }
            CONTROLLER_LIGHT_COLOR => {
                NifControllerKind::LightColor { ambient: Codec::decode(reader)?, interp: Codec::decode(reader)? }
            }
            CONTROLLER_LIGHT_DIMMER => NifControllerKind::LightDimmer(Codec::decode(reader)?),
            CONTROLLER_TEXTURE_TRANSFORM => NifControllerKind::TextureTransform {
                slot: Codec::decode(reader)?,
                member: Codec::decode(reader)?,
                interp: Codec::decode(reader)?,
            },
            CONTROLLER_MORPH => {
                NifControllerKind::Morph { mesh: reader.index("morph mesh")?, weights: Codec::decode(reader)? }
            }
            CONTROLLER_EMITTER => NifControllerKind::Emitter {
                modifier: reader.opt_index()?,
                birth_rate: Codec::decode(reader)?,
                active: Codec::decode(reader)?,
            },
            CONTROLLER_MODIFIER_FLOAT => NifControllerKind::ModifierFloat {
                modifier: reader.opt_index()?,
                param: Codec::decode(reader)?,
                interp: Codec::decode(reader)?,
            },
            CONTROLLER_MODIFIER_ACTIVE => {
                NifControllerKind::ModifierActive { modifier: reader.opt_index()?, interp: Codec::decode(reader)? }
            }
            CONTROLLER_PARTICLE_UPDATE => NifControllerKind::ParticleUpdate,
            CONTROLLER_RESET_ON_LOOP => NifControllerKind::ResetOnLoop,
            CONTROLLER_UNSUPPORTED => NifControllerKind::Unsupported { type_name: Codec::decode(reader)? },
            tag => return Err(DescDecodeError::InvalidTag { what: "controller kind", tag: u32::from(tag) }),
        })
    }
}

impl Codec for NifController {
    fn encode(&self, writer: &mut Writer) {
        let NifController { owner, target, timing, kind } = self;
        writer.opt_index(*owner);
        target.encode(writer);
        timing.encode(writer);
        kind.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifController {
            owner: reader.opt_index()?,
            target: Codec::decode(reader)?,
            timing: Codec::decode(reader)?,
            kind: Codec::decode(reader)?,
        })
    }
}

impl Codec for NifEffectDesc {
    fn encode(&self, writer: &mut Writer) {
        let NifEffectDesc {
            name,
            nodes,
            root,
            meshes,
            particle_systems,
            materials,
            texturings,
            textures,
            lights,
            texture_effects,
            controllers,
        } = self;
        name.encode(writer);
        nodes.encode(writer);
        writer.opt_index(*root);
        meshes.encode(writer);
        particle_systems.encode(writer);
        materials.encode(writer);
        texturings.encode(writer);
        textures.encode(writer);
        lights.encode(writer);
        texture_effects.encode(writer);
        controllers.encode(writer);
    }

    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(NifEffectDesc {
            name: Codec::decode(reader)?,
            nodes: Codec::decode(reader)?,
            root: reader.opt_index()?,
            meshes: Codec::decode(reader)?,
            particle_systems: Codec::decode(reader)?,
            materials: Codec::decode(reader)?,
            texturings: Codec::decode(reader)?,
            textures: Codec::decode(reader)?,
            lights: Codec::decode(reader)?,
            texture_effects: Codec::decode(reader)?,
            controllers: Codec::decode(reader)?,
        })
    }
}

// endregion: --- NIF

#[cfg(test)]
mod tests;
