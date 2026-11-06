//! Runtime support for `tex_aux_ec.uddp`.
//!
//! This package holds the image-bearing support texture refs a runtime consumer reads: tileart
//! refs that are not the primary image (hue masks, terrain RGB B and alphas, water stages, noise) of
//! the families listed by the packer (currently WorldArt only). The primary image of each item is in
//! `tex_art_ec.uddp` instead.
//!
//! Package layout:
//! - `metadata/slots.bin`: flat array of `TexAuxEcSlotRecord`, sorted by `texture_id`.
//! - `textures/{texture_id:08x}`: raw pixel bytes per `texture_id` (format in slot record).
//!   The virtual path is hashed for lookup in the UDDP path-hash index.
//! - The client's missing-texture image (`Data\SystemTextures\missing.tga`) under the reserved
//!   id [`TEX_AUX_EC_MISSING_TEXTURE_ID`], with a slot record like any other image.
//! - One alias slot per referenced source id whose image the EC archive lacks
//!   ([`TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE`]). It has no payload of its own: its pixels are the
//!   missing-texture image, which the client substitutes in that case. A normal lookup
//!   ([`TexAuxEcPackage::slot_for_texture_id`], [`TexAuxEcPackage::load_rgba`]) therefore returns
//!   the magenta image for an absent id.
//! - `metadata/water_primary_extents.bin`: sorted native `u32 [id, width, height]`
//!   rows for water map-1 images, including primaries not stored in this package.
//!
//! Packages built before the aliases existed lack them (and may carry an unread
//! `metadata/absent_sources.bin`); absent ids then have no slot.

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::{xxh64_virtual_path, UddpReader};
use udd_image_codecs::bc7::{decode_from_vram, RawImageFormat, VramTextureData, VramTextureFormat};

use crate::common::read_pod_vec;
use crate::tex_art_cc::PagePixelFormat;

pub const TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";
/// Native (pre-upscale) dimensions of block-0 water primary images. The shader
/// samples other maps, but its local UV window is defined against these images.
pub const TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH: &str = "metadata/water_primary_extents.bin";
/// `TexAuxEcSlotRecord::flags`: the EC archive lacks this id's image. The slot is an alias of the
/// reserved missing-texture slot: width, height and pixel format are that image's, and its pixels
/// are read from the reserved payload, which is stored once.
pub const TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE: u8 = 1 << 0;

/// Reserved key of the client's missing-texture image (`Data\SystemTextures\missing.tga`).
///
/// No real texture can have this id. The client builds a tile texture path from its id with a
/// signed `%08d` (`Build/WorldArt/%08d.dds`), so the reachable ids stop at `i32::MAX`; `u32::MAX`
/// would print as `-0000001`, which names no client file.
pub const TEX_AUX_EC_MISSING_TEXTURE_ID: u32 = u32::MAX;

/// Virtual path template for a packed auxiliary texture image.
pub fn texture_entry_path(texture_id: u32) -> String {
    format!("textures/{:08x}", texture_id)
}

/// Binary record stored in `metadata/slots.bin`.
///
/// One record per packed auxiliary texture ref.
/// Records are sorted by `texture_id` for runtime binary search.
///
/// The first padding byte carries `pixel_format` (0 = Rgba8888, 1 = Bc7); old UDDP files with
/// all-zero padding read as Rgba8888. The second carries `flags` (`TEX_AUX_EC_SLOT_FLAG_*`).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TexAuxEcSlotRecord {
    /// Source texture id from `TileMetaItemTextureRef.texture_id`.
    pub texture_id: u32,
    /// Image width in pixels.
    pub width: u16,
    /// Image height in pixels.
    pub height: u16,
    /// Logical family byte (`EcMaterialLogicalFamily as u8`).
    pub logical_family: u8,
    /// Stable role byte (`EcMaterialStableRole as u8`).
    pub stable_role: u8,
    /// Pixel format: 0 = Rgba8888, 1 = Bc7.
    pub pixel_format: u8,
    /// `TEX_AUX_EC_SLOT_FLAG_*` bits.
    pub flags: u8,
}

