#[cfg(feature = "bc7-encode")]
use std::borrow::Cow;
#[cfg(feature = "bc7-encode")]
use std::time::{Duration, Instant};
use std::{error::Error, fmt, sync::Arc};
pub use wgpu_types::TextureFormat;

#[cfg(feature = "bc7-encode")]
#[path = "bc7/analytical.rs"]
pub mod analytical;
#[cfg(feature = "bc7-encode")]
#[path = "bc7/analytical_wide.rs"]
pub mod analytical_wide;
#[cfg(feature = "bc7-encode")]
#[path = "bc7/rdo.rs"]
pub mod rdo;
#[cfg(feature = "bc7-encode")]
#[path = "bc7/tables.rs"]
pub(crate) mod tables;

// Simple container used to persist pre-encoded VRAM textures on disk.
// Layout: magic(4) | version(2) | format(2) | width(4) | height(4) | payload_len(4)
const VRAM_TEXTURE_CONTAINER_MAGIC: [u8; 4] = *b"UDT1";
const VRAM_TEXTURE_CONTAINER_VERSION: u16 = 1;
const VRAM_TEXTURE_CONTAINER_HEADER_LEN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawImageFormat {
    Rgb888,
    Rgba8888,
}

impl RawImageFormat {
    pub const fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Rgb888 => 3,
            Self::Rgba8888 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bc7EncoderBackend {
    Analytical,
    AnalyticalWide,
}

pub trait Bc7EncoderBackendExt {
    fn is_available(&self) -> bool;
    fn unavailable_reason(&self) -> Option<&'static str>;
}

impl Bc7EncoderBackendExt for Bc7EncoderBackend {
    fn is_available(&self) -> bool {
        cfg!(feature = "bc7-encode")
    }

    fn unavailable_reason(&self) -> Option<&'static str> {
        if self.is_available() {
            None
        } else {
            Some("udd-image-codecs was built without the bc7-encode feature")
        }
    }
}

pub fn is_bc7_encoder_backend_available(backend: Bc7EncoderBackend) -> bool {
    backend.is_available()
}

pub fn resolve_bc7_encoder_backend(backend: Bc7EncoderBackend) -> Bc7EncoderBackend {
    backend
}

pub const fn preferred_bc7_encoder_backend() -> Bc7EncoderBackend {
    Bc7EncoderBackend::AnalyticalWide
}

pub const DEFAULT_BC7_RDO_LAMBDA: f32 = 0.05;
pub const DEFAULT_BC7_RDO_LOOKBACK_BLOCKS: usize = 64;
pub const BC7_RDO_LOOKBACK_BLOCK_PRESETS: [usize; 3] = [64, 128, 256];

#[cfg(feature = "bc7-encode")]
#[derive(Clone, Debug)]
pub struct Bc7RdoOptions {
    pub lambda: f32,
    pub lookback_blocks: usize,
    pub skip_zero_mse_blocks: bool,
}

#[cfg(feature = "bc7-encode")]
impl Bc7RdoOptions {
    pub fn new(lambda: f32, lookback_blocks: usize) -> Self {
        Self {
            lambda,
            lookback_blocks,
            skip_zero_mse_blocks: false,
        }
    }

    pub fn sparse_atlas(lambda: f32, lookback_blocks: usize) -> Self {
        Self {
            lambda,
            lookback_blocks,
            skip_zero_mse_blocks: true,
        }
    }

    fn to_rdo_params(&self) -> crate::bc7::rdo::Bc7RdoParams {
        crate::bc7::rdo::Bc7RdoParams {
            lambda: self.lambda,
            lookback_window_size: bc7_rdo_lookback_window_size(self.lookback_blocks),
            skip_zero_mse_blocks: self.skip_zero_mse_blocks,
            ..Default::default()
        }
    }
}

#[cfg(feature = "bc7-encode")]
impl Default for Bc7RdoOptions {
    fn default() -> Self {
        Self::new(DEFAULT_BC7_RDO_LAMBDA, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS)
    }
}

#[cfg(feature = "bc7-encode")]
const BC7_PROGRESS_BLOCK_BATCH: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VramTextureEncoding {
    Rgba8UnormSrgb,
    Bc7(Bc7EncoderBackend),
    /// BC1 through the `dds` crate ([`crate::bc1`]), with one bit of alpha.
    Bc1,
}

