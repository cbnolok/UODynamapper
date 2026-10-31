//! DDS textures of client packages: the pixel formats, mip chains and header conventions the shipped files use,
//! and a writer that reproduces them (see "DDS formats in client packages" in `ENHANCED_CLIENT_FILES.md`).

use std::io::Cursor;
use std::path::Path;

use dds::header::{Caps, Caps2, DdsFlags, FourCC, PixelFormatFlags, RawHeader};
use image::RgbaImage;

use crate::enhanced::uop_entries;
use crate::uop_container::package::{LoadMode, UopPackage};

crate::eyre_imports!();

pub const DDS_MAGIC: &[u8; 4] = b"DDS ";
/// The `dwSize` of the DDS header, which follows the magic.
pub const DDS_HEADER_STRUCT_SIZE: u32 = 124;
/// Bytes of the magic and the header.
pub const DDS_HEADER_SIZE: usize = DDS_MAGIC.len() + DDS_HEADER_STRUCT_SIZE as usize;
/// Bytes of the DX10 extension header.
pub const DDS_DX10_HEADER_SIZE: usize = 20;
/// The `dwSize` of the pixel format.
pub const DDS_PIXEL_FORMAT_STRUCT_SIZE: u32 = 32;
const BLOCK_SIDE: u32 = 4;
const BITS_PER_BYTE: u32 = 8;
const FOURCC_LENGTH: usize = 4;

/// The pixel format of a DDS, as its header names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DdsTextureFormat {
    Dxt1,
    Dxt2,
    Dxt3,
    Dxt4,
    Dxt5,
    Ati1,
    Ati2,
    Bc4U,
    Bc5U,
    /// A DX10 extension header with this DXGI format.
    Dx10(u32),
    /// Any other FourCC.
    OtherFourCc(u32),
    A8R8G8B8,
    X8R8G8B8,
    A8B8G8R8,
    X8B8G8R8,
    R8G8B8,
    R5G6B5,
    A1R5G5B5,
    X1R5G5B5,
    A4R4G4B4,
    X4R4G4B4,
    L8,
    A8L8,
    A4L4,
    A8,
    L16,
    /// Any other uncompressed layout.
    OtherMask { flags: u32, bit_count: u32, masks: [u32; 4] },
}

/// Pixel format flags, bit count and red, green, blue, alpha masks of the named uncompressed formats.
const MASK_FORMATS: [(DdsTextureFormat, u32, u32, [u32; 4]); 16] = {
    const RGB: u32 = PixelFormatFlags::RGB.bits();
    const RGBA: u32 = PixelFormatFlags::RGBA.bits();
    const LUMINANCE: u32 = PixelFormatFlags::LUMINANCE.bits();
    const LUMINANCE_ALPHA: u32 = PixelFormatFlags::LUMINANCE_ALPHA.bits();
    const ALPHA: u32 = PixelFormatFlags::ALPHA.bits();
    use DdsTextureFormat::*;
    [
        (A8R8G8B8, RGBA, 32, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000]),
        (X8R8G8B8, RGB, 32, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0]),
        (A8B8G8R8, RGBA, 32, [0x0000_00ff, 0x0000_ff00, 0x00ff_0000, 0xff00_0000]),
        (X8B8G8R8, RGB, 32, [0x0000_00ff, 0x0000_ff00, 0x00ff_0000, 0]),
        (R8G8B8, RGB, 24, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0]),
        (R5G6B5, RGB, 16, [0xf800, 0x07e0, 0x001f, 0]),
        (A1R5G5B5, RGBA, 16, [0x7c00, 0x03e0, 0x001f, 0x8000]),
        (X1R5G5B5, RGB, 16, [0x7c00, 0x03e0, 0x001f, 0]),
        (A4R4G4B4, RGBA, 16, [0x0f00, 0x00f0, 0x000f, 0xf000]),
        (X4R4G4B4, RGB, 16, [0x0f00, 0x00f0, 0x000f, 0]),
        (L8, LUMINANCE, 8, [0xff, 0, 0, 0]),
        (A8L8, LUMINANCE_ALPHA, 16, [0x00ff, 0, 0, 0xff00]),
        (A4L4, LUMINANCE_ALPHA, 8, [0x0f, 0, 0, 0xf0]),
        (A8, ALPHA, 8, [0, 0, 0, 0xff]),
        (L16, LUMINANCE, 16, [0xffff, 0, 0, 0]),
        // The alpha-only layout some writers flag as ALPHAPIXELS instead of ALPHA.
        (A8, PixelFormatFlags::ALPHAPIXELS.bits(), 8, [0, 0, 0, 0xff]),
    ]
};

const FOURCC_FORMATS: [(&[u8; FOURCC_LENGTH], DdsTextureFormat); 9] = [
    (b"DXT1", DdsTextureFormat::Dxt1),
    (b"DXT2", DdsTextureFormat::Dxt2),
    (b"DXT3", DdsTextureFormat::Dxt3),
    (b"DXT4", DdsTextureFormat::Dxt4),
    (b"DXT5", DdsTextureFormat::Dxt5),
    (b"ATI1", DdsTextureFormat::Ati1),
    (b"ATI2", DdsTextureFormat::Ati2),
    (b"BC4U", DdsTextureFormat::Bc4U),
    (b"BC5U", DdsTextureFormat::Bc5U),
];

/// DXGI formats a DX10 header can name, as the census reports them.
const DXGI_FORMAT_NAMES: [(u32, &str); 12] = [
    (28, "R8G8B8A8_UNORM"),
    (29, "R8G8B8A8_UNORM_SRGB"),
    (71, "BC1_UNORM"),
    (72, "BC1_UNORM_SRGB"),
    (74, "BC2_UNORM"),
    (77, "BC3_UNORM"),
    (80, "BC4_UNORM"),
    (83, "BC5_UNORM"),
    (87, "B8G8R8A8_UNORM"),
    (88, "B8G8R8X8_UNORM"),
    (98, "BC7_UNORM"),
    (99, "BC7_UNORM_SRGB"),
];

/// DXGI block-compressed formats by bytes per 4x4 block.
const DXGI_BC8_FORMATS: [u32; 4] = [70, 71, 72, 80];
const DXGI_BC16_FORMATS: [u32; 10] = [73, 74, 75, 76, 77, 78, 83, 95, 96, 98];
const DXGI_BC16_SRGB: u32 = 99;
const DXGI_32BIT_FORMATS: [u32; 4] = [28, 29, 87, 88];
const BC8_BLOCK_BYTES: u32 = 8;
const BC16_BLOCK_BYTES: u32 = 16;
const RGBA32_BITS: u32 = 32;

