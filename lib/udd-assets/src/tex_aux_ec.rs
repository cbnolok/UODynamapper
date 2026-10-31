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
//! - `metadata/absent_sources.bin`: the referenced source ids whose image the EC archive lacks
//!   (see [`TexAuxEcAbsentSourceRecord`]). The client substitutes `missing.tga` for exactly those;
//!   [`TexAuxEcPackage::get_or_missing`] does the same.
//!
//! Packages built before the fallback existed have neither the reserved entry nor the absent list;
//! they read as having no fallback.

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::{xxh64_virtual_path, UddpReader};
use udd_image_codecs::bc7::{decode_from_vram, RawImageFormat, VramTextureData};

use crate::common::{ensure_manifest_version, read_pod_vec};

pub const TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";
pub const TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH: &str = "metadata/absent_sources.bin";
/// Bump when the layout of `metadata/absent_sources.bin` changes.
pub const TEX_AUX_EC_ABSENT_SOURCES_VERSION: u32 = 1;
/// Byte size of the `u32` version that starts `metadata/absent_sources.bin`.
const ABSENT_SOURCES_HEADER_SIZE: usize = std::mem::size_of::<u32>();

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
/// The first padding byte carries `pixel_format` (0 = Rgba8888, 1 = Bc7).
/// Old UDDP files with all-zero padding read as Rgba8888.
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
    pub _pad: u8,
}

/// Binary record of `metadata/absent_sources.bin`, after its `u32` version.
///
/// One record per distinct source image a packed-family tileart ref names but the EC archive
/// (`physical_package`) does not contain. Records are sorted by `texture_id`. A source that exists
/// but fails to decode is not listed: the client shows Gamebryo's default texture for those, not
/// `missing.tga`.
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TexAuxEcAbsentSourceRecord {
    /// Source texture id from `TileMetaItemTextureRef.texture_id`.
    pub texture_id: u32,
    /// Logical family byte of the referencing ref (`EcMaterialLogicalFamily as u8`).
    pub logical_family: u8,
    /// Archive the image was looked up in (`EcMaterialPhysicalPackage as u8`).
    pub physical_package: u8,
    pub _pad: [u8; 2],
}

/// Serializes `metadata/absent_sources.bin`: the version, then `records` sorted by `texture_id`.
pub fn encode_absent_sources(records: &[TexAuxEcAbsentSourceRecord]) -> Vec<u8> {
    let mut sorted = records.to_vec();
    sorted.sort_by_key(|record| record.texture_id);
    let mut bytes = TEX_AUX_EC_ABSENT_SOURCES_VERSION.to_le_bytes().to_vec();
    bytes.extend_from_slice(bytemuck::cast_slice(&sorted));
    bytes
}

fn decode_absent_sources(bytes: &[u8]) -> eyre::Result<Vec<TexAuxEcAbsentSourceRecord>> {
    eyre::ensure!(bytes.len() >= ABSENT_SOURCES_HEADER_SIZE, "{TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH} has no version");
    let (version, records) = bytes.split_at(ABSENT_SOURCES_HEADER_SIZE);
    let version = u32::from_le_bytes(version.try_into().expect("header size"));
    ensure_manifest_version("tex_aux_ec absent sources", version, TEX_AUX_EC_ABSENT_SOURCES_VERSION)?;
    let records: Vec<TexAuxEcAbsentSourceRecord> = read_pod_vec(records, TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH)?;
    eyre::ensure!(records.windows(2).all(|pair| pair[0].texture_id <= pair[1].texture_id),
        "{TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH} is not sorted by texture id");
    Ok(records)
}

/// Result of [`TexAuxEcPackage::get_or_missing`].
#[derive(Debug, Clone)]
pub enum TexAuxEcLookup {
    /// The source image itself is packed.
    Packed(SlotInfo),
    /// The EC archive lacks the source image; this is the reserved missing-texture slot, which the
    /// client substitutes in that case.
    MissingSource(SlotInfo),
}