impl VramTextureEncoding {
    pub const fn format(&self) -> VramTextureFormat {
        match self {
            Self::Rgba8UnormSrgb => VramTextureFormat::Rgba8UnormSrgb,
            Self::Bc7(_) => VramTextureFormat::Bc7RgbaUnormSrgb,
            Self::Bc1 => VramTextureFormat::Bc1RgbaUnormSrgb,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum VramTextureFormat {
    Rgba8UnormSrgb = 1,
    Bc7RgbaUnormSrgb = 2,
    Bc1RgbaUnormSrgb = 3,
}

impl VramTextureFormat {
    pub const fn from_repr(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Rgba8UnormSrgb),
            2 => Some(Self::Bc7RgbaUnormSrgb),
            3 => Some(Self::Bc1RgbaUnormSrgb),
            _ => None,
        }
    }

    pub const fn texture_format(&self) -> TextureFormat {
        match self {
            Self::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            Self::Bc7RgbaUnormSrgb => TextureFormat::Bc7RgbaUnormSrgb,
            Self::Bc1RgbaUnormSrgb => TextureFormat::Bc1RgbaUnormSrgb,
        }
    }

    pub fn expected_byte_len(&self, extent: ImageExtent) -> usize {
        match self {
            Self::Rgba8UnormSrgb => extent.byte_len(RawImageFormat::Rgba8888),
            Self::Bc7RgbaUnormSrgb => expected_bc7_byte_len(extent),
            Self::Bc1RgbaUnormSrgb => crate::bc1::expected_bc1_byte_len(extent),
        }
    }

    pub const fn upload_layout(&self, extent: ImageExtent) -> TextureUploadLayout {
        match self {
            Self::Rgba8UnormSrgb => TextureUploadLayout {
                bytes_per_row: extent.width() * 4,
                rows_per_image: extent.height(),
                format: self.texture_format(),
            },
            Self::Bc7RgbaUnormSrgb => TextureUploadLayout {
                bytes_per_row: extent.blocks_wide() * 16,
                rows_per_image: extent.blocks_high(),
                format: self.texture_format(),
            },
            Self::Bc1RgbaUnormSrgb => TextureUploadLayout {
                bytes_per_row: extent.blocks_wide() * crate::bc1::BC1_BLOCK_BYTES as u32,
                rows_per_image: extent.blocks_high(),
                format: self.texture_format(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageExtent {
    width: u32,
    height: u32,
}

impl ImageExtent {
    pub fn new(width: u32, height: u32) -> Result<Self, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::ZeroExtent { width, height });
        }
        Ok(Self { width, height })
    }

    pub const fn width(&self) -> u32 { self.width }
    pub const fn height(&self) -> u32 { self.height }
    pub const fn blocks_wide(&self) -> u32 { self.width.div_ceil(4) }
    pub const fn blocks_high(&self) -> u32 { self.height.div_ceil(4) }
    pub const fn padded_width(&self) -> u32 { self.blocks_wide() * 4 }
    pub const fn padded_height(&self) -> u32 { self.blocks_high() * 4 }
    pub const fn bytes_per_row_rgba(&self) -> usize { self.width as usize * 4 }

    pub const fn byte_len(&self, format: RawImageFormat) -> usize {
        self.width as usize * self.height as usize * format.bytes_per_pixel()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureUploadLayout {
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub format: TextureFormat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VramTextureData {
    extent: ImageExtent,
    format: VramTextureFormat,
    bytes: Arc<[u8]>,
}

impl VramTextureData {
    pub fn new(
        extent: ImageExtent,
        format: VramTextureFormat,
        bytes: Arc<[u8]>,
    ) -> Result<Self, TextureError> {
        let expected_len = format.expected_byte_len(extent);
        if bytes.len() != expected_len {
            return Err(TextureError::InvalidTexturePayloadLength {
                expected: expected_len,
                actual: bytes.len(),
                format,
            });
        }

        Ok(Self {
            extent,
            format,
            bytes,
        })
    }

    pub const fn extent(&self) -> ImageExtent {
        self.extent
    }

    pub const fn format(&self) -> VramTextureFormat {
        self.format
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn bytes_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    pub fn into_bytes(self) -> Arc<[u8]> {
        self.bytes
    }

    pub const fn upload_layout(&self) -> TextureUploadLayout {
        self.format.upload_layout(self.extent)
    }

    pub fn to_container_bytes(&self) -> Vec<u8> {
        let mut container = Vec::with_capacity(VRAM_TEXTURE_CONTAINER_HEADER_LEN + self.bytes.len());
        container.extend_from_slice(&VRAM_TEXTURE_CONTAINER_MAGIC);
        container.extend_from_slice(&VRAM_TEXTURE_CONTAINER_VERSION.to_le_bytes());
        container.extend_from_slice(&(self.format as u16).to_le_bytes());
        container.extend_from_slice(&self.extent.width().to_le_bytes());
        container.extend_from_slice(&self.extent.height().to_le_bytes());
        container.extend_from_slice(&(self.bytes.len() as u32).to_le_bytes());
        container.extend_from_slice(self.bytes());
        container
    }

    pub fn from_container_bytes(container: &[u8]) -> Result<Self, TextureError> {
        if container.len() < VRAM_TEXTURE_CONTAINER_HEADER_LEN {
            return Err(TextureError::TextureContainerTooShort {
                actual: container.len(),
                minimum: VRAM_TEXTURE_CONTAINER_HEADER_LEN,
            });
        }

        if container[..4] != VRAM_TEXTURE_CONTAINER_MAGIC {
            return Err(TextureError::InvalidTextureContainerMagic);
        }

        let version = u16::from_le_bytes([container[4], container[5]]);
        if version != VRAM_TEXTURE_CONTAINER_VERSION {
            return Err(TextureError::UnsupportedTextureContainerVersion { version });
        }

        let format_repr = u16::from_le_bytes([container[6], container[7]]);
        let Some(format) = VramTextureFormat::from_repr(format_repr) else {
            return Err(TextureError::UnsupportedTextureContainerFormat { format: format_repr });
        };

        let width = u32::from_le_bytes([container[8], container[9], container[10], container[11]]);
        let height =
            u32::from_le_bytes([container[12], container[13], container[14], container[15]]);
        let payload_len =
            u32::from_le_bytes([container[16], container[17], container[18], container[19]]) as usize;
        let expected_len = VRAM_TEXTURE_CONTAINER_HEADER_LEN + payload_len;
        if container.len() != expected_len {
            return Err(TextureError::InvalidTextureContainerLength {
                expected: expected_len,
                actual: container.len(),
            });
        }

        let extent = ImageExtent::new(width, height)?;
        let payload = Arc::<[u8]>::from(&container[VRAM_TEXTURE_CONTAINER_HEADER_LEN..]);
        Self::new(extent, format, payload)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bc7TextureData {
    extent: ImageExtent,
    blocks: Vec<u8>,
}

#[cfg(feature = "bc7-encode")]
struct EncodedBc7Blocks {
    blocks: Vec<[u8; 16]>,
    rgba_blocks: Option<Vec<[u8; 4]>>,
}

impl Bc7TextureData {
    pub fn new(extent: ImageExtent, blocks: Vec<u8>) -> Result<Self, TextureError> {
        let expected_len = expected_bc7_byte_len(extent);
        if blocks.len() != expected_len {
            return Err(TextureError::InvalidBc7Length {
                expected: expected_len,
                actual: blocks.len(),
            });
        }

        Ok(Self { extent, blocks })
    }

    pub const fn extent(&self) -> ImageExtent {
        self.extent
    }

    pub fn blocks(&self) -> &[u8] {
        &self.blocks
    }

    pub fn into_blocks(self) -> Vec<u8> {
        self.blocks
    }

    pub const fn upload_layout(&self) -> TextureUploadLayout {
        VramTextureFormat::Bc7RgbaUnormSrgb.upload_layout(self.extent)
    }
}

impl From<Bc7TextureData> for VramTextureData {
    fn from(value: Bc7TextureData) -> Self {
        Self {
            extent: value.extent,
            format: VramTextureFormat::Bc7RgbaUnormSrgb,
            bytes: Arc::from(value.blocks),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureError {
    ZeroExtent { width: u32, height: u32 },
    InvalidInputLength { expected: usize, actual: usize, format: RawImageFormat },
    InvalidBc7Length { expected: usize, actual: usize },
    InvalidBc1Length { expected: usize, actual: usize },
    InvalidTexturePayloadLength {
        expected: usize,
        actual: usize,
        format: VramTextureFormat,
    },
    TextureContainerTooShort {
        actual: usize,
        minimum: usize,
    },
    InvalidTextureContainerMagic,
    UnsupportedTextureContainerVersion {
        version: u16,
    },
    UnsupportedTextureContainerFormat {
        format: u16,
    },
    InvalidTextureContainerLength {
        expected: usize,
        actual: usize,
    },
    BackendOperationFailed { backend: &'static str, operation: &'static str, message: String },
    RectOutOfBounds { rect: PixelRect, width: u32, height: u32 },
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroExtent { width, height } => {
                write!(f, "image extent must be non-zero, got {width}x{height}")
            }
            Self::InvalidInputLength {
                expected,
                actual,
                format,
            } => write!(
                f,
                "invalid input byte length for {format:?}: expected {expected}, got {actual}"
            ),
            Self::InvalidBc7Length { expected, actual } => write!(
                f,
                "invalid BC7 block payload length: expected {expected}, got {actual}"
            ),
            Self::InvalidBc1Length { expected, actual } => write!(
                f,
                "invalid BC1 block payload length: expected {expected}, got {actual}"
            ),
            Self::InvalidTexturePayloadLength {
                expected,
                actual,
                format,
            } => write!(
                f,
                "invalid VRAM texture payload length for {format:?}: expected {expected}, got {actual}"
            ),
            Self::TextureContainerTooShort { actual, minimum } => write!(
                f,
                "texture container is too short: expected at least {minimum} bytes, got {actual}"
            ),
            Self::InvalidTextureContainerMagic => write!(f, "invalid texture container magic"),
            Self::UnsupportedTextureContainerVersion { version } => write!(
                f,
                "unsupported texture container version: {version}"
            ),
            Self::UnsupportedTextureContainerFormat { format } => write!(
                f,
                "unsupported texture container format: {format}"
            ),
            Self::InvalidTextureContainerLength { expected, actual } => write!(
                f,
                "invalid texture container length: expected {expected}, got {actual}"
            ),
            Self::BackendOperationFailed {
                backend,
                operation,
                message,
            } => write!(
                f,
                "texture backend {backend} failed during {operation}: {message}"
            ),
            Self::RectOutOfBounds { rect, width, height } => write!(
                f,
                "rect {}x{} at ({}, {}) is outside the {width}x{height} image",
                rect.width, rect.height, rect.x, rect.y
            ),
        }
    }
}
impl Error for TextureError {}

pub fn expected_bc7_byte_len(extent: ImageExtent) -> usize {
    extent.blocks_wide() as usize * extent.blocks_high() as usize * 16
}

pub fn extract_bc7_subrect(
    src_blocks: &[u8],
    src_extent: ImageExtent,
    src_x: u32,
    src_y: u32,
    dst_extent: ImageExtent,
) -> Vec<u8> {
    assert!(src_x % 4 == 0);
    assert!(src_y % 4 == 0);
    assert!(dst_extent.width() % 4 == 0);
    assert!(dst_extent.height() % 4 == 0);

    let src_blocks_wide = src_extent.blocks_wide();
    let dst_blocks_wide = dst_extent.blocks_wide();
    let dst_blocks_high = dst_extent.blocks_high();

    let start_block_x = src_x / 4;
    let start_block_y = src_y / 4;

    let mut dst_blocks = vec![0u8; expected_bc7_byte_len(dst_extent)];

    for by in 0..dst_blocks_high {
        let src_row_y = start_block_y + by;
        let src_offset = (src_row_y * src_blocks_wide + start_block_x) as usize * 16;
        let dst_offset = (by * dst_blocks_wide) as usize * 16;
        let row_len = dst_blocks_wide as usize * 16;

        dst_blocks[dst_offset..dst_offset + row_len]
            .copy_from_slice(&src_blocks[src_offset..src_offset + row_len]);
    }

    dst_blocks
}

pub fn extract_rgba8_subrect(
    src_pixels: &[u8],
    src_width: u32,
    src_x: u32,
    src_y: u32,
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    let mut dst_pixels = vec![0u8; (dst_width * dst_height * 4) as usize];
    for y in 0..dst_height {
        let src_offset = ((src_y + y) * src_width + src_x) as usize * 4;
        let dst_offset = (y * dst_width) as usize * 4;
        let row_len = dst_width as usize * 4;
        dst_pixels[dst_offset..dst_offset + row_len]
            .copy_from_slice(&src_pixels[src_offset..src_offset + row_len]);
    }
    dst_pixels
}

pub fn decode_bc7_to_rgba8888(blocks: &[u8], extent: ImageExtent) -> Result<Vec<u8>, TextureError> {
    decode_bc7(blocks, extent, RawImageFormat::Rgba8888)
}

/// Pixels per side of a BC7 block, and bytes per block.
pub const BC7_BLOCK_EDGE: u32 = 4;
pub const BC7_BLOCK_BYTES: usize = 16;

/// A pixel rectangle of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Decodes only the blocks of `blocks` (a BC7 image of `extent`) that cover `rect`, and returns the rect's
/// RGBA8888 pixels: the same bytes as [`decode_bc7_to_rgba8888`] of the whole image cropped to `rect`, since
/// each block decodes on its own.
pub fn decode_bc7_rect_to_rgba8888(
    blocks: &[u8],
    extent: ImageExtent,
    rect: PixelRect,
) -> Result<Vec<u8>, TextureError> {
    let expected_len = expected_bc7_byte_len(extent);
    if blocks.len() != expected_len {
        return Err(TextureError::InvalidBc7Length { expected: expected_len, actual: blocks.len() });
    }
    let right = rect.x.checked_add(rect.width);
    let bottom = rect.y.checked_add(rect.height);
    if !right.is_some_and(|right| right <= extent.width()) || !bottom.is_some_and(|bottom| bottom <= extent.height()) {
        return Err(TextureError::RectOutOfBounds { rect, width: extent.width(), height: extent.height() });
    }
    if rect.width == 0 || rect.height == 0 {
        return Ok(Vec::new());
    }

    let (first_block_x, first_block_y) = (rect.x / BC7_BLOCK_EDGE, rect.y / BC7_BLOCK_EDGE);
    let blocks_wide = (rect.x + rect.width).div_ceil(BC7_BLOCK_EDGE) - first_block_x;
    let blocks_high = (rect.y + rect.height).div_ceil(BC7_BLOCK_EDGE) - first_block_y;
    let row_len = blocks_wide as usize * BC7_BLOCK_BYTES;
    let mut covering = Vec::with_capacity(row_len * blocks_high as usize);
    for block_y in first_block_y..first_block_y + blocks_high {
        let start = (block_y as usize * extent.blocks_wide() as usize + first_block_x as usize) * BC7_BLOCK_BYTES;
        covering.extend_from_slice(&blocks[start..start + row_len]);
    }

    let covering_extent = ImageExtent::new(blocks_wide * BC7_BLOCK_EDGE, blocks_high * BC7_BLOCK_EDGE)?;
    let rgba = decode_bc7_to_rgba8888(&covering, covering_extent)?;
    Ok(extract_rgba8_subrect(
        &rgba,
        covering_extent.width(),
        rect.x - first_block_x * BC7_BLOCK_EDGE,
        rect.y - first_block_y * BC7_BLOCK_EDGE,
        rect.width,
        rect.height,
    ))
}

pub fn decode_bc7_to_rgb888(
    blocks: &[u8],
    extent: ImageExtent,
) -> Result<Vec<u8>, TextureError> {
    decode_bc7(blocks, extent, RawImageFormat::Rgb888)
}

pub fn decode_from_vram(
    texture: &VramTextureData,
    output_format: RawImageFormat,
) -> Result<Vec<u8>, TextureError> {
    match texture.format() {
        VramTextureFormat::Rgba8UnormSrgb => match output_format {
            RawImageFormat::Rgba8888 => Ok(texture.bytes().to_vec()),
            RawImageFormat::Rgb888 => Ok(rgba8888_to_rgb888(texture.bytes())),
        },
        VramTextureFormat::Bc7RgbaUnormSrgb => {
            decode_bc7(texture.bytes(), texture.extent(), output_format)
        }
        VramTextureFormat::Bc1RgbaUnormSrgb => {
            let rgba = crate::bc1::decode_bc1_to_rgba8888(texture.bytes(), texture.extent())?;
            Ok(match output_format {
                RawImageFormat::Rgba8888 => rgba,
                RawImageFormat::Rgb888 => rgba8888_to_rgb888(&rgba),
            })
        }
    }
}

pub fn decode_bc7(
    blocks: &[u8],
    extent: ImageExtent,
    output_format: RawImageFormat,
) -> Result<Vec<u8>, TextureError> {
    use dds::{ColorFormat, DecodeOptions, Format, ImageViewMut, Size};
    use std::io::Cursor;

    let expected_len = expected_bc7_byte_len(extent);
    if blocks.len() != expected_len {
        return Err(TextureError::InvalidBc7Length {
            expected: expected_len,
            actual: blocks.len(),
        });
    }

    let mut rgba = vec![0u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let size = Size::new(extent.width(), extent.height());
    let image = ImageViewMut::new(&mut rgba, size, ColorFormat::RGBA_U8).ok_or_else(|| {
        TextureError::BackendOperationFailed {
            backend: "dds decode",
            operation: "decode",
            message: "failed to construct RGBA8 output view".to_string(),
        }
    })?;
    let options = DecodeOptions::default();
    let mut reader = Cursor::new(blocks);
    dds::decode(&mut reader, image, Format::BC7_UNORM, &options).map_err(|error| {
        TextureError::BackendOperationFailed {
            backend: "dds decode",
            operation: "decode",
            message: error.to_string(),
        }
    })?;

    Ok(match output_format {
        RawImageFormat::Rgba8888 => rgba,
        RawImageFormat::Rgb888 => rgba8888_to_rgb888(&rgba),
    })
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
) -> Result<Bc7TextureData, TextureError> {
    encode_to_bc7_with_rdo_lambda(pixels, extent, input_format, backend, 0.0)
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_lambda(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
) -> Result<Bc7TextureData, TextureError> {
    encode_to_bc7_with_rdo(
        pixels,
        extent,
        input_format,
        backend,
        rdo_lambda,
        DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
    rdo_lookback_blocks: usize,
) -> Result<Bc7TextureData, TextureError> {
    encode_to_bc7_with_rdo_options(
        pixels,
        extent,
        input_format,
        backend,
        &Bc7RdoOptions::new(rdo_lambda, rdo_lookback_blocks),
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_options(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_options: &Bc7RdoOptions,
) -> Result<Bc7TextureData, TextureError> {
    validate_input_len(pixels, extent, input_format)?;
    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);
    let backend = resolve_bc7_encoder_backend(backend);

    let mut encoded = match backend {
        Bc7EncoderBackend::Analytical => encode_with_analytical(rgba_pixels.as_ref(), extent),
        Bc7EncoderBackend::AnalyticalWide => {
            encode_with_analytical_wide(rgba_pixels.as_ref(), extent)
        }
    };
    apply_bc7_rdo(
        &mut encoded,
        rgba_pixels.as_ref(),
        extent,
        rdo_options,
    );

    Bc7TextureData::new(extent, flatten_bc7_blocks(encoded.blocks))
}

#[cfg(feature = "bc7-encode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bc7ProgressStage {
    Encode,
    Rdo,
}

#[cfg(feature = "bc7-encode")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bc7StageTimings {
    pub input: Duration,
    pub encode: Duration,
    pub rdo: Duration,
    pub flatten: Duration,
}

#[cfg(feature = "bc7-encode")]
impl Bc7StageTimings {
    pub fn add_assign(&mut self, other: Self) {
        self.input += other.input;
        self.encode += other.encode;
        self.rdo += other.rdo;
        self.flatten += other.flatten;
    }

    pub fn total(&self) -> Duration {
        self.input + self.encode + self.rdo + self.flatten
    }
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_lambda_and_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
    progress: F,
) -> Result<Bc7TextureData, TextureError>
where
    F: Fn(usize) + Sync,
{
    encode_to_bc7_with_rdo_lambda_and_stage_progress(
        pixels,
        extent,
        input_format,
        backend,
        rdo_lambda,
        |_, units| progress(units),
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_lambda_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
    progress: F,
) -> Result<Bc7TextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    encode_to_bc7_with_rdo_and_stage_progress(
        pixels,
        extent,
        input_format,
        backend,
        rdo_lambda,
        DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
        progress,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
    rdo_lookback_blocks: usize,
    progress: F,
) -> Result<Bc7TextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    encode_to_bc7_with_rdo_options_and_stage_progress(
        pixels,
        extent,
        input_format,
        backend,
        &Bc7RdoOptions::new(rdo_lambda, rdo_lookback_blocks),
        progress,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_options_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_options: &Bc7RdoOptions,
    progress: F,
) -> Result<Bc7TextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    let (texture, _) = encode_to_bc7_with_rdo_options_and_stage_progress_timed(
        pixels,
        extent,
        input_format,
        backend,
        rdo_options,
        progress,
    )?;
    Ok(texture)
}

#[cfg(feature = "bc7-encode")]
pub fn encode_to_bc7_with_rdo_options_and_stage_progress_timed<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_options: &Bc7RdoOptions,
    progress: F,
) -> Result<(Bc7TextureData, Bc7StageTimings), TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    let input_timer = Instant::now();
    validate_input_len(pixels, extent, input_format)?;
    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);
    let input_time = input_timer.elapsed();
    let backend = resolve_bc7_encoder_backend(backend);
    let encode_progress = |units| progress(Bc7ProgressStage::Encode, units);

    let encode_timer = Instant::now();
    let mut encoded = match backend {
        Bc7EncoderBackend::Analytical => {
            encode_with_analytical_and_progress(rgba_pixels.as_ref(), extent, &encode_progress)
        }
        Bc7EncoderBackend::AnalyticalWide => {
            encode_with_analytical_wide_and_progress(rgba_pixels.as_ref(), extent, &encode_progress)
        }
    };
    let encode_time = encode_timer.elapsed();
    let rdo_progress = |units| progress(Bc7ProgressStage::Rdo, units);
    let rdo_timer = Instant::now();
    apply_bc7_rdo_with_progress(
        &mut encoded,
        rgba_pixels.as_ref(),
        extent,
        rdo_options,
        &rdo_progress,
    );
    let rdo_time = rdo_timer.elapsed();

    let flatten_timer = Instant::now();
    let texture = Bc7TextureData::new(extent, flatten_bc7_blocks(encoded.blocks))?;
    let flatten_time = flatten_timer.elapsed();

    Ok((
        texture,
        Bc7StageTimings {
            input: input_time,
            encode: encode_time,
            rdo: rdo_time,
            flatten: flatten_time,
        },
    ))
}

#[cfg(feature = "bc7-encode")]
pub fn bc7_encode_progress_units(extent: ImageExtent, rdo_lambda: f32) -> usize {
    let blocks = extent.blocks_wide() as usize * extent.blocks_high() as usize;
    if rdo_lambda > 0.0 && rdo_lambda.is_finite() {
        blocks * 2
    } else {
        blocks
    }
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
) -> Result<VramTextureData, TextureError> {
    encode_for_vram_with_bc7_rdo_lambda(pixels, extent, input_format, encoding, 0.0)
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_lambda(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
) -> Result<VramTextureData, TextureError> {
    encode_for_vram_with_bc7_rdo(
        pixels,
        extent,
        input_format,
        encoding,
        bc7_rdo_lambda,
        DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> Result<VramTextureData, TextureError> {
    encode_for_vram_with_bc7_rdo_options(
        pixels,
        extent,
        input_format,
        encoding,
        &Bc7RdoOptions::new(bc7_rdo_lambda, bc7_rdo_lookback_blocks),
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_options(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_options: &Bc7RdoOptions,
) -> Result<VramTextureData, TextureError> {
    match encoding {
        VramTextureEncoding::Rgba8UnormSrgb => {
            validate_input_len(pixels, extent, input_format)?;
            let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent).into_owned();
            VramTextureData::new(
                extent,
                VramTextureFormat::Rgba8UnormSrgb,
                Arc::from(rgba_pixels),
            )
        }
        VramTextureEncoding::Bc7(backend) => {
            Ok(encode_to_bc7_with_rdo_options(
                pixels,
                extent,
                input_format,
                backend,
                bc7_rdo_options,
            )?.into())
        }
        VramTextureEncoding::Bc1 => encode_bc1_for_vram(pixels, extent, input_format),
    }
}

#[cfg(feature = "bc7-encode")]
fn encode_bc1_for_vram(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
) -> Result<VramTextureData, TextureError> {
    validate_input_len(pixels, extent, input_format)?;
    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);
    let blocks = crate::bc1::encode_bc1(&rgba_pixels, extent)?;
    VramTextureData::new(extent, VramTextureFormat::Bc1RgbaUnormSrgb, Arc::from(blocks))
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_lambda_and_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
    progress: F,
) -> Result<VramTextureData, TextureError>
where
    F: Fn(usize) + Sync,
{
    encode_for_vram_with_bc7_rdo_lambda_and_stage_progress(
        pixels,
        extent,
        input_format,
        encoding,
        bc7_rdo_lambda,
        |_, units| progress(units),
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_lambda_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
    progress: F,
) -> Result<VramTextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    encode_for_vram_with_bc7_rdo_and_stage_progress(
        pixels,
        extent,
        input_format,
        encoding,
        bc7_rdo_lambda,
        DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
        progress,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
    progress: F,
) -> Result<VramTextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    encode_for_vram_with_bc7_rdo_options_and_stage_progress(
        pixels,
        extent,
        input_format,
        encoding,
        &Bc7RdoOptions::new(bc7_rdo_lambda, bc7_rdo_lookback_blocks),
        progress,
    )
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_options_and_stage_progress<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_options: &Bc7RdoOptions,
    progress: F,
) -> Result<VramTextureData, TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    let (texture, _) = encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed(
        pixels,
        extent,
        input_format,
        encoding,
        bc7_rdo_options,
        progress,
    )?;
    Ok(texture)
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_with_bc7_rdo_options_and_stage_progress_timed<F>(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_options: &Bc7RdoOptions,
    progress: F,
) -> Result<(VramTextureData, Bc7StageTimings), TextureError>
where
    F: Fn(Bc7ProgressStage, usize) + Sync,
{
    match encoding {
        VramTextureEncoding::Rgba8UnormSrgb => {
            validate_input_len(pixels, extent, input_format)?;
            let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent).into_owned();
            let texture = VramTextureData::new(
                extent,
                VramTextureFormat::Rgba8UnormSrgb,
                Arc::from(rgba_pixels),
            )?;
            Ok((texture, Bc7StageTimings::default()))
        }
        VramTextureEncoding::Bc7(backend) => {
            let (texture, timings) = encode_to_bc7_with_rdo_options_and_stage_progress_timed(
                pixels,
                extent,
                input_format,
                backend,
                bc7_rdo_options,
                progress,
            )?;
            Ok((texture.into(), timings))
        }
        VramTextureEncoding::Bc1 => {
            Ok((encode_bc1_for_vram(pixels, extent, input_format)?, Bc7StageTimings::default()))
        }
    }
}

#[cfg(feature = "bc7-encode")]
pub fn encode_for_vram_arc(
    pixels: Arc<[u8]>,
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
) -> Result<VramTextureData, TextureError> {
    match (encoding, input_format) {
        (VramTextureEncoding::Rgba8UnormSrgb, RawImageFormat::Rgba8888) => {
            validate_input_len(&pixels, extent, input_format)?;
            VramTextureData::new(extent, VramTextureFormat::Rgba8UnormSrgb, pixels)
        }
        _ => encode_for_vram(&pixels, extent, input_format, encoding),
    }
}

#[cfg(feature = "bc7-encode")]
fn validate_input_len(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
) -> Result<(), TextureError> {
    let expected = extent.byte_len(input_format);
    if pixels.len() != expected {
        return Err(TextureError::InvalidInputLength {
            expected,
            actual: pixels.len(),
            format: input_format,
        });
    }

    Ok(())
}

pub fn extract_rgba8888_subrect_arc(
    src_pixels: &[u8],
    src_width: u32,
    src_x: u32,
    src_y: u32,
    dst_width: u32,
    dst_height: u32,
) -> Arc<[u8]> {
    Arc::from(extract_rgba8_subrect(
        src_pixels, src_width, src_x, src_y, dst_width, dst_height,
    ))
}

#[cfg(feature = "bc7-encode")]
fn normalize_to_rgba8888<'a>(
    pixels: &'a [u8],
    input_format: RawImageFormat,
    extent: ImageExtent,
) -> Cow<'a, [u8]> {
    match input_format {
        RawImageFormat::Rgba8888 => Cow::Borrowed(pixels),
        RawImageFormat::Rgb888 => {
            let pixel_count = extent.width() as usize * extent.height() as usize;
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for rgb in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            Cow::Owned(rgba)
        }
    }
}

fn rgba8888_to_rgb888(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for rgba_px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&rgba_px[..3]);
    }
    rgb
}

#[cfg(feature = "bc7-encode")]
fn encode_with_analytical(rgba_pixels: &[u8], extent: ImageExtent) -> EncodedBc7Blocks {
    use crate::bc7::analytical::{
        pack_bc7_rgba, Pixel, FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6,
    };
    use rayon::prelude::*;

    let blocks_x = extent.blocks_wide() as usize;
    let blocks_y = extent.blocks_high() as usize;
    let mut blocks = vec![[0u8; 16]; blocks_x * blocks_y];
    let rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
    let flags = FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6;

    blocks
        .par_iter_mut()
        .zip(rgba_blocks.par_chunks_exact(16))
        .for_each(|(block, pixels)| {
            let pixels: &[Pixel; 16] = pixels
                .try_into()
                .expect("BC7 block-order conversion always emits 16 pixels per block");
            pack_bc7_rgba(block, pixels, flags);
        });

    EncodedBc7Blocks {
        blocks,
        rgba_blocks: Some(rgba_blocks),
    }
}

#[cfg(feature = "bc7-encode")]
fn encode_with_analytical_and_progress<F>(
    rgba_pixels: &[u8],
    extent: ImageExtent,
    progress: &F,
) -> EncodedBc7Blocks
where
    F: Fn(usize) + Sync,
{
    use crate::bc7::analytical::{
        pack_bc7_rgba, Pixel, FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6,
    };
    use rayon::prelude::*;

    let blocks_x = extent.blocks_wide() as usize;
    let blocks_y = extent.blocks_high() as usize;
    let mut blocks = vec![[0u8; 16]; blocks_x * blocks_y];
    let rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
    let flags = FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6;

    blocks
        .par_chunks_mut(BC7_PROGRESS_BLOCK_BATCH)
        .zip(rgba_blocks.par_chunks(BC7_PROGRESS_BLOCK_BATCH * 16))
        .for_each(|(block_chunk, pixel_chunk)| {
            for (block, pixels) in block_chunk.iter_mut().zip(pixel_chunk.chunks_exact(16)) {
                let pixels: &[Pixel; 16] = pixels
                    .try_into()
                    .expect("BC7 block-order conversion always emits 16 pixels per block");
                pack_bc7_rgba(block, pixels, flags);
            }
            progress(block_chunk.len());
        });

    EncodedBc7Blocks {
        blocks,
        rgba_blocks: Some(rgba_blocks),
    }
}

#[cfg(feature = "bc7-encode")]
fn encode_with_analytical_wide(rgba_pixels: &[u8], extent: ImageExtent) -> EncodedBc7Blocks {
    use crate::bc7::analytical::{FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6};

    let mut blocks = vec![[0u8; 16]; extent.blocks_wide() as usize * extent.blocks_high() as usize];
    crate::bc7::analytical_wide::pack_bc7_rgba_blocks_wide(
        blocks.as_flattened_mut(),
        rgba_pixels,
        extent.width(),
        extent.height(),
        FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6,
    );
    EncodedBc7Blocks {
        blocks,
        rgba_blocks: None,
    }
}

#[cfg(feature = "bc7-encode")]
fn encode_with_analytical_wide_and_progress<F>(
    rgba_pixels: &[u8],
    extent: ImageExtent,
    progress: &F,
) -> EncodedBc7Blocks
where
    F: Fn(usize) + Sync,
{
    use crate::bc7::analytical::{FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6};

    let mut blocks = vec![[0u8; 16]; extent.blocks_wide() as usize * extent.blocks_high() as usize];
    crate::bc7::analytical_wide::pack_bc7_rgba_blocks_wide_with_progress(
        blocks.as_flattened_mut(),
        rgba_pixels,
        extent.width(),
        extent.height(),
        FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6,
        progress,
    );
    EncodedBc7Blocks {
        blocks,
        rgba_blocks: None,
    }
}

#[cfg(feature = "bc7-encode")]
fn apply_bc7_rdo(
    encoded: &mut EncodedBc7Blocks,
    rgba_pixels: &[u8],
    extent: ImageExtent,
    rdo_options: &Bc7RdoOptions,
) {
    if rdo_options.lambda <= 0.0 || !rdo_options.lambda.is_finite() {
        return;
    }

    let owned_rgba_blocks;
    let rgba_blocks = match encoded.rgba_blocks.as_deref() {
        Some(rgba_blocks) => rgba_blocks,
        None => {
            owned_rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
            &owned_rgba_blocks
        }
    };
    let params = rdo_options.to_rdo_params();
    crate::bc7::rdo::reduce_entropy_bc7_parallel(
        &mut encoded.blocks,
        rgba_blocks,
        extent.blocks_wide() as usize,
        extent.blocks_high() as usize,
        &params,
    );
}

#[cfg(feature = "bc7-encode")]
fn apply_bc7_rdo_with_progress<F>(
    encoded: &mut EncodedBc7Blocks,
    rgba_pixels: &[u8],
    extent: ImageExtent,
    rdo_options: &Bc7RdoOptions,
    progress: &F,
) where
    F: Fn(usize) + Sync,
{
    if rdo_options.lambda <= 0.0 || !rdo_options.lambda.is_finite() {
        return;
    }

    let owned_rgba_blocks;
    let rgba_blocks = match encoded.rgba_blocks.as_deref() {
        Some(rgba_blocks) => rgba_blocks,
        None => {
            owned_rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
            &owned_rgba_blocks
        }
    };
    let params = rdo_options.to_rdo_params();
    crate::bc7::rdo::reduce_entropy_bc7_parallel_with_progress(
        &mut encoded.blocks,
        rgba_blocks,
        extent.blocks_wide() as usize,
        extent.blocks_high() as usize,
        &params,
        progress,
    );
}

#[cfg(feature = "bc7-encode")]
pub fn normalize_bc7_rdo_lookback_blocks(blocks: usize) -> usize {
    if BC7_RDO_LOOKBACK_BLOCK_PRESETS.contains(&blocks) {
        blocks
    } else {
        DEFAULT_BC7_RDO_LOOKBACK_BLOCKS
    }
}

#[cfg(feature = "bc7-encode")]
fn bc7_rdo_lookback_window_size(blocks: usize) -> usize {
    normalize_bc7_rdo_lookback_blocks(blocks) * 16
}

#[cfg(feature = "bc7-encode")]
fn rgba_pixels_to_block_order(rgba_pixels: &[u8], extent: ImageExtent) -> Vec<[u8; 4]> {
    let blocks_x = extent.blocks_wide();
    let blocks_y = extent.blocks_high();
    let mut rgba_blocks = Vec::with_capacity((blocks_x * blocks_y * 16) as usize);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            for y in 0..4 {
                let src_y = (block_y * 4 + y).min(extent.height() - 1);
                for x in 0..4 {
                    let src_x = (block_x * 4 + x).min(extent.width() - 1);
                    let offset = ((src_y * extent.width() + src_x) * 4) as usize;
                    rgba_blocks.push([
                        rgba_pixels[offset],
                        rgba_pixels[offset + 1],
                        rgba_pixels[offset + 2],
                        rgba_pixels[offset + 3],
                    ]);
                }
            }
        }
    }

    rgba_blocks
}

#[cfg(feature = "bc7-encode")]
fn flatten_bc7_blocks(blocks: Vec<[u8; 16]>) -> Vec<u8> {
    let mut flat = Vec::with_capacity(blocks.len() * 16);
    for block in blocks {
        flat.extend_from_slice(&block);
    }
    flat
}

#[cfg(all(test, feature = "bc7-encode"))]
mod tests {
    use super::*;

    #[test]
    fn solid_mode5_roundtrips_every_channel_value_exactly() {
        let extent = ImageExtent::new(4, 4).unwrap();
        for value in 0..=255u8 {
            let color = [value, 255 - value, value.wrapping_mul(73), value.rotate_left(3)];
            let pixels = color.repeat(16);
            let encoded = encode_to_bc7(&pixels, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::Analytical).unwrap();
            let decoded = decode_bc7_to_rgba8888(encoded.blocks(), extent).unwrap();
            assert_eq!(decoded, pixels, "solid source {color:?}");
        }
    }

    /// Mode 4 stores a 2-bit and a 3-bit weight plane, each in the field of its size, whichever plane the index
    /// flag gives to colour. The block goes through the `dds` decoder and is compared with the format's own rule.
    #[test]
    fn mode4_decodes_each_weight_plane_for_every_index_flag_and_rotation() {
        use tables::{BC7_WEIGHTS2, BC7_WEIGHTS3};

        const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
        const BLOCKS_PER_CASE: usize = 2048;
        const BLOCK_PIXELS: usize = (BC7_BLOCK_EDGE * BC7_BLOCK_EDGE) as usize;
        const ROTATIONS: u32 = 4;
        const COLOR_ENDPOINT_BITS: u32 = 5;
        const ALPHA_ENDPOINT_BITS: u32 = 6;
        const ALPHA_CHANNEL: usize = 3;
        const CHANNEL_BITS: u32 = 8;
        const WEIGHT_BITS: u32 = 6;
        const WEIGHT_ONE: u32 = 1 << WEIGHT_BITS;
        const WEIGHT_ROUNDING: u32 = WEIGHT_ONE / 2;

        fn expand(value: u32, bits: u32) -> u32 {
            let shifted = value << (CHANNEL_BITS - bits);
            shifted | (shifted >> bits)
        }
        fn interpolate(low: u32, high: u32, weight: u32) -> u8 {
            (((WEIGHT_ONE - weight) * low + weight * high + WEIGHT_ROUNDING) >> WEIGHT_BITS) as u8
        }

        let extent = ImageExtent::new(BC7_BLOCK_EDGE, BC7_BLOCK_EDGE).unwrap();
        let mut state = SEED;
        let mut next = |limit: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % limit as u64) as u32
        };
        for index_flag in [0u32, 1] {
            for rotation in 0..ROTATIONS {
                for _ in 0..BLOCKS_PER_CASE {
                    let mut endpoint = |bits: u32| next(1 << bits);
                    let low = [
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(ALPHA_ENDPOINT_BITS),
                    ];
                    let high = [
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(COLOR_ENDPOINT_BITS),
                        endpoint(ALPHA_ENDPOINT_BITS),
                    ];
                    let weights3: [u8; BLOCK_PIXELS] = std::array::from_fn(|_| next(BC7_WEIGHTS3.len()) as u8);
                    let weights2: [u8; BLOCK_PIXELS] = std::array::from_fn(|_| next(BC7_WEIGHTS2.len()) as u8);

                    let mut block = [0u8; BC7_BLOCK_BYTES];
                    analytical::encode_mode4(
                        &mut block, low[0], low[1], low[2], low[3], high[0], high[1], high[2], high[3], &weights3,
                        &weights2, rotation, index_flag,
                    );

                    let expected: Vec<u8> = (0..BLOCK_PIXELS)
                        .flat_map(|index| {
                            let three = BC7_WEIGHTS3[weights3[index] as usize];
                            let two = BC7_WEIGHTS2[weights2[index] as usize];
                            let (color_weight, alpha_weight) = if index_flag == 1 { (three, two) } else { (two, three) };
                            let mut pixel: [u8; 4] = std::array::from_fn(|channel| {
                                let (bits, weight) = if channel == ALPHA_CHANNEL {
                                    (ALPHA_ENDPOINT_BITS, alpha_weight)
                                } else {
                                    (COLOR_ENDPOINT_BITS, color_weight)
                                };
                                interpolate(expand(low[channel], bits), expand(high[channel], bits), weight)
                            });
                            if rotation > 0 {
                                pixel.swap(ALPHA_CHANNEL, rotation as usize - 1);
                            }
                            pixel
                        })
                        .collect();
                    assert_eq!(
                        decode_bc7_to_rgba8888(&block, extent).unwrap(),
                        expected,
                        "index flag {index_flag}, rotation {rotation}"
                    );
                }
            }
        }
    }

    /// A sprite's edge has alpha falling with colour, so the correlation test in `pack_bc7_rgba` doesn't offer alpha its
    /// own plane there. The encoder has to try that plane anyway, and keep it whenever it decodes closer. The error is
    /// measured through the `dds` decoder, not the encoder's own.
    #[test]
    fn alpha_plane_is_kept_when_it_decodes_closer_on_sprite_edges() {
        use analytical::{pack_bc7_rgba, pack_mode4_or_5, Pixel, FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6};

        const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
        const BLOCKS: usize = 4096;
        const BLOCK_PIXELS: usize = (BC7_BLOCK_EDGE * BC7_BLOCK_EDGE) as usize;
        const ALPHA_PLANE: usize = 3;
        const CHANNEL_RANGE: usize = 256;
        const COVERAGE_LEVELS: usize = 16;
        const MAX_COVERAGE_LEVEL: usize = COVERAGE_LEVELS - 1;
        const MAX_CHANNEL: usize = CHANNEL_RANGE - 1;
        const TRANSPARENT_ONE_IN: usize = 3;
        const COLOUR_SPREAD: usize = 32;

        let extent = ImageExtent::new(BC7_BLOCK_EDGE, BC7_BLOCK_EDGE).unwrap();
        let flags = FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6;
        let error_of = |block: &[u8; BC7_BLOCK_BYTES], pixels: &[Pixel; BLOCK_PIXELS]| -> u64 {
            let decoded = decode_bc7_to_rgba8888(block, extent).unwrap();
            decoded.iter().zip(pixels.iter().flatten()).map(|(d, s)| u64::from(d.abs_diff(*s)).pow(2)).sum()
        };
        let mut state = SEED;
        let mut next = |limit: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % limit as u64) as usize
        };

        for _ in 0..BLOCKS {
            // One colour with some spread, on coverage levels as Enhanced frames have them, beside transparent black.
            let base: [usize; 3] = std::array::from_fn(|_| next(CHANNEL_RANGE));
            let pixels: [Pixel; BLOCK_PIXELS] = std::array::from_fn(|_| {
                if next(TRANSPARENT_ONE_IN) == 0 {
                    return [0; 4];
                }
                let coverage = next(COVERAGE_LEVELS) * MAX_CHANNEL / MAX_COVERAGE_LEVEL;
                let mut channel = |value: usize| (value + next(COLOUR_SPREAD)).min(MAX_CHANNEL) as u8;
                let colour = [channel(base[0]), channel(base[1]), channel(base[2])];
                [colour[0], colour[1], colour[2], coverage as u8]
            });

            let mut chosen = [0u8; BC7_BLOCK_BYTES];
            pack_bc7_rgba(&mut chosen, &pixels, flags);
            let mut plane = [0u8; BC7_BLOCK_BYTES];
            pack_mode4_or_5(&mut plane, &pixels, ALPHA_PLANE);
            assert!(
                error_of(&chosen, &pixels) <= error_of(&plane, &pixels),
                "the block alpha's own plane encodes closer to {pixels:?}"
            );
        }
    }

    #[test]
    fn bc7_progress_counts_encode_and_rdo_passes_separately() {
        let extent = ImageExtent::new(8, 8).expect("extent");
        let blocks = extent.blocks_wide() as usize * extent.blocks_high() as usize;

        assert_eq!(bc7_encode_progress_units(extent, 0.0), blocks);
        assert_eq!(bc7_encode_progress_units(extent, 1.0), blocks * 2);
    }
}

#[cfg(test)]
mod rect_decode_tests {
    use super::*;

