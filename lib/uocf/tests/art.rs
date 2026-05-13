use uocf::classic::art::*;

fn rgba_at(pixel_data: &[u8], x: usize, y: usize, width: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    [
        pixel_data[offset],
        pixel_data[offset + 1],
        pixel_data[offset + 2],
        pixel_data[offset + 3],
    ]
}

#[test]
fn land_decode_clears_transparent_corners() {
    let raw_data = vec![0xFFu8; LAND_DIAMOND_PIXEL_COUNT * 2];
    let mut pixel_data = [0x7Fu8; LAND_DIMENSION * LAND_DIMENSION * 4];

    decode_land_tile_from_raw(&raw_data, &mut pixel_data).unwrap();

    assert_eq!(rgba_at(&pixel_data, 0, 0, LAND_DIMENSION), [0, 0, 0, 0]);
    assert_eq!(
        rgba_at(&pixel_data, LAND_DIMENSION - 1, 0, LAND_DIMENSION),
        [0, 0, 0, 0]
    );
    assert_eq!(
        rgba_at(
            &pixel_data,
            LAND_DIMENSION / 2,
            LAND_DIMENSION / 2,
            LAND_DIMENSION
        ),
        [248, 248, 248, 255]
    );
}

#[test]
fn static_decode_preserves_rle_positions() {
    let raw_data = vec![
        0, 0, 0, 0, // flags
        4, 0, // width
        1, 0, // height
        0, 0, // lookup for row 0
        1, 0, // x_offset
        2, 0, // x_run
        0x00, 0x7C, // red
        0xE0, 0x03, // green
        0, 0, // row terminator
        0, 0,
    ];

    let (width, height, pixel_data) = decode_static_tile_from_raw(&raw_data).unwrap();

    assert_eq!((width, height), (4, 1));
    assert_eq!(rgba_at(&pixel_data, 0, 0, width as usize), [0, 0, 0, 0]);
    assert_eq!(rgba_at(&pixel_data, 1, 0, width as usize), [248, 0, 0, 255]);
    assert_eq!(rgba_at(&pixel_data, 2, 0, width as usize), [0, 248, 0, 255]);
    assert_eq!(rgba_at(&pixel_data, 3, 0, width as usize), [0, 0, 0, 0]);
}