impl TexAuxEcLookup {
    /// The slot to read: pass its `texture_id` to [`TexAuxEcPackage::load_rgba`].
    pub fn slot(&self) -> &SlotInfo {
        match self {
            Self::Packed(slot) | Self::MissingSource(slot) => slot,
        }
    }

    pub fn is_missing_source(&self) -> bool {
        matches!(self, Self::MissingSource(_))
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
}

/// Runtime reader for `tex_aux_ec.uddp`.
pub struct TexAuxEcPackage {
    package: UddpReader,
    slots: Vec<TexAuxEcSlotRecord>,
    absent_sources: Vec<TexAuxEcAbsentSourceRecord>,
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
        let slots = read_pod_vec(slot_bytes.as_ref(), TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH)?;
        let absent_sources = if package.find_by_path_hash(xxh64_virtual_path(TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH)).is_some() {
            let bytes = crate::common::read_path_entry_cow(&package, TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH)?;
            decode_absent_sources(bytes.as_ref())?
        } else {
            Vec::new()
        };
        Ok(Self { package, slots, absent_sources })
    }

    /// Return slot metadata for the given `texture_id`, if packed.
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

    /// Source ids the EC archive lacks, sorted by `texture_id`.
    pub fn absent_sources(&self) -> &[TexAuxEcAbsentSourceRecord] {
        &self.absent_sources
    }

    /// Whether a ref of `logical_family` names a source image the EC archive lacks.
    pub fn is_absent_source(&self, texture_id: u32, logical_family: u8) -> bool {
        let start = self.absent_sources.partition_point(|record| record.texture_id < texture_id);
        self.absent_sources[start..].iter().take_while(|record| record.texture_id == texture_id)
            .any(|record| record.logical_family == logical_family)
    }

    /// The image to sample for a ref's `texture_id` and `logical_family`, as the client resolves it.
    ///
    /// Returns the packed slot when its family matches. When the EC archive lacks the source
    /// image (it is listed in `metadata/absent_sources.bin`), returns the reserved missing-texture
    /// slot, which the client loads in that case. Returns `None` otherwise: the id is not packed
    /// for this family (another family's image, a base image, an undecodable source), or the
    /// package predates the fallback.
    pub fn get_or_missing(&self, texture_id: u32, logical_family: u8) -> Option<TexAuxEcLookup> {
        if let Some(slot) = self.slot_for_texture_id(texture_id) {
            return (slot.logical_family == logical_family && texture_id != TEX_AUX_EC_MISSING_TEXTURE_ID)
                .then_some(TexAuxEcLookup::Packed(slot));
        }
        if !self.is_absent_source(texture_id, logical_family) {
            return None;
        }
        self.missing_texture_slot().map(TexAuxEcLookup::MissingSource)
    }

