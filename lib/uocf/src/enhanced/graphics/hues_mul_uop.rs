//! Conversion between Classic `hues.mul` and Enhanced Client `Hues.uop`.
//!
//! `Hues.uop` as EA ships it (version 4, block size 100, zlib, 12-byte timestamp data block headers), in entry order:
//! - `FIXED_PALETTE_HASH`: 64 x 64 A8R8G8B8 DDS of hue swatches; hue `h` is the texel at
//!   `x = h / SWATCH_GRID_SIDE`, `y = h % SWATCH_GRID_SIDE` (column-major), texel 0 and unused texels are
//!   transparent black;
//! - `HUES_ATLAS_PATH`: 1024 x 1024 A8R8G8B8 DDS, no mipmaps, hue `h` in strip `h / 1024`, row `h % 1024`;
//! - `hue%04d.bmp` (1..=3000): 256 x 2, 32 bpp, bottom-up; image row 0 is the hue table (the atlas row),
//!   image row 1 the swatch colour;
//! - `HUENAMES_PATH`: `id,name` lines, CRLF.
//!
//! A table row expands the 32 `hues.mul` colours to 256 texels: colour `i` sits at `x = 8 i` as its 5-bit
//! channels `c << 3`, and the 7 texels after it ramp linearly to colour `i + 1` (colour 31 is held to the end),
//! [`HueRampMode::Linear`]. The ramp is exact in integers because both ends are multiples of 8. The swatch is
//! the larger of colour 0 and colour 31 as 15-bit values. Both rules reproduce every EA bitmap whose colours
//! still match the Classic file of the same release (2635 of 3000 hues in the 2024 clients). The other 365 are
//! 8-bit gradients that `hues.mul` holds only reduced to 5 bits: [`build_hues_uop_entries_on_base`] keeps them,
//! and every other unedited hue, from an existing package.
//!
//! EA's format drops what `hues.mul` has beyond the 15-bit colours: bit 15 of each colour, table start and
//! end, the raw 20 name bytes, the group headers and any bytes after the last whole group. This converter
//! stores them in one more entry, [`HUES_MUL_METADATA_PATH`], which the client never looks up (see
//! [`HuesMulMetadata`] for its layout), so a package written here converts back to the identical `hues.mul`.
//! Without that entry (EA's packages) the colours are still exact for every unedited hue and the rest is
//! defaulted: table start = colour 0, table end = colour 31 + 0x421 (the rule of every hue of the modern
//! Classic files), names from `huenames.csv`, headers 0.

use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use crate::classic::hues::{HueEntry, HuesMul, HUES_PER_GROUP, HUE_COLOR_COUNT, HUE_NAME_LEN};
use crate::enhanced::hues::{
    atlas_coord_for_hue, decode_hue_image_to_rgba, hue_bitmap_hash, hue_bitmap_path, parse_huenames_csv,
    FIXED_PALETTE_HASH, HUENAMES_PATH, HUES_ATLAS_HEIGHT, HUES_ATLAS_PATH, HUES_ATLAS_WIDTH, HUE_BITMAP_HEIGHT,
    HUE_BITMAP_WIDTH, HUE_TABLE_ENTRIES, HUE_TABLE_IMAGE_ROW, MAX_EC_HUES,
};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, NewEntryCompression, UopPackage};

/// Entry holding what `hues.mul` has and EA's format drops (see [`HuesMulMetadata`]).
pub const HUES_MUL_METADATA_PATH: &str = "build/hues/huesmul_metadata.bin";
/// First bytes of [`HUES_MUL_METADATA_PATH`].
pub const HUES_MUL_METADATA_MAGIC: [u8; 4] = *b"HMUL";
/// Layout version of [`HUES_MUL_METADATA_PATH`].
pub const HUES_MUL_METADATA_VERSION: u16 = 1;
/// UOP version and block size of the shipped `Hues.uop`.
pub const HUES_UOP_VERSION: u32 = 4;
pub const HUES_UOP_BLOCK_SIZE: u32 = 100;
/// Side of the square swatch texture `FIXED_PALETTE_HASH`.
pub const SWATCH_GRID_SIDE: u32 = 64;
/// Atlas texels per `hues.mul` colour.
pub const TEXELS_PER_HUE_COLOR: usize = HUE_TABLE_ENTRIES / HUE_COLOR_COUNT;
/// `table_end` of the modern Classic files is colour 31 plus this (wrapping `u16` add: +1 on each channel).
pub const TABLE_END_COLOR_OFFSET: u16 = 0x0421;

