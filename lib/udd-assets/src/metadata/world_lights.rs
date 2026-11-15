//! `world_lights.uddp`: the Classic `light.mul` lights by light id, each its signed intensities as stored, one byte
//! per pixel (`lights/{id:08}.i8`), and the slot manifest of their sizes.

use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::{ensure_manifest_version, read_path_entry, read_path_entry_cow};

pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;

pub const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"WLSL";
pub const WORLD_LIGHTS_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldLightSlotRecord {
    pub light_id: u32,
    pub flags: u16,
    pub width: u16,
    pub height: u16,
}

impl WorldLightSlotRecord {
    pub fn absent(light_id: u32) -> Self {
        Self {
            light_id,
            flags: 0,
            width: 0,
            height: 0,
        }
    }

    pub fn is_present(self) -> bool {
        (self.flags & SLOT_FLAG_PRESENT) != 0
    }
}

pub struct WorldLightsPackage {
    package: UddpReader,
    slots: Vec<WorldLightSlotRecord>,
}

impl WorldLightsPackage {
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
        let slot_manifest = read_path_entry_cow(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("world_lights.uddp missing metadata/slots.bin")?;

        let slots = parse_slot_manifest(&slot_manifest)?;
        // Packages made before the signed intensities hold RGBA masks under other paths.
        if let Some(slot) = slots.iter().find(|slot| slot.is_present()) {
            eyre::ensure!(
                package.find_by_path_hash(udd_container::xxh64_virtual_path(&light_entry_path(slot.light_id))).is_some(),
                "world_lights.uddp has no {}: it predates the signed light intensities, repack it",
                light_entry_path(slot.light_id),
            );
        }

        Ok(Self {
            package,
            slots,
        })
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    pub fn slots(&self) -> &[WorldLightSlotRecord] {
        &self.slots
    }

    /// The light's signed intensities, `width * height` of them (`present_slot`): positive values light, negative
    /// ones darken, -31 to 31 in the shipped files.
    pub fn read_light_intensities(&self, light_id: u32) -> eyre::Result<Vec<i8>> {
        let slot = self.present_slot(light_id).ok_or_else(|| eyre::eyre!("no light {light_id}"))?;
        let bytes = read_path_entry(&self.package, &light_entry_path(light_id))
            .wrap_err_with(|| format!("unpack light {light_id}"))?;
        let pixels = usize::from(slot.width) * usize::from(slot.height);
        eyre::ensure!(bytes.len() == pixels, "light {light_id} has {} intensities for {}x{}", bytes.len(), slot.width,
            slot.height);
        Ok(bytes.into_iter().map(|byte| byte as i8).collect())
    }

    pub fn present_slot(&self, light_id: u32) -> Option<&WorldLightSlotRecord> {
        self.slots
            .get(light_id as usize)
            .filter(|slot| slot.is_present())
    }
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<Vec<WorldLightSlotRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("world_lights slot manifest", version, WORLD_LIGHTS_METADATA_VERSION)?;
    
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        slots.push(WorldLightSlotRecord {
            light_id: cursor.read_u32::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok(slots)
}

pub fn light_entry_path(light_id: u32) -> String {
    format!("lights/{light_id:08}.i8")
}

/// The largest intensity of the shipped `light.mul` files, which lights fully.
pub const LIGHT_FULL_INTENSITY: i8 = 31;

/// A light's intensities as an RGBA8888 preview: a lighting pixel grey by its intensity, a darkening one blue by its
/// depth, the rest transparent.
pub fn light_preview_rgba(intensities: &[i8]) -> Vec<u8> {
    let level = |value: i8| (i32::from(value).unsigned_abs().min(LIGHT_FULL_INTENSITY as u32) * 255
        / LIGHT_FULL_INTENSITY as u32) as u8;
    intensities
        .iter()
        .flat_map(|&value| match value {
            0 => [0, 0, 0, 0],
            1.. => [level(value), level(value), level(value), u8::MAX],
            _ => [0, 0, level(value), u8::MAX],
        })
        .collect()
}