impl DdsTextureFormat {
    /// The format a header names. Fails only when the header is too short for its DX10 extension.
    pub fn of_header(header: &RawHeader) -> Self {
        let pixel_format = &header.pixel_format;
        if pixel_format.flags.contains(PixelFormatFlags::FOURCC) {
            if pixel_format.four_cc == FourCC::DX10 {
                return Self::Dx10(header.dx10.as_ref().map_or(0, |dx10| dx10.dxgi_format));
            }
            let code = pixel_format.four_cc.0.to_le_bytes();
            return FOURCC_FORMATS
                .iter()
                .find(|(name, _)| **name == code)
                .map_or(Self::OtherFourCc(pixel_format.four_cc.0), |&(_, format)| format);
        }
        let masks =
            [pixel_format.r_bit_mask, pixel_format.g_bit_mask, pixel_format.b_bit_mask, pixel_format.a_bit_mask];
        MASK_FORMATS
            .iter()
            .find(|&&(_, flags, bit_count, format_masks)| {
                pixel_format.flags.bits() == flags && pixel_format.rgb_bit_count == bit_count && masks == format_masks
            })
            .map_or(
                Self::OtherMask { flags: pixel_format.flags.bits(), bit_count: pixel_format.rgb_bit_count, masks },
                |&(format, ..)| format,
            )
    }

    /// The name the census uses: the FourCC, `DX10 <DXGI format>`, or the D3D9 name of the layout.
    pub fn label(&self) -> String {
        match *self {
            Self::Dx10(dxgi) => match DXGI_FORMAT_NAMES.iter().find(|(code, _)| *code == dxgi) {
                Some((_, name)) => format!("DX10 {name}"),
                None => format!("DX10 DXGI {dxgi}"),
            },
            Self::OtherFourCc(code) => format!("FourCC {}", String::from_utf8_lossy(&code.to_le_bytes())),
            Self::OtherMask { flags, bit_count, masks } => {
                format!("flags {flags:#x} {bit_count} bit {:08x}/{:08x}/{:08x}/{:08x}", masks[0], masks[1], masks[2], masks[3])
            }
            other => format!("{other:?}").to_uppercase(),
        }
    }

    /// Bytes per 4x4 block of a block-compressed format.
    pub fn block_bytes(&self) -> Option<u32> {
        match *self {
            Self::Dxt1 | Self::Ati1 | Self::Bc4U => Some(BC8_BLOCK_BYTES),
            Self::Dxt2 | Self::Dxt3 | Self::Dxt4 | Self::Dxt5 | Self::Ati2 | Self::Bc5U => Some(BC16_BLOCK_BYTES),
            Self::Dx10(dxgi) if DXGI_BC8_FORMATS.contains(&dxgi) => Some(BC8_BLOCK_BYTES),
            Self::Dx10(dxgi) if DXGI_BC16_FORMATS.contains(&dxgi) || dxgi == DXGI_BC16_SRGB => Some(BC16_BLOCK_BYTES),
            _ => None,
        }
    }

    /// Bits per pixel of an uncompressed format.
    pub fn bits_per_pixel(&self) -> Option<u32> {
        match *self {
            Self::Dx10(dxgi) if DXGI_32BIT_FORMATS.contains(&dxgi) => Some(RGBA32_BITS),
            Self::Dx10(_) | Self::OtherFourCc(_) => None,
            Self::OtherMask { bit_count, .. } => (bit_count != 0).then_some(bit_count),
            format if format.block_bytes().is_some() => None,
            format => MASK_FORMATS.iter().find(|(named, ..)| *named == format).map(|&(_, _, bits, _)| bits),
        }
    }

    pub fn is_block_compressed(&self) -> bool {
        self.block_bytes().is_some()
    }

    /// Bytes of one `width` x `height` surface, when the format's layout is known.
    pub fn surface_bytes(&self, width: u32, height: u32) -> Option<u64> {
        let (width, height) = (u64::from(width.max(1)), u64::from(height.max(1)));
        let side = u64::from(BLOCK_SIDE);
        if let Some(block_bytes) = self.block_bytes() {
            return Some(width.div_ceil(side) * height.div_ceil(side) * u64::from(block_bytes));
        }
        self.bits_per_pixel().map(|bits| self.row_pitch(width as u32, bits) * height)
    }

    /// Bytes of one row of `width` pixels of an uncompressed format of `bits` per pixel.
    fn row_pitch(&self, width: u32, bits: u32) -> u64 {
        (u64::from(width) * u64::from(bits)).div_ceil(u64::from(BITS_PER_BYTE))
    }
}

/// Mip levels of the full chain down to 1x1: `floor(log2(max(w, h))) + 1`.
pub fn full_mip_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.max(height).max(1).leading_zeros()
}

/// The size of mip `level`.
pub fn mip_level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// A DDS as its header describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsDescription {
    pub header: RawHeader,
    pub format: DdsTextureFormat,
    /// The mip levels stored: the mip count field, or 1 when it is 0.
    pub mip_count: u32,
    /// Faces: 6 for a cube map with all faces, the face count of a partial one, else 1.
    pub faces: u32,
    /// Depth slices of a volume texture, else 1.
    pub depth: u32,
    /// Bytes of the magic, header and DX10 header.
    pub header_len: usize,
    /// Bytes the surfaces take, when the format's layout is known.
    pub data_len: Option<u64>,
}

impl DdsDescription {
    /// Parses the header of a DDS file (magic included).
    pub fn parse(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < DDS_HEADER_SIZE || !bytes.starts_with(DDS_MAGIC) {
            eyre::bail!("not a DDS file");
        }
        let mut cursor = Cursor::new(&bytes[DDS_MAGIC.len()..]);
        let header = RawHeader::read(&mut cursor).wrap_err("the DDS header is truncated")?;
        let format = DdsTextureFormat::of_header(&header);
        let header_len = DDS_HEADER_SIZE + if header.dx10.is_some() { DDS_DX10_HEADER_SIZE } else { 0 };
        let mip_count = header.mipmap_count.max(1);
        let faces = if header.caps2.contains(Caps2::CUBE_MAP) {
            (header.caps2 & Caps2::CUBE_MAP_ALL_FACES).bits().count_ones()
        } else {
            1
        };
        let volume = header.caps2.contains(Caps2::VOLUME);
        let depth = if volume { header.depth.max(1) } else { 1 };
        let mut data_len = Some(0u64);
        for level in 0..mip_count {
            let (width, height) = mip_level_size(header.width, header.height, level);
            let slices = if volume { u64::from((depth >> level).max(1)) } else { 1 };
            data_len = data_len
                .zip(format.surface_bytes(width, height))
                .map(|(total, bytes)| total + bytes * slices * u64::from(faces));
        }
        Ok(Self { header, format, mip_count, faces, depth, header_len, data_len })
    }

    pub fn width(&self) -> u32 {
        self.header.width
    }

    pub fn height(&self) -> u32 {
        self.header.height
    }

    pub fn is_cube_map(&self) -> bool {
        self.header.caps2.contains(Caps2::CUBE_MAP)
    }