/// One RGBA texel.
pub type Rgba = [u8; 4];

const CHANNEL_MASK: u16 = 0x1f;
const RED_SHIFT: u16 = 10;
const GREEN_SHIFT: u16 = 5;
/// 5-bit to 8-bit channel expansion (`c << 3`, as the client) and its inverse (`v >> 3`).
const CHANNEL_EXPAND_SHIFT: u32 = 3;
/// Bit 15 of a `hues.mul` colour; not part of the colour.
const COLOR_FLAG_BIT: u16 = 0x8000;
const COLOR_VALUE_MASK: u16 = 0x7fff;
const OPAQUE: u8 = u8::MAX;
const TRANSPARENT: Rgba = [0; 4];
const RGBA_CHANNELS: usize = 4;

const BMP_FILE_HEADER_SIZE: u32 = 14;
const BMP_INFO_HEADER_SIZE: u32 = 40;
const BMP_PIXEL_OFFSET: u32 = BMP_FILE_HEADER_SIZE + BMP_INFO_HEADER_SIZE;
/// `bfSize` of every EA-written hue bitmap: EA's writer stores this constant, not the file size (2102).
const EA_BMP_FILE_SIZE_FIELD: u32 = 0x020d;
const BMP_PLANES: u16 = 1;
const BMP_BITS_PER_PIXEL: u16 = 32;
const BMP_COMPRESSION_RGB: u32 = 0;

const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DDS_HEADER_SIZE: u32 = 124;
const DDS_FILE_HEADER_SIZE: usize = DDS_MAGIC.len() + DDS_HEADER_SIZE as usize;
/// CAPS | HEIGHT | WIDTH | PIXELFORMAT | LINEARSIZE, as EA's files (no `MIPMAPCOUNT`).
const DDS_FLAGS: u32 = 0x0008_1007;
const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
/// ALPHAPIXELS | RGB.
const DDS_PIXEL_FORMAT_FLAGS: u32 = 0x41;
const DDS_RED_MASK: u32 = 0x00ff_0000;
const DDS_GREEN_MASK: u32 = 0x0000_ff00;
const DDS_BLUE_MASK: u32 = 0x0000_00ff;
const DDS_ALPHA_MASK: u32 = 0xff00_0000;
/// TEXTURE.
const DDS_CAPS: u32 = 0x1000;
const DDS_RESERVED_DWORDS: usize = 11;
const DDS_TRAILING_DWORDS: usize = 4;
const DDS_HEIGHT_OFFSET: usize = 12;
const DDS_WIDTH_OFFSET: usize = 16;
const DDS_PIXEL_FORMAT_OFFSET: usize = 76;

const CSV_LINE_END: &[u8] = b"\r\n";

const METADATA_HEADER_SIZE: usize = 16;
/// Per hue: `u32` colour flag bits, `u16` table start, `u16` table end, name.
const METADATA_ENTRY_SIZE: usize = 8 + HUE_NAME_LEN;
const METADATA_GROUP_SIZE: usize = 4 + HUES_PER_GROUP * METADATA_ENTRY_SIZE;

/// How a hue table row spreads the 32 `hues.mul` colours over its 256 texels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HueRampMode {
    /// Colour `i` at `x = 8 i`, ramping linearly to colour `i + 1`: EA's atlas and bitmaps.
    #[default]
    Linear,
    /// Each colour repeated over its 8 texels.
    Flat,
}

/// A `hues.mul` colour as an opaque RGBA texel (channels `c << 3`); bit 15 is ignored.
pub fn expand_hue_color(color: u16) -> Rgba {
    let channel = |shift: u16| (((color >> shift) & CHANNEL_MASK) << CHANNEL_EXPAND_SHIFT) as u8;
    [channel(RED_SHIFT), channel(GREEN_SHIFT), channel(0), OPAQUE]
}

