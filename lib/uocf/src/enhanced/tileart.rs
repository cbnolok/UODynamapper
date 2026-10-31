//! Represents tile art data from a UOP file for Enhanced Client "Art Tiles" (statics).

use crate::ec_material_axes::{EcLogicalFamily, EcPhysicalPackage};
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::effect_definition::{
    encode_tileart_effects_dialect, read_effect_definitions_dialect, EffectDefinition,
};
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, bail, ensure, WrapErr};
use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::Path;

// region: --- Public API (Convenience & Application Use)

/// Main entry point to load and process Art Tile data from an Enhanced Client UOP file.
/// This function handles the full pipeline: parsing the raw binary structure and
/// resolving string references into a clean, public-facing ArtData structure.
pub fn load(
    uop_file: &UopFile,
    string_dictionary: &UoStringDictionary,
) -> color_eyre::eyre::Result<ArtData> {
    let raw_entry = TileArtEntry::parse_raw(uop_file)?;
    let processed_data = raw_entry.process(string_dictionary);
    Ok(processed_data)
}

/// Record version accepted by the client (`UOTileArt::ParseObjectDefinition` rejects any other).
pub const TILEART_RECORD_VERSION: u16 = 4;
/// Record version of the Kingdom Reborn client (`UOKR.exe` reader 0x007bdeb2): the v4 record without
/// `uses_enhanced_tileart`, `alternate_animation`, sitting words 4 and 5, the radar colour and the TileArtEnhanced
/// shader block, with u32 flags and the Kingdom Reborn effect grammar
/// (`scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md`, answer 9).
pub const TILEART_KR_RECORD_VERSION: u16 = 3;
/// Client package holding one record per tile id.
pub const TILEART_UOP_NAME: &str = "tileart.uop";
/// Shader block `has_texture` value of a present block; any other value is a block without data.
pub const TILEART_SHADER_BLOCK_PRESENT: u8 = 1;
/// Compression of records added to a package without records (client records are zlib-compressed).
const DEFAULT_TILEART_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;
/// Shader blocks per record: 0 WorldArt, 1 TileArtLegacy, 2 TileArtEnhanced, 3 light texture.
pub const TILEART_SHADER_BLOCK_COUNT: usize = 4;
/// Number of u32 words in a sitting block (`UOTileArtSitting`).
pub const TILEART_SITTING_WORD_COUNT: usize = 6;
/// Number of u32 words in a Kingdom Reborn sitting block (the first four of the Enhanced Client block).
pub const TILEART_KR_SITTING_WORD_COUNT: usize = 4;
/// Shader block index of TileArtEnhanced, the block a Kingdom Reborn record lacks.
pub const TILEART_SHADER_BLOCK_ENHANCED: usize = 2;
/// Shader blocks of a Kingdom Reborn record: WorldArt, TileArtLegacy and light texture.
pub const TILEART_KR_SHADER_BLOCK_COUNT: usize = TILEART_SHADER_BLOCK_COUNT - 1;
/// Appearance filter subtype: appearance by owner body type.
pub const APPEARANCE_FILTER_BY_OWNER_BODY_TYPE: u8 = 0;
/// Appearance filter subtype: appearance by owner gender.
pub const APPEARANCE_FILTER_BY_OWNER_GENDER: u8 = 1;
/// Appearance filter subtype: animation appearance by owner body type.
pub const APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE: u8 = 2;

const TILEART_SITTING_ABSENT: u8 = 0;
const TILEART_SITTING_PRESENT: u8 = 1;

/// `TaeSittingAnimation::direction_mask` bit: the mobile may sit facing north.
pub const SITTING_DIRECTION_NORTH: u32 = 1 << 0;
/// `TaeSittingAnimation::direction_mask` bit: the mobile may sit facing east.
pub const SITTING_DIRECTION_EAST: u32 = 1 << 1;
/// `TaeSittingAnimation::direction_mask` bit: the mobile may sit facing south.
pub const SITTING_DIRECTION_SOUTH: u32 = 1 << 2;
/// `TaeSittingAnimation::direction_mask` bit: the mobile may sit facing west.
pub const SITTING_DIRECTION_WEST: u32 = 1 << 3;

// `TileArtEntry::house_join_bits` placement rules (`UOHouseCustomization::IsPlacementAllowed`)
// and support (`UOHouseCustomization::UpdateSupport`). Cells are relative to the house origin;
// x grows east, y grows south. Bits 0, 4 and 15 are not read by the client.

/// House placement: required on the north-west corner cell (0, 0).
pub const HOUSE_JOIN_CORNER: u32 = 1 << 1;
/// House placement: required on the west edge cells (x = 0).
pub const HOUSE_JOIN_WEST_EDGE: u32 = 1 << 2;
/// House placement: required on the north edge cells (y = 0).
pub const HOUSE_JOIN_NORTH_EDGE: u32 = 1 << 3;
/// House placement: the piece can stand directly on a floor or on a load-bearing piece below
/// (`HOUSE_JOIN_SUPPORTS_ABOVE`), so the client refuses it on stairs and teleporter cells and
/// above stairs. Pieces without it are supported only by a connected supported neighbour; the
/// client draws unsupported pieces with `HOUSE_UNSUPPORTED_HUE`.
pub const HOUSE_JOIN_RESTS_ON_FLOOR: u32 = 1 << 5;
/// House placement: a supported piece with this bit supports the cell above it, and with the
/// west/north edge bits also the floor of the next cell above.
pub const HOUSE_JOIN_SUPPORTS_ABOVE: u32 = 1 << 14;
/// Hue the client draws unsupported house customization pieces with (display only; the
/// client still commits a design with unsupported pieces).
pub const HOUSE_UNSUPPORTED_HUE: u16 = 43;

// House customization connectors (`UOHouseCustomization::TilesConnect`): two adjacent tiles
// join when a connector of one matches the same-numbered connector on the facing side of the
// other (west 1 with east 1, west 2 with east 2, north 1 with south 1, north 2 with south 2).

/// House connector: west side, kind 1.
pub const HOUSE_JOIN_WEST_1: u32 = 1 << 6;
/// House connector: west side, kind 2.
pub const HOUSE_JOIN_WEST_2: u32 = 1 << 7;
/// House connector: east side, kind 1.
pub const HOUSE_JOIN_EAST_1: u32 = 1 << 8;
/// House connector: east side, kind 2.
pub const HOUSE_JOIN_EAST_2: u32 = 1 << 9;
/// House connector: north side, kind 1.
pub const HOUSE_JOIN_NORTH_1: u32 = 1 << 10;
/// House connector: north side, kind 2.
pub const HOUSE_JOIN_NORTH_2: u32 = 1 << 11;
/// House connector: south side, kind 1.
pub const HOUSE_JOIN_SOUTH_1: u32 = 1 << 12;
/// House connector: south side, kind 2.
pub const HOUSE_JOIN_SOUTH_2: u32 = 1 << 13;

/// `TaeTextureImage::source_kind`: the client loads the texture. Any value other than
/// `TEXTURE_SOURCE_NORMAL` and `TEXTURE_SOURCE_WRAP` makes the client load no texture.
pub const TEXTURE_SOURCE_NORMAL: u8 = 0;
/// `TaeTextureImage::source_kind`: the client loads the texture and forces wrap addressing.
pub const TEXTURE_SOURCE_WRAP: u8 = 2;

/// Inferred tile classification used for high-level logic.
///
/// This is a convenience abstraction, not a file field: `get_tile_type` derives it
/// from the WorldArt block (block 0) shader name, the `Rotated` flag and the first
/// texture stretch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TileType {
    #[default]
    Static,
    Solid,
    Liquid,
}

/// Direct mapping of indices used in the raw `TaeProp` array in the binary file.
///
/// Each enum variant represents a specific property slot (0-11) as defined in the
/// `tileart.uop` format. These values are used byte-per-byte from the source.
#[derive(Debug, Clone, Copy)]
pub enum PropertyKey {
    /// The client tests `weight > 90`; the consumer of that test was not traced.
    Weight = 0,
    Quality = 1,
    Quantity = 2,
    /// Copied to the world object.
    Height = 3,
    Value = 4,
    AcVc = 5,
    /// Equipment layer of a wearable item (formerly `Slot`).
    Layer = 6,
    /// No client meaning found; set on beds, curtains, banners and cloth (formerly `OffC8`).
    Key7 = 7,
    Appearance = 8,
    Race = 9,
    Gender = 10,
    /// Appearance override: the definition of `value + 10000000`, used before `Appearance`.
    Paperdoll = 11,
}

