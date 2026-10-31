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
//!
//! Packages built before the aliases existed lack them (and may carry an unread
//! `metadata/absent_sources.bin`); absent ids then have no slot.

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::{xxh64_virtual_path, UddpReader};
use udd_image_codecs::bc7::{decode_from_vram, RawImageFormat, VramTextureData};

use crate::common::read_pod_vec;

pub const TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";
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
        Ok(Self { package, slots })
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
        let slot_pos = self
            .slots
            .binary_search_by_key(&texture_id, |s| s.texture_id)
            .ok();
        let payload_texture_id = slot_pos.map_or(texture_id, |pos| self.slots[pos].payload_texture_id());
        let path = texture_entry_path(payload_texture_id);
        let hash = xxh64_virtual_path(&path);
        match self.package.read_file_by_path_hash(hash) {
            Ok(bytes) => {
                let pixel_format = slot_pos.map(|pos| self.slots[pos].pixel_format).unwrap_or(0);
                if pixel_format == 1 {
                    let vram = VramTextureData::from_container_bytes(&bytes)
                        .map_err(|e| eyre::eyre!("parse BC7 container for tex_aux_ec texture {texture_id}: {e}"))?;
                    let rgba = decode_from_vram(&vram, RawImageFormat::Rgba8888)
                        .map_err(|e| eyre::eyre!("decode BC7 for tex_aux_ec texture {texture_id}: {e}"))?;
                    Ok(Some(rgba))
                } else {
                    Ok(Some(bytes))
                }
            }
            Err(udd_container::uddp::FormatError::FileNotFound) => Ok(None),
            Err(e) => Err(eyre::eyre!("load tex_aux_ec texture {texture_id}: {e}")),
        }
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