/// The 15-bit colour of a texel (`v >> 3` per channel), the inverse of [`expand_hue_color`].
pub fn reduce_hue_color(texel: Rgba) -> u16 {
    let channel = |value: u8| u16::from(value) >> CHANNEL_EXPAND_SHIFT;
    (channel(texel[0]) << RED_SHIFT) | (channel(texel[1]) << GREEN_SHIFT) | channel(texel[2])
}

/// The hue table row of `colors`: `HUE_TABLE_ENTRIES` texels.
pub fn hue_palette_row(colors: &[u16; HUE_COLOR_COUNT], mode: HueRampMode) -> [Rgba; HUE_TABLE_ENTRIES] {
    let step = TEXELS_PER_HUE_COLOR as u32;
    std::array::from_fn(|x| {
        let index = x / TEXELS_PER_HUE_COLOR;
        let from = expand_hue_color(colors[index]);
        let weight = (x % TEXELS_PER_HUE_COLOR) as u32;
        if mode == HueRampMode::Flat || weight == 0 {
            return from;
        }
        let to = expand_hue_color(colors[(index + 1).min(HUE_COLOR_COUNT - 1)]);
        let mut texel = from;
        for channel in 0..RGBA_CHANNELS - 1 {
            texel[channel] =
                ((u32::from(from[channel]) * (step - weight) + u32::from(to[channel]) * weight) / step) as u8;
        }
        texel
    })
}

/// The swatch colour of a hue (bitmap row 1, the swatch texture): the larger of colour 0 and colour 31.
pub fn hue_swatch_color(colors: &[u16; HUE_COLOR_COUNT]) -> Rgba {
    let first = colors[0] & COLOR_VALUE_MASK;
    let last = colors[HUE_COLOR_COUNT - 1] & COLOR_VALUE_MASK;
    expand_hue_color(first.max(last))
}

/// A `hue%04d.bmp` as EA writes it: 256 x 2, 32 bpp BGRA, bottom-up, image row 0 = `table`, row 1 = `swatch`.
pub fn encode_hue_bitmap(table: &[Rgba; HUE_TABLE_ENTRIES], swatch: Rgba) -> Vec<u8> {
    let pixel_bytes = HUE_TABLE_ENTRIES * RGBA_CHANNELS * HUE_BITMAP_HEIGHT as usize;
    let mut out = Vec::with_capacity(BMP_PIXEL_OFFSET as usize + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&EA_BMP_FILE_SIZE_FIELD.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&BMP_PIXEL_OFFSET.to_le_bytes());
    out.extend_from_slice(&BMP_INFO_HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&HUE_BITMAP_WIDTH.to_le_bytes());
    out.extend_from_slice(&HUE_BITMAP_HEIGHT.to_le_bytes());
    out.extend_from_slice(&BMP_PLANES.to_le_bytes());
    out.extend_from_slice(&BMP_BITS_PER_PIXEL.to_le_bytes());
    out.extend_from_slice(&BMP_COMPRESSION_RGB.to_le_bytes());
    // Image size, resolutions and palette counts: all zero in EA's files.
    out.extend_from_slice(&[0; 20]);
    // Bottom-up: the swatch row is stored first.
    for _ in 0..HUE_TABLE_ENTRIES {
        push_bgra(&mut out, swatch);
    }
    for &texel in table {
        push_bgra(&mut out, texel);
    }
    out
}

/// An uncompressed A8R8G8B8 DDS without mipmaps, with EA's header fields.
pub fn encode_argb32_dds(width: u32, height: u32, texels: &[Rgba]) -> Vec<u8> {
    debug_assert_eq!(texels.len(), width as usize * height as usize);
    let mut out = Vec::with_capacity(DDS_FILE_HEADER_SIZE + texels.len() * RGBA_CHANNELS);
    out.extend_from_slice(DDS_MAGIC);
    for value in [DDS_HEADER_SIZE, DDS_FLAGS, height, width, width * height * RGBA_CHANNELS as u32, 0, 0] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&[0; DDS_RESERVED_DWORDS * 4]);
    for value in [
        DDS_PIXEL_FORMAT_SIZE,
        DDS_PIXEL_FORMAT_FLAGS,
        0,
        RGBA_CHANNELS as u32 * u8::BITS,
        DDS_RED_MASK,
        DDS_GREEN_MASK,
        DDS_BLUE_MASK,
        DDS_ALPHA_MASK,
        DDS_CAPS,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&[0; DDS_TRAILING_DWORDS * 4]);
    for &texel in texels {
        push_bgra(&mut out, texel);
    }
    out
}

