//! Converts the `hues.mul` and `Hues.uop` of real clients both ways and checks the result against EA's files.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS` and from `UO_HUES_CLIENT_DIRS`, a
//! path list (`:`-separated on Unix) for comparing several releases at once. Each directory may hold a
//! `hues.mul`, a `Hues.uop`, or both. Packages written here go to `std::env::temp_dir()` and are deleted.

use crate::common;

use std::path::{Path, PathBuf};

use uocf::classic::hues::{HuesMul, HUE_COLOR_COUNT};
use uocf::enhanced::hues::{
    atlas_coord_for_hue, decode_hue_image_to_rgba, hue_bitmap_hash, FIXED_PALETTE_HASH, HUES_ATLAS_PATH,
    HUES_ATLAS_WIDTH, HUE_TABLE_ENTRIES, MAX_EC_HUES,
};
use uocf::enhanced::hues_mul_uop::{
    build_hues_uop_entries, decode_argb32_dds, encode_hue_bitmap, expand_hue_color, hue_palette_row,
    hue_swatch_color, hues_mul_from_uop_package, write_hues_uop, HueRampMode, SWATCH_GRID_SIDE,
    TEXELS_PER_HUE_COLOR,
};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

/// Extra client directories, as a path list.
const EXTRA_DIRS_ENV: &str = "UO_HUES_CLIENT_DIRS";
const HUES_MUL_NAME: &str = "hues.mul";
const HUES_UOP_NAME: &str = "hues.uop";
/// A fixed stamp, so the written packages are reproducible.
const TEST_FILETIME: u64 = 0x01DB_0000_0000_0000;
/// Bytes of one `hues.mul` entry.
const HUE_ENTRY_SIZE: usize = 88;
const DDS_HEADER_BYTES: usize = 128;
const RGBA_CHANNELS: usize = 4;

fn client_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        common::CLIENT_DIR_ENVS.into_iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    if let Some(list) = std::env::var_os(EXTRA_DIRS_ENV) {
        dirs.extend(std::env::split_paths(&list));
    }
    assert!(!dirs.is_empty(), "set {} and/or {EXTRA_DIRS_ENV}", common::CLIENT_DIR_ENVS.join(", "));
    dirs.sort();
    dirs.dedup();
    dirs
}

/// The file of `dir` named `name` in any letter case.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_file() && path.file_name().is_some_and(|file| file.eq_ignore_ascii_case(name)))
}

fn sources(name: &str) -> Vec<PathBuf> {
    let found: Vec<_> = client_dirs().iter().filter_map(|dir| find_file(dir, name)).collect();
    println!("{name}: {} source(s)", found.len());
    found
}

fn label(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into())
}

fn temp_path(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("uocf-hues-client-{}-{unique}-{name}", std::process::id()))
}

/// The top bitmap row of hue `id` in an EA package.
fn bitmap_row(package: &UopPackage, id: u16) -> Option<Vec<[u8; 4]>> {
    let bytes = package.unpack_file_by_hash(hue_bitmap_hash(id)).ok()??;
    let (_, _, rgba) = decode_hue_image_to_rgba(&bytes).ok()?;
    Some(rgba[..HUE_TABLE_ENTRIES * RGBA_CHANNELS].chunks_exact(RGBA_CHANNELS).map(|t| [t[0], t[1], t[2], t[3]]).collect())
}

#[test]
#[ignore = "needs client files: set UO_CC_CLIENT_DIR, UO_EC_CLIENT_DIR and/or UO_HUES_CLIENT_DIRS"]
fn classic_hues_mul_round_trips_through_own_uop_in_both_modes() {
    for mul_path in sources(HUES_MUL_NAME) {
        let source = std::fs::read(&mul_path).unwrap();
        let hues = HuesMul::parse(&source);
        for mode in [HueRampMode::Linear, HueRampMode::Flat] {
            let uop_path = temp_path("Hues.uop");
            write_hues_uop(&build_hues_uop_entries(&hues, mode), &uop_path, Some(TEST_FILETIME)).unwrap();
            let package = UopPackage::load_with_mode(&uop_path, LoadMode::Eager).unwrap();
            let recovered = hues_mul_from_uop_package(&package).unwrap();
            std::fs::remove_file(&uop_path).unwrap();
            assert!(recovered.had_metadata);
            assert_eq!(recovered.hues.to_bytes(), source, "{} {mode:?}", label(&mul_path));
            println!(
                "{}: {} hues, {} bytes: {mode:?} round trip byte-identical",
                label(&mul_path),
                hues.hue_count(),
                source.len()
            );
        }
    }
}

