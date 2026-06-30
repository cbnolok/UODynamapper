//! Runtime support for `tex_aux_ec.uddp`.
//!
//! This package holds image-bearing support texture refs that belong to tileart
//! entries but have a non-base stable role (ImageSupport, AlphaMask, Noise, etc.).
//! It is the natural home for any ref that is owned by tileart yet should not
//! land in `tex_art_ec.uddp` (which keeps only base-role images).
//!
//! Package layout:
//! - `metadata/slots.bin`: flat array of `TexAuxEcSlotRecord`, sorted by `texture_id`.
//! - `textures/{texture_id:08x}`: RGBA8888 raw pixel bytes per `texture_id`.
//!   The virtual path is hashed for lookup in the UDDP path-hash index.

use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::{xxh64_virtual_path, UddpReader};

use crate::common::read_pod_vec;

pub const TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

/// Virtual path template for a packed auxiliary texture image.
pub fn texture_entry_path(texture_id: u32) -> String {
    format!("textures/{:08x}", texture_id)
}

/// Binary record stored in `metadata/slots.bin`.
///
/// One record per packed auxiliary texture ref.
/// Records are sorted by `texture_id` for runtime binary search.
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
    pub _pad: u16,
}

/// Runtime handle for an auxiliary texture slot.
#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub texture_id: u32,
    pub width: u16,
    pub height: u16,
    pub logical_family: u8,
    pub stable_role: u8,
}

/// Runtime reader for `tex_aux_ec.uddp`.
pub struct TexAuxEcPackage {
    package: UddpReader,
    slots: Vec<TexAuxEcSlotRecord>,
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
        Ok(Self { package, slots })
    }

    /// Return slot metadata for the given `texture_id`, if packed.
    pub fn slot_for_texture_id(&self, texture_id: u32) -> Option<SlotInfo> {
        let pos = self
            .slots
            .binary_search_by_key(&texture_id, |slot| slot.texture_id)
            .ok()?;
        let slot = &self.slots[pos];
        Some(SlotInfo {
            texture_id: slot.texture_id,
            width: slot.width,
            height: slot.height,
            logical_family: slot.logical_family,
            stable_role: slot.stable_role,
        })
    }

    /// Load the RGBA8888 pixels for the given `texture_id`.
    ///
    /// Returns `None` when the id is not present in the package.
    pub fn load_rgba(&self, texture_id: u32) -> eyre::Result<Option<Vec<u8>>> {
        let path = texture_entry_path(texture_id);
        let hash = xxh64_virtual_path(&path);
        match self.package.read_file_by_path_hash(hash) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(udd_container::uddp::FormatError::FileNotFound) => Ok(None),
            Err(e) => Err(eyre::eyre!("load tex_aux_ec texture {texture_id}: {e}")),
        }
    }

    pub fn slots(&self) -> &[TexAuxEcSlotRecord] {
        &self.slots
    }
}
