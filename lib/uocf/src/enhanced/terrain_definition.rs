//! Reader, writer and checks for `TerrainDefinition.uop` (Enhanced Client and Kingdom Reborn).
//!
//! Each entry `build/terraindefinition/{id}.bin` (decimal, no padding) is the definition of terrain material `id`,
//! the id that sector squares and `legacyterrainmap.csv` (`newId`) refer to. Layout (little endian, traced in
//! `scripts/dev-helpers/ec-research/ghidra-terrain/results/uosa_terrain.md`, `UOTerrain::TerrainDefinition_Read`
//! 0x00845c25):
//!
//! ```text
//! u32 name                 string_dictionary index
//! u32 id                   the material
//! f32 field_34             no reader; 0.0 in every shipped record
//! f32 coverage_exponent    falloff of the layer's coverage blend (0: hard edge)
//! f32 field_3c             no reader; 0.0 in every shipped record
//! u32 n, then n aliases:   {u32 subtype, u32 legacy_land_id, u64 flags}   (Kingdom Reborn: u32 flags)
//! u8  shader block present (1 in every shipped record; 0 ends the record)
//! u8  field_04             no terrain reader; 0 in every shipped record
//! u32 shader name          string_dictionary index (UODefaultTerrainLayer, UOWaterTerrainLayer, UOBumpMapTerrainLayer)
//! u8  k, then k texture items: {u32 path, u8 source_kind, f32 stretch, u32 sampler_index, u32 uv_set}
//! u32 m, then m x i32      techniques (4 in every shipped record)
//! u32 m, then m x f32      tweaks (4 in every shipped record)
//! ```
//!
//! Nothing follows: the parser rejects trailing bytes, and [`TerrainDefinitionEntry::to_bytes`] is its exact inverse
//! (all 201 Enhanced Client and 191 Kingdom Reborn records). A square names `(material, subtype)`; the alias whose
//! `subtype` equals it supplies the cell flags, and the mesh builder dereferences the subtype-0 alias of every
//! drawn material without a null check, so a definition without one crashes the client
//! ([`TerrainDefinitionEntry::validate`], [`check_terrain_references`]).
//!
//! String indices are 1-based into the strings `UoStringDictionary` stores (index 0 is no string): the shipped
//! record 0 names index 110548, the dictionary's string 110547, `void_no_draw`.
//!
//! The package is also authoritative for the land tile ids owned by terrain materials, the world/legacy texture ids
//! that must be excluded from `tex_art_ec` and the primary terrain texture packed into `tex_land_ec`.

crate::eyre_imports!();
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::facet_decoder::DecodedEcFacet;
use crate::enhanced::legacy_terrain_map::LegacyTerrainMap;
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::tileart::TaeFlag;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub const TERRAIN_DEFINITION_UOP_NAME: &str = "TerrainDefinition.uop";
/// Compression of an entry added to a package without sized entries; the client compresses every entry.
const DEFAULT_TERRAIN_DEFINITION_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// The `present` byte of a definition with a shader block.
pub const SHADER_BLOCK_PRESENT: u8 = 1;
/// The `present` byte of a definition without a shader block (never in shipped records).
pub const SHADER_BLOCK_ABSENT: u8 = 0;
/// Technique and tweak values the client keeps (`int[4]`, `float[4]`; `SetTechniqueValue` ignores an index >= 4).
/// Every shipped record has exactly this many.
pub const SHADER_PARAMETER_COUNT: usize = 4;
/// The subtype the mesh builder looks up for every cell of a material.
pub const BASE_SUBTYPE: u32 = 0;
/// The shader names of the shipped records.
pub const DEFAULT_TERRAIN_LAYER_SHADER: &str = "UODefaultTerrainLayer";
pub const WATER_TERRAIN_LAYER_SHADER: &str = "UOWaterTerrainLayer";
pub const BUMP_MAP_TERRAIN_LAYER_SHADER: &str = "UOBumpMapTerrainLayer";
/// String index meaning "no string"; string `index` is the dictionary's string `index - 1`.
const NO_STRING_INDEX: u32 = 0;

// region: --- Alias flags

/// Alias flag bits, in the tiledata / `TaeFlag` layout of the Enhanced Client (u64). The client reads only
/// [`TERRAIN_FLAGS_READ_BY_CLIENT`]; the other bits are kept exactly as tiledata-style metadata.
pub const TERRAIN_FLAG_WALL: u64 = TaeFlag::Wall.bits();
/// Read by `UOTerrain::CollectSurfaces` 0x005f3b20: the land is impassable.
pub const TERRAIN_FLAG_IMPASSABLE: u64 = TaeFlag::Impassable.bits();
pub const TERRAIN_FLAG_WET: u64 = TaeFlag::Wet.bits();
/// Read by `CollectSurfaces`: the land square adds no walk surface.
pub const TERRAIN_FLAG_IGNORED: u64 = TaeFlag::Ignored.bits();
pub const TERRAIN_FLAG_SURFACE: u64 = TaeFlag::Surface.bits();
pub const TERRAIN_FLAG_MONGEN: u64 = TaeFlag::Mongen.bits();
/// Read by `CollectSurfaces` (its mode 3); never set in the shipped data.
pub const TERRAIN_FLAG_HOVER_OVER: u64 = TaeFlag::HoverOver.bits();
/// No client reader (house placement is server side).
pub const TERRAIN_FLAG_NO_HOUSE: u64 = TaeFlag::NoHouse.bits();
/// Read by `UOTerrain::BuildTerrainMesh` 0x00461bdb on the subtype-0 alias: the cell is not drawn.
pub const TERRAIN_FLAG_NO_DRAW: u64 = TaeFlag::NoDraw.bits();
/// The alias flag bits the Enhanced Client reads.
pub const TERRAIN_FLAGS_READ_BY_CLIENT: u64 =
    TERRAIN_FLAG_IMPASSABLE | TERRAIN_FLAG_IGNORED | TERRAIN_FLAG_HOVER_OVER | TERRAIN_FLAG_NO_DRAW;