/// Cleaned-up representation of texture coordinates and offsets for a tile.
///
/// Unlike the internal `TaeImgOffset`, this structure includes the resolved `texture_id`
/// and is what the rest of the application uses to render the tile.
#[derive(Debug, Default, Clone)]
pub struct ArtTexture {
    pub texture_id: u32,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// The primary public-facing structure for Enhanced Client static art data.
///
/// This is a convenience structure that aggregates and simplifies the raw data found in
/// `TileArtEntry`. It is intended for use by the renderer and other high-level systems.
/// It contains resolved textures for both Enhanced (EC) and Classic (CC) visual modes,
/// as well as unified flags and radar colors.
#[derive(Debug, Default, Clone)]
pub struct ArtData {
    pub id: u16,
    /// ID stored in the source record; it can differ from the asset lookup ID.
    pub source_tile_id: u32,
    pub tile_type: TileType,
    pub flags: TaeFlag,
    pub height: u8,
    pub ec_texture: Option<ArtTexture>,
    pub cc_texture: Option<ArtTexture>,
    pub radar_color: TaeRadarcol,
    pub texture_items: Vec<Vec<TextureItem>>,
    /// Original per-block shader identity and parameters, separate from image refs.
    pub materials: Vec<ArtMaterial>,
    pub appearance: Vec<TaeAppearanceFilter>,
    pub sitting: Option<TaeSittingAnimation>,
    /// Effect definitions of the record. Particle names stay client string dictionary
    /// indices; resolve them with `ParticleEffect::name`.
    pub effects: Vec<EffectDefinition>,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtMaterial {
    pub block_index: u8,
    pub shader_name: Option<String>,
    pub technique: Vec<i32>,
    pub tweaks: Vec<f32>,
    /// Shader block byte at `UOShaderDefinition` +0x04; 0 in all client records, meaning unknown.
    pub field_04: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureType {
    #[default]
    Undefined,
    WorldArt,
    TileArtLegacy,
    TileArtEnhanced,
    Textures,
}

#[derive(Debug, Clone, Default)]
pub struct TextureItem {
    pub texture_type: TextureType,
    pub id: u32,
    /// Raw path string as it appears in the string dictionary.
    pub raw_path: Option<String>,
    /// Lowercased path with backslashes converted to forward slashes.
    pub normalized_path: Option<String>,
    /// Legacy convenience accessor — same as `raw_path` unwrapped or empty.
    pub path: String,
    pub block_index: u8,
    pub item_index: u8,
    pub texture_stretch: f32,
    /// Texture load path selector (`TEXTURE_SOURCE_*`); see `TaeTextureImage::source_kind`.
    pub source_kind: u8,
    /// Not read by the client; see `TaeTextureImage::sampler_index`.
    pub sampler_index: u32,
    /// Texture coordinate set index; see `TaeTextureImage::uv_set`.
    pub uv_set: u32,
    pub is_auxiliary: bool,
    /// Logical folder family inferred from the path.
    pub ec_logical_family: EcLogicalFamily,
    /// Physical UOP package inferred from the path.
    pub ec_physical_package: EcPhysicalPackage,
}

bitflags! {
    /// Direct bit-for-bit mapping of the 64-bit tile flag field found in the binary file.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct TaeFlag: u64 {
        const None = 0x0;
        const Background = 0x1;
        const Weapon = 0x2;
        const Transparent = 0x4;
        const Translucent = 0x8;
        const Wall = 0x10;
        const Damaging = 0x20;
        const Impassable = 0x40;
        const Wet = 0x80;
        const Ignored = 0x100;
        const Surface = 0x200;
        const Bridge = 0x400;
        const Generic = 0x800;
        const Window = 0x1000;
        const NoShoot = 0x2000;
        const ArticleA = 0x4000;
        const ArticleAn = 0x8000;
        const ArticleThe = Self::ArticleA.bits() | Self::ArticleAn.bits();
        const Mongen = 0x10000;
        const Foliage = 0x20000;
        const PartialHue = 0x40000;
        const UseNewArt = 0x80000;
        const Map = 0x100000;
        const Container = 0x200000;
        const Wearable = 0x400000;
        const LightSource = 0x800000;
        const Animation = 0x1000000;
        const HoverOver = 0x2000000;
        const ArtUsed = 0x4000000;
        const Armor = 0x8000000;
        const Roof = 0x10000000;
        const Door = 0x20000000;
        const StairBack = 0x40000000;
        const StairRight = 0x80000000;
        const NoHouse = 0x100000000;
        const NoDraw = 0x200000000;
        /// Rotated tile art: the client applies `rotation_degrees` to the sprite and uses the
        /// full texture size in the UI (formerly `Unused1`).
        const Rotated = 0x400000000;
        const AlphaBlend = 0x800000000;
        const NoShadow = 0x1000000000;
        const PixelBleed = 0x2000000000;
        /// Never set in client data and never tested by the client: unused.
        const Unused2 = 0x4000000000;
        const PlayAnimOnce = 0x8000000000;
        const MultiMovable = 0x10000000000;
    }
}

/// Direct mapping of the 4-byte radar color data (typically BGRA) found in the file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeRadarcol {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

// endregion: --- Public API

// region: --- Internal Raw File-Mapping Structs (Binary Format)

impl Default for TaeFlag {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeProp {
    pub id: u8,
    pub val: u32,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeStackAlias {
    pub amount: u32,
    pub amount_id: u32,
}

/// Body type to appearance mapping of a body-type appearance filter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaeBodyAppearance {
    pub body_type: u32,
    pub appearance: u32,
}

/// One appearance filter, as read by `UOTileArt::ParseAppearanceFilters`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaeAppearanceFilter {
    /// Subtype 0 (`AppearanceFilterByOwnerBodyType`): appearance by owner body type.
    ByOwnerBodyType(Vec<TaeBodyAppearance>),
    /// Subtype 1 (`AppearanceFilterByOwnerGender`): appearance by owner gender.
    ByOwnerGender { gender: u8, appearance: u32 },
    /// Subtype 2 (`AnimationAppearanceFilterByOwnerBodyType`): animation appearance by owner body type.
    AnimationByOwnerBodyType(Vec<TaeBodyAppearance>),
}

impl TaeAppearanceFilter {
    /// Subtype byte as stored in the file.
    pub fn subtype(&self) -> u8 {
        match self {
            Self::ByOwnerBodyType(_) => APPEARANCE_FILTER_BY_OWNER_BODY_TYPE,
            Self::ByOwnerGender { .. } => APPEARANCE_FILTER_BY_OWNER_GENDER,
            Self::AnimationByOwnerBodyType(_) => APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE,
        }
    }
}

/// Sitting block (`UOTileArtSitting`), `TILEART_SITTING_WORD_COUNT` 32-bit words.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaeSittingAnimation {
    /// Word 0: allowed seated facings (`SITTING_DIRECTION_*` bits: 1 north, 2 east, 4 south,
    /// 8 west); the client snaps the mobile's facing to the nearest allowed one.
    pub direction_mask: u32,
    /// Word 1: seated mobile sprite x offset; the client draws the mobile at `-seat_offset_x`
    /// (formerly `field_04`).
    pub seat_offset_x: i32,
    /// Word 2: seated mobile sprite y offset; the client draws the mobile at `-seat_offset_y`
    /// (formerly `field_08`).
    pub seat_offset_y: i32,
    /// Word 3: for north (1) and west (8) facings the chair sprite is split into the chair and
    /// a chair-back overlay drawn over the mobile; this moves the split column.
    pub window_shift: i32,
    /// Word 4 (`GetSittingField10`): not read by the client.
    pub field_10: i32,
    /// Word 5 (`GetSittingField14`): not read by the client.
    pub field_14: i32,
}

/// Texture item of a shader block (`UOTileArtTextureItem`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeTextureImage {
    pub name_string_off: u32,
    /// 0 (`TEXTURE_SOURCE_NORMAL`) loads the texture; 2 (`TEXTURE_SOURCE_WRAP`) loads it and
    /// forces wrap addressing; any other value makes the client skip the texture. 0 in all
    /// client records.
    pub source_kind: u8,
    /// Texture stretch; the client switches the sampler to wrap when it is not 1.0.
    pub texture_stretch: f32,
    /// Not read by the client, which binds the items in order (item 0 the base map, the rest
    /// the shader maps); usually equal to the item position.
    pub sampler_index: u32,
    /// Texture coordinate set index (low byte of the Gamebryo texture map flags); 0 or 1.
    pub uv_set: u32,
}

/// Shader block (`UOShaderDefinition`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeTexture {
    pub has_texture: u8,
    /// Byte at `UOShaderDefinition` +0x04 (`GetField04`); 0 in all client records, meaning unknown.
    pub field_04: u8,
    pub type_string_off: u32,
    pub texture_items_count: u8,
    pub texture_items: Vec<TaeTextureImage>,
    pub technique_count: u32,
    /// Shader technique values (`SetTechniqueValue`; the client keeps up to 4).
    pub technique: Vec<i32>,
    pub tweak_count: u32,
    /// Shader tweak values (`SetTweakValue`; the client keeps up to 4).
    pub tweaks: Vec<f32>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaeImgOffset {
    pub x_start: i32,
    pub y_start: i32,
    pub x_end: i32,
    pub y_end: i32,
    pub x_off: i32,
    pub y_off: i32,
}

