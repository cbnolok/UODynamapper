//! Build-time and runtime support for `tex_land_cc.uddp`.
//!
//! Package layout:
//! - `pages/{page_index}.rgba8888` or `pages/{page_index}.bc7`: atlas page payloads.
//! - `metadata/pages.bin`: page table with atlas dimensions, per-page occupancy, and
//!   the pixel format used for each page.
//! - `metadata/slots.bin`: sparse slot table with one record per `texmap_id`.
//!
//! This is modeled after `tex_art_cc.uddp` but for the 128x128/64x64 terrain textures
//! found in `texmaps.mul`.

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
use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::{AtlasPackingMode, extrude_rgba_rect_edges, merge_unplaced_tiles, resolve_packing_axis};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::tex_land_cc::{
    page_entry_path, TexLandCcPageRecord, TexLandCcSlotRecord, MISSING_PAGE_INDEX,
    MISSING_PAGE_TILE_INDEX, PAGE_MANIFEST_ENTRY_PATH, SLOT_FLAG_PRESENT, SLOT_MANIFEST_ENTRY_PATH,
};
use udd_container::xxh64_virtual_path;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::classic::land_texture::TexMap;

use crate::upscale::{UpscaleFilter, UpscaleConfig};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CTXP";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CTXS";
const TEX_LAND_CC_METADATA_VERSION: u32 = 2;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

pub struct TexLandCcAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
    pub upscale_64: UpscaleConfig,
    pub upscale_128: UpscaleConfig,
    pub pixel_format: PagePixelFormat,
    pub packing_mode: AtlasPackingMode,
    pub filtering_ready: bool,
}

impl Default for TexLandCcAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
            compression: CompressionFlag::None,
            upscale_64: UpscaleConfig::default(),
            upscale_128: UpscaleConfig::default(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::MaximumPacking,
            filtering_ready: false,
        }
    }
}

