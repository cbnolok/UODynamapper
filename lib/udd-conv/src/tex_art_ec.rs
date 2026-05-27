//! Build-time and runtime support for `tex_art_ec.uddp`.
//!
//! Sources EC item/static visuals from tileart-owned references, resolving the
//! selected Enhanced and Classic texture payloads from `Texture.uop` and
//! `LegacyTexture.uop`.
//! Package layout:
//! - `pages/{page_index}.rgba8888` or `pages/{page_index}.bc7`: atlas page payloads.
//! - `metadata/pages.bin`: page table with atlas dimensions, occupancy and pixel format.
//! - `metadata/slots.bin`: sparse slot table with one record per art_id.
//!
//! This module deliberately separates three concerns:
//! - source selection: decide which EC tileart definitions are actually static art
//!   and which source texture each static should resolve to.
//! - atlas packing: deduplicate identical source textures, pack the decoded images,
//!   then alias repeated slots back to the canonical packed rectangle.
//! - runtime loading: expose a page table and sparse slot table that the renderer
//!   can query without needing to understand any of the original EC source files.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{size2, AtlasAllocator};

use crate::bc7::{
    bc7_encode_progress_units, encode_for_vram_with_bc7_rdo_lambda_and_progress,
    preferred_bc7_encoder_backend, ImageExtent, RawImageFormat, VramTextureEncoding,
};
use crate::{AtlasPackingMode, extrude_rgba_rect_edges, merge_unplaced_tiles, resolve_packing_axis};
use crate::package_progress::{
    atlas_payload_finish_message, atlas_payload_progress_message, build_and_write_package,
};
use crate::source_paths::{find_first_existing_file, source_path_label};
use udd_assets::tex_art_cc::{page_entry_path, PagePixelFormat};
use udd_assets::tex_art_ec::{
    TexArtEcCropAdjustment, TexArtEcPageRecord, TexArtEcSlotRecord, MISSING_PAGE_INDEX,
    MISSING_PAGE_TILE_INDEX, PAGE_MANIFEST_ENTRY_PATH, SLOT_FLAG_LAND, SLOT_FLAG_PRESENT,
    SLOT_FLAG_STATIC, SLOT_MANIFEST_ENTRY_PATH,
};
use udd_container::xxh64_virtual_path;
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::enhanced::{
    terrain_definition::TerrainDefinitionPackage,
    textures::{TextureFile, Textures},
    tile_database::ArtDefinition,
    tileart::{ArtData, ArtTexture, TaeFlag, TileType},
};

use crate::upscale::{apply_filter_passes, UpscaleFilter};
use crate::tex_art_cc::upscale_algorithm_code;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"EAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"EASL";
/// Bump version when the binary layout of either manifest changes.
const TEX_ART_EC_METADATA_VERSION: u32 = 5;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 4096;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

pub struct TexArtEcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    /// When `true`, trim transparent borders from decoded EC art textures before
    /// atlas packing. Matching tile metadata must subtract the same top/left crop
    /// from its EC sampling start coordinates.
    pub crop_transparent_bounds: bool,
    pub compression: CompressionFlag,
    pub upscale: UpscaleFilter,
    pub upscale_passes: Vec<UpscaleFilter>,
    pub pixel_format: PagePixelFormat,
    pub packing_mode: AtlasPackingMode,
    pub filtering_ready: bool,
    pub bc7_rdo_lambda: f32,
}

impl Default for TexArtEcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            crop_transparent_bounds: false,
            compression: CompressionFlag::JpegXl,
            upscale: UpscaleFilter::default(),
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::MaximumPacking,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        }
    }
}

fn art_upscale_passes(options: &TexArtEcAtlasOptions) -> Vec<UpscaleFilter> {
    if options.upscale_passes.is_empty() {
        vec![options.upscale]
    } else {
        options.upscale_passes.clone()
    }
}