    pub fn is_volume(&self) -> bool {
        self.header.caps2.contains(Caps2::VOLUME)
    }
}

// region: --- Surfaces

const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;
const OPAQUE: u8 = u8::MAX;
const BYTES_A8R8G8B8: usize = 4;
const BYTES_R8G8B8: usize = 3;
const BYTES_16BIT: usize = 2;
const CHANNEL_4BIT_MAX: u32 = 0xf;
const CHANNEL_5BIT_MAX: u32 = 0x1f;
const CHANNEL_8BIT_MAX: u32 = 0xff;
const A4R4G4B4_SHIFTS: [u32; RGBA_CHANNELS] = [8, 4, 0, 12];
const X1R5G5B5_SHIFTS: [u32; 3] = [10, 5, 0];
/// The X bit of X1R5G5B5 texels as the shipped files store it: clear (the four KR login backgrounds).
const X1R5G5B5_X_BIT: u16 = 0;

/// Expands an `n`-bit channel of maximum `max` to 8 bits, rounding (`c * 255 / max`).
fn expand_channel(value: u32, max: u32) -> u8 {
    ((value * CHANNEL_8BIT_MAX + max / 2) / max) as u8
}

/// Quantizes an 8-bit channel to maximum `max`, rounding; the inverse of [`expand_channel`].
fn quantize_channel(value: u8, max: u32) -> u32 {
    (u32::from(value) * max + CHANNEL_8BIT_MAX / 2) / CHANNEL_8BIT_MAX
}

impl DdsTextureFormat {
    /// The `dds` crate format of a block-compressed format the writer encodes.
    fn bc_format(&self) -> Option<dds::Format> {
        match self {
            Self::Dxt1 => Some(dds::Format::BC1_UNORM),
            Self::Dxt3 => Some(dds::Format::BC2_UNORM),
            Self::Dxt5 => Some(dds::Format::BC3_UNORM),
            _ => None,
        }
    }

    /// Whether [`encode_surface`] and [`decode_surface`] handle the format: every format of the census.
    pub fn is_supported(&self) -> bool {
        self.bc_format().is_some()
            || matches!(self, Self::A8R8G8B8 | Self::R8G8B8 | Self::A4R4G4B4 | Self::X1R5G5B5)
    }
}

/// Encodes one `width` x `height` RGBA8 surface (row-major) to `format`.
pub fn encode_surface(format: DdsTextureFormat, width: u32, height: u32, rgba: &[u8]) -> eyre::Result<Vec<u8>> {
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * RGBA_CHANNELS {
        eyre::bail!("{} RGBA bytes for a {width}x{height} surface", rgba.len());
    }
    if let Some(bc_format) = format.bc_format() {
        let mut options = dds::EncodeOptions::default();
        options.quality = dds::CompressionQuality::High;
        let image = dds::ImageView::new(rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8)
            .ok_or_else(|| eyre!("bad RGBA surface of {width}x{height}"))?;
        let mut out = Vec::new();
        dds::encode(&mut out, image, bc_format, None, &options)?;
        return Ok(out);
    }
    let texels = rgba.chunks_exact(RGBA_CHANNELS);
    let out = match format {
        DdsTextureFormat::A8R8G8B8 => texels.flat_map(|texel| [texel[2], texel[1], texel[0], texel[ALPHA_CHANNEL]]).collect(),
        DdsTextureFormat::R8G8B8 => texels.flat_map(|texel| [texel[2], texel[1], texel[0]]).collect(),
        DdsTextureFormat::A4R4G4B4 => texels
            .flat_map(|texel| {
                let packed = (0..RGBA_CHANNELS)
                    .map(|channel| quantize_channel(texel[channel], CHANNEL_4BIT_MAX) << A4R4G4B4_SHIFTS[channel])
                    .fold(0, |packed, bits| packed | bits);
                (packed as u16).to_le_bytes()
            })
            .collect(),
        DdsTextureFormat::X1R5G5B5 => texels
            .flat_map(|texel| {
                let packed = (0..X1R5G5B5_SHIFTS.len())
                    .map(|channel| quantize_channel(texel[channel], CHANNEL_5BIT_MAX) << X1R5G5B5_SHIFTS[channel])
                    .fold(0, |packed, bits| packed | bits);
                (packed as u16 | X1R5G5B5_X_BIT).to_le_bytes()
            })
            .collect(),
        other => eyre::bail!("the writer does not encode {}", other.label()),
    };
    Ok(out)
}

/// Decodes one `width` x `height` surface of `format` to RGBA8 (row-major). X1R5G5B5 decodes opaque.
pub fn decode_surface(format: DdsTextureFormat, width: u32, height: u32, data: &[u8]) -> eyre::Result<Vec<u8>> {
    let expected = format.surface_bytes(width, height).ok_or_else(|| eyre!("{} has no known layout", format.label()))?;
    if data.len() as u64 != expected {
        eyre::bail!("{} bytes for a {width}x{height} {} surface of {expected} bytes", data.len(), format.label());
    }
    let mut rgba = vec![0u8; width as usize * height as usize * RGBA_CHANNELS];
    if let Some(bc_format) = format.bc_format() {
        let image = dds::ImageViewMut::new(&mut rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8)
            .ok_or_else(|| eyre!("bad RGBA surface of {width}x{height}"))?;
        dds::decode(&mut Cursor::new(data), image, bc_format, &dds::DecodeOptions::default())?;
        return Ok(rgba);
    }
    let texels = rgba.chunks_exact_mut(RGBA_CHANNELS);
    match format {
        DdsTextureFormat::A8R8G8B8 => {
            for (texel, source) in texels.zip(data.chunks_exact(BYTES_A8R8G8B8)) {
                texel.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
        }
        DdsTextureFormat::R8G8B8 => {
            for (texel, source) in texels.zip(data.chunks_exact(BYTES_R8G8B8)) {
                texel.copy_from_slice(&[source[2], source[1], source[0], OPAQUE]);
            }
        }
        DdsTextureFormat::A4R4G4B4 => {
            for (texel, source) in texels.zip(data.chunks_exact(BYTES_16BIT)) {
                let packed = u32::from(u16::from_le_bytes([source[0], source[1]]));
                for channel in 0..RGBA_CHANNELS {
                    texel[channel] =
                        expand_channel(packed >> A4R4G4B4_SHIFTS[channel] & CHANNEL_4BIT_MAX, CHANNEL_4BIT_MAX);
                }
            }
        }
        DdsTextureFormat::X1R5G5B5 => {
            for (texel, source) in texels.zip(data.chunks_exact(BYTES_16BIT)) {
                let packed = u32::from(u16::from_le_bytes([source[0], source[1]]));
                for channel in 0..X1R5G5B5_SHIFTS.len() {
                    texel[channel] =
                        expand_channel(packed >> X1R5G5B5_SHIFTS[channel] & CHANNEL_5BIT_MAX, CHANNEL_5BIT_MAX);
                }
                texel[ALPHA_CHANNEL] = OPAQUE;
            }
        }
        other => eyre::bail!("the writer does not decode {}", other.label()),
    }
    Ok(rgba)
}

// endregion: --- Surfaces

// region: --- Mip chains

/// How a lower mip level is made from the level above: each texel averages the 2x2 texels above it (one texel
/// along a side of size 1). Alpha is always averaged as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipFilter {
    /// Averages the stored (sRGB-encoded) colour values.
    BoxGamma,
    /// Averages the colours in linear light (sRGB decoded, averaged, re-encoded).
    BoxLinear,
}