/// Raw, bit-for-bit mapping of an Art Tile entry as stored in the UOP binary.
///
/// Field meanings follow the client parser (`UOTileArt::ParseObjectDefinition` in UOSA.exe,
/// see `scripts/dev-helpers/ec-research/ghidra-tileart/results/uosa_tileart.md`); fields
/// without a known meaning are named after their in-memory offset. It is used internally
/// for parsing and should not be exposed to the main application. Use `ArtData` instead
/// for a cleaned-up version.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TileArtEntry {
    pub uop_block: i32,
    pub uop_file: i32,
    pub version: u16,
    pub string_dict_off: u32,
    pub tile_id: u32,
    /// Set exactly when the WorldArt shader block (block 0) is present.
    pub has_world_art: bool,
    /// Selects the TileArtEnhanced block (block 2) with a 1.5x legacy window; unset in all client records.
    pub uses_enhanced_tileart: bool,
    /// Sprite scale when drawing WorldArt (client default 1.0).
    pub world_art_scale: f32,
    /// Sprite scale when drawing legacy/enhanced tile art (client default 1.5).
    pub legacy_art_scale: f32,
    /// Object field +0x60; 0 in all client records; not read by the client.
    pub field_60: u32,
    /// Classic tile id the art replaces (default 0xffffff); not read by the client.
    pub old_id: i32,
    /// Sprite rotation in degrees, applied only with the `Rotated` flag.
    pub rotation_degrees: f32,
    /// An equipment appearance is only worn by a mobile with this body type (client default
    /// 400, human male; 666 gargoyle male; creature placeholders hold their body id).
    pub body_type: u32,
    /// When this item is equipped in layer 2 (two-handed), the wearer's stand, walk and run
    /// actions use the alternate entry of the animation action map. Set on gargoyle two-handed
    /// weapons and some halberds (formerly `field_e8`). The client copies the raw byte and tests
    /// it for non-zero, so any non-zero byte is true; client data holds only 0 and 1.
    pub alternate_animation: bool,
    /// Scale of the light sprite (client default 1.0; 2.0 on windows and arrow loops).
    pub light_scale: f32,
    /// Object field +0x70 next to the light fields; 0 in all client records, not read by the client.
    pub light_field_70: u32,
    /// Light sprite x offset (rotated by the view, added to the sprite position); only used
    /// with a light shader block.
    pub light_offset_x: f32,
    /// Light sprite y offset; see `light_offset_x`.
    pub light_offset_y: f32,
    /// House customization join bits: `HOUSE_JOIN_*` placement rules (bits 1, 2, 3), floor
    /// support (bit 5), support of the level above (bit 14) and connectors (bits 6..13); bits
    /// 0, 4 and 15 are not read by the client.
    pub house_join_bits: u32,
    pub flags1: TaeFlag,
    /// Not a copy of `flags1`; the client tests only bits 18 (paperdoll partial hue),
    /// 19 (`UseNewArt`) and 24 (`Animation`) in it.
    pub flags2: TaeFlag,
    /// Copied to the sprite component (client default 1); no client reader found (probably
    /// unused).
    pub facing: i32,
    pub ec_img_offset: TaeImgOffset,
    pub cc_img_offset: TaeImgOffset,
    pub prop_count1: u8,
    pub prop_vector1: Vec<TaeProp>,
    pub prop_count2: u8,
    /// Mostly mirrors `prop_vector1`; the client reads it only for the paperdoll gump
    /// (keys 11 and 8, before `prop_vector1`).
    pub prop_vector2: Vec<TaeProp>,
    pub stack_alias_count_offset: u32,
    pub stack_alias_count: u32,
    pub stack_alias_vector: Vec<TaeStackAlias>,
    pub appearance_count: u32,
    pub appearance_vector: Vec<TaeAppearanceFilter>,
    pub has_sitting: bool,
    pub sitting: Option<TaeSittingAnimation>,
    pub radarcol_offset: u32,
    pub radarcol: TaeRadarcol,
    /// The `TILEART_SHADER_BLOCK_COUNT` shader blocks: WorldArt, TileArtLegacy,
    /// TileArtEnhanced and light texture.
    pub texture_vector: Vec<TaeTexture>,
    /// Effect definitions after the shader blocks (`UOTileArt::ParseEffects`: u8 count, then
    /// the definitions); they end the record.
    pub effects: Vec<EffectDefinition>,
}

// endregion: --- Internal Raw File-Mapping Structs

impl TileArtEntry {
    pub fn parse_raw(uop_file: &UopFile) -> color_eyre::eyre::Result<Self> {
        let decompressed_data = uop_file.unpack_arc()?;
        Self::parse_bytes(decompressed_data.as_ref())
    }

    /// Parses one decompressed record the way `UOTileArt::ParseObjectDefinition` does.
    pub fn parse_bytes(data: &[u8]) -> color_eyre::eyre::Result<Self> {
        let mut reader = Cursor::new(data);
        let mut entry = TileArtEntry::default();

        macro_rules! read_value {
            ($field:expr, $reader_method:ident, $type:ty) => {
                $field = reader.$reader_method::<$type>()?;
            };
        }

        read_value!(entry.version, read_u16, LittleEndian);
        let dialect = record_dialect(entry.version)?;
        let kr = dialect.is_kingdom_reborn();
        read_value!(entry.string_dict_off, read_u32, LittleEndian);
        read_value!(entry.tile_id, read_u32, LittleEndian);
        entry.has_world_art = reader.read_u8()? != 0;
        if !kr {
            entry.uses_enhanced_tileart = reader.read_u8()? != 0;
        }
        read_value!(entry.world_art_scale, read_f32, LittleEndian);
        read_value!(entry.legacy_art_scale, read_f32, LittleEndian);
        read_value!(entry.field_60, read_u32, LittleEndian);
        read_value!(entry.old_id, read_i32, LittleEndian);
        read_value!(entry.rotation_degrees, read_f32, LittleEndian);
        read_value!(entry.body_type, read_u32, LittleEndian);
        if !kr {
            entry.alternate_animation = reader.read_u8()? != 0;
        }
        read_value!(entry.light_scale, read_f32, LittleEndian);
        read_value!(entry.light_field_70, read_u32, LittleEndian);
        read_value!(entry.light_offset_x, read_f32, LittleEndian);
        read_value!(entry.light_offset_y, read_f32, LittleEndian);
        read_value!(entry.house_join_bits, read_u32, LittleEndian);
        for flags in [&mut entry.flags1, &mut entry.flags2] {
            let bits = match kr {
                true => u64::from(reader.read_u32::<LittleEndian>()?),
                false => reader.read_u64::<LittleEndian>()?,
            };
            *flags = TaeFlag::from_bits_retain(bits);
        }
        read_value!(entry.facing, read_i32, LittleEndian);

        read_value!(entry.ec_img_offset.x_start, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_start, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.x_end, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_end, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.x_off, read_i32, LittleEndian);
        read_value!(entry.ec_img_offset.y_off, read_i32, LittleEndian);

        read_value!(entry.cc_img_offset.x_start, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_start, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.x_end, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_end, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.x_off, read_i32, LittleEndian);
        read_value!(entry.cc_img_offset.y_off, read_i32, LittleEndian);

        entry.prop_count1 = reader.read_u8()?;
        for _ in 0..entry.prop_count1 {
            let mut prop = TaeProp::default();
            prop.id = reader.read_u8()?;
            prop.val = reader.read_u32::<LittleEndian>()?;
            entry.prop_vector1.push(prop);
        }

        entry.prop_count2 = reader.read_u8()?;
        for _ in 0..entry.prop_count2 {
            let mut prop = TaeProp::default();
            prop.id = reader.read_u8()?;
            prop.val = reader.read_u32::<LittleEndian>()?;
            entry.prop_vector2.push(prop);
        }

        entry.stack_alias_count_offset = reader.position() as u32;
        read_value!(entry.stack_alias_count, read_u32, LittleEndian);
        for _ in 0..entry.stack_alias_count {
            let mut alias = TaeStackAlias::default();
            alias.amount = reader.read_u32::<LittleEndian>()?;
            alias.amount_id = reader.read_u32::<LittleEndian>()?;
            entry.stack_alias_vector.push(alias);
        }

        read_value!(entry.appearance_count, read_u32, LittleEndian);
        for _ in 0..entry.appearance_count {
            let appearance = match reader.read_u8()? {
                APPEARANCE_FILTER_BY_OWNER_BODY_TYPE => {
                    TaeAppearanceFilter::ByOwnerBodyType(read_body_appearances(&mut reader)?)
                }
                APPEARANCE_FILTER_BY_OWNER_GENDER => {
                    let gender = reader.read_u8()?;
                    let appearance = reader.read_u32::<LittleEndian>()?;
                    TaeAppearanceFilter::ByOwnerGender { gender, appearance }
                }
                APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE => {
                    TaeAppearanceFilter::AnimationByOwnerBodyType(read_body_appearances(&mut reader)?)
                }
                subtype => bail!("unknown tileart appearance filter subtype {subtype}"),
            };
            entry.appearance_vector.push(appearance);
        }

        entry.has_sitting = match reader.read_u8()? {
            TILEART_SITTING_ABSENT => false,
            TILEART_SITTING_PRESENT => true,
            flag => bail!("invalid tileart sitting flag {flag}"),
        };
        if entry.has_sitting {
            let mut words = [0u32; TILEART_SITTING_WORD_COUNT];
            let word_count = if kr { TILEART_KR_SITTING_WORD_COUNT } else { TILEART_SITTING_WORD_COUNT };
            reader.read_u32_into::<LittleEndian>(&mut words[..word_count])?;
            let [direction_mask, seat_offset_x, seat_offset_y, window_shift, field_10, field_14] = words;
            entry.sitting = Some(TaeSittingAnimation {
                direction_mask,
                seat_offset_x: seat_offset_x as i32,
                seat_offset_y: seat_offset_y as i32,
                window_shift: window_shift as i32,
                field_10: field_10 as i32,
                field_14: field_14 as i32,
            });
        }

        entry.radarcol_offset = reader.position() as u32;
        if !kr {
            entry.radarcol.r = reader.read_u8()?;
            entry.radarcol.g = reader.read_u8()?;
            entry.radarcol.b = reader.read_u8()?;
            entry.radarcol.a = reader.read_u8()?;
        }

        for index in 0..TILEART_SHADER_BLOCK_COUNT {
            let mut texture = TaeTexture::default();
            if kr && index == TILEART_SHADER_BLOCK_ENHANCED {
                entry.texture_vector.push(texture);
                continue;
            }
            texture.has_texture = reader.read_u8()?;
            if texture.has_texture == TILEART_SHADER_BLOCK_PRESENT {
                texture.field_04 = reader.read_u8()?;
                texture.type_string_off = reader.read_u32::<LittleEndian>()?;
                texture.texture_items_count = reader.read_u8()?;
                for _ in 0..texture.texture_items_count {
                    let mut item = TaeTextureImage::default();
                    item.name_string_off = reader.read_u32::<LittleEndian>()?;
                    item.source_kind = reader.read_u8()?;
                    item.texture_stretch = reader.read_f32::<LittleEndian>()?;
                    item.sampler_index = reader.read_u32::<LittleEndian>()?;
                    item.uv_set = reader.read_u32::<LittleEndian>()?;
                    texture.texture_items.push(item);
                }

                texture.technique_count = reader.read_u32::<LittleEndian>()?;
                for _ in 0..texture.technique_count {
                    texture.technique.push(reader.read_i32::<LittleEndian>()?);
                }

                texture.tweak_count = reader.read_u32::<LittleEndian>()?;
                for _ in 0..texture.tweak_count {
                    texture.tweaks.push(reader.read_f32::<LittleEndian>()?);
                }
            }
            entry.texture_vector.push(texture);
        }

        let effect_count = reader.read_u8()?;
        entry.effects = read_effect_definitions_dialect(&mut reader, usize::from(effect_count), dialect)?;
        let trailing = data.len() - reader.position() as usize;
        ensure!(trailing == 0, "{trailing} trailing bytes after the tileart effects");

        Ok(entry)
    }

