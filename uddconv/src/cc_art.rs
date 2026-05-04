//! Build-time and runtime support for `cc_art.uddp`.
//!
//! Package layout:
//! - `pages/{page_index}.rgba8888` or `pages/{page_index}.bc7`: atlas page payloads.
//!   When BC7 compression is enabled each page is block-compressed on the CPU before
//!   being stored in the UDDP container. RGBA pages are stored uncompressed (TODO: compress them with zstd instead).
//! - `metadata/pages.bin`: page table with atlas dimensions, per-page occupancy, and
//!   the pixel format used for each page.
//! - `metadata/slots.bin`: sparse slot table with one record per `art_id`, including
//!   empty slots from `artidx.mul`.
//!
//! This module has two layers:
//! - build-time conversion code that reads classic client art, decodes only the
//!   present tiles, and packs them into atlas pages.
//! - runtime loading code that treats the resulting package as a sparse slot table
//!   plus page table, so the renderer can jump directly from `art_id` to page/rect.
//!
//! The key high-level contract is that the slot table is authoritative for lookup,
//! while the page payloads are just backing storage for the rectangles referenced
//! by those slots. Empty records are kept on purpose so classic `art_id` lookups
//! stay O(1) and preserve the original sparse address space.

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
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_dir_matching;
use uocf::{
    classic::art::ArtMap,
    udd::{
        xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
        LookupMode, UddpBuilder, UddpReader,
    },
};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CASL";
/// Bump version when the binary layout of either manifest changes.
const CC_ART_METADATA_VERSION: u32 = 2;

/// Page pixel format stored in the page manifest and on-disk entry extension.
/// This mirrors `VramTextureFormat` but is kept local so the atlas modules
/// stay independent from the full bc7 encoding type hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PagePixelFormat {
    /// Raw RGBA8888, one byte per channel. File extension: `.rgba8888`.
    Rgba8888 = 0,
    /// BC7 block-compressed, 16 bytes per 4×4 texel block. File extension: `.bc7`.
    Bc7 = 1,
}

impl PagePixelFormat {
    pub(crate) fn from_repr(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rgba8888),
            1 => Some(Self::Bc7),
            _ => None,
        }
    }
    /// File extension used for pages stored in this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Rgba8888 => "rgba8888",
            Self::Bc7 => "bc7",
        }
    }
}
pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcArtAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    /// When `true` each atlas page is BC7-compressed on the CPU before being
    /// stored in the UDDP container, reducing VRAM usage by ~8×. Requires
    /// extra CPU time during the build step.
    pub use_bc7: bool,
}