/// Reads an uncompressed A8R8G8B8 DDS (the atlas and swatch textures): width, height and RGBA texels.
pub fn decode_argb32_dds(bytes: &[u8]) -> eyre::Result<(u32, u32, Vec<Rgba>)> {
    let read_u32 = |offset: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
    };
    if !bytes.starts_with(DDS_MAGIC) || bytes.len() < DDS_FILE_HEADER_SIZE {
        eyre::bail!("not a DDS file");
    }
    let field = |offset: usize| read_u32(offset).expect("the header is in bounds");
    let format = DDS_PIXEL_FORMAT_OFFSET;
    let masks = [field(format + 16), field(format + 20), field(format + 24), field(format + 28)];
    if field(format + 4) & DDS_PIXEL_FORMAT_FLAGS != DDS_PIXEL_FORMAT_FLAGS
        || field(format + 12) != RGBA_CHANNELS as u32 * u8::BITS
        || masks != [DDS_RED_MASK, DDS_GREEN_MASK, DDS_BLUE_MASK, DDS_ALPHA_MASK]
    {
        eyre::bail!("DDS is not uncompressed A8R8G8B8");
    }
    let (width, height) = (field(DDS_WIDTH_OFFSET), field(DDS_HEIGHT_OFFSET));
    let texel_count = width as usize * height as usize;
    let pixels = bytes
        .get(DDS_FILE_HEADER_SIZE..DDS_FILE_HEADER_SIZE + texel_count * RGBA_CHANNELS)
        .ok_or_else(|| eyre::eyre!("DDS {width}x{height} is truncated"))?;
    let texels = pixels.chunks_exact(RGBA_CHANNELS).map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]).collect();
    Ok((width, height, texels))
}

/// The atlas texels (`HUES_ATLAS_WIDTH` x `HUES_ATLAS_HEIGHT`) of every hue up to `MAX_EC_HUES`.
pub fn build_hues_atlas(hues: &HuesMul, mode: HueRampMode) -> Vec<Rgba> {
    atlas_of_tables(hues.hues().map(|hue| (hue.id, hue_palette_row(&hue.color_table, mode))))
}

/// The atlas texels of `(hue id, table row)` pairs; ids without an atlas row are skipped.
fn atlas_of_tables(tables: impl Iterator<Item = (u32, [Rgba; HUE_TABLE_ENTRIES])>) -> Vec<Rgba> {
    let width = HUES_ATLAS_WIDTH as usize;
    let mut atlas = vec![TRANSPARENT; width * HUES_ATLAS_HEIGHT as usize];
    for (id, table) in tables {
        let Some(coord) = u16::try_from(id).ok().and_then(atlas_coord_for_hue) else {
            continue;
        };
        let start = coord.y as usize * width + coord.x as usize;
        atlas[start..start + HUE_TABLE_ENTRIES].copy_from_slice(&table);
    }
    atlas
}

/// The swatch texture texels (`SWATCH_GRID_SIDE` squared) of every hue up to `MAX_EC_HUES`.
pub fn build_swatch_grid(hues: &HuesMul) -> Vec<Rgba> {
    swatch_grid_of(hues.hues().map(|hue| (hue.id, hue_swatch_color(&hue.color_table))))
}

/// The swatch texture texels of `(hue id, swatch)` pairs; ids above `MAX_EC_HUES` are skipped.
fn swatch_grid_of(swatches: impl Iterator<Item = (u32, Rgba)>) -> Vec<Rgba> {
    let side = SWATCH_GRID_SIDE as usize;
    let mut grid = vec![TRANSPARENT; side * side];
    for (id, swatch) in swatches.filter(|(id, _)| *id <= u32::from(MAX_EC_HUES)) {
        let (x, y) = (id as usize / side, id as usize % side);
        grid[y * side + x] = swatch;
    }
    grid
}

/// `huenames.csv`: one `id,name` line per hue (the name up to its first NUL), CRLF.
pub fn encode_huenames_csv(hues: &HuesMul) -> Vec<u8> {
    let mut out = Vec::new();
    for hue in hues.hues() {
        out.extend_from_slice(format!("{},", hue.id).as_bytes());
        out.extend_from_slice(hue.name_bytes());
        out.extend_from_slice(CSV_LINE_END);
    }
    out
}

