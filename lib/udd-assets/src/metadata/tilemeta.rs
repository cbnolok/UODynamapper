use crate::common::{ensure_manifest_version, read_path_entry_cow, read_pod_vec};
use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::UddpReader;

pub mod item_definitions;
pub use item_definitions::{TileMetaItemDefinition, TileMetaItemDefinitions};

pub const TILEMETA_LAND_ENTRY_PATH: &str = "metadata/land.bin";
pub const TILEMETA_ITEM_ENTRY_PATH: &str = "metadata/items.bin";
pub const TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH: &str = "metadata/item_texture_refs_index.bin";
/// The texture refs, after a header that carries `TILEMETA_FORMAT_VERSION` ([`encode_item_texture_refs`]).
pub const TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH: &str = "metadata/item_texture_refs.bin";
/// Magic of the `TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH` header: magic, then `u32` `TILEMETA_FORMAT_VERSION`.
pub const TILEMETA_ITEM_TEXTURE_REF_MAGIC: [u8; 4] = *b"TMTR";
/// Size of the `TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH` header: the magic and the `u32` version.
const TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE: usize = TILEMETA_ITEM_TEXTURE_REF_MAGIC.len() + std::mem::size_of::<u32>();
pub const TILEMETA_ITEM_MATERIAL_ENTRY_PATH: &str = "metadata/item_materials.json";
/// The `tileart.uop` records the Enhanced paperdoll reads ([`TileMetaItemDefinitions`]).
pub const TILEMETA_ITEM_DEFINITION_ENTRY_PATH: &str = "metadata/item_definitions.bin";
/// The light sprites of the Enhanced and KR clients ([`encode_item_lights`]). Required; empty without EC tileart.
pub const TILEMETA_ITEM_LIGHT_ENTRY_PATH: &str = "metadata/item_lights.bin";
/// Magic of the `TILEMETA_ITEM_LIGHT_ENTRY_PATH` header: magic, `u32` `TILEMETA_FORMAT_VERSION`, `u32` record count.
pub const TILEMETA_ITEM_LIGHT_MAGIC: [u8; 4] = *b"TMIL";
/// Size of the `TILEMETA_ITEM_LIGHT_ENTRY_PATH` header.
const TILEMETA_ITEM_LIGHT_HEADER_SIZE: usize = TILEMETA_ITEM_LIGHT_MAGIC.len() + 2 * std::mem::size_of::<u32>();

/// The item is not at its block's primary position (`uocf::enhanced::tileart::primary_texture_position`).
pub const TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY: u8 = 1 << 0;
/// Block 0: the item `ec_texture_id` was taken from (its primary position). Blocks 1 and 2: an item
/// naming the same texture id, that art set's counterpart of the primary image.
pub const TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED: u8 = 1 << 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialPhysicalPackage {
    #[default]
    Unknown = 0,
    Texture = 1,
    LegacyTexture = 2,
    TerrainTexture = 3,
    EffectTexture = 4,
    SystemTextures = 5,
    ShaderResources = 6,
}

impl EcMaterialPhysicalPackage {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Texture,
            2 => Self::LegacyTexture,
            3 => Self::TerrainTexture,
            4 => Self::EffectTexture,
            5 => Self::SystemTextures,
            6 => Self::ShaderResources,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialLogicalFamily {
    #[default]
    Unknown = 0,
    WorldArt = 1,
    TileArtLegacy = 2,
    TileArtEnhanced = 3,
    Textures = 4,
    Effects = 5,
    SystemTextures = 6,
    ShaderResources = 7,
}

impl EcMaterialLogicalFamily {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::WorldArt,
            2 => Self::TileArtLegacy,
            3 => Self::TileArtEnhanced,
            4 => Self::Textures,
            5 => Self::Effects,
            6 => Self::SystemTextures,
            7 => Self::ShaderResources,
            _ => Self::Unknown,
        }
    }
}

/// Version of the whole tilemeta package, in the `TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH` header. A package of another
/// version is rejected with a repack message.
pub const TILEMETA_FORMAT_VERSION: u32 = 1;

/// `TileMetaItemTile::ec_art_flags`: the tileart record sets `UseNewArt` in `flags2` (bit 19), so the
/// official Enhanced Client draws its block-0 KR art; without it the client draws classic-style art.
///
/// Informational only: the renderer deliberately ignores it. The flag is off for almost every item,
/// including items whose KR art the client still packs, so the client disabled that art on purpose.
/// Enhanced mode draws KR art wherever `tex_art_ec` has a slot, then the Classic art, then magenta.
pub const TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART: u16 = 1 << 0;

/// Role of a tileart texture item: the shader input it feeds.
///
/// The Enhanced Client binds item i of a block to shader map i (`SetShaderMap(i)`), and each UO
/// shader class reads its maps by fixed index. The role is therefore a function of the block, its
/// shader name and the item position (`for_texture_position`); the client never reads a role from a
/// file name. See `scripts/dev-helpers/ec-research/ghidra-texture-roles/results/uosa_texture_roles.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialStableRole {
    /// The block's shader name is not one of the UO shaders, so no input table applies.
    #[default]
    Unknown = 0,
    /// The colour image the material shows: sprite map 0, static terrain map 0 (RGB A), water map 1
    /// (the surface the client reads as the primary texture).
    Base = 1,
    /// Static terrain map 1: RGB B, blended with RGB A by the blend alpha.
    SecondaryBase = 2,
    /// Static terrain map 2: its alpha mixes RGB B toward RGB A.
    BlendAlpha = 3,
    /// Static terrain map 3: coverage alpha.
    CoverageAlpha = 4,
    /// Sprite map 1 (multiplied in at FFP stage 1; unused by the HLSL sprite shader), and map 1 of the
    /// light block.
    Noise = 5,
    /// Sprite map 2: its alpha blends the colour toward the hue (`HueMaskSampler`).
    HueMask = 6,
    /// Water map 0: the bump source (`D3DTOP_BUMPENVMAP`).
    WaterBump = 7,
    /// Water map 2: the surface sampled with reflection-vector texgen.
    WaterReflection = 8,
    /// Water map 3: the optional fourth stage, composed by `Technique[1]`.
    WaterFourthStage = 9,
    /// Light block (block 3) map 0: the light sprite texture.
    Light = 10,
    /// A position the block's shader never reads (sprite map 3 and later, or past a shader's last map).
    Unused = 11,
}

/// Role of each shader map a UO shader reads, indexed by item position.
const SPRITE_ROLES: [EcMaterialStableRole; 3] =
    [EcMaterialStableRole::Base, EcMaterialStableRole::Noise, EcMaterialStableRole::HueMask];
const WATER_ROLES: [EcMaterialStableRole; 4] = [
    EcMaterialStableRole::WaterBump, EcMaterialStableRole::Base,
    EcMaterialStableRole::WaterReflection, EcMaterialStableRole::WaterFourthStage,
];
const STATIC_TERRAIN_ROLES: [EcMaterialStableRole; 4] = [
    EcMaterialStableRole::Base, EcMaterialStableRole::SecondaryBase,
    EcMaterialStableRole::BlendAlpha, EcMaterialStableRole::CoverageAlpha,
];
const LIGHT_ROLES: [EcMaterialStableRole; 2] = [EcMaterialStableRole::Light, EcMaterialStableRole::Noise];

