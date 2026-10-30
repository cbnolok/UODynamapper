//! Reader of `effects_ec.uddp`: the Enhanced Client particle effects (effect definition trees, compiled Chaos and
//! NIF systems, their textures), ready for live simulation with `particle-sim`.
//!
//! The layout is documented in the crate README (`effects_ec.uddp`). The record types and the manifest encoders
//! here are shared with the packer (`udd-conv`), so both sides use one definition.

use std::path::Path;

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr, bail, ensure, eyre};
use particle_sim::ChaosSystemDesc;
use particle_sim::NifEffectDesc;
use particle_sim::chaos::desc::CHAOS_DESC_VERSION;
use particle_sim::codec::{DESC_CODEC_VERSION, decode_chaos_desc, decode_nif_desc};
use particle_sim::nif::desc::NIF_DESC_VERSION;
use udd_container::UddpReader;
use uocf::enhanced::effect_definition::{
    EFFECT_TYPE_EARTHQUAKE, EFFECT_TYPE_LOD, EFFECT_TYPE_PARTICLE, EFFECT_TYPE_POSITION_OFFSET,
    EFFECT_TYPE_PROPERTY_FLAG, EFFECT_TYPE_RANDOM, EFFECT_TYPE_ROUND_ROBIN, EFFECT_TYPE_SOUND,
    EFFECT_TYPE_USER_PREFERENCE, EFFECT_TYPE_WAIT, PARTICLE_RESOURCE_DIR, POSITION_OFFSET_DIRECTION_COUNT,
};
use uocf::enhanced::effect_texture::effect_texture_path;

use crate::common::{read_path_entry_cow, read_pod_vec};

/// Version of every manifest of the package; the reader accepts only its own.
pub const EFFECTS_EC_METADATA_VERSION: u32 = 1;
/// Package file name.
pub const EFFECTS_EC_PACKAGE_FILE_NAME: &str = "effects_ec.uddp";

pub const PACKAGE_ENTRY_PATH: &str = "metadata/package.bin";
pub const STRINGS_ENTRY_PATH: &str = "metadata/strings.bin";
pub const COLLECTION_ENTRY_PATH: &str = "metadata/collection.bin";
pub const TILEART_ENTRY_PATH: &str = "metadata/tileart.bin";
pub const SEQUENCE_FRAMES_ENTRY_PATH: &str = "metadata/sequence_frames.bin";
pub const DEF_NODES_ENTRY_PATH: &str = "metadata/def_nodes.bin";
pub const DEF_LISTS_ENTRY_PATH: &str = "metadata/def_lists.bin";
pub const DEF_BRANCHES_ENTRY_PATH: &str = "metadata/def_branches.bin";
pub const DEF_PARAMS_ENTRY_PATH: &str = "metadata/def_params.bin";
pub const SYSTEMS_ENTRY_PATH: &str = "metadata/systems.bin";
pub const DEPS_ENTRY_PATH: &str = "metadata/deps.bin";
pub const TEXTURES_ENTRY_PATH: &str = "metadata/textures.bin";

pub const PACKAGE_MAGIC: [u8; 4] = *b"EFPK";
pub const STRINGS_MAGIC: [u8; 4] = *b"EFST";
pub const COLLECTION_MAGIC: [u8; 4] = *b"EFCO";
pub const TILEART_MAGIC: [u8; 4] = *b"EFTA";
pub const SEQUENCE_FRAMES_MAGIC: [u8; 4] = *b"EFSF";
pub const DEF_NODES_MAGIC: [u8; 4] = *b"EFDN";
pub const DEF_LISTS_MAGIC: [u8; 4] = *b"EFDL";
pub const DEF_BRANCHES_MAGIC: [u8; 4] = *b"EFDB";
pub const DEF_PARAMS_MAGIC: [u8; 4] = *b"EFDP";
pub const SYSTEMS_MAGIC: [u8; 4] = *b"EFSY";
pub const DEPS_MAGIC: [u8; 4] = *b"EFDE";
pub const TEXTURES_MAGIC: [u8; 4] = *b"EFTX";

/// "No index" in every u32 index field.
pub const NONE: u32 = u32::MAX;

/// `DefNodeRecord::flags`: `ParticleEffect::looping`.
pub const NODE_FLAG_LOOPING: u8 = 0x01;
/// `DefNodeRecord::flags`: `ParticleEffect::hued`.
pub const NODE_FLAG_HUED: u8 = 0x02;
/// `DefNodeRecord::flags`: `WaitEffect::aligned`.
pub const NODE_FLAG_WAIT_ALIGNED: u8 = 0x04;
/// `DefNodeRecord::flags`: PropertyFlag mode `Property` (clear: `FlagMask`).
pub const NODE_FLAG_PROPERTY_MODE: u8 = 0x08;

/// Parameter words of a Particle node: system, offset_x, offset_y, camera_offset, scale, duration, attach_mode,
/// field_3c.
pub const PARTICLE_PARAM_WORDS: u32 = 8;
/// Parameter words of a Sound node: sound_id, value_08.
pub const SOUND_PARAM_WORDS: u32 = 2;
/// Parameter words of a PositionOffset node: 16 `i8` bytes, `x0 y0 .. x7 y7`.
pub const POSITION_OFFSET_PARAM_WORDS: u32 = 4;
/// Parameter words of a Wait node: min_delay_ms, max_delay_ms.
pub const WAIT_PARAM_WORDS: u32 = 2;
/// Parameter words of a PropertyFlag node: mask low, mask high, property, default_first, default_count.
pub const PROPERTY_FLAG_PARAM_WORDS: u32 = 5;
/// Parameter words of an Earthquake node: amplitude, duration, frequency as f64 (low word first).
pub const EARTHQUAKE_PARAM_WORDS: u32 = 6;
/// Parameter words of a UserPreference node: mask.
pub const USER_PREFERENCE_PARAM_WORDS: u32 = 1;

/// `SystemRecord::kind`: a Chaos script (`.ems`), payload `systems/NNNNN.chaos`.
pub const SYSTEM_KIND_CHAOS: u8 = 1;
/// `SystemRecord::kind`: a Gamebryo scene (`.nif`), payload `systems/NNNNN.nif`.
pub const SYSTEM_KIND_NIF: u8 = 2;
/// `SystemRecord::flags`: no `EffectTexture.uop` entry (the client plays nothing); no payload.
pub const SYSTEM_FLAG_MISSING: u8 = 0x01;
/// `SystemRecord::flags`: the source does not compile; no payload.
pub const SYSTEM_FLAG_COMPILE_FAILED: u8 = 0x02;
/// `SystemRecord::flags`: the compiler reported warnings (see the packer report).
pub const SYSTEM_FLAG_COMPILE_WARNINGS: u8 = 0x04;
/// `SystemRecord::flags`: a particle definition names the system (clear: only a Chaos MESH sequence does).
pub const SYSTEM_FLAG_DEFINITION_ROOT: u8 = 0x08;
/// `deps.bin` value bit: the rest is a system index (a Chaos MESH `.nif`); clear, a texture index.
pub const DEP_SYSTEM_BIT: u32 = 1 << 31;