impl TexAuxEcSlotRecord {
    /// The EC archive lacks the source image; the slot reads the missing-texture image.
    pub fn is_missing_source(&self) -> bool {
        self.flags & TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE != 0
    }

    /// The id whose `textures/{id:08x}` payload holds this slot's pixels.
    pub fn payload_texture_id(&self) -> u32 {
        if self.is_missing_source() { TEX_AUX_EC_MISSING_TEXTURE_ID } else { self.texture_id }
    }

    /// The payload holds BC7 blocks ([`TexAuxEcPackage::load_bc7`] reads them as stored).
    pub fn is_bc7(&self) -> bool {
        self.pixel_format == PagePixelFormat::Bc7 as u8
    }
}

/// Runtime handle for an auxiliary texture slot.
#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub texture_id: u32,
    pub width: u16,
    pub height: u16,
    pub logical_family: u8,
    pub stable_role: u8,
    pub pixel_format: u8,
    /// The id of the payload holding the pixels: `texture_id`, or the reserved missing-texture id
    /// for an absent source. Images can be cached by it.
    pub payload_texture_id: u32,
}

impl SlotInfo {
    /// The EC archive lacks the source image; the pixels are the client's missing-texture image.
    pub fn is_missing_source(&self) -> bool {
        self.payload_texture_id != self.texture_id
    }
}

/// Runtime reader for `tex_aux_ec.uddp`.
pub struct TexAuxEcPackage {
    package: UddpReader,
    slots: Vec<TexAuxEcSlotRecord>,
    water_primary_extents: Vec<[u32; 3]>,
}

impl From<&TexAuxEcSlotRecord> for SlotInfo {
    fn from(slot: &TexAuxEcSlotRecord) -> Self {
        Self {
            texture_id: slot.texture_id,
            width: slot.width,
            height: slot.height,
            logical_family: slot.logical_family,
            stable_role: slot.stable_role,
            pixel_format: slot.pixel_format,
            payload_texture_id: slot.payload_texture_id(),
        }
    }
}

