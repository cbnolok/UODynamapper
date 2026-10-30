//! Enhanced Client radar images (`facetNN.dds`): reading and writing the file, regenerating it from a facet
//! package, updating it around edited squares, and scoring a regenerated image against a shipped one.
//!
//! The client format and use are traced in
//! `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md`: a loose file `facet%02d.dds` (zero
//! padded, `facet0.dds` is never found), DXT1, one pixel per square, with the full mip chain of
//! `floor(log2(min(w, h))) + 1` levels. The client copies its 8-byte DXT1 blocks raw and never recomputes land
//! colours, so the land and far statics on the radar exist only in this file.
//!
//! The generator is not in the client. The colour rule here is the recipe fitted offline on the shipped images
//! (about 0.98-0.99 palette-index agreement):
//!
//! 1. The land colour is `radarcol.mul[land id]` (Classic `radarcol.mul`, RGB555).
//! 2. Among the square's statics with `z >= land z`, the highest z wins, the later one on equal z; its colour
//!    is `radarcol.mul[0x4000 + graphic]`. Hue and tileart height are ignored. Statics past the end of
//!    `radarcol.mul` are skipped.
//! 3. Land colours only (not a chosen static) are shaded by `D = z(x + 1, y) - z(x, y + 1)` of the land z,
//!    clamped at the map edge: each 5-bit channel times 0.6 for `D <= -3`, times 1.4 for `D >= 3`, floored and
//!    clamped to 31.
//! 4. The 5-bit channels expand to 8 bits (`c << 3 | c >> 2`), and mip 0 is DXT1-encoded (the `dds` crate's
//!    BC1 encoder). Each lower mip averages 2x2 pixels of the level above as decoded from its DXT1 blocks, then
//!    is encoded the same way. Building the mips from the decoded level (rather than from uncompressed
//!    pixels, which are not stored) is what lets [`FacetRadarDds::update_region`] give exactly the bytes of a
//!    full rebuild.
//!
//! Byte-identical regeneration is not possible: the shipped encoder is unknown. [`radar_index_agreement`]
//! measures the fraction of pixels whose nearest colour in the shipped block palette is the shipped pixel's
//! colour, which does not depend on the encoder.

use std::path::Path;

use dds::{ColorFormat, CompressionQuality, EncodeOptions, Format, ImageView, Size};
use rayon::prelude::*;

use crate::classic::radarcol::load_radarcol;
use crate::enhanced::facet_decoder::{DecodedEcFacet, EcFacetTile, StaticTile};
use crate::enhanced::facet_package::EcFacetPackage;
use crate::enhanced::facet_sector::{FacetGeometry, SECTOR_SIDE};
use crate::utils::color::Rgb555;

crate::eyre_imports!();

/// Offset of the static colours in `radarcol.mul` (after the 0x4000 land colours).
pub const RADAR_STATIC_COLOR_OFFSET: usize = 0x4000;
/// `|D|` at which land is shaded.
pub const RADAR_SHADE_STEP: i32 = 3;
/// Factor `numerator / denominator` of the 5-bit channels where the land falls away (`D <= -3`): 0.6.
pub const RADAR_SHADE_DARK: (u16, u16) = (6, 10);
/// Factor `numerator / denominator` of the 5-bit channels where the land rises (`D >= 3`): 1.4.
pub const RADAR_SHADE_BRIGHT: (u16, u16) = (14, 10);
/// Bytes of the DDS magic and header.
pub const RADAR_DDS_HEADER_SIZE: usize = 128;

const UNSHADED: (u16, u16) = (1, 1);
const CHANNEL_5BIT_MAX: u16 = 0x1f;
const RGB555_RED_SHIFT: u16 = 10;
const RGB555_GREEN_SHIFT: u16 = 5;
/// Set in a packed base colour when the colour comes from a static, which is not shaded (RGB555 leaves bit 15
/// free).
const BASE_FROM_STATIC: u16 = 0x8000;
const RGB_CHANNELS: usize = 3;
const BC1_BLOCK_SIDE: u32 = 4;
const BC1_BLOCK_PIXELS: usize = (BC1_BLOCK_SIDE * BC1_BLOCK_SIDE) as usize;
const BC1_BLOCK_BYTES: usize = 8;
const BC1_PALETTE_SIZE: usize = 4;
const BC1_INDEX_BITS: u32 = 2;
const BC1_INDEX_MASK: u32 = 0b11;
const RGB565_RED_SHIFT: u16 = 11;
const RGB565_GREEN_SHIFT: u16 = 5;
const CHANNEL_6BIT_MAX: u16 = 0x3f;
/// Rows of blocks per BC1 encoding job.
const ENCODE_STRIP_BLOCK_ROWS: u32 = 16;