    /// Encodes the record: the exact inverse of [`Self::parse_bytes`] for client records. The counts are taken from
    /// the vectors (`prop_count1`, `prop_count2`, `stack_alias_count`, `appearance_count`, `has_sitting`,
    /// `texture_items_count`, `technique_count` and `tweak_count` are ignored; [`Self::sync_counts`] updates them),
    /// the booleans are written as 0 or 1 (the only values in client data; the client tests the bytes for
    /// non-zero), and the derived `uop_block`, `uop_file`, `stack_alias_count_offset` and `radarcol_offset` are not
    /// stored. Fails on another version than [`TILEART_RECORD_VERSION`] or [`TILEART_KR_RECORD_VERSION`], not
    /// exactly [`TILEART_SHADER_BLOCK_COUNT`] shader blocks, or a list longer than its count field. A Kingdom
    /// Reborn (v3) record is written in its own layout and fails on a value it cannot hold: `uses_enhanced_tileart`
    /// or `alternate_animation` set, flags above 32 bits, sitting words 4 or 5, a radar colour, a present
    /// TileArtEnhanced block, or an effect the Kingdom Reborn grammar lacks.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let dialect = record_dialect(self.version)
            .wrap_err_with(|| format!("tileart record {} cannot be written", self.tile_id))?;
        let kr = dialect.is_kingdom_reborn();
        ensure!(
            self.texture_vector.len() == TILEART_SHADER_BLOCK_COUNT,
            "tileart record {} has {} shader blocks (expected {TILEART_SHADER_BLOCK_COUNT})",
            self.tile_id,
            self.texture_vector.len()
        );
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.to_le_bytes());
        put_u32(&mut out, self.string_dict_off);
        put_u32(&mut out, self.tile_id);
        if kr {
            self.ensure_kingdom_reborn_fits()?;
        }
        out.push(u8::from(self.has_world_art));
        if !kr {
            out.push(u8::from(self.uses_enhanced_tileart));
        }
        put_f32(&mut out, self.world_art_scale);
        put_f32(&mut out, self.legacy_art_scale);
        put_u32(&mut out, self.field_60);
        out.extend_from_slice(&self.old_id.to_le_bytes());
        put_f32(&mut out, self.rotation_degrees);
        put_u32(&mut out, self.body_type);
        if !kr {
            out.push(u8::from(self.alternate_animation));
        }
        put_f32(&mut out, self.light_scale);
        put_u32(&mut out, self.light_field_70);
        put_f32(&mut out, self.light_offset_x);
        put_f32(&mut out, self.light_offset_y);
        put_u32(&mut out, self.house_join_bits);
        for flags in [self.flags1, self.flags2] {
            match kr {
                // `ensure_kingdom_reborn_fits` checked that the bits fit.
                true => put_u32(&mut out, flags.bits() as u32),
                false => out.extend_from_slice(&flags.bits().to_le_bytes()),
            }
        }
        out.extend_from_slice(&self.facing.to_le_bytes());
        for offset in [&self.ec_img_offset, &self.cc_img_offset] {
            for value in [offset.x_start, offset.y_start, offset.x_end, offset.y_end, offset.x_off, offset.y_off] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }

        for (props, name) in [(&self.prop_vector1, "props1"), (&self.prop_vector2, "props2")] {
            out.push(count_u8(props.len(), name)?);
            for prop in props {
                out.push(prop.id);
                put_u32(&mut out, prop.val);
            }
        }

        put_u32(&mut out, count_u32(self.stack_alias_vector.len(), "stack aliases")?);
        for alias in &self.stack_alias_vector {
            put_u32(&mut out, alias.amount);
            put_u32(&mut out, alias.amount_id);
        }

        put_u32(&mut out, count_u32(self.appearance_vector.len(), "appearance filters")?);
        for filter in &self.appearance_vector {
            out.push(filter.subtype());
            match filter {
                TaeAppearanceFilter::ByOwnerBodyType(appearances)
                | TaeAppearanceFilter::AnimationByOwnerBodyType(appearances) => {
                    put_u32(&mut out, count_u32(appearances.len(), "body appearances")?);
                    for appearance in appearances {
                        put_u32(&mut out, appearance.body_type);
                        put_u32(&mut out, appearance.appearance);
                    }
                }
                TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
                    out.push(*gender);
                    put_u32(&mut out, *appearance);
                }
            }
        }

        match &self.sitting {
            None => out.push(TILEART_SITTING_ABSENT),
            Some(sitting) => {
                out.push(TILEART_SITTING_PRESENT);
                put_u32(&mut out, sitting.direction_mask);
                let words = [sitting.seat_offset_x, sitting.seat_offset_y, sitting.window_shift, sitting.field_10, sitting.field_14];
                let word_count = if kr { TILEART_KR_SITTING_WORD_COUNT } else { TILEART_SITTING_WORD_COUNT };
                for word in &words[..word_count - 1] {
                    out.extend_from_slice(&word.to_le_bytes());
                }
            }
        }

        if !kr {
            out.extend_from_slice(&[self.radarcol.r, self.radarcol.g, self.radarcol.b, self.radarcol.a]);
        }

        for (index, block) in self.texture_vector.iter().enumerate() {
            if kr && index == TILEART_SHADER_BLOCK_ENHANCED {
                continue;
            }
            out.push(block.has_texture);
            if block.has_texture != TILEART_SHADER_BLOCK_PRESENT {
                continue;
            }
            out.push(block.field_04);
            put_u32(&mut out, block.type_string_off);
            out.push(count_u8(block.texture_items.len(), "texture items")?);
            for item in &block.texture_items {
                put_u32(&mut out, item.name_string_off);
                out.push(item.source_kind);
                put_f32(&mut out, item.texture_stretch);
                put_u32(&mut out, item.sampler_index);
                put_u32(&mut out, item.uv_set);
            }
            put_u32(&mut out, count_u32(block.technique.len(), "technique values")?);
            for value in &block.technique {
                out.extend_from_slice(&value.to_le_bytes());
            }
            put_u32(&mut out, count_u32(block.tweaks.len(), "tweak values")?);
            for value in &block.tweaks {
                put_f32(&mut out, *value);
            }
        }

        out.extend(encode_tileart_effects_dialect(&self.effects, dialect)?);
        Ok(out)
    }

    /// The client whose layout the record has, by its version.
    pub fn dialect(&self) -> eyre::Result<ClientDialect> {
        record_dialect(self.version)
    }

    /// Fails when the record holds a value the Kingdom Reborn (v3) layout has no room for.
    fn ensure_kingdom_reborn_fits(&self) -> eyre::Result<()> {
        let id = self.tile_id;
        ensure!(!self.uses_enhanced_tileart, "tileart record {id}: Kingdom Reborn has no uses_enhanced_tileart");
        ensure!(!self.alternate_animation, "tileart record {id}: Kingdom Reborn has no alternate_animation");
        for (flags, name) in [(self.flags1, "flags1"), (self.flags2, "flags2")] {
            ensure!(
                u32::try_from(flags.bits()).is_ok(),
                "tileart record {id}: {name} {:#x} exceeds the Kingdom Reborn u32",
                flags.bits()
            );
        }
        if let Some(sitting) = &self.sitting {
            ensure!(
                sitting.field_10 == 0 && sitting.field_14 == 0,
                "tileart record {id}: Kingdom Reborn sitting blocks have no words 4 and 5"
            );
        }
        ensure!(self.radarcol == TaeRadarcol::default(), "tileart record {id}: Kingdom Reborn has no radar colour");
        ensure!(
            self.texture_vector.get(TILEART_SHADER_BLOCK_ENHANCED) == Some(&TaeTexture::default()),
            "tileart record {id}: Kingdom Reborn has no TileArtEnhanced shader block"
        );
        Ok(())
    }

    /// Sets the count fields from the vectors they count, as [`Self::to_bytes`] writes them (saturating counts
    /// that do not fit; `to_bytes` refuses those).
    pub fn sync_counts(&mut self) {
        self.prop_count1 = u8::try_from(self.prop_vector1.len()).unwrap_or(u8::MAX);
        self.prop_count2 = u8::try_from(self.prop_vector2.len()).unwrap_or(u8::MAX);
        self.stack_alias_count = u32::try_from(self.stack_alias_vector.len()).unwrap_or(u32::MAX);
        self.appearance_count = u32::try_from(self.appearance_vector.len()).unwrap_or(u32::MAX);
        self.has_sitting = self.sitting.is_some();
        for block in &mut self.texture_vector {
            block.texture_items_count = u8::try_from(block.texture_items.len()).unwrap_or(u8::MAX);
            block.technique_count = u32::try_from(block.technique.len()).unwrap_or(u32::MAX);
            block.tweak_count = u32::try_from(block.tweaks.len()).unwrap_or(u32::MAX);
        }
    }

    pub fn process(&self, string_dictionary: &UoStringDictionary) -> ArtData {
        let mut art_data = ArtData::default();

        art_data.id = self.tile_id as u16;
        art_data.source_tile_id = self.tile_id;
        art_data.flags = self.flags1;
        art_data.radar_color = self.radarcol.clone();
        art_data.height = self.get_property(PropertyKey::Height).unwrap_or(0) as u8;
        art_data.texture_items = self.get_texture_item_vector(string_dictionary);
        art_data.materials = self.texture_vector.iter().enumerate()
            .filter(|(_, block)| block.has_texture == 1)
            .map(|(index, block)| ArtMaterial {
                block_index: index as u8,
                shader_name: block.type_string_off.checked_sub(1)
                    .and_then(|offset| string_dictionary.get_string(offset as usize))
                    .map(str::to_owned),
                technique: block.technique.clone(),
                tweaks: block.tweaks.clone(),
                field_04: block.field_04,
            }).collect();
        art_data.appearance = self.appearance_vector.clone();
        art_data.sitting = self.sitting.clone();
        art_data.effects = self.effects.clone();

        if let Some(ec_texture_block) = self.texture_vector.first() {
            if ec_texture_block.has_texture == 1 {
                art_data.tile_type = self.get_tile_type(ec_texture_block, string_dictionary);
                if let Some(id) =
                    self.select_primary_texture_id(ec_texture_block, string_dictionary, None)
                {
                    art_data.ec_texture = Some(ArtTexture {
                        texture_id: id,
                        start_x: self.ec_img_offset.x_start,
                        start_y: self.ec_img_offset.y_start,
                        end_x: self.ec_img_offset.x_end,
                        end_y: self.ec_img_offset.y_end,
                        offset_x: self.ec_img_offset.x_off,
                        offset_y: self.ec_img_offset.y_off,
                    });
                }
            }
        }

        if let Some(cc_texture_block) = self.texture_vector.get(1) {
            if cc_texture_block.has_texture == 1 {
                if let Some(id) = self.select_primary_texture_id(
                    cc_texture_block,
                    string_dictionary,
                    Some(TextureType::TileArtLegacy),
                ) {
                    art_data.cc_texture = Some(ArtTexture {
                        texture_id: id,
                        start_x: self.cc_img_offset.x_start,
                        start_y: self.cc_img_offset.y_start,
                        end_x: self.cc_img_offset.x_end,
                        end_y: self.cc_img_offset.y_end,
                        offset_x: self.cc_img_offset.x_off,
                        offset_y: self.cc_img_offset.y_off,
                    });
                }
            }
        }

        art_data
    }

    fn get_texture_item_vector(
        &self,
        string_dictionary: &UoStringDictionary,
    ) -> Vec<Vec<TextureItem>> {
        let mut texture_ids = Vec::new();
        for (block_index, texture) in self.texture_vector.iter().enumerate() {
            if texture.has_texture == 1 {
                let mut items = Vec::new();
                for (item_index, texture_item) in texture.texture_items.iter().enumerate() {
                    if let Some(str) =
                        string_dictionary.get_string((texture_item.name_string_off - 1) as usize)
                    {
                        let raw_path = str.to_string();
                        // normalized_path: lowercased, backslash converted to forward slash
                        let normalized_path = str.to_ascii_lowercase().replace('\\', "/");
                        let mut item = TextureItem::default();
                        item.texture_type = classify_texture_path(str);
                        item.ec_logical_family = EcLogicalFamily::infer_from_path(str);
                        item.ec_physical_package = EcPhysicalPackage::infer_from_path(str);
                        if let Some(id) = crate::utils::path::extract_texture_id_from_path(str) {
                            item.id = id;
                        }
                        item.path = raw_path.clone();
                        item.raw_path = Some(raw_path);
                        item.normalized_path = Some(normalized_path);
                        item.block_index = block_index as u8;
                        item.item_index = item_index as u8;
                        item.texture_stretch = texture_item.texture_stretch;
                        item.source_kind = texture_item.source_kind;
                        item.sampler_index = texture_item.sampler_index;
                        item.uv_set = texture_item.uv_set;
                        item.is_auxiliary = is_auxiliary_texture_path(str);

                        items.push(item);
                    }
                }
                texture_ids.push(items);
            }
        }
        texture_ids
    }

    pub fn get_tile_type(
        &self,
        texture_block: &TaeTexture,
        string_dictionary: &UoStringDictionary,
    ) -> TileType {
        if let Some(shader_name) =
            string_dictionary.get_string((texture_block.type_string_off - 1) as usize)
        {
            match shader_name {
                "UOWaterShader" => TileType::Liquid,
                "UOStaticTerrainShader" => TileType::Solid,
                "UOSpriteShader" => {
                    if self.flags1.contains(TaeFlag::Rotated) {
                        return TileType::Solid;
                    }
                    if let Some(item) = texture_block.texture_items.first() {
                        if item.texture_stretch != 1.0 {
                            return TileType::Solid;
                        }
                    }
                    TileType::Static
                }
                _ => TileType::Static,
            }
        } else {
            TileType::Static
        }
    }

    /// Value of `key` in `prop_vector1` only, as the client's definition getters read it
    /// (for example `GetAppearance`, props1 key 8).
    pub fn property1(&self, key: PropertyKey) -> Option<u32> {
        let key_id = key as u8;
        self.prop_vector1
            .iter()
            .find(|prop| prop.id == key_id)
            .map(|prop| prop.val)
    }

    fn get_property(&self, key: PropertyKey) -> Option<u32> {
        let key_id = key as u8;
        for prop in &self.prop_vector1 {
            if prop.id == key_id {
                return Some(prop.val);
            }
        }
        for prop in &self.prop_vector2 {
            if prop.id == key_id {
                return Some(prop.val);
            }
        }
        None
    }

    fn get_texture_id_from_string_offset(
        offset: u32,
        string_dictionary: &UoStringDictionary,
    ) -> Option<u32> {
        string_dictionary
            .get_string((offset - 1) as usize)
            .and_then(crate::utils::path::extract_texture_id_from_path)
    }

    fn select_primary_texture_id(
        &self,
        texture_block: &TaeTexture,
        string_dictionary: &UoStringDictionary,
        preferred_type: Option<TextureType>,
    ) -> Option<u32> {
        let mut preferred_non_aux = None;
        let mut non_aux = None;
        let mut preferred_any = None;
        let mut any = None;

        for texture_item in &texture_block.texture_items {
            let Some(path) =
                string_dictionary.get_string((texture_item.name_string_off - 1) as usize)
            else {
                continue;
            };
            let Some(texture_id) = crate::utils::path::extract_texture_id_from_path(path) else {
                continue;
            };

            let texture_type = classify_texture_path(path);
            let auxiliary = is_auxiliary_texture_path(path);

            any.get_or_insert(texture_id);

            if !auxiliary {
                non_aux.get_or_insert(texture_id);
            }

            if preferred_type.is_some_and(|preferred| texture_type == preferred) {
                preferred_any.get_or_insert(texture_id);
                if !auxiliary {
                    preferred_non_aux.get_or_insert(texture_id);
                }
            }
        }

        preferred_non_aux.or(non_aux).or(preferred_any).or(any)
    }
}