/// Kingdom Reborn alias flags are u32: the void definition carries `0x4000100` where the Enhanced Client one has
/// `0x200000100`, so KR bit 26 is the EC no-draw bit 33 (high confidence).
pub const KR_TERRAIN_FLAG_NO_DRAW: u64 = 1 << 26;
/// KR bit 19 (11 aliases) is probably the EC no-house bit 32 (medium confidence). The other bits the KR data uses
/// (4, 6, 7, 8) have the EC positions.
pub const KR_TERRAIN_FLAG_NO_HOUSE: u64 = 1 << 19;

/// A named alias flag: its Enhanced Client and Kingdom Reborn masks, and whether the Enhanced Client reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainFlagName {
    pub name: &'static str,
    pub mask: u64,
    pub kr_mask: u64,
    pub read_by_client: bool,
}

/// The flag bits the shipped definitions use or the client reads.
pub const TERRAIN_FLAG_NAMES: [TerrainFlagName; 9] = [
    TerrainFlagName { name: "Wall", mask: TERRAIN_FLAG_WALL, kr_mask: TERRAIN_FLAG_WALL, read_by_client: false },
    TerrainFlagName {
        name: "Impassable",
        mask: TERRAIN_FLAG_IMPASSABLE,
        kr_mask: TERRAIN_FLAG_IMPASSABLE,
        read_by_client: true,
    },
    TerrainFlagName { name: "Wet", mask: TERRAIN_FLAG_WET, kr_mask: TERRAIN_FLAG_WET, read_by_client: false },
    TerrainFlagName { name: "Ignored", mask: TERRAIN_FLAG_IGNORED, kr_mask: TERRAIN_FLAG_IGNORED, read_by_client: true },
    TerrainFlagName { name: "Surface", mask: TERRAIN_FLAG_SURFACE, kr_mask: TERRAIN_FLAG_SURFACE, read_by_client: false },
    TerrainFlagName { name: "Mongen", mask: TERRAIN_FLAG_MONGEN, kr_mask: TERRAIN_FLAG_MONGEN, read_by_client: false },
    TerrainFlagName {
        name: "HoverOver",
        mask: TERRAIN_FLAG_HOVER_OVER,
        kr_mask: TERRAIN_FLAG_HOVER_OVER,
        read_by_client: true,
    },
    TerrainFlagName { name: "NoHouse", mask: TERRAIN_FLAG_NO_HOUSE, kr_mask: KR_TERRAIN_FLAG_NO_HOUSE, read_by_client: false },
    TerrainFlagName { name: "NoDraw", mask: TERRAIN_FLAG_NO_DRAW, kr_mask: KR_TERRAIN_FLAG_NO_DRAW, read_by_client: true },
];

impl TerrainFlagName {
    /// The mask of this flag in `dialect`.
    pub fn mask_in(&self, dialect: ClientDialect) -> u64 {
        match dialect {
            ClientDialect::Enhanced => self.mask,
            ClientDialect::KingdomReborn => self.kr_mask,
        }
    }
}

/// Kingdom Reborn alias flags in the Enhanced Client layout: KR bit 26 moves to bit 33 (no-draw) and KR bit 19 to
/// bit 32 (no-house); every other bit keeps its position.
pub fn kr_terrain_flags_to_enhanced(flags: u64) -> u64 {
    let moved = KR_TERRAIN_FLAG_NO_DRAW | KR_TERRAIN_FLAG_NO_HOUSE;
    let mut enhanced = flags & !moved;
    if flags & KR_TERRAIN_FLAG_NO_DRAW != 0 {
        enhanced |= TERRAIN_FLAG_NO_DRAW;
    }
    if flags & KR_TERRAIN_FLAG_NO_HOUSE != 0 {
        enhanced |= TERRAIN_FLAG_NO_HOUSE;
    }
    enhanced
}

// endregion: --- Alias flags

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TerrainTextureType {
    #[default]
    Undefined,
    WorldArt,
    TileArtLegacy,
    TileArtEnhanced,
    Textures,
}

/// One alias (subtype) of a material: the flags of the cells naming `(material, subtype)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TerrainDefinitionTileAlias {
    /// The subtype squares and `legacyterrainmap.csv` name; equals the alias position in every shipped record.
    pub subtype: u32,
    /// A representative Classic land id mapped to `(material, subtype)` by the csv (0 in 104 aliases); no client
    /// reader, kept for tools.
    pub legacy_land_id: u32,
    /// The cell flags ([`TERRAIN_FLAG_NAMES`]); u32 in Kingdom Reborn records, in the KR layout
    /// ([`kr_terrain_flags_to_enhanced`]).
    pub tile_flags: u64,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TerrainDefinitionTextureLayer {
    /// String index of the texture path (`02000010_Grass_C.tga`, ...).
    pub path_index: u32,
    /// Resolved from the path.
    pub texture_id: Option<u32>,
    /// Resolved from `path_index`.
    pub path: Option<String>,
    /// Resolved from the path.
    pub texture_type: TerrainTextureType,
    /// Read by `UOTerrain::LoadTerrainLayerTexture` 0x004613b0; 0 in every shipped item.
    pub source_kind: u8,
    /// The client's `stretch`: the UV scale is 1 / stretch, so it is the tile repeat (4, 16, 5, 1, 6, ...).
    pub texture_repetition: f32,
    /// Equals the item position in 625 of 657 shipped items (the rest are 4-item blocks `0, 1, 2, 4`); no terrain
    /// reader found.
    pub sampler_index: u32,
    /// 0 in every shipped item.
    pub uv_set: u32,
}

impl TerrainDefinitionTextureLayer {
    pub fn has_preferred_primary_repetition(&self) -> bool {
        (4.0..=8.0).contains(&self.texture_repetition)
    }

    pub fn is_support_layer_by_current_name_heuristic(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };

