use std::{borrow::Cow, error::Error, fmt, sync::Arc};

pub use udd_assets::bc7::{
    decode_bc7_to_rgba8888, expected_bc7_byte_len, Bc7EncoderBackend, ImageExtent, RawImageFormat,
    TextureFormat, TextureUploadLayout, VramTextureEncoding, VramTextureFormat,
};

// Simple container used to persist pre-encoded VRAM textures on disk.
// Layout: magic(4) | version(2) | format(2) | width(4) | height(4) | payload_len(4)
const VRAM_TEXTURE_CONTAINER_MAGIC: [u8; 4] = *b"UDT1";
const VRAM_TEXTURE_CONTAINER_VERSION: u16 = 1;
const VRAM_TEXTURE_CONTAINER_HEADER_LEN: usize = 20;

pub trait Bc7EncoderBackendExt {
    fn is_available(self) -> bool;
    fn unavailable_reason(self) -> Option<&'static str>;
}

impl Bc7EncoderBackendExt for Bc7EncoderBackend {
    fn is_available(self) -> bool {
        match self {
            Self::Analytical => true,
        }
    }

    fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Analytical => None,
        }
    }
}

pub fn is_bc7_encoder_backend_available(backend: Bc7EncoderBackend) -> bool {
    backend.is_available()
}

pub fn resolve_bc7_encoder_backend(backend: Bc7EncoderBackend) -> Bc7EncoderBackend {
    if backend.is_available() {
        backend
    } else {
        match backend {
            Bc7EncoderBackend::Analytical => backend,
        }
    }
}

pub const fn preferred_bc7_encoder_backend() -> Bc7EncoderBackend {
    Bc7EncoderBackend::Analytical
}

pub const DEFAULT_BC7_RDO_LAMBDA: f32 = 0.05;

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

        // Keep the encoded payload immutable and cheap to clone across cache layers.

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
        // The container is intentionally tiny so it can be parsed without any external schema.
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

/// Raw BC7 block payload paired with the logical image extent it represents.
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

/// Errors shared by BC7 conversion, validation, and container serialization.
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
    BackendOperationFailed {
        backend: &'static str,
        operation: &'static str,
        message: String,
    },
    TextureError(udd_assets::bc7::TextureError),
}

impl From<udd_assets::bc7::TextureError> for UddconvError {
    fn from(err: udd_assets::bc7::TextureError) -> Self {
        Self::TextureError(err)
    }
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
            Self::BackendOperationFailed {
                backend,
                operation,
                message,
            } => write!(
                f,
                "BC7 backend {backend} failed during {operation}: {message}"
            ),
            Self::TextureError(err) => write!(f, "texture error: {err}"),
        }
    }
}

impl Error for UddconvError {}


pub fn encode_to_bc7(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
) -> Result<Bc7TextureData, UddconvError> {
    encode_to_bc7_with_rdo_lambda(pixels, extent, input_format, backend, 0.0)
}

pub fn encode_to_bc7_with_rdo_lambda(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    backend: Bc7EncoderBackend,
    rdo_lambda: f32,
) -> Result<Bc7TextureData, UddconvError> {
    validate_input_len(pixels, extent, input_format)?;
    // All encoders work on RGBA8 input, so normalize once before dispatch.
    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);
    let backend = resolve_bc7_encoder_backend(backend);

    let mut blocks = match backend {
        Bc7EncoderBackend::Analytical => encode_with_analytical(rgba_pixels.as_ref(), extent),
    };
    apply_bc7_rdo(&mut blocks, rgba_pixels.as_ref(), extent, rdo_lambda);

    Bc7TextureData::new(extent, blocks)
}

pub fn encode_for_vram(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
) -> Result<VramTextureData, UddconvError> {
    encode_for_vram_with_bc7_rdo_lambda(pixels, extent, input_format, encoding, 0.0)
}

