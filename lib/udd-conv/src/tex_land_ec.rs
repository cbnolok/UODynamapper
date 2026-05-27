//! Build-time and runtime support for `tex_land_ec.uddp`.
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use serde::Serialize;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};

use crate::bc7::{
    bc7_encode_progress_units, encode_for_vram_with_bc7_rdo_lambda_and_progress,
    preferred_bc7_encoder_backend, ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::{AtlasPackingMode, extrude_rgba_rect_edges, resolve_packing_axis};
use crate::package_progress::{
    atlas_payload_finish_message, atlas_payload_progress_message, build_and_write_package,
};
use crate::source_paths::{find_first_existing_file, source_path_label_from_dirs};
use udd_assets::tex_art_cc::{page_entry_path, PagePixelFormat};
use udd_assets::tex_land_ec::{
    TexLandEcPageRecord, TexLandEcSlotRecord, TexLandEcTerrainProvenanceRecord,
    MISSING_PAGE_INDEX, MISSING_PAGE_TILE_INDEX, MISSING_SLOT_ID, MISSING_TERRAIN_LAYER_INDEX,
    MISSING_TEXTURE_ID, TERRAIN_PRIMARY_FLAG_FALLBACK_REASON,
    TERRAIN_PRIMARY_FLAG_MULTIPLE_PREFERRED_NON_SUPPORT,
    TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER,
    TERRAIN_PRIMARY_FLAG_SELECTED_CURRENT_SUPPORT, TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION,
    TERRAIN_PRIMARY_FLAG_SELECTED_SUPPORT_LIKE,
    TERRAIN_PRIMARY_FLAG_SUPPORT_LIKE_OUTSIDE_CURRENT_HEURISTIC,
    TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION,
    TERRAIN_PRIMARY_REASON_NON_SUPPORT_REPETITION_FALLBACK,
    TERRAIN_PRIMARY_REASON_SUPPORT_PREFERRED_REPETITION_FALLBACK,
    TERRAIN_PRIMARY_REASON_SUPPORT_REPETITION_FALLBACK, TERRAIN_PRIMARY_REASON_UNKNOWN,
    UDDP_PAGE_MANIFEST_ENTRY_VPATH, UDDP_SLOT_MANIFEST_ENTRY_VPATH,
    UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH, UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH,
    UDDP_TRANSCODE_ENTRY_VPATH,
};
use udd_assets::ec_terrain_overrides::{
    EcTerrainOverrideEntry, EcTerrainOverrides, TerrainIgnoreOverride, TerrainLayerOverride,
    TerrainLiquidOverride, TerrainPolicyOverride, TerrainTextureOverride,
};
use udd_container::xxh64_virtual_path;
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::enhanced::{
    terrain_definition::{
        TerrainDefinitionEntry, TerrainDefinitionPackage, TerrainDefinitionPrimaryLayerReason,
        TerrainDefinitionTextureLayer,
    },
    textures::{ECImageFormat, Textures},
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
const TEX_LAND_EC_METADATA_VERSION: u32 = 3;
const TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION: u32 = 3;
const TEX_LAND_EC_TERRAIN_OVERRIDES_SCHEMA_VERSION: u32 = 1;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Serialize)]
struct TerrainOverridesMetadata {
    schema: &'static str,
    schema_version: u32,
    source_path: String,
    override_count: u32,
    active_action_count: u32,
    entries: Vec<TerrainOverrideMetadataEntry>,
}

#[derive(Debug, Serialize)]
struct TerrainOverrideMetadataEntry {
    material_id: u32,
    active_action_count: u32,
    policies: Vec<TerrainPolicyMetadata>,
    liquid: Option<TerrainLiquidMetadata>,
    layers: Vec<TerrainLayerMetadata>,
    textures: Vec<TerrainTextureMetadata>,
    ignore: Option<TerrainIgnoreMetadata>,
}

#[derive(Debug, Serialize)]
struct TerrainPolicyMetadata {
    policy: String,
    code: Option<String>,
}

#[derive(Debug, Serialize)]
struct TerrainLiquidMetadata {
    speed: Option<f32>,
    waveheight: Option<f32>,
    code: Option<String>,
}

#[derive(Debug, Serialize)]
struct TerrainLayerMetadata {
    role: String,
    texture_id: u32,
    stretch: Option<f32>,
    code: Option<String>,
}

#[derive(Debug, Serialize)]
struct TerrainTextureMetadata {
    texture_id: u32,
    role: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Serialize)]
struct TerrainIgnoreMetadata {
    code: Option<String>,
}

use crate::upscale::{apply_filter_passes, UpscaleConfig, UpscaleFilter};

pub struct TexLandEcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub upscale_64: UpscaleConfig,
    pub upscale_128: UpscaleConfig,
    pub upscale_256: UpscaleConfig,
    pub upscale_512: UpscaleConfig,
    pub upscale_64_passes: Vec<UpscaleFilter>,
    pub upscale_128_passes: Vec<UpscaleFilter>,
    pub upscale_256_passes: Vec<UpscaleFilter>,
    pub upscale_512_passes: Vec<UpscaleFilter>,
    pub pixel_format: PagePixelFormat,
    pub packing_mode: AtlasPackingMode,
    pub filtering_ready: bool,
    pub bc7_rdo_lambda: f32,
    pub transcode_kdl_path: Option<PathBuf>,
}