        let path = path.to_ascii_lowercase();
        path.contains("noise")
            || path.contains("normal")
            || path.contains("mask")
            || path.contains("_alpha")
    }

    pub fn has_support_like_name_clue(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };

        let path = path.to_ascii_lowercase();
        path.contains("alpha")
            || path.contains("mask")
            || path.contains("noise")
            || path.contains("normal")
            || path.contains("bump")
            || path.contains("ripple")
            || path.contains("flow")
            || path.contains("distort")
    }

    pub fn current_primary_selection_reason(&self) -> TerrainDefinitionPrimaryLayerReason {
        match (
            self.is_support_layer_by_current_name_heuristic(),
            self.has_preferred_primary_repetition(),
        ) {
            (false, true) => TerrainDefinitionPrimaryLayerReason::NonSupportPreferredRepetition,
            (false, false) => TerrainDefinitionPrimaryLayerReason::NonSupportRepetitionFallback,
            (true, true) => TerrainDefinitionPrimaryLayerReason::SupportPreferredRepetitionFallback,
            (true, false) => TerrainDefinitionPrimaryLayerReason::SupportRepetitionFallback,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainDefinitionPrimaryLayerReason {
    NonSupportPreferredRepetition,
    NonSupportRepetitionFallback,
    SupportPreferredRepetitionFallback,
    SupportRepetitionFallback,
}

impl TerrainDefinitionPrimaryLayerReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonSupportPreferredRepetition => "non_support_preferred_repetition",
            Self::NonSupportRepetitionFallback => "non_support_repetition_fallback",
            Self::SupportPreferredRepetitionFallback => "support_preferred_repetition_fallback",
            Self::SupportRepetitionFallback => "support_repetition_fallback",
        }
    }
}

/// The shader block (`UOTileArt::ParseShaderBlock` 0x008456a3, shared with tileart).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TerrainDefinitionTexture {
    /// Shader +0x04; no terrain reader, 0 in every shipped record.
    pub field_04: u8,
    /// String index of the layer shader name.
    pub shader_name_index: u32,
    /// Resolved from `shader_name_index`.
    pub shader_name: Option<String>,
    /// The texture items (at most 255).
    pub layers: Vec<TerrainDefinitionTextureLayer>,
    /// The shader techniques (`int[4]`).
    pub techniques: Vec<i32>,
    /// The shader tweaks (`float[4]`).
    pub tweaks: Vec<f32>,
}

impl TerrainDefinitionTexture {
    /// The shader technique values.
    pub fn techniques(&self) -> &[i32] {
        &self.techniques
    }

    pub fn techniques_mut(&mut self) -> &mut Vec<i32> {
        &mut self.techniques
    }

    /// The shader tweak values.
    pub fn tweaks(&self) -> &[f32] {
        &self.tweaks
    }

    pub fn tweaks_mut(&mut self) -> &mut Vec<f32> {
        &mut self.tweaks
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TerrainDefinitionEntry {
    /// String index of the material name (`void_no_draw`, `grass`, ...).
    pub name_index: u32,
    /// The material id; the entry name is `build/terraindefinition/{id}.bin`.
    pub id: u32,
    /// Object +0x34; no reader, 0.0 in every shipped record.
    pub field_34: f32,
    /// The per-layer coverage exponent (object +0x38; `UOTerrain::BuildTerrainCoverage` 0x00461060): positive
    /// values expand ownership with distinct cardinal/diagonal bytes and blend softer and wider; 0 keeps a binary
    /// mask (a hard edge).
    pub coverage_exponent: f32,
    /// Object +0x3c; no reader, 0.0 in every shipped record.
    pub field_3c: f32,
    /// Resolved from `name_index`.
    pub name: Option<String>,
    pub aliases: Vec<TerrainDefinitionTileAlias>,
    /// The shader block; `Some` in every shipped record.
    pub texture: Option<TerrainDefinitionTexture>,
}

#[derive(Debug, Clone, Default)]
pub struct TerrainDefinitionPackage {
    pub entries: Vec<TerrainDefinitionEntry>,
}

impl TerrainDefinitionPackage {
    /// Loads the package at `path` in its own dialect ([`detect_terrain_definition_dialect`]), resolving strings with
    /// the `string_dictionary.uop` next to it when there is one.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let string_dictionary = find_neighboring_string_dictionary(path)
            .map(|dict_path| UoStringDictionary::load(&dict_path))
            .transpose()?;
        let package = UopPackage::load(path)
            .wrap_err_with(|| format!("Failed to load {:?}", path))?;
        Self::from_package(&package, string_dictionary.as_ref())
    }

    pub fn load_with_dictionary(path: &Path, string_dict_path: &Path) -> eyre::Result<Self> {
        let string_dictionary = UoStringDictionary::load(string_dict_path)
            .wrap_err_with(|| format!("Failed to load {:?}", string_dict_path))?;
        let package = UopPackage::load(path)
            .wrap_err_with(|| format!("Failed to load {:?}", path))?;
        Self::from_package(&package, Some(&string_dictionary))
    }

    /// Every definition of `package`, in its own dialect ([`detect_terrain_definition_dialect`]).
    pub fn from_package(
        package: &UopPackage,
        string_dictionary: Option<&UoStringDictionary>,
    ) -> eyre::Result<Self> {
        let dialect = detect_terrain_definition_dialect(package)?;
        Self::from_package_dialect(package, string_dictionary, dialect)
    }

    /// Every definition of `package`, read in `dialect`, sorted by id.
    pub fn from_package_dialect(
        package: &UopPackage,
        string_dictionary: Option<&UoStringDictionary>,
        dialect: ClientDialect,
    ) -> eyre::Result<Self> {
        let mut entries = Vec::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            entries.push(parse_entry_dialect(file, string_dictionary, dialect)?);
        }
        entries.sort_by_key(|entry| entry.id);
        Ok(Self { entries })
    }

    pub fn land_tile_ids(&self) -> BTreeSet<u32> {
        self.entries
            .iter()
            .flat_map(|entry| entry.runtime_slot_ids())
            .collect()
    }

    pub fn land_slot_ids(&self) -> BTreeSet<u32> {
        self.land_tile_ids()
    }

    pub fn land_source_texture_ids(&self) -> BTreeSet<u32> {
        self.entries
            .iter()
            .flat_map(|entry| {
                entry
                    .texture
                    .iter()
                    .flat_map(|texture| texture.layers.iter().filter_map(|layer| layer.texture_id))
            })
            .collect()
    }

    pub fn texture_selections(&self) -> Vec<(u32, u32)> {
        let mut selections = BTreeMap::new();

        for entry in &self.entries {
            let Some(texture_id) = entry.primary_texture_id() else {
                continue;
            };

            for slot_id in entry.runtime_slot_ids() {
                selections.entry(slot_id).or_insert(texture_id);
            }
        }

        selections.into_iter().collect()
    }
}