// DDS header, as the shipped files write it (NVTT).
const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DDS_HEADER_STRUCT_SIZE: u32 = 124;
/// CAPS | HEIGHT | WIDTH | PIXELFORMAT | MIPMAPCOUNT | LINEARSIZE.
const DDS_HEADER_FLAGS: u32 = 0x000a_1007;
const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
const DDS_PIXEL_FORMAT_FOURCC: u32 = 0x4;
const DDS_FOURCC_DXT1: &[u8; 4] = b"DXT1";
/// COMPLEX | TEXTURE | MIPMAP.
const DDS_CAPS: u32 = 0x0040_1008;
const NVTT_TAG: &[u8; 4] = b"NVTT";
const NVTT_VERSION: u32 = 0x0000_0100;
const DDS_OFFSET_HEADER_SIZE: usize = 4;
const DDS_OFFSET_FLAGS: usize = 8;
const DDS_OFFSET_HEIGHT: usize = 12;
const DDS_OFFSET_WIDTH: usize = 16;
const DDS_OFFSET_LINEAR_SIZE: usize = 20;
const DDS_OFFSET_MIP_COUNT: usize = 28;
const DDS_OFFSET_NVTT_TAG: usize = 68;
const DDS_OFFSET_NVTT_VERSION: usize = 72;
const DDS_OFFSET_PIXEL_FORMAT: usize = 76;
const DDS_OFFSET_FOURCC: usize = 84;
const DDS_OFFSET_CAPS: usize = 108;

/// The radar file name of facet `facet`: `facet%02d.dds`.
pub fn radar_dds_file_name(facet: u8) -> String {
    format!("facet{facet:02}.dds")
}

/// Mip levels of a radar image: `floor(log2(min(w, h))) + 1`.
pub fn radar_mip_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.min(height).max(1).leading_zeros()
}

/// The size of mip `level`.
pub fn radar_level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// Expands a 5-bit channel to 8 bits.
pub fn expand_5bit(channel: u16) -> u8 {
    ((channel << 3) | (channel >> 2)) as u8
}

fn expand_6bit(channel: u16) -> u8 {
    ((channel << 2) | (channel >> 4)) as u8
}

/// `radarcol.mul`: the land colours, then the static colours from [`RADAR_STATIC_COLOR_OFFSET`].
#[derive(Debug, Clone)]
pub struct RadarPalette {
    colors: Vec<u16>,
}

impl RadarPalette {
    pub fn new(colors: &[Rgb555]) -> Self {
        let pack = |color: &Rgb555| {
            u16::from(color.r()) << RGB555_RED_SHIFT | u16::from(color.g()) << RGB555_GREEN_SHIFT | u16::from(color.b())
        };
        Self { colors: colors.iter().map(pack).collect() }
    }

    pub fn load_radarcol(path: &Path) -> eyre::Result<Self> {
        Ok(Self::new(&load_radarcol(path).wrap_err_with(|| format!("load {}", path.display()))?))
    }

    /// The RGB555 colour of a land id (black past the end of the table).
    pub fn land_color(&self, land: u16) -> u16 {
        self.colors.get(usize::from(land)).copied().unwrap_or_default()
    }

    /// The RGB555 colour of a static graphic, when the table has one.
    pub fn static_color(&self, graphic: u32) -> Option<u16> {
        self.colors.get(RADAR_STATIC_COLOR_OFFSET.checked_add(graphic as usize)?).copied()
    }
}

/// The static that colours a square: among those with `z >= land_z` and a radar colour, the highest z, the
/// later one on equal z.
pub fn choose_radar_static<'a>(land_z: i8, statics: &'a [StaticTile], palette: &RadarPalette) -> Option<&'a StaticTile> {
    statics
        .iter()
        .filter(|item| item.z >= land_z && palette.static_color(item.graphic_id).is_some())
        .max_by_key(|item| item.z)
}

/// The square's colour before shading, packed with [`BASE_FROM_STATIC`].
fn radar_square_base(tile: &EcFacetTile, palette: &RadarPalette) -> u16 {
    match choose_radar_static(tile.z, &tile.statics, palette) {
        Some(item) => palette.static_color(item.graphic_id).expect("chosen statics have a colour") | BASE_FROM_STATIC,
        None => palette.land_color(tile.land_graphic),
    }
}