pub fn encode_for_vram_with_bc7_rdo_lambda(
    pixels: &[u8],
    extent: ImageExtent,
    input_format: RawImageFormat,
    encoding: VramTextureEncoding,
    bc7_rdo_lambda: f32,
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
            Ok(encode_to_bc7_with_rdo_lambda(
                pixels,
                extent,
                input_format,
                backend,
                bc7_rdo_lambda,
            )?.into())
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
    use dds::{ColorFormat, DecodeOptions, Format, ImageViewMut, Size};
    use std::io::Cursor;

    let expected_len = expected_bc7_byte_len(extent);
    if blocks.len() != expected_len {
        return Err(UddconvError::InvalidBc7Length {
            expected: expected_len,
            actual: blocks.len(),
        });
    }

    // `dds` can decode BC7 directly from the raw block stream, so this path does not need
    // a temporary DDS container wrapper.
    let mut rgba = vec![0u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let size = Size::new(extent.width(), extent.height());
    let image = ImageViewMut::new(&mut rgba, size, ColorFormat::RGBA_U8).ok_or_else(|| {
        UddconvError::BackendOperationFailed {
            backend: "dds decode",
            operation: "decode",
            message: "failed to construct RGBA8 output view".to_string(),
        }
    })?;
    let options = DecodeOptions::default();
    let mut reader = Cursor::new(blocks);
    dds::decode(&mut reader, image, Format::BC7_UNORM, &options).map_err(|error| {
        UddconvError::BackendOperationFailed {
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

/// Extract a block-aligned sub-rect from a BC7 payload.
/// Both `src_rect` and `dst_extent` must be 4-pixel aligned.
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

pub fn decode_bc7_to_rgb888(
    blocks: &[u8],
    extent: ImageExtent,
) -> Result<Vec<u8>, UddconvError> {
    decode_bc7(blocks, extent, RawImageFormat::Rgb888)
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

fn normalize_to_rgba8888<'a>(
    pixels: &'a [u8],
    input_format: RawImageFormat,
    extent: ImageExtent,
) -> Cow<'a, [u8]> {
    match input_format {
        RawImageFormat::Rgba8888 => Cow::Borrowed(pixels),
        RawImageFormat::Rgb888 => {
            // BC7 encoders expect an explicit alpha channel. RGB input is treated as opaque.
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

fn bc7_block_byte_len(width: u32, height: u32) -> usize {
    // BC7 always stores one 16-byte block per 4x4 texel group.
    width.div_ceil(4) as usize * height.div_ceil(4) as usize * 16
}

fn encode_with_analytical(rgba_pixels: &[u8], extent: ImageExtent) -> Vec<u8> {
    use image_postprocess::bc7_analytical::{
        pack_bc7_rgba, Pixel, FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE,
    };

    let blocks_x = extent.blocks_wide() as usize;
    let blocks_y = extent.blocks_high() as usize;
    let mut blocks = vec![0u8; blocks_x * blocks_y * 16];
    let rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
    let flags = FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE;

    for block_index in 0..blocks_x * blocks_y {
        let pixels: &[Pixel; 16] = rgba_blocks[block_index * 16..(block_index + 1) * 16]
            .try_into()
            .expect("BC7 block-order conversion always emits 16 pixels per block");
        let block: &mut [u8; 16] = blocks[block_index * 16..(block_index + 1) * 16]
            .as_mut()
            .try_into()
            .expect("BC7 block buffer is allocated in 16-byte blocks");
        pack_bc7_rgba(block, pixels, flags);
    }

    blocks
}

fn apply_bc7_rdo(blocks: &mut [u8], rgba_pixels: &[u8], extent: ImageExtent, rdo_lambda: f32) {
    if rdo_lambda <= 0.0 || !rdo_lambda.is_finite() {
        return;
    }

    let mut block_arrays = Vec::with_capacity(blocks.len() / 16);
    for block in blocks.chunks_exact(16) {
        let mut block_array = [0u8; 16];
        block_array.copy_from_slice(block);
        block_arrays.push(block_array);
    }

    let rgba_blocks = rgba_pixels_to_block_order(rgba_pixels, extent);
    let params = image_postprocess::bc7_rdo::Bc7RdoParams {
        lambda: rdo_lambda,
        ..Default::default()
    };
    image_postprocess::bc7_rdo::reduce_entropy_bc7(
        &mut block_arrays,
        &rgba_blocks,
        extent.blocks_wide() as usize,
        extent.blocks_high() as usize,
        &params,
    );

    for (dst, src) in blocks.chunks_exact_mut(16).zip(block_arrays) {
        dst.copy_from_slice(&src);
    }
}

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
