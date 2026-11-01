//! Writing a `hues.mul` as a `Hues.uop` on top of an existing package (EA's), so that every hue the `hues.mul`
//! leaves unedited keeps the package's bitmap, atlas row, swatch and `huenames.csv` line byte for byte.
//!
//! EA's bitmaps are not all derivable from `hues.mul`. In the 2024 EC, 365 of the 3000 hues are 8-bit gradients
//! whose texels at `x = 8 i` reduce (`v >> 3`) to the `hues.mul` colours but carry low bits, and whose other
//! texels and swatch follow no rule of those colours: the Classic file holds only their 5-bit reduction. They
//! were saved by other tools, with other BMP headers: 293 with `bfSize` = the file size and 3780 ppm, 72 of 2104
//! bytes (`biSizeImage` 2050, 2834 ppm, 2 bytes after the pixels); EA's converter writes the rest with `bfSize`
//! 525 and zero resolutions. The KR has 4 bitmaps of the 2104-byte kind (two with a hand-picked swatch). And
//! `huenames.csv` keeps names of an older `hues.mul` (all 20 bytes, where the current file clears the 20th) and
//! leaves out the comma on some rows with an empty name, by no rule of the id. Such data can only come from the
//! package itself.
//!
//! `UOHues::LoadHueBitmap` (UOSA.exe 0x004271f0) checks only `biBitCount` 32, `biWidth` 256 and `biHeight` 2,
//! takes the pixels at `bfOffBits`, and reads the first texel of the first stored row (the swatch) and the 256
//! texels of the second (the table); the other header fields and swatch texels are never read.
//!
//! A hue is unedited when the base has a bitmap of that layout whose table texels at `x = 8 i` reduce to the
//! `hues.mul` colours (bit 15 aside). A name is unedited when it equals the base's, or when the `hues.mul` name is
//! 19 bytes and the base's is those 19 bytes plus one. Every other hue is written by the rules of
//! [`build_hues_uop_entries`](super::build_hues_uop_entries) with the linear ramp. The metadata entry is written as always, so the package
//! converts back to the identical `hues.mul`.

use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use super::{
    assemble_hues_uop_entries, hue_bitmap_file_path, reduce_hue_color, write_hues_uop,
    HueParts, HueRampMode, HuesUopEntry, Rgba, COLOR_VALUE_MASK, CSV_LINE_END, RGBA_CHANNELS,
    TEXELS_PER_HUE_COLOR,
};
use crate::classic::hues::{HueEntry, HuesMul, HUE_COLOR_COUNT, HUE_NAME_LEN};
use crate::enhanced::hues::{HUENAMES_PATH, HUE_BITMAP_HEIGHT, HUE_BITMAP_WIDTH, HUE_TABLE_ENTRIES};
use crate::enhanced::uop_entries::refuse_to_overwrite;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};

/// Offsets of the BMP header fields `UOHues::LoadHueBitmap` reads.
const BMP_PIXEL_OFFSET_FIELD: usize = 10;
const BMP_WIDTH_FIELD: usize = 18;
const BMP_HEIGHT_FIELD: usize = 22;
const BMP_BIT_COUNT_FIELD: usize = 28;
/// The only bit depth the client accepts.
const CLIENT_BITS_PER_PIXEL: u16 = 32;
const CSV_FIELD_SEPARATOR: u8 = b',';
const LINE_FEED: u8 = b'\n';
const CARRIAGE_RETURN: u8 = b'\r';

/// How much of a base package a write kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseReuse {
    /// Hues whose bitmap, atlas row and swatch came from the base.
    pub bitmaps_kept: usize,
    /// Hues written by the rules: edited, or without a readable bitmap in the base.
    pub bitmaps_rebuilt: Vec<u32>,
    /// Hues whose `huenames.csv` line came from the base.
    pub names_kept: usize,
    /// Hues whose `huenames.csv` line was written as `id,name`.
    pub names_rebuilt: Vec<u32>,
}

/// The entries of the `Hues.uop` of `hues`, keeping from `base` every unedited hue's bitmap, atlas row, swatch
/// and name line; the other hues as [`build_hues_uop_entries`](super::build_hues_uop_entries) with [`HueRampMode::Linear`].
pub fn build_hues_uop_entries_on_base(
    hues: &HuesMul,
    base: &UopPackage,
) -> eyre::Result<(Vec<HuesUopEntry>, BaseReuse)> {
    let mut reuse = BaseReuse::default();
    let mut parts = Vec::with_capacity(hues.hue_count());
    for hue in hues.hues() {
        let path = hue_bitmap_file_path(hue.id);
        let bitmap = base.unpack_file_by_hash(hash_file_name_single(&path)).wrap_err_with(|| format!("failed to read {path} of the base"))?;
        match bitmap.and_then(|bitmap| unedited_parts(hue, bitmap)) {
            Some(part) => {
                reuse.bitmaps_kept += 1;
                parts.push(part);
            }
            None => {
                reuse.bitmaps_rebuilt.push(hue.id);
                parts.push(HueParts::by_rule(hue, HueRampMode::Linear));
            }
        }
    }

    let base_lines = base
        .unpack_file_by_hash(hash_file_name_single(HUENAMES_PATH))
        .wrap_err("failed to read huenames.csv of the base")?
        .map(|bytes| name_lines(&bytes))
        .unwrap_or_default();
    let mut csv = Vec::new();
    for hue in hues.hues() {
        match base_lines.get(&hue.id).filter(|line| name_is_unedited(hue, line_name(line))) {
            Some(line) => {
                reuse.names_kept += 1;
                csv.extend_from_slice(line);
            }
            None => {
                reuse.names_rebuilt.push(hue.id);
                csv.extend_from_slice(format!("{},", hue.id).as_bytes());
                csv.extend_from_slice(hue.name_bytes());
            }
        }
        csv.extend_from_slice(CSV_LINE_END);
    }
    Ok((assemble_hues_uop_entries(hues, parts, csv), reuse))
}