/// The fields of one hue that EA's format drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HueMetadata {
    /// Bit `i` is bit 15 of colour `i`.
    pub color_flag_bits: u32,
    pub table_start: u16,
    pub table_end: u16,
    pub name: [u8; HUE_NAME_LEN],
}

/// What `hues.mul` has beyond the 15-bit colours: [`HUES_MUL_METADATA_PATH`].
///
/// Layout (little-endian), version [`HUES_MUL_METADATA_VERSION`]:
///
/// | offset | size | field |
/// |---|---|---|
/// | 0 | 4 | magic [`HUES_MUL_METADATA_MAGIC`] |
/// | 4 | 2 | version |
/// | 6 | 2 | reserved, 0 |
/// | 8 | 4 | group count `g` |
/// | 12 | 4 | trailing byte count `t` |
/// | 16 | `g` × 228 | groups: `u32` header, then 8 × (`u32` colour bit-15 flags, `u16` table start, `u16` table end, 20-byte name) |
/// | 16 + 228 `g` | `t` | the bytes after the last whole group of `hues.mul` |
///
/// The colours themselves are not repeated: they come from the bitmaps (`v >> 3` at `x = 8 i`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuesMulMetadata {
    pub group_headers: Vec<u32>,
    /// One per hue, `HUES_PER_GROUP` per group.
    pub hues: Vec<HueMetadata>,
    pub trailing: Vec<u8>,
}

impl HuesMulMetadata {
    pub fn from_hues(hues: &HuesMul) -> Self {
        Self {
            group_headers: hues.groups.iter().map(|group| group.header).collect(),
            hues: hues
                .hues()
                .map(|hue| HueMetadata {
                    color_flag_bits: hue
                        .color_table
                        .iter()
                        .enumerate()
                        .filter(|(_, color)| *color & COLOR_FLAG_BIT != 0)
                        .fold(0, |bits, (index, _)| bits | 1 << index),
                    table_start: hue.table_start,
                    table_end: hue.table_end,
                    name: hue.name,
                })
                .collect(),
            trailing: hues.trailing.clone(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            METADATA_HEADER_SIZE + self.group_headers.len() * METADATA_GROUP_SIZE + self.trailing.len(),
        );
        out.extend_from_slice(&HUES_MUL_METADATA_MAGIC);
        out.extend_from_slice(&HUES_MUL_METADATA_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(self.group_headers.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.trailing.len() as u32).to_le_bytes());
        for (header, hues) in self.group_headers.iter().zip(self.hues.chunks(HUES_PER_GROUP)) {
            out.extend_from_slice(&header.to_le_bytes());
            for hue in hues {
                out.extend_from_slice(&hue.color_flag_bits.to_le_bytes());
                out.extend_from_slice(&hue.table_start.to_le_bytes());
                out.extend_from_slice(&hue.table_end.to_le_bytes());
                out.extend_from_slice(&hue.name);
            }
        }
        out.extend_from_slice(&self.trailing);
        out
    }

    pub fn decode(bytes: &[u8]) -> eyre::Result<Self> {
        let header = bytes.get(..METADATA_HEADER_SIZE).ok_or_else(|| eyre::eyre!("hues.mul metadata is truncated"))?;
        let read_u16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        if header[..HUES_MUL_METADATA_MAGIC.len()] != HUES_MUL_METADATA_MAGIC {
            eyre::bail!("hues.mul metadata has a bad magic");
        }
        let version = read_u16(4);
        if version != HUES_MUL_METADATA_VERSION {
            eyre::bail!("hues.mul metadata version {version} is not supported (expected {HUES_MUL_METADATA_VERSION})");
        }
        let group_count = read_u32(8) as usize;
        let trailing_len = read_u32(12) as usize;
        let expected = group_count
            .checked_mul(METADATA_GROUP_SIZE)
            .and_then(|size| size.checked_add(METADATA_HEADER_SIZE + trailing_len))
            .ok_or_else(|| eyre::eyre!("hues.mul metadata sizes overflow"))?;
        if bytes.len() != expected {
            eyre::bail!("hues.mul metadata is {} bytes, expected {expected}", bytes.len());
        }
        let mut metadata = Self::default();
        for group in 0..group_count {
            let base = METADATA_HEADER_SIZE + group * METADATA_GROUP_SIZE;
            metadata.group_headers.push(read_u32(base));
            for entry in 0..HUES_PER_GROUP {
                let at = base + 4 + entry * METADATA_ENTRY_SIZE;
                let mut name = [0u8; HUE_NAME_LEN];
                name.copy_from_slice(&bytes[at + 8..at + 8 + HUE_NAME_LEN]);
                metadata.hues.push(HueMetadata {
                    color_flag_bits: read_u32(at),
                    table_start: read_u16(at + 4),
                    table_end: read_u16(at + 6),
                    name,
                });
            }
        }
        metadata.trailing = bytes[expected - trailing_len..].to_vec();
        Ok(metadata)
    }
}

/// One entry of a `Hues.uop` to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuesUopEntry {
    pub hash: u64,
    /// The packed name; None for the swatch texture, known only by hash.
    pub path: Option<String>,
    pub data: Vec<u8>,
}