/// The 8-bit RGB of a square from its packed base colour and its shading slope `D`.
fn radar_pixel(base: u16, slope: i32) -> [u8; RGB_CHANNELS] {
    let (numerator, denominator) = if base & BASE_FROM_STATIC != 0 {
        UNSHADED
    } else if slope <= -RADAR_SHADE_STEP {
        RADAR_SHADE_DARK
    } else if slope >= RADAR_SHADE_STEP {
        RADAR_SHADE_BRIGHT
    } else {
        UNSHADED
    };
    let shade = |channel: u16| expand_5bit((channel * numerator / denominator).min(CHANNEL_5BIT_MAX));
    [
        shade((base >> RGB555_RED_SHIFT) & CHANNEL_5BIT_MAX),
        shade((base >> RGB555_GREEN_SHIFT) & CHANNEL_5BIT_MAX),
        shade(base & CHANNEL_5BIT_MAX),
    ]
}

/// The per-square radar inputs of a whole facet: the base colour and the land z, row-major.
pub struct FacetRadarSource {
    geometry: FacetGeometry,
    base: Vec<u16>,
    z: Vec<i8>,
}

impl FacetRadarSource {
    /// All squares land 0 at z 0 until sectors are added.
    pub fn new(geometry: FacetGeometry) -> Self {
        let squares = geometry.width as usize * geometry.height as usize;
        Self { geometry, base: vec![0; squares], z: vec![0; squares] }
    }

    /// Reads every sector of `facet` (with its unflushed edits).
    pub fn from_facet(facet: &EcFacetPackage, palette: &RadarPalette) -> eyre::Result<Self> {
        let mut source = Self::new(facet.geometry());
        facet.for_each_sector(|sector, decoded| {
            source.add_sector(sector, decoded, palette);
            Ok(())
        })?;
        Ok(source)
    }

    /// Fills the squares of sector `sector`; its padding squares are skipped.
    pub fn add_sector(&mut self, sector: u32, decoded: &DecodedEcFacet, palette: &RadarPalette) {
        let (origin_x, origin_y) = self.geometry.sector_origin(sector);
        for (index, tile) in decoded.tiles.iter().enumerate() {
            let x = origin_x + index as u32 / SECTOR_SIDE;
            let y = origin_y + index as u32 % SECTOR_SIDE;
            if self.geometry.contains(x, y) {
                let at = self.index(x, y);
                self.base[at] = radar_square_base(tile, palette);
                self.z[at] = tile.z;
            }
        }
    }

    pub fn geometry(&self) -> FacetGeometry {
        self.geometry
    }

    /// The 8-bit RGB image, row-major.
    pub fn rgb8(&self) -> Vec<u8> {
        let width = self.geometry.width as usize;
        let mut rgb = vec![0u8; self.base.len() * RGB_CHANNELS];
        rgb.par_chunks_mut(width * RGB_CHANNELS).enumerate().for_each(|(y, row)| {
            for (x, pixel) in row.chunks_exact_mut(RGB_CHANNELS).enumerate() {
                let (x, y) = (x as u32, y as u32);
                pixel.copy_from_slice(&radar_pixel(self.base[self.index(x, y)], self.slope(x, y)));
            }
        });
        rgb
    }

    fn index(&self, x: u32, y: u32) -> usize {
        y as usize * self.geometry.width as usize + x as usize
    }

    /// `D = z(x + 1, y) - z(x, y + 1)`, clamped at the map edge.
    fn slope(&self, x: u32, y: u32) -> i32 {
        let right = self.z[self.index((x + 1).min(self.geometry.width - 1), y)];
        let below = self.z[self.index(x, (y + 1).min(self.geometry.height - 1))];
        i32::from(right) - i32::from(below)
    }
}

/// A rectangle of pixels (or squares), `x0..x1` by `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl PixelRect {
    /// The single square `(x, y)`.
    pub fn square(x: u32, y: u32) -> Self {
        Self { x0: x, y0: y, x1: x + 1, y1: y + 1 }
    }

    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Self) -> Self {
        Self { x0: self.x0.min(other.x0), y0: self.y0.min(other.y0), x1: self.x1.max(other.x1), y1: self.y1.max(other.y1) }
    }

    /// Grown by `margin` on every side, then clamped to `width` x `height`.
    fn grown(&self, margin: u32, width: u32, height: u32) -> Self {
        Self {
            x0: self.x0.saturating_sub(margin),
            y0: self.y0.saturating_sub(margin),
            x1: (self.x1 + margin).min(width),
            y1: (self.y1 + margin).min(height),
        }
    }

    /// Grown outward to whole BC1 blocks, clamped to `width` x `height`.
    fn block_aligned(&self, width: u32, height: u32) -> Self {
        Self {
            x0: self.x0 / BC1_BLOCK_SIDE * BC1_BLOCK_SIDE,
            y0: self.y0 / BC1_BLOCK_SIDE * BC1_BLOCK_SIDE,
            x1: self.x1.next_multiple_of(BC1_BLOCK_SIDE).min(width),
            y1: self.y1.next_multiple_of(BC1_BLOCK_SIDE).min(height),
        }
    }

    /// The pixels of the next mip level that average any of these, clamped to `width` x `height`.
    fn halved(&self, width: u32, height: u32) -> Self {
        Self { x0: (self.x0 / 2).min(width - 1), y0: (self.y0 / 2).min(height - 1), x1: self.x1.div_ceil(2).min(width), y1: self.y1.div_ceil(2).min(height) }
    }

    /// The pixels of the level above that these average, clamped to `width` x `height`.
    fn doubled(&self, width: u32, height: u32) -> Self {
        Self { x0: (self.x0 * 2).min(width - 1), y0: (self.y0 * 2).min(height - 1), x1: (self.x1 * 2).min(width), y1: (self.y1 * 2).min(height) }
    }

}