/// Converts a `hues.mul` file to a `Hues.uop` file on top of the `base_path` package (see the module
/// documentation). The output must not be the base.
pub fn convert_hues_mul_to_uop_on_base(
    mul_path: &Path,
    base_path: &Path,
    uop_path: &Path,
    filetime: Option<u64>,
) -> eyre::Result<BaseReuse> {
    refuse_to_overwrite(base_path, uop_path)?;
    let hues = HuesMul::load(mul_path)?;
    let base = UopPackage::load_with_mode(base_path, LoadMode::Eager)
        .wrap_err_with(|| format!("failed to load {}", base_path.display()))?;
    let (entries, reuse) = build_hues_uop_entries_on_base(&hues, &base)?;
    write_hues_uop(&entries, uop_path, filetime)?;
    Ok(reuse)
}

/// The parts of `bitmap` when the client reads it and its table still holds `hue`'s colours.
fn unedited_parts(hue: &HueEntry, bitmap: Vec<u8>) -> Option<HueParts> {
    let (table, swatch) = client_view(&bitmap)?;
    let unedited = (0..HUE_COLOR_COUNT)
        .all(|index| reduce_hue_color(table[index * TEXELS_PER_HUE_COLOR]) == hue.color_table[index] & COLOR_VALUE_MASK);
    unedited.then_some(HueParts { id: hue.id, table, swatch, bitmap })
}

/// The table and swatch of a hue bitmap as `UOHues::LoadHueBitmap` reads them, or None when it refuses it.
fn client_view(bitmap: &[u8]) -> Option<([Rgba; HUE_TABLE_ENTRIES], Rgba)> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(bitmap.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bitmap.get(at..at + 4)?.try_into().ok()?));
    if u16_at(BMP_BIT_COUNT_FIELD)? != CLIENT_BITS_PER_PIXEL
        || u32_at(BMP_WIDTH_FIELD)? != HUE_BITMAP_WIDTH
        || u32_at(BMP_HEIGHT_FIELD)? != HUE_BITMAP_HEIGHT
    {
        return None;
    }
    let row_bytes = HUE_TABLE_ENTRIES * RGBA_CHANNELS;
    let start = usize::try_from(u32_at(BMP_PIXEL_OFFSET_FIELD)?).ok()?;
    let pixels = bitmap.get(start..start.checked_add(row_bytes * HUE_BITMAP_HEIGHT as usize)?)?;
    let texel = |bgra: &[u8]| [bgra[2], bgra[1], bgra[0], bgra[3]];
    let swatch = texel(&pixels[..RGBA_CHANNELS]);
    let mut table = [[0; RGBA_CHANNELS]; HUE_TABLE_ENTRIES];
    for (entry, bgra) in table.iter_mut().zip(pixels[row_bytes..].chunks_exact(RGBA_CHANNELS)) {
        *entry = texel(bgra);
    }
    Some((table, swatch))
}

/// The lines of a `huenames.csv` without their line ends, by the id before the first comma (the first line of
/// an id wins; lines without a numeric id are skipped).
fn name_lines(bytes: &[u8]) -> BTreeMap<u32, Vec<u8>> {
    let mut lines = BTreeMap::new();
    for line in bytes.split(|&byte| byte == LINE_FEED) {
        let line = line.strip_suffix(&[CARRIAGE_RETURN]).unwrap_or(line);
        let id_end = line.iter().position(|&byte| byte == CSV_FIELD_SEPARATOR).unwrap_or(line.len());
        let Some(id) = std::str::from_utf8(&line[..id_end]).ok().and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        lines.entry(id).or_insert_with(|| line.to_vec());
    }
    lines
}

/// The name of a `huenames.csv` line: what follows the first comma, or nothing.
fn line_name(line: &[u8]) -> &[u8] {
    line.iter().position(|&byte| byte == CSV_FIELD_SEPARATOR).map_or(&[], |comma| &line[comma + 1..])
}

/// Whether `hue`'s name is the one of a base line `base_name` (see the module documentation).
fn name_is_unedited(hue: &HueEntry, base_name: &[u8]) -> bool {
    let name = hue.name_bytes();
    base_name == name
        || (name.len() == HUE_NAME_LEN - 1 && base_name.len() == HUE_NAME_LEN && base_name.starts_with(name))
}
