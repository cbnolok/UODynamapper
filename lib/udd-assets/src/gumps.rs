//! `gumps_cc.uddp` and `gumps_ec.uddp`: paperdoll gumps on lossless RGBA8888 atlas pages, every other gump in an entry
//! of its own (a single gump).
//!
//! A single gump's payload is `u32 width, u32 height, u8 pixel format` ([`GumpPixelFormat`]), then RGBA8888 rows, or
//! BC1 or BC7 blocks covering the size rounded up to 4. The payload carries no version, so every package carries the
//! atlas manifests, whose version covers both layouts, even with no paperdoll gump in it.
//!
//! Lossy packs are BC1 for `gumps_cc` and BC7 for `gumps_ec` (the user, 2026-11-06), whose gumps in the Classic colour
//! space take BC1 too, so a lossy `gumps_ec` mixes BC1, BC7 and its RGBA8888 paperdoll body gumps. BC7 blocks hold
//! premultiplied colour, the rule for block formats; the reader gives every gump back in straight colour, by its own
//! format byte.
//!
//! A `gumps_ec` packed with `--share-with` leaves out the single gumps that `gumps_cc` draws the same (the user,
//! 2026-11-06) and lists each in the shared Classic gump table, with the [`gump_image_hash`] of the `gumps_cc` image it
//! was checked against. A listed id reads as absent here: the caller draws it from the Classic source.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, Context, ContextCompat};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use udd_container::{DataType, FileKey, UddpReader};
use uot_texture_codecs::bc1::{decode_bc1_to_rgba8888, encode_bc1, expected_bc1_byte_len};
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, expected_bc7_byte_len, ImageExtent, RawImageFormat};
use uot_texture_codecs::premultiplied::unpremultiply_rgba8888;

use crate::common::ensure_manifest_version;

const GUMP_ATLAS_PAGE_MANIFEST_ID: u32 = 0xE000_0000;
const GUMP_ATLAS_SLOT_MANIFEST_ID: u32 = 0xE000_0001;
const GUMP_ATLAS_PAGE_ID_BASE: u32 = 0xF000_0000;
const GUMP_ATLAS_PAGE_MANIFEST_MAGIC: [u8; 4] = *b"GAPG";
const GUMP_ATLAS_SLOT_MANIFEST_MAGIC: [u8; 4] = *b"GASL";
/// 3: single gumps carry a pixel format byte, and every package carries the atlas manifests.
pub const GUMP_ATLAS_METADATA_VERSION: u32 = 3;
/// `u32 width, u32 height, u8 pixel format` before a single gump's pixels.
pub const SINGLE_GUMP_HEADER_BYTES: usize = 2 * size_of::<u32>() + size_of::<u8>();

/// The shared Classic gump table: the single gumps a `gumps_ec` leaves out because `gumps_cc` draws them the same. Only a
/// `gumps_ec` packed with `--share-with` carries it.
pub const SHARED_CLASSIC_GUMP_TABLE_ID: u32 = 0xE000_0002;
const SHARED_CLASSIC_GUMP_TABLE_MAGIC: [u8; 4] = *b"GSHC";
/// 1: `u32 count`, then per gump `u32 gump id, u64 image hash`, sorted by id with no repeats.
pub const SHARED_CLASSIC_GUMP_TABLE_VERSION: u32 = 1;
/// `magic, u32 version, u32 count` before the records.
const SHARED_CLASSIC_GUMP_TABLE_HEADER_BYTES: usize = SHARED_CLASSIC_GUMP_TABLE_MAGIC.len() + 2 * size_of::<u32>();
/// `u32 gump id, u64 image hash`.
const SHARED_CLASSIC_GUMP_RECORD_BYTES: usize = size_of::<u32>() + size_of::<u64>();
/// The xxh64 seed of [`gump_image_hash`].
pub const GUMP_IMAGE_HASH_SEED: u64 = 0;

/// The hash a `gumps_ec` records for a gump it shares with `gumps_cc`: xxh64, seed [`GUMP_IMAGE_HASH_SEED`], over
/// `u32 physical width, u32 physical height, u16 upscale factor` (little endian) and then the exact straight RGBA8888
/// bytes, colour under clear texels included.
pub fn gump_image_hash(image: &GumpImage) -> u64 {
    let mut hasher = xxhash_rust::xxh64::Xxh64::new(GUMP_IMAGE_HASH_SEED);
    hasher.update(&image.physical_width.to_le_bytes());
    hasher.update(&image.physical_height.to_le_bytes());
    hasher.update(&image.upscale_factor.to_le_bytes());
    hasher.update(&image.rgba);
    hasher.digest()
}

/// A gump a `gumps_ec` leaves out because `gumps_cc` draws it the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedClassicGump {
    pub gump_id: u32,
    /// The [`gump_image_hash`] of the `gumps_cc` image the packer checked it against.
    pub image_hash: u64,
}

/// The shared Classic gump table's payload; `gumps` must be sorted by id, with no repeats.
pub fn encode_shared_classic_gump_table(gumps: &[SharedClassicGump]) -> eyre::Result<Vec<u8>> {
    ensure_shared_gump_ids_ascend(gumps)?;
    let count = u32::try_from(gumps.len()).context("shared Classic gump table: too many gumps")?;
    let mut bytes =
        Vec::with_capacity(SHARED_CLASSIC_GUMP_TABLE_HEADER_BYTES + gumps.len() * SHARED_CLASSIC_GUMP_RECORD_BYTES);
    bytes.extend_from_slice(&SHARED_CLASSIC_GUMP_TABLE_MAGIC);
    bytes.write_u32::<LittleEndian>(SHARED_CLASSIC_GUMP_TABLE_VERSION)?;
    bytes.write_u32::<LittleEndian>(count)?;
    for gump in gumps {
        bytes.write_u32::<LittleEndian>(gump.gump_id)?;
        bytes.write_u64::<LittleEndian>(gump.image_hash)?;
    }
    Ok(bytes)
}

fn ensure_shared_gump_ids_ascend(gumps: &[SharedClassicGump]) -> eyre::Result<()> {
    if let Some(pair) = gumps.windows(2).find(|pair| pair[0].gump_id >= pair[1].gump_id) {
        eyre::bail!(
            "shared Classic gump table: gump {} follows gump {}; the ids must ascend with no repeats",
            pair[1].gump_id,
            pair[0].gump_id
        );
    }
    Ok(())
}

