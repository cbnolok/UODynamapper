use std::{borrow::Cow, error::Error, fmt, sync::Arc};

use wgpu_types::TextureFormat;

#[cfg(feature = "gpu")]
pub use block_compression::GpuBlockCompressor;

const VRAM_TEXTURE_CONTAINER_MAGIC: [u8; 4] = *b"UDT1";
const VRAM_TEXTURE_CONTAINER_VERSION: u16 = 1;
const VRAM_TEXTURE_CONTAINER_HEADER_LEN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawImageFormat {
    Rgb888,
    Rgba8888,
}

impl RawImageFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb888 => 3,
            Self::Rgba8888 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bc7EncoderBackend {
    BlockCompression,
    Ispc,
}

impl Bc7EncoderBackend {
    pub const fn is_available(self) -> bool {
        match self {
            Self::BlockCompression => true,
            Self::Ispc => cfg!(feature = "intel_tex"),
        }
    }

    pub const fn fallback(self) -> Self {
        match self {
            Self::BlockCompression => Self::BlockCompression,
            Self::Ispc => Self::BlockCompression,
        }
    }

    pub const fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::BlockCompression => None,
            Self::Ispc if cfg!(feature = "intel_tex") => None,
            Self::Ispc => Some(
                "this build does not include uddconv/intel_tex support for the Intel ISPC backend",
            ),
        }
    }
}

pub const fn is_bc7_encoder_backend_available(backend: Bc7EncoderBackend) -> bool {
    backend.is_available()
}

