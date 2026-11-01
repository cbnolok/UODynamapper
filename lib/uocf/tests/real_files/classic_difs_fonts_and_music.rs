//! The map and static difs, `fonts.mul`, `unifont*.mul` and `Music/Digital/Config.txt` of real clients of every
//! era read strictly and write back byte for byte; an edit changes only itself.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::dif_files::{MapDifFile, StaticDifFile};
use uocf::classic::facet_sources::{MapDifFiles, StaticDifFiles};
use uocf::classic::fonts::{
    ASCII_GLYPH_COUNT, AsciiFontsFile, UNICODE_FONT_FILES_MAX, UnicodeFontData, UnicodeFontGlyph, UnicodeFontSlot, unicode_font_file_name,
};
use uocf::classic::statics_write::StaticRecord;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The facets whose difs a client may ship.
const DIF_FACETS: u32 = 5;
/// `fonts.mul`.
const ASCII_FONTS_FILE: &str = "fonts.mul";
/// The code point the Unicode edit replaces.
const EDITED_CODEPOINT: u16 = 'A' as u16;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn read_all<const N: usize>(root: &Path, names: [String; N]) -> Option<[Vec<u8>; N]> {
    let paths: Vec<PathBuf> = names.iter().map(|name| find_file_ignoring_case(root, name)).collect::<Option<_>>()?;
    Some(std::array::from_fn(|position| std::fs::read(&paths[position]).unwrap()))
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_map_and_static_difs_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        for facet in 0..DIF_FACETS {
            if let Some([list, blocks]) = read_all(&root, MapDifFiles::file_names(facet)) {
                let mut file = MapDifFile::from_bytes(&list, &blocks).unwrap_or_else(|error| panic!("{client} mapdif{facet}: {error:#}"));
                assert_eq!(file.to_bytes().unwrap(), (list.clone(), blocks.clone()), "{client} mapdif{facet}");
                println!("{client}: mapdifl{facet}/mapdif{facet} ({} listings) written back byte for byte", file.entries.len());

                let (id, mut block) = file.entries[0].clone();
                block.cells[0].1 = block.cells[0].1.wrapping_add(1);
                file.set_block(id, block.clone()).unwrap();
                let (edited_list, edited_blocks) = file.to_bytes().unwrap();
                assert_eq!(edited_list, list, "{client} mapdifl{facet}: an edited block keeps the list");
                assert_eq!(MapDifFile::from_bytes(&edited_list, &edited_blocks).unwrap().block(id), Some(&block));
                let changed = blocks.iter().zip(&edited_blocks).filter(|(a, b)| a != b).count();
                assert_eq!(changed, 1, "{client} mapdif{facet}: one z byte changed");
                checked += 1;
            }
            if let Some([list, index, statics]) = read_all(&root, StaticDifFiles::file_names(facet)) {
                let mut file = StaticDifFile::from_bytes(&list, &index, &statics)
                    .unwrap_or_else(|error| panic!("{client} stadif{facet}: {error:#}"));
                assert_eq!(file.to_bytes().unwrap(), (list.clone(), index.clone(), statics.clone()), "{client} stadif{facet}");
                println!("{client}: stadifl{facet}/stadifi{facet}/stadif{facet} ({} listings) written back byte for byte", file.block_ids.len());

                let id = *file.block_ids.last().unwrap();
                let mut records = file.block(id).unwrap();
                records.push(StaticRecord { graphic: 1, x: 0, y: 0, z: 0, hue: 0 });
                file.set_block(id, &records);
                let (edited_list, edited_index, edited_statics) = file.to_bytes().unwrap();
                assert_eq!(edited_list, list, "{client} stadifl{facet}: an edited block keeps the list");
                let back = StaticDifFile::from_bytes(&edited_list, &edited_index, &edited_statics).unwrap();
                assert_eq!(back.block(id), Some(records), "{client} stadif{facet}: the edited block");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships a dif");
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_fonts_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        if let Some([bytes]) = read_all(&root, [ASCII_FONTS_FILE.to_owned()]) {
            let mut file = AsciiFontsFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {ASCII_FONTS_FILE}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {ASCII_FONTS_FILE}");
            assert!(file.fonts.iter().all(|font| font.glyphs.len() == ASCII_GLYPH_COUNT));
            println!(
                "{client}: {ASCII_FONTS_FILE} ({} faces, {} bytes after them) written back byte for byte",
                file.fonts.len(),
                file.tail.len()
            );
            let glyph = &mut file.fonts[0].glyphs[0];
            glyph.pixels.extend(std::iter::repeat_n(0, glyph.height as usize));
            glyph.width += 1;
            let edited = file.to_bytes().unwrap();
            assert_eq!(AsciiFontsFile::from_bytes(&edited).unwrap(), file, "{client} {ASCII_FONTS_FILE}: the widened glyph");
            checked += 1;
        }
        for index in 0..UNICODE_FONT_FILES_MAX {
            let name = unicode_font_file_name(index);
            let Some([bytes]) = read_all(&root, [name.clone()]) else { continue };
            let mut font = UnicodeFontData::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {name}: {error:#}"));
            assert_eq!(font.to_bytes().unwrap(), bytes, "{client} {name}");
            let glyphs = (0..=u16::MAX).filter(|&cp| matches!(font.slot(cp), UnicodeFontSlot::Glyph { .. })).count();
            println!("{client}: {name} ({glyphs} glyphs) written back byte for byte");

            let before = font.clone();
            let glyph = UnicodeFontGlyph { offset_x: 1, offset_y: 2, width: 9, height: 2, bitmap: vec![0xFF, 0x80, 0x00, 0x80] };
            font.set_glyph(EDITED_CODEPOINT, Some(glyph.clone())).unwrap();
            let back = UnicodeFontData::from_bytes(&font.to_bytes().unwrap()).unwrap();
            assert_eq!(back.glyph(EDITED_CODEPOINT), Some(&glyph), "{client} {name}: the edited glyph");
            for cp in (0..=u16::MAX).filter(|&cp| cp != EDITED_CODEPOINT) {
                assert_eq!(back.slot(cp), before.slot(cp), "{client} {name}: U+{cp:04X}");
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships a font");
}