const SRGB_LINEAR_THRESHOLD: f32 = 0.04045;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_SCALE: f32 = 1.055;
const SRGB_GAMMA: f32 = 2.4;
const LINEAR_SRGB_THRESHOLD: f32 = 0.003_130_8;
const CHANNEL_LEVELS: usize = 256;
const COLOR_CHANNELS: usize = 3;
const BOX_SIDE: u32 = 2;

fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / CHANNEL_8BIT_MAX as f32;
    if value <= SRGB_LINEAR_THRESHOLD {
        value / SRGB_LINEAR_SLOPE
    } else {
        ((value + SRGB_OFFSET) / SRGB_SCALE).powf(SRGB_GAMMA)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let encoded = if value <= LINEAR_SRGB_THRESHOLD {
        value * SRGB_LINEAR_SLOPE
    } else {
        SRGB_SCALE * value.powf(1.0 / SRGB_GAMMA) - SRGB_OFFSET
    };
    (encoded * CHANNEL_8BIT_MAX as f32).round().clamp(0.0, CHANNEL_8BIT_MAX as f32) as u8
}

/// The next mip level of a `width` x `height` RGBA8 level (row-major).
pub fn downsample_level(width: u32, height: u32, rgba: &[u8], filter: MipFilter) -> Vec<u8> {
    let (next_width, next_height) = mip_level_size(width, height, 1);
    let linear: Option<Vec<f32>> =
        (filter == MipFilter::BoxLinear).then(|| (0..CHANNEL_LEVELS).map(|value| srgb_to_linear(value as u8)).collect());
    let mut out = Vec::with_capacity(next_width as usize * next_height as usize * RGBA_CHANNELS);
    for y in 0..next_height {
        let rows: Vec<u32> = (0..BOX_SIDE).map(|dy| (y * BOX_SIDE + dy).min(height - 1)).collect();
        for x in 0..next_width {
            let columns: Vec<u32> = (0..BOX_SIDE).map(|dx| (x * BOX_SIDE + dx).min(width - 1)).collect();
            // A side of size 1 samples its one texel twice, which leaves the average unchanged.
            let texels: Vec<&[u8]> = rows
                .iter()
                .flat_map(|&row| {
                    columns.iter().map(move |&column| {
                        let at = (row as usize * width as usize + column as usize) * RGBA_CHANNELS;
                        &rgba[at..at + RGBA_CHANNELS]
                    })
                })
                .collect();
            let count = texels.len() as u32;
            for channel in 0..RGBA_CHANNELS {
                let value = match &linear {
                    Some(linear) if channel < COLOR_CHANNELS => {
                        let sum: f32 = texels.iter().map(|texel| linear[texel[channel] as usize]).sum();
                        linear_to_srgb(sum / count as f32)
                    }
                    _ => {
                        let sum: u32 = texels.iter().map(|texel| u32::from(texel[channel])).sum();
                        ((sum + count / 2) / count) as u8
                    }
                };
                out.push(value);
            }
        }
    }
    out
}

/// `mip_count` levels of a `width` x `height` RGBA8 image, mip 0 first; each level is filtered from the level
/// above at full precision (before encoding).
pub fn build_mip_chain(width: u32, height: u32, rgba: &[u8], mip_count: u32, filter: MipFilter) -> Vec<Vec<u8>> {
    let mut levels = vec![rgba.to_vec()];
    for level in 1..mip_count {
        let (above_width, above_height) = mip_level_size(width, height, level - 1);
        let next = downsample_level(above_width, above_height, &levels[level as usize - 1], filter);
        levels.push(next);
    }
    levels
}

// endregion: --- Mip chains

// region: --- Header conventions

/// What `pitchOrLinearSize` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PitchRule {
    /// The bytes of mip 0 (what DDSD_LINEARSIZE means for block-compressed formats; the shipped files also write
    /// it for uncompressed ones).
    MipZeroBytes,
    /// The bytes of one row of mip 0 (DDSD_PITCH).
    RowBytes,
    Zero,
}

/// How many mip levels a texture stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipRule {
    /// One level, with this mip count field (0 or 1).
    Single { field: u32 },
    /// The full chain down to 1x1 ([`full_mip_count`]).
    FullChain,
    /// This many levels, at most the full chain.
    Count(u32),
}

impl MipRule {
    /// The levels of a `width` x `height` texture.
    pub fn mip_count(&self, width: u32, height: u32) -> u32 {
        match *self {
            Self::Single { .. } => 1,
            Self::FullChain => full_mip_count(width, height),
            Self::Count(count) => count.clamp(1, full_mip_count(width, height)),
        }
    }
}

/// Everything a DDS header holds that does not depend on the image size, plus how the size-dependent fields
/// (`pitchOrLinearSize`, the mip count) are written. [`Self::of_header`] reads it from a shipped header and
/// [`Self::header`] writes the header of any size with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsHeaderConvention {
    pub flags: DdsFlags,
    pub caps: Caps,
    pub caps2: Caps2,
    pub pixel_format: dds::header::RawPixelFormat,
    pub depth: u32,
    pub reserved1: [u32; RESERVED1_DWORDS],
    pub caps3: u32,
    pub caps4: u32,
    pub reserved2: u32,
    pub pitch: PitchRule,
    pub mips: MipRule,
}

/// The header conventions of the shipped textures (the census in `ENHANCED_CLIENT_FILES.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DdsHeaderStyle {
    /// One level: CAPS | HEIGHT | WIDTH | PIXELFORMAT | LINEARSIZE, caps TEXTURE, `pitchOrLinearSize` the bytes
    /// of mip 0 (for uncompressed formats too), mip count field 0. Nearly every single-level texture.
    Linear,
    /// The full chain: [`Self::Linear`] plus MIPMAPCOUNT, caps COMPLEX | TEXTURE | MIPMAP.
    Mipmapped,
    /// [`Self::Mipmapped`] with `DDSX` in caps3 and 0xFFFFFFFF in caps4 and reserved2, and the pixel format bit
    /// count 0x100 for DXT3, 0x200 for DXT5 (some effect and interface textures).
    MipmappedDdsx,
    /// One level: CAPS | HEIGHT | WIDTH | PITCH | PIXELFORMAT, `pitchOrLinearSize` the row bytes, mip count
    /// field 1 (the unnamed A8R8G8B8 interface images).
    Pitched,
    /// One level: CAPS | HEIGHT | WIDTH | PIXELFORMAT, `pitchOrLinearSize` 0.
    Unsized,
}