impl Default for TexLandEcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::JpegXl,
            upscale_64: UpscaleConfig::default(),
            upscale_128: UpscaleConfig::default(),
            upscale_256: UpscaleConfig::default(),
            upscale_512: UpscaleConfig::default(),
            upscale_64_passes: Vec::new(),
            upscale_128_passes: Vec::new(),
            upscale_256_passes: Vec::new(),
            upscale_512_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::MaximumPacking,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
            transcode_kdl_path: None,
        }
    }
}

fn packing_mode_repr(mode: AtlasPackingMode) -> u8 {
    match mode {
        AtlasPackingMode::MaximumPacking => 0,
        AtlasPackingMode::Bc7Oriented => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexLandEcBuildSummary {
    pub terrain_entry_count: u32,
    pub terrain_alias_ref_count: u32,
    pub unique_alias_slot_count: u32,
    pub unique_source_texture_count: u32,
    pub unique_texture_selection_count: u32,
    pub unique_packed_texture_count: u32,
    pub terrain_override_entry_count: u32,
    pub terrain_override_source: Option<PathBuf>,
    pub ignored_source_texture_ids: Vec<u32>,
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
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
    pub record: TexLandEcPageRecord,
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;

pub fn convert_tex_land_ec_uop_to_tex_land_ec_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<TexLandEcBuildSummary> {
    convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<TexLandEcBuildSummary> {
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
        source_path_label_from_dirs(source_dirs, &terrain_definition_path)
    );
    if let Some(path) = texture_uop_path.as_ref() {
        println!("Using Texture.uop: {}", source_path_label_from_dirs(source_dirs, path));
    }
    if let Some(path) = legacy_texture_uop_path.as_ref() {
        println!("Using LegacyTexture.uop: {}", source_path_label_from_dirs(source_dirs, path));
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

    convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_loaded_sources(
        source_dirs,
        &terrain_definition_path,
        texture_uop_path.as_deref(),
        legacy_texture_uop_path.as_deref(),
        &terrain_definition,
        world_textures.as_ref(),
        legacy_textures.as_ref(),
        out_file,
        options,
    )
}

pub fn convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_loaded_sources(
    source_dirs: &[PathBuf],
    terrain_definition_path: &Path,
    texture_uop_path: Option<&Path>,
    legacy_texture_uop_path: Option<&Path>,
    terrain_definition: &TerrainDefinitionPackage,
    world_textures: Option<&Textures>,
    legacy_textures: Option<&Textures>,
    out_file: &Path,
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<TexLandEcBuildSummary> {
    validate_options(options)?;

    let terrain_entry_count = terrain_definition.entries.len() as u32;
    let terrain_alias_ref_count = terrain_definition
        .entries
        .iter()
        .map(|entry| entry.aliases.len() as u32)
        .sum();
    let terrain_overrides_kdl_path = find_first_existing_file_in_sources_or_cwd(
        source_dirs,
        &[
            "EcTerrainOverrides.kdl",
            "cc_ec_convtables/EcTerrainOverrides.kdl",
            "dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl",
        ],
    );
    let terrain_overrides = terrain_overrides_kdl_path
        .as_ref()
        .map(|path| EcTerrainOverrides::load(path).map(|overrides| (path.clone(), overrides)))
        .transpose()?
        .map(|(path, overrides)| LoadedTerrainOverrides { path, overrides });
    let terrain_override_layers_by_material = terrain_overrides
        .as_ref()
        .map(|loaded| terrain_override_layers_by_material(&loaded.overrides))
        .unwrap_or_default();
    let terrain_override_texture_ids = terrain_override_layers_by_material
        .values()
        .flat_map(|layers| layers.keys().copied())
        .collect::<BTreeSet<_>>();
    let land_slot_ids = terrain_definition.land_slot_ids();
    let source_texture_ids = terrain_definition
        .land_source_texture_ids()
        .into_iter()
        .chain(terrain_override_texture_ids.iter().copied())
        .collect::<BTreeSet<_>>();
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
            world_textures,
            legacy_textures,
            terrain_definition,
            &terrain_override_texture_ids,
            options,
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
        &terrain_override_layers_by_material,
    );

    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;
    let terrain_provenance_manifest = serialize_terrain_provenance_manifest(&terrain_provenance)?;
    let terrain_overrides_manifest = terrain_overrides
        .as_ref()
        .map(|loaded| {
            serialize_terrain_overrides_metadata_from_overrides(&loaded.path, &loaded.overrides)
        })
        .transpose()?;
    let terrain_override_entry_count = terrain_overrides_manifest
        .as_ref()
        .map(|manifest| manifest.entry_count)
        .unwrap_or(0);

    let transcode_kdl_path = if let Some(path) = options.transcode_kdl_path.as_ref() {
        if !path.is_file() {
            eyre::bail!("terrain transcode KDL not found: {}", path.display());
        }
        Some(path.clone())
    } else {
        find_first_existing_file(
            source_dirs,
            &[
                "TerrainTranscode.kdl",
                "cc_ec_convtables/TerrainTranscode.kdl",
                "dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl",
            ],
        )
    };

    info!(
        "Converting EC Land from TerrainDefinition.uop / Texture.uop / LegacyTexture.uop to {}",
        out_file.display()
    );
    println!(
        "Using TerrainDefinition.uop: {}",
        source_path_label_from_dirs(source_dirs, terrain_definition_path)
    );
    if let Some(path) = texture_uop_path {
        println!("Using Texture.uop: {}", source_path_label_from_dirs(source_dirs, path));
    }
    if let Some(path) = legacy_texture_uop_path {
        println!("Using LegacyTexture.uop: {}", source_path_label_from_dirs(source_dirs, path));
    }

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(UDDP_PAGE_MANIFEST_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &page_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(UDDP_SLOT_MANIFEST_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH),
        path_hash64: None,
        id: None,
        data: &terrain_provenance_manifest,
    })?;
    if let Some(manifest) = terrain_overrides_manifest.as_ref() {
        package.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(UDDP_TERRAIN_OVERRIDES_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &manifest.bytes,
        })?;
    }

    if let Some(transcode_path) = transcode_kdl_path {
        let transcode_bytes = fs::read(&transcode_path)?;
        package.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(UDDP_TRANSCODE_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &transcode_bytes,
        })?;
    }

    // Determine the final pixel format and encoding for atlas pages.
    let use_bc7 = options.pixel_format == PagePixelFormat::Bc7;

    let (encoding, pixel_format) = if use_bc7 {
        (
            VramTextureEncoding::Bc7(preferred_bc7_encoder_backend()),
            PagePixelFormat::Bc7,
        )
    } else {
        (
            VramTextureEncoding::Rgba8UnormSrgb,
            PagePixelFormat::Rgba8888,
        )
    };

    let compression = options.compression;

    let bc7_extent = if use_bc7 {
        Some(ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?)
    } else {
        None
    };
    let progress_len = if let Some(extent) = bc7_extent {
        pages.len() as u64 * bc7_encode_progress_units(extent, options.bc7_rdo_lambda) as u64
    } else {
        pages.len() as u64
    };
    let progress_message = atlas_payload_progress_message(
        "EC land atlas pages",
        use_bc7,
        compression,
        options.bc7_rdo_lambda,
    );

    let pb = ProgressBar::new(progress_len);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(progress_message);

    let encoded_pages = if use_bc7 {
        let extent = bc7_extent.expect("BC7 extent is initialized when BC7 output is selected");
        let encoded_pages = pages
            .par_iter()
            .map(|page| {
                let page_path = page_entry_path(page.record.page_index, pixel_format);
                let encoded =
                    encode_for_vram_with_bc7_rdo_lambda_and_progress(
                        &page.pixels,
                        extent,
                        RawImageFormat::Rgba8888,
                        encoding,
                        options.bc7_rdo_lambda,
                        |units| pb.inc(units as u64),
                    )
                        .map_err(|e| {
                            eyre::eyre!("BC7 encode page {}: {e}", page.record.page_index)
                        })?
                        .into_bytes()
                        .to_vec();
                Ok((
                    page_path,
                    encoded,
                    options.atlas_width,
                    options.atlas_height,
                ))
            })
            .collect::<Vec<eyre::Result<(String, Vec<u8>, u32, u32)>>>();

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
                    page.record.used_width,
                    page.record.used_height,
                )
            })
            .collect()
    };

    for (page_path, encoded, width, height) in encoded_pages {
        package.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression,
            width,
            height,
            virtual_path: Some(page_path),
            path_hash64: None,
            id: None,
            data: encoded,
        })?;
    }
    pb.finish_with_message(atlas_payload_finish_message(
        "EC land atlas pages",
        use_bc7,
        compression,
        options.bc7_rdo_lambda,
    ));

    build_and_write_package(&mut package, out_file)?;

    Ok(TexLandEcBuildSummary {
        terrain_entry_count,
        terrain_alias_ref_count,
        unique_alias_slot_count,
        unique_source_texture_count,
        unique_texture_selection_count,
        unique_packed_texture_count,
        terrain_override_entry_count,
        terrain_override_source: terrain_overrides.map(|loaded| loaded.path),
        ignored_source_texture_ids,
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &TexLandEcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    if options.pixel_format == PagePixelFormat::Bc7
        && (options.atlas_width % 4 != 0 || options.atlas_height % 4 != 0)
    {
        eyre::bail!("BC7 compression requires atlas dimensions divisible by 4");
    }
    Ok(())
}

struct SerializedTerrainOverridesMetadata {
    bytes: Vec<u8>,
    entry_count: u32,
}

struct LoadedTerrainOverrides {
    path: PathBuf,
    overrides: EcTerrainOverrides,
}

fn find_first_existing_file_in_sources_or_cwd(
    source_dirs: &[PathBuf],
    file_names: &[&str],
) -> Option<PathBuf> {
    find_first_existing_file(source_dirs, file_names).or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| find_first_existing_file(&[cwd], file_names))
    })
}

