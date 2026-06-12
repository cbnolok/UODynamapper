use std::io::Cursor;
use std::path::Path;

use color_eyre::eyre::{self, Context};

use crate::bc7::{decode_bc7_to_rgba8888, Bc7TextureData, ImageExtent};

pub fn write_ktx2_bc7_zstd(
    bc7_data: Bc7TextureData,
    output_path: &Path,
    zstd_level: i32,
) -> eyre::Result<()> {
    let extent = bc7_data.extent();
    println!("Packing into KTX2 (BC7 + Zstd level {})...", zstd_level);
    let blocks = bc7_data.into_blocks();
    let output = encode_ktx2_bc7(&blocks, extent, zstd_level)?;
    std::fs::write(output_path, output)
        .wrap_err_with(|| format!("Failed to write KTX2 file: {:?}", output_path))?;

    Ok(())
}

pub fn read_ktx2_bc7_zstd(path: &Path) -> eyre::Result<Bc7TextureData> {
    let data = std::fs::read(path)
        .wrap_err_with(|| format!("Failed to read KTX2 file: {:?}", path))?;
    let reader = ktx2::Reader::new(&data)
        .map_err(|error| eyre::eyre!("KTX2 parse error for {:?}: {}", path, error))?;
    let header = reader.header();

    if header.format != Some(ktx2::Format::BC7_UNORM_BLOCK) {
        eyre::bail!(
            "KTX2 texture {:?} has format {:?}, expected BC7 UNORM",
            path,
            header.format
        );
    }

    let level0 = reader
        .levels()
        .next()
        .ok_or_else(|| eyre::eyre!("KTX2 texture {:?} has no level 0 image", path))?;
    let blocks = match header.supercompression_scheme {
        Some(ktx2::SupercompressionScheme::Zstandard) => zstd::decode_all(Cursor::new(level0.data))
            .wrap_err_with(|| format!("Failed to decompress KTX2 Zstd payload: {:?}", path))?,
        Some(scheme) => {
            eyre::bail!("KTX2 texture {:?} uses unsupported supercompression {:?}", path, scheme);
        }
        None => level0.data.to_vec(),
    };

    let extent = ImageExtent::new(header.pixel_width, header.pixel_height)
        .map_err(|error| eyre::eyre!("invalid KTX2 extent: {}", error))?;
    Bc7TextureData::new(extent, blocks)
        .map_err(|error| eyre::eyre!("invalid KTX2 BC7 payload: {}", error))
}

pub fn decode_ktx2_bc7_zstd_to_rgba8888(path: &Path) -> eyre::Result<(u32, u32, Vec<u8>)> {
    let bc7_data = read_ktx2_bc7_zstd(path)?;
    let extent = bc7_data.extent();
    let rgba = decode_bc7_to_rgba8888(bc7_data.blocks(), extent)
        .map_err(|error| eyre::eyre!("failed decoding KTX2 BC7 payload: {}", error))?;
    Ok((extent.width(), extent.height(), rgba))
}

fn encode_ktx2_bc7(
    blocks: &[u8],
    extent: ImageExtent,
    zstd_level: i32,
) -> eyre::Result<Vec<u8>> {
    let (dfd, type_size) = ktx2::dfd::Basic::from_format(ktx2::Format::BC7_UNORM_BLOCK)
        .map_err(|error| eyre::eyre!("failed to build KTX2 DFD: {}", error))?;
    let dfd_block = ktx2::dfd::Block::Basic(dfd).to_vec();
    let dfd_byte_length = 4usize
        .checked_add(dfd_block.len())
        .ok_or_else(|| eyre::eyre!("KTX2 DFD length overflows"))?;
    let mut dfd_section = Vec::with_capacity(dfd_byte_length);
    dfd_section.extend_from_slice(&(dfd_byte_length as u32).to_le_bytes());
    dfd_section.extend_from_slice(&dfd_block);

    let supercompression_scheme = if zstd_level > 0 {
        Some(ktx2::SupercompressionScheme::Zstandard)
    } else {
        None
    };
    let level_data = if zstd_level > 0 {
        zstd::encode_all(Cursor::new(blocks), zstd_level)
            .map_err(|error| eyre::eyre!("failed to compress KTX2 BC7 payload: {}", error))?
    } else {
        blocks.to_vec()
    };

    let dfd_byte_offset = ktx2::Header::LENGTH + ktx2::LevelIndex::LENGTH;
    let level_byte_offset = align_up(dfd_byte_offset + dfd_section.len(), 8)
        .ok_or_else(|| eyre::eyre!("KTX2 level offset overflows"))?;
    let header = ktx2::Header {
        format: Some(ktx2::Format::BC7_UNORM_BLOCK),
        type_size,
        pixel_width: extent.width(),
        pixel_height: extent.height(),
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: 1,
        supercompression_scheme,
        index: ktx2::Index {
            dfd_byte_offset: dfd_byte_offset as u32,
            dfd_byte_length: dfd_section.len() as u32,
            kvd_byte_offset: 0,
            kvd_byte_length: 0,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    let level = ktx2::LevelIndex {
        byte_offset: level_byte_offset as u64,
        byte_length: level_data.len() as u64,
        uncompressed_byte_length: blocks.len() as u64,
    };

    let mut output = Vec::with_capacity(level_byte_offset + level_data.len());
    output.extend_from_slice(&header.as_bytes());
    output.extend_from_slice(&level.as_bytes());
    output.extend_from_slice(&dfd_section);
    output.resize(level_byte_offset, 0);
    output.extend_from_slice(&level_data);

    Ok(output)
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment.checked_sub(1)?)
        .map(|value| value / alignment * alignment)
}
