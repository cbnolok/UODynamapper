use std::io::{Seek, SeekFrom, Write};

use uocf::classic::fonts::*;

fn push_ascii_glyph(bytes: &mut Vec<u8>, width: u8, height: u8, unknown: u8, pixels: &[u16]) {
    bytes.push(width);
    bytes.push(height);
    bytes.push(unknown);
    for pixel in pixels {
        bytes.extend_from_slice(&pixel.to_le_bytes());
    }
}

fn sample_ascii_font() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0x42);
    push_ascii_glyph(&mut bytes, 1, 1, 0, &[0x8000]);

    for index in 1..ASCII_GLYPH_COUNT {
        if index == ascii_index_for_char('A') {
            push_ascii_glyph(&mut bytes, 2, 1, 7, &[0x1111, 0x2222]);
        } else {
            push_ascii_glyph(&mut bytes, 0, 0, 0, &[]);
        }
    }

    bytes
}

#[test]
fn ascii_fonts_mul_decodes_complete_faces() {
    let fonts = decode_ascii_fonts(&sample_ascii_font()).unwrap();

    assert_eq!(fonts.len(), 1);
    assert_eq!(fonts[0].header, 0x42);
    assert_eq!(fonts[0].glyphs.len(), ASCII_GLYPH_COUNT);

    let space = &fonts[0].glyphs[0];
    assert_eq!((space.width, space.height, space.pixels.as_slice()), (1, 1, [0x8000].as_slice()));

    let glyph_a = &fonts[0].glyphs[ascii_index_for_char('A')];
    assert_eq!((glyph_a.width, glyph_a.height, glyph_a.unknown), (2, 1, 7));
    assert_eq!(glyph_a.pixels, [0x1111, 0x2222]);
}

#[test]
fn ascii_fonts_mul_ignores_trailing_incomplete_face() {
    let mut bytes = sample_ascii_font();
    bytes.extend_from_slice(&[0x99, 10, 10]);

    let fonts = decode_ascii_fonts(&bytes).unwrap();

    assert_eq!(fonts.len(), 1);
    assert_eq!(fonts[0].header, 0x42);
}

#[test]
fn classic_fonts_loads_ascii_and_measures_width() {
    let unique = format!(
        "uocf_fonts_test_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();

    let result = (|| -> std::io::Result<()> {
        std::fs::write(dir.join("fonts.mul"), sample_ascii_font())?;

        let fonts = ClassicFonts::load(&dir).unwrap();
        assert_eq!(fonts.ascii_font_count(), 1);
        assert_eq!(fonts.ascii_text_width(0, " A"), 3);
        assert_eq!(fonts.ascii_glyph(0, '\n').unwrap().width, 1);

        Ok(())
    })();

    let _ = std::fs::remove_file(dir.join("fonts.mul"));
    let _ = std::fs::remove_dir(&dir);

    result.unwrap();
}

#[test]
fn unicode_font_decodes_bitpacked_glyphs() {
    let unique = format!(
        "uocf_unifont_test_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    );
    let path = std::env::temp_dir().join(unique);

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&path)?;
        file.set_len((0x10000 * 4 + 7) as u64)?;
        file.seek(SeekFrom::Start(('A' as u32 * 4) as u64))?;
        file.write_all(&(0x10000i32 * 4).to_le_bytes())?;
        file.seek(SeekFrom::Start((0x10000 * 4) as u64))?;
        file.write_all(&[1u8, 255u8, 9u8, 2u8, 0b1000_0000, 0b0100_0000, 0b0010_0000, 0b0000_0000])?;

        let font = UnicodeFontFile::load(&path).unwrap();
        let glyph = font.glyph('A' as u16).unwrap().unwrap();

        assert_eq!((glyph.offset_x, glyph.offset_y, glyph.width, glyph.height), (1, -1, 9, 2));
        assert_eq!(glyph.row_stride_bytes(), Some(2));
        assert_eq!(glyph.bit_at(0, 0), Some(true));
        assert_eq!(glyph.bit_at(8, 0), Some(false));
        assert_eq!(glyph.bit_at(1, 1), Some(false));
        assert_eq!(glyph.bit_at(2, 1), Some(true));
        assert_eq!(glyph.bit_at(9, 0), None);

        Ok(())
    })();

    let _ = std::fs::remove_file(path);

    result.unwrap();
}

#[test]
fn classic_fonts_unicode_metrics_include_offsets() {
    let unique = format!(
        "uocf_unifont_metrics_test_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();

    let result = (|| -> std::io::Result<()> {
        let path = dir.join("unifont.mul");
        let mut file = std::fs::File::create(&path)?;
        file.set_len((0x10000 * 4 + 12) as u64)?;

        file.seek(SeekFrom::Start(('A' as u32 * 4) as u64))?;
        file.write_all(&(0x10000i32 * 4).to_le_bytes())?;
        file.seek(SeekFrom::Start(('B' as u32 * 4) as u64))?;
        file.write_all(&((0x10000i32 * 4) + 6).to_le_bytes())?;

        file.seek(SeekFrom::Start((0x10000 * 4) as u64))?;
        file.write_all(&[1u8, 2u8, 3u8, 4u8, 0u8, 0u8])?;
        file.write_all(&[255u8, 1u8, 5u8, 2u8, 0u8, 0u8])?;

        let fonts = ClassicFonts::load(&dir).unwrap();

        assert!(fonts.unicode_font_exists(0));
        assert!(fonts.unicode_font_exists(1));
        assert_eq!(fonts.unicode_text_width(0, "AB").unwrap(), 8);
        assert_eq!(fonts.unicode_text_height(0, "AB").unwrap(), 6);

        Ok(())
    })();

    let _ = std::fs::remove_file(dir.join("unifont.mul"));
    let _ = std::fs::remove_dir(&dir);

    result.unwrap();
}