fn parse_shared_classic_gump_table(bytes: &[u8]) -> eyre::Result<Vec<SharedClassicGump>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).context("shared Classic gump table: shorter than its header")?;
    if magic != SHARED_CLASSIC_GUMP_TABLE_MAGIC {
        eyre::bail!("invalid shared Classic gump table magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("shared Classic gump table", version, SHARED_CLASSIC_GUMP_TABLE_VERSION)?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let expected_len = count
        .checked_mul(SHARED_CLASSIC_GUMP_RECORD_BYTES)
        .and_then(|records| records.checked_add(SHARED_CLASSIC_GUMP_TABLE_HEADER_BYTES));
    eyre::ensure!(
        expected_len == Some(bytes.len()),
        "shared Classic gump table: the entry's {} bytes don't match the {count} gumps it declares",
        bytes.len()
    );
    let mut gumps = Vec::with_capacity(count);
    for _ in 0..count {
        gumps.push(SharedClassicGump {
            gump_id: cursor.read_u32::<LittleEndian>()?,
            image_hash: cursor.read_u64::<LittleEndian>()?,
        });
    }
    ensure_shared_gump_ids_ascend(&gumps)?;
    Ok(gumps)
}

/// How a single gump's pixels are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GumpPixelFormat {
    /// Lossless RGBA8888 rows.
    Rgba8888 = 0,
    /// BC1 blocks with one bit of alpha: Classic's colour 0 and any texel under half alpha decode to transparent black.
    /// The lossy Classic gumps, and the lossy Enhanced gumps in the Classic colour space.
    Bc1 = 1,
    /// BC7 blocks of premultiplied colour, soft alpha kept: the lossy Enhanced gumps.
    Bc7 = 2,
}

impl GumpPixelFormat {
    pub const fn from_repr(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Rgba8888),
            1 => Some(Self::Bc1),
            2 => Some(Self::Bc7),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Rgba8888 => "RGBA8888",
            Self::Bc1 => "BC1",
            Self::Bc7 => "BC7",
        }
    }

    /// The bytes of a `width` x `height` gump's pixels in this format: none for an empty gump.
    pub fn pixel_byte_len(self, width: u32, height: u32) -> usize {
        match (self, gump_extent(width, height)) {
            (_, None) => 0,
            (Self::Rgba8888, Some(extent)) => extent.byte_len(RawImageFormat::Rgba8888),
            (Self::Bc1, Some(extent)) => expected_bc1_byte_len(extent),
            (Self::Bc7, Some(extent)) => expected_bc7_byte_len(extent),
        }
    }
}

/// A gump's extent, or `None` for an empty one: the clients hold a few 0x0 gumps, which keep a payload with no pixels.
fn gump_extent(width: u32, height: u32) -> Option<ImageExtent> {
    ImageExtent::new(width, height).ok()
}

/// A single gump's payload from its RGBA8888 pixels (straight colour), in `format`.
pub fn encode_single_gump_payload(
    width: u32,
    height: u32,
    rgba: &[u8],
    format: GumpPixelFormat,
) -> eyre::Result<Vec<u8>> {
    let expected_rgba_len = GumpPixelFormat::Rgba8888.pixel_byte_len(width, height);
    eyre::ensure!(
        rgba.len() == expected_rgba_len,
        "single gump {width}x{height}: expected {expected_rgba_len} RGBA bytes, got {}",
        rgba.len()
    );
    let mut payload = Vec::with_capacity(SINGLE_GUMP_HEADER_BYTES + format.pixel_byte_len(width, height));
    payload.write_u32::<LittleEndian>(width)?;
    payload.write_u32::<LittleEndian>(height)?;
    payload.write_u8(format as u8)?;
    match (format, gump_extent(width, height)) {
        (_, None) => {}
        (GumpPixelFormat::Rgba8888, Some(_)) => payload.extend_from_slice(rgba),
        (GumpPixelFormat::Bc1, Some(extent)) => payload.extend_from_slice(
            &encode_bc1(rgba, extent).map_err(|error| eyre::eyre!("single gump {width}x{height}: {error}"))?,
        ),
        (GumpPixelFormat::Bc7, Some(extent)) => payload.extend_from_slice(
            &encode_premultiplied_bc7(rgba, extent)
                .map_err(|error| error.wrap_err(format!("single gump {width}x{height}")))?,
        ),
    }
    Ok(payload)
}

/// BC7 blocks of `rgba` premultiplied, through the analytical encoder with the alpha plane search ("fix C") and no RDO,
/// as the lossy measurements of 2026-11-06.
#[cfg(any(feature = "bc7-encode", test))]
fn encode_premultiplied_bc7(rgba: &[u8], extent: ImageExtent) -> eyre::Result<Vec<u8>> {
    use uot_texture_codecs::bc7::{encode_to_bc7, preferred_bc7_encoder_backend};

    let mut premultiplied = rgba.to_vec();
    uot_texture_codecs::premultiplied::premultiply_rgba8888(&mut premultiplied);
    let blocks = encode_to_bc7(&premultiplied, extent, RawImageFormat::Rgba8888, preferred_bc7_encoder_backend())
        .map_err(|error| eyre::eyre!("BC7: {error}"))?;
    Ok(blocks.into_blocks())
}

#[cfg(not(any(feature = "bc7-encode", test)))]
fn encode_premultiplied_bc7(_rgba: &[u8], _extent: ImageExtent) -> eyre::Result<Vec<u8>> {
    eyre::bail!("BC7 gumps need udd-assets built with the bc7-encode feature")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpAtlasPageRecord {
    pub page_index: u32,
    pub gump_count: u32,
    pub used_width: u32,
    pub used_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpAtlasSlotRecord {
    pub gump_id: u32,
    pub page_index: u32,
    pub page_gump_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub upscale_factor: u16,
}

/// A decoded gump: `rgba` is straight (not premultiplied) RGBA8888 whatever the payload's format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GumpImage {
    pub physical_width: u32,
    pub physical_height: u32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub upscale_factor: u16,
    pub rgba: Vec<u8>,
}

pub struct GumpsPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    atlas_gutter: u16,
    atlas_pages: Vec<GumpAtlasPageRecord>,
    atlas_slots: Vec<GumpAtlasSlotRecord>,
    atlas_page_cache: Mutex<HashMap<u32, Arc<[u8]>>>,
    /// The gumps left out for `gumps_cc` to draw, sorted by id; empty unless packed with `--share-with`.
    shared_classic_gumps: Vec<SharedClassicGump>,
}