impl EcMaterialStableRole {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Base,
            2 => Self::SecondaryBase,
            3 => Self::BlendAlpha,
            4 => Self::CoverageAlpha,
            5 => Self::Noise,
            6 => Self::HueMask,
            7 => Self::WaterBump,
            8 => Self::WaterReflection,
            9 => Self::WaterFourthStage,
            10 => Self::Light,
            11 => Self::Unused,
            _ => Self::Unknown,
        }
    }

    /// Role of the item at `item_index` of block `block_index`, whose shader is `shader_name`.
    ///
    /// Block 3 is the light texture block whatever its shader name. An unknown shader name gives
    /// `Unknown`; a position past the shader's last map gives `Unused`.
    pub fn for_texture_position(block_index: u8, shader_name: Option<&str>, item_index: u8) -> Self {
        use uocf::enhanced::tileart::{LIGHT_TEXTURE_BLOCK_INDEX, SPRITE_SHADER_NAME, STATIC_TERRAIN_SHADER_NAME, WATER_SHADER_NAME};
        let roles: &[Self] = if block_index == LIGHT_TEXTURE_BLOCK_INDEX {
            &LIGHT_ROLES
        } else {
            match shader_name {
                Some(SPRITE_SHADER_NAME) => &SPRITE_ROLES,
                Some(WATER_SHADER_NAME) => &WATER_ROLES,
                Some(STATIC_TERRAIN_SHADER_NAME) => &STATIC_TERRAIN_ROLES,
                _ => return Self::Unknown,
            }
        };
        roles.get(item_index as usize).copied().unwrap_or(Self::Unused)
    }

    /// The material's colour images: `Base` and `SecondaryBase`.
    pub fn is_colour(self) -> bool {
        matches!(self, Self::Base | Self::SecondaryBase)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Base => "base",
            Self::SecondaryBase => "secondary_base",
            Self::BlendAlpha => "blend_alpha",
            Self::CoverageAlpha => "coverage_alpha",
            Self::Noise => "noise",
            Self::HueMask => "hue_mask",
            Self::WaterBump => "water_bump",
            Self::WaterReflection => "water_reflection",
            Self::WaterFourthStage => "water_fourth_stage",
            Self::Light => "light",
            Self::Unused => "unused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileMetaMainEcTextureReason {
    RoleBaseExactTileId,
    RoleBasePrimarySelected,
    RoleBaseFirst,
    RoleSecondaryBasePrimarySelected,
    RoleSecondaryBaseFirst,
    ExactWorldArtTileId,
    PrimarySelectedWorldArt,
    FirstWorldArt,
    LegacyEcTextureIdFallback,
}

impl TileMetaMainEcTextureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoleBaseExactTileId => "role_base_exact_tile_id",
            Self::RoleBasePrimarySelected => "role_base_primary_selected",
            Self::RoleBaseFirst => "role_base_first",
            Self::RoleSecondaryBasePrimarySelected => "role_secondary_base_primary_selected",
            Self::RoleSecondaryBaseFirst => "role_secondary_base_first",
            Self::ExactWorldArtTileId => "exact_worldart_tile_id",
            Self::PrimarySelectedWorldArt => "primary_selected_worldart",
            Self::FirstWorldArt => "first_worldart",
            Self::LegacyEcTextureIdFallback => "legacy_ec_texture_id_fallback",
        }
    }
}

/// Reason produced by the stable-role-aware base texture chooser.
///
/// Priority order (highest first):
/// 1. `RoleBase`              — `stable_role == Base` && primary-selected
/// 2. `RoleBaseFirst`         — `stable_role == Base`, any (first found)
/// 3. `RoleSecondaryBase`     — `stable_role == SecondaryBase`, primary-selected
/// 4. `FallbackWorldArtPrimary` — no Base/SecondaryBase; WorldArt + primary-selected
/// 5. `FallbackWorldArtFirst`   — no Base/SecondaryBase; WorldArt, first non-aux
/// 6. `FallbackAnyPrimary`      — primary-selected, any non-aux ref
/// 7. `FallbackAnyFirst`        — first non-aux ref
/// 8. `FallbackLegacyEcTextureId` — used `item.ec_texture_id`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainEcTextureRoleReason {
    RoleBase,
    RoleBaseFirst,
    RoleSecondaryBase,
    FallbackWorldArtPrimary,
    FallbackWorldArtFirst,
    FallbackAnyPrimary,
    FallbackAnyFirst,
    FallbackLegacyEcTextureId,
}

impl MainEcTextureRoleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoleBase => "role_base",
            Self::RoleBaseFirst => "role_base_first",
            Self::RoleSecondaryBase => "role_secondary_base",
            Self::FallbackWorldArtPrimary => "fallback_worldart_primary",
            Self::FallbackWorldArtFirst => "fallback_worldart_first",
            Self::FallbackAnyPrimary => "fallback_any_primary",
            Self::FallbackAnyFirst => "fallback_any_first",
            Self::FallbackLegacyEcTextureId => "fallback_legacy_ec_texture_id",
        }
    }

    pub fn is_low_confidence(self) -> bool {
        matches!(
            self,
            Self::FallbackWorldArtFirst
                | Self::FallbackAnyPrimary
                | Self::FallbackAnyFirst
                | Self::FallbackLegacyEcTextureId
        )
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaLandTile {
    pub tile_id: u32,
    pub texture_id: u16,
    pub tile_type: u8,
    pub _pad1: u8,
    pub flags: u64,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],
}

impl TileMetaLandTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaItemTile {
    pub tile_id: u32,
    pub weight: u8,
    pub quality: u8,
    pub quantity: u8,
    pub hue_extra: u8,
    pub flags: u64,
    pub anim_id: u16,
    pub stacking_offset: u8,
    pub value: u8,
    pub height: i8,
    pub _pad1: u8,
    /// `TILEMETA_ITEM_EC_ART_FLAG_*` bits from the tileart record. Informational client data: the
    /// renderer does not read them (see `TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART`).
    pub ec_art_flags: u16,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],

    pub ec_texture_id: u32,
    pub ec_start_x: i16,
    pub ec_start_y: i16,

    pub cc_texture_id: u32,
    pub cc_start_x: i16,
    pub cc_start_y: i16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TileMetaItemVisualKind {
    #[default]
    RegularArt = 0,
    SurfaceLike = 1,
}

impl TileMetaItemTile {
    pub fn is_nodraw(&self) -> bool {
        self.flags & uocf::enhanced::tileart::TaeFlag::NoDraw.bits() != 0
    }