impl TerrainDefinitionEntry {
    pub fn runtime_slot_ids(&self) -> Vec<u32> {
        let concrete_aliases = self
            .aliases
            .iter()
            .map(|alias| alias.legacy_land_id)
            .filter(|alias| *alias != 0)
            .collect::<Vec<_>>();

        if concrete_aliases.is_empty() {
            vec![self.id]
        } else {
            concrete_aliases
        }
    }

    pub fn primary_texture_id(&self) -> Option<u32> {
        self.primary_texture_layer_with_reason()
            .and_then(|(layer, _)| layer.texture_id)
    }

    pub fn primary_texture_layer_with_reason(
        &self,
    ) -> Option<(&TerrainDefinitionTextureLayer, TerrainDefinitionPrimaryLayerReason)> {
        let texture = self.texture.as_ref()?;

        texture
            .layers
            .iter()
            .filter(|layer| layer.texture_id.is_some())
            .min_by(|left, right| {
                let left_rank = (
                    left.is_support_layer_by_current_name_heuristic(),
                    !left.has_preferred_primary_repetition(),
                    left.sampler_index,
                    left.path_index,
                );
                let right_rank = (
                    right.is_support_layer_by_current_name_heuristic(),
                    !right.has_preferred_primary_repetition(),
                    right.sampler_index,
                    right.path_index,
                );
                left_rank.cmp(&right_rank)
            })
            .map(|layer| (layer, layer.current_primary_selection_reason()))
    }

    /// The coverage exponent (object +0x38).
    pub fn coverage_exponent(&self) -> f32 {
        self.coverage_exponent
    }

    pub fn set_coverage_exponent(&mut self, exponent: f32) {
        self.coverage_exponent = exponent;
    }

    /// The alias of `subtype`, whose flags the cells naming it get.
    pub fn alias(&self, subtype: u32) -> Option<&TerrainDefinitionTileAlias> {
        self.aliases.iter().find(|alias| alias.subtype == subtype)
    }

    /// Parses an Enhanced Client record ([`Self::from_bytes_dialect`]).
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        Self::from_bytes_dialect(bytes, ClientDialect::Enhanced)
    }

    /// Parses one record of `dialect`. The strings are not resolved ([`Self::resolve_strings`]). Fails on a record
    /// that ends early, has bytes after the shader block or a `present` byte other than 0 or 1.
    pub fn from_bytes_dialect(bytes: &[u8], dialect: ClientDialect) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let name_index = reader.read_u32::<LittleEndian>()?;
        let id = reader.read_u32::<LittleEndian>()?;
        let field_34 = reader.read_f32::<LittleEndian>()?;
        let coverage_exponent = reader.read_f32::<LittleEndian>()?;
        let field_3c = reader.read_f32::<LittleEndian>()?;
        let alias_count = reader.read_u32::<LittleEndian>()?;
        let mut aliases = Vec::new();
        for _ in 0..alias_count {
            let subtype = reader.read_u32::<LittleEndian>()?;
            let legacy_land_id = reader.read_u32::<LittleEndian>()?;
            let tile_flags = match dialect {
                ClientDialect::Enhanced => reader.read_u64::<LittleEndian>()?,
                ClientDialect::KingdomReborn => u64::from(reader.read_u32::<LittleEndian>()?),
            };
            aliases.push(TerrainDefinitionTileAlias { subtype, legacy_land_id, tile_flags });
        }
        let texture = match reader.read_u8()? {
            SHADER_BLOCK_ABSENT => None,
            SHADER_BLOCK_PRESENT => Some(read_shader_block(&mut reader)?),
            other => eyre::bail!("material {id}: shader block present byte {other} is neither 0 nor 1"),
        };
        let consumed = reader.position() as usize;
        eyre::ensure!(
            consumed == bytes.len(),
            "material {id}: {} bytes after the shader block",
            bytes.len() - consumed
        );
        Ok(Self {
            name_index,
            id,
            field_34,
            coverage_exponent,
            field_3c,
            name: None,
            aliases,
            texture,
        })
    }

    /// The Enhanced Client record ([`Self::to_bytes_dialect`]).
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        self.to_bytes_dialect(ClientDialect::Enhanced)
    }

    /// The record in `dialect`; the exact inverse of [`Self::from_bytes_dialect`]. The resolved strings are not
    /// written, only the indices. Fails on more than 255 texture items, more than `u32::MAX` aliases or values, or
    /// (Kingdom Reborn) flags above 32 bits.
    pub fn to_bytes_dialect(&self, dialect: ClientDialect) -> eyre::Result<Vec<u8>> {
        let id = self.id;
        let mut out = Vec::new();
        out.write_u32::<LittleEndian>(self.name_index)?;
        out.write_u32::<LittleEndian>(id)?;
        out.write_f32::<LittleEndian>(self.field_34)?;
        out.write_f32::<LittleEndian>(self.coverage_exponent())?;
        out.write_f32::<LittleEndian>(self.field_3c)?;
        out.write_u32::<LittleEndian>(count_u32(self.aliases.len(), id, "aliases")?)?;
        for alias in &self.aliases {
            out.write_u32::<LittleEndian>(alias.subtype)?;
            out.write_u32::<LittleEndian>(alias.legacy_land_id)?;
            match dialect {
                ClientDialect::Enhanced => out.write_u64::<LittleEndian>(alias.tile_flags)?,
                ClientDialect::KingdomReborn => {
                    let flags = u32::try_from(alias.tile_flags).map_err(|_| {
                        eyre!(
                            "material {id} subtype {}: flags {:#x} do not fit the 32-bit Kingdom Reborn alias",
                            alias.subtype,
                            alias.tile_flags
                        )
                    })?;
                    out.write_u32::<LittleEndian>(flags)?;
                }
            }
        }
        let Some(texture) = &self.texture else {
            out.write_u8(SHADER_BLOCK_ABSENT)?;
            return Ok(out);
        };
        out.write_u8(SHADER_BLOCK_PRESENT)?;
        out.write_u8(texture.field_04)?;
        out.write_u32::<LittleEndian>(texture.shader_name_index)?;
        let layer_count = u8::try_from(texture.layers.len())
            .map_err(|_| eyre!("material {id}: {} texture items, at most {}", texture.layers.len(), u8::MAX))?;
        out.write_u8(layer_count)?;
        for layer in &texture.layers {
            out.write_u32::<LittleEndian>(layer.path_index)?;
            out.write_u8(layer.source_kind)?;
            out.write_f32::<LittleEndian>(layer.texture_repetition)?;
            out.write_u32::<LittleEndian>(layer.sampler_index)?;
            out.write_u32::<LittleEndian>(layer.uv_set)?;
        }
        out.write_u32::<LittleEndian>(count_u32(texture.techniques().len(), id, "techniques")?)?;
        for value in texture.techniques() {
            out.write_i32::<LittleEndian>(*value)?;
        }
        out.write_u32::<LittleEndian>(count_u32(texture.tweaks().len(), id, "tweaks")?)?;
        for value in texture.tweaks() {
            out.write_f32::<LittleEndian>(*value)?;
        }
        Ok(out)
    }

    /// Fills `name`, `shader_name` and each layer's `path`, `texture_id` and `texture_type` from the string indices.
    pub fn resolve_strings(&mut self, string_dictionary: Option<&UoStringDictionary>) {
        self.name = resolve_dictionary_string(string_dictionary, self.name_index);
        if let Some(texture) = &mut self.texture {
            texture.shader_name = resolve_dictionary_string(string_dictionary, texture.shader_name_index);
            for layer in &mut texture.layers {
                layer.path = resolve_dictionary_string(string_dictionary, layer.path_index);
                layer.texture_type = classify_texture_path(layer.path.as_deref());
                layer.texture_id = layer.path.as_deref().and_then(crate::utils::path::extract_texture_id_from_path);
            }
        }
    }

    /// The definition with Kingdom Reborn alias flags moved to the Enhanced Client layout
    /// ([`kr_terrain_flags_to_enhanced`]); everything else is the same in both layouts.
    pub fn kr_to_enhanced(&self) -> Self {
        let mut converted = self.clone();
        for alias in &mut converted.aliases {
            alias.tile_flags = kr_terrain_flags_to_enhanced(alias.tile_flags);
        }
        converted
    }

    /// What the client needs of one definition: a subtype-0 alias, unique subtypes (the shipped data has subtype ==
    /// position), a shader block with 4 techniques and 4 tweaks.
    pub fn validate(&self) -> Vec<TerrainDefinitionIssue> {
        let material = self.id;
        let mut issues = Vec::new();
        if self.alias(BASE_SUBTYPE).is_none() {
            issues.push(TerrainDefinitionIssue::NoBaseSubtype { material });
        }
        let mut seen = BTreeSet::new();
        for (position, alias) in self.aliases.iter().enumerate() {
            if !seen.insert(alias.subtype) {
                issues.push(TerrainDefinitionIssue::DuplicateSubtype { material, subtype: alias.subtype });
            }
            if u32::try_from(position).ok() != Some(alias.subtype) {
                issues.push(TerrainDefinitionIssue::SubtypeNotAtPosition { material, position, subtype: alias.subtype });
            }
        }
        match &self.texture {
            None => issues.push(TerrainDefinitionIssue::NoShaderBlock { material }),
            Some(texture) => {
                if texture.techniques().len() != SHADER_PARAMETER_COUNT || texture.tweaks().len() != SHADER_PARAMETER_COUNT {
                    issues.push(TerrainDefinitionIssue::ShaderParameterCount {
                        material,
                        techniques: texture.techniques().len(),
                        tweaks: texture.tweaks().len(),
                    });
                }
            }
        }
        issues
    }
}