impl GumpsPackage {
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
        let (atlas_width, atlas_height, atlas_gutter, atlas_pages, atlas_slots) =
            match (
                package.find_by_sparse_id(GUMP_ATLAS_PAGE_MANIFEST_ID),
                package.find_by_sparse_id(GUMP_ATLAS_SLOT_MANIFEST_ID),
            ) {
                (Some(_), Some(_)) => {
                    let page_manifest = package
                        .read_file_by_sparse_id_cow(GUMP_ATLAS_PAGE_MANIFEST_ID)
                        .context("gumps package page manifest could not be read")?;
                    let slot_manifest = package
                        .read_file_by_sparse_id_cow(GUMP_ATLAS_SLOT_MANIFEST_ID)
                        .context("gumps package slot manifest could not be read")?;
                    let (page_width, page_height, page_gutter, pages) =
                        parse_page_manifest(&page_manifest)?;
                    let (slot_width, slot_height, slot_gutter, slots) =
                        parse_slot_manifest(&slot_manifest)?;
                    if (page_width, page_height, page_gutter)
                        != (slot_width, slot_height, slot_gutter)
                    {
                        eyre::bail!(
                            "gump atlas metadata headers disagree on dimensions or gutter"
                        );
                    }
                    (page_width, page_height, page_gutter, pages, slots)
                }
                _ => eyre::bail!(
                    "gump atlas manifest: missing, so the package predates version {GUMP_ATLAS_METADATA_VERSION}; \
                     repack with the current udd-cli"
                ),
            };

        let shared_classic_gumps = match package.find_by_sparse_id(SHARED_CLASSIC_GUMP_TABLE_ID) {
            Some(_) => parse_shared_classic_gump_table(
                &package
                    .read_file_by_sparse_id_cow(SHARED_CLASSIC_GUMP_TABLE_ID)
                    .context("gumps package shared Classic gump table could not be read")?,
            )?,
            None => Vec::new(),
        };