    /// The IsWet tiledata/tileart flag: an object property (water, swamp, wave, sink ...) for gameplay
    /// and future client logic. It does not choose the Enhanced render path: there an item with an
    /// EC material renders wet only through its shader (`TileMetaPackage::uses_water_shader`).
    pub fn is_wet(&self) -> bool {
        self.flags & uocf::enhanced::tileart::TaeFlag::Wet.bits() != 0
    }

    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }

    pub fn visual_kind(&self) -> TileMetaItemVisualKind {
        match self._pad1 {
            1 => TileMetaItemVisualKind::SurfaceLike,
            _ => TileMetaItemVisualKind::RegularArt,
        }
    }

    pub fn is_surface_like(&self) -> bool {
        self.visual_kind() == TileMetaItemVisualKind::SurfaceLike
    }

    /// The tileart record sets `UseNewArt`: the official Enhanced Client draws this item's block-0
    /// KR art (`tex_art_ec`), otherwise its classic-style art. The renderer deliberately ignores it
    /// (see `TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART`).
    pub fn uses_new_art(&self) -> bool {
        self.ec_art_flags & TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART != 0
    }

    pub fn set_visual_kind(&mut self, kind: TileMetaItemVisualKind) {
        self._pad1 = kind as u8;
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRefSpan {
    pub start: u32,
    pub len: u16,
    pub _pad: u16,
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRef {
    pub texture_id: u32,
    pub texture_type: u8,
    pub block_index: u8,
    pub item_index: u8,
    pub flags: u8,
    pub texture_stretch: f32,
    /// Tileart texture item load path selector (medium confidence).
    pub source_kind: u8,
    pub physical_package: u8,
    /// `EcMaterialStableRole`, from the block shader and `item_index`.
    pub stable_role: u8,
    /// Reserved, 0.
    pub _reserved: u8,
    /// Stored by the client but never read: it binds item i to shader map i. Use `item_index`.
    pub sampler_index: u32,
    /// Tileart texture item UV set (medium confidence).
    pub uv_set: u32,
}

/// On-disk size of one `item_texture_refs.bin` record, after the header.
const TILEMETA_ITEM_TEXTURE_REF_SIZE: usize = 24;

// The record stays packed: 24 bytes with no padding.
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<TileMetaItemTextureRef>() == TILEMETA_ITEM_TEXTURE_REF_SIZE);
    assert!(offset_of!(TileMetaItemTextureRef, source_kind)
        == offset_of!(TileMetaItemTextureRef, texture_stretch) + size_of::<f32>());
    assert!(offset_of!(TileMetaItemTextureRef, sampler_index)
        == offset_of!(TileMetaItemTextureRef, _reserved) + size_of::<u8>());
    assert!(offset_of!(TileMetaItemTextureRef, uv_set)
        == offset_of!(TileMetaItemTextureRef, sampler_index) + size_of::<u32>());
    assert!(offset_of!(TileMetaItemTextureRef, uv_set) + size_of::<u32>() == TILEMETA_ITEM_TEXTURE_REF_SIZE);
};

/// The `TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH` entry: the header with `TILEMETA_FORMAT_VERSION`, then the records.
pub fn encode_item_texture_refs(refs: &[TileMetaItemTextureRef]) -> Vec<u8> {
    let records: &[u8] = bytemuck::cast_slice(refs);
    let mut bytes = Vec::with_capacity(TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE + records.len());
    bytes.extend_from_slice(&TILEMETA_ITEM_TEXTURE_REF_MAGIC);
    bytes.extend_from_slice(&TILEMETA_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(records);
    bytes
}

/// Reads the `TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH` entry, refusing another magic or version.
fn decode_item_texture_refs(bytes: &[u8]) -> eyre::Result<Vec<TileMetaItemTextureRef>> {
    let path = TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH;
    eyre::ensure!(bytes.len() >= TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE && bytes.starts_with(&TILEMETA_ITEM_TEXTURE_REF_MAGIC),
        "{path}: no tilemeta header; repack with the current udd-cli");
    let (header, records) = bytes.split_at(TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE);
    let version = u32::from_le_bytes(header[TILEMETA_ITEM_TEXTURE_REF_MAGIC.len()..].try_into()?);
    ensure_manifest_version(path, version, TILEMETA_FORMAT_VERSION)?;
    read_pod_vec(records, path)
}

/// The light sprite of an item, as the Enhanced and KR clients place it (`UOObjectComponentLighting`).
///
/// One record per item whose tileart shader block 3 (`uocf::enhanced::tileart::LIGHT_TEXTURE_BLOCK_INDEX`) names a
/// texture at map 0; the client creates the light from that name alone, not from the LightSource flag.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct TileMetaItemLight {
    /// Item id, as `TileMetaItemTile::tile_id`.
    pub tile_id: u32,
    /// `TerrainTexture.uop` entry of the light texture (`build/terraintexture/%08d.dds`); its `tex_aux_ec` key is
    /// `tex_aux_ec_terrain_texture_key(entry)`.
    pub light_texture_entry: u32,
    /// Tileart `light_scale`: texture pixels to sprite pixels.
    pub scale: f32,
    /// Tileart `light_offset_x`: world units along camera right, added after the camera rotation.
    pub offset_x: f32,
    /// Tileart `light_offset_y`: world units along camera up, added after the camera rotation.
    pub offset_y: f32,
}

pub const TILEMETA_ITEM_LIGHT_SIZE: usize = 20;
const _: () = assert!(std::mem::size_of::<TileMetaItemLight>() == TILEMETA_ITEM_LIGHT_SIZE);

/// The `TILEMETA_ITEM_LIGHT_ENTRY_PATH` entry: the header, then `lights`, which must be sorted by unique `tile_id`.
pub fn encode_item_lights(lights: &[TileMetaItemLight]) -> Vec<u8> {
    let records: &[u8] = bytemuck::cast_slice(lights);
    let mut bytes = Vec::with_capacity(TILEMETA_ITEM_LIGHT_HEADER_SIZE + records.len());
    bytes.extend_from_slice(&TILEMETA_ITEM_LIGHT_MAGIC);
    bytes.extend_from_slice(&TILEMETA_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(lights.len()).expect("item light count fits u32").to_le_bytes());
    bytes.extend_from_slice(records);
    bytes
}

/// Reads the `TILEMETA_ITEM_LIGHT_ENTRY_PATH` entry, refusing another magic, version or count, or unsorted records.
pub fn decode_item_lights(bytes: &[u8]) -> eyre::Result<Vec<TileMetaItemLight>> {
    let path = TILEMETA_ITEM_LIGHT_ENTRY_PATH;
    eyre::ensure!(bytes.len() >= TILEMETA_ITEM_LIGHT_HEADER_SIZE && bytes.starts_with(&TILEMETA_ITEM_LIGHT_MAGIC),
        "{path}: no TMIL header; repack with the current udd-cli");
    let word = |index: usize| {
        let start = TILEMETA_ITEM_LIGHT_MAGIC.len() + index * std::mem::size_of::<u32>();
        u32::from_le_bytes(bytes[start..start + std::mem::size_of::<u32>()].try_into().expect("slice length checked"))
    };
    ensure_manifest_version(path, word(0), TILEMETA_FORMAT_VERSION)?;
    let lights: Vec<TileMetaItemLight> = read_pod_vec(&bytes[TILEMETA_ITEM_LIGHT_HEADER_SIZE..], path)?;
    eyre::ensure!(lights.len() == word(1) as usize, "{path}: {} records, the header says {}", lights.len(), word(1));
    eyre::ensure!(lights.windows(2).all(|pair| pair[0].tile_id < pair[1].tile_id), "{path}: not sorted by unique tile id");
    Ok(lights)
}