pub const fn resolve_bc7_encoder_backend(backend: Bc7EncoderBackend) -> Bc7EncoderBackend {
    if backend.is_available() {
        backend
    } else {
        backend.fallback()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VramTextureEncoding {
    Rgba8UnormSrgb,
    Bc7(Bc7EncoderBackend),
}

impl VramTextureEncoding {
    pub const fn format(self) -> VramTextureFormat {
        match self {
            Self::Rgba8UnormSrgb => VramTextureFormat::Rgba8UnormSrgb,
            Self::Bc7(_) => VramTextureFormat::Bc7RgbaUnormSrgb,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum VramTextureFormat {
    Rgba8UnormSrgb = 1,
    Bc7RgbaUnormSrgb = 2,
}

impl VramTextureFormat {
    pub const fn texture_format(self) -> TextureFormat {
        match self {
            Self::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            Self::Bc7RgbaUnormSrgb => TextureFormat::Bc7RgbaUnormSrgb,
        }
    }

    pub fn expected_byte_len(self, extent: ImageExtent) -> usize {
        match self {
            Self::Rgba8UnormSrgb => extent.byte_len(RawImageFormat::Rgba8888),
            Self::Bc7RgbaUnormSrgb => expected_bc7_byte_len(extent),
        }
    }

    pub const fn upload_layout(self, extent: ImageExtent) -> TextureUploadLayout {
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
        }
    }

    fn from_repr(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Rgba8UnormSrgb),
            2 => Some(Self::Bc7RgbaUnormSrgb),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageExtent {
    width: u32,
    height: u32,
}

impl ImageExtent {
    pub fn new(width: u32, height: u32) -> Result<Self, UddconvError> {
        if width == 0 || height == 0 {
            return Err(UddconvError::ZeroExtent { width, height });
        }

        Ok(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn blocks_wide(self) -> u32 {
        self.width.div_ceil(4)
    }

    pub const fn blocks_high(self) -> u32 {
        self.height.div_ceil(4)
    }

    pub const fn padded_width(self) -> u32 {
        self.blocks_wide() * 4
    }

    pub const fn padded_height(self) -> u32 {
        self.blocks_high() * 4
    }

    pub const fn bytes_per_row_rgba(self) -> usize {
        self.width as usize * 4
    }

    pub const fn byte_len(self, format: RawImageFormat) -> usize {
        self.width as usize * self.height as usize * format.bytes_per_pixel()
    }

    pub const fn padded_rgba_byte_len(self) -> usize {
        self.padded_width() as usize * self.padded_height() as usize * 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureUploadLayout {
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub format: TextureFormat,
}

pub type Bc7UploadLayout = TextureUploadLayout;

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
    ) -> Result<Self, UddconvError> {
        let expected_len = format.expected_byte_len(extent);
        if bytes.len() != expected_len {
            return Err(UddconvError::InvalidTexturePayloadLength {
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

    pub fn from_container_bytes(container: &[u8]) -> Result<Self, UddconvError> {
        if container.len() < VRAM_TEXTURE_CONTAINER_HEADER_LEN {
            return Err(UddconvError::TextureContainerTooShort {
                actual: container.len(),
                minimum: VRAM_TEXTURE_CONTAINER_HEADER_LEN,
            });
        }

        if container[..4] != VRAM_TEXTURE_CONTAINER_MAGIC {
            return Err(UddconvError::InvalidTextureContainerMagic);
        }

        let version = u16::from_le_bytes([container[4], container[5]]);
        if version != VRAM_TEXTURE_CONTAINER_VERSION {
            return Err(UddconvError::UnsupportedTextureContainerVersion { version });
        }

        let format_repr = u16::from_le_bytes([container[6], container[7]]);
        let Some(format) = VramTextureFormat::from_repr(format_repr) else {
            return Err(UddconvError::UnsupportedTextureContainerFormat { format: format_repr });
        };

        let width = u32::from_le_bytes([container[8], container[9], container[10], container[11]]);
        let height =
            u32::from_le_bytes([container[12], container[13], container[14], container[15]]);
        let payload_len =
            u32::from_le_bytes([container[16], container[17], container[18], container[19]]) as usize;
        let expected_len = VRAM_TEXTURE_CONTAINER_HEADER_LEN + payload_len;
        if container.len() != expected_len {
            return Err(UddconvError::InvalidTextureContainerLength {
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

impl Bc7TextureData {
    pub fn new(extent: ImageExtent, blocks: Vec<u8>) -> Result<Self, UddconvError> {
        let expected_len = expected_bc7_byte_len(extent);
        if blocks.len() != expected_len {
            return Err(UddconvError::InvalidBc7Length {
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
pub enum UddconvError {
    ZeroExtent { width: u32, height: u32 },
    InvalidInputLength {
        expected: usize,
        actual: usize,
        format: RawImageFormat,
    },
    InvalidBc7Length {
        expected: usize,
        actual: usize,
    },
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
    BackendUnavailable(Bc7EncoderBackend),
}

impl fmt::Display for UddconvError {
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
            Self::InvalidTextureContainerMagic => {
                write!(f, "invalid texture container magic")
            }
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
            Self::BackendUnavailable(backend) => {
                write!(f, "requested BC7 backend is unavailable in this build: {backend:?}")
            }
        }
    }
}

impl Error for UddconvError {}

pub fn expected_bc7_byte_len(extent: ImageExtent) -> usize {
    block_compression::CompressionVariant::BC7(block_compression::BC7Settings::alpha_basic())
        .blocks_byte_size(extent.width(), extent.height())
}

pub fn encode_to_bc7(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
) -> Result<Bc7TextureData, UddconvError> {
    validate_input_len(pixels, extent, input_format)?;
    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);

    let blocks = match backend {
        Bc7EncoderBackend::BlockCompression => {
            let variant = block_compression::CompressionVariant::BC7(
                block_compression::BC7Settings::alpha_basic(),
            );
            let mut blocks = vec![0u8; variant.blocks_byte_size(extent.width(), extent.height())];
            block_compression::encode::compress_rgba8(
                variant,
                rgba_pixels.as_ref(),
                &mut blocks,
                extent.width(),
                extent.height(),
                extent.width() * 4,
            );
            blocks
        }
        Bc7EncoderBackend::Ispc => encode_with_ispc(rgba_pixels.as_ref(), extent)?,
    };

    Bc7TextureData::new(extent, blocks)
}

pub fn encode_for_vram(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
) -> Result<VramTextureData, UddconvError> {
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
            Ok(encode_to_bc7(pixels, extent, input_format, backend)?.into())
        }
    }
}

pub fn encode_for_vram_arc(
    pixels: Arc<[u8]>,
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
) -> Result<VramTextureData, UddconvError> {
    match (encoding, input_format) {
        (VramTextureEncoding::Rgba8UnormSrgb, RawImageFormat::Rgba8888) => {
            validate_input_len(&pixels, extent, input_format)?;
            VramTextureData::new(extent, VramTextureFormat::Rgba8UnormSrgb, pixels)
        }
        _ => encode_for_vram(&pixels, extent, input_format, encoding),
    }
}

pub fn decode_from_vram(
    texture: &VramTextureData,
    output_format: RawImageFormat,
) -> Result<Vec<u8>, UddconvError> {
    match texture.format() {
        VramTextureFormat::Rgba8UnormSrgb => match output_format {
            RawImageFormat::Rgba8888 => Ok(texture.bytes().to_vec()),
            RawImageFormat::Rgb888 => Ok(rgba8888_to_rgb888(texture.bytes())),
        },
        VramTextureFormat::Bc7RgbaUnormSrgb => {
            decode_bc7(texture.bytes(), texture.extent(), output_format)
        }
    }
}

pub fn decode_bc7(
    blocks: &[u8],
    extent: ImageExtent,
    output_format: RawImageFormat,
) -> Result<Vec<u8>, UddconvError> {
    let expected_len = expected_bc7_byte_len(extent);
    if blocks.len() != expected_len {
        return Err(UddconvError::InvalidBc7Length {
            expected: expected_len,
            actual: blocks.len(),
        });
    }

    let padded_width = extent.padded_width();
    let padded_height = extent.padded_height();
    let variant =
        block_compression::CompressionVariant::BC7(block_compression::BC7Settings::alpha_basic());
    let mut padded_rgba = vec![0u8; extent.padded_rgba_byte_len()];
    block_compression::decode::decompress_blocks_as_rgba8(
        variant,
        padded_width,
        padded_height,
        blocks,
        &mut padded_rgba,
    );

    let rgba = crop_rgba8888(&padded_rgba, extent);
    Ok(match output_format {
        RawImageFormat::Rgba8888 => rgba,
        RawImageFormat::Rgb888 => rgba8888_to_rgb888(&rgba),
    })
}

pub fn decode_bc7_to_rgb888(
    blocks: &[u8],
    extent: ImageExtent,
) -> Result<Vec<u8>, UddconvError> {
    decode_bc7(blocks, extent, RawImageFormat::Rgb888)
}

pub fn decode_bc7_to_rgba8888(
    blocks: &[u8],
    extent: ImageExtent,
) -> Result<Vec<u8>, UddconvError> {
    decode_bc7(blocks, extent, RawImageFormat::Rgba8888)
}

fn validate_input_len(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
) -> Result<(), UddconvError> {
    let expected = extent.byte_len(input_format);
    if pixels.len() != expected {
        return Err(UddconvError::InvalidInputLength {
            expected,
            actual: pixels.len(),
            format: input_format,
        });
    }

    Ok(())
}

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

fn crop_rgba8888(padded_rgba: &[u8], extent: ImageExtent) -> Vec<u8> {
    let padded_row_len = extent.padded_width() as usize * 4;
    let row_len = extent.bytes_per_row_rgba();
    let mut rgba = Vec::with_capacity(extent.byte_len(RawImageFormat::Rgba8888));

    for row in 0..extent.height() as usize {
        let start = row * padded_row_len;
        rgba.extend_from_slice(&padded_rgba[start..start + row_len]);
    }

    rgba
}

fn rgba8888_to_rgb888(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for rgba_px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&rgba_px[..3]);
    }
    rgb
}

fn encode_with_ispc(
    rgba_pixels: &[u8],
    extent: ImageExtent,
) -> Result<Vec<u8>, UddconvError> {
    #[cfg(feature = "intel_tex")]
    {
        let surface = intel_tex_2::RgbaSurface {
            data: rgba_pixels,
            width: extent.width(),
            height: extent.height(),
            stride: extent.width() * 4,
        };
        let settings = intel_tex_2::bc7::alpha_basic_settings();
        Ok(intel_tex_2::bc7::compress_blocks(&settings, &surface))
    }

    #[cfg(not(feature = "intel_tex"))]
    {
        let _ = rgba_pixels;
        let _ = extent;
        Err(UddconvError::BackendUnavailable(Bc7EncoderBackend::Ispc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc7_upload_layout_matches_block_math() {
        let extent = ImageExtent::new(5, 7).unwrap();
        let encoded = Bc7TextureData::new(extent, vec![0u8; expected_bc7_byte_len(extent)]).unwrap();
        let layout = encoded.upload_layout();

        assert_eq!(layout.bytes_per_row, 32);
        assert_eq!(layout.rows_per_image, 2);
        assert_eq!(layout.format, TextureFormat::Bc7RgbaUnormSrgb);
    }

    #[test]
    fn rgb888_roundtrip_preserves_sizes() {
        let extent = ImageExtent::new(4, 4).unwrap();
        let rgb = vec![96u8; extent.byte_len(RawImageFormat::Rgb888)];
        let bc7 = encode_to_bc7(
            &rgb,
            extent,
            RawImageFormat::Rgb888,
            Bc7EncoderBackend::BlockCompression,
        )
        .unwrap();
        let decoded = decode_bc7_to_rgb888(bc7.blocks(), extent).unwrap();

        assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
        assert_eq!(decoded.len(), rgb.len());
    }

    #[test]
    fn rgba8888_roundtrip_preserves_sizes() {
        let extent = ImageExtent::new(4, 4).unwrap();
        let rgba = vec![255u8; extent.byte_len(RawImageFormat::Rgba8888)];
        let bc7 = encode_to_bc7(
            &rgba,
            extent,
            RawImageFormat::Rgba8888,
            Bc7EncoderBackend::BlockCompression,
        )
        .unwrap();
        let decoded = decode_bc7_to_rgba8888(bc7.blocks(), extent).unwrap();

        assert_eq!(bc7.blocks().len(), expected_bc7_byte_len(extent));
        assert_eq!(decoded.len(), rgba.len());
    }

    #[test]
    fn vram_texture_container_roundtrip_preserves_bc7_metadata() {
        let extent = ImageExtent::new(8, 4).unwrap();
        let rgba = vec![128u8; extent.byte_len(RawImageFormat::Rgba8888)];
        let texture = encode_for_vram(
            &rgba,
            extent,
            RawImageFormat::Rgba8888,
            VramTextureEncoding::Bc7(Bc7EncoderBackend::BlockCompression),
        )
        .unwrap();

        let container = texture.to_container_bytes();
        let decoded = VramTextureData::from_container_bytes(&container).unwrap();

        assert_eq!(decoded.extent(), extent);
        assert_eq!(decoded.format(), VramTextureFormat::Bc7RgbaUnormSrgb);
        assert_eq!(decoded.bytes().len(), expected_bc7_byte_len(extent));
    }

    #[test]
    fn encode_for_vram_arc_reuses_rgba_input_for_uncompressed_uploads() {
        let extent = ImageExtent::new(4, 4).unwrap();
        let rgba = Arc::<[u8]>::from(vec![42u8; extent.byte_len(RawImageFormat::Rgba8888)]);
        let texture = encode_for_vram_arc(
            Arc::clone(&rgba),
            extent,
            RawImageFormat::Rgba8888,
            VramTextureEncoding::Rgba8UnormSrgb,
        )
        .unwrap();

        assert!(Arc::ptr_eq(&texture.bytes_arc(), &rgba));
        assert_eq!(
            texture.upload_layout(),
            TextureUploadLayout {
                bytes_per_row: 16,
                rows_per_image: 4,
                format: TextureFormat::Rgba8UnormSrgb,
            }
        );
    }
}