fn serialize_terrain_overrides_metadata(
    path: &Path,
) -> eyre::Result<SerializedTerrainOverridesMetadata> {
    let overrides = EcTerrainOverrides::load(path)?;
    serialize_terrain_overrides_metadata_from_overrides(path, &overrides)
}

fn serialize_terrain_overrides_metadata_from_overrides(
    path: &Path,
    overrides: &EcTerrainOverrides,
) -> eyre::Result<SerializedTerrainOverridesMetadata> {
    let mut entries = overrides
        .to_map()
        .into_values()
        .filter(|entry| entry.active_action_count() > 0)
        .map(terrain_override_metadata_entry)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.material_id);

    let active_action_count = entries
        .iter()
        .map(|entry| entry.active_action_count)
        .sum::<u32>();
    let metadata = TerrainOverridesMetadata {
        schema: "tex_land_ec_terrain_overrides",
        schema_version: TEX_LAND_EC_TERRAIN_OVERRIDES_SCHEMA_VERSION,
        source_path: path.display().to_string(),
        override_count: entries.len() as u32,
        active_action_count,
        entries,
    };
    let entry_count = metadata.override_count;
    let mut bytes = serde_json::to_vec_pretty(&metadata)
        .wrap_err_with(|| format!("serialize terrain overrides metadata from {}", path.display()))?;
    bytes.push(b'\n');

    Ok(SerializedTerrainOverridesMetadata { bytes, entry_count })
}