    /// Load the RGBA8888 pixels for the given `texture_id`.
    ///
    /// Returns `None` when the id is not present in the package.
    /// Transparently decodes BC7-encoded entries so callers always receive RGBA8888.
    pub fn load_rgba(&self, texture_id: u32) -> eyre::Result<Option<Vec<u8>>> {
        let slot_pos = self
            .slots
            .binary_search_by_key(&texture_id, |s| s.texture_id)
            .ok();
        let path = texture_entry_path(texture_id);
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
    const TEXTURE_PACKAGE: u8 = crate::tilemeta::EcMaterialPhysicalPackage::Texture as u8;
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

    fn package(with_fallback: bool, absent_bytes: Option<Vec<u8>>) -> eyre::Result<TexAuxEcPackage> {
        let mut slots = vec![slot(PACKED_ID, WORLD_ART)];
        let mut images = vec![(PACKED_ID, PACKED_PIXEL)];
        if with_fallback {
            slots.push(slot(TEX_AUX_EC_MISSING_TEXTURE_ID, SYSTEM_TEXTURES));
            images.push((TEX_AUX_EC_MISSING_TEXTURE_ID, MISSING_PIXEL));
        }
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        fn metadata<'a>(path: &'a str, data: &'a [u8]) -> AddFileRequest<'a> {
            AddFileRequest {
                data_type: DataType::Metadata as u8, compression: CompressionFlag::None, width: 0, height: 0,
                virtual_path: Some(path), path_hash64: None, id: None, data,
            }
        }
        builder.add_file(metadata(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH, bytemuck::cast_slice(&slots)))?;
        if let Some(bytes) = &absent_bytes {
            builder.add_file(metadata(TEX_AUX_EC_ABSENT_SOURCES_ENTRY_PATH, bytes))?;
        }
        let paths: Vec<String> = images.iter().map(|(id, _)| texture_entry_path(*id)).collect();
        for ((_, pixel), path) in images.iter().zip(&paths) {
            let data = pixel.repeat(usize::from(IMAGE_SIDE) * usize::from(IMAGE_SIDE));
            builder.add_file(AddFileRequest {
                data_type: DataType::Texture as u8, compression: CompressionFlag::None,
                width: IMAGE_SIDE.into(), height: IMAGE_SIDE.into(), virtual_path: Some(path),
                path_hash64: None, id: None, data: &data,
            })?;
        }
        TexAuxEcPackage::from_uddp_package(UddpReader::open(builder.build()?)?)
    }

    fn absent(texture_id: u32) -> TexAuxEcAbsentSourceRecord {
        TexAuxEcAbsentSourceRecord { texture_id, logical_family: WORLD_ART, physical_package: TEXTURE_PACKAGE, _pad: [0; 2] }
    }

    #[test]
    fn absent_sources_resolve_to_the_missing_texture() -> eyre::Result<()> {
        let package = package(true, Some(encode_absent_sources(&[absent(ABSENT_ID)])))?;
        let packed = package.get_or_missing(PACKED_ID, WORLD_ART).expect("packed slot");
        assert!(!packed.is_missing_source());
        assert_eq!(packed.slot().texture_id, PACKED_ID);

        let fallback = package.get_or_missing(ABSENT_ID, WORLD_ART).expect("missing-texture fallback");
        assert!(fallback.is_missing_source());
        assert_eq!(fallback.slot().texture_id, TEX_AUX_EC_MISSING_TEXTURE_ID);
        let rgba = package.load_rgba(fallback.slot().texture_id)?.expect("fallback pixels");
        assert_eq!(&rgba[..RGBA_CHANNELS], &MISSING_PIXEL);

        assert!(package.get_or_missing(ABSENT_ID, SYSTEM_TEXTURES).is_none(), "absence is per family");
        assert!(package.get_or_missing(PACKED_ID, SYSTEM_TEXTURES).is_none(), "another family's image is not substituted");
        assert!(package.get_or_missing(UNLISTED_ID, WORLD_ART).is_none(), "an unlisted id is not an absent source");
        assert!(package.get_or_missing(TEX_AUX_EC_MISSING_TEXTURE_ID, SYSTEM_TEXTURES).is_none(), "the reserved key is not a ref target");
        Ok(())
    }

    #[test]
    fn packages_without_the_fallback_read_as_having_none() -> eyre::Result<()> {
        let package = package(false, None)?;
        assert!(package.absent_sources().is_empty());
        assert!(package.missing_texture_slot().is_none());
        assert!(package.get_or_missing(ABSENT_ID, WORLD_ART).is_none());
        assert!(package.get_or_missing(PACKED_ID, WORLD_ART).is_some());
        Ok(())
    }

    #[test]
    fn absent_sources_are_versioned_and_sorted() -> eyre::Result<()> {
        let bytes = encode_absent_sources(&[absent(ABSENT_ID), absent(UNLISTED_ID)]);
        let records = decode_absent_sources(&bytes)?;
        assert_eq!(records.iter().map(|record| record.texture_id).collect::<Vec<_>>(), [UNLISTED_ID, ABSENT_ID]);
        let mut wrong_version = bytes.clone();
        wrong_version[..ABSENT_SOURCES_HEADER_SIZE].copy_from_slice(&(TEX_AUX_EC_ABSENT_SOURCES_VERSION + 1).to_le_bytes());
        assert!(package(true, Some(wrong_version)).is_err());
        Ok(())
    }
}