fn count_u32(len: usize, material: u32, what: &str) -> eyre::Result<u32> {
    u32::try_from(len).map_err(|_| eyre!("material {material}: {len} {what} do not fit a u32 count"))
}

fn read_shader_block(reader: &mut Cursor<&[u8]>) -> eyre::Result<TerrainDefinitionTexture> {
    let field_04 = reader.read_u8()?;
    let shader_name_index = reader.read_u32::<LittleEndian>()?;
    let layer_count = reader.read_u8()?;
    let mut layers = Vec::with_capacity(usize::from(layer_count));
    for _ in 0..layer_count {
        layers.push(TerrainDefinitionTextureLayer {
            path_index: reader.read_u32::<LittleEndian>()?,
            source_kind: reader.read_u8()?,
            texture_repetition: reader.read_f32::<LittleEndian>()?,
            sampler_index: reader.read_u32::<LittleEndian>()?,
            uv_set: reader.read_u32::<LittleEndian>()?,
            ..TerrainDefinitionTextureLayer::default()
        });
    }
    let technique_count = reader.read_u32::<LittleEndian>()?;
    let mut techniques = Vec::new();
    for _ in 0..technique_count {
        techniques.push(reader.read_i32::<LittleEndian>()?);
    }
    let tweak_count = reader.read_u32::<LittleEndian>()?;
    let mut tweaks = Vec::new();
    for _ in 0..tweak_count {
        tweaks.push(reader.read_f32::<LittleEndian>()?);
    }
    Ok(TerrainDefinitionTexture { field_04, shader_name_index, shader_name: None, layers, techniques, tweaks })
}

/// Parses an Enhanced Client entry and resolves its strings ([`parse_entry_dialect`]).
pub fn parse_entry(
    uop_file: &UopFile,
    string_dictionary: Option<&UoStringDictionary>,
) -> eyre::Result<TerrainDefinitionEntry> {
    parse_entry_dialect(uop_file, string_dictionary, ClientDialect::Enhanced)
}

/// Parses the entry `uop_file` of `dialect` and resolves its strings with `string_dictionary`.
pub fn parse_entry_dialect(
    uop_file: &UopFile,
    string_dictionary: Option<&UoStringDictionary>,
    dialect: ClientDialect,
) -> eyre::Result<TerrainDefinitionEntry> {
    let bytes = uop_file.unpack_arc()?;
    let mut entry = TerrainDefinitionEntry::from_bytes_dialect(&bytes, dialect)
        .wrap_err_with(|| format!("TerrainDefinition entry {:016X}", uop_file.filename_hash()))?;
    entry.resolve_strings(string_dictionary);
    Ok(entry)
}