impl TileMetaItemTextureRef {
    pub fn is_auxiliary(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY != 0
    }

    pub fn is_primary_selected(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED != 0
    }

    pub fn is_world_art(&self) -> bool {
        self.texture_type == 1
    }

    pub fn logical_family(&self) -> EcMaterialLogicalFamily {
        EcMaterialLogicalFamily::from_u8(self.texture_type)
    }

    pub fn physical_package(&self) -> EcMaterialPhysicalPackage {
        EcMaterialPhysicalPackage::from_u8(self.physical_package)
    }

    pub fn stable_role(&self) -> EcMaterialStableRole {
        EcMaterialStableRole::from_u8(self.stable_role)
    }
}

pub struct TileMetaPackage {
    #[allow(dead_code)]
    package: UddpReader,
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    item_texture_ref_spans: Vec<TileMetaItemTextureRefSpan>,
    item_texture_refs: Vec<TileMetaItemTextureRef>,
    item_materials: std::collections::BTreeMap<u32, Vec<uocf::enhanced::tileart::ArtMaterial>>,
    item_definitions: TileMetaItemDefinitions,
    item_lights: Vec<TileMetaItemLight>,
}

impl TileMetaPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let land_bytes = read_path_entry_cow(&package, TILEMETA_LAND_ENTRY_PATH)?;
        let item_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_ENTRY_PATH)?;
        let item_texture_ref_span_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH)
            .wrap_err("tilemeta missing item texture refs; repack with the current udd-cli")?;
        let item_texture_ref_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH)
            .wrap_err("tilemeta missing item texture refs; repack with the current udd-cli")?;
        let item_texture_ref_spans =
            read_pod_vec(item_texture_ref_span_bytes.as_ref(), TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH)?;
        let item_texture_refs = decode_item_texture_refs(item_texture_ref_bytes.as_ref())?;
        let land_tiles = read_pod_vec(land_bytes.as_ref(), TILEMETA_LAND_ENTRY_PATH)?;
        let item_tiles = read_pod_vec(item_bytes.as_ref(), TILEMETA_ITEM_ENTRY_PATH)?;
        let material_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_MATERIAL_ENTRY_PATH)
            .wrap_err("tilemeta missing source item materials; repack with the current udd-cli")?;
        let item_materials = serde_json::from_slice(material_bytes.as_ref())
            .wrap_err("parse source item material metadata")?;
        let definition_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_DEFINITION_ENTRY_PATH)?;
        let item_definitions =
            TileMetaItemDefinitions::from_bytes(definition_bytes.as_ref(), TILEMETA_ITEM_DEFINITION_ENTRY_PATH)?;
        let light_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_LIGHT_ENTRY_PATH)
            .wrap_err("tilemeta missing item lights; repack with the current udd-cli")?;
        let item_lights = decode_item_lights(light_bytes.as_ref())?;

        Ok(Self {
            package,
            land_tiles,
            item_tiles,
            item_texture_ref_spans,
            item_texture_refs,
            item_materials,
            item_definitions,
            item_lights,
        })
    }

    /// The light sprite of `tile_id` in the Enhanced and KR clients, if its tileart names a light texture.
    pub fn item_light(&self, tile_id: u32) -> Option<&TileMetaItemLight> {
        self.item_lights.binary_search_by_key(&tile_id, |light| light.tile_id).ok().map(|index| &self.item_lights[index])
    }

    /// Every item light, sorted by `tile_id`.
    pub fn item_lights(&self) -> &[TileMetaItemLight] {
        &self.item_lights
    }

    /// The `tileart.uop` records the Enhanced paperdoll reads: the lookup of `uocf::enhanced::paperdoll` runs on
    /// `&TileMetaItemDefinitions`.
    pub fn item_definitions(&self) -> &TileMetaItemDefinitions {
        &self.item_definitions
    }

    pub fn land_tiles(&self) -> &[TileMetaLandTile] {
        &self.land_tiles
    }

    pub fn item_tiles(&self) -> &[TileMetaItemTile] {
        &self.item_tiles
    }

    pub fn item_material(&self, tile_id: u32, block_index: u8) -> Option<&uocf::enhanced::tileart::ArtMaterial> {
        self.item_materials(tile_id).iter()
            .find(|material| material.block_index == block_index)
    }

    pub fn item_materials(&self, tile_id: u32) -> &[uocf::enhanced::tileart::ArtMaterial] {
        self.item_materials.get(&tile_id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn land_tile(&self, tile_id: u32) -> Option<&TileMetaLandTile> {
        self.land_tiles.get(tile_id as usize)
    }

    pub fn item_tile(&self, tile_id: u32) -> Option<&TileMetaItemTile> {
        self.item_tiles.get(tile_id as usize)
    }

    /// Whether the official Enhanced Client draws the KR art (`tex_art_ec`) of `art_id`
    /// (`TileMetaItemTile::uses_new_art`); false for an id outside the item table. Informational:
    /// the renderer deliberately ignores it (see `TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART`).
    pub fn uses_new_art(&self, art_id: u32) -> bool {
        self.item_tile(art_id).is_some_and(TileMetaItemTile::uses_new_art)
    }

    /// Whether `art_id` has a material in the EC block (`TILEMETA_EC_TEXTURE_BLOCK_INDEX`), i.e. the
    /// Enhanced Client draws it with its own shader rather than the classic-style art.
    pub fn has_ec_material(&self, art_id: u32) -> bool {
        self.item_material(art_id, TILEMETA_EC_TEXTURE_BLOCK_INDEX).is_some()
    }

    /// Whether the EC block (`TILEMETA_EC_TEXTURE_BLOCK_INDEX`) material of `art_id` uses
    /// `UOWaterShader`: the item is Enhanced Client water (or lava), so the EC renderer treats it as wet.
    /// False when the item has no stored EC block material.
    pub fn uses_water_shader(&self, art_id: u32) -> bool {
        self.item_material(art_id, TILEMETA_EC_TEXTURE_BLOCK_INDEX)
            .is_some_and(|material| material.shader_name.as_deref() == Some(uocf::enhanced::tileart::WATER_SHADER_NAME))
    }

    pub fn item_texture_refs(&self, tile_id: u32) -> &[TileMetaItemTextureRef] {
        let Some(span) = self.item_texture_ref_spans.get(tile_id as usize) else {
            return &self.item_texture_refs[0..0];
        };

        let start = span.start as usize;
        let end = start.saturating_add(span.len as usize);
        if start > self.item_texture_refs.len() || end > self.item_texture_refs.len() {
            return &self.item_texture_refs[0..0];
        }

        &self.item_texture_refs[start..end]
    }

    pub fn main_ec_texture_ref(&self, tile_id: u32) -> Option<&TileMetaItemTextureRef> {
        self.main_ec_texture_ref_with_reason(tile_id)
            .map(|(texture_ref, _)| texture_ref)
    }

    pub fn main_ec_texture_ref_with_reason(
        &self,
        tile_id: u32,
    ) -> Option<(&TileMetaItemTextureRef, TileMetaMainEcTextureReason)> {
        let allow_exact_tile_id = self
            .item_tile(tile_id)
            .is_none_or(|item| !item.is_surface_like());
        choose_main_ec_texture_ref(self.item_texture_refs(tile_id), tile_id, allow_exact_tile_id)
    }

    pub fn main_ec_texture_id(&self, tile_id: u32) -> Option<u32> {
        self.main_ec_texture_id_with_reason(tile_id)
            .map(|(texture_id, _)| texture_id)
    }

    pub fn main_ec_texture_id_with_reason(
        &self,
        tile_id: u32,
    ) -> Option<(u32, TileMetaMainEcTextureReason)> {
        self.main_ec_texture_ref_with_reason(tile_id)
            .map(|(texture_ref, reason)| (texture_ref.texture_id, reason))
            .or_else(|| {
                self.item_tile(tile_id)
                    .and_then(|item| (item.ec_texture_id != 0).then_some(item.ec_texture_id))
                    .map(|texture_id| {
                        (
                            texture_id,
                            TileMetaMainEcTextureReason::LegacyEcTextureIdFallback,
                        )
                    })
            })
    }

    /// Stable-role-aware base texture selector.
    ///
    /// Returns `None` only when there are no candidates at all.
    pub fn main_ec_texture_id_role_aware(
        &self,
        tile_id: u32,
    ) -> Option<(u32, EcMaterialStableRole, MainEcTextureRoleReason)> {
        choose_main_ec_texture_id_role_aware(
            self.item_texture_refs(tile_id),
            self.item_tile(tile_id),
        )
    }

    /// Returns `true` when `main_ec_texture_id_role_aware` would select a low-confidence fallback.
    pub fn main_ec_texture_selection_is_low_confidence(&self, tile_id: u32) -> bool {
        self.main_ec_texture_id_role_aware(tile_id)
            .is_some_and(|(_, _, reason)| reason.is_low_confidence())
    }

    pub fn main_ec_texture_id_with_branch(&self, tile_id: u32) -> Option<(u32, &'static str)> {
        self.main_ec_texture_id_role_aware(tile_id)
            .map(|(texture_id, _, reason)| (texture_id, reason.as_str()))
    }
}

/// Tileart block holding the Enhanced Client's own (KR) art; blocks 1 and 2 are the legacy and
/// enhanced art sets, whose position 0 is a `Base` image too.
pub const TILEMETA_EC_TEXTURE_BLOCK_INDEX: u8 = 0;
/// Tileart block of the legacy art: the KR client draws it for an item without KR art.
pub const TILEMETA_LEGACY_ART_BLOCK_INDEX: u8 = 1;
/// The KR client's grunge texture, `01000045_noise` (`build/worldart/01000045.dds`). The Enhanced Client ships another
/// image under this id.
pub const CLIENT_GRUNGE_TEXTURE_ID: u32 = 1_000_045;
/// The shader map the KR sprite shader samples as its grunge (`GrungeSampler` in `UOSprite.psh`): a ref's
/// `item_index`.
pub const CLIENT_GRUNGE_SHADER_MAP: u8 = 1;

impl TileMetaPackage {
    /// Whether the KR client multiplies `tile_id`'s sprite by its grunge texture: see
    /// [`item_refs_have_client_grunge`].
    pub fn item_has_client_grunge(&self, tile_id: u32) -> bool {
        item_refs_have_client_grunge(self.item_texture_refs(tile_id))
    }
}

/// Whether the tileart block the KR client draws an item from binds [`CLIENT_GRUNGE_TEXTURE_ID`] as its shader map
/// [`CLIENT_GRUNGE_SHADER_MAP`] (research P3.6). That block is the KR art's ([`TILEMETA_EC_TEXTURE_BLOCK_INDEX`]) when
/// the item has KR art, else the legacy art's. The light block (`LIGHT_TEXTURE_BLOCK_INDEX`) names the same texture
/// for the light sprite, which the client never grunges, so it is never read here.
pub fn item_refs_have_client_grunge(texture_refs: &[TileMetaItemTextureRef]) -> bool {
    let has_kr_art = texture_refs.iter().any(|texture_ref| texture_ref.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX);
    let drawn_block = if has_kr_art { TILEMETA_EC_TEXTURE_BLOCK_INDEX } else { TILEMETA_LEGACY_ART_BLOCK_INDEX };
    texture_refs.iter().any(|texture_ref| {
        texture_ref.block_index == drawn_block
            && texture_ref.item_index == CLIENT_GRUNGE_SHADER_MAP
            && texture_ref.texture_id == CLIENT_GRUNGE_TEXTURE_ID
    })
}

/// Refs of the EC block. The main-texture selectors read only these through their role branches:
/// every block has a `Base` image at its primary position.
fn ec_block_refs(texture_refs: &[TileMetaItemTextureRef]) -> impl Iterator<Item = &TileMetaItemTextureRef> + Clone {
    texture_refs.iter().filter(|texture_ref| texture_ref.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX)
}

fn choose_main_ec_texture_ref(
    texture_refs: &[TileMetaItemTextureRef],
    tile_id: u32,
    allow_exact_tile_id: bool,
) -> Option<(&TileMetaItemTextureRef, TileMetaMainEcTextureReason)> {
    let ec_refs = ec_block_refs(texture_refs);
    ec_refs.clone()
        .find(|texture_ref| {
            allow_exact_tile_id
                && !texture_ref.is_auxiliary()
                && texture_ref.stable_role() == EcMaterialStableRole::Base
                && texture_ref.texture_id == tile_id
        })
        .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::RoleBaseExactTileId))
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::Base
                        && texture_ref.is_primary_selected()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleBasePrimarySelected,
                    )
                })
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::Base
                })
                .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::RoleBaseFirst))
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::SecondaryBase
                        && texture_ref.is_primary_selected()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleSecondaryBasePrimarySelected,
                    )
                })
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::SecondaryBase
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleSecondaryBaseFirst,
                    )
                })
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    allow_exact_tile_id
                        && texture_ref.stable_role() == EcMaterialStableRole::Unknown
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_world_art()
                        && texture_ref.texture_id == tile_id
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::ExactWorldArtTileId,
                    )
                })
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    texture_ref.stable_role() == EcMaterialStableRole::Unknown
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_primary_selected()
                        && texture_ref.is_world_art()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::PrimarySelectedWorldArt,
                    )
                })
        })
        .or_else(|| {
            ec_refs.clone()
                .find(|texture_ref| {
                    texture_ref.stable_role() == EcMaterialStableRole::Unknown
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_world_art()
                })
                .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::FirstWorldArt))
        })
}

