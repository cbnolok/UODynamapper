use super::*;

#[test]
fn official_archive_name_and_lazy_names_survive_full_conversion() {
    use uocf::enhanced::hues::{hue_bitmap_path, HUENAMES_PATH};
    use uocf::uop_container::file::CompressionFlag as UopCompression;
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir().join(format!("udd-ec-hues-{}-{unique}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    // The top row is a gradient; the bottom row is a solid UI preview, as in EC.
    let mut bitmap = vec![0u8; 70];
    bitmap[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, 70u32), (10, 54), (14, 40), (18, 2), (22, 2), (34, 16)] {
        bitmap[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&32u16.to_le_bytes());
    // BMP stores bottom-up BGRA rows.
    bitmap[54..].copy_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255,
        0, 0, 255, 255, 0, 255, 0, 255]);
    let mut archive = UopPackage::new_default();
    archive.add_file_from_memory(b"1,masked flowers\n", HUENAMES_PATH, UopCompression::Zlib).unwrap();
    archive.add_file_from_memory(&bitmap, &hue_bitmap_path(1), UopCompression::Zlib).unwrap();
    archive.finalize_and_save(directory.join("Hues.uop")).unwrap();
    let output = directory.join("hues.uddp");
    convert_hues_to_hues_uddp_from_sources(&[directory.clone()], &output,
        &HuesOptions { compression: CompressionFlag::None, ..Default::default() }).unwrap();
    let package = udd_assets::hues::HuesPackage::load(&output).unwrap();
    assert_eq!(package.hue_name(1), Some("masked flowers"));
    let texture = package.read_texture_bytes().unwrap();
    let start = (HUES_TEXTURE_WIDTH * 4) as usize;
    assert_eq!(&texture[start..start + 4], &[255, 0, 0, 255]);
    let end = start + (HUE_STRIP_WIDTH as usize - 1) * 4;
    assert_eq!(&texture[end..end + 4], &[0, 255, 0, 255]);
    drop(package);
    std::fs::remove_dir_all(directory).unwrap();
}