/// A radar image: DXT1 blocks per mip level, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetRadarDds {
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
}

impl FacetRadarDds {
    /// Parses a DXT1 DDS with its mip chain.
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < RADAR_DDS_HEADER_SIZE || &bytes[..DDS_MAGIC.len()] != DDS_MAGIC {
            eyre::bail!("not a DDS file");
        }
        let read = |offset: usize| u32::from_le_bytes(bytes[offset..offset + size_of::<u32>()].try_into().expect("u32 slice"));
        if &bytes[DDS_OFFSET_FOURCC..DDS_OFFSET_FOURCC + DDS_FOURCC_DXT1.len()] != DDS_FOURCC_DXT1 {
            eyre::bail!("the DDS is not DXT1; the client copies raw DXT1 blocks");
        }
        let (width, height, mips) = (read(DDS_OFFSET_WIDTH), read(DDS_OFFSET_HEIGHT), read(DDS_OFFSET_MIP_COUNT).max(1));
        if width == 0 || height == 0 {
            eyre::bail!("empty DDS ({width}x{height})");
        }
        let mut levels = Vec::with_capacity(mips as usize);
        let mut offset = RADAR_DDS_HEADER_SIZE;
        for level in 0..mips {
            let length = level_block_bytes(width, height, level);
            let data = bytes
                .get(offset..offset + length)
                .ok_or_else(|| eyre!("the DDS ends inside mip {level}"))?;
            levels.push(data.to_vec());
            offset += length;
        }
        if offset != bytes.len() {
            eyre::bail!("{} bytes after the last mip", bytes.len() - offset);
        }
        Ok(Self { width, height, levels })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("parse {}", path.display()))
    }

    /// The file bytes, with the header the shipped files have.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; RADAR_DDS_HEADER_SIZE];
        let mut put = |offset: usize, value: u32| out[offset..offset + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
        put(DDS_OFFSET_HEADER_SIZE, DDS_HEADER_STRUCT_SIZE);
        put(DDS_OFFSET_FLAGS, DDS_HEADER_FLAGS);
        put(DDS_OFFSET_HEIGHT, self.height);
        put(DDS_OFFSET_WIDTH, self.width);
        put(DDS_OFFSET_LINEAR_SIZE, self.levels[0].len() as u32);
        put(DDS_OFFSET_MIP_COUNT, self.levels.len() as u32);
        put(DDS_OFFSET_NVTT_VERSION, NVTT_VERSION);
        put(DDS_OFFSET_PIXEL_FORMAT, DDS_PIXEL_FORMAT_SIZE);
        put(DDS_OFFSET_PIXEL_FORMAT + size_of::<u32>(), DDS_PIXEL_FORMAT_FOURCC);
        put(DDS_OFFSET_CAPS, DDS_CAPS);
        out[..DDS_MAGIC.len()].copy_from_slice(DDS_MAGIC);
        out[DDS_OFFSET_NVTT_TAG..DDS_OFFSET_NVTT_TAG + NVTT_TAG.len()].copy_from_slice(NVTT_TAG);
        out[DDS_OFFSET_FOURCC..DDS_OFFSET_FOURCC + DDS_FOURCC_DXT1.len()].copy_from_slice(DDS_FOURCC_DXT1);
        for level in &self.levels {
            out.extend_from_slice(level);
        }
        out
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        std::fs::write(path, self.to_bytes()).wrap_err_with(|| format!("write {}", path.display()))
    }

    /// DXT1-encodes a row-major RGB8 image with the full mip chain.
    pub fn encode_rgb8(width: u32, height: u32, rgb: &[u8]) -> eyre::Result<Self> {
        if rgb.len() != width as usize * height as usize * RGB_CHANNELS {
            eyre::bail!("{} RGB bytes for a {width}x{height} image", rgb.len());
        }
        let mip_count = radar_mip_count(width, height);
        let mut levels = Vec::with_capacity(mip_count as usize);
        levels.push(encode_bc1(width, height, rgb)?);
        let mut image = Self { width, height, levels };
        for level in 1..mip_count {
            let (level_width, level_height) = radar_level_size(width, height, level);
            let pixels = image.downsample_level(level, PixelRect { x0: 0, y0: 0, x1: level_width, y1: level_height });
            image.levels.push(encode_bc1(level_width, level_height, &pixels)?);
        }
        Ok(image)
    }

    /// Regenerates the radar image of `facet` (with its unflushed edits).
    pub fn build(facet: &EcFacetPackage, palette: &RadarPalette) -> eyre::Result<Self> {
        let source = FacetRadarSource::from_facet(facet, palette)?;
        let geometry = source.geometry();
        Self::encode_rgb8(geometry.width, geometry.height, &source.rgb8())
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn mip_count(&self) -> u32 {
        self.levels.len() as u32
    }

    /// The DXT1 blocks of mip `level`.
    pub fn level(&self, level: u32) -> &[u8] {
        &self.levels[level as usize]
    }

    /// Decodes mip `level` to row-major RGB8.
    pub fn decode_level_rgb8(&self, level: u32) -> Vec<u8> {
        let (width, height) = radar_level_size(self.width, self.height, level);
        self.decode_rect(level, PixelRect { x0: 0, y0: 0, x1: width, y1: height })
    }

    /// Re-encodes the pixels whose colour can depend on the squares in `squares` after an edit: at mip 0 the
    /// edited squares and one square around them (the shading reads the right and lower neighbours' z),
    /// recomputed from `facet`; at each lower mip the pixels averaging those, from the updated level above.
    /// Returns the mip 0 pixels rewritten. Every level comes out byte-identical to a full [`Self::build`].
    ///
    /// A static change only changes its own square; a land z change also changes the shading of the squares to
    /// its left and above; both are inside the region.
    pub fn update_region(&mut self, facet: &mut EcFacetPackage, palette: &RadarPalette, squares: PixelRect) -> eyre::Result<PixelRect> {
        let geometry = facet.geometry();
        if (geometry.width, geometry.height) != (self.width, self.height) {
            eyre::bail!(
                "the radar image is {}x{}, the facet {}x{}",
                self.width,
                self.height,
                geometry.width,
                geometry.height
            );
        }
        let rect = squares.grown(1, self.width, self.height).block_aligned(self.width, self.height);
        let inputs = rect.grown(1, self.width, self.height);
        let mut base = Vec::with_capacity((inputs.width() * inputs.height()) as usize);
        let mut z = Vec::with_capacity(base.capacity());
        for y in inputs.y0..inputs.y1 {
            for x in inputs.x0..inputs.x1 {
                let tile = facet.square(x, y)?;
                base.push(radar_square_base(tile, palette));
                z.push(tile.z);
            }
        }
        let input_at = |x: u32, y: u32| ((y - inputs.y0) * inputs.width() + (x - inputs.x0)) as usize;
        let mut pixels = Vec::with_capacity((rect.width() * rect.height()) as usize * RGB_CHANNELS);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let right = z[input_at((x + 1).min(self.width - 1), y)];
                let below = z[input_at(x, (y + 1).min(self.height - 1))];
                pixels.extend_from_slice(&radar_pixel(base[input_at(x, y)], i32::from(right) - i32::from(below)));
            }
        }
        self.write_rect(0, rect, &pixels)?;

        let mut above_rect = rect;
        for level in 1..self.mip_count() {
            let (width, height) = radar_level_size(self.width, self.height, level);
            let level_rect = above_rect.halved(width, height).block_aligned(width, height);
            let level_pixels = self.downsample_level(level, level_rect);
            self.write_rect(level, level_rect, &level_pixels)?;
            above_rect = level_rect;
        }
        Ok(rect)
    }

    /// The pixels of the block-aligned `rect` of mip `level`, averaged from the decoded level above.
    fn downsample_level(&self, level: u32, rect: PixelRect) -> Vec<u8> {
        let (above_width, above_height) = radar_level_size(self.width, self.height, level - 1);
        let footprint = rect.doubled(above_width, above_height).block_aligned(above_width, above_height);
        downsample(&self.decode_rect(level - 1, footprint), footprint, rect)
    }

    /// Encodes the block-aligned `rect` of mip `level` from its RGB8 pixels.
    fn write_rect(&mut self, level: u32, rect: PixelRect, pixels: &[u8]) -> eyre::Result<()> {
        let (width, _) = radar_level_size(self.width, self.height, level);
        let blocks_per_row = width.div_ceil(BC1_BLOCK_SIDE) as usize;
        let encoded = encode_bc1(rect.width(), rect.height(), pixels)?;
        let rect_blocks_per_row = rect.width().div_ceil(BC1_BLOCK_SIDE) as usize;
        let data = &mut self.levels[level as usize];
        for (row, blocks) in encoded.chunks_exact(rect_blocks_per_row * BC1_BLOCK_BYTES).enumerate() {
            let block_row = (rect.y0 / BC1_BLOCK_SIDE) as usize + row;
            let at = (block_row * blocks_per_row + (rect.x0 / BC1_BLOCK_SIDE) as usize) * BC1_BLOCK_BYTES;
            data[at..at + blocks.len()].copy_from_slice(blocks);
        }
        Ok(())
    }

    /// Decodes the block-aligned `rect` of mip `level` to RGB8.
    fn decode_rect(&self, level: u32, rect: PixelRect) -> Vec<u8> {
        let (width, _) = radar_level_size(self.width, self.height, level);
        let blocks_per_row = width.div_ceil(BC1_BLOCK_SIDE) as usize;
        let data = &self.levels[level as usize];
        let row_bytes = rect.width() as usize * RGB_CHANNELS;
        let mut out = vec![0u8; rect.height() as usize * row_bytes];
        out.par_chunks_mut(BC1_BLOCK_SIDE as usize * row_bytes).enumerate().for_each(|(strip, rows)| {
            let block_y = rect.y0 / BC1_BLOCK_SIDE + strip as u32;
            for block_x in rect.x0 / BC1_BLOCK_SIDE..rect.x1.div_ceil(BC1_BLOCK_SIDE) {
                let block = block_y as usize * blocks_per_row + block_x as usize;
                let block = &data[block * BC1_BLOCK_BYTES..(block + 1) * BC1_BLOCK_BYTES];
                let palette = bc1_palette(block);
                for y in 0..(rows.len() / row_bytes) as u32 {
                    for x in block_x * BC1_BLOCK_SIDE..((block_x + 1) * BC1_BLOCK_SIDE).min(rect.x1) {
                        let color = palette[bc1_index(block, x, y)];
                        let at = y as usize * row_bytes + (x - rect.x0) as usize * RGB_CHANNELS;
                        for channel in 0..RGB_CHANNELS {
                            rows[at + channel] = color[channel] as u8;
                        }
                    }
                }
            }
        });
        out
    }
}