        let gumps = Self {
            package,
            atlas_width,
            atlas_height,
            atlas_gutter,
            atlas_pages,
            atlas_slots,
            atlas_page_cache: Mutex::new(HashMap::new()),
            shared_classic_gumps,
        };
        gumps.ensure_shared_gumps_are_left_out()?;
        Ok(gumps)
    }

    /// Refuses a package that both stores a gump and lists it as shared with `gumps_cc`: the two would disagree on which
    /// image the Enhanced presets draw.
    fn ensure_shared_gumps_are_left_out(&self) -> eyre::Result<()> {
        let stored = self.shared_classic_gumps.iter().find(|shared| {
            self.package.find_by_sparse_id(shared.gump_id).is_some() || self.atlas_slot(shared.gump_id).is_some()
        });
        if let Some(shared) = stored {
            eyre::bail!(
                "gumps package stores gump {} and also lists it as shared with gumps_cc, so it is inconsistent; \
                 repack with the current udd-cli",
                shared.gump_id
            );
        }
        Ok(())
    }

    /// The [`gump_image_hash`] of the `gumps_cc` image a gump left out for `gumps_cc` was packed against, or `None` when
    /// the package doesn't share `gump_id`.
    pub fn shared_classic_gump_hash(&self, gump_id: u32) -> Option<u64> {
        self.shared_classic_gumps
            .binary_search_by_key(&gump_id, |shared| shared.gump_id)
            .ok()
            .map(|index| self.shared_classic_gumps[index].image_hash)
    }

    /// How many gumps the package leaves out for `gumps_cc` to draw.
    pub fn shared_classic_gump_count(&self) -> usize {
        self.shared_classic_gumps.len()
    }

    /// The gumps left out for `gumps_cc` to draw, by ascending id.
    pub fn shared_classic_gumps(&self) -> impl ExactSizeIterator<Item = SharedClassicGump> + '_ {
        self.shared_classic_gumps.iter().copied()
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

    pub fn atlas_gutter(&self) -> u16 {
        self.atlas_gutter
    }

    pub fn atlas_pages(&self) -> &[GumpAtlasPageRecord] {
        &self.atlas_pages
    }

    pub fn atlas_slots(&self) -> &[GumpAtlasSlotRecord] {
        &self.atlas_slots
    }

    pub fn available_gump_ids(&self) -> Vec<u32> {
        let mut ids = self
            .package
            .records()
            .into_iter()
            .filter_map(|record| match record.key {
                FileKey::Id(id) => self
                    .package
                    .find_by_sparse_id(id)
                    .filter(|file| file.data_type == DataType::Gump as u8)
                    .map(|_| id),
                FileKey::PathHash(_) => None,
            })
            .collect::<Vec<_>>();

        ids.extend(self.atlas_slots.iter().map(|slot| slot.gump_id));
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn atlas_slot(&self, gump_id: u32) -> Option<&GumpAtlasSlotRecord> {
        self.atlas_slots
            .binary_search_by_key(&gump_id, |slot| slot.gump_id)
            .ok()
            .map(|index| &self.atlas_slots[index])
    }

    #[cfg(test)]
    fn cached_atlas_page_count(&self) -> usize {
        self.atlas_page_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or(0)
    }

    /// The pixel format of a single gump's payload, or `None` for a gump on an atlas page (always RGBA8888) or none.
    pub fn single_gump_pixel_format(&self, gump_id: u32) -> eyre::Result<Option<GumpPixelFormat>> {
        let Some(file) = self.package.find_by_sparse_id(gump_id) else {
            return Ok(None);
        };
        if file.data_type != DataType::Gump as u8 {
            return Ok(None);
        }
        let payload = self.package.read_file_by_sparse_id_cow(gump_id)?;
        let format_byte = *payload
            .get(SINGLE_GUMP_HEADER_BYTES - size_of::<u8>())
            .context("single gump payload is shorter than its header")?;
        Ok(Some(single_gump_pixel_format(format_byte)?))
    }

    /// A single gump's pixel format and image, or `None` for a gump on an atlas page or none.
    pub fn read_single_gump_image(&self, gump_id: u32) -> eyre::Result<Option<(GumpPixelFormat, GumpImage)>> {
        match self.package.find_by_sparse_id(gump_id) {
            Some(file) if file.data_type == DataType::Gump as u8 => {
                let payload = self.package.read_file_by_sparse_id_cow(gump_id)?;
                let format_byte = *payload
                    .get(SINGLE_GUMP_HEADER_BYTES - size_of::<u8>())
                    .context("single gump payload is shorter than its header")?;
                Ok(Some((single_gump_pixel_format(format_byte)?, decode_single_gump_payload(&payload)?)))
            }
            _ => Ok(None),
        }
    }

    pub fn read_gump_rgba(&self, gump_id: u32) -> eyre::Result<(u32, u32, Vec<u8>)> {
        let image = self.read_gump_image(gump_id)?;
        Ok((image.physical_width, image.physical_height, image.rgba))
    }

    pub fn read_gump_image(&self, gump_id: u32) -> eyre::Result<GumpImage> {
        if let Some(file) = self.package.find_by_sparse_id(gump_id) {
            if file.data_type == DataType::Gump as u8 {
                return decode_single_gump_payload(&self.package.read_file_by_sparse_id(gump_id)?);
            }
        }

        let slot = self
            .atlas_slot(gump_id)
            .ok_or_else(|| eyre::eyre!("gump {gump_id} not found in package"))?;
        self.read_atlas_gump(slot)
    }

    fn read_atlas_gump(&self, slot: &GumpAtlasSlotRecord) -> eyre::Result<GumpImage> {
        let page = self
            .atlas_pages
            .get(slot.page_index as usize)
            .filter(|page| page.page_index == slot.page_index)
            .context("gump atlas slot references missing page metadata")?;
        let page_bytes = self.read_atlas_page_bytes(slot.page_index)?;
        let page_stride = page.used_width as usize * 4;
        let slot_width = slot.width as usize;
        let slot_height = slot.height as usize;
        let mut rgba = vec![0u8; slot_width * slot_height * 4];

        if u32::from(slot.x) + u32::from(slot.width) > page.used_width
            || u32::from(slot.y) + u32::from(slot.height) > page.used_height
        {
            eyre::bail!("gump atlas slot rectangle exceeds page used bounds");
        }

        for row in 0..slot_height {
            let src_start = ((slot.y as usize + row) * page_stride) + slot.x as usize * 4;
            let src_end = src_start + slot_width * 4;
            let dst_start = row * slot_width * 4;
            let src = page_bytes
                .get(src_start..src_end)
                .context("gump atlas page payload is shorter than metadata bounds")?;
            rgba[dst_start..dst_start + slot_width * 4].copy_from_slice(src);
        }

        let upscale_factor = slot.upscale_factor.max(1);
        Ok(GumpImage {
            physical_width: u32::from(slot.width),
            physical_height: u32::from(slot.height),
            logical_width: (u32::from(slot.width) / u32::from(upscale_factor)).max(1),
            logical_height: (u32::from(slot.height) / u32::from(upscale_factor)).max(1),
            upscale_factor,
            rgba,
        })
    }

    fn read_atlas_page_bytes(&self, page_index: u32) -> eyre::Result<Arc<[u8]>> {
        if let Some(bytes) = self
            .atlas_page_cache
            .lock()
            .map_err(|_| eyre::eyre!("gump atlas page cache lock poisoned"))?
            .get(&page_index)
            .cloned()
        {
            return Ok(bytes);
        }

        let bytes = self
            .package
            .read_file_by_sparse_id(GUMP_ATLAS_PAGE_ID_BASE + page_index)
            .wrap_err_with(|| format!("read gump atlas page {page_index}"))?;
        let bytes = Arc::<[u8]>::from(bytes);
        let mut cache = self
            .atlas_page_cache
            .lock()
            .map_err(|_| eyre::eyre!("gump atlas page cache lock poisoned"))?;
        Ok(cache.entry(page_index).or_insert_with(|| bytes.clone()).clone())
    }
}

fn single_gump_pixel_format(format_byte: u8) -> eyre::Result<GumpPixelFormat> {
    GumpPixelFormat::from_repr(format_byte)
        .ok_or_else(|| eyre::eyre!("invalid single gump pixel format {format_byte}"))
}