impl HuesUopEntry {
    fn named(path: String, data: Vec<u8>) -> Self {
        Self { hash: hash_file_name_single(&path), path: Some(path), data }
    }
}

/// The entries of the `Hues.uop` of `hues`, in EA's order, then the metadata entry. Every hue gets a bitmap;
/// the atlas and swatch texture hold the hues up to `MAX_EC_HUES`.
pub fn build_hues_uop_entries(hues: &HuesMul, mode: HueRampMode) -> Vec<HuesUopEntry> {
    let parts = hues.hues().map(|hue| HueParts::by_rule(hue, mode)).collect::<Vec<_>>();
    assemble_hues_uop_entries(hues, parts, encode_huenames_csv(hues))
}

/// What one hue puts in a `Hues.uop`: its table row (bitmap and atlas), swatch (swatch texture) and bitmap.
struct HueParts {
    id: u32,
    table: [Rgba; HUE_TABLE_ENTRIES],
    swatch: Rgba,
    bitmap: Vec<u8>,
}

impl HueParts {
    /// The parts of `hue` by EA's rules: the `mode` ramp and the larger end colour as the swatch.
    fn by_rule(hue: &HueEntry, mode: HueRampMode) -> Self {
        let table = hue_palette_row(&hue.color_table, mode);
        let swatch = hue_swatch_color(&hue.color_table);
        Self { id: hue.id, table, swatch, bitmap: encode_hue_bitmap(&table, swatch) }
    }
}

/// The entries of a `Hues.uop` in EA's order: the swatch texture, the atlas, the bitmaps (`parts`, one per hue of
/// `hues`), `huenames.csv`, then the metadata entry.
fn assemble_hues_uop_entries(hues: &HuesMul, parts: Vec<HueParts>, huenames_csv: Vec<u8>) -> Vec<HuesUopEntry> {
    let mut entries = Vec::with_capacity(parts.len() + 4);
    entries.push(HuesUopEntry {
        hash: FIXED_PALETTE_HASH,
        path: None,
        data: encode_argb32_dds(
            SWATCH_GRID_SIDE,
            SWATCH_GRID_SIDE,
            &swatch_grid_of(parts.iter().map(|part| (part.id, part.swatch))),
        ),
    });
    entries.push(HuesUopEntry::named(
        HUES_ATLAS_PATH.to_string(),
        encode_argb32_dds(HUES_ATLAS_WIDTH, HUES_ATLAS_HEIGHT, &atlas_of_tables(parts.iter().map(|part| (part.id, part.table)))),
    ));
    for part in parts {
        entries.push(HuesUopEntry::named(hue_bitmap_file_path(part.id), part.bitmap));
    }
    entries.push(HuesUopEntry::named(HUENAMES_PATH.to_string(), huenames_csv));
    entries.push(HuesUopEntry::named(HUES_MUL_METADATA_PATH.to_string(), HuesMulMetadata::from_hues(hues).encode()));
    entries
}