/// The dialect every sized entry of `package` parses in: Enhanced Client when all do, else Kingdom Reborn when all
/// do. Fails when neither reads every entry.
pub fn detect_terrain_definition_dialect(package: &UopPackage) -> eyre::Result<ClientDialect> {
    let payloads = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.unpack_arc())
        .collect::<Result<Vec<_>, _>>()?;
    let mut first_error = None;
    for dialect in [ClientDialect::Enhanced, ClientDialect::KingdomReborn] {
        match payloads.iter().try_for_each(|payload| TerrainDefinitionEntry::from_bytes_dialect(payload, dialect).map(drop)) {
            Ok(()) => return Ok(dialect),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    Err(first_error.expect("both dialects were tried"))
        .wrap_err("the TerrainDefinition entries parse neither as Enhanced Client nor as Kingdom Reborn records")
}

/// The entry path of the definition of material `id`.
pub fn terrain_definition_entry_path(id: u32) -> String {
    format!("build/terraindefinition/{id}.bin")
}

/// The entry hash of the definition of material `id`.
pub fn terrain_definition_entry_hash(id: u32) -> u64 {
    hash_file_name_single(&terrain_definition_entry_path(id))
}

/// Every definition of `package` in `dialect` by material id, with resolved strings. Fails on an entry that does
/// not parse or that is not the entry of its own id.
pub fn read_terrain_definitions(
    package: &UopPackage,
    string_dictionary: Option<&UoStringDictionary>,
    dialect: ClientDialect,
) -> eyre::Result<BTreeMap<u32, TerrainDefinitionEntry>> {
    let mut definitions = BTreeMap::new();
    for file in package.iter_files().filter(|file| file.has_size()) {
        let entry = parse_entry_dialect(file, string_dictionary, dialect)?;
        let id = entry.id;
        eyre::ensure!(
            terrain_definition_entry_hash(id) == file.filename_hash(),
            "TerrainDefinition entry {:016X} holds material {id}, whose entry is {}",
            file.filename_hash(),
            terrain_definition_entry_path(id)
        );
        definitions.insert(id, entry);
    }
    Ok(definitions)
}

// region: --- Package edits

/// What [`apply_terrain_definition_edits`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerrainDefinitionChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

/// Stores each of `definitions` (encoded in `dialect`) as the entry of its material id and removes the entries of
/// the `removed` ids; every other entry is left untouched (same compressed bytes and data block header). A
/// definition whose entry already holds its bytes is left untouched; a changed one is replaced keeping its
/// compression flag; a new one is added with the compression of the package's entries (zlib for an empty package),
/// through the classic-zlib UOP writer. Replaced and added entries get a fresh timestamp data block header. Fails on
/// an id listed twice or both stored and removed; a removed id without an entry is not counted.
pub fn apply_terrain_definition_edits(
    package: &mut UopPackage,
    definitions: &[TerrainDefinitionEntry],
    removed: &[u32],
    dialect: ClientDialect,
) -> eyre::Result<TerrainDefinitionChanges> {
    let mut stored = BTreeSet::new();
    for definition in definitions {
        let id = definition.id;
        eyre::ensure!(stored.insert(id), "material {id} is listed twice");
        eyre::ensure!(!removed.contains(&id), "material {id} is both stored and removed");
    }
    let compression = package
        .iter_files()
        .find(|file| file.has_size())
        .map_or(DEFAULT_TERRAIN_DEFINITION_COMPRESSION, |file| file.compression());
    let mut changes = TerrainDefinitionChanges::default();
    for definition in definitions {
        let hash = terrain_definition_entry_hash(definition.id);
        let payload = definition.to_bytes_dialect(dialect)?;
        let current = package
            .unpack_file_by_hash(hash)
            .wrap_err_with(|| format!("failed to unpack TerrainDefinition entry {}", definition.id))?;
        if current.as_deref() == Some(payload.as_slice()) {
            continue;
        }
        match uop_entries::put_entry(package, hash, &payload, compression)? {
            EntryPut::Replaced => changes.replaced += 1,
            EntryPut::Added => changes.added += 1,
        }
    }
    for id in removed.iter().copied().collect::<BTreeSet<_>>() {
        if uop_entries::remove_entry(package, terrain_definition_entry_hash(id)) {
            changes.removed += 1;
        }
    }
    Ok(changes)
}

/// Writes a copy of the `source` `TerrainDefinition.uop` to `output` with [`apply_terrain_definition_edits`] applied
/// (stamped with the current time); the package keeps its version, block size and layout. `output` must not be
/// `source`.
pub fn write_terrain_definition_package(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    definitions: &[TerrainDefinitionEntry],
    removed: &[u32],
    dialect: ClientDialect,
) -> eyre::Result<TerrainDefinitionChanges> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let changes = apply_terrain_definition_edits(&mut package, definitions, removed, dialect)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(changes)
}

// endregion: --- Package edits

// region: --- Checks

/// A `(material, subtype)` pair a square or a `legacyterrainmap.csv` row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerrainReference {
    pub material: u32,
    pub subtype: u32,
}

/// A definition the client would mishandle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerrainDefinitionIssue {
    /// No alias has subtype 0: `BuildTerrainMesh` dereferences it without a null check, the client crashes.
    NoBaseSubtype { material: u32 },
    /// Two aliases have the subtype: only the first is found.
    DuplicateSubtype { material: u32, subtype: u32 },
    /// The subtype differs from the alias position (never in the shipped data; the lookup is by value).
    SubtypeNotAtPosition { material: u32, position: usize, subtype: u32 },
    /// No shader block: the material has no layer to draw (never in the shipped data).
    NoShaderBlock { material: u32 },
    /// The shader holds `int[4]` techniques and `float[4]` tweaks: extra values are lost, missing ones stay 0.
    ShaderParameterCount { material: u32, techniques: usize, tweaks: usize },
    /// Squares or csv rows name a material without a definition: the mesh builder crashes.
    MissingMaterial { reference: TerrainReference, uses: u64 },
    /// Squares or csv rows name a subtype the material lacks: their cells get flags 0 (walkable, drawn).
    MissingSubtype { reference: TerrainReference, uses: u64 },
}

impl TerrainDefinitionIssue {
    /// True for the issues that crash the client.
    pub fn crashes_client(&self) -> bool {
        matches!(self, Self::NoBaseSubtype { .. } | Self::MissingMaterial { .. })
    }
}