/// `TextureRecord::format`: no payload.
pub const TEXTURE_FORMAT_NONE: u8 = 0;
/// `TextureRecord::format`: 8-bit RGBA rows, top row first, straight alpha.
pub const TEXTURE_FORMAT_RGBA8: u8 = 1;
/// `TextureRecord::format`: BC7 blocks.
pub const TEXTURE_FORMAT_BC7: u8 = 2;
/// `TextureRecord::format`: BC1 (DXT1) blocks.
pub const TEXTURE_FORMAT_BC1: u8 = 3;
/// `TextureRecord::format`: BC3 (DXT5) blocks.
pub const TEXTURE_FORMAT_BC3: u8 = 4;
/// `TextureRecord::flags`: the pixels come from a NIF (`NiPixelData`); the name is `<nif name>#<index>`.
pub const TEXTURE_FLAG_EMBEDDED: u8 = 0x01;
/// `TextureRecord::flags`: every alpha value is 255.
pub const TEXTURE_FLAG_OPAQUE: u8 = 0x02;
/// `TextureRecord::flags`: mip levels 1.. come from the source, not from the packer.
pub const TEXTURE_FLAG_AUTHORED_MIPS: u8 = 0x04;
/// `TextureRecord::flags`: no `EffectTexture.uop` entry; no payload.
pub const TEXTURE_FLAG_MISSING: u8 = 0x08;
/// `TextureRecord::flags`: embedded pixel data in a format the packer does not decode; no payload.
pub const TEXTURE_FLAG_UNDECODED: u8 = 0x10;
/// `TextureRecord::flags`: the source is present but does not decode; no payload.
pub const TEXTURE_FLAG_DECODE_FAILED: u8 = 0x20;
/// `TextureRecord::source`: none (missing).
pub const TEXTURE_SOURCE_NONE: u8 = 0;
/// `TextureRecord::source`: a TGA file.
pub const TEXTURE_SOURCE_TGA: u8 = 1;
/// `TextureRecord::source`: a DDS file.
pub const TEXTURE_SOURCE_DDS: u8 = 2;
/// `TextureRecord::source`: `NiPixelData` of a NIF.
pub const TEXTURE_SOURCE_NIF_PIXEL_DATA: u8 = 3;

/// Bytes per RGBA8 pixel.
pub const RGBA8_BYTES_PER_PIXEL: usize = 4;
/// Block edge of the BC formats.
pub const BC_BLOCK_EDGE: u32 = 4;
/// Bytes per BC1 block.
pub const BC1_BLOCK_BYTES: usize = 8;
/// Bytes per BC3 and BC7 block.
pub const BC3_BC7_BLOCK_BYTES: usize = 16;

/// Bytes of the header of every manifest.
pub const MANIFEST_HEADER_SIZE: usize = std::mem::size_of::<ManifestHeader>();

/// Header of every `metadata/*.bin` manifest; records follow, tightly packed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct ManifestHeader {
    pub magic: [u8; 4],
    /// [`EFFECTS_EC_METADATA_VERSION`].
    pub version: u32,
    pub record_count: u32,
    /// `size_of` the record type.
    pub record_size: u32,
}

/// `package.bin`: the desc and codec versions of the system blobs.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct PackageRecord {
    pub chaos_desc_version: u32,
    pub nif_desc_version: u32,
    pub desc_codec_version: u32,
}

impl PackageRecord {
    /// The versions of this build.
    pub const CURRENT: Self = Self {
        chaos_desc_version: CHAOS_DESC_VERSION,
        nif_desc_version: NIF_DESC_VERSION,
        desc_codec_version: DESC_CODEC_VERSION,
    };
}

/// `strings.bin` record: a string of the byte area after the records.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct StringRecord {
    pub offset: u32,
    pub len: u32,
}

/// `collection.bin` record: an `EffectDefinitionCollection` entry id and its root list.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct CollectionRecord {
    pub collection_id: u32,
    pub list_first: u32,
    pub list_count: u32,
}

/// `tileart.bin` record: a tile id and its root list.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TileArtRecord {
    pub tile_id: u32,
    pub list_first: u32,
    pub list_count: u32,
}

/// `sequence_frames.bin` record: the effects of a frame of an `AnimationSequence` action.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct SequenceFrameRecord {
    /// `AnimationSequence.uop` entry id (`build/animationsequence/%08d.bin`), the
    /// `MobileAnimEcBodyRecord::sequence_id`, not the entry's `stored_id`.
    pub sequence_id: u32,
    /// The action whose record holds the effects, as stored (no alias resolution).
    pub action_id: u16,
    /// Frame index within the direction.
    pub frame: u16,
    pub list_first: u32,
    pub list_count: u32,
}

/// `def_nodes.bin` record: one effect definition. The index is the definition's identity.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefNodeRecord {
    /// The client type key (`uocf` `EFFECT_TYPE_*`).
    pub type_key: u8,
    /// `NODE_FLAG_*`.
    pub flags: u8,
    pub reserved: u16,
    /// First word in `def_params.bin`, or [`NONE`].
    pub param: u32,
    /// Start in `def_lists.bin` (RoundRobin, Wait, UserPreference) or `def_branches.bin` (Lod, PropertyFlag,
    /// Random); 0 for the others.
    pub first: u32,
    pub count: u32,
}

/// `def_branches.bin` record: a keyed (Lod level, PropertyFlag case) or weighted (Random, key = f32 bits) list.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefBranchRecord {
    pub key: u32,
    pub list_first: u32,
    pub list_count: u32,
}

impl DefBranchRecord {
    /// The weight of a Random entry.
    pub fn weight(&self) -> f32 {
        f32::from_bits(self.key)
    }
}

/// `systems.bin` record: a Chaos or NIF system, sorted by name. The index is the `NNNNN` of its payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct SystemRecord {
    /// String index of the normalized file name.
    pub name: u32,
    /// `SYSTEM_KIND_*`.
    pub kind: u8,
    /// `SYSTEM_FLAG_*`.
    pub flags: u8,
    pub dep_count: u16,
    /// Start in `deps.bin`.
    pub dep_first: u32,
}

