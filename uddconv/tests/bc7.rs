use std::sync::Arc;
use uddconv::bc7::*;
use wgpu_types::TextureFormat;

#[test]
fn bc7_upload_layout_matches_block_math() {
    let extent = ImageExtent::new(5, 7).unwrap();
    let encoded = Bc7TextureData::new(extent, vec![0u8; expected_bc7_byte_len(extent)]).unwrap();
    let layout = encoded.upload_layout();

    assert_eq!(layout.bytes_per_row, 32);
    assert_eq!(layout.rows_per_image, 2);
    assert_eq!(layout.format, TextureFormat::Bc7RgbaUnormSrgb);
}

#[test]
fn rgb888_roundtrip_preserves_sizes() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgb = vec![96u8; extent.byte_len(RawImageFormat::Rgb888)];
    let backend = resolve_bc7_encoder_backend(Bc7EncoderBackend::Dds);
    let bc7 = encode_to_bc7(&rgb, extent, RawImageFormat::Rgb888, backend).unwrap();
    let decoded = decode_bc7_to_rgb888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert_eq!(decoded.len(), rgb.len());
}

#[test]
fn rgba8888_roundtrip_preserves_sizes() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgba = vec![255u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let backend = resolve_bc7_encoder_backend(Bc7EncoderBackend::Dds);
    let bc7 = encode_to_bc7(&rgba, extent, RawImageFormat::Rgba8888, backend).unwrap();
    let decoded = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert_eq!(decoded.len(), rgba.len());
}

#[test]
fn vram_texture_container_roundtrip_preserves_bc7_metadata() {
    let extent = ImageExtent::new(8, 4).unwrap();
    let rgba = vec![128u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let backend = resolve_bc7_encoder_backend(Bc7EncoderBackend::Dds);
    let texture = encode_for_vram(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        VramTextureEncoding::Bc7(backend),
    )
    .unwrap();

    let container = texture.to_container_bytes();
    let decoded = VramTextureData::from_container_bytes(&container).unwrap();

    assert_eq!(decoded.extent(), extent);
    assert_eq!(decoded.format(), VramTextureFormat::Bc7RgbaUnormSrgb);
    assert_eq!(decoded.bytes().len(), expected_bc7_byte_len(extent));
}

#[test]
fn unavailable_backends_resolve_to_a_supported_backend_when_possible() {
    assert_eq!(
        resolve_bc7_encoder_backend(Bc7EncoderBackend::BlockCompression),
        if cfg!(feature = "block_compression") {
            Bc7EncoderBackend::BlockCompression
        } else if Bc7EncoderBackend::Dds.is_available() {
            Bc7EncoderBackend::Dds
        } else if cfg!(feature = "ispc") {
            Bc7EncoderBackend::Ispc
        } else {
            Bc7EncoderBackend::BlockCompression
        }
    );
    assert_eq!(
        resolve_bc7_encoder_backend(Bc7EncoderBackend::Ispc),
        if cfg!(feature = "ispc") {
            Bc7EncoderBackend::Ispc
        } else if Bc7EncoderBackend::Dds.is_available() {
            Bc7EncoderBackend::Dds
        } else if cfg!(feature = "block_compression") {
            Bc7EncoderBackend::BlockCompression
        } else {
            Bc7EncoderBackend::Ispc
        }
    );
}

#[test]
fn encode_to_bc7_resolves_unavailable_backends_before_encoding() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgba = vec![7u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let requested_backend = if cfg!(feature = "block_compression") {
        Bc7EncoderBackend::BlockCompression
    } else {
        Bc7EncoderBackend::Ispc
    };

    let bc7 = encode_to_bc7(&rgba, extent, RawImageFormat::Rgba8888, requested_backend).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
}

#[test]
fn encode_for_vram_arc_reuses_rgba_input_for_uncompressed_uploads() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgba = Arc::<[u8]>::from(vec![42u8; extent.byte_len(RawImageFormat::Rgba8888)]);
    let texture = encode_for_vram_arc(
        Arc::clone(&rgba),
        extent,
        RawImageFormat::Rgba8888,
        VramTextureEncoding::Rgba8UnormSrgb,
    )
    .unwrap();

    assert!(Arc::ptr_eq(&texture.bytes_arc(), &rgba));
    assert_eq!(
        texture.upload_layout(),
        TextureUploadLayout {
            bytes_per_row: 16,
            rows_per_image: 4,
            format: TextureFormat::Rgba8UnormSrgb,
        }
    );
}