fn valid_ec_art_texture_identity(
    texture_ref: &TileMetaItemTextureRef,
    tile_id: u32,
    allow_exact_tile_id: bool,
) -> bool {
    allow_exact_tile_id || texture_ref.texture_id != tile_id
}

fn choose_main_ec_texture_id_role_aware(
    texture_refs: &[TileMetaItemTextureRef],
    item_tile: Option<&TileMetaItemTile>,
) -> Option<(u32, EcMaterialStableRole, MainEcTextureRoleReason)> {
    // Steps 1-5 read roles, so only the EC block; steps 6-7 take any block.
    let ec_refs = ec_block_refs(texture_refs);
    // 1. Base + primary-selected
    if let Some(r) = ec_refs.clone().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::Base
            && r.is_primary_selected()
    }) {
        return Some((r.texture_id, EcMaterialStableRole::Base, MainEcTextureRoleReason::RoleBase));
    }

    // 2. Base, any (first found)
    if let Some(r) = ec_refs.clone().find(|r| {
        !r.is_auxiliary() && r.stable_role() == EcMaterialStableRole::Base
    }) {
        return Some((r.texture_id, EcMaterialStableRole::Base, MainEcTextureRoleReason::RoleBaseFirst));
    }

    // 3. SecondaryBase + primary-selected
    if let Some(r) = ec_refs.clone().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::SecondaryBase
            && r.is_primary_selected()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::SecondaryBase,
            MainEcTextureRoleReason::RoleSecondaryBase,
        ));
    }

    // 4. WorldArt + primary-selected (Unknown only)
    if let Some(r) = ec_refs.clone().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::Unknown
            && r.is_world_art()
            && r.is_primary_selected()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::Unknown,
            MainEcTextureRoleReason::FallbackWorldArtPrimary,
        ));
    }

    // 5. WorldArt, first non-aux (Unknown only)
    if let Some(r) = ec_refs.clone().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::Unknown
            && r.is_world_art()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::Unknown,
            MainEcTextureRoleReason::FallbackWorldArtFirst,
        ));
    }

    // 6. Any primary-selected non-aux
    if let Some(r) = texture_refs
        .iter()
        .find(|r| !r.is_auxiliary() && r.is_primary_selected())
    {
        return Some((
            r.texture_id,
            r.stable_role(),
            MainEcTextureRoleReason::FallbackAnyPrimary,
        ));
    }

    // 7. First non-aux
    if let Some(r) = texture_refs.iter().find(|r| !r.is_auxiliary()) {
        return Some((
            r.texture_id,
            r.stable_role(),
            MainEcTextureRoleReason::FallbackAnyFirst,
        ));
    }

    // 8. Legacy item.ec_texture_id
    if let Some(item) = item_tile {
        if item.ec_texture_id != 0 {
            return Some((
                item.ec_texture_id,
                EcMaterialStableRole::Unknown,
                MainEcTextureRoleReason::FallbackLegacyEcTextureId,
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `item_definitions.bin` of no record: its count, 0.
    const EMPTY_DEFINITIONS: [u8; 4] = [0; 4];

    #[test]
    fn source_item_material_metadata_is_required_and_validated() {
        let no_refs = encode_item_texture_refs(&[]);
        let no_lights = encode_item_lights(&[]);
        for source in [None, Some(b"invalid JSON".as_slice())] {
            let mut builder = udd_container::UddpBuilder::new(udd_container::LookupMode::VirtualPathHash);
            let entries = [
                (TILEMETA_LAND_ENTRY_PATH, Some(b"".as_slice())),
                (TILEMETA_ITEM_ENTRY_PATH, Some(b"".as_slice())),
                (TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, Some(b"".as_slice())),
                (TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, Some(no_refs.as_slice())),
                (TILEMETA_ITEM_MATERIAL_ENTRY_PATH, source),
                (TILEMETA_ITEM_DEFINITION_ENTRY_PATH, Some(EMPTY_DEFINITIONS.as_slice())),
                (TILEMETA_ITEM_LIGHT_ENTRY_PATH, Some(no_lights.as_slice())),
            ];
            for (path, data) in entries {
                if let Some(data) = data {
                    builder.add_file(udd_container::AddFileRequest {
                        data_type: udd_container::DataType::Metadata as u8,
                        compression: udd_container::CompressionFlag::ZstdNoDict,
                        width: 0, height: 0, virtual_path: Some(path), path_hash64: None,
                        id: None, data,
                    }).expect("add test metadata");
                }
            }
            let result = TileMetaPackage::from_uddp_package(
                UddpReader::open(builder.build().expect("build test metadata")).expect("open test metadata"),
            );
            let message = result.err().expect("reject incomplete material metadata").to_string();
            assert!(message.contains(if source.is_none() { "repack" } else { "parse source item material" }));
        }
    }

    fn package_with_refs(refs: &[u8]) -> eyre::Result<TileMetaPackage> {
        package_with_entries(refs, Some(&encode_item_lights(&[])))
    }

    fn package_with_entries(refs: &[u8], lights: Option<&[u8]>) -> eyre::Result<TileMetaPackage> {
        let mut builder = udd_container::UddpBuilder::new(udd_container::LookupMode::VirtualPathHash);
        let entries = [
            (TILEMETA_LAND_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, refs),
            (TILEMETA_ITEM_MATERIAL_ENTRY_PATH, b"{}".as_slice()),
            (TILEMETA_ITEM_DEFINITION_ENTRY_PATH, EMPTY_DEFINITIONS.as_slice()),
        ];
        for (path, data) in entries.into_iter().chain(lights.map(|lights| (TILEMETA_ITEM_LIGHT_ENTRY_PATH, lights))) {
            builder.add_file(udd_container::AddFileRequest {
                data_type: udd_container::DataType::Metadata as u8,
                compression: udd_container::CompressionFlag::ZstdNoDict,
                width: 0, height: 0, virtual_path: Some(path), path_hash64: None, id: None, data,
            }).expect("add test metadata");
        }
        TileMetaPackage::from_uddp_package(UddpReader::open(builder.build().expect("build test metadata")).expect("open test metadata"))
    }

    #[test]
    fn only_the_current_format_version_loads() {
        let refs = [TileMetaItemTextureRef { texture_id: 7, ..Default::default() }];
        let current = encode_item_texture_refs(&refs);
        let loaded = package_with_refs(&current).expect("the current version loads").item_texture_refs;
        assert_eq!(loaded.iter().map(|r| r.texture_id).collect::<Vec<_>>(), [refs[0].texture_id]);
        let mut other_version = current.clone();
        other_version[TILEMETA_ITEM_TEXTURE_REF_MAGIC.len()..TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE]
            .copy_from_slice(&(TILEMETA_FORMAT_VERSION + 1).to_le_bytes());
        let headerless = current[TILEMETA_ITEM_TEXTURE_REF_HEADER_SIZE..].to_vec();
        for bytes in [other_version, headerless] {
            let message = package_with_refs(&bytes).err().expect("reject another tilemeta").to_string();
            assert!(message.contains("repack with the current udd-cli"), "{message}");
        }
    }

    /// Lantern 2594 as the EC tileart carries it, and a later item with the south window light values.
    fn sample_item_lights() -> [TileMetaItemLight; 2] {
        [
            TileMetaItemLight { tile_id: 2594, light_texture_entry: 1, scale: 2.0, offset_x: 0.0, offset_y: -4.5 },
            TileMetaItemLight { tile_id: 2850, light_texture_entry: 3, scale: 1.0, offset_x: -1.35, offset_y: -3.0 },
        ]
    }

    #[test]
    fn item_lights_round_trip_and_are_found_by_tile_id() {
        let lights = sample_item_lights();
        let bytes = encode_item_lights(&lights);
        assert_eq!(&bytes[..4], b"TMIL");
        assert_eq!(bytes.len(), TILEMETA_ITEM_LIGHT_HEADER_SIZE + lights.len() * TILEMETA_ITEM_LIGHT_SIZE);
        // The record is five little-endian words: tile id, texture entry, scale, offset x, offset y.
        let first = &bytes[TILEMETA_ITEM_LIGHT_HEADER_SIZE..][..TILEMETA_ITEM_LIGHT_SIZE];
        assert_eq!(first[..8], [0x22, 0x0A, 0, 0, 1, 0, 0, 0]);
        assert_eq!(f32::from_le_bytes(first[8..12].try_into().unwrap()), 2.0);
        assert_eq!(f32::from_le_bytes(first[16..20].try_into().unwrap()), -4.5);
        assert_eq!(decode_item_lights(&bytes).expect("decode"), lights);

        let package = package_with_entries(&encode_item_texture_refs(&[]), Some(&bytes)).expect("package");
        assert_eq!(package.item_light(2594), Some(&lights[0]));
        assert_eq!(package.item_light(2850).map(|light| light.light_texture_entry), Some(3));
        assert_eq!(package.item_light(2595), None);
        assert_eq!(package.item_lights(), &lights);
    }

    #[test]
    fn item_lights_are_required_at_the_current_version() {
        let refs = encode_item_texture_refs(&[]);
        let lights = sample_item_lights();
        let current = encode_item_lights(&lights);
        let mut other_version = current.clone();
        other_version[4..8].copy_from_slice(&(TILEMETA_FORMAT_VERSION + 1).to_le_bytes());
        let mut wrong_count = current.clone();
        wrong_count[8..12].copy_from_slice(&3u32.to_le_bytes());
        let unsorted = encode_item_lights(&[lights[1], lights[0]]);
        let message = |lights: Option<&[u8]>| package_with_entries(&refs, lights).err().expect("rejected").to_string();
        assert!(message(None).contains("repack with the current udd-cli"));
        assert!(message(Some(&other_version)).contains("repack with the current udd-cli"));
        assert!(message(Some(&current[TILEMETA_ITEM_LIGHT_HEADER_SIZE..])).contains("repack with the current udd-cli"));
        assert!(message(Some(&wrong_count)).contains("the header says 3"));
        assert!(message(Some(&unsorted)).contains("not sorted"));
    }

    #[test]
    fn water_shader_items_are_read_from_the_ec_block_material() {
        use uocf::enhanced::tileart::{ArtMaterial, SPRITE_SHADER_NAME, WATER_SHADER_NAME};
        const EC_WATER: u32 = 1;
        const LEGACY_BLOCK_WATER: u32 = 2;
        const SPRITE: u32 = 3;
        const NO_MATERIAL: u32 = 4;
        const LEGACY_BLOCK: u8 = 1;
        let material = |block_index, shader: &str| ArtMaterial {
            block_index, shader_name: Some(shader.to_owned()), ..ArtMaterial::default() };
        let mut package = package_with_refs(&encode_item_texture_refs(&[])).expect("test package");
        package.item_materials.insert(EC_WATER, vec![material(TILEMETA_EC_TEXTURE_BLOCK_INDEX, WATER_SHADER_NAME)]);
        package.item_materials.insert(LEGACY_BLOCK_WATER, vec![
            material(TILEMETA_EC_TEXTURE_BLOCK_INDEX, SPRITE_SHADER_NAME), material(LEGACY_BLOCK, WATER_SHADER_NAME)]);
        package.item_materials.insert(SPRITE, vec![material(TILEMETA_EC_TEXTURE_BLOCK_INDEX, SPRITE_SHADER_NAME)]);
        assert!(package.uses_water_shader(EC_WATER));
        for art_id in [LEGACY_BLOCK_WATER, SPRITE, NO_MATERIAL] {
            assert!(!package.uses_water_shader(art_id), "{art_id}");
        }
        for art_id in [EC_WATER, LEGACY_BLOCK_WATER, SPRITE] {
            assert!(package.has_ec_material(art_id), "{art_id}");
        }
        assert!(!package.has_ec_material(NO_MATERIAL));
    }

    #[test]
    fn roles_come_from_the_shader_and_the_item_position() {
        use uocf::enhanced::tileart::{LIGHT_TEXTURE_BLOCK_INDEX, SPRITE_SHADER_NAME, STATIC_TERRAIN_SHADER_NAME, WATER_SHADER_NAME};
        use EcMaterialStableRole as R;
        let roles = |block, shader, count| (0..count).map(|position| R::for_texture_position(block, shader, position)).collect::<Vec<_>>();
        assert_eq!(roles(0, Some(SPRITE_SHADER_NAME), 4), [R::Base, R::Noise, R::HueMask, R::Unused]);
        assert_eq!(roles(1, Some(SPRITE_SHADER_NAME), 4), [R::Base, R::Noise, R::HueMask, R::Unused]);
        assert_eq!(roles(0, Some(WATER_SHADER_NAME), 5),
            [R::WaterBump, R::Base, R::WaterReflection, R::WaterFourthStage, R::Unused]);
        assert_eq!(roles(0, Some(STATIC_TERRAIN_SHADER_NAME), 4), [R::Base, R::SecondaryBase, R::BlendAlpha, R::CoverageAlpha]);
        assert_eq!(roles(LIGHT_TEXTURE_BLOCK_INDEX, Some(SPRITE_SHADER_NAME), 3), [R::Light, R::Noise, R::Unused]);
        assert_eq!(roles(0, Some("SomeOtherShader"), 2), [R::Unknown, R::Unknown]);
        assert_eq!(roles(0, None, 1), [R::Unknown]);
        for value in 0..=R::Unused as u8 {
            assert_eq!(R::from_u8(value) as u8, value);
        }
        assert_eq!(std::mem::size_of::<TileMetaItemTextureRef>(), 24);
    }

    #[test]
    fn main_ec_texture_reason_freezes_exact_worldart_branch() {
        let texture_refs = [
            texture_ref(42, EcMaterialLogicalFamily::Textures, 0),
            texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, true).expect("chosen texture");

        assert_eq!(chosen.texture_id, 100);
        assert_eq!(reason, TileMetaMainEcTextureReason::ExactWorldArtTileId);
        assert_eq!(reason.as_str(), "exact_worldart_tile_id");
    }

    #[test]
    fn main_ec_texture_ignores_unknown_primary_non_worldart() {
        let texture_refs = [
            texture_ref(10, EcMaterialLogicalFamily::Textures, TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY),
            texture_ref(
                42,
                EcMaterialLogicalFamily::Textures,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, true).is_none());
    }

    #[test]
    fn main_ec_texture_prefers_base_role_over_exact_unknown_worldart() {
        let texture_refs = [
            texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::Textures,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, true).expect("chosen texture");

        assert_eq!(chosen.texture_id, 42);
        assert_eq!(reason, TileMetaMainEcTextureReason::RoleBasePrimarySelected);
    }

    #[test]
    fn main_ec_texture_disables_exact_id_for_surface_like_selection() {
        let texture_refs = [
            texture_ref_with_role(
                100,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                0,
            ),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, false).expect("chosen texture");

        assert_eq!(chosen.texture_id, 42);
        assert_eq!(reason, TileMetaMainEcTextureReason::RoleBasePrimarySelected);
    }

    #[test]
    fn main_ec_texture_disables_unknown_exact_id_for_surface_like_selection() {
        let texture_refs = [texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0)];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, false).is_none());
    }

    #[test]
    fn main_ec_texture_rejects_explicit_support_roles() {
        let texture_refs = [
            texture_ref_with_role(
                100,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::BlendAlpha,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                101,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::WaterBump,
                0,
            ),
        ];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, true).is_none());
    }

    fn texture_ref(
        texture_id: u32,
        family: EcMaterialLogicalFamily,
        flags: u8,
    ) -> TileMetaItemTextureRef {
        texture_ref_with_role(texture_id, family, EcMaterialStableRole::Unknown, flags)
    }

    fn texture_ref_with_role(
        texture_id: u32,
        family: EcMaterialLogicalFamily,
        role: EcMaterialStableRole,
        flags: u8,
    ) -> TileMetaItemTextureRef {
        TileMetaItemTextureRef {
            texture_id,
            texture_type: family as u8,
            stable_role: role as u8,
            flags,
            ..TileMetaItemTextureRef::zeroed()
        }
    }

    #[test]
    fn client_grunge_follows_the_block_the_kr_client_draws() {
        use uocf::enhanced::tileart::LIGHT_TEXTURE_BLOCK_INDEX;
        const OTHER_TEXTURE_ID: u32 = 2_000_000;
        let reference = |block_index: u8, item_index: u8, texture_id: u32| TileMetaItemTextureRef {
            texture_id,
            block_index,
            item_index,
            ..TileMetaItemTextureRef::zeroed()
        };
        let kr_base = reference(TILEMETA_EC_TEXTURE_BLOCK_INDEX, 0, OTHER_TEXTURE_ID);
        let kr_grunge = reference(TILEMETA_EC_TEXTURE_BLOCK_INDEX, CLIENT_GRUNGE_SHADER_MAP, CLIENT_GRUNGE_TEXTURE_ID);
        let legacy_grunge = reference(TILEMETA_LEGACY_ART_BLOCK_INDEX, CLIENT_GRUNGE_SHADER_MAP, CLIENT_GRUNGE_TEXTURE_ID);
        let light_grunge = reference(LIGHT_TEXTURE_BLOCK_INDEX, CLIENT_GRUNGE_SHADER_MAP, CLIENT_GRUNGE_TEXTURE_ID);
        let kr_other_map1 = reference(TILEMETA_EC_TEXTURE_BLOCK_INDEX, CLIENT_GRUNGE_SHADER_MAP, OTHER_TEXTURE_ID);
        let kr_grunge_map0 = reference(TILEMETA_EC_TEXTURE_BLOCK_INDEX, 0, CLIENT_GRUNGE_TEXTURE_ID);

        assert!(item_refs_have_client_grunge(&[kr_base, kr_grunge]));
        // KR art without a grunge map wins over a grunged legacy block: the client draws the KR art.
        assert!(!item_refs_have_client_grunge(&[kr_base, legacy_grunge]));
        // No KR art: the legacy block decides.
        assert!(item_refs_have_client_grunge(&[legacy_grunge]));
        // The light block is a light sprite, never grunged; another texture or shader map is no grunge.
        assert!(!item_refs_have_client_grunge(&[light_grunge]));
        assert!(!item_refs_have_client_grunge(&[kr_base, kr_other_map1]));
        assert!(!item_refs_have_client_grunge(&[kr_grunge_map0]));
        assert!(!item_refs_have_client_grunge(&[]));
    }

    // --- Role-aware selection tests ---

    #[test]
    fn selects_primary_base_first() {
        let texture_refs = [
            texture_ref_with_role(
                99,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                0,
            ),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::Textures,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                77,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Unknown,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (id, role, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(id, 42);
        assert_eq!(role, EcMaterialStableRole::Base);
        assert_eq!(reason, MainEcTextureRoleReason::RoleBase);
    }

    #[test]
    fn falls_back_to_worldart_when_no_base() {
        let texture_refs = [
            texture_ref_with_role(
                55,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Unknown,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                66,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Unknown,
                0,
            ),
        ];

        let (id, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(id, 55);
        assert_eq!(reason, MainEcTextureRoleReason::FallbackWorldArtPrimary);
    }

    #[test]
    fn low_confidence_when_fallback_any() {
        let texture_refs = [texture_ref_with_role(
            10,
            EcMaterialLogicalFamily::Textures,
            EcMaterialStableRole::WaterBump,
            0,
        )];

        let (_, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(reason, MainEcTextureRoleReason::FallbackAnyFirst);
        assert!(reason.is_low_confidence());
    }

    #[test]
    fn not_found_when_no_refs() {
        let result = choose_main_ec_texture_id_role_aware(&[], None);
        assert!(result.is_none());
    }

    #[test]
    fn legacy_ec_texture_id_used_as_last_resort() {
        // All refs are auxiliary — none qualify as non-aux candidates.
        let texture_refs = [TileMetaItemTextureRef {
            texture_id: 999,
            texture_type: EcMaterialLogicalFamily::WorldArt as u8,
            stable_role: EcMaterialStableRole::Base as u8,
            flags: TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY,
            ..TileMetaItemTextureRef::zeroed()
        }];
        let mut item_tile = TileMetaItemTile::zeroed();
        item_tile.ec_texture_id = 777;

        let (id, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, Some(&item_tile)).expect("result");

        assert_eq!(id, 777);
        assert_eq!(reason, MainEcTextureRoleReason::FallbackLegacyEcTextureId);
        assert!(reason.is_low_confidence());
    }
}