impl SystemRecord {
    /// Whether the system has a payload.
    pub fn has_payload(&self) -> bool {
        self.flags & (SYSTEM_FLAG_MISSING | SYSTEM_FLAG_COMPILE_FAILED) == 0
    }
}

/// `textures.bin` record: a texture, sorted by name. The index is the `NNNNN` of its payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TextureRecord {
    /// String index of the normalized file name, or of `<nif name>#<index>` for embedded pixels.
    pub name: u32,
    pub width: u16,
    pub height: u16,
    /// `TEXTURE_FORMAT_*`.
    pub format: u8,
    /// Levels in the payload (0 without payload).
    pub mip_count: u8,
    /// `TEXTURE_FLAG_*`.
    pub flags: u8,
    /// `TEXTURE_SOURCE_*`.
    pub source: u8,
}

impl TextureRecord {
    pub fn has_payload(&self) -> bool {
        self.format != TEXTURE_FORMAT_NONE
    }
}

/// Path of the payload of system `index`.
pub fn system_entry_path(index: usize, kind: u8) -> String {
    let extension = if kind == SYSTEM_KIND_CHAOS { "chaos" } else { "nif" };
    format!("systems/{index:05}.{extension}")
}

/// Path of the payload of texture `index`.
pub fn texture_entry_path(index: usize) -> String {
    format!("textures/{index:05}.tex")
}

/// The name under which the package stores a resource: the last path segment, trimmed, ASCII-lowercased (the
/// client's `data/effects/<name>` lookup).
pub fn normalize_resource_name(name: &str) -> String {
    let path = effect_texture_path(name);
    path.strip_prefix(PARTICLE_RESOURCE_DIR).unwrap_or(&path).to_owned()
}

/// Bytes of one mip level.
pub fn texture_level_len(format: u8, width: u32, height: u32) -> Option<usize> {
    let blocks = || (width.div_ceil(BC_BLOCK_EDGE) as usize) * (height.div_ceil(BC_BLOCK_EDGE) as usize);
    match format {
        TEXTURE_FORMAT_RGBA8 => Some(width as usize * height as usize * RGBA8_BYTES_PER_PIXEL),
        TEXTURE_FORMAT_BC1 => Some(blocks() * BC1_BLOCK_BYTES),
        TEXTURE_FORMAT_BC3 | TEXTURE_FORMAT_BC7 => Some(blocks() * BC3_BC7_BLOCK_BYTES),
        _ => None,
    }
}

/// Size of level `level` of a `width` x `height` texture: `max(1, size >> level)`.
pub fn mip_extent(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// Bytes of a payload: every level, level 0 first.
pub fn texture_payload_len(format: u8, width: u32, height: u32, mip_count: u32) -> Option<usize> {
    (0..mip_count).try_fold(0usize, |total, level| {
        let (w, h) = mip_extent(width, height, level);
        Some(total + texture_level_len(format, w, h)?)
    })
}

/// A manifest: header and records.
pub fn encode_manifest<T: Pod>(magic: [u8; 4], records: &[T]) -> Vec<u8> {
    let header = ManifestHeader {
        magic,
        version: EFFECTS_EC_METADATA_VERSION,
        record_count: u32::try_from(records.len()).expect("manifest record count fits u32"),
        record_size: std::mem::size_of::<T>() as u32,
    };
    let mut bytes = bytemuck::bytes_of(&header).to_vec();
    bytes.extend_from_slice(bytemuck::cast_slice(records));
    bytes
}

/// `strings.bin`: header, records, then the bytes.
pub fn encode_strings<S: AsRef<str>>(strings: &[S]) -> Vec<u8> {
    let mut records = Vec::with_capacity(strings.len());
    let mut area = Vec::new();
    for string in strings {
        let string = string.as_ref().as_bytes();
        let offset = u32::try_from(area.len()).expect("string area fits u32");
        records.push(StringRecord { offset, len: u32::try_from(string.len()).expect("string fits u32") });
        area.extend_from_slice(string);
    }
    let mut bytes = encode_manifest(STRINGS_MAGIC, &records);
    bytes.extend_from_slice(&area);
    bytes
}

/// Splits a manifest into its records and the bytes after them, checking the header.
fn decode_manifest<'a, T: Pod>(
    bytes: &'a [u8],
    magic: [u8; 4],
    entry_path: &str,
    trailing_allowed: bool,
) -> eyre::Result<(Vec<T>, &'a [u8])> {
    ensure!(bytes.len() >= MANIFEST_HEADER_SIZE, "{entry_path}: {} bytes, shorter than its header", bytes.len());
    let header: ManifestHeader = bytemuck::pod_read_unaligned(&bytes[..MANIFEST_HEADER_SIZE]);
    ensure!(
        header.magic == magic,
        "{entry_path}: magic {:?}, expected {:?}",
        String::from_utf8_lossy(&header.magic),
        String::from_utf8_lossy(&magic)
    );
    ensure!(
        header.version == EFFECTS_EC_METADATA_VERSION,
        "{entry_path}: metadata version {}, this build reads {EFFECTS_EC_METADATA_VERSION}; rebuild {EFFECTS_EC_PACKAGE_FILE_NAME}",
        header.version
    );
    let record_size = std::mem::size_of::<T>();
    ensure!(
        header.record_size as usize == record_size,
        "{entry_path}: record size {}, expected {record_size}",
        header.record_size
    );
    let records_len = (header.record_count as usize)
        .checked_mul(record_size)
        .filter(|&len| len <= bytes.len() - MANIFEST_HEADER_SIZE)
        .ok_or_else(|| eyre!("{entry_path}: {} records do not fit {} bytes", header.record_count, bytes.len()))?;
    let records_end = MANIFEST_HEADER_SIZE + records_len;
    let trailing = &bytes[records_end..];
    ensure!(trailing_allowed || trailing.is_empty(), "{entry_path}: {} trailing bytes", trailing.len());
    Ok((read_pod_vec(&bytes[MANIFEST_HEADER_SIZE..records_end], entry_path)?, trailing))
}

/// Decodes `strings.bin`.
pub fn decode_strings(bytes: &[u8]) -> eyre::Result<Vec<String>> {
    let (records, area) = decode_manifest::<StringRecord>(bytes, STRINGS_MAGIC, STRINGS_ENTRY_PATH, true)?;
    records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let start = record.offset as usize;
            let bytes = start
                .checked_add(record.len as usize)
                .and_then(|end| area.get(start..end))
                .ok_or_else(|| eyre!("{STRINGS_ENTRY_PATH}: string {index} out of the byte area"))?;
            String::from_utf8(bytes.to_vec()).wrap_err_with(|| format!("{STRINGS_ENTRY_PATH}: string {index}"))
        })
        .collect()
}