const LINEAR_FLAGS: DdsFlags = DdsFlags::REQUIRED.union(DdsFlags::LINEAR_SIZE);
const MIPMAPPED_FLAGS: DdsFlags = LINEAR_FLAGS.union(DdsFlags::MIPMAP_COUNT);
const PITCHED_FLAGS: DdsFlags = DdsFlags::REQUIRED.union(DdsFlags::PITCH);
const MIPMAPPED_CAPS: Caps = Caps::TEXTURE.union(Caps::COMPLEX).union(Caps::MIPMAP);
const DDSX_CAPS3: u32 = u32::from_le_bytes(*b"DDSX");
const DDSX_FILL: u32 = u32::MAX;
/// The mip count field of single-level textures: 0 in [`DdsHeaderStyle::Linear`] and [`DdsHeaderStyle::Unsized`]
/// headers, 1 in [`DdsHeaderStyle::Pitched`] ones.
const LINEAR_MIP_FIELD: u32 = 0;
const PITCHED_MIP_FIELD: u32 = 1;
/// The bit count field `DDSX` headers write for these FourCC formats (0 for DXT1, as in the other headers).
const DDSX_FOURCC_BIT_COUNTS: [(DdsTextureFormat, u32); 2] =
    [(DdsTextureFormat::Dxt3, 0x100), (DdsTextureFormat::Dxt5, 0x200)];
const RESERVED1_DWORDS: usize = 11;

impl DdsHeaderStyle {
    pub const ALL: [Self; 5] = [Self::Linear, Self::Mipmapped, Self::MipmappedDdsx, Self::Pitched, Self::Unsized];

    /// The convention of this style for `format`, with the standard pixel format block.
    pub fn convention(self, format: DdsTextureFormat) -> eyre::Result<DdsHeaderConvention> {
        let single = |field| MipRule::Single { field };
        let (flags, caps, pitch, mips) = match self {
            Self::Linear => (LINEAR_FLAGS, Caps::TEXTURE, PitchRule::MipZeroBytes, single(LINEAR_MIP_FIELD)),
            Self::Mipmapped | Self::MipmappedDdsx => {
                (MIPMAPPED_FLAGS, MIPMAPPED_CAPS, PitchRule::MipZeroBytes, MipRule::FullChain)
            }
            Self::Pitched => (PITCHED_FLAGS, Caps::TEXTURE, PitchRule::RowBytes, single(PITCHED_MIP_FIELD)),
            Self::Unsized => (DdsFlags::REQUIRED, Caps::TEXTURE, PitchRule::Zero, single(LINEAR_MIP_FIELD)),
        };
        let ddsx = self == Self::MipmappedDdsx;
        let mut pixel_format = standard_pixel_format(format)?;
        if ddsx {
            if let Some(&(_, bit_count)) = DDSX_FOURCC_BIT_COUNTS.iter().find(|(named, _)| *named == format) {
                pixel_format.rgb_bit_count = bit_count;
            }
        }
        Ok(DdsHeaderConvention {
            flags,
            caps,
            caps2: Caps2::empty(),
            pixel_format,
            depth: 0,
            reserved1: [0; RESERVED1_DWORDS],
            caps3: if ddsx { DDSX_CAPS3 } else { 0 },
            caps4: if ddsx { DDSX_FILL } else { 0 },
            reserved2: if ddsx { DDSX_FILL } else { 0 },
            pitch,
            mips,
        })
    }

    /// The style whose convention for the header's format is `convention`, if any.
    pub fn of_convention(convention: &DdsHeaderConvention) -> Option<Self> {
        let format = DdsTextureFormat::of_pixel_format(&convention.pixel_format);
        Self::ALL.into_iter().find(|style| style.convention(format).is_ok_and(|preset| preset == *convention))
    }
}

/// The pixel format block of `format` as the shipped files write it: the FourCC alone, or the flags, bit count
/// and masks.
pub fn standard_pixel_format(format: DdsTextureFormat) -> eyre::Result<dds::header::RawPixelFormat> {
    let mut pixel_format = dds::header::RawPixelFormat {
        size: DDS_PIXEL_FORMAT_STRUCT_SIZE,
        flags: PixelFormatFlags::empty(),
        four_cc: FourCC::NONE,
        rgb_bit_count: 0,
        r_bit_mask: 0,
        g_bit_mask: 0,
        b_bit_mask: 0,
        a_bit_mask: 0,
    };
    if let Some((code, _)) = FOURCC_FORMATS.iter().find(|(_, named)| *named == format) {
        pixel_format.flags = PixelFormatFlags::FOURCC;
        pixel_format.four_cc = FourCC(u32::from_le_bytes(**code));
        return Ok(pixel_format);
    }
    let &(_, flags, bit_count, masks) = MASK_FORMATS
        .iter()
        .find(|(named, ..)| *named == format)
        .ok_or_else(|| eyre!("no standard pixel format block for {}", format.label()))?;
    pixel_format.flags = PixelFormatFlags::from_bits_retain(flags);
    pixel_format.rgb_bit_count = bit_count;
    [pixel_format.r_bit_mask, pixel_format.g_bit_mask, pixel_format.b_bit_mask, pixel_format.a_bit_mask] = masks;
    Ok(pixel_format)
}

impl DdsTextureFormat {
    fn of_pixel_format(pixel_format: &dds::header::RawPixelFormat) -> Self {
        let mut header = blank_header();
        header.pixel_format = pixel_format.clone();
        Self::of_header(&header)
    }
}

fn blank_header() -> RawHeader {
    RawHeader {
        size: DDS_HEADER_STRUCT_SIZE,
        flags: DdsFlags::empty(),
        height: 0,
        width: 0,
        pitch_or_linear_size: 0,
        depth: 0,
        mipmap_count: 0,
        reserved1: [0; RESERVED1_DWORDS],
        pixel_format: dds::header::RawPixelFormat {
            size: DDS_PIXEL_FORMAT_STRUCT_SIZE,
            flags: PixelFormatFlags::empty(),
            four_cc: FourCC::NONE,
            rgb_bit_count: 0,
            r_bit_mask: 0,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0,
        },
        caps: Caps::empty(),
        caps2: Caps2::empty(),
        caps3: 0,
        caps4: 0,
        reserved2: 0,
        dx10: None,
    }
}