/// Writes `entries` as a `Hues.uop` like EA's: version 4, block size 100, zlib, every entry with a timestamp
/// data block header (`filetime`, or the current time when None). Empty payloads (`huenames.csv` of a file
/// without hues) are left out: the client fails to read entries of size zero.
pub fn write_hues_uop(entries: &[HuesUopEntry], path: &Path, filetime: Option<u64>) -> eyre::Result<()> {
    let mut package = UopPackage::new(HUES_UOP_VERSION, HUES_UOP_BLOCK_SIZE)?;
    package.set_edit_filetime(filetime);
    for entry in entries.iter().filter(|entry| !entry.data.is_empty()) {
        package
            .add_entry(entry.hash, &entry.data, NewEntryCompression::Flag(CompressionFlag::Zlib))
            .wrap_err_with(|| format!("failed to add {}", entry.path.as_deref().unwrap_or("the swatch texture")))?;
    }
    package.finalize_and_save(path).wrap_err_with(|| format!("failed to write {}", path.display()))
}

/// Converts a `hues.mul` file to a `Hues.uop` file.
pub fn convert_hues_mul_to_uop(
    mul_path: &Path,
    uop_path: &Path,
    mode: HueRampMode,
    filetime: Option<u64>,
) -> eyre::Result<()> {
    let hues = HuesMul::load(mul_path)?;
    write_hues_uop(&build_hues_uop_entries(&hues, mode), uop_path, filetime)
}

/// A `hues.mul` read back from a `Hues.uop`, and how it was recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuesFromUop {
    pub hues: HuesMul,
    /// The package had [`HUES_MUL_METADATA_PATH`]; without it, metadata was defaulted.
    pub had_metadata: bool,
    /// Hues whose colours came from the atlas because their bitmap is missing or unreadable.
    pub colors_from_atlas: Vec<u32>,
    /// Hues with neither a bitmap nor an atlas row: their colours are zero.
    pub missing_colors: Vec<u32>,
}

/// Recovers the `hues.mul` of a `Hues.uop`. Colours come from the bitmaps' top row at `x = 8 i` (the atlas
/// when a bitmap is missing); everything else from the metadata entry, or defaults without it.
pub fn hues_mul_from_uop_package(package: &UopPackage) -> eyre::Result<HuesFromUop> {
    let metadata = package
        .unpack_file_by_hash(hash_file_name_single(HUES_MUL_METADATA_PATH))?
        .map(|bytes| HuesMulMetadata::decode(&bytes))
        .transpose()?;
    let atlas = package
        .unpack_file_by_hash(hash_file_name_single(HUES_ATLAS_PATH))?
        .map(|bytes| decode_argb32_dds(&bytes))
        .transpose()
        .wrap_err("failed to read the hue atlas")?
        .filter(|(width, height, _)| (*width, *height) == (HUES_ATLAS_WIDTH, HUES_ATLAS_HEIGHT))
        .map(|(_, _, texels)| texels);

    let group_count = match &metadata {
        Some(metadata) => metadata.group_headers.len(),
        None => {
            // The last hue with a bitmap, else every hue of the atlas.
            let last_bitmap = (1..=MAX_EC_HUES).rev().find(|&id| package.get_file_by_hash(hue_bitmap_hash(id)).is_some());
            let last = last_bitmap.or(atlas.as_ref().map(|_| MAX_EC_HUES)).unwrap_or(0);
            usize::from(last).div_ceil(HUES_PER_GROUP)
        }
    };
    let mut hues = HuesMul::with_group_count(group_count);
    let mut colors_from_atlas = Vec::new();
    let mut missing_colors = Vec::new();
    for id in 1..=hues.hue_count() as u32 {
        let row = match read_bitmap_row(package, id) {
            Some(row) => Some(row),
            None => {
                let atlas_row = atlas.as_ref().zip(u16::try_from(id).ok().and_then(atlas_coord_for_hue)).map(
                    |(atlas, coord)| {
                        let start = coord.y as usize * HUES_ATLAS_WIDTH as usize + coord.x as usize;
                        atlas[start..start + HUE_TABLE_ENTRIES].to_vec()
                    },
                );
                if atlas_row.is_some() { colors_from_atlas.push(id) } else { missing_colors.push(id) }
                atlas_row
            }
        };
        if let Some(row) = row {
            let hue = hues.hue_mut(id).expect("the hue is in range");
            for (index, color) in hue.color_table.iter_mut().enumerate() {
                *color = reduce_hue_color(row[index * TEXELS_PER_HUE_COLOR]);
            }
        }
    }

    match &metadata {
        Some(metadata) => {
            for (group, &header) in hues.groups.iter_mut().zip(&metadata.group_headers) {
                group.header = header;
            }
            for (hue, meta) in hues.groups.iter_mut().flat_map(|group| group.entries.iter_mut()).zip(&metadata.hues) {
                for (index, color) in hue.color_table.iter_mut().enumerate() {
                    if meta.color_flag_bits & 1 << index != 0 {
                        *color |= COLOR_FLAG_BIT;
                    }
                }
                hue.table_start = meta.table_start;
                hue.table_end = meta.table_end;
                hue.name = meta.name;
            }
            hues.trailing = metadata.trailing.clone();
        }
        None => {
            let names = match package.unpack_file_by_hash(hash_file_name_single(HUENAMES_PATH))? {
                Some(bytes) => parse_huenames_csv(&bytes),
                None => Default::default(),
            };
            for hue in hues.groups.iter_mut().flat_map(|group| group.entries.iter_mut()) {
                apply_default_metadata(hue, u16::try_from(hue.id).ok().and_then(|id| names.get(&id)).map(String::as_bytes));
            }
        }
    }

    Ok(HuesFromUop { hues, had_metadata: metadata.is_some(), colors_from_atlas, missing_colors })
}

