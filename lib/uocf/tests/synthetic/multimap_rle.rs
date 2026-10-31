use uocf::classic::multimap_rle::{
    decode_rle_bytes, encode_rle_bytes, MultimapRleImage, BLACK_PIXEL, WHITE_PIXEL,
};

#[test]
fn multimap_rle_decodes_reference_run_format() {
    let bytes = [
        4, 0, 0, 0,
        2, 0, 0, 0,
        3, 0x82, 2, 0x81,
    ];

    let image = decode_rle_bytes(&bytes).expect("decode multimap");

    assert_eq!(image.width, 4);
    assert_eq!(image.height, 2);
    assert_eq!(
        image.pixels,
        vec![
            WHITE_PIXEL, WHITE_PIXEL, WHITE_PIXEL, BLACK_PIXEL,
            BLACK_PIXEL, WHITE_PIXEL, WHITE_PIXEL, BLACK_PIXEL,
        ]
    );
}

#[test]
fn multimap_rle_roundtrips_long_runs() {
    let image = MultimapRleImage::new(
        130,
        1,
        vec![WHITE_PIXEL; 127]
            .into_iter()
            .chain(vec![BLACK_PIXEL; 3])
            .collect(),
    )
    .expect("image");

    let encoded = encode_rle_bytes(&image);
    assert_eq!(&encoded[8..], &[0x7f, 0x83]);

    let decoded = decode_rle_bytes(&encoded).expect("decode encoded multimap");
    assert_eq!(decoded, image);
}

#[test]
fn multimap_rle_rejects_short_pixel_data() {
    let bytes = [
        4, 0, 0, 0,
        2, 0, 0, 0,
        7,
    ];

    let err = decode_rle_bytes(&bytes).expect_err("short run data should fail");
    assert!(err.to_string().contains("decoded 7 pixels"));
}

#[test]
fn multimap_rgba_conversion_classifies_to_black_and_white() {
    let rgba = [
        255, 255, 255, 255,
        0, 0, 0, 255,
        240, 240, 240, 64,
        100, 100, 100, 255,
    ];

    let image = MultimapRleImage::from_rgba8(2, 2, &rgba).expect("rgba image");

    assert_eq!(
        image.pixels,
        vec![WHITE_PIXEL, BLACK_PIXEL, WHITE_PIXEL, BLACK_PIXEL]
    );
    assert_eq!(
        image.to_rgba8(),
        vec![
            255, 255, 255, 255,
            0, 0, 0, 255,
            255, 255, 255, 255,
            0, 0, 0, 255,
        ]
    );
}