impl TexAuxEcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let slot_bytes = crate::common::read_path_entry_cow(&package, TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH)
            .wrap_err("read tex_aux_ec slot manifest")?;
        let slots: Vec<TexAuxEcSlotRecord> = read_pod_vec(slot_bytes.as_ref(), TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH)?;
        eyre::ensure!(slots.windows(2).all(|pair| pair[0].texture_id < pair[1].texture_id),
            "{TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH} is not sorted by unique texture id");
        let aliases_missing = slots.iter().any(TexAuxEcSlotRecord::is_missing_source);
        let has_missing = slots.binary_search_by_key(&TEX_AUX_EC_MISSING_TEXTURE_ID, |slot| slot.texture_id).is_ok();
        eyre::ensure!(!aliases_missing || has_missing,
            "tex_aux_ec has missing-source aliases but no missing-texture slot {TEX_AUX_EC_MISSING_TEXTURE_ID:#x}");
        let extent_bytes = crate::common::read_path_entry_cow(&package, TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH)
            .wrap_err("read native water-primary extents; rebuild tex_aux_ec with the current packer")?;
        let water_primary_extents = read_pod_vec::<[u32; 3]>(extent_bytes.as_ref(), TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH)?;
        eyre::ensure!(water_primary_extents.windows(2).all(|pair| pair[0][0] < pair[1][0])
            && water_primary_extents.iter().all(|row| row[1] > 0 && row[2] > 0),
            "invalid native water-primary extents");
        Ok(Self { package, slots, water_primary_extents })
    }

    /// Full source dimensions, independent of the trimmed art slot and auxiliary upscaling.
    /// This does not decode or upload the hardware-fallback image.
    pub fn water_primary_extent(&self, texture_id: u32) -> Option<[u32; 2]> {
        let pos = self.water_primary_extents.binary_search_by_key(&texture_id, |row| row[0]).ok()?;
        Some([self.water_primary_extents[pos][1], self.water_primary_extents[pos][2]])
    }

    /// Return slot metadata for the given `texture_id`, if packed. For a source the EC archive
    /// lacks, this is the alias of the missing-texture image (`SlotInfo::is_missing_source`).
    pub fn slot_for_texture_id(&self, texture_id: u32) -> Option<SlotInfo> {
        let pos = self
            .slots
            .binary_search_by_key(&texture_id, |slot| slot.texture_id)
            .ok()?;
        Some(SlotInfo::from(&self.slots[pos]))
    }

    /// The client's missing-texture image, when the package carries it.
    pub fn missing_texture_slot(&self) -> Option<SlotInfo> {
        self.slot_for_texture_id(TEX_AUX_EC_MISSING_TEXTURE_ID)
    }

    /// Load the RGBA8888 pixels for the given `texture_id`.
    ///
    /// Returns `None` when the id is not present in the package. A missing-source alias returns the
    /// missing-texture image. Transparently decodes BC7-encoded entries so callers always receive
    /// RGBA8888.
    pub fn load_rgba(&self, texture_id: u32) -> eyre::Result<Option<Vec<u8>>> {
        let Some((bc7, bytes)) = self.read_payload(texture_id)? else { return Ok(None) };
        if bc7 {
            let vram = Self::bc7_container(texture_id, &bytes)?;
            let rgba = decode_from_vram(&vram, RawImageFormat::Rgba8888)
                .map_err(|e| eyre::eyre!("decode BC7 for tex_aux_ec texture {texture_id}: {e}"))?;
            Ok(Some(rgba))
        } else {
            Ok(Some(bytes))
        }
    }

    /// Load the BC7 blocks for the given `texture_id` as stored, without decoding them, for an upload that keeps them.
    ///
    /// Returns `None` when the id is not present in the package; a missing-source alias returns the missing-texture
    /// image. An RGBA8888 entry is an error: [`Self::load_rgba`] reads every entry.
    pub fn load_bc7(&self, texture_id: u32) -> eyre::Result<Option<VramTextureData>> {
        let Some((bc7, bytes)) = self.read_payload(texture_id)? else { return Ok(None) };
        eyre::ensure!(bc7, "tex_aux_ec texture {texture_id} is RGBA8888, not BC7");
        let vram = Self::bc7_container(texture_id, &bytes)?;
        eyre::ensure!(vram.format() == VramTextureFormat::Bc7RgbaUnormSrgb,
            "tex_aux_ec texture {texture_id} container holds {:?}, not BC7", vram.format());
        Ok(Some(vram))
    }

    /// Whether the id's slot is BC7, and its payload's stored bytes; `None` when the package has no payload for it.
    fn read_payload(&self, texture_id: u32) -> eyre::Result<Option<(bool, Vec<u8>)>> {
        let slot_pos = self
            .slots
            .binary_search_by_key(&texture_id, |s| s.texture_id)
            .ok();
        let payload_texture_id = slot_pos.map_or(texture_id, |pos| self.slots[pos].payload_texture_id());
        let path = texture_entry_path(payload_texture_id);
        let hash = xxh64_virtual_path(&path);
        match self.package.read_file_by_path_hash(hash) {
            Ok(bytes) => Ok(Some((slot_pos.is_some_and(|pos| self.slots[pos].is_bc7()), bytes))),
            Err(udd_container::uddp::FormatError::FileNotFound) => Ok(None),
            Err(e) => Err(eyre::eyre!("load tex_aux_ec texture {texture_id}: {e}")),
        }
    }

    fn bc7_container(texture_id: u32, bytes: &[u8]) -> eyre::Result<VramTextureData> {
        VramTextureData::from_container_bytes(bytes)
            .map_err(|e| eyre::eyre!("parse BC7 container for tex_aux_ec texture {texture_id}: {e}"))
    }

    pub fn slots(&self) -> &[TexAuxEcSlotRecord] {
        &self.slots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

    const WORLD_ART: u8 = crate::tilemeta::EcMaterialLogicalFamily::WorldArt as u8;
    const SYSTEM_TEXTURES: u8 = crate::tilemeta::EcMaterialLogicalFamily::SystemTextures as u8;
    const PACKED_ID: u32 = 20_002_472;
    const ABSENT_ID: u32 = 20_004_039;
    const UNLISTED_ID: u32 = 20_000_001;
    const IMAGE_SIDE: u16 = 2;
    const RGBA_CHANNELS: usize = 4;
    const MISSING_PIXEL: [u8; 4] = [255, 0, 192, 255];
    const PACKED_PIXEL: [u8; 4] = [10, 20, 30, 40];

    fn slot(texture_id: u32, logical_family: u8) -> TexAuxEcSlotRecord {
        TexAuxEcSlotRecord { texture_id, width: IMAGE_SIDE, height: IMAGE_SIDE, logical_family, ..Default::default() }
    }

    fn alias(texture_id: u32) -> TexAuxEcSlotRecord {
        TexAuxEcSlotRecord { flags: TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE, ..slot(texture_id, WORLD_ART) }
    }

    /// `slots` must be sorted; payloads are written for the non-alias slots only.
    fn package(slots: &[TexAuxEcSlotRecord]) -> eyre::Result<TexAuxEcPackage> {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        builder.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8, compression: CompressionFlag::None, width: 0, height: 0,
            virtual_path: Some(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH), path_hash64: None, id: None,
            data: bytemuck::cast_slice(slots),
        })?;
        builder.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8, compression: CompressionFlag::None, width: 0, height: 0,
            virtual_path: Some(TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH), path_hash64: None, id: None,
            data: bytemuck::cast_slice(&[[4651u32, 64, 32]]),
        })?;
        let images: Vec<(String, Vec<u8>)> = slots.iter().filter(|slot| !slot.is_missing_source()).map(|slot| {
            let pixel = if slot.texture_id == TEX_AUX_EC_MISSING_TEXTURE_ID { MISSING_PIXEL } else { PACKED_PIXEL };
            (texture_entry_path(slot.texture_id), pixel.repeat(usize::from(IMAGE_SIDE) * usize::from(IMAGE_SIDE)))
        }).collect();
        for (path, data) in &images {
            builder.add_file(AddFileRequest {
                data_type: DataType::Texture as u8, compression: CompressionFlag::None,
                width: IMAGE_SIDE.into(), height: IMAGE_SIDE.into(), virtual_path: Some(path),
                path_hash64: None, id: None, data,
            })?;
        }
        TexAuxEcPackage::from_uddp_package(UddpReader::open(builder.build()?)?)
    }

    #[test]
    fn an_absent_source_reads_the_missing_texture_through_a_normal_lookup() -> eyre::Result<()> {
        let package = package(&[slot(PACKED_ID, WORLD_ART), alias(ABSENT_ID), slot(TEX_AUX_EC_MISSING_TEXTURE_ID, SYSTEM_TEXTURES)])?;
        let packed = package.slot_for_texture_id(PACKED_ID).expect("packed slot");
        assert!(!packed.is_missing_source());
        assert_eq!(&package.load_rgba(PACKED_ID)?.expect("packed pixels")[..RGBA_CHANNELS], &PACKED_PIXEL);

        let absent = package.slot_for_texture_id(ABSENT_ID).expect("alias slot");
        assert!(absent.is_missing_source());
        assert_eq!(absent.logical_family, WORLD_ART, "the alias keeps the ref family consumers check");
        assert_eq!(absent.payload_texture_id, TEX_AUX_EC_MISSING_TEXTURE_ID);
        assert_eq!(&package.load_rgba(ABSENT_ID)?.expect("missing-texture pixels")[..RGBA_CHANNELS], &MISSING_PIXEL);

        assert!(package.slot_for_texture_id(UNLISTED_ID).is_none());
        assert!(package.load_rgba(UNLISTED_ID)?.is_none());
        assert!(package.missing_texture_slot().is_some_and(|slot| !slot.is_missing_source()));
        assert_eq!(package.water_primary_extent(4651), Some([64, 32]));
        assert_eq!(package.water_primary_extent(7379), None);
        Ok(())
    }

    /// A BC7 slot's blocks come back as stored and decode through `load_rgba`; an RGBA slot is no BC7 source.
    #[test]
    fn bc7_entries_load_as_their_blocks() -> eyre::Result<()> {
        const BC7_ID: u32 = 20_002_473;
        // Any 16 bytes are one decodable BC7 block; distinct ones show a reordered copy.
        const BLOCK: [u8; udd_image_codecs::bc7::BC7_BLOCK_BYTES] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let extent = udd_image_codecs::bc7::ImageExtent::new(IMAGE_SIDE.into(), IMAGE_SIDE.into())?;
        let rgba = PACKED_PIXEL.repeat(usize::from(IMAGE_SIDE) * usize::from(IMAGE_SIDE));
        let blocks = VramTextureData::new(extent, VramTextureFormat::Bc7RgbaUnormSrgb, BLOCK.to_vec().into())?;
        let bc7_slot = TexAuxEcSlotRecord { pixel_format: PagePixelFormat::Bc7 as u8, ..slot(BC7_ID, WORLD_ART) };
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        let slots = [slot(PACKED_ID, WORLD_ART), bc7_slot];
        for (path, data) in [(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH, bytemuck::cast_slice(&slots).to_vec()),
            (TEX_AUX_EC_WATER_EXTENTS_ENTRY_PATH, bytemuck::cast_slice(&[[4651u32, 64, 32]]).to_vec()),
            (&texture_entry_path(PACKED_ID), rgba.clone()), (&texture_entry_path(BC7_ID), blocks.to_container_bytes())] {
            builder.add_file(AddFileRequest {
                data_type: DataType::Metadata as u8, compression: CompressionFlag::None, width: 0, height: 0,
                virtual_path: Some(path), path_hash64: None, id: None, data: &data,
            })?;
        }
        let package = TexAuxEcPackage::from_uddp_package(UddpReader::open(builder.build()?)?)?;
        assert!(package.slots()[1].is_bc7() && !package.slots()[0].is_bc7());
        let loaded = package.load_bc7(BC7_ID)?.expect("BC7 blocks");
        assert_eq!((loaded.bytes(), loaded.extent()), (blocks.bytes(), extent));
        assert_eq!(package.load_rgba(BC7_ID)?, Some(decode_from_vram(&blocks, RawImageFormat::Rgba8888)?));
        assert!(package.load_bc7(PACKED_ID).is_err());
        assert!(package.load_bc7(UNLISTED_ID)?.is_none());
        Ok(())
    }

    #[test]
    fn aliases_need_the_missing_texture_slot() -> eyre::Result<()> {
        assert!(package(&[slot(PACKED_ID, WORLD_ART), alias(ABSENT_ID)]).is_err());
        let without_fallback = package(&[slot(PACKED_ID, WORLD_ART)])?;
        assert!(without_fallback.missing_texture_slot().is_none());
        assert!(without_fallback.slot_for_texture_id(ABSENT_ID).is_none());
        assert!(package(&[slot(TEX_AUX_EC_MISSING_TEXTURE_ID, SYSTEM_TEXTURES), slot(PACKED_ID, WORLD_ART)]).is_err(), "unsorted");
        Ok(())
    }
}