fn packing_mode_repr(mode: AtlasPackingMode) -> u8 {
    match mode {
        AtlasPackingMode::MaximumPacking => 0,
        AtlasPackingMode::Bc7Oriented => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandCcBuildSummary {
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedTexmapTile {
    pub id: u32,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PlacedTile {
    pub id: u32,
    pub page_tile_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct BuiltPage {
    pub record: TexLandCcPageRecord,
    pub pixels: Vec<u8>,
    pub placed_tiles: Vec<PlacedTile>,
}

pub fn convert_texmaps_mul_to_tex_land_cc_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<TexLandCcBuildSummary> {
    convert_texmaps_mul_to_tex_land_cc_uddp_with_patches(
        client_dir,
        out_file,
        options,
        &ClassicPatchOptions::NONE,
    )
}

pub fn convert_texmaps_mul_to_tex_land_cc_uddp_with_patches(
    client_dir: &Path,
    out_file: &Path,
    options: &TexLandCcAtlasOptions,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<TexLandCcBuildSummary> {
    let texmaps_path = find_first_existing_file(&[client_dir.to_path_buf()], &[&"texmaps.mul"])
        .ok_or_else(|| eyre::eyre!("missing texmaps.mul in {}", client_dir.display()))?;
    let texidx_path = find_first_existing_file(&[client_dir.to_path_buf()], &[&"texidx.mul"])
        .ok_or_else(|| eyre::eyre!("missing texidx.mul in {}", client_dir.display()))?;

    info!("Converting CC TexMaps to {}", out_file.display());

    let texmap_source = TexMap::load_with_verdata(
        texmaps_path,
        texidx_path,
        load_verdata_if_enabled(&[client_dir.to_path_buf()], patch_options)?,
    )
        .wrap_err_with(|| format!("load texmaps from {}", client_dir.display()))?;

    let slot_count = texmap_source.len() as u32;
    let decoded_tiles = decode_present_tiles(&texmap_source, options)?;
    let populated_slot_count = decoded_tiles.len() as u32;

    let (pages, slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
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

    let pb = ProgressBar::new(pages.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} encoding atlas pages ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let encoded_pages = if use_bc7 {
        let extent = ImageExtent::new(options.atlas_width, options.atlas_height)
            .map_err(|e| eyre::eyre!("{e}"))?;
        pages
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
                Ok((
                    page_path,
                    encoded,
                    options.atlas_width,
                    options.atlas_height,
                ))
            })
            .collect::<Vec<eyre::Result<(String, Vec<u8>, u32, u32)>>>()
            .into_iter()
            .collect::<eyre::Result<Vec<_>>>()?
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
        package.add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width,
            height,
            virtual_path: Some(&page_path),
            path_hash64: None,
            id: None,
            data: &encoded,
        })?;
    }
    pb.finish_with_message("Atlas pages encoded");

    build_and_write_package(&mut package, out_file)?;

    Ok(TexLandCcBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn decode_present_tiles(
    texmap_source: &TexMap,
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<Vec<DecodedTexmapTile>> {
    let mut decoded_tiles = Vec::new();
    let max_id = texmap_source.len();
    let now = std::time::Instant::now();

    let pb = ProgressBar::new(max_id as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} decoding texmaps ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    for id in 0..max_id {
        if let Some(rgba_arc) = texmap_source.get_pixel_data(id, now) {
            let element = texmap_source.element(id).unwrap();
            let (orig_w, orig_h) = element.size().dimensions();

            let (w, h, rgba) = if orig_w == 64 && orig_h == 64 {
                let target = if options.upscale_64.target_size == 0 { orig_w as u32 * options.upscale_64.filter.scale_factor() } else { options.upscale_64.target_size };
                (target, target, options.upscale_64.filter.apply_to_size(orig_w as u32, orig_h as u32, &rgba_arc, target, target))
            } else if orig_w == 128 && orig_h == 128 {
                let target = if options.upscale_128.target_size == 0 { orig_w as u32 * options.upscale_128.filter.scale_factor() } else { options.upscale_128.target_size };
                (target, target, options.upscale_128.filter.apply_to_size(orig_w as u32, orig_h as u32, &rgba_arc, target, target))
            } else {
                (orig_w as u32, orig_h as u32, rgba_arc.to_vec())
            };
            decoded_tiles.push(DecodedTexmapTile {
                id: id as u32,
                width: w as u16,
                height: h as u16,
                rgba,
            });
        }
        pb.inc(1);
    }
    pb.finish_with_message("Texmaps decoded");

    // Sort by area descending for better packing
    decoded_tiles.sort_by(|a, b| {
        let area_a = a.width as u32 * a.height as u32;
        let area_b = b.width as u32 * b.height as u32;
        area_b.cmp(&area_a).then_with(|| a.id.cmp(&b.id))
    });

    Ok(decoded_tiles)
}

fn pack_tiles_into_pages(
    tiles: Vec<DecodedTexmapTile>,
    slot_count: u32,
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<TexLandCcSlotRecord>)> {
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count)
        .map(TexLandCcSlotRecord::absent)
        .collect::<Vec<_>>();
    let mut remaining = tiles;
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let prefix_len = max_fitting_page_prefix_len(&remaining, options)?;
        if prefix_len == 0 {
            eyre::bail!("could not fit any texmap tile into atlas page");
        }

        let mut page_tiles = remaining.drain(..prefix_len).collect::<Vec<_>>();
        let (page, unplaced) = build_page(page_index, page_tiles, options)?;
        for placed in &page.placed_tiles {
            let slot = &mut slot_records[placed.id as usize];
            *slot = TexLandCcSlotRecord {
                id: placed.id,
                page_index,
                page_tile_index: placed.page_tile_index,
                flags: SLOT_FLAG_PRESENT,
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
            };
        }

        pages.push(page);
        remaining = merge_unplaced_tiles(remaining, unplaced, |tile| tile.id);
        page_index += 1;
    }

    Ok((pages, slot_records))
}

fn max_fitting_page_prefix_len(
    tiles: &[DecodedTexmapTile],
    options: &TexLandCcAtlasOptions,
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

fn page_prefix_fits(
    tiles: &[DecodedTexmapTile],
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<bool> {
    let mut tiles = tiles.to_vec();
    sort_tiles_within_page(&mut tiles, options);
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));

    for tile in &tiles {
        let width_axis = resolve_packing_axis(
            tile.width as u32,
            options.atlas_width,
            options.gutter,
            options.packing_mode,
            false,
        );
        let height_axis = resolve_packing_axis(
            tile.height as u32,
            options.atlas_height,
            options.gutter,
            options.packing_mode,
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!("tile too large for atlas");
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

fn sort_tiles_within_page(tiles: &mut [DecodedTexmapTile], options: &TexLandCcAtlasOptions) {
    if options.packing_mode != AtlasPackingMode::Bc7Oriented {
        return;
    }

    tiles.sort_by(|left, right| {
        let left_area = sort_area(left, options);
        let right_area = sort_area(right, options);
        right_area
            .cmp(&left_area)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_area(tile: &DecodedTexmapTile, options: &TexLandCcAtlasOptions) -> u32 {
    let width_axis = resolve_packing_axis(
        tile.width as u32,
        options.atlas_width,
        options.gutter,
        options.packing_mode,
        false,
    )
    .unwrap_or_else(|| unreachable!("validated before placement"));
    let height_axis = resolve_packing_axis(
        tile.height as u32,
        options.atlas_height,
        options.gutter,
        options.packing_mode,
        false,
    )
    .unwrap_or_else(|| unreachable!("validated before placement"));
    width_axis.alloc_extent * height_axis.alloc_extent
}

fn build_page(
    page_index: u32,
    mut tiles: Vec<DecodedTexmapTile>,
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedTexmapTile>)> {
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
        let width_axis = resolve_packing_axis(
            tile.width as u32,
            options.atlas_width,
            options.gutter,
            options.packing_mode,
            false,
        );
        let height_axis = resolve_packing_axis(
            tile.height as u32,
            options.atlas_height,
            options.gutter,
            options.packing_mode,
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!("tile too large for atlas");
        };

        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let x = allocation.rectangle.min.x as u32 + width_axis.leading_padding;
            let y = allocation.rectangle.min.y as u32 + height_axis.leading_padding;

            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                x,
                y,
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
                    x,
                    y,
                    tile.width as u32,
                    tile.height as u32,
                );
            }

            if options.filtering_ready {
                used_width = used_width.max(allocation.rectangle.min.x as u32 + width_axis.alloc_extent);
                used_height = used_height.max(allocation.rectangle.min.y as u32 + height_axis.alloc_extent);
            } else {
                used_width = used_width.max(x + width_axis.used_extent);
                used_height = used_height.max(y + height_axis.used_extent);
            }

            placed_tiles.push(PlacedTile {
                id: tile.id,
                page_tile_index: placed_tiles.len() as u16,
                x: x as u16,
                y: y as u16,
                width: tile.width,
                height: tile.height,
            });
        } else {
            leftovers.push(tile);
        }
    }

    Ok((
        BuiltPage {
            record: TexLandCcPageRecord {
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
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let dst_stride = dst_width as usize * 4;
    let src_stride = w as usize * 4;
    for row in 0..h as usize {
        let src_start = row * src_stride;
        let dst_start = (y as usize + row) * dst_stride + x as usize * 4;
        dst[dst_start..dst_start + src_stride]
            .copy_from_slice(&src[src_start..src_start + src_stride]);
    }
    Ok(())
}

fn crop_rgba_page(src: &[u8], src_width: u32, crop_width: u32, crop_height: u32) -> Vec<u8> {
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

fn serialize_page_manifest(
    pages: &[BuiltPage],
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let pixel_format = options.pixel_format;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_LAND_CC_METADATA_VERSION)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(id: u32, width: u16, height: u16) -> DecodedTexmapTile {
        DecodedTexmapTile {
            id,
            width,
            height,
            rgba: vec![255; width as usize * height as usize * 4],
        }
    }

    #[test]
    fn bc7_oriented_land_page_is_block_aligned() {
        let options = TexLandCcAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
            compression: CompressionFlag::None,
            upscale_64: UpscaleConfig::default(),
            upscale_128: UpscaleConfig::default(),
            pixel_format: PagePixelFormat::Rgba8888,
            packing_mode: AtlasPackingMode::Bc7Oriented,
            filtering_ready: false,
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
}

fn serialize_slot_manifest(
    slots: &[TexLandCcSlotRecord],
    options: &TexLandCcAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(TEX_LAND_CC_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.push(packing_mode_repr(options.packing_mode));
    bytes.write_u32::<LittleEndian>(slots.len() as u32)?;
    for slot in slots {
        bytes.write_u32::<LittleEndian>(slot.id)?;
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
