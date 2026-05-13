use std::io::Write;
use std::path::Path;

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, Context};
use log::info;

use uocf::classic::light::LightMap;
use udd_assets::world_lights::{
    light_entry_path, WorldLightSlotRecord, SLOT_FLAG_PRESENT, SLOT_MANIFEST_ENTRY_PATH,
};
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

use crate::package_progress::build_and_write_package;

pub struct WorldLightsOptions {
    pub compression: CompressionFlag,
}

impl Default for WorldLightsOptions {
    fn default() -> Self {
        Self {
            compression: CompressionFlag::ZstdNoDict,
        }
    }
}

pub fn convert_client_lights_to_world_lights_uddp(
    cc_path: Option<&Path>,
    ec_path: Option<&Path>,
    out_file: &Path,
    options: &WorldLightsOptions,
) -> eyre::Result<()> {
    info!("Converting World Lights to {}", out_file.display());

    let light_map = LightMap::load(cc_path, ec_path).wrap_err("Failed to load client lights")?;

    let mut slots = Vec::new();
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);

    // CC lights are up to 100 entries. EC might have more.
    // We iterate up to 1024 to cover most standard and extended IDs.
    let limit = 1024;
    let mut added_count = 0;

    for id in 0..limit {
        if light_map.has_id(id) {
            match light_map.decode_light(id) {
                Ok((width, height, pixels)) => {
                    if width > 0 && height > 0 && !pixels.is_empty() {
                        let vpath = light_entry_path(id);
                        package.add_file(AddFileRequest {
                            data_type: DataType::Texture as u8,
                            compression: options.compression,
                            width: width as u32,
                            height: height as u32,
                            virtual_path: Some(&vpath),
                            path_hash64: None,
                            id: None,
                            data: &pixels,
                        })?;

                        slots.push(WorldLightSlotRecord {
                            light_id: id,
                            flags: SLOT_FLAG_PRESENT,
                            width,
                            height,
                        });
                        added_count += 1;
                        continue;
                    }
                }
                Err(e) => {
                    log::error!("Failed to decode light {}: {}", id, e);
                }
            }
        }
        slots.push(WorldLightSlotRecord::absent(id));
    }

    info!("Added {} lights to package", added_count);

    // Serialize slot manifest
    let slot_manifest = serialize_slot_manifest(&slots)?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: options.compression,
        width: 0,
        height: 0,
        virtual_path: Some(SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &slot_manifest,
    })?;

    build_and_write_package(&mut package, out_file)?;

    Ok(())
}

const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"WLSL";
const WORLD_LIGHTS_METADATA_VERSION: u32 = 1;

fn serialize_slot_manifest(slots: &[WorldLightSlotRecord]) -> eyre::Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.write_all(&SLOT_MANIFEST_MAGIC)?;
    buf.write_u32::<LittleEndian>(WORLD_LIGHTS_METADATA_VERSION)?;
    buf.write_u32::<LittleEndian>(slots.len() as u32)?;

    for slot in slots {
        buf.write_u32::<LittleEndian>(slot.light_id)?;
        buf.write_u16::<LittleEndian>(slot.flags)?;
        buf.write_u16::<LittleEndian>(slot.width)?;
        buf.write_u16::<LittleEndian>(slot.height)?;
    }

    Ok(buf)
}
