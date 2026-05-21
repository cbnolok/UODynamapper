use std::io::Write;
use std::path::PathBuf;

use uocf::classic::gump::*;
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::package::UopPackage;

fn rgba_at(pixel_data: &[u8], x: usize, y: usize, width: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    [
        pixel_data[offset],
        pixel_data[offset + 1],
        pixel_data[offset + 2],
        pixel_data[offset + 3],
    ]
}

fn sample_gump_payload() -> Vec<u8> {
    vec![
        2, 0, 0, 0, // row 0 RLE starts after the two-entry lookup table
        5, 0, 0, 0, // row 1 RLE starts after row 0's three pairs
        0x00, 0x7C, // red
        1, 0,
        0, 0, // transparent
        2, 0,
        0xE0, 0x03, // green
        1, 0,
        0x1F, 0x00, // blue
        4, 0,
    ]
}

#[test]
fn gump_decode_preserves_runs_and_transparency() {
    let pixel_data = decode_gump_from_raw(&sample_gump_payload(), 4, 2).unwrap();

    assert_eq!(rgba_at(&pixel_data, 0, 0, 4), [248, 0, 0, 255]);
    assert_eq!(rgba_at(&pixel_data, 1, 0, 4), [0, 0, 0, 0]);
    assert_eq!(rgba_at(&pixel_data, 2, 0, 4), [0, 0, 0, 0]);
    assert_eq!(rgba_at(&pixel_data, 3, 0, 4), [0, 248, 0, 255]);
    assert_eq!(rgba_at(&pixel_data, 0, 1, 4), [0, 0, 248, 255]);
    assert_eq!(rgba_at(&pixel_data, 3, 1, 4), [0, 0, 248, 255]);
}

#[test]
fn gump_decode_rejects_runs_past_row_width() {
    let raw_data = vec![
        1, 0, 0, 0, // row 0 RLE starts after lookup table
        0x00, 0x7C,
        5, 0,
    ];

    assert!(decode_gump_from_raw(&raw_data, 4, 1).is_err());
}

#[test]
fn gump_map_reads_dimensions_from_legacy_uop_header() {
    let mut uop_payload = Vec::new();
    uop_payload.write_all(&4u32.to_le_bytes()).unwrap();
    uop_payload.write_all(&2u32.to_le_bytes()).unwrap();
    uop_payload.write_all(&sample_gump_payload()).unwrap();

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            &uop_payload,
            "build/gumpartlegacymul/00001283.tga",
            CompressionFlag::None,
        )
        .unwrap();

    let map = GumpMap::load_standalone_uop(package);
    let mut scratch = Vec::new();
    let (width, height, pixel_data) = map.decode_gump(1283, &mut scratch).unwrap();

    assert_eq!((width, height), (4, 2));
    assert_eq!(scratch, sample_gump_payload());
    assert_eq!(rgba_at(&pixel_data, 3, 0, width as usize), [0, 248, 0, 255]);
}

#[test]
fn gump_map_reads_classic_mul_pair() {
    let unique = format!(
        "uocf_gump_test_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();

    let result = write_classic_pair_and_read(&dir);
    let _ = std::fs::remove_file(dir.join("gumpidx.mul"));
    let _ = std::fs::remove_file(dir.join("gumpart.mul"));
    let _ = std::fs::remove_dir(&dir);

    result.unwrap();
}

fn write_classic_pair_and_read(dir: &PathBuf) -> std::io::Result<()> {
    let payload = sample_gump_payload();
    let mut idx = std::fs::File::create(dir.join("gumpidx.mul"))?;
    idx.write_all(&0u32.to_le_bytes())?;
    idx.write_all(&(payload.len() as u32).to_le_bytes())?;
    idx.write_all(&((4u32 << 16) | 2u32).to_le_bytes())?;
    std::fs::write(dir.join("gumpart.mul"), payload)?;

    let map = GumpMap::load(dir).unwrap();
    let mut scratch = Vec::new();
    let (width, height, pixel_data) = map.decode_gump(0, &mut scratch).unwrap();

    assert_eq!((width, height), (4, 2));
    assert_eq!(rgba_at(&pixel_data, 0, 0, width as usize), [248, 0, 0, 255]);

    Ok(())
}
