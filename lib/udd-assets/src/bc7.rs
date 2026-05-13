use std::{error::Error, fmt};
pub use wgpu_types::TextureFormat;

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
    Dds,
    BlockCompression,
    Ispc,
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
    pub const fn from_repr(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Rgba8UnormSrgb),
            2 => Some(Self::Bc7RgbaUnormSrgb),
            _ => None,
        }
    }

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

    pub const fn width(self) -> u32 { self.width }
    pub const fn height(self) -> u32 { self.height }
    pub const fn blocks_wide(self) -> u32 { self.width.div_ceil(4) }
    pub const fn blocks_high(self) -> u32 { self.height.div_ceil(4) }
    pub const fn padded_width(self) -> u32 { self.blocks_wide() * 4 }
    pub const fn padded_height(self) -> u32 { self.blocks_high() * 4 }
    pub const fn bytes_per_row_rgba(self) -> usize { self.width as usize * 4 }

    pub const fn byte_len(self, format: RawImageFormat) -> usize {
        self.width as usize * self.height as usize * format.bytes_per_pixel()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureUploadLayout {
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub format: TextureFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureError {
    ZeroExtent { width: u32, height: u32 },
    InvalidInputLength { expected: usize, actual: usize, format: RawImageFormat },
    InvalidBc7Length { expected: usize, actual: usize },
    BackendOperationFailed { backend: &'static str, operation: &'static str, message: String },
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Error for TextureError {}

pub fn expected_bc7_byte_len(extent: ImageExtent) -> usize {
    extent.blocks_wide() as usize * extent.blocks_high() as usize * 16
}

pub fn decode_bc7_to_rgba8888(blocks: &[u8], extent: ImageExtent) -> Result<Vec<u8>, TextureError> {
    use dds::{ColorFormat, DecodeOptions, Format, ImageViewMut, Size};
    use std::io::Cursor;

    let expected_len = expected_bc7_byte_len(extent);
    if blocks.len() != expected_len {
        return Err(TextureError::InvalidBc7Length { expected: expected_len, actual: blocks.len() });
    }

    let mut rgba = vec![0u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let size = Size::new(extent.width(), extent.height());
    let image = ImageViewMut::new(&mut rgba, size, ColorFormat::RGBA_U8).ok_or_else(|| {
        TextureError::BackendOperationFailed { backend: "dds", operation: "decode", message: "failed to construct view".to_string() }
    })?;
    let options = DecodeOptions::default();
    let mut reader = Cursor::new(blocks);
    dds::decode(&mut reader, image, Format::BC7_UNORM, &options).map_err(|e| {
        TextureError::BackendOperationFailed { backend: "dds", operation: "decode", message: e.to_string() }
    })?;

    Ok(rgba)
}
