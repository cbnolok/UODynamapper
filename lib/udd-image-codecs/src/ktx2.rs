use std::path::Path;
use std::sync::{Arc, Mutex};

use color_eyre::eyre::{self, Context};
use libktx_rs::enums::CreateStorage;
use libktx_rs::sinks::StreamSink;
use libktx_rs::sources::{CommonCreateInfo, Ktx2CreateInfo};
use libktx_rs::stream::RustKtxStream;
use libktx_rs::texture::Texture;

use crate::bc7::Bc7TextureData;

const VK_FORMAT_BC7_UNORM_BLOCK: u32 = 145;

pub fn write_ktx2_bc7_zstd(
    bc7_data: Bc7TextureData,
    output_path: &Path,
    zstd_level: i32,
) -> eyre::Result<()> {
    let extent = bc7_data.extent();
    println!("Packing into KTX2 (BC7 + Zstd level {})...", zstd_level);
    let dfd = vk2dfd::vk2dfd(VK_FORMAT_BC7_UNORM_BLOCK)
        .map_err(|error| eyre::eyre!("vk2dfd error: {:?}", error))?;
    let info = Ktx2CreateInfo {
        vk_format: VK_FORMAT_BC7_UNORM_BLOCK,
        dfd: Some(dfd.to_vec()),
        common: CommonCreateInfo {
            create_storage: CreateStorage::AllocStorage,
            base_width: extent.width(),
            base_height: extent.height(),
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            is_array: false,
            generate_mipmaps: false,
        },
    };

    let mut texture =
        Texture::new(info).map_err(|error| eyre::eyre!("libktx error: {:?}", error))?;

    let offset = texture
        .get_image_offset(0, 0, 0)
        .map_err(|error| eyre::eyre!("libktx offset error: {:?}", error))?;

    let blocks = bc7_data.into_blocks();
    let data = texture.data_mut();
    let end = offset + blocks.len();
    if end > data.len() {
        return Err(eyre::eyre!(
            "libktx allocated {} bytes for radar texture, but BC7 payload needs {} bytes (offset {})",
            data.len(),
            blocks.len(),
            offset
        ));
    }
    data[offset..end].copy_from_slice(&blocks);

    if zstd_level > 0 {
        if let Some(mut ktx2) = texture.ktx2() {
            ktx2.deflate_zstd(zstd_level as u32)
                .map_err(|error| eyre::eyre!("libktx zstd error: {:?}", error))?;
        }
    }

    let output_file = std::fs::File::create(output_path)
        .wrap_err_with(|| format!("Failed to create output file: {:?}", output_path))?;

    let stream = Arc::new(Mutex::new(
        RustKtxStream::new(Box::new(output_file))
            .map_err(|error| eyre::eyre!("libktx stream error: {:?}", error))?,
    ));
    let mut sink = StreamSink::new(stream);

    texture
        .write_to(&mut sink)
        .map_err(|error| eyre::eyre!("libktx write error: {:?}", error))?;

    Ok(())
}