impl Default for CcArtAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            use_bc7: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcArtBuildSummary {
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
    pub fn slot_flags(self) -> u16 {
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
pub struct CcArtPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    /// Pixel format of the page payload stored in the UDDP container.
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcArtSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl CcArtSlotRecord {
    pub fn absent(art_id: u32) -> Self {
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
    pub record: CcArtPageRecord,
    /// Raw RGBA8888 pixels as produced by the guillotiere packer.
    /// Encoded to the final format (RGBA or BC7) at pack time.
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

pub struct CcArtPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    pages: Vec<CcArtPageRecord>,
    slots: Vec<CcArtSlotRecord>,
}

impl CcArtPackage {
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
        let page_manifest = read_path_entry(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("cc_art.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("cc_art.uddp missing metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, pages) = parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slots) = parse_slot_manifest(&slot_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("cc_art metadata headers disagree on atlas dimensions or gutter");
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

    pub fn pages(&self) -> &[CcArtPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[CcArtSlotRecord] {
        &self.slots
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        read_path_entry(&self.package, &page_entry_path(page_index, fmt))
            .wrap_err_with(|| format!("unpack atlas page {page_index}"))
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&CcArtSlotRecord> {
        self.slots
            .get(art_id as usize)
            .filter(|slot| slot.is_present())
    }
}

pub fn convert_art_mul_to_cc_art_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &CcArtAtlasOptions,
) -> eyre::Result<CcArtBuildSummary> {
    convert_art_mul_to_cc_art_uddp_from_sources(&[client_dir.to_path_buf()], out_file, options)
}

pub fn convert_art_mul_to_cc_art_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &CcArtAtlasOptions,
) -> eyre::Result<CcArtBuildSummary> {
    validate_options(options)?;

    let client_dir = find_first_dir_matching(source_dirs, &[&["artlegacymul.uop"], &["artLegacyMUL.uop"], &["artidx.mul", "art.mul"]])
        .ok_or_else(|| eyre::eyre!(
            "no art sources found in any provided path: expected artLegacyMUL.uop or art.mul/artidx.mul"
        ))?;

    let has_uop = ["artlegacymul.uop", "artLegacyMUL.uop"]
        .iter()
        .any(|name| client_dir.join(name).is_file());

    if has_uop {
        info!("Converting CC Art from UOP format to {}", out_file.display());
    } else {
        info!("Converting CC Art from classic MUL format to {}", out_file.display());
    }
    println!("Using CC art source dir: {}", client_dir.display());

    let art_map = ArtMap::load(&client_dir)
        .wrap_err_with(|| format!("load art sources from {}", client_dir.display()))?;

    let slot_count = art_map.max_id();
    let decoded_tiles = decode_present_tiles(&art_map)?;
    let populated_slot_count = decoded_tiles.len() as u32;

    let (pages, slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
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

    Ok(CcArtBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &CcArtAtlasOptions) -> eyre::Result<()> {
    if options.use_bc7 {
        eyre::bail!("BC7 output is disabled for cc_art atlas pages because small art tiles lose sharpness");
    }
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    Ok(())
}

fn decode_present_tiles(art_map: &ArtMap) -> eyre::Result<Vec<DecodedArtTile>> {
    // Decode every occupied art slot up front so the packer can sort by area and
    // feed the atlas allocator largest-first. Classic clients are messy in practice:
    // some slots are structurally present but malformed, so the converter skips
    // those and reports a compact sample instead of aborting the whole package.
    let mut decoded_tiles = Vec::new();
    let mut skipped_tiles = 0u32;
    let mut skipped_land_tiles = 0u32;
    let mut skipped_static_tiles = 0u32;
    let mut skipped_land_samples = Vec::new();
    let mut skipped_static_samples = Vec::new();

    let max_id = art_map.max_id();
    let art_ids = (0..max_id)
        .filter(|&art_id| art_map.has_id(art_id))
        .collect::<Vec<_>>();
    let pb = ProgressBar::new(art_ids.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding tiles ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    enum DecodeOutcome {
        Decoded(DecodedArtTile),
        SkippedLand(String),
        SkippedStatic(String),
    }

    let decode_outcomes = art_ids
        .into_par_iter()
        .map(|art_id| {
            let kind = if art_id < 0x4000 {
                ArtTileKind::Land
            } else {
                ArtTileKind::Static
            };
            let mut scratch_raw = Vec::new();

            let outcome = match kind {
                ArtTileKind::Land => {
                    let mut rgba = [0u8; 44 * 44 * 4];
                    match art_map.decode_land_tile(art_id, &mut scratch_raw, &mut rgba) {
                        Ok(()) => DecodeOutcome::Decoded(DecodedArtTile {
                            art_id,
                            kind,
                            width: 44,
                            height: 44,
                            rgba: rgba.to_vec(),
                        }),
                        Err(error) => DecodeOutcome::SkippedLand(format!("{art_id} ({error})")),
                    }
                }
                ArtTileKind::Static => {
                    match art_map.decode_static_tile(art_id, &mut scratch_raw) {
                        Ok((width, height, rgba)) => DecodeOutcome::Decoded(DecodedArtTile {
                            art_id,
                            kind,
                            width,
                            height,
                            rgba,
                        }),
                        Err(error) => DecodeOutcome::SkippedStatic(format!("{art_id} ({error})")),
                    }
                }
            };
            pb.inc(1);
            outcome
        })
        .collect::<Vec<_>>();

    for outcome in decode_outcomes {
        match outcome {
            DecodeOutcome::Decoded(tile) => decoded_tiles.push(tile),
            DecodeOutcome::SkippedLand(sample) => {
                skipped_tiles += 1;
                skipped_land_tiles += 1;
                if skipped_land_samples.len() < 8 {
                    skipped_land_samples.push(sample);
                }
            }
            DecodeOutcome::SkippedStatic(sample) => {
                skipped_tiles += 1;
                skipped_static_tiles += 1;
                if skipped_static_samples.len() < 8 {
                    skipped_static_samples.push(sample);
                }
            }
        }
    }
    if skipped_tiles > 0 {
        let land_summary = format_skip_summary(skipped_land_tiles, &skipped_land_samples);
        let static_summary = format_skip_summary(skipped_static_tiles, &skipped_static_samples);
        pb.finish_with_message(format!(
            "Tiles decoded (skipped {skipped_tiles} malformed entries; land: {land_summary}; static: {static_summary})"
        ));
    } else {
        pb.finish_with_message("Tiles decoded");
    }

    decoded_tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });

    Ok(decoded_tiles)
}

fn format_skip_summary(skipped_count: u32, samples: &[String]) -> String {
    if skipped_count == 0 {
        return "0".to_string();
    }

    if skipped_count as usize > samples.len() {
        format!("{skipped_count} [{}; ...]", samples.join(", "))
    } else {
        format!("{skipped_count} [{}]", samples.join(", "))
    }
}

pub fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &CcArtAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<CcArtSlotRecord>)> {
    // Build full sparse metadata up front. Empty slots are kept explicitly so the
    // runtime can answer `art_id -> atlas location` without a side lookup table.
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(CcArtSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    remaining.sort_by_key(|tile| tile.art_id);
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page_tiles, leftovers) = take_page_tile_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_tiles, options)?;
        debug_assert!(unplaced.is_empty(), "selected page tile prefix must fit entirely");
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
            *slot = CcArtSlotRecord {
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

fn take_page_tile_prefix(
    tiles: Vec<DecodedArtTile>,
    options: &CcArtAtlasOptions,
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
    options: &CcArtAtlasOptions,
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

fn page_prefix_fits(tiles: &[DecodedArtTile], options: &CcArtAtlasOptions) -> eyre::Result<bool> {
    let mut to_pack = tiles.to_vec();
    sort_tiles_within_page(&mut to_pack);

    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let gutter = i32::from(options.gutter);

    for tile in &to_pack {
        let alloc_width = tile.width as i32 + gutter * 2;
        let alloc_height = tile.height as i32 + gutter * 2;
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

        if allocator.allocate(size2(alloc_width, alloc_height)).is_none() {
            return Ok(false);
        }
    }

    Ok(true)
}

fn sort_tiles_within_page(tiles: &mut [DecodedArtTile]) {
    tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });
}

fn build_page(
    page_index: u32,
    mut tiles: Vec<DecodedArtTile>,
    options: &CcArtAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedArtTile>)> {
    // Pages are always assembled as full-size RGBA images in memory even when the
    // stored package payload is later cropped or BC7-encoded. That keeps placement,
    // blitting, and runtime atlas coordinates in one consistent page space.
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

    sort_tiles_within_page(&mut tiles);

    for tile in tiles {
        // The allocator reserves the requested gutter as part of the rectangle so
        // neighboring tiles do not bleed into one another when sampled with filtering.
        let alloc_width = tile.width as i32 + gutter * 2;
        let alloc_height = tile.height as i32 + gutter * 2;
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
            let inner_x = allocation.rectangle.min.x + gutter;
            let inner_y = allocation.rectangle.min.y + gutter;
            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                tile.width as u32,
                tile.height as u32,
                &tile.rgba,
            )?;

            // Track the furthest written texel, not the allocator rectangle. The
            // later crop step trims only guaranteed-empty space from the right/bottom
            // edges while preserving the logical tile coordinates recorded in metadata.
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
            record: CcArtPageRecord {
                page_index,
                tile_count: placed_tiles.len() as u32,
                used_width,
                used_height,
                // Pixel format is not yet known here; it will be resolved at
                // pack time once the caller decides the encoding. Use Rgba8888
                // as the placeholder — it is updated in serialize_page_manifest.
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
    // Only the stored payload is cropped. The atlas still behaves logically as a
    // full `atlas_width x atlas_height` page because manifests keep the slot coords
    // in that original space plus the `used_width/used_height` bounds needed to read
    // the compact payload back.
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
    options: &CcArtAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    // The page manifest carries both the logical atlas dimensions and the per-page
    // used rectangle. Readers reconstruct a full page view from those two facts:
    // atlas coordinates stay stable, but package I/O only touches the occupied area.
    let pixel_format = if options.use_bc7 {
        PagePixelFormat::Bc7
    } else {
        PagePixelFormat::Rgba8888
    };
    let mut bytes = Vec::with_capacity(25 + pages.len() * 17);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(CC_ART_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    // 1 byte: page pixel format (0 = RGBA8888, 1 = BC7)
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
    slots: &[CcArtSlotRecord],
    options: &CcArtAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + slots.len() * 20);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(CC_ART_METADATA_VERSION)?;
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
    slots: &[CcArtSlotRecord],
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
) -> eyre::Result<Vec<u8>> {
    serialize_slot_manifest(
        slots,
        &CcArtAtlasOptions {
            atlas_width,
            atlas_height,
            gutter,
            use_bc7: false,
        },
    )
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<CcArtPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid cc_art page manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CC_ART_METADATA_VERSION {
        eyre::bail!("unsupported cc_art page manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    // 1 byte: page pixel format (shared for all pages in the package)
    let pixel_format_byte = cursor.read_u8()?;
    let pixel_format = PagePixelFormat::from_repr(pixel_format_byte)
        .ok_or_else(|| eyre::eyre!("unknown page pixel format byte {pixel_format_byte}"))?;
    let page_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        pages.push(CcArtPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            tile_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((atlas_width, atlas_height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<CcArtSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid cc_art slot manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CC_ART_METADATA_VERSION {
        eyre::bail!("unsupported cc_art slot manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let slot_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        slots.push(CcArtSlotRecord {
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