/// A single gump's payload ([`encode_single_gump_payload`]) decoded to straight RGBA8888.
pub fn decode_single_gump_payload(payload: &[u8]) -> eyre::Result<GumpImage> {
    let mut cursor = Cursor::new(payload);
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let format = single_gump_pixel_format(cursor.read_u8()?)?;
    let expected_len = format.pixel_byte_len(width, height);
    let pixels = &payload[SINGLE_GUMP_HEADER_BYTES..];
    if pixels.len() != expected_len {
        eyre::bail!(
            "invalid gump {} payload length for {width}x{height}: expected {expected_len}, got {}",
            format.label(),
            pixels.len()
        );
    }
    let rgba = match (format, gump_extent(width, height)) {
        (_, None) => Vec::new(),
        (GumpPixelFormat::Rgba8888, Some(_)) => pixels.to_vec(),
        (GumpPixelFormat::Bc1, Some(extent)) => {
            decode_bc1_to_rgba8888(pixels, extent).map_err(|error| eyre::eyre!("single gump {width}x{height}: {error}"))?
        }
        (GumpPixelFormat::Bc7, Some(extent)) => {
            let mut rgba = decode_bc7_to_rgba8888(pixels, extent)
                .map_err(|error| eyre::eyre!("single gump {width}x{height}: {error}"))?;
            unpremultiply_rgba8888(&mut rgba);
            rgba
        }
    };
    Ok(GumpImage {
        physical_width: width,
        physical_height: height,
        logical_width: width,
        logical_height: height,
        upscale_factor: 1,
        rgba,
    })
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<GumpAtlasPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != GUMP_ATLAS_PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid gump atlas page manifest magic");
    }
    let (width, height, gutter, count) = parse_common_manifest_header(&mut cursor)?;
    let mut pages = Vec::with_capacity(count as usize);
    for _ in 0..count {
        pages.push(GumpAtlasPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            gump_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok((width, height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<GumpAtlasSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != GUMP_ATLAS_SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid gump atlas slot manifest magic");
    }
    let (width, height, gutter, count) = parse_common_manifest_header(&mut cursor)?;
    let mut slots = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let slot = GumpAtlasSlotRecord {
            gump_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_gump_index: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            upscale_factor: cursor.read_u16::<LittleEndian>()?.max(1),
        };
        slots.push(slot);
    }
    slots.sort_by_key(|slot| slot.gump_id);
    Ok((width, height, gutter, slots))
}

fn parse_common_manifest_header<R: Read>(reader: &mut R) -> eyre::Result<(u32, u32, u16, u32)> {
    let version = reader.read_u32::<LittleEndian>()?;
    ensure_manifest_version("gump atlas manifest", version, GUMP_ATLAS_METADATA_VERSION)?;
    let width = reader.read_u32::<LittleEndian>()?;
    let height = reader.read_u32::<LittleEndian>()?;
    let gutter = reader.read_u32::<LittleEndian>()? as u16;
    let count = reader.read_u32::<LittleEndian>()?;
    Ok((width, height, gutter, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use udd_container::{AddFileRequest, CompressionFlag, LookupMode, UddpBuilder};

    #[test]
    fn package_reads_single_gump_payload() {
        let package = GumpsPackage::from_uddp_package(build_test_package(false)).unwrap();

        let (width, height, rgba) = package.read_gump_rgba(42).unwrap();

        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(package.single_gump_pixel_format(42).unwrap(), Some(GumpPixelFormat::Rgba8888));
    }

    /// A BC1 single gump keeps its true size over whole blocks, and its transparent texels decode to transparent black.
    #[test]
    fn package_reads_bc1_single_gump_payload() {
        const GUMP_ID: u32 = 7;
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const PAINTED: [u8; 4] = [255, 255, 255, 255];
        const CLEAR: [u8; 4] = [0, 0, 0, 0];
        let mut rgba = PAINTED.repeat((WIDTH * HEIGHT) as usize);
        rgba[..CLEAR.len()].copy_from_slice(&CLEAR);
        let payload = encode_single_gump_payload(WIDTH, HEIGHT, &rgba, GumpPixelFormat::Bc1).unwrap();
        assert_eq!(
            payload.len(),
            SINGLE_GUMP_HEADER_BYTES + expected_bc1_byte_len(ImageExtent::new(WIDTH, HEIGHT).unwrap())
        );
        let empty = encode_single_gump_payload(0, 0, &[], GumpPixelFormat::Bc1).unwrap();
        assert_eq!(empty.len(), SINGLE_GUMP_HEADER_BYTES);
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&[(GUMP_ID, payload)])).unwrap();

        let image = package.read_gump_image(GUMP_ID).unwrap();

        assert_eq!((image.physical_width, image.physical_height), (WIDTH, HEIGHT));
        assert_eq!(image.rgba, rgba);
        assert_eq!(package.single_gump_pixel_format(GUMP_ID).unwrap(), Some(GumpPixelFormat::Bc1));
    }

    /// A BC7 single gump keeps its true size (odd here) over whole blocks and stores premultiplied colour; it reads
    /// back as those blocks in straight colour, with its soft alpha and transparent texels kept.
    #[test]
    fn package_reads_bc7_single_gump_payload() {
        const GUMP_ID: u32 = 9;
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const SOFT_GREY: [u8; 4] = [180, 180, 180, 128];
        const CLEAR: [u8; 4] = [0, 0, 0, 0];
        const PREMULTIPLIED_SOFT_GREY: [u8; 4] = [90, 90, 90, 128];
        /// What BC7 may move a channel of this flat gump by.
        const BC7_TOLERANCE: u8 = 2;
        let mut rgba = SOFT_GREY.repeat((WIDTH * HEIGHT) as usize);
        let clear_texel = rgba.len() - CLEAR.len();
        rgba[clear_texel..].copy_from_slice(&CLEAR);
        let extent = ImageExtent::new(WIDTH, HEIGHT).unwrap();
        let payload = encode_single_gump_payload(WIDTH, HEIGHT, &rgba, GumpPixelFormat::Bc7).unwrap();
        assert_eq!(payload.len(), SINGLE_GUMP_HEADER_BYTES + expected_bc7_byte_len(extent));
        let stored = decode_bc7_to_rgba8888(&payload[SINGLE_GUMP_HEADER_BYTES..], extent).unwrap();
        for (channel, premultiplied) in stored.iter().zip(PREMULTIPLIED_SOFT_GREY) {
            assert!(channel.abs_diff(premultiplied) <= BC7_TOLERANCE, "stored {channel}, expected {premultiplied}");
        }
        let empty = encode_single_gump_payload(0, 0, &[], GumpPixelFormat::Bc7).unwrap();
        assert_eq!(empty.len(), SINGLE_GUMP_HEADER_BYTES);
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&[(GUMP_ID, payload)])).unwrap();

        let image = package.read_gump_image(GUMP_ID).unwrap();

        assert_eq!((image.physical_width, image.physical_height), (WIDTH, HEIGHT));
        let mut straight = stored;
        unpremultiply_rgba8888(&mut straight);
        assert_eq!(image.rgba, straight);
        let is_grey = |texel: &[u8]| texel[0] == texel[1] && texel[1] == texel[2];
        assert!(image.rgba.chunks_exact(CLEAR.len()).all(is_grey), "greys stay greys");
        assert_eq!(image.rgba[clear_texel..], CLEAR);
        assert_eq!(package.single_gump_pixel_format(GUMP_ID).unwrap(), Some(GumpPixelFormat::Bc7));
    }

    /// A BC7 payload whose blocks don't cover its size is an error, not a crop.
    #[test]
    fn short_bc7_single_gump_payload_is_an_error() {
        const GUMP_ID: u32 = 9;
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const OPAQUE: [u8; 4] = [10, 20, 30, 255];
        let mut payload =
            encode_single_gump_payload(WIDTH, HEIGHT, &OPAQUE.repeat((WIDTH * HEIGHT) as usize), GumpPixelFormat::Bc7)
                .unwrap();
        payload.truncate(payload.len() - 1);
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&[(GUMP_ID, payload)])).unwrap();

        let error = package.read_gump_image(GUMP_ID).unwrap_err();
        assert!(error.to_string().contains("BC7 payload length"), "{error}");
    }

    /// A lossy `gumps_ec` mixes formats: each single gump decodes by its own format byte. BC1 reads back straight, its
    /// clear texels transparent black whatever colour they held; only BC7 is un-premultiplied, so an RGBA8888 gump's soft
    /// texel comes back exactly as written.
    #[test]
    fn mixed_format_package_decodes_each_gump_by_its_format() {
        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const RGBA_GUMP_ID: u32 = 12;
        const BC1_GUMP_ID: u32 = 1000;
        const BC7_GUMP_ID: u32 = 1001;
        const CLASSIC_OPAQUE: [u8; 4] = [248, 128, 8, 255];
        const COLOURED_CLEAR: [u8; 4] = [255, 7, 93, 0];
        const SOFT_GREY: [u8; 4] = [180, 180, 180, 128];
        const CLEAR: [u8; 4] = [0, 0, 0, 0];
        const OPAQUE: u8 = u8::MAX;
        const ALPHA_CHANNEL: usize = 3;
        let extent = ImageExtent::new(WIDTH, HEIGHT).unwrap();
        let mut classic = CLASSIC_OPAQUE.repeat((WIDTH * HEIGHT) as usize);
        classic[..COLOURED_CLEAR.len()].copy_from_slice(&COLOURED_CLEAR);
        let mut soft = CLASSIC_OPAQUE.repeat((WIDTH * HEIGHT) as usize);
        soft[..SOFT_GREY.len()].copy_from_slice(&SOFT_GREY);
        let gumps = [
            (RGBA_GUMP_ID, GumpPixelFormat::Rgba8888, &soft),
            (BC1_GUMP_ID, GumpPixelFormat::Bc1, &classic),
            (BC7_GUMP_ID, GumpPixelFormat::Bc7, &soft),
        ];
        let singles = gumps
            .iter()
            .map(|&(gump_id, format, rgba)| (gump_id, encode_single_gump_payload(WIDTH, HEIGHT, rgba, format).unwrap()))
            .collect::<Vec<_>>();
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&singles)).unwrap();

        for (&(gump_id, format, _), (_, payload)) in gumps.iter().zip(&singles) {
            assert_eq!(package.single_gump_pixel_format(gump_id).unwrap(), Some(format), "gump {gump_id}");
            let blocks = &payload[SINGLE_GUMP_HEADER_BYTES..];
            let expected = match format {
                GumpPixelFormat::Rgba8888 => soft.clone(),
                GumpPixelFormat::Bc1 => decode_bc1_to_rgba8888(blocks, extent).unwrap(),
                GumpPixelFormat::Bc7 => {
                    let mut straight = decode_bc7_to_rgba8888(blocks, extent).unwrap();
                    unpremultiply_rgba8888(&mut straight);
                    straight
                }
            };
            assert_eq!(package.read_gump_image(gump_id).unwrap().rgba, expected, "gump {gump_id}");
        }

        let bc1 = package.read_gump_image(BC1_GUMP_ID).unwrap().rgba;
        assert_eq!(bc1[..CLEAR.len()], CLEAR);
        assert!(bc1[CLEAR.len()..].chunks_exact(CLEAR.len()).all(|texel| texel[ALPHA_CHANNEL] == OPAQUE));
        let (_, bc7_payload) = singles.iter().find(|(gump_id, _)| *gump_id == BC7_GUMP_ID).unwrap();
        let bc7_stored = decode_bc7_to_rgba8888(&bc7_payload[SINGLE_GUMP_HEADER_BYTES..], extent).unwrap();
        let bc7 = package.read_gump_image(BC7_GUMP_ID).unwrap().rgba;
        assert_ne!(bc7[..SOFT_GREY.len()], bc7_stored[..SOFT_GREY.len()], "BC7's soft texel is un-premultiplied");
    }

    /// The clients hold a few 0x0 gumps: they keep a payload with no pixels, in any format.
    #[test]
    fn empty_single_gumps_read_back_empty() {
        const GUMP_IDS: [u32; 3] = [128, 129, 130];
        let singles = [GumpPixelFormat::Rgba8888, GumpPixelFormat::Bc1, GumpPixelFormat::Bc7]
            .into_iter()
            .zip(GUMP_IDS)
            .map(|(format, gump_id)| (gump_id, encode_single_gump_payload(0, 0, &[], format).unwrap()))
            .collect::<Vec<_>>();
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&singles)).unwrap();

        for gump_id in GUMP_IDS {
            assert_eq!(package.read_gump_rgba(gump_id).unwrap(), (0, 0, Vec::new()));
        }
    }

    #[test]
    fn unknown_single_gump_pixel_format_is_an_error() {
        const GUMP_ID: u32 = 7;
        const UNKNOWN_PIXEL_FORMAT: u8 = u8::MAX;
        let mut payload = single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        payload[SINGLE_GUMP_HEADER_BYTES - size_of::<u8>()] = UNKNOWN_PIXEL_FORMAT;
        let package = GumpsPackage::from_uddp_package(build_package_with_singles(&[(GUMP_ID, payload)])).unwrap();

        let error = package.read_gump_image(GUMP_ID).unwrap_err();
        assert!(error.to_string().contains("pixel format"), "{error}");
        assert!(package.single_gump_pixel_format(GUMP_ID).is_err());
    }

    /// A package from before version 3 fails to load with the repack message, whether its manifests are at version 2
    /// or, with no paperdoll gump in it, missing.
    #[test]
    fn packages_before_version_3_must_be_repacked() {
        const OLD_VERSION: u32 = 2;
        let old_manifests = build_package_with_manifests(
            common_manifest_header_at(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC, OLD_VERSION, 3, 2, 1, 0),
            common_manifest_header_at(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC, OLD_VERSION, 3, 2, 1, 0),
        );
        let no_manifests = {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            add_single(&mut builder, 42, &single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]));
            UddpReader::open(builder.build().unwrap()).unwrap()
        };

        for package in [old_manifests, no_manifests] {
            let error = GumpsPackage::from_uddp_package(package).err().expect("an old package is rejected");
            assert!(format!("{error:#}").contains("repack with the current udd-cli"), "{error:#}");
        }
    }

    #[test]
    fn package_reads_atlas_gump_payload() {
        let package = GumpsPackage::from_uddp_package(build_test_package(true)).unwrap();

        assert_eq!(package.cached_atlas_page_count(), 0);
        let image = package.read_gump_image(50_001).unwrap();

        assert_eq!((image.physical_width, image.physical_height), (2, 1));
        assert_eq!((image.logical_width, image.logical_height), (1, 1));
        assert_eq!(image.upscale_factor, 2);
        assert_eq!(image.rgba, vec![9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(package.cached_atlas_page_count(), 1);

        let image = package.read_gump_image(50_001).unwrap();

        assert_eq!(image.rgba, vec![9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(package.cached_atlas_page_count(), 1);
    }

    /// A `gumps_ec` that shares gumps: the listed ids read as absent, and each keeps the hash it was packed with.
    #[test]
    fn shared_classic_gumps_read_as_absent_with_their_hashes() {
        const STORED_GUMP_ID: u32 = 42;
        const SHARED: [SharedClassicGump; 2] = [
            SharedClassicGump { gump_id: 7, image_hash: 0x0123_4567_89AB_CDEF },
            SharedClassicGump { gump_id: 2000, image_hash: u64::MAX },
        ];
        const UNLISTED_GUMP_ID: u32 = 8;
        let package = GumpsPackage::from_uddp_package(build_package_with_shared_table(
            &[(STORED_GUMP_ID, single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]))],
            &encode_shared_classic_gump_table(&SHARED).unwrap(),
        ))
        .unwrap();

        assert_eq!(package.shared_classic_gump_count(), SHARED.len());
        assert_eq!(package.shared_classic_gumps().collect::<Vec<_>>(), SHARED);
        for shared in SHARED {
            assert_eq!(package.shared_classic_gump_hash(shared.gump_id), Some(shared.image_hash));
            assert!(package.read_gump_image(shared.gump_id).is_err(), "gump {}", shared.gump_id);
            assert_eq!(package.read_single_gump_image(shared.gump_id).unwrap(), None);
        }
        assert_eq!(package.shared_classic_gump_hash(UNLISTED_GUMP_ID), None);
        assert_eq!(package.shared_classic_gump_hash(STORED_GUMP_ID), None);
        assert_eq!(package.available_gump_ids(), vec![STORED_GUMP_ID]);

        let standalone = GumpsPackage::from_uddp_package(build_test_package(false)).unwrap();
        assert_eq!(standalone.shared_classic_gump_count(), 0);
        assert_eq!(standalone.shared_classic_gump_hash(STORED_GUMP_ID), None);
    }

    /// A package that stores a gump and also lists it as shared is refused, whether the gump is single or on an atlas
    /// page.
    #[test]
    fn a_gump_both_stored_and_shared_is_refused() {
        const SINGLE_GUMP_ID: u32 = 42;
        const ATLAS_GUMP_ID: u32 = 50_001;
        const HASH: u64 = 1;
        for gump_id in [SINGLE_GUMP_ID, ATLAS_GUMP_ID] {
            let table = encode_shared_classic_gump_table(&[SharedClassicGump { gump_id, image_hash: HASH }]).unwrap();
            let mut builder = atlas_test_package_builder();
            add_metadata(&mut builder, SHARED_CLASSIC_GUMP_TABLE_ID, &table);
            let package = UddpReader::open(builder.build().unwrap()).unwrap();

            let error = GumpsPackage::from_uddp_package(package).err().expect("an inconsistent package is refused");
            let message = format!("{error:#}");
            assert!(message.contains(&format!("stores gump {gump_id} and also lists it as shared")), "{message}");
        }
    }

    /// A table out of order, repeating an id, at another version or with a length that doesn't match its count is
    /// refused when the package opens.
    #[test]
    fn a_malformed_shared_table_is_refused() {
        const OTHER_VERSION: u32 = SHARED_CLASSIC_GUMP_TABLE_VERSION + 1;
        const VERSION_OFFSET: usize = SHARED_CLASSIC_GUMP_TABLE_MAGIC.len();
        let gump = |gump_id| SharedClassicGump { gump_id, image_hash: u64::from(gump_id) };
        let descending = [gump(9), gump(8)];
        let repeated = [gump(8), gump(8)];
        for gumps in [&descending, &repeated] {
            let error = encode_shared_classic_gump_table(gumps).unwrap_err();
            assert!(error.to_string().contains("must ascend"), "{error}");
        }

        let valid = encode_shared_classic_gump_table(&[gump(8), gump(9)]).unwrap();
        let mut reversed = valid.clone();
        let records = SHARED_CLASSIC_GUMP_TABLE_HEADER_BYTES;
        reversed[records..].rotate_left(SHARED_CLASSIC_GUMP_RECORD_BYTES);
        let mut other_version = valid.clone();
        other_version[VERSION_OFFSET..VERSION_OFFSET + size_of::<u32>()].copy_from_slice(&OTHER_VERSION.to_le_bytes());
        let truncated = valid[..valid.len() - 1].to_vec();
        for (table, expected) in [
            (reversed, "must ascend"),
            (other_version, "repack with the current udd-cli"),
            (truncated, "don't match the 2 gumps it declares"),
        ] {
            let error = GumpsPackage::from_uddp_package(build_package_with_shared_table(&[], &table))
                .err()
                .expect("a malformed table is refused");
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }

    /// The hash covers the size, the upscale factor and every RGBA byte, colour under a clear texel too, as xxh64 seed 0
    /// over their little-endian bytes.
    #[test]
    fn gump_image_hash_covers_size_factor_and_every_byte() {
        const CLEAR_RED: [u8; 4] = [255, 0, 0, 0];
        const CLEAR_GREEN: [u8; 4] = [0, 255, 0, 0];
        let image = GumpImage {
            physical_width: 2,
            physical_height: 1,
            logical_width: 2,
            logical_height: 1,
            upscale_factor: 1,
            rgba: [CLEAR_RED, [1, 2, 3, 255]].concat(),
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&image.physical_width.to_le_bytes());
        bytes.extend_from_slice(&image.physical_height.to_le_bytes());
        bytes.extend_from_slice(&image.upscale_factor.to_le_bytes());
        bytes.extend_from_slice(&image.rgba);
        assert_eq!(gump_image_hash(&image), xxhash_rust::xxh64::xxh64(&bytes, GUMP_IMAGE_HASH_SEED));

        let transposed = GumpImage { physical_width: 1, physical_height: 2, ..image.clone() };
        let upscaled = GumpImage { upscale_factor: 2, ..image.clone() };
        let mut recoloured_clear = image.clone();
        recoloured_clear.rgba[..CLEAR_GREEN.len()].copy_from_slice(&CLEAR_GREEN);
        for other in [transposed, upscaled, recoloured_clear] {
            assert_ne!(gump_image_hash(&other), gump_image_hash(&image), "{other:?}");
        }
    }

    fn add_metadata(builder: &mut UddpBuilder, id: u32, data: &[u8]) {
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: None,
                path_hash64: None,
                id: Some(id),
                data,
            })
            .unwrap();
    }

    fn build_package_with_shared_table(singles: &[(u32, Vec<u8>)], table: &[u8]) -> UddpReader {
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        for (gump_id, payload) in singles {
            add_single(&mut builder, *gump_id, payload);
        }
        add_manifests(
            &mut builder,
            &common_manifest_header(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC, 3, 2, 1, 0),
            &common_manifest_header(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC, 3, 2, 1, 0),
        );
        add_metadata(&mut builder, SHARED_CLASSIC_GUMP_TABLE_ID, table);
        UddpReader::open(builder.build().unwrap()).unwrap()
    }

    fn add_single(builder: &mut UddpBuilder, gump_id: u32, payload: &[u8]) {
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Gump as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: None,
                path_hash64: None,
                id: Some(gump_id),
                data: payload,
            })
            .unwrap();
    }

    fn add_manifests(builder: &mut UddpBuilder, page_manifest: &[u8], slot_manifest: &[u8]) {
        for (id, data) in [(GUMP_ATLAS_PAGE_MANIFEST_ID, page_manifest), (GUMP_ATLAS_SLOT_MANIFEST_ID, slot_manifest)] {
            builder
                .add_file(AddFileRequest {
                    data_type: DataType::Metadata as u8,
                    compression: CompressionFlag::None,
                    width: 0,
                    height: 0,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(id),
                    data,
                })
                .unwrap();
        }
    }

    /// Single gumps and the empty atlas manifests every current package carries.
    fn build_package_with_singles(singles: &[(u32, Vec<u8>)]) -> UddpReader {
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        for (gump_id, payload) in singles {
            add_single(&mut builder, *gump_id, payload);
        }
        add_manifests(
            &mut builder,
            &common_manifest_header(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC, 3, 2, 1, 0),
            &common_manifest_header(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC, 3, 2, 1, 0),
        );
        UddpReader::open(builder.build().unwrap()).unwrap()
    }

    fn build_package_with_manifests(page_manifest: Vec<u8>, slot_manifest: Vec<u8>) -> UddpReader {
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        add_manifests(&mut builder, &page_manifest, &slot_manifest);
        UddpReader::open(builder.build().unwrap()).unwrap()
    }

    fn build_test_package(include_atlas: bool) -> UddpReader {
        if !include_atlas {
            return build_package_with_singles(&[(42, single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]))]);
        }
        UddpReader::open(atlas_test_package_builder().build().unwrap()).unwrap()
    }

    /// Single gump 42 and paperdoll gump 50001 on an atlas page.
    fn atlas_test_package_builder() -> UddpBuilder {
        let mut builder = UddpBuilder::new(LookupMode::SparseId);
        add_single(&mut builder, 42, &single_payload(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]));

        let page_manifest = page_manifest();
        let slot_manifest = slot_manifest();
        let page_pixels = vec![
            0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        for (id, data_type, data, width, height) in [
            (GUMP_ATLAS_PAGE_MANIFEST_ID, DataType::Metadata as u8, page_manifest.as_slice(), 0, 0),
            (GUMP_ATLAS_SLOT_MANIFEST_ID, DataType::Metadata as u8, slot_manifest.as_slice(), 0, 0),
            (GUMP_ATLAS_PAGE_ID_BASE, DataType::Texture as u8, page_pixels.as_slice(), 3, 2),
        ] {
            builder
                .add_file(AddFileRequest {
                    data_type,
                    compression: CompressionFlag::None,
                    width,
                    height,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(id),
                    data,
                })
                .unwrap();
        }

        builder
    }

    fn single_payload(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        encode_single_gump_payload(width, height, rgba, GumpPixelFormat::Rgba8888).unwrap()
    }

    fn page_manifest() -> Vec<u8> {
        let mut bytes = common_manifest_header(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC, 3, 2, 1, 1);
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes
    }

    fn slot_manifest() -> Vec<u8> {
        let mut bytes = common_manifest_header(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC, 3, 2, 1, 1);
        bytes.write_u32::<LittleEndian>(50_001).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u16::<LittleEndian>(2).unwrap();
        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u16::<LittleEndian>(2).unwrap();
        bytes
    }

    fn common_manifest_header(
        magic: &[u8; 4],
        width: u32,
        height: u32,
        gutter: u32,
        count: u32,
    ) -> Vec<u8> {
        common_manifest_header_at(magic, GUMP_ATLAS_METADATA_VERSION, width, height, gutter, count)
    }

    fn common_manifest_header_at(
        magic: &[u8; 4],
        version: u32,
        width: u32,
        height: u32,
        gutter: u32,
        count: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(version).unwrap();
        bytes.write_u32::<LittleEndian>(width).unwrap();
        bytes.write_u32::<LittleEndian>(height).unwrap();
        bytes.write_u32::<LittleEndian>(gutter).unwrap();
        bytes.write_u32::<LittleEndian>(count).unwrap();
        bytes
    }
}