/// Reads a body-type appearance list: u32 count, then count x {u32 body type, u32 appearance}.
/// Entry path of the record of `tile_id` in `tileart.uop` (`data/assetmap.xml` of `MainMisc.uop`:
/// `Build/TileArt/%08d.bin`, hashed lowercase). Ids above 8 digits keep all their digits, as with `%08d`. The client
/// finds a tile's record by this name (the tile id is the asset id); the `tile_id` stored in the record is only
/// copied (`GetTileId`). Every client entry is named after a tile id, and 25 client records store another id than
/// their name's (e.g. 43957..=43963 store 43956), so edits are keyed by the entry, not by the stored id.
pub fn tileart_entry_path(tile_id: u32) -> String {
    format!("build/tileart/{tile_id:08}.bin")
}

/// Entry hash of the record of `tile_id` in `tileart.uop`.
pub fn tileart_entry_hash(tile_id: u32) -> u64 {
    hash_file_name_single(&tileart_entry_path(tile_id))
}

/// What [`apply_tileart_edits`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileArtChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

/// Stores each `(entry hash, record)` of `records` as that entry (a new tile's hash is
/// [`tileart_entry_hash`] of its id) and removes the entries of the `removed` hashes; every other entry is left
/// untouched (same compressed bytes and data block header). A record whose entry already holds its bytes is left
/// untouched; a changed one is replaced keeping its compression flag; a new one is added with the compression of
/// the package's records. Both get a fresh timestamp data block header (the package's edit time). Fails on an
/// entry listed twice or both stored and removed; a removed hash without an entry is not counted.
pub fn apply_tileart_edits(
    package: &mut UopPackage,
    records: &[(u64, TileArtEntry)],
    removed: &[u64],
) -> eyre::Result<TileArtChanges> {
    let mut stored = BTreeSet::new();
    for (hash, record) in records {
        ensure!(stored.insert(*hash), "tileart entry {hash:016X} (tile {}) appears twice", record.tile_id);
    }
    if let Some(hash) = removed.iter().find(|hash| stored.contains(*hash)) {
        bail!("tileart entry {hash:016X} is both stored and removed");
    }
    let compression = package
        .iter_files()
        .find(|file| file.has_size())
        .map_or(DEFAULT_TILEART_COMPRESSION, |file| file.compression());
    let mut changes = TileArtChanges::default();
    for (hash, record) in records {
        let payload = record.to_bytes()?;
        let current = package
            .unpack_file_by_hash(*hash)
            .wrap_err_with(|| format!("failed to unpack tileart entry {hash:016X}"))?;
        if current.as_deref() == Some(payload.as_slice()) {
            continue;
        }
        match uop_entries::put_entry(package, *hash, &payload, compression)? {
            EntryPut::Replaced => changes.replaced += 1,
            EntryPut::Added => changes.added += 1,
        }
    }
    for hash in removed.iter().copied().collect::<BTreeSet<_>>() {
        if uop_entries::remove_entry(package, hash) {
            changes.removed += 1;
        }
    }
    Ok(changes)
}