/// The fraction of mip 0 pixels where the colour of `rgb8` (row-major, the image's size) picks, from the
/// shipped block's 4-colour palette, the shipped pixel's colour. It does not depend on the DXT1 encoder.
pub fn radar_index_agreement(shipped: &FacetRadarDds, rgb8: &[u8]) -> eyre::Result<f64> {
    let (width, height) = (shipped.width, shipped.height);
    if rgb8.len() != width as usize * height as usize * RGB_CHANNELS {
        eyre::bail!("{} RGB bytes for a {width}x{height} radar image", rgb8.len());
    }
    let blocks_per_row = width.div_ceil(BC1_BLOCK_SIDE) as usize;
    let data = shipped.level(0);
    let agreeing: u64 = (0..height)
        .into_par_iter()
        .map(|y| {
            let mut agreeing = 0u64;
            for x in 0..width {
                let block = (y / BC1_BLOCK_SIDE) as usize * blocks_per_row + (x / BC1_BLOCK_SIDE) as usize;
                let block = &data[block * BC1_BLOCK_BYTES..(block + 1) * BC1_BLOCK_BYTES];
                let palette = bc1_palette(block);
                let at = (y as usize * width as usize + x as usize) * RGB_CHANNELS;
                let ours = &rgb8[at..at + RGB_CHANNELS];
                let distance = |color: &[i32; RGB_CHANNELS]| {
                    color.iter().zip(ours).map(|(&a, &b)| (a - i32::from(b)).pow(2)).sum::<i32>()
                };
                let nearest = (0..BC1_PALETTE_SIZE)
                    .min_by_key(|&k| distance(&palette[k]))
                    .expect("the palette has 4 colours");
                agreeing += u64::from(palette[nearest] == palette[bc1_index(block, x, y)]);
            }
            agreeing
        })
        .sum();
    Ok(agreeing as f64 / (u64::from(width) * u64::from(height)) as f64)
}

