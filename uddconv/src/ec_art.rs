//! Build-time and runtime support for `ec_art.uddp`.
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
        tile_database::ArtDefinition,
        textures::{TextureFile, Textures},
        tileart::{ArtData, ArtTexture, TileType},
    },
    udd::{
        xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
        LookupMode, UddpBuilder, UddpReader,
    },
};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"EAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"EASL";
/// Bump version when the binary layout of either manifest changes.
const EC_ART_METADATA_VERSION: u32 = 2;
pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

fn find_string_dictionary_path(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    find_first_existing_file(source_dirs, &["string_dictionary.uop"])
}

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 4096;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcArtAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    /// When `true`, trim transparent borders from decoded EC art textures before
    /// atlas packing. Matching tile metadata must subtract the same top/left crop
    /// from its EC sampling start coordinates.
    pub crop_transparent_bounds: bool,
    /// When `true` each atlas page is BC7-compressed on the CPU before being
    /// stored in the UDDP container, reducing VRAM usage by ~8×.
    pub use_bc7: bool,
}

impl Default for EcArtAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            crop_transparent_bounds: false,
            use_bc7: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcArtBuildSummary {
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub cropped_slot_count: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EcArtCropAdjustment {
    pub left: i16,
    pub top: i16,
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
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PreparedArtDecodeGroup {
    canonical_art_id: u32,
    texture_bounds: ArtTexture,
    file: TextureFile,
    alias_art_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
struct DecodedArtDecodeGroup {
    canonical_art_id: u32,
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    crop_adjustment: EcArtCropAdjustment,
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

pub struct EcArtLoadedSources {
    pub tileart_path: PathBuf,
    pub terrain_definition_path: PathBuf,
    pub stringdict_path: PathBuf,
    pub texture_uop_path: Option<PathBuf>,
    pub legacy_texture_uop_path: Option<PathBuf>,
    pub terrain_source_texture_ids: HashSet<u32>,
    pub art_definition: ArtDefinition,
    pub world_textures: Option<Textures>,
    pub legacy_textures: Option<Textures>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcArtPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    /// Pixel format of the page payload stored in the UDDP container.
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcArtSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl EcArtSlotRecord {
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
    pub record: EcArtPageRecord,
    /// Raw RGBA8888 pixels produced by the guillotiere packer.
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

pub struct EcArtPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    pages: Vec<EcArtPageRecord>,
    slots: Vec<EcArtSlotRecord>,
}

impl EcArtPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::open(fs::read(path.as_ref())?)
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let page_manifest = read_path_entry(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("ec_art.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("ec_art.uddp missing metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, pages) = parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slots) = parse_slot_manifest(&slot_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("ec_art metadata headers disagree on atlas dimensions or gutter");
        }

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            pages,
            slots,
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

    pub fn pages(&self) -> &[EcArtPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[EcArtSlotRecord] {
        &self.slots
    }

    pub fn slot_record(&self, art_id: u32) -> Option<&EcArtSlotRecord> {
        self.slots.get(art_id as usize)
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&EcArtSlotRecord> {
        self.slot_record(art_id).filter(|slot| slot.is_present())
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

pub fn convert_ec_art_uop_to_ec_art_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &EcArtAtlasOptions,
) -> eyre::Result<EcArtBuildSummary> {
    convert_ec_art_uop_to_ec_art_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_ec_art_uop_to_ec_art_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &EcArtAtlasOptions,
) -> eyre::Result<EcArtBuildSummary> {
    validate_options(options)?;

    let sources = load_ec_art_sources(source_dirs)?;

    info!("Converting EC Art from Texture.uop / LegacyTexture.uop to {}", out_file.display());
    println!("Using tileart.uop: {}", sources.tileart_path.display());
    println!(
        "Using TerrainDefinition.uop: {}",
        sources.terrain_definition_path.display()
    );
    println!("Using string dictionary: {}", sources.stringdict_path.display());
    if let Some(path) = sources.texture_uop_path.as_ref() {
        println!("Using Texture.uop: {}", path.display());
    }
    if let Some(path) = sources.legacy_texture_uop_path.as_ref() {
        println!("Using LegacyTexture.uop: {}", path.display());
    }

    let slot_count = 0x10000_u32; // 65536 max art items in EC
    let (decoded_tiles, aliases, crop_adjustments) = decode_present_tiles(
        &sources.art_definition,
        &sources.terrain_source_texture_ids,
        sources.world_textures.as_ref(),
        sources.legacy_textures.as_ref(),
        options.crop_transparent_bounds,
    )?;

    let (pages, mut slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
    apply_slot_aliases(&mut slot_records, &aliases)?;
    let populated_slot_count = slot_records.iter().filter(|slot| slot.is_present()).count() as u32;
    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(PAGE_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &page_manifest,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;

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
                let encoded = encode_for_vram(&page.pixels, extent, RawImageFormat::Rgba8888, encoding)
                    .map_err(|e| eyre::eyre!("BC7 encode page {}: {e}", page.record.page_index))?
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

    Ok(EcArtBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
        cropped_slot_count: crop_adjustments.values().filter(|adjustment| **adjustment != EcArtCropAdjustment::default()).count() as u32,
    })
}

pub fn compute_ec_art_crop_adjustments_from_sources(
    source_dirs: &[PathBuf],
) -> eyre::Result<Vec<Option<EcArtCropAdjustment>>> {
    let sources = load_ec_art_sources(source_dirs)?;
    let (_decoded_tiles, aliases, canonical_adjustments) = decode_present_tiles(
        &sources.art_definition,
        &sources.terrain_source_texture_ids,
        sources.world_textures.as_ref(),
        sources.legacy_textures.as_ref(),
        true,
    )?;

    Ok(build_crop_adjustment_lookup(
        0x10000,
        &canonical_adjustments,
        &aliases,
    ))
}

fn load_ec_art_sources(source_dirs: &[PathBuf]) -> eyre::Result<EcArtLoadedSources> {
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

    Ok(EcArtLoadedSources {
        tileart_path,
        terrain_definition_path,
        stringdict_path,
        texture_uop_path,
        legacy_texture_uop_path,
        terrain_source_texture_ids: terrain_definition.land_source_texture_ids().into_iter().collect(),
        art_definition,
        world_textures,
        legacy_textures,
    })
}

fn validate_options(options: &EcArtAtlasOptions) -> eyre::Result<()> {
    if options.use_bc7 {
        eyre::bail!("BC7 output is disabled for ec_art atlas pages because small art tiles lose sharpness");
    }
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
    crop_transparent_bounds: bool,
) -> eyre::Result<(
    Vec<DecodedArtTile>,
    Vec<SlotAlias>,
    HashMap<u32, EcArtCropAdjustment>,
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
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} resolving art tile sources ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    let mut decode_groups: Vec<PreparedArtDecodeGroup> = Vec::new();

    for art_id in art_ids {
        resolve_pb.inc(1);
        let art_data = art_definition
            .definitions
            .get(&art_id)
            .context("missing tileart definition during art decode")?;

        if art_data.tile_type != TileType::Static {
            continue;
        }
        let resolved = if let Some(texture) = art_data.ec_texture.as_ref() {
            if let Some(world_textures) = world_textures {
                world_textures
                    .get_from_id(texture.texture_id)?
                    .map(|file| (TextureSourceKey::World(texture.texture_id), texture, file))
            } else {
                None
            }
        } else {
            None
        }
        .or(if let Some(texture) = art_data.cc_texture.as_ref() {
            if let Some(legacy_textures) = legacy_textures {
                legacy_textures
                    .get_from_id(texture.texture_id)?
                    .map(|file| (TextureSourceKey::Legacy(texture.texture_id), texture, file))
            } else {
                None
            }
        } else {
            None
        });

        if let Some((source_key, texture_bounds, file)) = resolved {
            if should_skip_terrain_material_static(
                art_data,
                texture_bounds,
                &file,
                terrain_source_texture_ids,
            )? {
                continue;
            }

            // Tileart ownership is authoritative for ec_art packing.
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

            canonical_by_source.insert(canonical_key, decode_groups.len());
            decode_groups.push(PreparedArtDecodeGroup {
                canonical_art_id: art_id as u32,
                texture_bounds: texture_bounds.clone(),
                file,
                alias_art_ids: Vec::new(),
            });
        }
    }
    resolve_pb.finish_with_message("Art tile sources resolved");

    let decode_pb = ProgressBar::new(decode_groups.len() as u64);
    decode_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding unique art tiles ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    let decoded_groups = decode_groups
        .par_iter()
        .map(|group| -> eyre::Result<DecodedArtDecodeGroup> {
            let img = group.file.decode_to_rgba()?;
            let rgba = img.to_rgba8();
            let source_width = rgba.width() as u16;
            let source_height = rgba.height() as u16;
            let clip_rect = normalized_source_clip_rect(
                source_width,
                source_height,
                &group.texture_bounds,
            );
            let (width, height, rgba, crop_adjustment) = if crop_transparent_bounds {
                crop_rgba_tile_to_bounds(
                    source_width,
                    source_height,
                    rgba.into_raw(),
                    clip_rect,
                )?
            } else {
                apply_requested_clip_rect(source_width,
                    source_height,
                    rgba.into_raw(),
                    clip_rect,
                )?
            };
            decode_pb.inc(1);
            Ok(DecodedArtDecodeGroup {
                canonical_art_id: group.canonical_art_id,
                width,
                height,
                rgba,
                crop_adjustment,
                alias_art_ids: group.alias_art_ids.clone(),
            })
        })
        .collect::<Vec<_>>();
    decode_pb.finish_with_message("Unique art tiles decoded");

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
            crop_adjustments.insert(decoded_group.canonical_art_id, decoded_group.crop_adjustment);
            decoded_tiles.push(DecodedArtTile {
                art_id: decoded_group.canonical_art_id,
                kind: ArtTileKind::Static,
                width: decoded_group.width,
                height: decoded_group.height,
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

pub fn requested_source_window(texture: &ArtTexture) -> Option<SourceClipRect> {
    Some(SourceClipRect {
        left: texture.start_x.max(0) as u16,
        top: texture.start_y.max(0) as u16,
        right: texture.end_x.max(0) as u16,
        bottom: texture.end_y.max(0) as u16,
    })
}

fn is_flat_material_sheet_static(art_data: &ArtData) -> bool {
    if art_data.height != 0 {
        return false;
    }

    let Some(texture) = art_data.ec_texture.as_ref() else {
        return false;
    };

    texture.start_x == 0
        && texture.start_y == 0
        && texture.end_x == 0
        && texture.end_y == 0
        && texture.offset_x == 0
        && texture.offset_y == 0
}

fn should_skip_terrain_material_static(
    art_data: &ArtData,
    texture: &ArtTexture,
    file: &TextureFile,
    terrain_source_texture_ids: &HashSet<u32>,
) -> eyre::Result<bool> {
    if is_flat_material_sheet_static(art_data) {
        return Ok(true);
    }

    if !terrain_source_texture_ids.contains(&texture.texture_id) {
        return Ok(false);
    }

    if texture.offset_x != 0 || texture.offset_y != 0 {
        return Ok(false);
    }

    let image = file.decode_to_rgba()?;
    let source_width = image.width() as u16;
    let source_height = image.height() as u16;

    Ok(normalized_source_clip_rect(source_width, source_height, texture).is_none())
}

pub fn register_rendered_tile_alias(
    art_id: u32,
    width: u16,
    height: u16,
    rgba: &[u8],
    canonical_by_rendered_tile: &mut HashMap<Vec<u8>, u32>,
) -> Option<u32> {
    let key = [width.to_le_bytes().to_vec(), height.to_le_bytes().to_vec(), rgba.to_vec()].concat();
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
) -> eyre::Result<(u16, u16, Vec<u8>, EcArtCropAdjustment)> {
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
    let clip_right = clip_rect.map(|clip| clip.right as usize).unwrap_or(width_usize);
    let clip_bottom = clip_rect.map(|clip| clip.bottom as usize).unwrap_or(height_usize);
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
        return Ok((width, height, rgba, EcArtCropAdjustment::default()));
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
        return Ok((width, height, rgba, EcArtCropAdjustment::default()));
    }

    crop_rgba_subrect(width, height, rgba, min_x, min_y, max_x + 1, max_y + 1)
}

pub fn apply_requested_clip_rect(
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    clip_rect: Option<SourceClipRect>,
) -> eyre::Result<(u16, u16, Vec<u8>, EcArtCropAdjustment)> {
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
        Ok((width, height, rgba, EcArtCropAdjustment::default()))
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
) -> eyre::Result<(u16, u16, Vec<u8>, EcArtCropAdjustment)> {
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
        EcArtCropAdjustment {
            left: left as i16,
            top: top as i16,
        },
    ))
}

pub fn build_crop_adjustment_lookup(
    slot_count: u32,
    canonical_adjustments: &HashMap<u32, EcArtCropAdjustment>,
    aliases: &[SlotAlias],
) -> Vec<Option<EcArtCropAdjustment>> {
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

pub fn apply_slot_aliases(slots: &mut [EcArtSlotRecord], aliases: &[SlotAlias]) -> eyre::Result<()> {
    for alias in aliases {
        let canonical = *slots
            .get(alias.canonical_art_id as usize)
            .context("canonical ec_art slot outside slot table")?;
        let slot = slots
            .get_mut(alias.art_id as usize)
            .context("alias ec_art slot outside slot table")?;
        if !canonical.is_present() {
            eyre::bail!(
                "canonical ec_art slot {} missing while applying alias {}",
                alias.canonical_art_id,
                alias.art_id
            );
        }
        *slot = EcArtSlotRecord {
            art_id: alias.art_id,
            ..canonical
        };
    }
    Ok(())
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &EcArtAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<EcArtSlotRecord>)> {
    // Like the CC path, keep a sparse slot table for the full art id range. That
    // makes runtime lookup deterministic even when many ids are absent.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(EcArtSlotRecord::absent)
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
            *slot = EcArtSlotRecord {
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
    options: &EcArtAtlasOptions,
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
    let gutter = i32::from(options.gutter);

    for tile in tiles {
        // A few EC statics legitimately span the full atlas width. Keep the gutter
        // where it fits, but drop it on a saturated axis so those tiles can still be packed.
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

            // Record the furthest texel that carries real image data. The package
            // stores only `used_width x used_height`, while the manifest preserves
            // the original atlas dimensions needed to interpret these coordinates.
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
            record: EcArtPageRecord {
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
    options: &EcArtAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // Manifest records tell the reader how to reinterpret cropped payloads as
    // logical atlas pages. `used_width/used_height` describe the stored bytes,
    // while `atlas_width/atlas_height` keep the public coordinate system stable.
    let pixel_format = if options.use_bc7 {
        PagePixelFormat::Bc7
    } else {
        PagePixelFormat::Rgba8888
    };
    let mut bytes = Vec::with_capacity(25 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(EC_ART_METADATA_VERSION)?;
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
    slots: &[EcArtSlotRecord],
    options: &EcArtAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + slots.len() * 20);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(EC_ART_METADATA_VERSION)?;
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
    slots: &[EcArtSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &EcArtAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            crop_transparent_bounds: false,
            use_bc7: false,
        },
    )
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<EcArtPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid ec_art page manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != EC_ART_METADATA_VERSION {
        eyre::bail!("unsupported ec_art page manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("unknown ec_art page pixel format"))?;
    let page_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        pages.push(EcArtPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            tile_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((atlas_width, atlas_height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<EcArtSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid ec_art slot manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != EC_ART_METADATA_VERSION {
        eyre::bail!("unsupported ec_art slot manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let slot_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        slots.push(EcArtSlotRecord {
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

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}


fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("read {path}"))
}
