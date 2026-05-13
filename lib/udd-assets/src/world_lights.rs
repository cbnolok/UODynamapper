use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::read_path_entry;

pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;

const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"WLSL";
const WORLD_LIGHTS_METADATA_VERSION: u32 = 1;

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
        let slot_manifest = read_path_entry(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("world_lights.uddp missing metadata/slots.bin")?;

        let slots = parse_slot_manifest(&slot_manifest)?;

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

    pub fn read_light_bytes(&self, light_id: u32) -> eyre::Result<Vec<u8>> {
        read_path_entry(&self.package, &light_entry_path(light_id))
            .wrap_err_with(|| format!("unpack light {light_id}"))
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
    if version != WORLD_LIGHTS_METADATA_VERSION { eyre::bail!("invalid version"); }
    
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
    format!("lights/{light_id:08}.rgba8888")
}