#[derive(Debug, Clone, Copy)]
struct TerrainLayerProvenance {
    layer_index: u32,
    texture_repetition: f32,
}

fn terrain_override_layers_by_material(
    overrides: &EcTerrainOverrides,
) -> HashMap<u32, BTreeMap<u32, TerrainLayerProvenance>> {
    let mut layers_by_material = HashMap::<u32, BTreeMap<u32, TerrainLayerProvenance>>::new();
    for (material_id, entry) in overrides.to_map() {
        let layers = layers_by_material.entry(material_id).or_default();
        for (index, layer) in entry.layers.iter().enumerate() {
            layers.insert(
                layer.texture,
                TerrainLayerProvenance {
                    layer_index: index as u32,
                    texture_repetition: layer.stretch.unwrap_or(1.0),
                },
            );
        }
        for texture in &entry.textures {
            layers.entry(texture.texture).or_insert(TerrainLayerProvenance {
                layer_index: MISSING_TERRAIN_LAYER_INDEX,
                texture_repetition: 1.0,
            });
        }
    }
    layers_by_material
}

fn terrain_override_metadata_entry(entry: EcTerrainOverrideEntry) -> TerrainOverrideMetadataEntry {
    TerrainOverrideMetadataEntry {
        material_id: entry.id,
        active_action_count: entry.active_action_count() as u32,
        policies: entry
            .policies
            .into_iter()
            .map(terrain_policy_metadata)
            .collect(),
        liquid: entry.liquid.map(terrain_liquid_metadata),
        layers: entry
            .layers
            .into_iter()
            .map(terrain_layer_metadata)
            .collect(),
        textures: entry
            .textures
            .into_iter()
            .map(terrain_texture_metadata)
            .collect(),
        ignore: entry.ignore.map(terrain_ignore_metadata),
    }
}

fn terrain_policy_metadata(policy: TerrainPolicyOverride) -> TerrainPolicyMetadata {
    TerrainPolicyMetadata {
        policy: policy.policy,
        code: policy.code,
    }
}

fn terrain_liquid_metadata(liquid: TerrainLiquidOverride) -> TerrainLiquidMetadata {
    TerrainLiquidMetadata {
        speed: liquid.speed,
        waveheight: liquid.waveheight,
        code: liquid.code,
    }
}

fn terrain_layer_metadata(layer: TerrainLayerOverride) -> TerrainLayerMetadata {
    TerrainLayerMetadata {
        role: layer.role,
        texture_id: layer.texture,
        stretch: layer.stretch,
        code: layer.code,
    }
}

fn terrain_texture_metadata(texture: TerrainTextureOverride) -> TerrainTextureMetadata {
    TerrainTextureMetadata {
        texture_id: texture.texture,
        role: texture.role,
        code: texture.code,
    }
}

fn terrain_ignore_metadata(ignore: TerrainIgnoreOverride) -> TerrainIgnoreMetadata {
    TerrainIgnoreMetadata { code: ignore.code }
}