/// A Particle node's placement and playback parameters (`ParticleEffect` fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleNode {
    /// Index in [`EffectsEcPackage::systems`]; `None` when the name index is not in the string dictionary.
    pub system: Option<u32>,
    pub offset_x: f32,
    pub offset_y: f32,
    pub camera_offset: f32,
    pub scale: f32,
    pub duration: f32,
    pub attach_mode: u8,
    pub field_3c: u8,
    pub looping: bool,
    pub hued: bool,
}

/// A decoded view of a definition node; lists are indices in [`EffectsEcPackage::nodes`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectNode<'a> {
    Particle(ParticleNode),
    Sound { sound_id: u32, value_08: f32 },
    RoundRobin { children: &'a [u32] },
    /// Per direction (0 north .. 7 north-west): (x, y).
    PositionOffset { offsets: [(i8, i8); POSITION_OFFSET_DIRECTION_COUNT] },
    Wait { children: &'a [u32], min_delay_ms: u32, max_delay_ms: u32, aligned: bool },
    /// Levels by ascending key; each branch's list in [`EffectsEcPackage::list`].
    Lod { levels: &'a [DefBranchRecord] },
    PropertyFlag {
        /// `PropertyFlagMode::Property` (else `FlagMask`).
        property_mode: bool,
        mask: u64,
        property: u32,
        default_children: &'a [u32],
        /// Cases by ascending key.
        cases: &'a [DefBranchRecord],
    },
    Earthquake { amplitude: f64, duration: f64, frequency: f64 },
    /// Entries in file order; [`DefBranchRecord::weight`].
    Random { entries: &'a [DefBranchRecord] },
    UserPreference { mask: u32, children: &'a [u32] },
}

/// A texture payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    /// `TEXTURE_FORMAT_*`.
    pub format: u8,
    pub mip_count: u32,
    /// Every level, level 0 first ([`texture_level_len`] bytes each).
    pub data: Vec<u8>,
}

/// What a node's `first`/`count` index.
enum NodeLinks {
    None,
    Lists,
    Branches,
}

/// Parameter words and links of a type key.
fn node_layout(type_key: u8) -> Option<(Option<u32>, NodeLinks)> {
    Some(match u32::from(type_key) {
        EFFECT_TYPE_PARTICLE => (Some(PARTICLE_PARAM_WORDS), NodeLinks::None),
        EFFECT_TYPE_SOUND => (Some(SOUND_PARAM_WORDS), NodeLinks::None),
        EFFECT_TYPE_ROUND_ROBIN => (None, NodeLinks::Lists),
        EFFECT_TYPE_POSITION_OFFSET => (Some(POSITION_OFFSET_PARAM_WORDS), NodeLinks::None),
        EFFECT_TYPE_WAIT => (Some(WAIT_PARAM_WORDS), NodeLinks::Lists),
        EFFECT_TYPE_LOD => (None, NodeLinks::Branches),
        EFFECT_TYPE_PROPERTY_FLAG => (Some(PROPERTY_FLAG_PARAM_WORDS), NodeLinks::Branches),
        EFFECT_TYPE_EARTHQUAKE => (Some(EARTHQUAKE_PARAM_WORDS), NodeLinks::None),
        EFFECT_TYPE_RANDOM => (None, NodeLinks::Branches),
        EFFECT_TYPE_USER_PREFERENCE => (Some(USER_PREFERENCE_PARAM_WORDS), NodeLinks::Lists),
        _ => return None,
    })
}

/// Index of the particle system in a Particle node's parameters.
const PARTICLE_SYSTEM_WORD: usize = 0;
/// Index of the mask (low, high) in a PropertyFlag node's parameters.
const PROPERTY_FLAG_MASK_WORD: usize = 0;
/// Index of the property in a PropertyFlag node's parameters.
const PROPERTY_FLAG_PROPERTY_WORD: usize = 2;
/// Index of the PropertyFlag default list (first, count) in its parameters.
const PROPERTY_FLAG_DEFAULT_WORDS: usize = 3;
/// u32 words of an f64 parameter (low word first).
const F64_WORDS: usize = 2;
/// Bytes per direction of a PositionOffset node: x, y.
const POSITION_OFFSET_BYTES_PER_DIRECTION: usize = 2;

/// Parameter words of a Particle node (flags hold `looping` and `hued`).
pub fn particle_params(node: &ParticleNode) -> [u32; PARTICLE_PARAM_WORDS as usize] {
    let ParticleNode { system, offset_x, offset_y, camera_offset, scale, duration, attach_mode, field_3c, looping: _, hued: _ } =
        *node;
    [
        system.unwrap_or(NONE),
        offset_x.to_bits(),
        offset_y.to_bits(),
        camera_offset.to_bits(),
        scale.to_bits(),
        duration.to_bits(),
        u32::from(attach_mode),
        u32::from(field_3c),
    ]
}

fn decode_particle_params(words: &[u32], flags: u8) -> ParticleNode {
    let [system, offset_x, offset_y, camera_offset, scale, duration, attach_mode, field_3c] =
        words.try_into().expect("particle words");
    ParticleNode {
        system: (system != NONE).then_some(system),
        offset_x: f32::from_bits(offset_x),
        offset_y: f32::from_bits(offset_y),
        camera_offset: f32::from_bits(camera_offset),
        scale: f32::from_bits(scale),
        duration: f32::from_bits(duration),
        attach_mode: attach_mode as u8,
        field_3c: field_3c as u8,
        looping: flags & NODE_FLAG_LOOPING != 0,
        hued: flags & NODE_FLAG_HUED != 0,
    }
}

/// Parameter words of a PositionOffset node: the (x, y) bytes per direction, in byte order.
pub fn position_offset_params(offsets: &[(i8, i8); POSITION_OFFSET_DIRECTION_COUNT]) -> [u32; POSITION_OFFSET_PARAM_WORDS as usize] {
    let bytes: Vec<u8> = offsets.iter().flat_map(|&(x, y)| [x as u8, y as u8]).collect();
    let mut words = [0; POSITION_OFFSET_PARAM_WORDS as usize];
    for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(size_of::<u32>())) {
        *word = u32::from_le_bytes(chunk.try_into().expect("word bytes"));
    }
    words
}

fn decode_position_offset_params(words: &[u32]) -> [(i8, i8); POSITION_OFFSET_DIRECTION_COUNT] {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut offsets = [(0, 0); POSITION_OFFSET_DIRECTION_COUNT];
    for (offset, pair) in offsets.iter_mut().zip(bytes.chunks_exact(POSITION_OFFSET_BYTES_PER_DIRECTION)) {
        *offset = (pair[0] as i8, pair[1] as i8);
    }
    offsets
}

/// Parameter words of a PropertyFlag node.
pub fn property_flag_params(mask: u64, property: u32, default_first: u32, default_count: u32) -> [u32; PROPERTY_FLAG_PARAM_WORDS as usize] {
    [mask as u32, (mask >> u32::BITS) as u32, property, default_first, default_count]
}

/// Parameter words of an Earthquake node.
pub fn earthquake_params(amplitude: f64, duration: f64, frequency: f64) -> [u32; EARTHQUAKE_PARAM_WORDS as usize] {
    let words = |value: f64| [value.to_bits() as u32, (value.to_bits() >> u32::BITS) as u32];
    let [a, b, c] = [words(amplitude), words(duration), words(frequency)];
    [a[0], a[1], b[0], b[1], c[0], c[1]]
}

fn range(first: u32, count: u32, len: usize, what: &str) -> eyre::Result<std::ops::Range<usize>> {
    let start = first as usize;
    let end = start.checked_add(count as usize).filter(|&end| end <= len);
    end.map(|end| start..end).ok_or_else(|| eyre!("{what}: range {first}+{count} outside {len} entries"))
}

fn f64_from_words(words: &[u32]) -> f64 {
    f64::from_bits(u64::from(words[0]) | (u64::from(words[1]) << u32::BITS))
}

/// `effects_ec.uddp` opened: every manifest decoded and checked, payloads read on demand.
pub struct EffectsEcPackage {
    reader: UddpReader,
    strings: Vec<String>,
    collection: Vec<CollectionRecord>,
    tileart: Vec<TileArtRecord>,
    sequence_frames: Vec<SequenceFrameRecord>,
    nodes: Vec<DefNodeRecord>,
    lists: Vec<u32>,
    branches: Vec<DefBranchRecord>,
    params: Vec<u32>,
    systems: Vec<SystemRecord>,
    deps: Vec<u32>,
    textures: Vec<TextureRecord>,
}

impl EffectsEcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let reader = UddpReader::load(path).wrap_err_with(|| format!("open {}", path.display()))?;
        Self::from_reader(reader).wrap_err_with(|| format!("read {}", path.display()))
    }

    pub fn from_reader(reader: UddpReader) -> eyre::Result<Self> {
        fn manifest<T: Pod>(reader: &UddpReader, path: &str, magic: [u8; 4]) -> eyre::Result<Vec<T>> {
            let bytes = read_path_entry_cow(reader, path)?;
            Ok(decode_manifest(&bytes, magic, path, false)?.0)
        }
        let package: Vec<PackageRecord> = manifest(&reader, PACKAGE_ENTRY_PATH, PACKAGE_MAGIC)?;
        ensure!(package.len() == 1, "{PACKAGE_ENTRY_PATH}: {} records, expected 1", package.len());
        ensure!(
            package[0] == PackageRecord::CURRENT,
            "system blob versions {:?}, this build reads {:?}; rebuild {EFFECTS_EC_PACKAGE_FILE_NAME}",
            package[0],
            PackageRecord::CURRENT
        );
        let strings = decode_strings(&read_path_entry_cow(&reader, STRINGS_ENTRY_PATH)?)?;
        let package = Self {
            collection: manifest(&reader, COLLECTION_ENTRY_PATH, COLLECTION_MAGIC)?,
            tileart: manifest(&reader, TILEART_ENTRY_PATH, TILEART_MAGIC)?,
            sequence_frames: manifest(&reader, SEQUENCE_FRAMES_ENTRY_PATH, SEQUENCE_FRAMES_MAGIC)?,
            nodes: manifest(&reader, DEF_NODES_ENTRY_PATH, DEF_NODES_MAGIC)?,
            lists: manifest(&reader, DEF_LISTS_ENTRY_PATH, DEF_LISTS_MAGIC)?,
            branches: manifest(&reader, DEF_BRANCHES_ENTRY_PATH, DEF_BRANCHES_MAGIC)?,
            params: manifest(&reader, DEF_PARAMS_ENTRY_PATH, DEF_PARAMS_MAGIC)?,
            systems: manifest(&reader, SYSTEMS_ENTRY_PATH, SYSTEMS_MAGIC)?,
            deps: manifest(&reader, DEPS_ENTRY_PATH, DEPS_MAGIC)?,
            textures: manifest(&reader, TEXTURES_ENTRY_PATH, TEXTURES_MAGIC)?,
            strings,
            reader,
        };
        package.validate()?;
        Ok(package)
    }

    /// Checks every index, range, order and payload length once, so lookups never fail on a loaded package.
    fn validate(&self) -> eyre::Result<()> {
        let lists = self.lists.len();
        for (index, &node) in self.lists.iter().enumerate() {
            ensure!((node as usize) < self.nodes.len(), "list entry {index}: node {node} out of range");
        }
        for (index, branch) in self.branches.iter().enumerate() {
            range(branch.list_first, branch.list_count, lists, &format!("branch {index}"))?;
        }
        for (index, node) in self.nodes.iter().enumerate() {
            self.validate_node(index, node)?;
        }
        let roots = self
            .collection
            .iter()
            .map(|r| (r.list_first, r.list_count))
            .chain(self.tileart.iter().map(|r| (r.list_first, r.list_count)))
            .chain(self.sequence_frames.iter().map(|r| (r.list_first, r.list_count)));
        for (first, count) in roots {
            range(first, count, lists, "root list")?;
        }
        ensure!(self.collection.is_sorted_by(|a, b| a.collection_id < b.collection_id), "collection ids not sorted");
        ensure!(self.tileart.is_sorted_by(|a, b| a.tile_id < b.tile_id), "tile ids not sorted");
        ensure!(
            self.sequence_frames.is_sorted_by(|a, b| frame_key(a) < frame_key(b)),
            "sequence frames not sorted"
        );
        let name = |index: u32, what: &str| {
            self.strings.get(index as usize).map(String::as_str).ok_or_else(|| eyre!("{what}: string {index} out of range"))
        };
        let mut previous: Option<&str> = None;
        for (index, system) in self.systems.iter().enumerate() {
            let system_name = name(system.name, "system name")?;
            ensure!(previous.is_none_or(|previous| previous < system_name), "systems not sorted at {index}");
            previous = Some(system_name);
            ensure!(
                matches!(system.kind, SYSTEM_KIND_CHAOS | SYSTEM_KIND_NIF),
                "system {index}: kind {}",
                system.kind
            );
            for &dep in &self.deps[range(system.dep_first, u32::from(system.dep_count), self.deps.len(), "deps")?] {
                let target = (dep & !DEP_SYSTEM_BIT) as usize;
                let len = if dep & DEP_SYSTEM_BIT != 0 { self.systems.len() } else { self.textures.len() };
                ensure!(target < len, "system {index}: dependency {dep:#x} out of range");
            }
        }
        let mut previous: Option<&str> = None;
        for (index, texture) in self.textures.iter().enumerate() {
            let texture_name = name(texture.name, "texture name")?;
            ensure!(previous.is_none_or(|previous| previous < texture_name), "textures not sorted at {index}");
            previous = Some(texture_name);
            if texture.has_payload() {
                ensure!(
                    texture_level_len(texture.format, 1, 1).is_some(),
                    "texture {index}: format {}",
                    texture.format
                );
                ensure!(texture.width > 0 && texture.height > 0 && texture.mip_count > 0, "texture {index}: empty");
            }
        }
        Ok(())
    }

    fn validate_node(&self, index: usize, node: &DefNodeRecord) -> eyre::Result<()> {
        let what = || format!("node {index}");
        let (param_words, links) =
            node_layout(node.type_key).ok_or_else(|| eyre!("{}: type key {}", what(), node.type_key))?;
        match param_words {
            Some(words) => {
                let params = range(node.param, words, self.params.len(), &what())?;
                let params = &self.params[params];
                if u32::from(node.type_key) == EFFECT_TYPE_PARTICLE {
                    let system = params[PARTICLE_SYSTEM_WORD];
                    ensure!(system == NONE || (system as usize) < self.systems.len(), "{}: system {system}", what());
                }
                if u32::from(node.type_key) == EFFECT_TYPE_PROPERTY_FLAG {
                    let [first, count] = [params[PROPERTY_FLAG_DEFAULT_WORDS], params[PROPERTY_FLAG_DEFAULT_WORDS + 1]];
                    range(first, count, self.lists.len(), &what())?;
                }
            }
            None => ensure!(node.param == NONE, "{}: unexpected parameters", what()),
        }
        match links {
            NodeLinks::None => ensure!(node.count == 0, "{}: unexpected children", what()),
            NodeLinks::Lists => drop(range(node.first, node.count, self.lists.len(), &what())?),
            NodeLinks::Branches => drop(range(node.first, node.count, self.branches.len(), &what())?),
        }
        Ok(())
    }

    pub fn strings(&self) -> &[String] {
        &self.strings
    }

    pub fn collection(&self) -> &[CollectionRecord] {
        &self.collection
    }

    pub fn tileart(&self) -> &[TileArtRecord] {
        &self.tileart
    }

    pub fn sequence_frames(&self) -> &[SequenceFrameRecord] {
        &self.sequence_frames
    }

    pub fn nodes(&self) -> &[DefNodeRecord] {
        &self.nodes
    }

    pub fn systems(&self) -> &[SystemRecord] {
        &self.systems
    }

    pub fn textures(&self) -> &[TextureRecord] {
        &self.textures
    }

    /// Nodes of a list range.
    pub fn list(&self, first: u32, count: u32) -> &[u32] {
        &self.lists[first as usize..(first + count) as usize]
    }

    /// The root list of a collection entry id (no fallback: the caller applies `EFFECT_ID_FALLBACK`).
    pub fn collection_roots(&self, collection_id: u32) -> Option<&[u32]> {
        let index = self.collection.binary_search_by_key(&collection_id, |record| record.collection_id).ok()?;
        let record = self.collection[index];
        Some(self.list(record.list_first, record.list_count))
    }

    /// The root list of a tile.
    pub fn tileart_roots(&self, tile_id: u32) -> Option<&[u32]> {
        let index = self.tileart.binary_search_by_key(&tile_id, |record| record.tile_id).ok()?;
        let record = self.tileart[index];
        Some(self.list(record.list_first, record.list_count))
    }

    /// The root list of a frame of a sequence action.
    pub fn frame_roots(&self, sequence_id: u32, action_id: u16, frame: u16) -> Option<&[u32]> {
        let key = (sequence_id, action_id, frame);
        let index = self.sequence_frames.binary_search_by_key(&key, frame_key).ok()?;
        let record = self.sequence_frames[index];
        Some(self.list(record.list_first, record.list_count))
    }

    /// The node at `index` (a value of a list).
    pub fn node(&self, index: u32) -> EffectNode<'_> {
        let record = self.nodes[index as usize];
        let words = |count: u32| &self.params[record.param as usize..(record.param + count) as usize];
        let children = || self.list(record.first, record.count);
        let branches = || &self.branches[record.first as usize..(record.first + record.count) as usize];
        match u32::from(record.type_key) {
            EFFECT_TYPE_PARTICLE => EffectNode::Particle(decode_particle_params(words(PARTICLE_PARAM_WORDS), record.flags)),
            EFFECT_TYPE_SOUND => {
                let [sound_id, value_08] = words(SOUND_PARAM_WORDS).try_into().expect("sound words");
                EffectNode::Sound { sound_id, value_08: f32::from_bits(value_08) }
            }
            EFFECT_TYPE_ROUND_ROBIN => EffectNode::RoundRobin { children: children() },
            EFFECT_TYPE_POSITION_OFFSET => {
                EffectNode::PositionOffset { offsets: decode_position_offset_params(words(POSITION_OFFSET_PARAM_WORDS)) }
            }
            EFFECT_TYPE_WAIT => {
                let [min_delay_ms, max_delay_ms] = words(WAIT_PARAM_WORDS).try_into().expect("wait words");
                EffectNode::Wait {
                    children: children(),
                    min_delay_ms,
                    max_delay_ms,
                    aligned: record.flags & NODE_FLAG_WAIT_ALIGNED != 0,
                }
            }
            EFFECT_TYPE_LOD => EffectNode::Lod { levels: branches() },
            EFFECT_TYPE_PROPERTY_FLAG => {
                let p = words(PROPERTY_FLAG_PARAM_WORDS);
                EffectNode::PropertyFlag {
                    property_mode: record.flags & NODE_FLAG_PROPERTY_MODE != 0,
                    mask: u64::from(p[PROPERTY_FLAG_MASK_WORD]) | (u64::from(p[PROPERTY_FLAG_MASK_WORD + 1]) << u32::BITS),
                    property: p[PROPERTY_FLAG_PROPERTY_WORD],
                    default_children: self.list(p[PROPERTY_FLAG_DEFAULT_WORDS], p[PROPERTY_FLAG_DEFAULT_WORDS + 1]),
                    cases: branches(),
                }
            }
            EFFECT_TYPE_EARTHQUAKE => {
                let p = words(EARTHQUAKE_PARAM_WORDS);
                let value = |field: usize| f64_from_words(&p[field * F64_WORDS..(field + 1) * F64_WORDS]);
                EffectNode::Earthquake { amplitude: value(0), duration: value(1), frequency: value(2) }
            }
            EFFECT_TYPE_RANDOM => EffectNode::Random { entries: branches() },
            EFFECT_TYPE_USER_PREFERENCE => {
                EffectNode::UserPreference { mask: words(USER_PREFERENCE_PARAM_WORDS)[0], children: children() }
            }
            _ => unreachable!("type keys are checked at load"),
        }
    }

    pub fn system(&self, index: u32) -> &SystemRecord {
        &self.systems[index as usize]
    }

    pub fn system_name(&self, index: u32) -> &str {
        &self.strings[self.system(index).name as usize]
    }

    /// The system of a name, normalized first.
    pub fn system_by_name(&self, name: &str) -> Option<u32> {
        let name = normalize_resource_name(name);
        self.systems.binary_search_by(|record| self.strings[record.name as usize].as_str().cmp(&name)).ok().map(|i| i as u32)
    }

    pub fn texture(&self, index: u32) -> &TextureRecord {
        &self.textures[index as usize]
    }

    pub fn texture_name(&self, index: u32) -> &str {
        &self.strings[self.texture(index).name as usize]
    }

    /// The texture of a name, normalized first (render output names textures by file name, or `<nif>#<index>`
    /// for embedded ones).
    pub fn texture_by_name(&self, name: &str) -> Option<u32> {
        let name = normalize_resource_name(name);
        self.textures.binary_search_by(|record| self.strings[record.name as usize].as_str().cmp(&name)).ok().map(|i| i as u32)
    }

    /// Direct dependencies of a system: `deps.bin` values ([`DEP_SYSTEM_BIT`]).
    pub fn system_dependencies(&self, index: u32) -> &[u32] {
        let record = self.system(index);
        &self.deps[record.dep_first as usize..record.dep_first as usize + usize::from(record.dep_count)]
    }

    fn system_payload(&self, index: u32, kind: u8) -> eyre::Result<std::borrow::Cow<'_, [u8]>> {
        let record = self.system(index);
        ensure!(record.kind == kind, "system {} is not of kind {kind}", self.system_name(index));
        ensure!(record.has_payload(), "system {} has no payload (flags {:#x})", self.system_name(index), record.flags);
        read_path_entry_cow(&self.reader, &system_entry_path(index as usize, kind))
    }

    pub fn read_chaos_desc(&self, index: u32) -> eyre::Result<ChaosSystemDesc> {
        let bytes = self.system_payload(index, SYSTEM_KIND_CHAOS)?;
        decode_chaos_desc(&bytes).wrap_err_with(|| format!("decode {}", self.system_name(index)))
    }

    pub fn read_nif_desc(&self, index: u32) -> eyre::Result<NifEffectDesc> {
        let bytes = self.system_payload(index, SYSTEM_KIND_NIF)?;
        decode_nif_desc(&bytes).wrap_err_with(|| format!("decode {}", self.system_name(index)))
    }

    pub fn read_texture(&self, index: u32) -> eyre::Result<TextureData> {
        let record = *self.texture(index);
        if !record.has_payload() {
            bail!("texture {} has no payload (flags {:#x})", self.texture_name(index), record.flags);
        }
        let data = read_path_entry_cow(&self.reader, &texture_entry_path(index as usize))?.into_owned();
        let (width, height, mip_count) = (u32::from(record.width), u32::from(record.height), u32::from(record.mip_count));
        let expected = texture_payload_len(record.format, width, height, mip_count);
        ensure!(
            expected == Some(data.len()),
            "texture {}: {} bytes, expected {expected:?}",
            self.texture_name(index),
            data.len()
        );
        Ok(TextureData { width, height, format: record.format, mip_count, data })
    }
}