impl std::fmt::Display for TerrainDefinitionIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBaseSubtype { material } => write!(f, "material {material} has no subtype-0 alias (the client crashes)"),
            Self::DuplicateSubtype { material, subtype } => write!(f, "material {material} repeats subtype {subtype}"),
            Self::SubtypeNotAtPosition { material, position, subtype } => {
                write!(f, "material {material}: alias {position} has subtype {subtype}")
            }
            Self::NoShaderBlock { material } => write!(f, "material {material} has no shader block"),
            Self::ShaderParameterCount { material, techniques, tweaks } => write!(
                f,
                "material {material}: {techniques} techniques and {tweaks} tweaks, the client keeps {SHADER_PARAMETER_COUNT} of each"
            ),
            Self::MissingMaterial { reference, uses } => write!(
                f,
                "material {} (subtype {}) is used {uses} times but has no definition (the client crashes)",
                reference.material, reference.subtype
            ),
            Self::MissingSubtype { reference, uses } => write!(
                f,
                "material {} has no subtype {}, used {uses} times (the cells get flags 0)",
                reference.material, reference.subtype
            ),
        }
    }
}

/// [`TerrainDefinitionEntry::validate`] of every definition.
pub fn validate_terrain_definitions(definitions: &BTreeMap<u32, TerrainDefinitionEntry>) -> Vec<TerrainDefinitionIssue> {
    definitions.values().flat_map(TerrainDefinitionEntry::validate).collect()
}

/// The `(material, subtype)` pairs used by sector squares and `legacyterrainmap.csv` rows, with their use counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerrainReferences {
    pub uses: BTreeMap<TerrainReference, u64>,
    /// Classic land ids of Enhanced Client squares that have no `legacyterrainmap.csv` route, with their counts.
    pub unrouted_legacy_land: BTreeMap<u16, u64>,
}

impl TerrainReferences {
    pub fn add(&mut self, reference: TerrainReference, uses: u64) {
        *self.uses.entry(reference).or_default() += uses;
    }

    /// Every route of the csv, counted once.
    pub fn add_legacy_terrain_map(&mut self, map: &LegacyTerrainMap) {
        for route in map.routes.values() {
            self.add(TerrainReference { material: route.material_id, subtype: route.subtype }, 1);
        }
    }

    /// Every square of `sector`: a Kingdom Reborn square names its material and subtype; an Enhanced Client square
    /// names a Classic land id, routed through `map` (the client's `legacyterrainmap.csv`).
    pub fn add_sector(&mut self, sector: &DecodedEcFacet, map: &LegacyTerrainMap) {
        for tile in &sector.tiles {
            let reference = match &tile.kr {
                Some(kr) => TerrainReference { material: u32::from(tile.land_graphic), subtype: u32::from(kr.subtype) },
                None => match map.routes.get(&tile.land_graphic) {
                    Some(route) => TerrainReference { material: route.material_id, subtype: route.subtype },
                    None => {
                        *self.unrouted_legacy_land.entry(tile.land_graphic).or_default() += 1;
                        continue;
                    }
                },
            };
            self.add(reference, 1);
        }
    }
}

/// The references of `references` that `definitions` lack: a missing material, or a missing subtype.
pub fn check_terrain_references(
    definitions: &BTreeMap<u32, TerrainDefinitionEntry>,
    references: &TerrainReferences,
) -> Vec<TerrainDefinitionIssue> {
    references
        .uses
        .iter()
        .filter_map(|(&reference, &uses)| match definitions.get(&reference.material) {
            None => Some(TerrainDefinitionIssue::MissingMaterial { reference, uses }),
            Some(definition) if definition.alias(reference.subtype).is_none() => {
                Some(TerrainDefinitionIssue::MissingSubtype { reference, uses })
            }
            Some(_) => None,
        })
        .collect()
}

// endregion: --- Checks

fn resolve_dictionary_string(
    string_dictionary: Option<&UoStringDictionary>,
    string_index: u32,
) -> Option<String> {
    if string_index == NO_STRING_INDEX {
        return None;
    }
    string_dictionary
        .and_then(|dict| dict.get_string((string_index - 1) as usize))
        .map(str::to_owned)
}

fn classify_texture_path(path: Option<&str>) -> TerrainTextureType {
    match path.map(crate::utils::path::normalize_dictionary_path) {
        Some(value) if value.contains("data\\worldart\\") => TerrainTextureType::WorldArt,
        Some(value) if value.contains("data\\tileartlegacy\\") => {
            TerrainTextureType::TileArtLegacy
        }
        Some(value) if value.contains("data\\tileartenhanced\\") => {
            TerrainTextureType::TileArtEnhanced
        }
        Some(value) if value.contains("data\\textures\\") => TerrainTextureType::Textures,
        _ => TerrainTextureType::Undefined,
    }
}