/// Converts a `Hues.uop` file to a `hues.mul` file.
pub fn convert_hues_uop_to_mul(uop_path: &Path, mul_path: &Path) -> eyre::Result<HuesFromUop> {
    let package = UopPackage::load_with_mode(uop_path, LoadMode::Eager)
        .wrap_err_with(|| format!("failed to load {}", uop_path.display()))?;
    let recovered = hues_mul_from_uop_package(&package)?;
    recovered.hues.save(mul_path)?;
    Ok(recovered)
}

/// Table start = colour 0, table end = colour 31 + `TABLE_END_COLOR_OFFSET`, the name truncated to 20 bytes.
fn apply_default_metadata(hue: &mut HueEntry, name: Option<&[u8]>) {
    hue.table_start = hue.color_table[0];
    hue.table_end = hue.color_table[HUE_COLOR_COUNT - 1].wrapping_add(TABLE_END_COLOR_OFFSET);
    hue.name = [0; HUE_NAME_LEN];
    if let Some(name) = name {
        let len = name.len().min(HUE_NAME_LEN);
        hue.name[..len].copy_from_slice(&name[..len]);
    }
}

/// The top image row of hue `id`'s bitmap, when it exists and decodes to at least `HUE_TABLE_ENTRIES` texels.
fn read_bitmap_row(package: &UopPackage, id: u32) -> Option<Vec<Rgba>> {
    let hash = hash_file_name_single(&hue_bitmap_file_path(id));
    let bytes = package.unpack_file_by_hash(hash).ok()??;
    let (width, height, rgba) = decode_hue_image_to_rgba(&bytes).ok()?;
    if width < HUE_BITMAP_WIDTH || height <= HUE_TABLE_IMAGE_ROW as u32 {
        return None;
    }
    let start = HUE_TABLE_IMAGE_ROW * width as usize * RGBA_CHANNELS;
    let row = &rgba[start..start + HUE_TABLE_ENTRIES * RGBA_CHANNELS];
    Some(row.chunks_exact(RGBA_CHANNELS).map(|texel| [texel[0], texel[1], texel[2], texel[3]]).collect())
}

/// `hue_bitmap_path` for any hue id (ids above `u16::MAX` cannot occur in a `hues.mul` of sane size).
fn hue_bitmap_file_path(id: u32) -> String {
    match u16::try_from(id) {
        Ok(id) => hue_bitmap_path(id),
        Err(_) => format!("data/definitions/hues/hue{id:04}.bmp"),
    }
}

fn push_bgra(out: &mut Vec<u8>, texel: Rgba) {
    out.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]);
}

#[path = "hues_mul_uop_base.rs"]
mod base;
pub use base::{build_hues_uop_entries_on_base, convert_hues_mul_to_uop_on_base, put_edited_hues, BaseReuse};

#[cfg(test)]
#[path = "hues_mul_uop_tests.rs"]
mod tests;