fn packing_mode_repr(mode: AtlasPackingMode) -> u8 {
    match mode {
        AtlasPackingMode::MaximumPacking => 0,
        AtlasPackingMode::Bc7Oriented => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtEcBuildSummary {
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub cropped_slot_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceClipRect {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
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
    pub upscale_factor: u16,
    pub upscale_algorithm: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PreparedArtDecodeGroup {
    canonical_art_id: u32,
    kind: ArtTileKind,
    texture_bounds: ArtTexture,
    file: TextureFile,
    alias_art_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
struct DecodedArtDecodeGroup {
    canonical_art_id: u32,
    kind: ArtTileKind,
    width: u16,
    height: u16,
    upscale_factor: u16,
    upscale_algorithm: u16,
    rgba: Vec<u8>,
    crop_adjustment: TexArtEcCropAdjustment,
    alias_art_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSourceKey {
    World(u32),
    Legacy(u32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RequestedSourceWindow {
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalTileKey {
    pub source: TextureSourceKey,
    pub window: Option<SourceClipRect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderedTileKey {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotAlias {
    pub art_id: u32,
    pub canonical_art_id: u32,
}

pub struct TexArtEcLoadedSources {
    pub tileart_path: PathBuf,
    pub terrain_definition_path: PathBuf,
    pub stringdict_path: PathBuf,
    pub texture_uop_path: Option<PathBuf>,
    pub legacy_texture_uop_path: Option<PathBuf>,
    pub terrain_definition: TerrainDefinitionPackage,
    pub terrain_source_texture_ids: HashSet<u32>,
    pub art_definition: ArtDefinition,
    pub world_textures: Option<Textures>,
    pub legacy_textures: Option<Textures>,
}

impl TexArtEcLoadedSources {
    pub fn terrain_definition(&self) -> &TerrainDefinitionPackage {
        &self.terrain_definition
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
    pub upscale_factor: u16,
    pub upscale_algorithm: u16,
}

#[derive(Debug, Clone)]
pub struct BuiltPage {
    pub record: TexArtEcPageRecord,
    /// Raw RGBA8888 pixels produced by the guillotiere packer.
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

fn find_string_dictionary_path(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    find_first_existing_file(source_dirs, &["string_dictionary.uop"])
}

fn source_file_label(path: &Path) -> String {
    path.parent()
        .map(|parent| source_path_label(parent, path))
        .unwrap_or_else(|| path.display().to_string())
}

pub fn convert_tex_art_ec_uop_to_tex_art_ec_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<TexArtEcBuildSummary> {
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<TexArtEcBuildSummary> {
    let sources = load_tex_art_ec_sources(source_dirs)?;

    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(&sources, out_file, options)
}

pub fn convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(
    sources: &TexArtEcLoadedSources,
    out_file: &Path,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<TexArtEcBuildSummary> {
    validate_options(options)?;

    info!(
        "Converting EC Art from Texture.uop / LegacyTexture.uop to {}",
        out_file.display()
    );
    println!("Using tileart.uop: {}", source_file_label(&sources.tileart_path));
    println!(
        "Using TerrainDefinition.uop: {}",
        source_file_label(&sources.terrain_definition_path)
    );
    println!(
        "Using string dictionary: {}",
        source_file_label(&sources.stringdict_path)
    );
    if let Some(path) = sources.texture_uop_path.as_ref() {
        println!("Using Texture.uop: {}", source_file_label(path));
    }
    if let Some(path) = sources.legacy_texture_uop_path.as_ref() {
        println!("Using LegacyTexture.uop: {}", source_file_label(path));
    }

    let slot_count = 0x10000_u32; // 65536 max art items in EC
    let (decoded_tiles, aliases, crop_adjustments) = decode_present_tiles(
        &sources.art_definition,
        &sources.terrain_source_texture_ids,
        sources.world_textures.as_ref(),
        sources.legacy_textures.as_ref(),
        options,
    )?;

    let (pages, mut slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
    apply_slot_aliases(&mut slot_records, &aliases)?;
    let populated_slot_count = slot_records.iter().filter(|slot| slot.is_present()).count() as u32;
    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(PAGE_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &page_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;

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
        "EC art atlas pages",
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
        "EC art atlas pages",
        use_bc7,
        compression,
        options.bc7_rdo_lambda,
    ));

    build_and_write_package(&mut package, out_file)?;

    Ok(TexArtEcBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
        cropped_slot_count: crop_adjustments
            .values()
            .filter(|adjustment| **adjustment != TexArtEcCropAdjustment::default())
            .count() as u32,
    })
}

pub fn compute_tex_art_ec_crop_adjustments_from_sources(
    source_dirs: &[PathBuf],
) -> eyre::Result<Vec<Option<TexArtEcCropAdjustment>>> {
    let sources = load_tex_art_ec_sources(source_dirs)?;
    compute_tex_art_ec_crop_adjustments_from_loaded_sources(&sources)
}

pub fn compute_tex_art_ec_crop_adjustments_from_loaded_sources(
    sources: &TexArtEcLoadedSources,
) -> eyre::Result<Vec<Option<TexArtEcCropAdjustment>>> {
    let mut options = TexArtEcAtlasOptions::default();
    options.crop_transparent_bounds = true;

    let (_decoded_tiles, aliases, canonical_adjustments) = decode_present_tiles(
        &sources.art_definition,
        &sources.terrain_source_texture_ids,
        sources.world_textures.as_ref(),
        sources.legacy_textures.as_ref(),
        &options,
    )?;

    Ok(build_crop_adjustment_lookup(
        0x10000,
        &canonical_adjustments,
        &aliases,
    ))
}

pub fn load_tex_art_ec_sources(source_dirs: &[PathBuf]) -> eyre::Result<TexArtEcLoadedSources> {
    let tileart_path = find_first_existing_file(source_dirs, &["tileart.uop"])
        .ok_or_else(|| eyre::eyre!("missing required file: tileart.uop"))?;
    let terrain_definition_path = find_first_existing_file(source_dirs, &["TerrainDefinition.uop"])
        .ok_or_else(|| eyre::eyre!("missing required file: TerrainDefinition.uop"))?;
    let stringdict_path = find_string_dictionary_path(source_dirs)
        .ok_or_else(|| eyre::eyre!("missing string_dictionary.uop"))?;
    let texture_uop_path = find_first_existing_file(source_dirs, &["Texture.uop"]);
    let legacy_texture_uop_path = find_first_existing_file(source_dirs, &["LegacyTexture.uop"]);

    if texture_uop_path.is_none() && legacy_texture_uop_path.is_none() {
        eyre::bail!("missing required files: Texture.uop and LegacyTexture.uop not found");
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

    let art_definition = ArtDefinition::load(&tileart_path, &stringdict_path)
        .wrap_err("load tileart-driven art definition")?;
    let terrain_definition = TerrainDefinitionPackage::load(&terrain_definition_path)
        .wrap_err("load TerrainDefinition.uop")?;
    let terrain_source_texture_ids = terrain_definition
        .land_source_texture_ids()
        .into_iter()
        .collect();

    Ok(TexArtEcLoadedSources {
        tileart_path,
        terrain_definition_path,
        stringdict_path,
        texture_uop_path,
        legacy_texture_uop_path,
        terrain_definition,
        terrain_source_texture_ids,
        art_definition,
        world_textures,
        legacy_textures,
    })
}

fn validate_options(options: &TexArtEcAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    Ok(())
}

fn decode_present_tiles(
    art_definition: &ArtDefinition,
    terrain_source_texture_ids: &HashSet<u32>,
    world_textures: Option<&Textures>,
    legacy_textures: Option<&Textures>,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<(
    Vec<DecodedArtTile>,
    Vec<SlotAlias>,
    HashMap<u32, TexArtEcCropAdjustment>,
)> {
    // Art ownership comes from tileart definitions. Resolve each item/static's
    // selected EC or CC texture payload, but keep the slot table keyed by art id.
    // Multiple art ids can intentionally share the same texture payload, but the
    // tileart sampling window is still part of the packed-art contract. Deduping
    // therefore has to key on both the resolved source texture and the requested
    // per-art source window before aliasing slot records afterward.
    let mut decoded_tiles = Vec::new();
    let mut aliases = Vec::new();
    let mut canonical_by_source: HashMap<CanonicalTileKey, usize> = HashMap::new();
    let mut canonical_by_rendered_tile = HashMap::new();
    let mut crop_adjustments = HashMap::new();

    let mut art_ids = art_definition
        .definitions
        .keys()
        .copied()
        .collect::<Vec<_>>();
    art_ids.sort_unstable();

    let resolve_pb = ProgressBar::new(art_ids.len() as u64);
    resolve_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} extracting EC art tile sources ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    let mut decode_groups: Vec<PreparedArtDecodeGroup> = Vec::new();

    for art_id in art_ids {
        resolve_pb.inc(1);
        let art_data = art_definition
            .definitions
            .get(&art_id)
            .context("missing tileart definition during art decode")?;

        if !should_include_art_tile(art_data.tile_type) {
            continue;
        }
        let resolved = if let (Some(texture), Some(_)) =
            (art_data.ec_texture.as_ref(), world_textures)
        {
            Some((TextureSourceKey::World(texture.texture_id), texture))
        } else if let (Some(texture), Some(_)) = (art_data.cc_texture.as_ref(), legacy_textures) {
            Some((TextureSourceKey::Legacy(texture.texture_id), texture))
        } else {
            None
        };

        if let Some((source_key, texture_bounds)) = resolved {
            // Tileart ownership is authoritative for tex_art_ec packing.
            // A source texture id may legitimately appear in both terrain and tileart
            // metadata, and shared ids should survive in both packages.
            let canonical_key = CanonicalTileKey {
                source: source_key,
                window: requested_source_window(texture_bounds),
            };

            if let Some(&group_index) = canonical_by_source.get(&canonical_key) {
                decode_groups[group_index].alias_art_ids.push(art_id as u32);
                continue;
            }

            let file = match source_key {
                TextureSourceKey::World(texture_id) => world_textures
                    .expect("world texture source was selected only when available")
                    .get_from_id(texture_id)?,
                TextureSourceKey::Legacy(texture_id) => legacy_textures
                    .expect("legacy texture source was selected only when available")
                    .get_from_id(texture_id)?,
            };
            let Some(file) = file else {
                continue;
            };

            if should_skip_terrain_material_static(
                art_data,
                texture_bounds,
                &file,
                terrain_source_texture_ids,
            )? {
                continue;
            }

            canonical_by_source.insert(canonical_key, decode_groups.len());
            decode_groups.push(PreparedArtDecodeGroup {
                canonical_art_id: art_id as u32,
                kind: art_tile_kind_for_tileart(art_data.tile_type, art_data.flags),
                texture_bounds: texture_bounds.clone(),
                file,
                alias_art_ids: Vec::new(),
            });
        }
    }
    resolve_pb.finish_with_message("EC art tile sources extracted");

    let decode_pb = ProgressBar::new(decode_groups.len() as u64);
    decode_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} extracting unique EC art tiles ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    let decoded_groups = decode_groups
        .par_iter()
        .map(|group| -> eyre::Result<DecodedArtDecodeGroup> {
            let img = group.file.decode_to_rgba()?;
            let rgba = img.to_rgba8();
            let source_width = rgba.width() as u16;
            let source_height = rgba.height() as u16;
            let clip_rect =
                normalized_source_clip_rect(source_width, source_height, &group.texture_bounds);
            let (width, height, rgba, crop_adjustment) = if options.crop_transparent_bounds {
                crop_rgba_tile_to_bounds(source_width, source_height, rgba.into_raw(), clip_rect)?
            } else {
                apply_requested_clip_rect(source_width, source_height, rgba.into_raw(), clip_rect)?
            };

            let upscale_passes = art_upscale_passes(options);
            let (width, height, rgba, upscale_factor, upscale_filter) =
                apply_filter_passes(width as u32, height as u32, &rgba, &upscale_passes);

            decode_pb.inc(1);
            Ok(DecodedArtDecodeGroup {
                canonical_art_id: group.canonical_art_id,
                kind: group.kind,
                width: width as u16,
                height: height as u16,
                upscale_factor: upscale_factor as u16,
                upscale_algorithm: upscale_algorithm_code(upscale_filter),
                rgba,
                crop_adjustment,
                alias_art_ids: group.alias_art_ids.clone(),
            })
        })
        .collect::<Vec<_>>();
    decode_pb.finish_with_message("Unique EC art tiles extracted");

    for decoded_group in decoded_groups {
        let decoded_group = decoded_group?;
        if let Some(rendered_canonical_art_id) = register_rendered_tile_alias(
            decoded_group.canonical_art_id,
            decoded_group.width,
            decoded_group.height,
            &decoded_group.rgba,
            &mut canonical_by_rendered_tile,
        ) {
            aliases.push(SlotAlias {
                art_id: decoded_group.canonical_art_id,
                canonical_art_id: rendered_canonical_art_id,
            });
        } else {
            crop_adjustments.insert(
                decoded_group.canonical_art_id,
                decoded_group.crop_adjustment,
            );
            decoded_tiles.push(DecodedArtTile {
                art_id: decoded_group.canonical_art_id,
                kind: decoded_group.kind,
                width: decoded_group.width,
                height: decoded_group.height,
                upscale_factor: decoded_group.upscale_factor,
                upscale_algorithm: decoded_group.upscale_algorithm,
                rgba: decoded_group.rgba,
            });
        }

        for alias_art_id in decoded_group.alias_art_ids {
            aliases.push(SlotAlias {
                art_id: alias_art_id,
                canonical_art_id: decoded_group.canonical_art_id,
            });
        }
    }

    decoded_tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });

    Ok((decoded_tiles, aliases, crop_adjustments))
}

fn should_include_art_tile(tile_type: TileType) -> bool {
    // Tileart ownership is authoritative for tex_art_ec packing. Surface-like
    // entries still belong to tileart even when higher-level metadata classifies
    // them as solid or liquid for runtime routing. Entries without a decodable
    // base naturally fall out later when no source texture can be resolved.
    let _ = tile_type;
    true
}

pub fn art_tile_kind_for_tileart(tile_type: TileType, flags: TaeFlag) -> ArtTileKind {
    if tile_type != TileType::Static || flags.contains(TaeFlag::Unused1) {
        ArtTileKind::Land
    } else {
        ArtTileKind::Static
    }
}

pub fn requested_source_window(texture: &ArtTexture) -> Option<SourceClipRect> {
    Some(SourceClipRect {
        left: texture.start_x.max(0) as u16,
        top: texture.start_y.max(0) as u16,
        right: texture.end_x.max(0) as u16,
        bottom: texture.end_y.max(0) as u16,
    })
}

fn should_skip_terrain_material_static(
    _art_data: &ArtData,
    _texture: &ArtTexture,
    _file: &TextureFile,
    _terrain_source_texture_ids: &HashSet<u32>,
) -> eyre::Result<bool> {
    // Tileart ownership is authoritative for tex_art_ec. Sharing a source texture
    // id with terrain materials, or using a whole flat material sheet, is not
    // enough reason to drop a static art entry. If an art-owned surface should
    // route to tex_land_ec instead, routing metadata must prove that separately.
    Ok(false)
}

pub fn register_rendered_tile_alias(
    art_id: u32,
    width: u16,
    height: u16,
    rgba: &[u8],
    canonical_by_rendered_tile: &mut HashMap<Vec<u8>, u32>,
) -> Option<u32> {
    let key = [
        width.to_le_bytes().to_vec(),
        height.to_le_bytes().to_vec(),
        rgba.to_vec(),
    ]
    .concat();
    if let Some(&canonical_art_id) = canonical_by_rendered_tile.get(&key) {
        Some(canonical_art_id)
    } else {
        canonical_by_rendered_tile.insert(key, art_id);
        None
    }
}

pub fn normalized_source_clip_rect(
    width: u16,
    height: u16,
    texture: &ArtTexture,
) -> Option<SourceClipRect> {
    let left = texture.start_x.max(0).min(i32::from(width)) as u16;
    let top = texture.start_y.max(0).min(i32::from(height)) as u16;
    let right = texture.end_x.max(0).min(i32::from(width)) as u16;
    let bottom = texture.end_y.max(0).min(i32::from(height)) as u16;

    if right <= left || bottom <= top {
        return None;
    }

    if left == 0 && top == 0 && right == width && bottom == height {
        return None;
    }

    Some(SourceClipRect {
        left,
        top,
        right,
        bottom,
    })
}

pub fn crop_rgba_tile_to_bounds(
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    clip_rect: Option<SourceClipRect>,
) -> eyre::Result<(u16, u16, Vec<u8>, TexArtEcCropAdjustment)> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for crop {}x{}: expected {}, got {}",
            width,
            height,
            expected_len,
            rgba.len()
        );
    }

    let width_usize = width as usize;
    let height_usize = height as usize;
    let clip_left = clip_rect.map(|clip| clip.left as usize).unwrap_or(0);
    let clip_top = clip_rect.map(|clip| clip.top as usize).unwrap_or(0);
    let clip_right = clip_rect
        .map(|clip| clip.right as usize)
        .unwrap_or(width_usize);
    let clip_bottom = clip_rect
        .map(|clip| clip.bottom as usize)
        .unwrap_or(height_usize);
    let mut min_x = clip_right;
    let mut min_y = clip_bottom;
    let mut max_x = clip_left;
    let mut max_y = clip_top;
    let mut found_opaque = false;

    for y in clip_top..clip_bottom {
        for x in clip_left..clip_right {
            let alpha = rgba[(y * width_usize + x) * 4 + 3];
            if alpha != 0 {
                found_opaque = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if !found_opaque {
        if let Some(clip) = clip_rect {
            return crop_rgba_subrect(
                width,
                height,
                rgba,
                clip.left as usize,
                clip.top as usize,
                clip.right as usize,
                clip.bottom as usize,
            );
        }
        return Ok((width, height, rgba, TexArtEcCropAdjustment::default()));
    }

    if min_x == clip_left
        && min_y == clip_top
        && max_x + 1 == clip_right
        && max_y + 1 == clip_bottom
    {
        if let Some(clip) = clip_rect {
            return crop_rgba_subrect(
                width,
                height,
                rgba,
                clip.left as usize,
                clip.top as usize,
                clip.right as usize,
                clip.bottom as usize,
            );
        }
        return Ok((width, height, rgba, TexArtEcCropAdjustment::default()));
    }

    crop_rgba_subrect(width, height, rgba, min_x, min_y, max_x + 1, max_y + 1)
}

pub fn apply_requested_clip_rect(
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    clip_rect: Option<SourceClipRect>,
) -> eyre::Result<(u16, u16, Vec<u8>, TexArtEcCropAdjustment)> {
    if let Some(clip) = clip_rect {
        crop_rgba_subrect(
            width,
            height,
            rgba,
            clip.left as usize,
            clip.top as usize,
            clip.right as usize,
            clip.bottom as usize,
        )
    } else {
        Ok((width, height, rgba, TexArtEcCropAdjustment::default()))
    }
}

fn crop_rgba_subrect(
    width: u16,
    _height: u16,
    rgba: Vec<u8>,
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
) -> eyre::Result<(u16, u16, Vec<u8>, TexArtEcCropAdjustment)> {
    let cropped_width = (right - left) as u16;
    let cropped_height = (bottom - top) as u16;
    let mut cropped = vec![0u8; cropped_width as usize * cropped_height as usize * 4];
    let src_stride = width as usize * 4;
    let dst_stride = cropped_width as usize * 4;
    for row in 0..cropped_height as usize {
        let src_start = ((top + row) * src_stride) + left * 4;
        let dst_start = row * dst_stride;
        cropped[dst_start..dst_start + dst_stride]
            .copy_from_slice(&rgba[src_start..src_start + dst_stride]);
    }

    Ok((
        cropped_width,
        cropped_height,
        cropped,
        TexArtEcCropAdjustment {
            left: left as i16,
            top: top as i16,
        },
    ))
}

pub fn build_crop_adjustment_lookup(
    slot_count: u32,
    canonical_adjustments: &HashMap<u32, TexArtEcCropAdjustment>,
    aliases: &[SlotAlias],
) -> Vec<Option<TexArtEcCropAdjustment>> {
    let mut adjustments = vec![None; slot_count as usize];
    for (&art_id, &adjustment) in canonical_adjustments {
        if let Some(slot) = adjustments.get_mut(art_id as usize) {
            *slot = Some(adjustment);
        }
    }
    for alias in aliases {
        let Some(adjustment) = canonical_adjustments.get(&alias.canonical_art_id).copied() else {
            continue;
        };
        if let Some(slot) = adjustments.get_mut(alias.art_id as usize) {
            *slot = Some(adjustment);
        }
    }
    adjustments
}

pub fn apply_slot_aliases(
    slots: &mut [TexArtEcSlotRecord],
    aliases: &[SlotAlias],
) -> eyre::Result<()> {
    for alias in aliases {
        let canonical = *slots
            .get(alias.canonical_art_id as usize)
            .context("canonical tex_art_ec slot outside slot table")?;
        let slot = slots
            .get_mut(alias.art_id as usize)
            .context("alias tex_art_ec slot outside slot table")?;
        if !canonical.is_present() {
            eyre::bail!(
                "canonical tex_art_ec slot {} missing while applying alias {}",
                alias.canonical_art_id,
                alias.art_id
            );
        }
        *slot = TexArtEcSlotRecord {
            art_id: alias.art_id,
            ..canonical
        };
    }
    Ok(())
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<TexArtEcSlotRecord>)> {
    // Like the CC path, keep a sparse slot table for the full art id range. That
    // makes runtime lookup deterministic even when many ids are absent.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(TexArtEcSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    let total_tiles = remaining.len() as u64;
    remaining.sort_by_key(|tile| tile.art_id);
    let mut page_index = 0u32;
    let pb = ProgressBar::new(total_tiles);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message("creating EC art atlas pages");

    while !remaining.is_empty() {
        pb.set_message(format!("creating EC art atlas page {page_index}"));
        let (page_tiles, leftovers) = take_page_tile_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_tiles, options)?;
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
            *slot = TexArtEcSlotRecord {
                art_id: placed.art_id,
                page_index,
                page_tile_index: placed.page_tile_index,
                flags: placed.kind.slot_flags(),
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
                upscale_factor: placed.upscale_factor,
                upscale_algorithm: placed.upscale_algorithm,
            };
        }

        pb.inc(page.placed_tiles.len() as u64);
        pages.push(page);
        remaining = merge_unplaced_tiles(leftovers, unplaced, |tile| tile.art_id);
        page_index += 1;
    }

    pb.finish_with_message(format!("EC art atlas pages created ({page_index} pages)"));

    Ok((pages, slot_records))
}

fn take_page_tile_prefix(
    tiles: Vec<DecodedArtTile>,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<(Vec<DecodedArtTile>, Vec<DecodedArtTile>)> {
    let prefix_len = max_fitting_page_prefix_len(&tiles, options)?;

    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any art tile into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }

    let mut leftovers = tiles;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((selected, leftovers))
}

fn max_fitting_page_prefix_len(
    tiles: &[DecodedArtTile],
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<usize> {
    let mut low = 1usize;
    let mut high = tiles.len();
    let mut best = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        if page_prefix_fits(&tiles[..mid], options)? {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }

    Ok(best)
}

fn page_prefix_fits(tiles: &[DecodedArtTile], options: &TexArtEcAtlasOptions) -> eyre::Result<bool> {
    let mut to_pack = tiles.to_vec();
    sort_tiles_within_page(&mut to_pack, options);

    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));

    for tile in &to_pack {
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

        if allocator
            .allocate(size2(width_axis.alloc_extent as i32, height_axis.alloc_extent as i32))
            .is_none()
        {
            return Ok(false);
        }
    }

    Ok(true)
}

fn sort_tiles_within_page(tiles: &mut [DecodedArtTile], options: &TexArtEcAtlasOptions) {
    tiles.sort_by(|left, right| {
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });
}

fn sort_area(tile: &DecodedArtTile, options: &TexArtEcAtlasOptions) -> u32 {
    match options.packing_mode {
        AtlasPackingMode::MaximumPacking => tile.width as u32 * tile.height as u32,
        AtlasPackingMode::Bc7Oriented => {
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
    }
}

fn build_page(
    page_index: u32,
    mut tiles: Vec<DecodedArtTile>,
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedArtTile>)> {
    // The working page is always a full-size RGBA canvas. Cropping happens only
    // at storage time so slot coordinates remain expressed in atlas-page space.
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut placed_tiles = Vec::new();
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let _gutter = i32::from(options.gutter);

    sort_tiles_within_page(&mut tiles, options);

    for tile in tiles {
        // A few EC statics legitimately span the full atlas width. Keep the gutter
        // where it fits, but drop it on a saturated axis so those tiles can still be packed.
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

            // Record the furthest texel that carries real image data. The package
            // stores only `used_width x used_height`, while the manifest preserves
            // the original atlas dimensions needed to interpret these coordinates.
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
                upscale_factor: tile.upscale_factor,
                upscale_algorithm: tile.upscale_algorithm,
            });
        } else {
            leftovers.push(tile);
        }
    }

    Ok((
        BuiltPage {
            record: TexArtEcPageRecord {
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
    // The crop is intentionally one-sided: we remove only empty rows/columns from
    // the lower-right region. That avoids rewriting slot coordinates or changing
    // the atlas origin while still cutting a large amount of dead transparent space.
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
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // Manifest records tell the reader how to reinterpret cropped payloads as
    // logical atlas pages. `used_width/used_height` describe the stored bytes,
    // while `atlas_width/atlas_height` keep the public coordinate system stable.
    let pixel_format = options.pixel_format;
    let mut bytes = Vec::with_capacity(26 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_ART_EC_METADATA_VERSION)?;
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
    slots: &[TexArtEcSlotRecord],
    options: &TexArtEcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(25 + slots.len() * 24);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_ART_EC_METADATA_VERSION)?;
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
        bytes.write_u16::<LittleEndian>(slot.upscale_factor.max(1))?;
        bytes.write_u16::<LittleEndian>(slot.upscale_algorithm)?;
    }
    Ok(bytes)
}

pub fn encode_slot_manifest(
    slots: &[TexArtEcSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &TexArtEcAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::default(),
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::MaximumPacking,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(art_id: u32, width: u16, height: u16) -> DecodedArtTile {
        DecodedArtTile {
            art_id,
            kind: ArtTileKind::Static,
            width,
            height,
            upscale_factor: 1,
            upscale_algorithm: 0,
            rgba: vec![255; width as usize * height as usize * 4],
        }
    }

    #[test]
    fn requeue_path_preserves_all_unplaced_tiles() {
        let merged = merge_unplaced_tiles(
            vec![tile(10, 1, 1), tile(30, 1, 1)],
            vec![tile(20, 1, 1), tile(15, 1, 1)],
            |tile| tile.art_id,
        );

        let ids = merged.into_iter().map(|tile| tile.art_id).collect::<Vec<_>>();
        assert_eq!(ids, vec![10, 15, 20, 30]);
    }

    #[test]
    fn bc7_oriented_art_page_is_block_aligned() {
        let options = TexArtEcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
            crop_transparent_bounds: false,
            compression: CompressionFlag::None,
            upscale: UpscaleFilter::None,
            upscale_passes: Vec::new(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::Bc7Oriented,
            filtering_ready: false,
            bc7_rdo_lambda: crate::bc7::DEFAULT_BC7_RDO_LAMBDA,
        };

        let (page, leftovers) = build_page(0, vec![tile(7, 3, 3)], &options).unwrap();
        assert!(leftovers.is_empty());
        assert_eq!(page.placed_tiles.len(), 1);
        let placed = &page.placed_tiles[0];
        assert_eq!(placed.x % 4, 0);
        assert_eq!(placed.y % 4, 0);
        assert_eq!(page.record.used_width % 4, 0);
        assert_eq!(page.record.used_height % 4, 0);
        assert_eq!(placed.width, 3);
        assert_eq!(placed.height, 3);
    }

    #[test]
    fn art_decode_includes_tileart_owned_surface_entries() {
        assert!(should_include_art_tile(TileType::Static));
        assert!(should_include_art_tile(TileType::Solid));
        assert!(should_include_art_tile(TileType::Liquid));
    }
}