impl DdsHeaderConvention {
    /// The convention of a 2D texture header. Fails for cube maps, volumes, DX10 headers and formats of unknown
    /// layout, which no client package holds.
    pub fn of_header(description: &DdsDescription) -> eyre::Result<Self> {
        let header = &description.header;
        if description.is_cube_map() || description.is_volume() || header.dx10.is_some() {
            eyre::bail!("only 2D textures without a DX10 header are supported");
        }
        if header.size != DDS_HEADER_STRUCT_SIZE {
            eyre::bail!("header size {} instead of {DDS_HEADER_STRUCT_SIZE}", header.size);
        }
        let format = description.format;
        let (width, height) = (description.width(), description.height());
        let value = u64::from(header.pitch_or_linear_size);
        let pitch = if value == 0 {
            PitchRule::Zero
        } else if format.surface_bytes(width, height) == Some(value) {
            PitchRule::MipZeroBytes
        } else if !format.is_block_compressed() && format.surface_bytes(width, 1) == Some(value) {
            PitchRule::RowBytes
        } else {
            eyre::bail!("pitchOrLinearSize {value} is neither mip 0 nor row bytes of {width}x{height} {}", format.label());
        };
        let mips = match header.mipmap_count {
            0 | 1 => MipRule::Single { field: header.mipmap_count },
            count if count == full_mip_count(width, height) => MipRule::FullChain,
            count => MipRule::Count(count),
        };
        Ok(Self {
            flags: header.flags,
            caps: header.caps,
            caps2: header.caps2,
            pixel_format: header.pixel_format.clone(),
            depth: header.depth,
            reserved1: header.reserved1,
            caps3: header.caps3,
            caps4: header.caps4,
            reserved2: header.reserved2,
            pitch,
            mips,
        })
    }

    pub fn format(&self) -> DdsTextureFormat {
        DdsTextureFormat::of_pixel_format(&self.pixel_format)
    }

    /// The header of a `width` x `height` texture with this convention.
    pub fn header(&self, width: u32, height: u32) -> eyre::Result<RawHeader> {
        let format = self.format();
        let mip_count = self.mips.mip_count(width, height);
        let bytes = |rows: u32| {
            format
                .surface_bytes(width, rows)
                .and_then(|bytes| u32::try_from(bytes).ok())
                .ok_or_else(|| eyre!("no layout for {width}x{height} {}", format.label()))
        };
        let pitch_or_linear_size = match self.pitch {
            PitchRule::MipZeroBytes => bytes(height)?,
            PitchRule::RowBytes => bytes(1)?,
            PitchRule::Zero => 0,
        };
        let mipmap_count = match self.mips {
            MipRule::Single { field } => field,
            _ => mip_count,
        };
        Ok(RawHeader {
            size: DDS_HEADER_STRUCT_SIZE,
            flags: self.flags,
            height,
            width,
            pitch_or_linear_size,
            depth: self.depth,
            mipmap_count,
            reserved1: self.reserved1,
            pixel_format: self.pixel_format.clone(),
            caps: self.caps,
            caps2: self.caps2,
            caps3: self.caps3,
            caps4: self.caps4,
            reserved2: self.reserved2,
            dx10: None,
        })
    }

    /// The magic and header bytes of a `width` x `height` texture with this convention.
    pub fn header_bytes(&self, width: u32, height: u32) -> eyre::Result<Vec<u8>> {
        let mut out = DDS_MAGIC.to_vec();
        self.header(width, height)?.write(&mut out)?;
        Ok(out)
    }
}

// endregion: --- Header conventions

// region: --- Textures

/// The filter [`replace_dds_entry`] builds mip chains with; the closer of the two to the shipped mips (see "DDS
/// formats in client packages" in `ENHANCED_CLIENT_FILES.md`).
pub const SHIPPED_MIP_FILTER: MipFilter = MipFilter::BoxGamma;

/// Writes a DDS of `image` with `convention`: its mip rule gives the levels, built with `filter`.
pub fn encode_dds(convention: &DdsHeaderConvention, image: &RgbaImage, filter: MipFilter) -> eyre::Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        eyre::bail!("empty image ({width}x{height})");
    }
    let format = convention.format();
    let mut out = convention.header_bytes(width, height)?;
    let mip_count = convention.mips.mip_count(width, height);
    for (level, rgba) in build_mip_chain(width, height, image.as_raw(), mip_count, filter).iter().enumerate() {
        let (level_width, level_height) = mip_level_size(width, height, level as u32);
        out.extend(encode_surface(format, level_width, level_height, rgba)?);
    }
    Ok(out)
}

/// The levels of a 2D DDS (mip 0 first), decoded to RGBA8.
pub fn decode_dds_levels(bytes: &[u8]) -> eyre::Result<(DdsDescription, Vec<RgbaImage>)> {
    let description = DdsDescription::parse(bytes)?;
    if description.is_cube_map() || description.is_volume() {
        eyre::bail!("only 2D textures are supported");
    }
    let mut levels = Vec::with_capacity(description.mip_count as usize);
    let mut offset = description.header_len;
    for level in 0..description.mip_count {
        let (width, height) = mip_level_size(description.width(), description.height(), level);
        let length = description
            .format
            .surface_bytes(width, height)
            .ok_or_else(|| eyre!("{} has no known layout", description.format.label()))? as usize;
        let data = bytes.get(offset..offset + length).ok_or_else(|| eyre!("the DDS ends inside mip {level}"))?;
        let rgba = decode_surface(description.format, width, height, data)?;
        levels.push(RgbaImage::from_raw(width, height, rgba).expect("the decoded surface has the level's size"));
        offset += length;
    }
    Ok((description, levels))
}

/// Whether [`replace_dds_entry`] accepts an image of another size than the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeChange {
    Refuse,
    /// Accepted with a warning: the client may assume the original size (UVs, gump and button layouts).
    Allow,
}

/// What [`replace_dds_entry`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsReplacement {
    pub bytes: Vec<u8>,
    pub format: DdsTextureFormat,
    pub mip_count: u32,
    /// The header style of the original, when it is one of the shipped ones.
    pub style: Option<DdsHeaderStyle>,
    /// The original size, when the replacement has another one.
    pub resized_from: Option<(u32, u32)>,
}

impl DdsReplacement {
    /// The warnings to show: a size change.
    pub fn warnings(&self) -> Vec<String> {
        self.resized_from
            .map(|(width, height)| {
                format!(
                    "the texture was {width}x{height}: code or layouts that assume that size (UVs, gump and button \
                     layouts) will stretch or crop it"
                )
            })
            .into_iter()
            .collect()
    }
}

