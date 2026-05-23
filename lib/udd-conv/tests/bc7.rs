use std::sync::Arc;
use udd_conv::bc7::*;
use wgpu_types::TextureFormat;

fn patterned_rgba(extent: ImageExtent) -> Vec<u8> {
    let mut rgba = vec![0u8; extent.byte_len(RawImageFormat::Rgba8888)];
    for y in 0..extent.height() {
        for x in 0..extent.width() {
            let block_index = (y / 4) * extent.blocks_wide() + (x / 4);
            let local_x = (x % 4) as u8 * 50;
            let local_y = (y % 4) as u8 * 50;
            let offset = block_index as u8 * 2;
            let pixel_offset = ((y * extent.width() + x) * 4) as usize;
            let rgba_px = if block_index % 2 == 0 {
                [
                    local_x.saturating_add(offset),
                    local_y.saturating_add(offset),
                    128,
                    255,
                ]
            } else {
                [
                    local_y.saturating_add(offset),
                    local_x.saturating_add(offset),
                    64,
                    255,
                ]
            };
            rgba[pixel_offset..pixel_offset + 4].copy_from_slice(&rgba_px);
        }
    }
    rgba
}

fn rgba_mse(source: &[u8], decoded: &[u8]) -> f32 {
    assert_eq!(source.len(), decoded.len());
    let sse = source
        .iter()
        .zip(decoded)
        .map(|(src, dst)| {
            let diff = *src as f32 - *dst as f32;
            diff * diff
        })
        .sum::<f32>();
    sse / source.len() as f32
}

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
    let bc7 = encode_to_bc7(
        &rgb,
        extent,
        RawImageFormat::Rgb888,
        Bc7EncoderBackend::Analytical,
    )
    .unwrap();
    let decoded = decode_bc7_to_rgb888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert_eq!(decoded.len(), rgb.len());
}

#[test]
fn rgba8888_roundtrip_preserves_sizes() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgba = vec![255u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let bc7 = encode_to_bc7(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
    )
    .unwrap();
    let decoded = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert_eq!(decoded.len(), rgba.len());
}

#[test]
fn vram_texture_container_roundtrip_preserves_bc7_metadata() {
    let extent = ImageExtent::new(8, 4).unwrap();
    let rgba = vec![128u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let texture = encode_for_vram(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        VramTextureEncoding::Bc7(Bc7EncoderBackend::Analytical),
    )
    .unwrap();

    let container = texture.to_container_bytes();
    let decoded = VramTextureData::from_container_bytes(&container).unwrap();

    assert_eq!(decoded.extent(), extent);
    assert_eq!(decoded.format(), VramTextureFormat::Bc7RgbaUnormSrgb);
    assert_eq!(decoded.bytes().len(), expected_bc7_byte_len(extent));
}

#[test]
fn analytical_bc7_with_rdo_lambda_preserves_block_layout() {
    let extent = ImageExtent::new(8, 4).unwrap();
    let rgba = vec![128u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let bc7 = encode_to_bc7_with_rdo_lambda(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
        DEFAULT_BC7_RDO_LAMBDA,
    )
    .unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
}

#[test]
fn analytical_bc7_decodes_with_bounded_error() {
    let extent = ImageExtent::new(16, 16).unwrap();
    let rgba = patterned_rgba(extent);
    let bc7 = encode_to_bc7(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
    )
    .unwrap();
    let decoded = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert!(rgba_mse(&rgba, &decoded) < 150.0);
}

#[test]
fn analytical_bc7_default_rdo_decodes_with_bounded_error() {
    let extent = ImageExtent::new(16, 16).unwrap();
    let rgba = patterned_rgba(extent);
    let bc7 = encode_to_bc7_with_rdo_lambda(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
        DEFAULT_BC7_RDO_LAMBDA,
    )
    .unwrap();
    let decoded = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();

    assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
    assert!(rgba_mse(&rgba, &decoded) < 150.0);
}

#[test]
fn analytical_bc7_high_rdo_can_modify_blocks() {
    let extent = ImageExtent::new(16, 16).unwrap();
    let rgba = patterned_rgba(extent);
    let plain = encode_to_bc7_with_rdo_lambda(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
        0.0,
    )
    .unwrap();
    let rdo = encode_to_bc7_with_rdo_lambda(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
        100.0,
    )
    .unwrap();
    let decoded = decode_bc7_to_rgba8888(rdo.blocks(), extent).unwrap();

    assert_ne!(plain.blocks(), rdo.blocks());
    assert!(rgba_mse(&rgba, &decoded) < 250.0);
}

#[test]
fn project_bc7_backend_is_available() {
    assert!(Bc7EncoderBackend::Analytical.is_available());
    assert_eq!(
        resolve_bc7_encoder_backend(Bc7EncoderBackend::Analytical),
        Bc7EncoderBackend::Analytical
    );
}

#[test]
fn encode_to_bc7_uses_analytical_backend() {
    let extent = ImageExtent::new(4, 4).unwrap();
    let rgba = vec![7u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let bc7 = encode_to_bc7(
        &rgba,
        extent,
        RawImageFormat::Rgba8888,
        Bc7EncoderBackend::Analytical,
    )
    .unwrap();

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