/// Bytes of the DXT1 blocks of mip `level`.
fn level_block_bytes(width: u32, height: u32, level: u32) -> usize {
    let (width, height) = radar_level_size(width, height, level);
    width.div_ceil(BC1_BLOCK_SIDE) as usize * height.div_ceil(BC1_BLOCK_SIDE) as usize * BC1_BLOCK_BYTES
}

/// Averages 2x2 pixels of `source` (the RGB8 pixels of `source_rect` of the level above, which the averaged
/// pixels must lie in or be clamped to) into the pixels of `rect` of the next level. At an odd edge the last
/// row or column is repeated.
fn downsample(source: &[u8], source_rect: PixelRect, rect: PixelRect) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let mut out = Vec::with_capacity((rect.width() * rect.height()) as usize * RGB_CHANNELS);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let mut sum = [0u32; RGB_CHANNELS];
            for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (2 * x + sx).min(source_rect.x1 - 1);
                let sy = (2 * y + sy).min(source_rect.y1 - 1);
                let at = ((sy - source_rect.y0) * source_rect.width() + (sx - source_rect.x0)) as usize * RGB_CHANNELS;
                for channel in 0..RGB_CHANNELS {
                    sum[channel] += u32::from(source[at + channel]);
                }
            }
            out.extend(sum.map(|total| ((total + SAMPLES / 2) / SAMPLES) as u8));
        }
    }
    out
}