/// A DDS with the pixels of `image` that keeps the original's pixel format, mip rule (one level or the full
/// chain for the new size) and header conventions; the mips are built with [`SHIPPED_MIP_FILTER`]. An image of
/// another size is refused unless `size_change` allows it.
pub fn replace_dds_entry(original: &[u8], image: &RgbaImage, size_change: SizeChange) -> eyre::Result<DdsReplacement> {
    let description = DdsDescription::parse(original).wrap_err("the original entry is not a readable DDS")?;
    let convention = DdsHeaderConvention::of_header(&description)?;
    let format = convention.format();
    if !format.is_supported() {
        eyre::bail!("the writer does not encode {}", format.label());
    }
    let original_size = (description.width(), description.height());
    let resized_from = (image.dimensions() != original_size).then_some(original_size);
    if resized_from.is_some() && size_change == SizeChange::Refuse {
        eyre::bail!(
            "the image is {}x{}, the texture {}x{}: a size change can break what assumes the original size (allow it \
             explicitly)",
            image.width(),
            image.height(),
            original_size.0,
            original_size.1
        );
    }
    let bytes = encode_dds(&convention, image, SHIPPED_MIP_FILTER)?;
    Ok(DdsReplacement {
        bytes,
        format,
        mip_count: convention.mips.mip_count(image.width(), image.height()),
        style: DdsHeaderStyle::of_convention(&convention),
        resized_from,
    })
}

/// Replaces the DDS entry `filename_hash` of the package `source` with `image` ([`replace_dds_entry`]) and saves
/// the package to `output`, which must not be `source`. The entry keeps its compression flag and gets a fresh
/// timestamp data block header; every other entry keeps its bytes and place in the file.
pub fn replace_texture_in_uop(
    source: &Path,
    filename_hash: u64,
    image: &RgbaImage,
    size_change: SizeChange,
    output: &Path,
) -> eyre::Result<DdsReplacement> {
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load_with_mode(source, LoadMode::Lazy)
        .wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let original = package
        .unpack_file_by_hash(filename_hash)
        .wrap_err_with(|| format!("failed to read entry {filename_hash:016X}"))?
        .ok_or_else(|| eyre!("{} has no entry {filename_hash:016X}", source.display()))?;
    let replacement = replace_dds_entry(&original, image, size_change)
        .wrap_err_with(|| format!("entry {filename_hash:016X}"))?;
    let compression = package.get_file_by_hash(filename_hash).expect("the entry was just read").compression();
    uop_entries::put_entry(&mut package, filename_hash, &replacement.bytes, compression)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(replacement)
}

// endregion: --- Textures

#[cfg(test)]
mod tests {
    use super::*;

