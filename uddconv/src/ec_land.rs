//! Build-time and runtime support for `ec_land.uddp`.
//!
//! Sources one primary EC terrain texture per terrain tile id, using
//! `TerrainDefinition.uop` aliases as the slot ids and the layered terrain
//! material block as the source-texture selector.
//! Package layout:
//! - `pages/{page_index}.rgba8888` or `pages/{page_index}.bc7`: atlas page payloads.
//! - `metadata/pages.bin`: page table with atlas dimensions, occupancy and pixel format.
//! - `metadata/slots.bin`: sparse slot table with one record per land art_id.
//! - `metadata/terrain_provenance.bin`: required terrain-material provenance for each
//!   alias slot, preserving how `TerrainDefinition.uop` collapsed into packed land slots.
//!
//! This package is intentionally richer than a plain atlas:
//! - the slot table is the runtime lookup surface used by the renderer.
//! - the page table describes the packed atlas payloads backing those slots.
//! - the terrain provenance table is required metadata that preserves the semantic
//!   path from TerrainDefinition material entries to alias slots, selected texture ids,
//!   and canonical packed slots.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};

use crate::bc7::{
    encode_for_vram, preferred_bc7_encoder_backend, ImageExtent, RawImageFormat,
    VramTextureEncoding,
};
use crate::cc_art::PagePixelFormat;
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use uocf::{
    enhanced::{
        terrain_definition::TerrainDefinitionPackage,
        textures::{ECImageFormat, Textures},
    },
    udd::{
        xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
        LookupMode, UddpBuilder, UddpReader,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerrainTextureSelection {
    slot_id: u32,
    texture_id: u32,
}

pub struct SlotAlias {
    pub art_id: u32,
    pub canonical_art_id: u32,
}

#[derive(Debug, Clone)]
struct DecodedLayerTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"ELPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"ELSL";
const TERRAIN_PROVENANCE_MAGIC: [u8; 4] = *b"ELTP";
/// Bump version when the binary layout of either manifest changes.
const EC_LAND_METADATA_VERSION: u32 = 2;
const EC_LAND_TERRAIN_PROVENANCE_VERSION: u32 = 1;
pub const UDDP_PAGE_MANIFEST_ENTRY_VPATH: &str = "metadata/pages.bin";
pub const UDDP_SLOT_MANIFEST_ENTRY_VPATH: &str = "metadata/slots.bin";
pub const UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH: &str = "metadata/terrain_provenance.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;
pub const MISSING_TEXTURE_ID: u32 = u32::MAX;
pub const MISSING_SLOT_ID: u32 = u32::MAX;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcLandAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    /// When `true` each atlas page is BC7-compressed on the CPU before being
    /// stored in the UDDP container, reducing VRAM usage by ~8×.
    pub use_bc7: bool,
}

impl Default for EcLandAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            use_bc7: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcLandBuildSummary {
    pub terrain_entry_count: u32,
    pub terrain_alias_ref_count: u32,
    pub unique_alias_slot_count: u32,
    pub unique_source_texture_count: u32,
    pub unique_texture_selection_count: u32,
    pub unique_packed_texture_count: u32,
    pub ignored_source_texture_ids: Vec<u32>,
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcLandTerrainProvenanceRecord {
    pub material_id: u32,
    pub material_name_id: i32,
    pub alias_count_index: u32,
    pub alias_slot_id: u32,
    pub alias_tile_flags: u64,
    pub selected_texture_id: u32,
    pub canonical_slot_id: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtTileKind {
    Land,
    Static,
}

impl ArtTileKind {
    fn slot_flags(self) -> u16 {
        match self {
            Self::Land => SLOT_FLAG_PRESENT | SLOT_FLAG_LAND,
            Self::Static => SLOT_FLAG_PRESENT | SLOT_FLAG_STATIC,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedArtTile {
    pub art_id: u32,
    pub kind: ArtTileKind,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcLandPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    /// Pixel format of the page payload stored in the UDDP container.
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcLandSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl EcLandSlotRecord {
    fn absent(art_id: u32) -> Self {
        Self {
            art_id,
            page_index: MISSING_PAGE_INDEX,
            page_tile_index: MISSING_PAGE_TILE_INDEX,
            flags: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }

    pub fn is_present(self) -> bool {
        (self.flags & SLOT_FLAG_PRESENT) != 0
    }

    pub fn is_land(self) -> bool {
        (self.flags & SLOT_FLAG_LAND) != 0
    }

    pub fn is_static(self) -> bool {
        (self.flags & SLOT_FLAG_STATIC) != 0
    }
}

#[derive(Debug, Clone)]
pub struct PlacedTile {
    pub art_id: u32,
    pub kind: ArtTileKind,
    pub page_tile_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct BuiltPage {
    pub record: EcLandPageRecord,
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

pub struct EcLandPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    pages: Vec<EcLandPageRecord>,
    slots: Vec<EcLandSlotRecord>,
    terrain_provenance: Vec<EcLandTerrainProvenanceRecord>,
    transcode: HashMap<u32, u32>,
}

impl EcLandPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::open(fs::read(path.as_ref())?)
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let page_manifest = read_path_entry(&package, UDDP_PAGE_MANIFEST_ENTRY_VPATH)
            .context("ec_land.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, UDDP_SLOT_MANIFEST_ENTRY_VPATH)
            .context("ec_land.uddp missing metadata/slots.bin")?;
        let terrain_provenance_manifest =
            read_path_entry(&package, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH)
                .context("ec_land.uddp missing metadata/terrain_provenance.bin")?;

        let (page_width, page_height, page_gutter, pages) = parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slots) = parse_slot_manifest(&slot_manifest)?;
        let terrain_provenance = parse_terrain_provenance_manifest(&terrain_provenance_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("ec_land metadata headers disagree on atlas dimensions or gutter");
        }

        let transcode = read_transcode_from_package(&package).unwrap_or_default();
        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            pages,
            slots,
            terrain_provenance,
            transcode,
        })
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    pub fn atlas_width(&self) -> u32 {
        self.atlas_width
    }

    pub fn atlas_height(&self) -> u32 {
        self.atlas_height
    }

    pub fn gutter(&self) -> u16 {
        self.gutter
    }

    pub fn pages(&self) -> &[EcLandPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[EcLandSlotRecord] {
        &self.slots
    }

    pub fn terrain_provenance(&self) -> &[EcLandTerrainProvenanceRecord] {
        &self.terrain_provenance
    }

    pub fn slot_record(&self, art_id: u32) -> Option<&EcLandSlotRecord> {
        self.slots.get(art_id as usize)
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&EcLandSlotRecord> {
        self.slot_record(art_id).filter(|slot| slot.is_present())
    }

    /// Resolves a runtime terrain id to the packed EC land slot the renderer should sample.
    ///
    /// For CC-sourced map data, the terrain id is a CC tile ID. We must apply the transcode
    /// mapping unconditionally before attempting any EC slot lookups to avoid namespace collisions
    /// (where a CC tile ID accidentally equals an EC alias slot ID of a different terrain type).
    ///
    /// Resolution order:
    /// Resolves a Classic Client (CC) Tile ID to a packed EC Slot ID (atlas index).
    ///
    /// This function implements the CC ➔ EC Texture Resolution Chain:
    /// 1. **KDL Lookup**: Maps CC Tile ID ➔ EC Material ID (via TerrainTranscode.kdl).
    /// 2. **Provenance Lookup**: Maps EC Material ID ➔ EC Canonical/Alias Slot ID (via TerrainDefinition.uop data).
    /// 3. **Slot Mapping**: The returned ID is an index into the UDDP `slots` table, which holds atlas UVs.
    ///
    /// # Logic Steps:
    /// - **Step 1 (Transcode)**: Check if there's an explicit CC➔EC Material mapping. If so,
    ///   resolve that material to its best available texture slot (Canonical or Alias).
    /// - **Step 2 (Legacy Alias)**: If no transcode exists, check if the CC ID itself matches
    ///   a "legacy alias" slot in the EC provenance records.
    /// - **Step 3 (Direct Hit)**: Check if the CC ID matches a physical slot ID directly
    ///   (used for legacy textures that kept their original IDs).
    pub fn resolve_runtime_slot_id(&self, cc_tile_id: u32) -> Option<u32> {
        // Step 1: Transcode CC -> EC Material -> EC Slot.
        // This uses TerrainTranscode.kdl mappings.
        if let Some(&material_id) = self.transcode.get(&cc_tile_id) {
            let material_records = self
                .terrain_provenance
                .iter()
                .filter(|r| r.material_id == material_id)
                .collect::<Vec<_>>();

            // Resolve the material ID to a canonical slot using the best alias available.
            if let Some(slot_id) = material_records
                .iter()
                .filter(|record| {
                    record.alias_slot_id != 0
                        && record.alias_slot_id != MISSING_SLOT_ID
                        && self.resolve_provenance_record_slot(record).is_some()
                })
                .min_by_key(|record| record.alias_count_index)
                .and_then(|record| self.resolve_provenance_record_slot(record))
            {
                return Some(slot_id);
            }

            if let Some(slot_id) = material_records
                .iter()
                .filter(|record| {
                    record.alias_slot_id == 0 && self.resolve_provenance_record_slot(record).is_some()
                })
                .min_by_key(|record| record.alias_count_index)
                .and_then(|record| self.resolve_provenance_record_slot(record))
            {
                return Some(slot_id);
            }
        }

        // Step 2: Legacy Alias mapping.
        // If CC ID 168 doesn't have a transcode, but a provenance record says Alias 168 ➔ Slot X,
        // we use Slot X. This handles legacy assets that were ported with original IDs.
        for record in self
            .terrain_provenance
            .iter()
            .filter(|r| r.alias_slot_id == cc_tile_id)
        {
            if let Some(slot_id) = self.resolve_provenance_record_slot(record) {
                return Some(slot_id);
            }
        }

        // Step 3: Direct slot hit. The CC tile ID might be exactly the canonical slot ID 
        // (e.g. for legacy textures like water that share the same ID).
        if self.present_slot(cc_tile_id).is_some() {
            return Some(cc_tile_id);
        }

        None
    }

    fn resolve_provenance_record_slot(
        &self,
        record: &EcLandTerrainProvenanceRecord,
    ) -> Option<u32> {
        if record.canonical_slot_id != 0 && record.canonical_slot_id != MISSING_SLOT_ID {
            if self.present_slot(record.canonical_slot_id).is_some() {
                return Some(record.canonical_slot_id);
            }
        }

        if record.alias_slot_id != 0 && record.alias_slot_id != MISSING_SLOT_ID {
            if self.present_slot(record.alias_slot_id).is_some() {
                return Some(record.alias_slot_id);
            }
        }

        None
    }

    /// Returns the raw bytes of an atlas page as stored on disk.
    /// Check `pages()[page_index].pixel_format` to know how to interpret them.
    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        read_path_entry(&self.package, &page_entry_path(page_index, fmt))
            .wrap_err_with(|| format!("unpack atlas page {page_index}"))
    }
}

pub fn convert_ec_land_uop_to_ec_land_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &EcLandAtlasOptions,
) -> eyre::Result<EcLandBuildSummary> {
    convert_ec_land_uop_to_ec_land_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_ec_land_uop_to_ec_land_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &EcLandAtlasOptions,
) -> eyre::Result<EcLandBuildSummary> {
    validate_options(options)?;

    let terrain_definition_path = find_first_existing_file(source_dirs, &["TerrainDefinition.uop"])
        .ok_or_else(|| eyre::eyre!("missing required file: TerrainDefinition.uop"))?;
    let texture_uop_path = find_first_existing_file(source_dirs, &["Texture.uop"]);
    let legacy_texture_uop_path = find_first_existing_file(source_dirs, &["LegacyTexture.uop"]);

    if texture_uop_path.is_none() && legacy_texture_uop_path.is_none() {
        eyre::bail!("missing required files: Texture.uop and LegacyTexture.uop not found");
    }

    info!(
        "Converting EC Land from TerrainDefinition.uop / Texture.uop / LegacyTexture.uop to {}",
        out_file.display()
    );
    println!(
        "Using TerrainDefinition.uop: {}",
        terrain_definition_path.display()
    );
    if let Some(path) = texture_uop_path.as_ref() {
        println!("Using Texture.uop: {}", path.display());
    }
    if let Some(path) = legacy_texture_uop_path.as_ref() {
        println!("Using LegacyTexture.uop: {}", path.display());
    }

    let world_textures = if let Some(texture_uop_path) = texture_uop_path.as_ref() {
        Some(
            Textures::new(texture_uop_path, None, None)
                .wrap_err_with(|| format!("load {}", texture_uop_path.display()))?,
        )
    } else {
        None
    };

    let legacy_textures = if let Some(legacy_texture_uop_path) = legacy_texture_uop_path.as_ref() {
        Some(
            Textures::new(legacy_texture_uop_path, None, None)
                .wrap_err_with(|| format!("load {}", legacy_texture_uop_path.display()))?,
        )
    } else {
        None
    };

    let terrain_definition = TerrainDefinitionPackage::load(&terrain_definition_path)
        .wrap_err("load TerrainDefinition.uop")?;
    let terrain_entry_count = terrain_definition.entries.len() as u32;
    let terrain_alias_ref_count = terrain_definition
        .entries
        .iter()
        .map(|entry| entry.aliases.len() as u32)
        .sum();
    let land_slot_ids = terrain_definition.land_slot_ids();
    let source_texture_ids = terrain_definition.land_source_texture_ids();
    let unique_alias_slot_count = land_slot_ids.len() as u32;
    let selections = terrain_definition
        .texture_selections()
        .into_iter()
        .map(|(slot_id, texture_id)| TerrainTextureSelection {
            slot_id,
            texture_id,
        })
        .collect::<Vec<_>>();
    let unique_source_texture_count = source_texture_ids.len() as u32;
    let unique_texture_selection_count = selections.len() as u32;
    let (decoded_tiles, aliases, texture_slot_by_texture_id, ignored_source_texture_ids) =
        decode_present_tiles(
            world_textures.as_ref(),
            legacy_textures.as_ref(),
            &terrain_definition,
        )?;
    let slot_count = land_slot_ids
        .iter()
        .copied()
        .chain(decoded_tiles.iter().map(|tile| tile.art_id))
        .max()
        .map(|max_id| max_id + 1)
        .unwrap_or(0);
    let unique_packed_texture_count = decoded_tiles.len() as u32;

    let (pages, mut slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
    apply_slot_aliases(&mut slot_records, &aliases)?;
    let populated_slot_count = slot_records.iter().filter(|slot| slot.is_present()).count() as u32;

    // Build provenance after packing so canonical_slot_id reflects the real packed slot.
    let terrain_provenance = build_terrain_provenance_records(
        &terrain_definition,
        &selections,
        &texture_slot_by_texture_id,
    );

    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;
    let terrain_provenance_manifest = serialize_terrain_provenance_manifest(&terrain_provenance)?;

    let transcode_kdl_path = find_first_existing_file(
        source_dirs,
        &[
            "TerrainTranscode.kdl",
            "cc_ec_convtables/TerrainTranscode.kdl",
            "dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl",
        ],
    );

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(UDDP_PAGE_MANIFEST_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &page_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(UDDP_SLOT_MANIFEST_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &terrain_provenance_manifest,
    })?;

    if let Some(transcode_path) = transcode_kdl_path {
        let transcode_bytes = fs::read(&transcode_path)?;
        package.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            virtual_path: Some(UDDP_TRANSCODE_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &transcode_bytes,
        })?;
    }

    // Determine the final pixel format and encoding for atlas pages.
    let encoding = if options.use_bc7 {
        VramTextureEncoding::Bc7(preferred_bc7_encoder_backend())
    } else {
        VramTextureEncoding::Rgba8UnormSrgb
    };
    let pixel_format = if options.use_bc7 {
        PagePixelFormat::Bc7
    } else {
        PagePixelFormat::Rgba8888
    };
    let compression = if options.use_bc7 {
        UddCompressionFlag::None
    } else {
        UddCompressionFlag::ZstdNoDict
    };

    let pb = ProgressBar::new(pages.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} encoding atlas pages ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let encoded_pages = if options.use_bc7 {
        let extent = ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?;
        let encoded_pages = pages
            .par_iter()
            .map(|page| {
                let page_path = page_entry_path(page.record.page_index, pixel_format);
                let encoded =
                    encode_for_vram(&page.pixels, extent, RawImageFormat::Rgba8888, encoding)
                        .map_err(|e| {
                            eyre::eyre!("BC7 encode page {}: {e}", page.record.page_index)
                        })?
                        .into_bytes()
                        .to_vec();
                pb.inc(1);
                Ok((page_path, encoded))
            })
            .collect::<Vec<eyre::Result<(String, Vec<u8>)>>>();

        let mut resolved = Vec::with_capacity(encoded_pages.len());
        for page in encoded_pages {
            resolved.push(page?);
        }
        resolved
    } else {
        pages
            .iter()
            .map(|page| {
                pb.inc(1);
                (
                    page_entry_path(page.record.page_index, pixel_format),
                    crop_rgba_page(
                        &page.pixels,
                        options.atlas_width,
                        page.record.used_width,
                        page.record.used_height,
                    ),
                )
            })
            .collect()
    };

    for (page_path, encoded) in encoded_pages {
        package.add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression,
            virtual_path: Some(&page_path),
            path_hash64: None,
            id: None,
            data: &encoded,
        })?;
    }
    pb.finish_with_message("Atlas pages encoded");

    build_and_write_package(&mut package, out_file)?;

    Ok(EcLandBuildSummary {
        terrain_entry_count,
        terrain_alias_ref_count,
        unique_alias_slot_count,
        unique_source_texture_count,
        unique_texture_selection_count,
        unique_packed_texture_count,
        ignored_source_texture_ids,
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &EcLandAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    if options.use_bc7 && (options.atlas_width % 4 != 0 || options.atlas_height % 4 != 0) {
        eyre::bail!("BC7 compression requires atlas dimensions divisible by 4");
    }
    Ok(())
}

fn build_terrain_provenance_records(
    terrain_definition: &TerrainDefinitionPackage,
    selections: &[TerrainTextureSelection],
    texture_slot_by_texture_id: &HashMap<u32, u32>,
) -> Vec<EcLandTerrainProvenanceRecord> {
    let selected_texture_by_slot = selections
        .iter()
        .map(|selection| (selection.slot_id, selection.texture_id))
        .collect::<HashMap<_, _>>();

    let mut records = Vec::new();
    for entry in &terrain_definition.entries {
        let runtime_slot_id = entry
            .runtime_slot_ids()
            .first()
            .copied()
            .unwrap_or(entry.id);
        for alias in &entry.aliases {
            let provenance_slot_id = if alias.alias == 0 {
                runtime_slot_id
            } else {
                alias.alias
            };
            let selected_texture_id = selected_texture_by_slot
                .get(&provenance_slot_id)
                .copied()
                .unwrap_or(MISSING_TEXTURE_ID);
            let canonical_slot_id = texture_slot_by_texture_id
                .get(&selected_texture_id)
                .copied()
                .unwrap_or(MISSING_SLOT_ID);
            records.push(EcLandTerrainProvenanceRecord {
                material_id: entry.id,
                material_name_id: entry.name_id,
                alias_count_index: alias.count_index,
                alias_slot_id: alias.alias,
                alias_tile_flags: alias.tile_flags,
                selected_texture_id,
                canonical_slot_id,
            });
        }
    }

    records.sort_by_key(|record| {
        (
            record.alias_slot_id,
            record.material_id,
            record.alias_count_index,
        )
    });
    records
}

fn decode_present_tiles(
    world_textures: Option<&Textures>,
    legacy_textures: Option<&Textures>,
    terrain_definition: &TerrainDefinitionPackage,
) -> eyre::Result<(
    Vec<DecodedArtTile>,
    Vec<SlotAlias>,
    HashMap<u32, u32>,
    Vec<u32>,
)> {
    // Pack raw terrain source textures directly. Runtime terrain shading can
    // decide how to combine them later; build-time packing should not flatten
    // layered materials into one representative image.
    let mut decoded_tiles: Vec<DecodedArtTile> = Vec::new();
    let mut aliases = Vec::new();
    let mut texture_slot_by_texture_id = HashMap::new();
    let mut ignored_source_texture_ids = Vec::new();
    let texture_ids = terrain_definition
        .land_source_texture_ids()
        .into_iter()
        .collect::<Vec<_>>();
    let mut next_synthetic_slot_id = terrain_definition
        .land_slot_ids()
        .into_iter()
        .next_back()
        .map(|max_id| max_id + 1)
        .unwrap_or(0);

    let texture_pb = ProgressBar::new(texture_ids.len() as u64);
    texture_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding unique land textures ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    let decoded_textures = texture_ids
        .par_iter()
        .map(
            |&texture_id| -> eyre::Result<(u32, Option<DecodedLayerTexture>)> {
                let decoded =
                    decode_layer_texture_rgba(texture_id, world_textures, legacy_textures)?;
                texture_pb.inc(1);
                Ok((texture_id, decoded))
            },
        )
        .collect::<Vec<_>>();
    texture_pb.finish_with_message("Unique land textures decoded");

    let mut decoded_texture_cache = HashMap::with_capacity(decoded_textures.len());
    for decoded_texture in decoded_textures {
        let (texture_id, decoded) = decoded_texture?;
        decoded_texture_cache.insert(texture_id, decoded);
    }

    for texture_id in &texture_ids {
        let Some(decoded) = decoded_texture_cache.get(texture_id).cloned().flatten() else {
            ignored_source_texture_ids.push(*texture_id);
            continue;
        };

        let synthetic_slot_id = next_synthetic_slot_id;
        next_synthetic_slot_id += 1;
        texture_slot_by_texture_id.insert(*texture_id, synthetic_slot_id);

        decoded_tiles.push(DecodedArtTile {
            art_id: synthetic_slot_id,
            kind: ArtTileKind::Land,
            width: decoded.width as u16,
            height: decoded.height as u16,
            rgba: decoded.rgba,
        });
    }

    let pb = ProgressBar::new(terrain_definition.entries.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} aliasing land slots ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let texture_selection_by_slot = terrain_definition
        .texture_selections()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let mut claimed_slots = HashSet::new();

    for entry in &terrain_definition.entries {
        pb.inc(1);

        let claimed_aliases = entry
            .runtime_slot_ids()
            .into_iter()
            .filter(|slot_id| claimed_slots.insert(*slot_id))
            .collect::<Vec<_>>();

        if claimed_aliases.is_empty() {
            continue;
        }

        let canonical_texture_id = claimed_aliases
            .iter()
            .find_map(|slot_id| texture_selection_by_slot.get(slot_id).copied())
            .or_else(|| entry.primary_texture_id());

        let Some(canonical_texture_id) = canonical_texture_id else {
            continue;
        };

        let Some(&canonical_slot_id) = texture_slot_by_texture_id.get(&canonical_texture_id) else {
            continue;
        };

        for alias_art_id in claimed_aliases {
            if alias_art_id == canonical_slot_id {
                continue;
            }
            aliases.push(SlotAlias {
                art_id: alias_art_id,
                canonical_art_id: canonical_slot_id,
            });
        }
    }
    pb.finish_with_message("Land slots aliased");

    decoded_tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });

    Ok((
        decoded_tiles,
        aliases,
        texture_slot_by_texture_id,
        ignored_source_texture_ids,
    ))
}

fn decode_layer_texture_rgba(
    texture_id: u32,
    world_textures: Option<&Textures>,
    legacy_textures: Option<&Textures>,
) -> eyre::Result<Option<DecodedLayerTexture>> {
    let file = if let Some(wt) = world_textures {
        let world_terrain_path = format!("build/worldart/land/{texture_id:08}.dds");
        let world_terrain_hash = uocf::uop::hash::hash_file_name_single(&world_terrain_path);
        wt.get_from_hash(
            world_terrain_hash,
            Some(&world_terrain_path),
            ECImageFormat::DDS,
        )?
    } else {
        None
    };

    let file = if file.is_none() {
        if let Some(lt) = legacy_textures {
            let legacy_terrain_path = format!("build/legacyland/{texture_id:08}.dat");
            let legacy_terrain_hash = uocf::uop::hash::hash_file_name_single(&legacy_terrain_path);
            lt.get_from_hash(
                legacy_terrain_hash,
                Some(&legacy_terrain_path),
                ECImageFormat::DDS,
            )?
        } else {
            None
        }
    } else {
        file
    };

    let file = if file.is_none() {
        if let Some(wt) = world_textures {
            wt.get_from_id(texture_id)?
        } else {
            None
        }
    } else {
        file
    };

    let file = if file.is_none() {
        if let Some(lt) = legacy_textures {
            lt.get_from_id(texture_id)?
        } else {
            None
        }
    } else {
        file
    };

    let decoded = if let Some(file) = file {
        let rgba = file.decode_to_rgba()?.to_rgba8();
        Some(DecodedLayerTexture {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        })
    } else {
        None
    };

    Ok(decoded)
}

pub fn apply_slot_aliases(
    slots: &mut [EcLandSlotRecord],
    aliases: &[SlotAlias],
) -> eyre::Result<()> {
    for alias in aliases {
        let canonical = *slots
            .get(alias.canonical_art_id as usize)
            .context("canonical ec_land slot outside slot table")?;
        let slot = slots
            .get_mut(alias.art_id as usize)
            .context("alias ec_land slot outside slot table")?;
        if !canonical.is_present() {
            eyre::bail!(
                "canonical ec_land slot {} missing while applying alias {}",
                alias.canonical_art_id,
                alias.art_id
            );
        }
        *slot = EcLandSlotRecord {
            art_id: alias.art_id,
            ..canonical
        };
    }
    Ok(())
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &EcLandAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<EcLandSlotRecord>)> {
    // Preserve the dense land-id address space in metadata even though the packed
    // payload contains only present tiles. Runtime lookup then becomes a single array read.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(EcLandSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page, leftovers) = build_page(page_index, remaining, options)?;
        if page.placed_tiles.is_empty() {
            eyre::bail!(
                "could not fit any art tile into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }

        for placed in &page.placed_tiles {
            let slot = slot_records
                .get_mut(placed.art_id as usize)
                .context("placed tile art_id outside slot table")?;
            *slot = EcLandSlotRecord {
                art_id: placed.art_id,
                page_index,
                page_tile_index: placed.page_tile_index,
                flags: placed.kind.slot_flags(),
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
            };
        }

        pages.push(page);
        remaining = leftovers;
        page_index += 1;
    }

    Ok((pages, slot_records))
}

fn build_page(
    page_index: u32,
    tiles: Vec<DecodedArtTile>,
    options: &EcLandAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedArtTile>)> {
    // Assemble into a full-size RGBA page first, then store a cropped payload later.
    // This keeps placement logic and manifest coordinates expressed in one atlas space.
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut placed_tiles = Vec::new();
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let gutter = i32::from(options.gutter);

    for tile in tiles {
        // Some EC land textures already span the full atlas width or height.
        // In that case a symmetric gutter would make them mathematically impossible
        // to place, so drop the gutter only on the overflowing axis.
        let gutter_x = if tile.width as u32 + (options.gutter as u32 * 2) > options.atlas_width {
            0
        } else {
            gutter
        };
        let gutter_y = if tile.height as u32 + (options.gutter as u32 * 2) > options.atlas_height {
            0
        } else {
            gutter
        };
        let alloc_width = tile.width as i32 + gutter_x * 2;
        let alloc_height = tile.height as i32 + gutter_y * 2;
        if alloc_width > options.atlas_width as i32 || alloc_height > options.atlas_height as i32 {
            eyre::bail!(
                "art tile {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                tile.art_id,
                tile.width,
                tile.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        }

        if let Some(allocation) = allocator.allocate(size2(alloc_width, alloc_height)) {
            let inner_x = allocation.rectangle.min.x + gutter_x;
            let inner_y = allocation.rectangle.min.y + gutter_y;
            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                tile.width as u32,
                tile.height as u32,
                &tile.rgba,
            )?;

            // Store the occupied rectangle in texel space. The package writer crops
            // to exactly this rectangle, which is why wide but short land strips can
            // compress so well compared with saving whole 2048x2048 pages verbatim.
            used_width = used_width.max(inner_x as u32 + tile.width as u32);
            used_height = used_height.max(inner_y as u32 + tile.height as u32);
            placed_tiles.push(PlacedTile {
                art_id: tile.art_id,
                kind: tile.kind,
                page_tile_index: placed_tiles.len() as u16,
                x: inner_x as u16,
                y: inner_y as u16,
                width: tile.width,
                height: tile.height,
            });
        } else {
            leftovers.push(tile);
        }
    }

    Ok((
        BuiltPage {
            record: EcLandPageRecord {
                page_index,
                tile_count: placed_tiles.len() as u32,
                used_width,
                used_height,
                pixel_format: PagePixelFormat::Rgba8888,
            },
            pixels,
            placed_tiles,
        },
        leftovers,
    ))
}

fn blit_rgba_tile(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    tile_width: u32,
    tile_height: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let expected_len = tile_width as usize * tile_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for tile {}x{}: expected {}, got {}",
            tile_width,
            tile_height,
            expected_len,
            src.len()
        );
    }

    let dst_stride = dst_width as usize * 4;
    let src_stride = tile_width as usize * 4;
    for row in 0..tile_height as usize {
        let src_start = row * src_stride;
        let dst_start = ((dst_y as usize + row) * dst_stride) + dst_x as usize * 4;
        let dst_end = dst_start + src_stride;
        dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + src_stride]);
    }

    Ok(())
}

pub fn crop_rgba_page(src: &[u8], src_width: u32, crop_width: u32, crop_height: u32) -> Vec<u8> {
    // Copy only the occupied top-left rectangle into the stored payload. Readers use
    // the manifest's `used_*` bounds to reconstruct the compact buffer correctly and
    // the slot metadata still points into the original logical atlas coordinate space.
    let mut cropped = vec![0u8; crop_width as usize * crop_height as usize * 4];
    let src_stride = src_width as usize * 4;
    let dst_stride = crop_width as usize * 4;

    for row in 0..crop_height as usize {
        let src_start = row * src_stride;
        let dst_start = row * dst_stride;
        cropped[dst_start..dst_start + dst_stride]
            .copy_from_slice(&src[src_start..src_start + dst_stride]);
    }

    cropped
}

pub fn serialize_page_manifest(
    pages: &[BuiltPage],
    options: &EcLandAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // The manifest separates two concerns: logical atlas dimensions for consumers,
    // and compact stored bounds for I/O. That split is what lets the package shrink
    // aggressively without changing any slot coordinates.
    let pixel_format = if options.use_bc7 {
        PagePixelFormat::Bc7
    } else {
        PagePixelFormat::Rgba8888
    };
    let mut bytes = Vec::with_capacity(25 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(EC_LAND_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(pixel_format as u8);
    bytes.write_u32::<LittleEndian>(pages.len() as u32)?;
    for page in pages {
        bytes.write_u32::<LittleEndian>(page.record.page_index)?;
        bytes.write_u32::<LittleEndian>(page.record.tile_count)?;
        bytes.write_u32::<LittleEndian>(page.record.used_width)?;
        bytes.write_u32::<LittleEndian>(page.record.used_height)?;
    }
    Ok(bytes)
}

pub fn serialize_slot_manifest(
    slots: &[EcLandSlotRecord],
    options: &EcLandAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + slots.len() * 20);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(EC_LAND_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.write_u32::<LittleEndian>(slots.len() as u32)?;
    for slot in slots {
        bytes.write_u32::<LittleEndian>(slot.art_id)?;
        bytes.write_u32::<LittleEndian>(slot.page_index)?;
        bytes.write_u16::<LittleEndian>(slot.page_tile_index)?;
        bytes.write_u16::<LittleEndian>(slot.flags)?;
        bytes.write_u16::<LittleEndian>(slot.x)?;
        bytes.write_u16::<LittleEndian>(slot.y)?;
        bytes.write_u16::<LittleEndian>(slot.width)?;
        bytes.write_u16::<LittleEndian>(slot.height)?;
    }
    Ok(bytes)
}

pub fn encode_slot_manifest(
    slots: &[EcLandSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &EcLandAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            use_bc7: false,
        },
    )
}

pub fn serialize_terrain_provenance_manifest(
    records: &[EcLandTerrainProvenanceRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + records.len() * 32);
    bytes.extend_from_slice(&TERRAIN_PROVENANCE_MAGIC);
    bytes.write_u32::<LittleEndian>(EC_LAND_TERRAIN_PROVENANCE_VERSION)?;
    bytes.write_u32::<LittleEndian>(records.len() as u32)?;
    for record in records {
        bytes.write_u32::<LittleEndian>(record.material_id)?;
        bytes.write_i32::<LittleEndian>(record.material_name_id)?;
        bytes.write_u32::<LittleEndian>(record.alias_count_index)?;
        bytes.write_u32::<LittleEndian>(record.alias_slot_id)?;
        bytes.write_u64::<LittleEndian>(record.alias_tile_flags)?;
        bytes.write_u32::<LittleEndian>(record.selected_texture_id)?;
        bytes.write_u32::<LittleEndian>(record.canonical_slot_id)?;
    }
    Ok(bytes)
}

pub fn encode_terrain_provenance_manifest(
    records: &[EcLandTerrainProvenanceRecord],
) -> eyre::Result<Vec<u8>> {
    serialize_terrain_provenance_manifest(records)
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<EcLandPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid ec_land page manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != EC_LAND_METADATA_VERSION {
        eyre::bail!("unsupported ec_land page manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("unknown ec_land page pixel format"))?;
    let page_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        pages.push(EcLandPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            tile_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((atlas_width, atlas_height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<EcLandSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid ec_land slot manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != EC_LAND_METADATA_VERSION {
        eyre::bail!("unsupported ec_land slot manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let slot_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        slots.push(EcLandSlotRecord {
            art_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_tile_index: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok((atlas_width, atlas_height, gutter, slots))
}

fn parse_terrain_provenance_manifest(
    bytes: &[u8],
) -> eyre::Result<Vec<EcLandTerrainProvenanceRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != TERRAIN_PROVENANCE_MAGIC {
        eyre::bail!("invalid ec_land terrain provenance manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != EC_LAND_TERRAIN_PROVENANCE_VERSION {
        eyre::bail!("unsupported ec_land terrain provenance manifest version {version}");
    }
    let record_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut records = Vec::with_capacity(record_count);
    for _ in 0..record_count {
        records.push(EcLandTerrainProvenanceRecord {
            material_id: cursor.read_u32::<LittleEndian>()?,
            material_name_id: cursor.read_i32::<LittleEndian>()?,
            alias_count_index: cursor.read_u32::<LittleEndian>()?,
            alias_slot_id: cursor.read_u32::<LittleEndian>()?,
            alias_tile_flags: cursor.read_u64::<LittleEndian>()?,
            selected_texture_id: cursor.read_u32::<LittleEndian>()?,
            canonical_slot_id: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok(records)
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}

pub const UDDP_TRANSCODE_ENTRY_VPATH: &str = "metadata/transcode.kdl";

fn read_transcode_from_package(package: &UddpReader) -> Option<HashMap<u32, u32>> {
    let bytes = read_path_entry(package, UDDP_TRANSCODE_ENTRY_VPATH).ok()?;
    let content = String::from_utf8(bytes).ok()?;
    let transcode: crate::cc_ec_conv::TerrainTranscode =
        knuffel::parse("transcode.kdl", &content).ok()?;
    Some(transcode.to_map())
}

fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("read {path}"))
}