fn frame_key(record: &SequenceFrameRecord) -> (u32, u16, u16) {
    (record.sequence_id, record.action_id, record.frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_sizes_match_the_layout() {
        assert_eq!(MANIFEST_HEADER_SIZE, 16);
        assert_eq!(std::mem::size_of::<PackageRecord>(), 12);
        assert_eq!(std::mem::size_of::<StringRecord>(), 8);
        assert_eq!(std::mem::size_of::<CollectionRecord>(), 12);
        assert_eq!(std::mem::size_of::<TileArtRecord>(), 12);
        assert_eq!(std::mem::size_of::<SequenceFrameRecord>(), 16);
        assert_eq!(std::mem::size_of::<DefNodeRecord>(), 16);
        assert_eq!(std::mem::size_of::<DefBranchRecord>(), 12);
        assert_eq!(std::mem::size_of::<SystemRecord>(), 12);
        assert_eq!(std::mem::size_of::<TextureRecord>(), 12);
    }

    #[test]
    fn manifest_round_trips() {
        let records = vec![
            CollectionRecord { collection_id: 1, list_first: 0, list_count: 2 },
            CollectionRecord { collection_id: 9502, list_first: 2, list_count: 1 },
        ];
        let bytes = encode_manifest(COLLECTION_MAGIC, &records);
        assert_eq!(bytes.len(), MANIFEST_HEADER_SIZE + records.len() * std::mem::size_of::<CollectionRecord>());
        let (decoded, trailing) =
            decode_manifest::<CollectionRecord>(&bytes, COLLECTION_MAGIC, COLLECTION_ENTRY_PATH, false).unwrap();
        assert_eq!(decoded, records);
        assert!(trailing.is_empty());
    }

    #[test]
    fn strings_round_trip() {
        let strings = ["00005001_clumsy.nif", "", "lightning.ems", "03002014_flare.nif#0"];
        assert_eq!(decode_strings(&encode_strings(&strings)).unwrap(), strings);
    }

    #[test]
    fn manifest_rejects_bad_magic_version_size_and_length() {
        let records = [TileArtRecord { tile_id: 7, list_first: 0, list_count: 1 }];
        let bytes = encode_manifest(TILEART_MAGIC, &records);
        fn decode(bytes: &[u8]) -> eyre::Result<Vec<TileArtRecord>> {
            Ok(decode_manifest::<TileArtRecord>(bytes, TILEART_MAGIC, TILEART_ENTRY_PATH, false)?.0)
        }
        assert!(decode(&bytes).is_ok());
        assert!(decode_manifest::<TileArtRecord>(&bytes, COLLECTION_MAGIC, TILEART_ENTRY_PATH, false).is_err());
        let field = |offset: usize| {
            let mut bytes = bytes.clone();
            bytes[offset] ^= 1;
            bytes
        };
        let version_offset = std::mem::offset_of!(ManifestHeader, version);
        let error = decode(&field(version_offset)).unwrap_err().to_string();
        assert!(error.contains("metadata version"), "{error}");
        assert!(decode(&field(std::mem::offset_of!(ManifestHeader, record_size))).is_err());
        assert!(decode(&field(std::mem::offset_of!(ManifestHeader, record_count))).is_err());
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        assert!(decode(&bytes[..MANIFEST_HEADER_SIZE - 1]).is_err());
    }

    #[test]
    fn node_params_round_trip() {
        let particle = ParticleNode {
            system: Some(3),
            offset_x: 1.5,
            offset_y: -2.0,
            camera_offset: 0.25,
            scale: -1.0,
            duration: 4.0,
            attach_mode: 2,
            field_3c: 0,
            looping: true,
            hued: false,
        };
        assert_eq!(decode_particle_params(&particle_params(&particle), NODE_FLAG_LOOPING), particle);
        let offsets = [(1, -1), (2, -2), (3, -3), (4, -4), (5, -5), (6, -6), (7, -7), (-128, 127)];
        assert_eq!(decode_position_offset_params(&position_offset_params(&offsets)), offsets);
        let words = earthquake_params(0.5, 2.0, -3.25);
        let value = |field: usize| f64_from_words(&words[field * F64_WORDS..]);
        assert_eq!([value(0), value(1), value(2)], [0.5, 2.0, -3.25]);
    }

    /// A package with empty tables, `package` as its versions and `collection` as its collection manifest.
    fn package_bytes(package: PackageRecord, collection: Vec<u8>) -> Vec<u8> {
        use udd_container::{AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
        let empty = |magic| encode_manifest::<u32>(magic, &[]);
        let files = [
            (PACKAGE_ENTRY_PATH, encode_manifest(PACKAGE_MAGIC, &[package])),
            (STRINGS_ENTRY_PATH, encode_strings::<&str>(&[])),
            (COLLECTION_ENTRY_PATH, collection),
            (TILEART_ENTRY_PATH, encode_manifest::<TileArtRecord>(TILEART_MAGIC, &[])),
            (SEQUENCE_FRAMES_ENTRY_PATH, encode_manifest::<SequenceFrameRecord>(SEQUENCE_FRAMES_MAGIC, &[])),
            (DEF_NODES_ENTRY_PATH, encode_manifest::<DefNodeRecord>(DEF_NODES_MAGIC, &[])),
            (DEF_LISTS_ENTRY_PATH, empty(DEF_LISTS_MAGIC)),
            (DEF_BRANCHES_ENTRY_PATH, encode_manifest::<DefBranchRecord>(DEF_BRANCHES_MAGIC, &[])),
            (DEF_PARAMS_ENTRY_PATH, empty(DEF_PARAMS_MAGIC)),
            (SYSTEMS_ENTRY_PATH, encode_manifest::<SystemRecord>(SYSTEMS_MAGIC, &[])),
            (DEPS_ENTRY_PATH, empty(DEPS_MAGIC)),
            (TEXTURES_ENTRY_PATH, encode_manifest::<TextureRecord>(TEXTURES_MAGIC, &[])),
        ];
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data) in files {
            builder
                .add_owned_file(AddOwnedFileRequest {
                    data_type: DataType::Metadata as u8,
                    compression: CompressionFlag::None,
                    width: 0,
                    height: 0,
                    virtual_path: Some(path.to_owned()),
                    path_hash64: None,
                    id: None,
                    data,
                })
                .unwrap();
        }
        builder.into_bytes().unwrap()
    }

    fn open(bytes: Vec<u8>) -> eyre::Result<EffectsEcPackage> {
        EffectsEcPackage::from_reader(UddpReader::open(bytes)?)
    }

    #[test]
    fn package_opens_only_with_its_own_versions_and_magics() {
        let collection = || encode_manifest::<CollectionRecord>(COLLECTION_MAGIC, &[]);
        let package = open(package_bytes(PackageRecord::CURRENT, collection())).unwrap();
        assert!(package.collection_roots(0).is_none());

        let other_desc = PackageRecord { nif_desc_version: NIF_DESC_VERSION + 1, ..PackageRecord::CURRENT };
        let error = open(package_bytes(other_desc, collection())).err().unwrap().to_string();
        assert!(error.contains("rebuild"), "{error}");
        let other_codec = PackageRecord { desc_codec_version: DESC_CODEC_VERSION + 1, ..PackageRecord::CURRENT };
        assert!(open(package_bytes(other_codec, collection())).is_err());

        let wrong_magic = encode_manifest::<CollectionRecord>(TILEART_MAGIC, &[]);
        assert!(open(package_bytes(PackageRecord::CURRENT, wrong_magic)).is_err());
        let dangling = encode_manifest(COLLECTION_MAGIC, &[CollectionRecord { collection_id: 1, list_first: 0, list_count: 1 }]);
        assert!(open(package_bytes(PackageRecord::CURRENT, dangling)).is_err());
    }

    #[test]
    fn texture_payload_lengths() {
        assert_eq!(texture_payload_len(TEXTURE_FORMAT_RGBA8, 4, 2, 3), Some(4 * 2 * 4 + 2 * 4 + 4));
        assert_eq!(texture_payload_len(TEXTURE_FORMAT_BC1, 8, 8, 4), Some((4 + 1 + 1 + 1) * BC1_BLOCK_BYTES));
        assert_eq!(texture_payload_len(TEXTURE_FORMAT_BC7, 16, 4, 1), Some(4 * BC3_BC7_BLOCK_BYTES));
        assert_eq!(texture_payload_len(TEXTURE_FORMAT_NONE, 16, 4, 1), None);
    }

    #[test]
    fn names_are_normalized_like_the_client() {
        assert_eq!(normalize_resource_name("Data\\Effects\\03002012_Flare.TGA "), "03002012_flare.tga");
        assert_eq!(normalize_resource_name("00014732_field of fire.nif"), "00014732_field of fire.nif");
        assert_eq!(normalize_resource_name("03002014_flare.nif#0"), "03002014_flare.nif#0");
    }
}
