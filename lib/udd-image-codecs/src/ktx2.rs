use std::path::Path;
use std::fs::File;
use std::sync::{Arc, Mutex};

use color_eyre::eyre::{self, Context};
use libktx_rs::enums::{CreateStorage, TextureCreateFlags};
use libktx_rs::sinks::StreamSink;
use libktx_rs::sources::{CommonCreateInfo, Ktx2CreateInfo, StreamSource};
use libktx_rs::stream::RustKtxStream;
use libktx_rs::texture::Texture;

use crate::bc7::{decode_bc7_to_rgba8888, Bc7TextureData, ImageExtent};

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

pub fn read_ktx2_bc7_zstd(path: &Path) -> eyre::Result<Bc7TextureData> {
    let input_file = File::open(path)
        .wrap_err_with(|| format!("Failed to open KTX2 file: {:?}", path))?;
    let stream = Arc::new(Mutex::new(
        RustKtxStream::new(Box::new(input_file))
            .map_err(|error| eyre::eyre!("libktx stream error: {:?}", error))?,
    ));
    let source = StreamSource::new(stream, TextureCreateFlags::LOAD_IMAGE_DATA);
    let mut texture =
        Texture::new(source).map_err(|error| eyre::eyre!("libktx read error: {:?}", error))?;

    let (vk_format, width, height) = {
        let ktx2 = texture
            .ktx2()
            .ok_or_else(|| eyre::eyre!("KTX file is not KTX2: {:?}", path))?;
        let vk_format = ktx2.vk_format();
        let handle = ktx2.handle();
        // SAFETY: `ktx2.handle()` is owned by libktx for this live texture.
        let width = unsafe { (*handle).baseWidth };
        let height = unsafe { (*handle).baseHeight };
        (vk_format, width, height)
    };

    if vk_format != VK_FORMAT_BC7_UNORM_BLOCK {
        eyre::bail!(
            "KTX2 texture {:?} has Vulkan format {}, expected BC7 UNORM ({})",
            path,
            vk_format,
            VK_FORMAT_BC7_UNORM_BLOCK
        );
    }

    let offset = texture
        .get_image_offset(0, 0, 0)
        .map_err(|error| eyre::eyre!("libktx offset error: {:?}", error))?;
    let image_size = texture
        .get_image_size(0)
        .map_err(|error| eyre::eyre!("libktx image size error: {:?}", error))?;
    let data = texture.data();
    let end = offset
        .checked_add(image_size)
        .ok_or_else(|| eyre::eyre!("KTX2 image byte range overflows"))?;
    if end > data.len() {
        eyre::bail!(
            "KTX2 image byte range {}..{} exceeds loaded data length {}",
            offset,
            end,
            data.len()
        );
    }

    let extent = ImageExtent::new(width, height)
        .map_err(|error| eyre::eyre!("invalid KTX2 extent: {}", error))?;
    Bc7TextureData::new(extent, data[offset..end].to_vec())
        .map_err(|error| eyre::eyre!("invalid KTX2 BC7 payload: {}", error))
}

pub fn decode_ktx2_bc7_zstd_to_rgba8888(path: &Path) -> eyre::Result<(u32, u32, Vec<u8>)> {
    let bc7_data = read_ktx2_bc7_zstd(path)?;
    let extent = bc7_data.extent();
    let rgba = decode_bc7_to_rgba8888(bc7_data.blocks(), extent)
        .map_err(|error| eyre::eyre!("failed decoding KTX2 BC7 payload: {}", error))?;
    Ok((extent.width(), extent.height(), rgba))
}