fn build_terrain_provenance_records(
    terrain_definition: &TerrainDefinitionPackage,
    selections: &[TerrainTextureSelection],
    texture_slot_by_texture_id: &HashMap<u32, u32>,
    override_layers_by_material: &HashMap<u32, BTreeMap<u32, TerrainLayerProvenance>>,
) -> Vec<TexLandEcTerrainProvenanceRecord> {
    let selected_texture_by_slot = selections
        .iter()
        .map(|selection| (selection.slot_id, selection.texture_id))
        .collect::<HashMap<_, _>>();

    let mut records = Vec::new();
    for entry in &terrain_definition.entries {
        let primary = terrain_primary_provenance(entry);
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
            let mut selected_layers = entry
                .texture
                .as_ref()
                .map(|texture| {
                    texture
                        .layers
                        .iter()
                        .enumerate()
                        .filter_map(|(layer_index, layer)| {
                            layer.texture_id.map(|texture_id| {
                                (
                                    texture_id,
                                    TerrainLayerProvenance {
                                        layer_index: layer_index as u32,
                                        texture_repetition: layer.texture_repetition,
                                    },
                                )
                            })
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            if let Some(selected_texture_id) = selected_texture_by_slot.get(&provenance_slot_id).copied() {
                selected_layers.entry(selected_texture_id).or_insert(TerrainLayerProvenance {
                    layer_index: MISSING_TERRAIN_LAYER_INDEX,
                    texture_repetition: 1.0,
                });
            }
            if let Some(override_layers) = override_layers_by_material.get(&entry.id) {
                selected_layers.extend(override_layers.iter().map(|(texture_id, layer)| (*texture_id, *layer)));
            }

            if selected_layers.is_empty() {
                selected_layers.insert(
                    MISSING_TEXTURE_ID,
                    TerrainLayerProvenance {
                        layer_index: MISSING_TERRAIN_LAYER_INDEX,
                        texture_repetition: 0.0,
                    },
                );
            }

            for (selected_texture_id, selected_layer) in selected_layers {
                let canonical_slot_id = texture_slot_by_texture_id
                    .get(&selected_texture_id)
                    .copied()
                    .unwrap_or(MISSING_SLOT_ID);
                records.push(TexLandEcTerrainProvenanceRecord {
                    material_id: entry.id,
                    material_name_id: entry.name_id,
                    alias_count_index: alias.count_index,
                    alias_slot_id: alias.alias,
                    alias_tile_flags: alias.tile_flags,
                    selected_texture_id,
                    canonical_slot_id,
                    selected_layer_index: selected_layer.layer_index,
                    selected_texture_repetition: selected_layer.texture_repetition,
                    primary_texture_id: primary.texture_id,
                    primary_layer_index: primary.layer_index,
                    primary_selection_reason: primary.reason,
                    primary_selection_flags: primary.flags,
                });
            }
        }
    }

    records.sort_by_key(|record| {
        (
            record.alias_slot_id,
            record.material_id,
            record.alias_count_index,
            record.selected_texture_id,
        )
    });
    records
}

#[derive(Debug, Clone, Copy)]
struct TerrainPrimaryProvenance {
    texture_id: u32,
    layer_index: u32,
    reason: u8,
    flags: u16,
}

fn terrain_primary_provenance(entry: &TerrainDefinitionEntry) -> TerrainPrimaryProvenance {
    let Some(texture) = entry.texture.as_ref() else {
        return TerrainPrimaryProvenance {
            texture_id: MISSING_TEXTURE_ID,
            layer_index: MISSING_TERRAIN_LAYER_INDEX,
            reason: TERRAIN_PRIMARY_REASON_UNKNOWN,
            flags: 0,
        };
    };
    let Some((layer, reason)) = entry.primary_texture_layer_with_reason() else {
        return TerrainPrimaryProvenance {
            texture_id: MISSING_TEXTURE_ID,
            layer_index: MISSING_TERRAIN_LAYER_INDEX,
            reason: TERRAIN_PRIMARY_REASON_UNKNOWN,
            flags: 0,
        };
    };

    let layer_index = texture
        .layers
        .iter()
        .position(|candidate| std::ptr::eq(candidate, layer))
        .map(|index| index as u32)
        .unwrap_or(MISSING_TERRAIN_LAYER_INDEX);

    TerrainPrimaryProvenance {
        texture_id: layer.texture_id.unwrap_or(MISSING_TEXTURE_ID),
        layer_index,
        reason: terrain_primary_reason_code(reason),
        flags: terrain_primary_flags(texture.layers.as_slice(), layer, reason),
    }
}

fn terrain_primary_reason_code(reason: TerrainDefinitionPrimaryLayerReason) -> u8 {
    match reason {
        TerrainDefinitionPrimaryLayerReason::NonSupportPreferredRepetition => {
            TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION
        }
        TerrainDefinitionPrimaryLayerReason::NonSupportRepetitionFallback => {
            TERRAIN_PRIMARY_REASON_NON_SUPPORT_REPETITION_FALLBACK
        }
        TerrainDefinitionPrimaryLayerReason::SupportPreferredRepetitionFallback => {
            TERRAIN_PRIMARY_REASON_SUPPORT_PREFERRED_REPETITION_FALLBACK
        }
        TerrainDefinitionPrimaryLayerReason::SupportRepetitionFallback => {
            TERRAIN_PRIMARY_REASON_SUPPORT_REPETITION_FALLBACK
        }
    }
}

fn terrain_primary_flags(
    layers: &[TerrainDefinitionTextureLayer],
    selected_layer: &TerrainDefinitionTextureLayer,
    reason: TerrainDefinitionPrimaryLayerReason,
) -> u16 {
    let mut flags = 0u16;
    if selected_layer.is_support_layer_by_current_name_heuristic() {
        flags |= TERRAIN_PRIMARY_FLAG_SELECTED_CURRENT_SUPPORT;
    }
    if selected_layer.has_support_like_name_clue() {
        flags |= TERRAIN_PRIMARY_FLAG_SELECTED_SUPPORT_LIKE;
    }
    if selected_layer.has_preferred_primary_repetition() {
        flags |= TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION;
    }
    if matches!(
        reason,
        TerrainDefinitionPrimaryLayerReason::NonSupportRepetitionFallback
            | TerrainDefinitionPrimaryLayerReason::SupportPreferredRepetitionFallback
            | TerrainDefinitionPrimaryLayerReason::SupportRepetitionFallback
    ) {
        flags |= TERRAIN_PRIMARY_FLAG_FALLBACK_REASON;
    }

    let preferred_non_support_count = layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && !layer.is_support_layer_by_current_name_heuristic()
                && layer.has_preferred_primary_repetition()
        })
        .count();
    if preferred_non_support_count > 1 {
        flags |= TERRAIN_PRIMARY_FLAG_MULTIPLE_PREFERRED_NON_SUPPORT;
    }

    let support_like_not_current_count = layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && layer.has_support_like_name_clue()
                && !layer.is_support_layer_by_current_name_heuristic()
        })
        .count();
    if support_like_not_current_count > 0 {
        flags |= TERRAIN_PRIMARY_FLAG_SUPPORT_LIKE_OUTSIDE_CURRENT_HEURISTIC;
    }

    let selected_rank_peer_count = layers
        .iter()
        .filter(|layer| {
            layer.texture_id.is_some()
                && layer.is_support_layer_by_current_name_heuristic()
                    == selected_layer.is_support_layer_by_current_name_heuristic()
                && layer.has_preferred_primary_repetition()
                    == selected_layer.has_preferred_primary_repetition()
        })
        .count();
    if selected_rank_peer_count > 1 {
        flags |= TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER;
    }

    flags
}