    const DXT_SIZE: (u32, u32) = (64, 32);
    const NON_POW2_SIZE: (u32, u32) = (50, 4);
    const DXT1_BLOCK_BYTES: u32 = 8;
    const DXT5_BLOCK_BYTES: u32 = 16;
    const MID_GREY: u8 = 128;
    /// Black and white averaged in linear light, re-encoded to sRGB.
    const LINEAR_MID_GREY: u8 = 188;
    const UNCOMPRESSED: [DdsTextureFormat; 4] =
        [DdsTextureFormat::A8R8G8B8, DdsTextureFormat::R8G8B8, DdsTextureFormat::A4R4G4B4, DdsTextureFormat::X1R5G5B5];
    /// The largest error of a BC encoding of a smooth gradient, per channel.
    const GRADIENT_BC_MAX_ERROR: u8 = 16;

    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x * 4) as u8, (y * 8) as u8, ((x + y) * 2) as u8, OPAQUE])
        })
    }

    fn fields(bytes: &[u8]) -> RawHeader {
        RawHeader::read(&mut Cursor::new(&bytes[DDS_MAGIC.len()..])).unwrap()
    }

    #[test]
    fn channels_expand_and_quantize_exactly() {
        for max in [CHANNEL_4BIT_MAX, CHANNEL_5BIT_MAX] {
            for value in 0..=max {
                assert_eq!(quantize_channel(expand_channel(value, max), max), value, "{value} of {max}");
            }
            for value in 0..=u8::MAX {
                let quantized = quantize_channel(value, max);
                assert!(quantized <= max);
                let error = expand_channel(quantized, max).abs_diff(value);
                assert!(u32::from(error) <= CHANNEL_8BIT_MAX / max / 2 + 1, "{value} of {max}: error {error}");
            }
        }
    }

    #[test]
    fn uncompressed_surfaces_round_trip() {
        for format in UNCOMPRESSED {
            let (width, height) = NON_POW2_SIZE;
            let rgba = gradient(width, height).into_raw();
            let encoded = encode_surface(format, width, height, &rgba).unwrap();
            assert_eq!(encoded.len() as u64, format.surface_bytes(width, height).unwrap());
            let decoded = decode_surface(format, width, height, &encoded).unwrap();
            assert_eq!(encode_surface(format, width, height, &decoded).unwrap(), encoded, "{}", format.label());
        }
        let texel = [1, 2, 3, 4];
        assert_eq!(encode_surface(DdsTextureFormat::A8R8G8B8, 1, 1, &texel).unwrap(), [3, 2, 1, 4]);
        assert_eq!(encode_surface(DdsTextureFormat::R8G8B8, 1, 1, &texel).unwrap(), [3, 2, 1]);
    }

    #[test]
    fn block_compressed_surfaces_stay_close() {
        let (width, height) = DXT_SIZE;
        let rgba = gradient(width, height).into_raw();
        for format in [DdsTextureFormat::Dxt1, DdsTextureFormat::Dxt3, DdsTextureFormat::Dxt5] {
            let encoded = encode_surface(format, width, height, &rgba).unwrap();
            let decoded = decode_surface(format, width, height, &encoded).unwrap();
            let max = decoded.iter().zip(&rgba).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(max <= GRADIENT_BC_MAX_ERROR, "{}: max error {max}", format.label());
        }
    }

    #[test]
    fn mip_counts() {
        assert_eq!(full_mip_count(1, 1), 1);
        assert_eq!(full_mip_count(DXT_SIZE.0, DXT_SIZE.1), 7);
        assert_eq!(full_mip_count(NON_POW2_SIZE.0, NON_POW2_SIZE.1), 6);
        assert_eq!(mip_level_size(NON_POW2_SIZE.0, NON_POW2_SIZE.1, 3), (6, 1));
    }

    #[test]
    fn box_filters_average_in_their_space() {
        let checker: Vec<u8> = [0, OPAQUE, OPAQUE, 0].iter().flat_map(|&value| [value, value, value, OPAQUE]).collect();
        assert_eq!(downsample_level(2, 2, &checker, MipFilter::BoxGamma), [MID_GREY, MID_GREY, MID_GREY, OPAQUE]);
        assert_eq!(
            downsample_level(2, 2, &checker, MipFilter::BoxLinear),
            [LINEAR_MID_GREY, LINEAR_MID_GREY, LINEAR_MID_GREY, OPAQUE]
        );
        // An odd side keeps the last texel's own column.
        let row: Vec<u8> = [0, OPAQUE, OPAQUE].iter().flat_map(|&value| [value, 0, 0, value]).collect();
        assert_eq!(downsample_level(3, 1, &row, MipFilter::BoxGamma), [MID_GREY, 0, 0, MID_GREY]);
        let chain = build_mip_chain(DXT_SIZE.0, DXT_SIZE.1, gradient(DXT_SIZE.0, DXT_SIZE.1).as_raw(), 7, MipFilter::BoxGamma);
        assert_eq!(chain.last().unwrap().len(), RGBA_CHANNELS);
    }

    #[test]
    fn header_styles_write_the_shipped_fields() {
        let (width, height) = DXT_SIZE;
        let linear = DdsHeaderStyle::Linear.convention(DdsTextureFormat::Dxt5).unwrap();
        let header = fields(&linear.header_bytes(width, height).unwrap());
        assert_eq!(header.flags.bits(), 0x0008_1007);
        assert_eq!(header.caps.bits(), 0x1000);
        assert_eq!(header.mipmap_count, LINEAR_MIP_FIELD);
        assert_eq!(header.pitch_or_linear_size, width * height / (BLOCK_SIDE * BLOCK_SIDE) * DXT5_BLOCK_BYTES);

        let mipmapped = DdsHeaderStyle::Mipmapped.convention(DdsTextureFormat::Dxt1).unwrap();
        let header = fields(&mipmapped.header_bytes(width, height).unwrap());
        assert_eq!(header.flags.bits(), 0x000a_1007);
        assert_eq!(header.caps.bits(), 0x0040_1008);
        assert_eq!(header.mipmap_count, full_mip_count(width, height));
        assert_eq!(header.pitch_or_linear_size, width * height / (BLOCK_SIDE * BLOCK_SIDE) * DXT1_BLOCK_BYTES);

        let ddsx = DdsHeaderStyle::MipmappedDdsx.convention(DdsTextureFormat::Dxt5).unwrap();
        let header = fields(&ddsx.header_bytes(width, height).unwrap());
        assert_eq!((header.caps3, header.caps4, header.reserved2), (DDSX_CAPS3, DDSX_FILL, DDSX_FILL));
        assert_eq!(header.pixel_format.rgb_bit_count, 0x200);

        let (width, height) = NON_POW2_SIZE;
        let pitched = DdsHeaderStyle::Pitched.convention(DdsTextureFormat::A8R8G8B8).unwrap();
        let header = fields(&pitched.header_bytes(width, height).unwrap());
        assert_eq!(header.flags.bits(), 0x0000_100f);
        assert_eq!(header.mipmap_count, PITCHED_MIP_FIELD);
        assert_eq!(header.pitch_or_linear_size, width * BYTES_A8R8G8B8 as u32);
        assert_eq!(header.pixel_format.flags.bits(), 0x41);

        let unsized_header = DdsHeaderStyle::Unsized.convention(DdsTextureFormat::R8G8B8).unwrap();
        assert_eq!(fields(&unsized_header.header_bytes(width, height).unwrap()).pitch_or_linear_size, 0);
    }

    #[test]
    fn conventions_read_back_from_their_headers() {
        for style in DdsHeaderStyle::ALL {
            for format in [DdsTextureFormat::Dxt1, DdsTextureFormat::Dxt5]
                .into_iter()
                .chain(UNCOMPRESSED)
                .filter(|format| style != DdsHeaderStyle::Pitched || !format.is_block_compressed())
            {
                let convention = style.convention(format).unwrap();
                let bytes = convention.header_bytes(DXT_SIZE.0, DXT_SIZE.1).unwrap();
                let read = DdsHeaderConvention::of_header(&DdsDescription::parse(&bytes).unwrap()).unwrap();
                assert_eq!(read, convention, "{style:?} {}", format.label());
                assert_eq!(DdsHeaderStyle::of_convention(&read), Some(style));
                assert_eq!(read.format(), format);
            }
        }
    }

    fn texture(style: DdsHeaderStyle, format: DdsTextureFormat, (width, height): (u32, u32)) -> Vec<u8> {
        let convention = style.convention(format).unwrap();
        encode_dds(&convention, &gradient(width, height), MipFilter::BoxGamma).unwrap()
    }

    #[test]
    fn replacing_keeps_format_mips_and_header() {
        let original = texture(DdsHeaderStyle::Mipmapped, DdsTextureFormat::Dxt5, DXT_SIZE);
        let colour = [MID_GREY, 0, OPAQUE, OPAQUE];
        let image = RgbaImage::from_pixel(DXT_SIZE.0, DXT_SIZE.1, image::Rgba(colour));
        let replacement = replace_dds_entry(&original, &image, SizeChange::Refuse).unwrap();
        assert_eq!(replacement.bytes.len(), original.len());
        assert_eq!(replacement.bytes[..DDS_HEADER_SIZE], original[..DDS_HEADER_SIZE]);
        assert_eq!((replacement.format, replacement.style), (DdsTextureFormat::Dxt5, Some(DdsHeaderStyle::Mipmapped)));
        assert_eq!(replacement.mip_count, full_mip_count(DXT_SIZE.0, DXT_SIZE.1));
        let (_, levels) = decode_dds_levels(&replacement.bytes).unwrap();
        assert_eq!(levels.len() as u32, replacement.mip_count);
        let channels = levels.iter().flat_map(|level| level.pixels()).flat_map(|pixel| pixel.0.into_iter().zip(colour));
        assert!(channels.map(|(a, b)| a.abs_diff(b)).max().unwrap() <= GRADIENT_BC_MAX_ERROR);
        assert!(replacement.warnings().is_empty());

        let pitched = texture(DdsHeaderStyle::Pitched, DdsTextureFormat::A8R8G8B8, NON_POW2_SIZE);
        let replacement = replace_dds_entry(&pitched, &gradient(NON_POW2_SIZE.0, NON_POW2_SIZE.1), SizeChange::Refuse).unwrap();
        assert_eq!(replacement.bytes, pitched);
    }

    #[test]
    fn a_size_change_is_refused_unless_allowed() {
        let original = texture(DdsHeaderStyle::Mipmapped, DdsTextureFormat::Dxt1, DXT_SIZE);
        let larger = gradient(DXT_SIZE.0 * 2, DXT_SIZE.1);
        assert!(replace_dds_entry(&original, &larger, SizeChange::Refuse).is_err());
        let replacement = replace_dds_entry(&original, &larger, SizeChange::Allow).unwrap();
        assert_eq!(replacement.resized_from, Some(DXT_SIZE));
        assert_eq!(replacement.warnings().len(), 1);
        assert_eq!(replacement.mip_count, full_mip_count(larger.width(), larger.height()));
        let description = DdsDescription::parse(&replacement.bytes).unwrap();
        assert_eq!((description.width(), description.height()), larger.dimensions());
        assert_eq!(description.data_len.unwrap() as usize + DDS_HEADER_SIZE, replacement.bytes.len());

        let single = texture(DdsHeaderStyle::Linear, DdsTextureFormat::Dxt5, DXT_SIZE);
        let replacement = replace_dds_entry(&single, &larger, SizeChange::Allow).unwrap();
        assert_eq!(replacement.mip_count, 1);
    }
}