/// DXT1-encodes a row-major RGB8 image, in strips of block rows in parallel.
fn encode_bc1(width: u32, height: u32, rgb: &[u8]) -> eyre::Result<Vec<u8>> {
    let mut options = EncodeOptions::default();
    options.quality = CompressionQuality::Normal;
    options.parallel = false;
    let strip_rows = ENCODE_STRIP_BLOCK_ROWS * BC1_BLOCK_SIDE;
    let row_bytes = width as usize * RGB_CHANNELS;
    let strips: Vec<Vec<u8>> = rgb
        .par_chunks(strip_rows as usize * row_bytes)
        .map(|strip| -> eyre::Result<Vec<u8>> {
            let rows = (strip.len() / row_bytes) as u32;
            let image = ImageView::new(strip, Size::new(width, rows), ColorFormat::RGB_U8)
                .ok_or_else(|| eyre!("bad RGB strip of {width}x{rows}"))?;
            let mut out = Vec::new();
            dds::encode(&mut out, image, Format::BC1_UNORM, None, &options)?;
            Ok(out)
        })
        .collect::<eyre::Result<_>>()?;
    Ok(strips.concat())
}

/// The 4-colour palette of a DXT1 block, 8-bit RGB.
fn bc1_palette(block: &[u8]) -> [[i32; RGB_CHANNELS]; BC1_PALETTE_SIZE] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let expand = |c: u16| {
        [
            i32::from(expand_5bit((c >> RGB565_RED_SHIFT) & CHANNEL_5BIT_MAX)),
            i32::from(expand_6bit((c >> RGB565_GREEN_SHIFT) & CHANNEL_6BIT_MAX)),
            i32::from(expand_5bit(c & CHANNEL_5BIT_MAX)),
        ]
    };
    let (e0, e1) = (expand(c0), expand(c1));
    let mix = |a: i32, b: i32, weight_a: i32, weight_b: i32| (weight_a * a + weight_b * b) / (weight_a + weight_b);
    let (p2, p3) = if c0 > c1 {
        (std::array::from_fn(|i| mix(e0[i], e1[i], 2, 1)), std::array::from_fn(|i| mix(e0[i], e1[i], 1, 2)))
    } else {
        (std::array::from_fn(|i| mix(e0[i], e1[i], 1, 1)), [0; RGB_CHANNELS])
    };
    [e0, e1, p2, p3]
}

