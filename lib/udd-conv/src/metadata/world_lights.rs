use std::io::Write;
use std::path::Path;

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, Context};
use log::info;

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use uocf::classic::light::LightMap;
use udd_assets::world_lights::{
    light_entry_path, WorldLightSlotRecord, SLOT_FLAG_PRESENT, SLOT_MANIFEST_ENTRY_PATH, SLOT_MANIFEST_MAGIC,
    WORLD_LIGHTS_METADATA_VERSION,
};
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

use crate::package_progress::build_and_write_package;

pub struct WorldLightsOptions {
    pub compression: CompressionFlag,
    pub classic_patches: ClassicPatchOptions,
}

impl Default for WorldLightsOptions {
    fn default() -> Self {
        Self {
            compression: CompressionFlag::ZstdNoDict,
            classic_patches: ClassicPatchOptions::NONE,
        }
    }
}

/// Light ids of the slot manifest: `0..WORLD_LIGHT_SLOT_COUNT`. Classic `lightidx.mul` holds about 100.
const WORLD_LIGHT_SLOT_COUNT: u32 = 1024;

/// Packs the Classic light masks (`light.mul`, `lightidx.mul`, patched by verdata when enabled), keyed by light id.
///
/// The Enhanced and KR light sprites are not here: their textures are `TerrainTexture.uop` entries named by the
/// tileart, packed in `tex_aux_ec.uddp`, and their entry numbers are not light ids.
pub fn convert_client_lights_to_world_lights_uddp(
    cc_path: &Path,
    out_file: &Path,
    options: &WorldLightsOptions,
) -> eyre::Result<()> {
    info!("Converting World Lights to {}", out_file.display());

    let cc_source_dirs = [cc_path.to_path_buf()];
    let no_ec_path: Option<&Path> = None;
    let light_map = if let Some(verdata) =
        load_verdata_if_enabled(&cc_source_dirs, &options.classic_patches)?
    {
        LightMap::load_with_verdata(Some(cc_path), no_ec_path, verdata)
    } else {
        LightMap::load(Some(cc_path), no_ec_path)
    }
    .wrap_err("Failed to load client lights")?;
    eyre::ensure!(light_map.element_count() > 0, "no light.mul lights in {}", cc_path.display());

    let mut slots = Vec::new();
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    let mut added_count = 0;

    for id in 0..WORLD_LIGHT_SLOT_COUNT {
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

    build_and_write_package(package, out_file)?;

    Ok(())
}

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