    /// Pseudo-random block bytes (xorshift), so that every BC7 mode and partition occurs.
    fn random_blocks(extent: ImageExtent, mut state: u64) -> Vec<u8> {
        (0..expected_bc7_byte_len(extent))
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    #[test]
    fn a_rect_decodes_as_the_whole_image_cropped() {
        const SEED: u64 = 0x2545_F491_4F6C_DD1D;
        // Not a multiple of the block size, so the last blocks are partly outside the image.
        let extent = ImageExtent::new(37, 22).unwrap();
        let blocks = random_blocks(extent, SEED);
        let whole = decode_bc7_to_rgba8888(&blocks, extent).unwrap();
        let rects = [(0, 0, 37, 22), (0, 0, 1, 1), (3, 5, 9, 7), (4, 4, 8, 8), (36, 21, 1, 1), (13, 0, 24, 22)];
        for (x, y, width, height) in rects {
            let rect = PixelRect { x, y, width, height };
            let cropped = extract_rgba8_subrect(&whole, extent.width(), x, y, width, height);
            assert_eq!(decode_bc7_rect_to_rgba8888(&blocks, extent, rect).unwrap(), cropped, "{rect:?}");
        }
    }

    #[test]
    fn a_rect_outside_the_image_or_a_short_payload_fails() {
        let extent = ImageExtent::new(8, 8).unwrap();
        let blocks = random_blocks(extent, 1);
        let outside = PixelRect { x: 5, y: 0, width: 4, height: 1 };
        assert!(matches!(
            decode_bc7_rect_to_rgba8888(&blocks, extent, outside),
            Err(TextureError::RectOutOfBounds { .. })
        ));
        let inside = PixelRect { x: 0, y: 0, width: 1, height: 1 };
        assert!(matches!(
            decode_bc7_rect_to_rgba8888(&blocks[1..], extent, inside),
            Err(TextureError::InvalidBc7Length { .. })
        ));
        assert!(decode_bc7_rect_to_rgba8888(&blocks, extent, PixelRect { width: 0, ..inside }).unwrap().is_empty());
    }
}