/// Writes a copy of the `source` `tileart.uop` to `output` with [`apply_tileart_edits`] applied (stamped with the
/// current time); the package keeps its version, block size and layout. `output` must not be `source`.
pub fn write_tileart_package(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    records: &[(u64, TileArtEntry)],
    removed: &[u64],
) -> eyre::Result<TileArtChanges> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let changes = apply_tileart_edits(&mut package, records, removed)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(changes)
}

/// The client of tileart record version `version`.
fn record_dialect(version: u16) -> eyre::Result<ClientDialect> {
    match version {
        TILEART_RECORD_VERSION => Ok(ClientDialect::Enhanced),
        TILEART_KR_RECORD_VERSION => Ok(ClientDialect::KingdomReborn),
        _ => bail!(
            "unsupported tileart record version {version} (expected {TILEART_RECORD_VERSION}, or \
             {TILEART_KR_RECORD_VERSION} for Kingdom Reborn)"
        ),
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn count_u8(count: usize, what: &str) -> eyre::Result<u8> {
    u8::try_from(count).map_err(|_| eyre::eyre!("{count} {what} do not fit the u8 count of a tileart record"))
}

fn count_u32(count: usize, what: &str) -> eyre::Result<u32> {
    u32::try_from(count).map_err(|_| eyre::eyre!("{count} {what} do not fit the u32 count of a tileart record"))
}

fn read_body_appearances(reader: &mut Cursor<&[u8]>) -> color_eyre::eyre::Result<Vec<TaeBodyAppearance>> {
    let count = reader.read_u32::<LittleEndian>()?;
    let mut mappings = Vec::new();
    for _ in 0..count {
        let body_type = reader.read_u32::<LittleEndian>()?;
        let appearance = reader.read_u32::<LittleEndian>()?;
        mappings.push(TaeBodyAppearance { body_type, appearance });
    }
    Ok(mappings)
}

pub fn classify_texture_path(path: &str) -> TextureType {
    let normalized = crate::utils::path::normalize_dictionary_path(path);
    if normalized.contains("data\\worldart\\") {
        TextureType::WorldArt
    } else if normalized.contains("data\\tileartlegacy\\") {
        TextureType::TileArtLegacy
    } else if normalized.contains("data\\tileartenhanced\\") {
        TextureType::TileArtEnhanced
    } else if normalized.contains("data\\textures\\") {
        TextureType::Textures
    } else {
        TextureType::Undefined
    }
}

fn is_auxiliary_texture_path(path: &str) -> bool {
    let normalized = crate::utils::path::normalize_dictionary_path(path);
    let file_name = normalized
        .rsplit('\\')
        .next()
        .unwrap_or(normalized.as_str());
    let stem = file_name.split('.').next().unwrap_or(file_name);

    stem.contains("noise")
        || stem.contains("normal")
        || stem
            .split(['_', '-'])
            .any(|segment| matches!(segment, "n" | "nm" | "nrm" | "norm"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::{SoundEffect, EFFECT_TYPE_SOUND, POSITION_OFFSET_DIRECTION_COUNT};

    fn dictionary_with_strings(values: &[&str]) -> UoStringDictionary {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(values.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for value in values {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }

        UoStringDictionary::from_payload_bytes(&payload, "test dictionary")
            .expect("build test dictionary")
    }

    fn dictionary_with_single_string(value: &str) -> UoStringDictionary {
        dictionary_with_strings(&[value])
    }

    #[test]
    fn tileart_process_preserves_material_parameters_and_original_block_indices() {
        let dictionary = dictionary_with_strings(&["UOWaterShader", "UOSpriteShader"]);
        let entry = TileArtEntry {
            texture_vector: vec![
                TaeTexture { has_texture: 1, type_string_off: 1,
                    technique: vec![0, 2, -7, 9],
                    tweaks: vec![-0.01, 0.0, 0.25, -0.5], field_04: 3,
                    ..Default::default() },
                TaeTexture::default(),
                TaeTexture { has_texture: 1, type_string_off: 2, ..Default::default() },
            ],
            ..Default::default()
        };
        let art = entry.process(&dictionary);
        assert_eq!(art.materials.len(), 2);
        assert_eq!(art.materials[0], ArtMaterial {
            block_index: 0, shader_name: Some("UOWaterShader".to_owned()),
            technique: vec![0, 2, -7, 9], tweaks: vec![-0.01, 0.0, 0.25, -0.5], field_04: 3,
        });
        assert_eq!(art.materials[1].block_index, 2);
        assert_eq!(art.materials[1].shader_name.as_deref(), Some("UOSpriteShader"));
    }

    #[test]
    fn tileart_get_tile_type_treats_rotated_sprite_as_solid() {
        let dictionary = dictionary_with_single_string("UOSpriteShader");
        let entry = TileArtEntry {
            flags1: TaeFlag::Rotated,
            ..Default::default()
        };
        let texture_block = TaeTexture {
            type_string_off: 1,
            has_texture: 1,
            ..Default::default()
        };

        assert_eq!(
            entry.get_tile_type(&texture_block, &dictionary),
            TileType::Solid
        );
    }

    #[test]
    fn tileart_process_prefers_non_aux_ec_texture_item() {
        let dictionary = dictionary_with_strings(&[
            "UOStaticTerrainShader",
            "Data\\WorldArt\\01000045_noise.tga",
            "Data\\WorldArt\\02000130_Cave_Floor_A.tga",
        ]);
        let entry = TileArtEntry {
            tile_id: 1345,
            texture_vector: vec![
                TaeTexture {
                    has_texture: 1,
                    type_string_off: 1,
                    texture_items: vec![
                        TaeTextureImage {
                            name_string_off: 2,
                            ..Default::default()
                        },
                        TaeTextureImage {
                            name_string_off: 3,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                TaeTexture::default(),
                TaeTexture::default(),
                TaeTexture::default(),
            ],
            ..Default::default()
        };

        let art_data = entry.process(&dictionary);

        assert_eq!(
            art_data.ec_texture.map(|texture| texture.texture_id),
            Some(2000130)
        );
    }

    #[test]
    fn classify_texture_path_recognizes_textures_family() {
        assert_eq!(
            classify_texture_path("Data\\Textures\\00001234_water_alpha.dds"),
            TextureType::Textures
        );
    }

    #[test]
    fn tileart_process_preserves_animation_and_sitting_metadata() {
        let dictionary = dictionary_with_single_string("UOSpriteShader");
        let entry = TileArtEntry {
            appearance_vector: vec![TaeAppearanceFilter::ByOwnerGender {
                gender: 7,
                appearance: 1234,
            }],
            sitting: Some(TaeSittingAnimation {
                direction_mask: SITTING_DIRECTION_NORTH,
                seat_offset_x: 2,
                seat_offset_y: 3,
                window_shift: 4,
                field_10: 5,
                field_14: 6,
            }),
            ..Default::default()
        };

        let art_data = entry.process(&dictionary);

        assert_eq!(art_data.appearance.len(), 1);
        assert_eq!(art_data.appearance[0].subtype(), APPEARANCE_FILTER_BY_OWNER_GENDER);
        assert_eq!(art_data.sitting.as_ref().map(|sitting| sitting.seat_offset_y), Some(3));
    }

    fn sample_effect() -> EffectDefinition {
        EffectDefinition::Sound(SoundEffect { sound_id: 43, value_08: 1.0 })
    }

    #[test]
    fn tileart_process_preserves_effects() {
        let entry = TileArtEntry { effects: vec![sample_effect()], ..Default::default() };
        assert_eq!(entry.process(&dictionary_with_strings(&[])).effects, vec![sample_effect()]);
    }

    /// Little-endian writer for synthetic records.
    #[derive(Default)]
    struct RecordWriter(Vec<u8>);

    impl RecordWriter {
        fn u8(&mut self, value: u8) -> &mut Self { self.0.push(value); self }
        fn u16(&mut self, value: u16) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn u32(&mut self, value: u32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn i32(&mut self, value: i32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn u64(&mut self, value: u64) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn f32(&mut self, value: f32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }

        /// Shader block with one texture item per `(path index, source_kind, stretch, sampler, uv)`.
        fn block(&mut self, shader: u32, items: &[(u32, u8, f32, u32, u32)], technique: &[i32], tweaks: &[f32]) -> &mut Self {
            self.u8(1).u8(0).u32(shader).u8(items.len() as u8);
            for &(path, source_kind, stretch, sampler_index, uv_set) in items {
                self.u32(path).u8(source_kind).f32(stretch).u32(sampler_index).u32(uv_set);
            }
            self.u32(technique.len() as u32);
            technique.iter().for_each(|value| { self.i32(*value); });
            self.u32(tweaks.len() as u32);
            tweaks.iter().for_each(|value| { self.f32(*value); });
            self
        }
    }

    /// Builds a record with every variable part populated; the arguments override the
    /// version, the last appearance filter subtype and the sitting flag.
    fn sample_record(version: u16, last_appearance_subtype: u8, sitting_flag: u8) -> Vec<u8> {
        let mut writer = RecordWriter::default();
        writer.u16(version).u32(7).u32(1234).u8(1).u8(0).f32(1.4).f32(1.5).u32(0).i32(1233)
            .f32(90.0).u32(666).u8(3).f32(2.0).u32(0).f32(-5.0).f32(-4.0).u32(0x3FC0)
            .u64((TaeFlag::Rotated | TaeFlag::AlphaBlend).bits()).u64(TaeFlag::UseNewArt.bits()).u32(3);
        (1..=6).chain(7..=12).for_each(|value| { writer.i32(value); });
        // props1, props2, stack aliases
        writer.u8(1).u8(PropertyKey::Height as u8).u32(5).u8(0).u32(1).u32(5).u32(1235);
        // appearance filters 0, 1, 2 (the last subtype is overridable)
        writer.u32(3)
            .u8(APPEARANCE_FILTER_BY_OWNER_BODY_TYPE).u32(2).u32(400).u32(10).u32(401).u32(11)
            .u8(APPEARANCE_FILTER_BY_OWNER_GENDER).u8(1).u32(77)
            .u8(last_appearance_subtype).u32(1).u32(666).u32(12);
        writer.u8(sitting_flag).u32(SITTING_DIRECTION_WEST).i32(-3).i32(-20).i32(-6).i32(3).i32(-4);
        writer.u8(10).u8(20).u8(30).u8(255);
        writer.block(2, &[(3, TEXTURE_SOURCE_NORMAL, 2.0, 1, 1)], &[0, 2, -7, 9], &[-0.01, 0.0, 0.25, -0.5])
            .block(4, &[], &[], &[])
            .u8(0)
            .block(5, &[(6, TEXTURE_SOURCE_WRAP, 1.0, 0, 0)], &[], &[]);
        writer.u8(1).u32(EFFECT_TYPE_SOUND).u32(43).f32(1.0);
        writer.0
    }

    fn valid_sample_record() -> Vec<u8> {
        sample_record(TILEART_RECORD_VERSION, APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE, TILEART_SITTING_PRESENT)
    }

    #[test]
    fn tileart_parse_bytes_reads_every_record_part() {
        let entry = TileArtEntry::parse_bytes(&valid_sample_record()).expect("parse sample record");

        assert_eq!(entry.version, TILEART_RECORD_VERSION);
        assert_eq!((entry.string_dict_off, entry.tile_id), (7, 1234));
        assert!(entry.has_world_art);
        assert!(!entry.uses_enhanced_tileart);
        assert_eq!((entry.world_art_scale, entry.legacy_art_scale), (1.4, 1.5));
        assert_eq!((entry.field_60, entry.old_id), (0, 1233));
        assert_eq!((entry.rotation_degrees, entry.body_type, entry.alternate_animation), (90.0, 666, true));
        assert_eq!((entry.light_scale, entry.light_field_70), (2.0, 0));
        assert_eq!((entry.light_offset_x, entry.light_offset_y), (-5.0, -4.0));
        assert_eq!(entry.house_join_bits, 0x3FC0);
        assert_eq!(entry.flags1, TaeFlag::Rotated | TaeFlag::AlphaBlend);
        assert_eq!(entry.flags2, TaeFlag::UseNewArt);
        assert_eq!(entry.facing, 3);
        let ec = &entry.ec_img_offset;
        assert_eq!([ec.x_start, ec.y_start, ec.x_end, ec.y_end, ec.x_off, ec.y_off], [1, 2, 3, 4, 5, 6]);
        let cc = &entry.cc_img_offset;
        assert_eq!([cc.x_start, cc.y_start, cc.x_end, cc.y_end, cc.x_off, cc.y_off], [7, 8, 9, 10, 11, 12]);

        assert_eq!(entry.prop_count1, 1);
        assert_eq!((entry.prop_vector1[0].id, entry.prop_vector1[0].val), (PropertyKey::Height as u8, 5));
        assert_eq!(entry.prop_count2, 0);
        assert!(entry.prop_vector2.is_empty());
        assert_eq!(entry.stack_alias_count, 1);
        assert_eq!((entry.stack_alias_vector[0].amount, entry.stack_alias_vector[0].amount_id), (5, 1235));

        assert_eq!(entry.appearance_count, 3);
        assert_eq!(entry.appearance_vector, vec![
            TaeAppearanceFilter::ByOwnerBodyType(vec![
                TaeBodyAppearance { body_type: 400, appearance: 10 },
                TaeBodyAppearance { body_type: 401, appearance: 11 },
            ]),
            TaeAppearanceFilter::ByOwnerGender { gender: 1, appearance: 77 },
            TaeAppearanceFilter::AnimationByOwnerBodyType(vec![
                TaeBodyAppearance { body_type: 666, appearance: 12 },
            ]),
        ]);

        assert!(entry.has_sitting);
        assert_eq!(entry.sitting, Some(TaeSittingAnimation {
            direction_mask: SITTING_DIRECTION_WEST, seat_offset_x: -3, seat_offset_y: -20, window_shift: -6,
            field_10: 3, field_14: -4,
        }));
        assert_eq!((entry.radarcol.r, entry.radarcol.g, entry.radarcol.b, entry.radarcol.a), (10, 20, 30, 255));

        assert_eq!(entry.texture_vector.len(), TILEART_SHADER_BLOCK_COUNT);
        let [world_art, legacy, enhanced, light] = &entry.texture_vector[..] else { unreachable!() };
        assert_eq!((world_art.has_texture, world_art.field_04, world_art.type_string_off), (1, 0, 2));
        assert_eq!(world_art.texture_items_count, 1);
        let item = &world_art.texture_items[0];
        assert_eq!((item.name_string_off, item.source_kind, item.texture_stretch), (3, TEXTURE_SOURCE_NORMAL, 2.0));
        assert_eq!((item.sampler_index, item.uv_set), (1, 1));
        assert_eq!((world_art.technique_count, world_art.technique.as_slice()), (4, &[0, 2, -7, 9][..]));
        assert_eq!((world_art.tweak_count, world_art.tweaks.as_slice()), (4, &[-0.01, 0.0, 0.25, -0.5][..]));
        assert_eq!((legacy.has_texture, legacy.type_string_off, legacy.texture_items_count), (1, 4, 0));
        assert_eq!((legacy.technique_count, legacy.tweak_count), (0, 0));
        assert_eq!(enhanced.has_texture, 0);
        assert_eq!((light.has_texture, light.type_string_off, light.texture_items_count), (1, 5, 1));
        let light_item = &light.texture_items[0];
        assert_eq!((light_item.name_string_off, light_item.source_kind, light_item.texture_stretch), (6, TEXTURE_SOURCE_WRAP, 1.0));
        assert_eq!((light_item.sampler_index, light_item.uv_set), (0, 0));

        assert_eq!(entry.effects, vec![sample_effect()]);
    }

    #[test]
    fn tileart_parse_bytes_rejects_what_the_client_rejects() {
        let unknown_subtype = APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE + 1;
        let invalid_sitting_flag = TILEART_SITTING_PRESENT + 1;
        let old_version = TILEART_KR_RECORD_VERSION - 1;
        let mut trailing_byte = valid_sample_record();
        trailing_byte.push(0);
        for (record, reason) in [
            (sample_record(TILEART_RECORD_VERSION, unknown_subtype, TILEART_SITTING_PRESENT), "appearance filter subtype"),
            (sample_record(TILEART_RECORD_VERSION, APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE, invalid_sitting_flag), "sitting flag"),
            (sample_record(old_version, APPEARANCE_FILTER_ANIMATION_BY_OWNER_BODY_TYPE, TILEART_SITTING_PRESENT), "record version"),
            (trailing_byte, "trailing bytes"),
        ] {
            let error = TileArtEntry::parse_bytes(&record).expect_err(reason);
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    /// Builds a record without sitting data or effects; the argument is the alternate animation byte.
    fn minimal_record(alternate_animation: u8) -> Vec<u8> {
        let mut writer = RecordWriter::default();
        writer.u16(TILEART_RECORD_VERSION).u32(1).u32(2).u8(0).u8(0).f32(1.0).f32(1.5).u32(0).i32(0xFFFFFF)
            .f32(0.0).u32(400).u8(alternate_animation).f32(1.0).u32(0).f32(0.0).f32(0.0).u32(0).u64(0).u64(0).u32(1);
        (0..12).for_each(|_| { writer.i32(0); });
        writer.u8(0).u8(0).u32(0).u32(0).u8(TILEART_SITTING_ABSENT).u8(0).u8(0).u8(0).u8(0);
        writer.u8(0).block(1, &[], &[], &[]).block(1, &[], &[], &[]).u8(0).u8(0);
        writer.0
    }

    #[test]
    fn tileart_parse_bytes_without_sitting_or_effects() {
        let entry = TileArtEntry::parse_bytes(&minimal_record(0)).expect("parse minimal record");

        assert!(!entry.has_sitting);
        assert_eq!(entry.sitting, None);
        assert!(entry.effects.is_empty());
        assert!(!entry.alternate_animation);
    }

    #[test]
    fn tileart_to_bytes_is_the_inverse_of_the_parser() {
        /// Offset of the alternate animation byte: version, name index, tile id, two flag bytes, six words.
        const ALTERNATE_ANIMATION_OFFSET: usize = 36;
        // The sample record holds 3 there; the encoder writes the boolean as 1.
        let mut sample = valid_sample_record();
        sample[ALTERNATE_ANIMATION_OFFSET] = 1;
        for record in [sample, minimal_record(0), minimal_record(1)] {
            let entry = TileArtEntry::parse_bytes(&record).expect("parse record");
            assert_eq!(entry.to_bytes().expect("encode record"), record);
        }
    }

    /// Builds a Kingdom Reborn (v3) record: one flag byte, no alternate animation byte, u32 flags, a 4-word
    /// sitting block, no radar colour, 3 shader blocks and a KR effect (position offset, KR key 6).
    fn kingdom_reborn_record() -> Vec<u8> {
        /// KR type key of position offset (EC 7).
        const KR_EFFECT_TYPE_POSITION_OFFSET: u32 = 6;
        let mut writer = RecordWriter::default();
        writer.u16(TILEART_KR_RECORD_VERSION).u32(7).u32(1234).u8(1).f32(1.0).f32(1.5).u32(0).i32(1233)
            .f32(0.0).u32(400).f32(2.0).u32(0).f32(-5.0).f32(-4.0).u32(0x3FC0)
            .u32((TaeFlag::Rotated | TaeFlag::AlphaBlend).bits() as u32).u32(TaeFlag::UseNewArt.bits() as u32).u32(1);
        (1..=12).for_each(|value| { writer.i32(value); });
        writer.u8(0).u8(0).u32(0).u32(0);
        writer.u8(TILEART_SITTING_PRESENT).u32(SITTING_DIRECTION_WEST).i32(-3).i32(-20).i32(-4);
        writer.block(2, &[(3, TEXTURE_SOURCE_NORMAL, 1.0, 0, 0)], &[], &[]).u8(0).block(5, &[], &[], &[1.0]);
        writer.u8(1).u32(KR_EFFECT_TYPE_POSITION_OFFSET);
        (0..2 * POSITION_OFFSET_DIRECTION_COUNT).for_each(|value| { writer.u8(value as u8); });
        writer.0
    }

    #[test]
    fn tileart_kingdom_reborn_records_round_trip_in_their_own_layout() {
        let record = kingdom_reborn_record();
        let entry = TileArtEntry::parse_bytes(&record).expect("parse KR record");
        assert_eq!(entry.dialect().unwrap(), ClientDialect::KingdomReborn);
        assert!(entry.has_world_art && !entry.uses_enhanced_tileart && !entry.alternate_animation);
        assert_eq!((entry.body_type, entry.light_scale, entry.house_join_bits), (400, 2.0, 0x3FC0));
        assert_eq!(entry.flags2, TaeFlag::UseNewArt);
        assert_eq!(entry.sitting, Some(TaeSittingAnimation {
            direction_mask: SITTING_DIRECTION_WEST, seat_offset_x: -3, seat_offset_y: -20, window_shift: -4,
            field_10: 0, field_14: 0,
        }));
        assert_eq!(entry.radarcol, TaeRadarcol::default());
        // KR blocks 0, 1, 2 are the EC blocks 0 (WorldArt), 1 (TileArtLegacy) and 3 (light).
        assert_eq!(entry.texture_vector.len(), TILEART_SHADER_BLOCK_COUNT);
        assert_eq!(entry.texture_vector[TILEART_SHADER_BLOCK_ENHANCED], TaeTexture::default());
        assert_eq!(entry.texture_vector[TILEART_SHADER_BLOCK_COUNT - 1].tweaks, vec![1.0]);
        assert!(matches!(entry.effects[..], [EffectDefinition::PositionOffset(_)]));
        assert_eq!(entry.to_bytes().expect("encode KR record"), record);

        for (edit, reason) in [
            (TileArtEntry { uses_enhanced_tileart: true, ..entry.clone() }, "uses_enhanced_tileart"),
            (TileArtEntry { alternate_animation: true, ..entry.clone() }, "alternate_animation"),
            (TileArtEntry { flags1: TaeFlag::from_bits_retain(1 << u32::BITS), ..entry.clone() }, "u32"),
            (TileArtEntry { radarcol: TaeRadarcol { r: 1, ..TaeRadarcol::default() }, ..entry.clone() }, "radar"),
        ] {
            let error = edit.to_bytes().expect_err(reason);
            assert!(format!("{error:#}").contains(reason), "{error:#}");
        }
        let mut enhanced = entry.clone();
        enhanced.texture_vector[TILEART_SHADER_BLOCK_ENHANCED].has_texture = TILEART_SHADER_BLOCK_PRESENT;
        assert!(enhanced.to_bytes().is_err());
        let mut sitting = entry;
        sitting.sitting.as_mut().unwrap().field_14 = 1;
        assert!(sitting.to_bytes().is_err());
    }

    #[test]
    fn tileart_to_bytes_keeps_unknown_flag_bits() {
        const UNKNOWN_FLAG_BIT: u64 = 1 << 63;
        let mut record = minimal_record(0);
        let entry = TileArtEntry::parse_bytes(&record).unwrap();
        let mut flags = entry.flags1;
        flags.insert(TaeFlag::from_bits_retain(UNKNOWN_FLAG_BIT));
        let edited = TileArtEntry { flags1: flags, ..entry };
        record = edited.to_bytes().unwrap();
        assert_eq!(TileArtEntry::parse_bytes(&record).unwrap().flags1.bits(), UNKNOWN_FLAG_BIT);
    }

    #[test]
    fn tileart_to_bytes_takes_the_counts_from_the_vectors() {
        let mut entry = TileArtEntry::parse_bytes(&valid_sample_record()).unwrap();
        entry.prop_vector1.push(TaeProp { id: PropertyKey::Weight as u8, val: 2 });
        entry.sitting = None;
        entry.texture_vector[0].tweaks.clear();
        let reparsed = TileArtEntry::parse_bytes(&entry.to_bytes().unwrap()).unwrap();
        assert_eq!((reparsed.prop_count1, reparsed.has_sitting, reparsed.texture_vector[0].tweak_count), (2, false, 0));
        entry.sync_counts();
        assert_eq!((entry.prop_count1, entry.has_sitting, entry.texture_vector[0].tweak_count), (2, false, 0));

        entry.texture_vector.pop();
        assert!(entry.to_bytes().is_err(), "a record needs every shader block");
        let wrong_version = TileArtEntry { version: TILEART_RECORD_VERSION - 1, ..TileArtEntry::parse_bytes(&minimal_record(0)).unwrap() };
        assert!(wrong_version.to_bytes().is_err());
    }

    #[test]
    fn apply_tileart_edits_replaces_adds_and_removes_records() {
        const KEPT: u32 = 2;
        const EDITED: u32 = 3;
        const REMOVED: u32 = 4;
        const ADDED: u32 = 10_000_001;
        let base = TileArtEntry::parse_bytes(&minimal_record(0)).unwrap();
        let record = |tile_id| TileArtEntry { tile_id, ..base.clone() };
        let mut package = UopPackage::new_default();
        for tile_id in [KEPT, EDITED, REMOVED] {
            let bytes = record(tile_id).to_bytes().unwrap();
            package.add_file_from_memory(&bytes, &tileart_entry_path(tile_id), CompressionFlag::Zlib).unwrap();
        }
        let kept_before = package.get_file_by_hash(tileart_entry_hash(KEPT)).unwrap().data().cloned();
        let edited = TileArtEntry { body_type: 666, ..record(EDITED) };

        let keyed = |record: TileArtEntry| (tileart_entry_hash(record.tile_id), record);
        let edits = [keyed(record(KEPT)), keyed(edited.clone()), keyed(record(ADDED))];
        let changes = apply_tileart_edits(&mut package, &edits, &[tileart_entry_hash(REMOVED)]).unwrap();

        assert_eq!(changes, TileArtChanges { replaced: 1, added: 1, removed: 1 });
        assert_eq!(package.get_file_by_hash(tileart_entry_hash(KEPT)).unwrap().data().cloned(), kept_before);
        let added = package.get_file_by_hash(tileart_entry_hash(ADDED)).unwrap();
        assert_eq!(added.compression(), CompressionFlag::Zlib);
        assert_eq!(TileArtEntry::parse_bytes(&added.unpack().unwrap()).unwrap().tile_id, ADDED);
        let edited_bytes = package.unpack_file_by_hash(tileart_entry_hash(EDITED)).unwrap().unwrap();
        assert_eq!(edited_bytes, edited.to_bytes().unwrap());
        assert!(package.get_file_by_hash(tileart_entry_hash(REMOVED)).is_none());
        assert_eq!(tileart_entry_path(ADDED), "build/tileart/10000001.bin");

        assert!(apply_tileart_edits(&mut package, &[keyed(record(KEPT)), keyed(record(KEPT))], &[]).is_err());
        assert!(apply_tileart_edits(&mut package, &[keyed(record(KEPT))], &[tileart_entry_hash(KEPT)]).is_err());

        // A record is stored under the entry it is keyed by, whatever tile id it holds.
        let stored_elsewhere = TileArtEntry { body_type: 401, ..record(KEPT) };
        let edits = [(tileart_entry_hash(EDITED), stored_elsewhere.clone())];
        assert_eq!(apply_tileart_edits(&mut package, &edits, &[]).unwrap().replaced, 1);
        let bytes = package.unpack_file_by_hash(tileart_entry_hash(EDITED)).unwrap().unwrap();
        assert_eq!(bytes, stored_elsewhere.to_bytes().unwrap());
        assert_eq!(package.get_file_by_hash(tileart_entry_hash(KEPT)).unwrap().data().cloned(), kept_before);
    }

    #[test]
    fn tileart_parse_bytes_treats_any_non_zero_alternate_animation_byte_as_set() {
        for byte in [1, 2, u8::MAX] {
            let entry = TileArtEntry::parse_bytes(&minimal_record(byte)).expect("parse minimal record");
            assert!(entry.alternate_animation, "byte {byte}");
        }
    }
}