#[test]
#[ignore = "needs client files: set UO_CC_CLIENT_DIR, UO_EC_CLIENT_DIR and/or UO_HUES_CLIENT_DIRS"]
fn linear_ramp_reproduces_ea_hue_data_for_unedited_hues() {
    let muls = sources(HUES_MUL_NAME);
    for uop_path in sources(HUES_UOP_NAME) {
        let package = UopPackage::load_with_mode(&uop_path, LoadMode::Eager).unwrap();
        let atlas_bytes = package.unpack_file_by_hash(hash_file_name_single(HUES_ATLAS_PATH)).unwrap().unwrap();
        let swatch_bytes = package.unpack_file_by_hash(FIXED_PALETTE_HASH).unwrap().unwrap();
        let (_, _, atlas) = decode_argb32_dds(&atlas_bytes).unwrap();
        let (_, _, swatches) = decode_argb32_dds(&swatch_bytes).unwrap();
        for mul_path in &muls {
            let hues = HuesMul::load(mul_path).unwrap();
            let ours = build_hues_uop_entries(&hues, HueRampMode::Linear);
            assert_eq!(ours[0].data[..DDS_HEADER_BYTES], swatch_bytes[..DDS_HEADER_BYTES], "swatch DDS header");
            assert_eq!(ours[1].data[..DDS_HEADER_BYTES], atlas_bytes[..DDS_HEADER_BYTES], "atlas DDS header");

            let (mut anchored, mut atlas_exact, mut bitmap_exact, mut swatch_exact, mut flat_exact) = (0, 0, 0, 0, 0);
            for id in 1..=MAX_EC_HUES {
                let (Some(hue), Some(row)) = (hues.hue(u32::from(id)), bitmap_row(&package, id)) else {
                    continue;
                };
                let anchors_match = (0..HUE_COLOR_COUNT)
                    .all(|index| row[index * TEXELS_PER_HUE_COLOR] == expand_hue_color(hue.color_table[index]));
                if !anchors_match {
                    continue;
                }
                anchored += 1;
                let linear = hue_palette_row(&hue.color_table, HueRampMode::Linear);
                let coord = atlas_coord_for_hue(id).unwrap();
                let start = coord.y as usize * HUES_ATLAS_WIDTH as usize + coord.x as usize;
                atlas_exact += usize::from(atlas[start..start + HUE_TABLE_ENTRIES] == linear);
                flat_exact += usize::from(row == hue_palette_row(&hue.color_table, HueRampMode::Flat));
                let ea_bitmap = package.unpack_file_by_hash(hue_bitmap_hash(id)).unwrap().unwrap();
                bitmap_exact += usize::from(ea_bitmap == encode_hue_bitmap(&linear, hue_swatch_color(&hue.color_table)));
                let side = SWATCH_GRID_SIDE as usize;
                let texel = swatches[(usize::from(id) % side) * side + usize::from(id) / side];
                swatch_exact += usize::from(texel == hue_swatch_color(&hue.color_table));
            }
            println!(
                "{} vs {}: {anchored} hues with matching colours; linear: {atlas_exact} atlas rows, {bitmap_exact} \
                 bitmaps byte-identical, {swatch_exact} swatches; flat: {flat_exact} rows",
                label(&uop_path),
                label(mul_path)
            );
            // Swatches and whole bitmaps are reported only: a few hues have hand-picked swatches (two in the KR).
            assert_eq!(atlas_exact, anchored, "every unedited hue's atlas row is reproduced");
        }
    }
}

#[test]
#[ignore = "needs client files: set UO_CC_CLIENT_DIR, UO_EC_CLIENT_DIR and/or UO_HUES_CLIENT_DIRS"]
fn ea_packages_convert_to_hues_mul_best_effort() {
    let muls = sources(HUES_MUL_NAME);
    for uop_path in sources(HUES_UOP_NAME) {
        let package = UopPackage::load_with_mode(&uop_path, LoadMode::Eager).unwrap();
        let recovered = hues_mul_from_uop_package(&package).unwrap();
        assert!(!recovered.had_metadata);
        println!(
            "{}: {} hues recovered, {} from the atlas, {} without colours",
            label(&uop_path),
            recovered.hues.hue_count(),
            recovered.colors_from_atlas.len(),
            recovered.missing_colors.len()
        );
        let ours = recovered.hues.to_bytes();
        for mul_path in &muls {
            let classic_bytes = std::fs::read(mul_path).unwrap();
            let classic = HuesMul::parse(&classic_bytes);
            let (mut colors, mut tables, mut names, mut entries, mut compared) = (0, 0, 0, 0, 0);
            for (ours, theirs) in recovered.hues.hues().zip(classic.hues()) {
                compared += 1;
                colors += usize::from(ours.color_table == theirs.color_table);
                tables += usize::from((ours.table_start, ours.table_end) == (theirs.table_start, theirs.table_end));
                names += usize::from(ours.name == theirs.name);
                entries += usize::from(ours == theirs);
            }
            let same_bytes = ours.iter().zip(&classic_bytes).filter(|(a, b)| a == b).count();
            println!(
                "  vs {}: of {compared} hues, colours exact {colors}, table start/end {tables}, names {names}, whole \
                 {HUE_ENTRY_SIZE}-byte entries {entries}; file {} / {} bytes equal{}",
                label(mul_path),
                same_bytes,
                classic_bytes.len(),
                if ours == classic_bytes { " (identical)" } else { "" }
            );
        }
    }
}