/// The palette index of pixel `(x, y)` (image coordinates) in its DXT1 block.
fn bc1_index(block: &[u8], x: u32, y: u32) -> usize {
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let pixel = (y % BC1_BLOCK_SIDE) * BC1_BLOCK_SIDE + x % BC1_BLOCK_SIDE;
    debug_assert!((pixel as usize) < BC1_BLOCK_PIXELS);
    ((bits >> (BC1_INDEX_BITS * pixel)) & BC1_INDEX_MASK) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::facet_sector::{empty_ec_sector, expected_delimiters};

    const FACET: u8 = 2;
    const WIDTH: u32 = 136;
    const HEIGHT: u32 = 100;
    /// Static graphics with a colour in the test palette.
    const STATIC_COLORS: usize = 0x100;
    /// Spreads the palette indexes over the RGB555 colours.
    const COLOR_SPREAD: u16 = 0x2b5;
    const RGB555_MASK: u16 = 0x7fff;
    /// The synthetic land: z steps and land ids repeating over the map.
    const Z_STEP_X: u32 = 3;
    const Z_STEP_Y: u32 = 5;
    const Z_LEVELS: u32 = 23;
    const Z_BASE: i8 = -10;
    const LAND_IDS: u32 = 40;

    fn palette() -> RadarPalette {
        let colors: Vec<Rgb555> = (0..RADAR_STATIC_COLOR_OFFSET + STATIC_COLORS)
            .map(|i| Rgb555::new_from_val((i as u16).wrapping_mul(COLOR_SPREAD) & RGB555_MASK))
            .collect();
        RadarPalette::new(&colors)
    }

    fn facet() -> EcFacetPackage {
        let geometry = FacetGeometry::new(WIDTH, HEIGHT);
        let land = |x: u32, y: u32| {
            (((x * Z_STEP_X + y * Z_STEP_Y) % Z_LEVELS) as i8 + Z_BASE, ((x / Z_STEP_X + y / Z_STEP_Y) % LAND_IDS) as u16)
        };
        let mut facet = EcFacetPackage::new(FACET, geometry).unwrap();
        for sector in 0..geometry.sector_count() {
            let (ox, oy) = geometry.sector_origin(sector);
            let mut decoded = empty_ec_sector(FACET, sector).unwrap();
            for (index, tile) in decoded.tiles.iter_mut().enumerate() {
                let (x, y) = (ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE);
                if geometry.contains(x, y) {
                    (tile.z, tile.land_graphic) = land(x, y);
                    tile.delimiters = expected_delimiters(&geometry, x, y, land);
                }
            }
            facet.put_sector(&decoded).unwrap();
        }
        facet
    }

    #[test]
    fn file_name_and_mip_chain_follow_the_client() {
        assert_eq!(radar_dds_file_name(4), "facet04.dds");
        assert_eq!(radar_mip_count(7168, 4096), 13);
        assert_eq!(radar_mip_count(1448, 1448), 11);
        assert_eq!(radar_mip_count(1280, 4096), 11);
        assert_eq!(radar_level_size(1280, 4096, 10), (1, 4));
    }

    #[test]
    fn static_choice_and_shading_follow_the_recipe() {
        let palette = palette();
        let item = |graphic, z| StaticTile { graphic_id: graphic, x: 0, y: 0, z, hue: 7 };
        let statics = [item(1, 5), item(2, 9), item(3, 9), item(4, -2), item(0x10000, 20)];
        assert_eq!(choose_radar_static(0, &statics, &palette).unwrap().graphic_id, 3);
        assert!(choose_radar_static(10, &statics, &palette).is_none());

        let land = 20 << RGB555_RED_SHIFT | 10 << RGB555_GREEN_SHIFT | 31;
        assert_eq!(radar_pixel(land, 0), [expand_5bit(20), expand_5bit(10), expand_5bit(31)]);
        assert_eq!(radar_pixel(land, -RADAR_SHADE_STEP), [expand_5bit(12), expand_5bit(6), expand_5bit(18)]);
        assert_eq!(radar_pixel(land, RADAR_SHADE_STEP), [expand_5bit(28), expand_5bit(14), expand_5bit(31)]);
        assert_eq!(radar_pixel(land | BASE_FROM_STATIC, RADAR_SHADE_STEP), radar_pixel(land, 0));
    }

    #[test]
    fn dds_round_trips_and_writes_the_shipped_header() {
        let facet = facet();
        let dds = FacetRadarDds::build(&facet, &palette()).unwrap();
        assert_eq!(dds.mip_count(), radar_mip_count(WIDTH, HEIGHT));
        let bytes = dds.to_bytes();
        assert_eq!(&bytes[DDS_OFFSET_FLAGS..DDS_OFFSET_FLAGS + 4], &DDS_HEADER_FLAGS.to_le_bytes());
        assert_eq!(FacetRadarDds::from_bytes(&bytes).unwrap(), dds);
    }

    #[test]
    fn region_update_matches_a_full_rebuild() {
        let palette = palette();
        let mut facet = facet();
        let original = FacetRadarDds::build(&facet, &palette).unwrap();
        let mut updated = original.clone();
        facet.set_land(64, 63, 33, 40).unwrap();
        facet.add_static(70, 10, 0x20, 50, 0).unwrap();
        let region = PixelRect::square(64, 63).union(&PixelRect::square(70, 10));
        let rewritten = updated.update_region(&mut facet, &palette, region).unwrap();
        assert!(rewritten.x0 <= 63 && rewritten.y0 <= 9 && rewritten.x1 >= 72 && rewritten.y1 >= 64);
        let rebuilt = FacetRadarDds::build(&facet, &palette).unwrap();
        assert_ne!(updated.level(0), original.level(0));
        assert_eq!(updated, rebuilt);
    }
}