fn find_neighboring_string_dictionary(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    ["string_dictionary.uop"]
        .into_iter()
        .map(|name| parent.join(name))
        .find(|candidate| candidate.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATERIAL: u32 = 7;
    const NAME_INDEX: u32 = 3;
    const SHADER_INDEX: u32 = 5;
    const LAYER_PATH_INDEX: u32 = 9;
    const COVERAGE_EXPONENT: f32 = 0.25;
    const STRETCH: f32 = 16.0;
    const TWEAK: f32 = -0.1;
    const LEGACY_LAND: u32 = 168;
    const KR_FLAGS: u64 = KR_TERRAIN_FLAG_NO_DRAW | TERRAIN_FLAG_IGNORED;

    fn definition() -> TerrainDefinitionEntry {
        TerrainDefinitionEntry {
            name_index: NAME_INDEX,
            id: MATERIAL,
            coverage_exponent: COVERAGE_EXPONENT,
            aliases: vec![
                TerrainDefinitionTileAlias { subtype: 0, legacy_land_id: LEGACY_LAND, tile_flags: TERRAIN_FLAG_IMPASSABLE },
                TerrainDefinitionTileAlias { subtype: 1, legacy_land_id: 0, tile_flags: TERRAIN_FLAG_NO_DRAW },
            ],
            texture: Some(TerrainDefinitionTexture {
                shader_name_index: SHADER_INDEX,
                layers: vec![TerrainDefinitionTextureLayer {
                    path_index: LAYER_PATH_INDEX,
                    texture_repetition: STRETCH,
                    sampler_index: 0,
                    ..TerrainDefinitionTextureLayer::default()
                }],
                techniques: vec![0; SHADER_PARAMETER_COUNT],
                tweaks: vec![TWEAK, 0.0, 0.0, 0.0],
                ..TerrainDefinitionTexture::default()
            }),
            ..TerrainDefinitionEntry::default()
        }
    }

    #[test]
    fn records_round_trip_in_both_dialects() {
        let mut definition = definition();
        let bytes = definition.to_bytes().unwrap();
        let ec_alias_size = 2 * size_of::<u32>() + size_of::<u64>();
        assert_eq!(TerrainDefinitionEntry::from_bytes(&bytes).unwrap(), definition);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(TerrainDefinitionEntry::from_bytes(&trailing).is_err(), "trailing bytes are rejected");
        assert!(TerrainDefinitionEntry::from_bytes(&bytes[..bytes.len() - 1]).is_err(), "a short record is rejected");

        assert!(definition.to_bytes_dialect(ClientDialect::KingdomReborn).is_err(), "EC no-draw does not fit 32 bits");
        definition.aliases[1].tile_flags = KR_FLAGS;
        let kr = definition.to_bytes_dialect(ClientDialect::KingdomReborn).unwrap();
        let kr_alias_size = 3 * size_of::<u32>();
        assert_eq!(bytes.len() - kr.len(), definition.aliases.len() * (ec_alias_size - kr_alias_size));
        assert_eq!(TerrainDefinitionEntry::from_bytes_dialect(&kr, ClientDialect::KingdomReborn).unwrap(), definition);
        assert!(TerrainDefinitionEntry::from_bytes(&kr).is_err(), "a KR record does not parse as EC");

        definition.texture = None;
        let bytes = definition.to_bytes().unwrap();
        assert_eq!(bytes.last(), Some(&SHADER_BLOCK_ABSENT));
        assert_eq!(TerrainDefinitionEntry::from_bytes(&bytes).unwrap(), definition);
    }

    #[test]
    fn kr_flags_convert_to_the_enhanced_layout() {
        assert_eq!(kr_terrain_flags_to_enhanced(KR_FLAGS), TERRAIN_FLAG_NO_DRAW | TERRAIN_FLAG_IGNORED);
        assert_eq!(kr_terrain_flags_to_enhanced(KR_TERRAIN_FLAG_NO_HOUSE), TERRAIN_FLAG_NO_HOUSE);
        assert_eq!(kr_terrain_flags_to_enhanced(TERRAIN_FLAG_WET), TERRAIN_FLAG_WET);
        let mut definition = definition();
        definition.aliases[1].tile_flags = KR_FLAGS;
        assert_eq!(definition.kr_to_enhanced().aliases[1].tile_flags, TERRAIN_FLAG_NO_DRAW | TERRAIN_FLAG_IGNORED);
    }

    #[test]
    fn validation_reports_what_the_client_needs() {
        let mut definition = definition();
        assert!(definition.validate().is_empty());
        definition.aliases[0].subtype = 1;
        definition.texture.as_mut().unwrap().techniques.push(0);
        let issues = definition.validate();
        assert!(issues.contains(&TerrainDefinitionIssue::NoBaseSubtype { material: MATERIAL }));
        assert!(issues.contains(&TerrainDefinitionIssue::DuplicateSubtype { material: MATERIAL, subtype: 1 }));
        assert!(issues.contains(&TerrainDefinitionIssue::SubtypeNotAtPosition { material: MATERIAL, position: 0, subtype: 1 }));
        assert!(issues.contains(&TerrainDefinitionIssue::ShaderParameterCount {
            material: MATERIAL,
            techniques: SHADER_PARAMETER_COUNT + 1,
            tweaks: SHADER_PARAMETER_COUNT
        }));
        assert!(issues.iter().any(TerrainDefinitionIssue::crashes_client));

        let definitions = BTreeMap::from([(MATERIAL, self::definition())]);
        let map = LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n3,7,0\n4,7,2\n5,8,0\n").unwrap();
        let mut references = TerrainReferences::default();
        references.add_legacy_terrain_map(&map);
        let missing_subtype = TerrainReference { material: MATERIAL, subtype: 2 };
        let missing_material = TerrainReference { material: 8, subtype: 0 };
        assert_eq!(
            check_terrain_references(&definitions, &references),
            vec![
                TerrainDefinitionIssue::MissingSubtype { reference: missing_subtype, uses: 1 },
                TerrainDefinitionIssue::MissingMaterial { reference: missing_material, uses: 1 },
            ]
        );
    }

    #[test]
    fn package_edits_replace_add_and_remove() {
        const ADDED: u32 = 8;
        let mut package = UopPackage::new_default();
        let definition = definition();
        package
            .add_file_from_memory(&definition.to_bytes().unwrap(), &terrain_definition_entry_path(MATERIAL), CompressionFlag::Zlib)
            .unwrap();
        assert_eq!(read_terrain_definitions(&package, None, ClientDialect::Enhanced).unwrap()[&MATERIAL], definition);
        assert_eq!(detect_terrain_definition_dialect(&package).unwrap(), ClientDialect::Enhanced);

        let unchanged = apply_terrain_definition_edits(&mut package, &[definition.clone()], &[], ClientDialect::Enhanced).unwrap();
        assert_eq!(unchanged, TerrainDefinitionChanges::default());
        let mut edited = definition.clone();
        edited.set_coverage_exponent(1.0);
        let added = TerrainDefinitionEntry { id: ADDED, ..definition.clone() };
        let changes = apply_terrain_definition_edits(&mut package, &[edited.clone(), added], &[], ClientDialect::Enhanced).unwrap();
        assert_eq!(changes, TerrainDefinitionChanges { replaced: 1, added: 1, removed: 0 });
        let definitions = read_terrain_definitions(&package, None, ClientDialect::Enhanced).unwrap();
        assert_eq!(definitions[&MATERIAL].coverage_exponent(), 1.0);
        let changes = apply_terrain_definition_edits(&mut package, &[], &[ADDED, ADDED], ClientDialect::Enhanced).unwrap();
        assert_eq!(changes.removed, 1);
        assert!(apply_terrain_definition_edits(&mut package, &[edited], &[MATERIAL], ClientDialect::Enhanced).is_err());
    }
}