fn decode_present_tiles(
    world_textures: Option<&Textures>,
    legacy_textures: Option<&Textures>,
    terrain_definition: &TerrainDefinitionPackage,
    override_texture_ids: &BTreeSet<u32>,
    options: &TexLandEcAtlasOptions,
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
        .chain(override_texture_ids.iter().copied())
        .collect::<BTreeSet<_>>()
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
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} extracting unique EC land textures ({eta})")
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
    texture_pb.finish_with_message("Unique EC land textures extracted");

    let mut decoded_texture_cache = HashMap::with_capacity(decoded_textures.len());
    for decoded_texture in decoded_textures {
        let (texture_id, decoded) = decoded_texture?;
        if let Some(mut decoded) = decoded {
            let upscale_config = match (decoded.width, decoded.height) {
                (64, 64) => Some((&options.upscale_64, options.upscale_64_passes.as_slice())),
                (128, 128) => Some((&options.upscale_128, options.upscale_128_passes.as_slice())),
                (256, 256) => Some((&options.upscale_256, options.upscale_256_passes.as_slice())),
                (512, 512) => Some((&options.upscale_512, options.upscale_512_passes.as_slice())),
                _ => None,
            };

            if let Some((cfg, passes)) = upscale_config {
                if !passes.is_empty() {
                    let (width, height, rgba, _, _) =
                        apply_filter_passes(decoded.width, decoded.height, &decoded.rgba, passes);
                    decoded.width = width;
                    decoded.height = height;
                    decoded.rgba = rgba;
                } else if cfg.target_size > 0 && !matches!(cfg.filter, UpscaleFilter::None) {
                    let rgba = cfg.filter.apply_to_size(
                        decoded.width,
                        decoded.height,
                        &decoded.rgba,
                        cfg.target_size,
                        cfg.target_size,
                    );
                    decoded.width = cfg.target_size;
                    decoded.height = cfg.target_size;
                    decoded.rgba = rgba;
                }
            }
            decoded_texture_cache.insert(texture_id, decoded);
        }
    }

    for texture_id in &texture_ids {
        let Some(decoded) = decoded_texture_cache.get(texture_id).cloned() else {
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
        let world_terrain_hash =
            uocf::uop_container::hash::hash_file_name_single(&world_terrain_path);
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
            let legacy_terrain_hash =
                uocf::uop_container::hash::hash_file_name_single(&legacy_terrain_path);
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
    slots: &mut [TexLandEcSlotRecord],
    aliases: &[SlotAlias],
) -> eyre::Result<()> {
    for alias in aliases {
        let canonical = *slots
            .get(alias.canonical_art_id as usize)
            .context("canonical tex_land_ec slot outside slot table")?;
        let slot = slots
            .get_mut(alias.art_id as usize)
            .context("alias tex_land_ec slot outside slot table")?;
        if !canonical.is_present() {
            eyre::bail!(
                "canonical tex_land_ec slot {} missing while applying alias {}",
                alias.canonical_art_id,
                alias.art_id
            );
        }
        *slot = TexLandEcSlotRecord {
            art_id: alias.art_id,
            ..canonical
        };
    }
    Ok(())
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<TexLandEcSlotRecord>)> {
    // Preserve the dense land-id address space in metadata even though the packed
    // payload contains only present tiles. Runtime lookup then becomes a single array read.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(TexLandEcSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    let total_tiles = remaining.len() as u64;
    let mut page_index = 0u32;
    let pb = ProgressBar::new(total_tiles);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating EC land atlas pages");

    while !remaining.is_empty() {
        pb.set_message(format!("creating EC land atlas page {page_index}"));
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
            *slot = TexLandEcSlotRecord {
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

        pb.inc(page.placed_tiles.len() as u64);
        pages.push(page);
        remaining = leftovers;
        page_index += 1;
    }

    pb.finish_with_message(format!("EC land atlas pages created ({page_index} pages)"));

    Ok((pages, slot_records))
}

fn build_page(
    page_index: u32,
    mut tiles: Vec<DecodedArtTile>,
    options: &TexLandEcAtlasOptions,
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

    sort_tiles_within_page(&mut tiles, options);

    for tile in tiles {
        // Some EC land textures already span the full atlas width or height.
        // In that case a symmetric gutter would make them mathematically impossible
        // to place, so drop the gutter only on the overflowing axis.
        let width_axis = resolve_packing_axis(
            tile.width as u32,
            options.atlas_width,
            options.gutter,
            options.packing_mode,
            true,
        );
        let height_axis = resolve_packing_axis(
            tile.height as u32,
            options.atlas_height,
            options.gutter,
            options.packing_mode,
            true,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!(
                "art tile {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                tile.art_id,
                tile.width,
                tile.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        };

        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                tile.width as u32,
                tile.height as u32,
                &tile.rgba,
            )?;
            if options.filtering_ready {
                extrude_rgba_rect_edges(
                    &mut pixels,
                    options.atlas_width,
                    options.atlas_height,
                    allocation.rectangle.min.x as u32,
                    allocation.rectangle.min.y as u32,
                    width_axis.alloc_extent,
                    height_axis.alloc_extent,
                    inner_x as u32,
                    inner_y as u32,
                    tile.width as u32,
                    tile.height as u32,
                );
            }

            // Store the occupied rectangle in texel space. The package writer crops
            // to exactly this rectangle, which is why wide but short land strips can
            // compress so well compared with saving whole 2048x2048 pages verbatim.
            if options.filtering_ready {
                used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
                used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            } else {
                used_width = used_width.max(inner_x as u32 + width_axis.used_extent);
                used_height = used_height.max(inner_y as u32 + height_axis.used_extent);
            }
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
            record: TexLandEcPageRecord {
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

fn sort_tiles_within_page(tiles: &mut [DecodedArtTile], options: &TexLandEcAtlasOptions) {
    if options.packing_mode != AtlasPackingMode::Bc7Oriented {
        return;
    }

    tiles.sort_by(|left, right| {
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });
}

fn sort_area(tile: &DecodedArtTile, options: &TexLandEcAtlasOptions) -> u32 {
    let width_axis = resolve_packing_axis(
        tile.width as u32,
        options.atlas_width,
        options.gutter,
        options.packing_mode,
        true,
    )
    .unwrap_or_else(|| unreachable!("validated before placement"));
    let height_axis = resolve_packing_axis(
        tile.height as u32,
        options.atlas_height,
        options.gutter,
        options.packing_mode,
        true,
    )
    .unwrap_or_else(|| unreachable!("validated before placement"));
    width_axis.alloc_extent * height_axis.alloc_extent
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
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // The manifest separates two concerns: logical atlas dimensions for consumers,
    // and compact stored bounds for I/O. That split is what lets the package shrink
    // aggressively without changing any slot coordinates.
    let pixel_format = options.pixel_format;
    let mut bytes = Vec::with_capacity(26 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_LAND_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(pixel_format as u8);
    bytes.push(packing_mode_repr(options.packing_mode));
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
    slots: &[TexLandEcSlotRecord],
    options: &TexLandEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(25 + slots.len() * 20);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_LAND_EC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(packing_mode_repr(options.packing_mode));
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
    slots: &[TexLandEcSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &TexLandEcAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            compression: CompressionFlag::None,
            upscale_64: UpscaleConfig::default(),
            upscale_128: UpscaleConfig::default(),
            upscale_256: UpscaleConfig::default(),
            upscale_512: UpscaleConfig::default(),
            upscale_64_passes: Vec::new(),
            upscale_128_passes: Vec::new(),
            upscale_256_passes: Vec::new(),
            upscale_512_passes: Vec::new(),
            pixel_format: PagePixelFormat::Bc7,
            packing_mode: AtlasPackingMode::MaximumPacking,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
            transcode_kdl_path: None,
        },
    )
}

pub fn serialize_terrain_provenance_manifest(
    records: &[TexLandEcTerrainProvenanceRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + records.len() * 43);
    bytes.extend_from_slice(&TERRAIN_PROVENANCE_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_LAND_EC_TERRAIN_PROVENANCE_VERSION)?;
    bytes.write_u32::<LittleEndian>(records.len() as u32)?;
    for record in records {
        bytes.write_u32::<LittleEndian>(record.material_id)?;
        bytes.write_i32::<LittleEndian>(record.material_name_id)?;
        bytes.write_u32::<LittleEndian>(record.alias_count_index)?;
        bytes.write_u32::<LittleEndian>(record.alias_slot_id)?;
        bytes.write_u64::<LittleEndian>(record.alias_tile_flags)?;
        bytes.write_u32::<LittleEndian>(record.selected_texture_id)?;
        bytes.write_u32::<LittleEndian>(record.canonical_slot_id)?;
        bytes.write_u32::<LittleEndian>(record.selected_layer_index)?;
        bytes.write_f32::<LittleEndian>(record.selected_texture_repetition)?;
        bytes.write_u32::<LittleEndian>(record.primary_texture_id)?;
        bytes.write_u32::<LittleEndian>(record.primary_layer_index)?;
        bytes.write_u8(record.primary_selection_reason)?;
        bytes.write_u16::<LittleEndian>(record.primary_selection_flags)?;
    }
    Ok(bytes)
}

pub fn encode_terrain_provenance_manifest(
    records: &[TexLandEcTerrainProvenanceRecord],
) -> eyre::Result<Vec<u8>> {
    serialize_terrain_provenance_manifest(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::terrain_definition::{
        TerrainDefinitionEntry, TerrainDefinitionPackage, TerrainDefinitionTexture,
        TerrainDefinitionTextureLayer, TerrainDefinitionTileAlias,
    };

    fn tile(art_id: u32, width: u16, height: u16) -> DecodedArtTile {
        DecodedArtTile {
            art_id,
            kind: ArtTileKind::Static,
            width,
            height,
            rgba: vec![255; width as usize * height as usize * 4],
        }
    }

    #[test]
    fn bc7_oriented_land_page_is_block_aligned() {
        let options = TexLandEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
            compression: CompressionFlag::None,
            upscale_64: UpscaleConfig::default(),
            upscale_128: UpscaleConfig::default(),
            upscale_256: UpscaleConfig::default(),
            upscale_512: UpscaleConfig::default(),
            upscale_64_passes: Vec::new(),
            upscale_128_passes: Vec::new(),
            upscale_256_passes: Vec::new(),
            upscale_512_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::Bc7Oriented,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
            transcode_kdl_path: None,
        };

        let (page, leftovers) = build_page(0, vec![tile(11, 3, 3)], &options).unwrap();
        assert!(leftovers.is_empty());
        assert_eq!(page.placed_tiles.len(), 1);
        let placed = &page.placed_tiles[0];
        assert_eq!(placed.x % 4, 0);
        assert_eq!(placed.y % 4, 0);
        assert_eq!(page.record.used_width % 4, 0);
        assert_eq!(page.record.used_height % 4, 0);
    }

    #[test]
    fn terrain_provenance_includes_layer_only_texture_ids() {
        let terrain_definition = TerrainDefinitionPackage {
            entries: vec![TerrainDefinitionEntry {
                id: 13,
                name_id: 0,
                aliases: vec![TerrainDefinitionTileAlias {
                    count_index: 0,
                    alias: 581,
                    tile_flags: 0,
                }],
                texture: Some(TerrainDefinitionTexture {
                    layers: vec![
                        TerrainDefinitionTextureLayer {
                            texture_id: Some(2000130),
                            texture_repetition: 1.0,
                            ..Default::default()
                        },
                        TerrainDefinitionTextureLayer {
                            texture_id: Some(2000131),
                            texture_repetition: 4.0,
                            unk6: 1,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        let selections = vec![TerrainTextureSelection {
            slot_id: 581,
            texture_id: 2000131,
        }];
        let texture_slot_by_texture_id =
            HashMap::from([(2000130, 9001), (2000131, 9002), (2000132, 9003)]);
        let override_texture_ids = HashMap::from([(
            13,
            BTreeMap::from([(
                2000132,
                TerrainLayerProvenance {
                    layer_index: 2,
                    texture_repetition: 6.0,
                },
            )]),
        )]);

        let records = build_terrain_provenance_records(
            &terrain_definition,
            &selections,
            &texture_slot_by_texture_id,
            &override_texture_ids,
        );

        assert!(records.iter().any(|record| {
            record.material_id == 13
                && record.alias_slot_id == 581
                && record.selected_texture_id == 2000130
                && record.canonical_slot_id == 9001
        }));
        assert!(records.iter().any(|record| {
            record.material_id == 13
                && record.alias_slot_id == 581
                && record.selected_texture_id == 2000131
                && record.canonical_slot_id == 9002
        }));
        assert!(records.iter().any(|record| {
            record.material_id == 13
                && record.alias_slot_id == 581
                && record.selected_texture_id == 2000132
                && record.canonical_slot_id == 9003
                && record.selected_layer_index == 2
                && record.selected_texture_repetition == 6.0
        }));
        for record in &records {
            assert_eq!(record.primary_texture_id, 2000131);
            assert_eq!(record.primary_layer_index, 1);
            assert_eq!(
                record.primary_selection_reason,
                TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION
            );
            assert_eq!(
                record.primary_selection_flags & TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION,
                TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION
            );
        }
    }

    #[test]
    fn terrain_override_metadata_preserves_reviewed_codes() {
        let path = std::env::temp_dir().join(format!(
            "ec_terrain_overrides_metadata_test_{}.kdl",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"
terrain 52 {
    policy "smooth" code="reviewed_runtime_policy"
    layer "t0" tex=2000510 stretch=5.0 code="reviewed_layer_texture"
}
"#,
        )
        .expect("write override fixture");

        let manifest = serialize_terrain_overrides_metadata(&path)
            .expect("serialize terrain override metadata");
        let json = String::from_utf8(manifest.bytes).expect("metadata json is utf8");
        let _ = std::fs::remove_file(&path);

        assert_eq!(manifest.entry_count, 1);
        assert!(json.contains("\"schema\": \"tex_land_ec_terrain_overrides\""));
        assert!(json.contains("\"material_id\": 52"));
        assert!(json.contains("\"code\": \"reviewed_layer_texture\""));
        assert!(json.contains("\"texture_id\": 2000510"));
    }
}
