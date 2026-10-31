//! Census of the images in client UOP packages, by content: every entry is decompressed and classified by its
//! bytes alone, never by its name ([`classify_content`]): a magic (DDS, PNG, JPEG, GIF, BMP with a consistent
//! header, PSD, KTX, RIFF/WAV, OGG, MP3, a zlib stream that inflates, Gamebryo NIF/KFM header text) or, for TGA,
//! which has none, a strict validation of the header and of the pixel data ([`validate_tga`]). The extension of
//! the entry name (from a hash dictionary) is the format the name declares; the report sets it against the content
//! and lists the disagreements. DDS headers are described field by field, and DDS files embedded inside other
//! payloads are listed. [`ImageCensus::to_markdown`] gives the report `uocf-cli image census` prints, which
//! `ENHANCED_CLIENT_FILES.md` summarizes ("Images in client packages").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::enhanced::dds_writer::{full_mip_count, DdsDescription, DdsTextureFormat, DDS_MAGIC};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::{LoadMode, UopPackage};

crate::eyre_imports!();

const UOP_EXTENSION: &str = "uop";
/// A DDS header inside another payload: the magic, then the header size 124.
const EMBEDDED_DDS_PATTERN: [u8; 8] = *b"DDS \x7c\0\0\0";
/// Examples kept per header convention, quirk or name/content disagreement.
const EXAMPLES_KEPT: usize = 3;
/// NVIDIA Texture Tools write `NVTT` and their version in the reserved dwords 9 and 10.
const NVTT_TAG: u32 = u32::from_le_bytes(*b"NVTT");
const NVTT_TAG_DWORD: usize = 9;
const NVTT_VERSION_DWORD: usize = 10;
const BITS_PER_BYTE: usize = 8;

// region: --- Content formats

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = &[0xff, 0xd8, 0xff];
const GIF_MAGICS: [&[u8]; 2] = [b"GIF87a", b"GIF89a"];
const PSD_MAGIC: &[u8] = b"8BPS";
/// PSD and PSB (large document) versions, big-endian after the magic.
const PSD_VERSIONS: [u16; 2] = [1, 2];
const PSD_VERSION_OFFSET: usize = 4;
const KTX_MAGIC: &[u8] = b"\xabKTX 11\xbb\r\n\x1a\n";
const KTX2_MAGIC: &[u8] = b"\xabKTX 20\xbb\r\n\x1a\n";
const RIFF_MAGIC: &[u8] = b"RIFF";
const RIFF_WAVE_FORM: &[u8] = b"WAVE";
const RIFF_FORM_OFFSET: usize = 8;
const OGG_MAGIC: &[u8] = b"OggS";
const ID3_MAGIC: &[u8] = b"ID3";
/// The header text of Gamebryo (and earlier NetImmerse) `.nif` and `.kf` files, and of `.kfm` files.
const NIF_MAGICS: [&[u8]; 2] = [b"Gamebryo File Format, Version ", b"NetImmerse File Format, Version"];
const KFM_MAGIC: &[u8] = b";Gamebryo KFM File Version ";
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const XML_OPEN: char = '<';

const BMP_MAGIC: &[u8] = b"BM";
const BMP_INFO_HEADER_SIZES: [u32; 6] = [12, 40, 52, 56, 108, 124];
const BMP_FILE_HEADER_SIZE: usize = 14;
const BMP_LENGTH_OFFSET: usize = 2;
const BMP_RESERVED_OFFSET: usize = 6;
const BMP_PIXELS_OFFSET: usize = 10;
const BMP_INFO_SIZE_OFFSET: usize = 14;
const BMP_WIDTH_OFFSET: usize = 18;
const BMP_HEIGHT_OFFSET: usize = 22;
const BMP_PLANES_OFFSET: usize = 26;
const BMP_BIT_COUNT_OFFSET: usize = 28;
const BMP_COMPRESSION_OFFSET: usize = 30;
const BMP_CORE_WIDTH_OFFSET: usize = 18;
const BMP_CORE_HEIGHT_OFFSET: usize = 20;
const BMP_CORE_PLANES_OFFSET: usize = 22;
const BMP_CORE_BIT_COUNT_OFFSET: usize = 24;
const BMP_CORE_HEADER_SIZE: u32 = 12;
const BMP_PLANES: u16 = 1;
const BMP_BIT_COUNTS: [u16; 6] = [1, 4, 8, 16, 24, 32];
/// `BI_RGB` and `BI_BITFIELDS`: pixels stored as rows padded to 32 bits, bottom-up or (negative height) top-down.
const BMP_UNCOMPRESSED: [u32; 2] = [0, 3];
const BMP_ROW_ALIGN_BITS: usize = 32;

/// The zlib header: compression method 8 (deflate) in the low nibble of CMF, a window of at most 2^15 (CMF high
/// nibble 7), CMF·256 + FLG a multiple of 31, no preset dictionary.
const ZLIB_HEADER_SIZE: usize = 2;
const ZLIB_METHOD_MASK: u8 = 0x0f;
const ZLIB_METHOD_DEFLATE: u8 = 8;
const ZLIB_WINDOW_SHIFT: u8 = 4;
const ZLIB_MAX_WINDOW: u8 = 7;
const ZLIB_CHECK_DIVISOR: u16 = 31;
const ZLIB_PRESET_DICTIONARY: u8 = 0x20;
/// The output buffer a zlib stream is inflated through (and discarded).
const ZLIB_PROBE_BUFFER_SIZE: usize = 64 * 1024;

/// An MPEG audio frame header: 11 sync bits, the version, the layer, then the bitrate, sample rate and padding.
const MPEG_SYNC_BYTE: u8 = 0xff;
const MPEG_SYNC_MASK: u8 = 0xe0;
const MPEG_VERSION_SHIFT: u8 = 3;
const MPEG_LAYER_SHIFT: u8 = 1;
const MPEG_TWO_BITS: u8 = 0b11;
const MPEG_VERSION_1: u8 = 0b11;
const MPEG_VERSION_2: u8 = 0b10;
const MPEG_VERSION_2_5: u8 = 0b00;
const MPEG_LAYER_1: u8 = 0b11;
const MPEG_LAYER_2: u8 = 0b10;
const MPEG_LAYER_3: u8 = 0b01;
const MPEG_BITRATE_SHIFT: u8 = 4;
/// Bitrate index 0 is "free format" (no length in the header) and 15 is invalid; neither is accepted.
const MPEG_FREE_BITRATE: u8 = 0;
const MPEG_SAMPLE_RATE_SHIFT: u8 = 2;
const MPEG_PADDING_SHIFT: u8 = 1;
const MPEG_HEADER_SIZE: usize = 3;
/// Kilobits per second by bitrate index (1-14): MPEG-1 layers I, II and III, then MPEG-2/2.5 layer I and layers
/// II and III.
const MPEG_BITRATES_KBPS: [[u32; 15]; 5] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];
/// Hertz by sample rate index (0-2): MPEG-1, MPEG-2, MPEG-2.5.
const MPEG_SAMPLE_RATES: [[u32; 3]; 3] = [[44_100, 48_000, 32_000], [22_050, 24_000, 16_000], [11_025, 12_000, 8_000]];
const BITS_PER_KILOBIT: u32 = 1000;
/// Frame length: layer I is `(12 · bitrate / rate + padding) · 4` bytes; layer II, and layer III of MPEG-1, are
/// `144 · bitrate / rate + padding`; layer III of MPEG-2/2.5 is `72 · bitrate / rate + padding`.
const MPEG_LAYER_1_FACTOR: u32 = 12;
const MPEG_LAYER_1_SLOT_BYTES: u32 = 4;
const MPEG_FRAME_FACTOR: u32 = 144;
const MPEG_LOW_RATE_LAYER_3_FACTOR: u32 = 72;
/// Consecutive frames that must chain (each header right after the previous frame, with the same version, layer
/// and sample rate) unless the payload ends first.
const MPEG_FRAMES_CHECKED: usize = 4;

/// What an entry holds, by its bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentFormat {
    Dds,
    Png,
    Jpeg,
    Gif,
    /// `BM` with consistent headers, the pixels inside the payload and the file size field equal to its length.
    Bmp,
    /// A BMP whose file size field is not the payload length (headers and pixels consistent).
    BmpBadSize,
    Psd,
    Ktx,
    Ktx2,
    /// A TGA whose header and pixel data validate, ending with its pixels or a TGA 2.0 footer.
    Tga,
    /// A TGA whose header and pixel data validate, followed by bytes that are not a TGA 2.0 footer.
    TgaTrailing,
    Wav,
    /// A RIFF file of another form than WAVE.
    Riff,
    Ogg,
    Mp3,
    /// A zlib stream that inflates to its end.
    Zlib,
    /// A Gamebryo `.nif` or `.kf` file.
    Nif,
    Kfm,
    Xml,
    /// UTF-8 text other than XML (KDL, Lua, CSV and the like, which have no magic).
    Text,
    Empty,
    /// None of the above.
    Binary,
}

impl ContentFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dds => "DDS",
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::Bmp => "BMP",
            Self::BmpBadSize => "BMP, wrong file size field",
            Self::Psd => "PSD",
            Self::Ktx => "KTX",
            Self::Ktx2 => "KTX2",
            Self::Tga => "TGA",
            Self::TgaTrailing => "TGA + trailing bytes",
            Self::Wav => "WAV",
            Self::Riff => "RIFF",
            Self::Ogg => "OGG",
            Self::Mp3 => "MP3",
            Self::Zlib => "zlib",
            Self::Nif => "NIF",
            Self::Kfm => "KFM",
            Self::Xml => "XML",
            Self::Text => "text",
            Self::Empty => "empty",
            Self::Binary => "binary",
        }
    }

    pub fn is_image(self) -> bool {
        matches!(
            self,
            Self::Dds
                | Self::Png
                | Self::Jpeg
                | Self::Gif
                | Self::Bmp
                | Self::BmpBadSize
                | Self::Psd
                | Self::Ktx
                | Self::Ktx2
                | Self::Tga
                | Self::TgaTrailing
        )
    }

    /// The format an extension (lowercase, without the dot) names, when it names one with a signature or a
    /// validation here.
    pub fn of_extension(extension: &str) -> Option<Self> {
        let format = match extension {
            "dds" => Self::Dds,
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "gif" => Self::Gif,
            "bmp" => Self::Bmp,
            "psd" => Self::Psd,
            "ktx" => Self::Ktx,
            "ktx2" => Self::Ktx2,
            "tga" => Self::Tga,
            "wav" => Self::Wav,
            "ogg" => Self::Ogg,
            "mp3" => Self::Mp3,
            "nif" | "kf" => Self::Nif,
            "kfm" => Self::Kfm,
            "xml" => Self::Xml,
            _ => return None,
        };
        Some(format)
    }
}

/// The extension of an entry name, lowercase and without the dot.
pub fn name_extension(name: &str) -> Option<String> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file_name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).filter(|extension| !extension.is_empty())
}

/// The format an entry name declares by its extension.
pub fn declared_format(name: &str) -> Option<ContentFormat> {
    name_extension(name).and_then(|extension| ContentFormat::of_extension(&extension))
}

/// What [`classify_content`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentClass {
    pub format: ContentFormat,
    /// The type of a non-DDS image: `TGA type 2, 32 bit`, `BMP 32 bit`.
    pub detail: Option<String>,
    /// The TGA validation of a payload no magic identifies.
    pub tga: Option<TgaVerdict>,
}

impl ContentClass {
    fn of(format: ContentFormat) -> Self {
        Self { format, detail: None, tga: None }
    }
}

/// The format of `payload`, by its bytes alone: the magics first, then a TGA validation, then text.
pub fn classify_content(payload: &[u8]) -> ContentClass {
    if payload.is_empty() {
        return ContentClass::of(ContentFormat::Empty);
    }
    if let Some(format) = magic_format(payload) {
        let detail = match format {
            ContentFormat::Bmp | ContentFormat::BmpBadSize => bmp_layout(payload).map(|layout| layout.detail()),
            _ => None,
        };
        return ContentClass { format, detail, tga: None };
    }
    let verdict = validate_tga(payload);
    let (format, detail) = match verdict {
        TgaVerdict::Valid { image_type, depth, .. } => {
            (ContentFormat::Tga, Some(format!("TGA type {image_type}, {depth} bit")))
        }
        TgaVerdict::Trailing { image_type, depth, .. } => {
            (ContentFormat::TgaTrailing, Some(format!("TGA type {image_type}, {depth} bit")))
        }
        TgaVerdict::BadData(_) | TgaVerdict::BadHeader(_) => match mpeg_stream_start(payload) {
            Some(start) => (ContentFormat::Mp3, (start > 0).then(|| format!("MP3 after {start} zero bytes"))),
            None => (text_format(payload).unwrap_or(ContentFormat::Binary), None),
        },
    };
    ContentClass { format, detail, tga: Some(verdict) }
}

/// The formats identified by a magic (checked with the fields that make it unambiguous).
fn magic_format(payload: &[u8]) -> Option<ContentFormat> {
    let starts = |magic: &[u8]| payload.starts_with(magic);
    let format = if starts(DDS_MAGIC) {
        ContentFormat::Dds
    } else if starts(PNG_MAGIC) {
        ContentFormat::Png
    } else if starts(JPEG_MAGIC) {
        ContentFormat::Jpeg
    } else if GIF_MAGICS.iter().any(|magic| starts(magic)) {
        ContentFormat::Gif
    } else if starts(PSD_MAGIC) && read_u16_be(payload, PSD_VERSION_OFFSET).is_some_and(|v| PSD_VERSIONS.contains(&v)) {
        ContentFormat::Psd
    } else if let Some(layout) = bmp_layout(payload) {
        if layout.size_field == payload.len() { ContentFormat::Bmp } else { ContentFormat::BmpBadSize }
    } else if starts(KTX_MAGIC) {
        ContentFormat::Ktx
    } else if starts(KTX2_MAGIC) {
        ContentFormat::Ktx2
    } else if starts(RIFF_MAGIC) {
        let wave = payload.get(RIFF_FORM_OFFSET..RIFF_FORM_OFFSET + RIFF_WAVE_FORM.len()) == Some(RIFF_WAVE_FORM);
        if wave { ContentFormat::Wav } else { ContentFormat::Riff }
    } else if starts(OGG_MAGIC) {
        ContentFormat::Ogg
    } else if starts(ID3_MAGIC) {
        ContentFormat::Mp3
    } else if NIF_MAGICS.iter().any(|magic| starts(magic)) {
        ContentFormat::Nif
    } else if starts(KFM_MAGIC) {
        ContentFormat::Kfm
    } else if is_zlib_stream(payload) {
        ContentFormat::Zlib
    } else {
        return None;
    };
    Some(format)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes.get(offset..offset + size_of::<u16>()).map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + size_of::<u32>()].try_into().expect("four bytes"))
}

/// The headers of a BMP that are consistent with each other and with the payload.
struct BmpLayout {
    bit_count: u16,
    /// The file size field.
    size_field: usize,
    /// Bytes after the pixel rows of an uncompressed bitmap.
    trailing: Option<usize>,
}

impl BmpLayout {
    fn detail(&self) -> String {
        let mut detail = format!("BMP {} bit", self.bit_count);
        if let Some(trailing) = self.trailing.filter(|&bytes| bytes > 0) {
            let _ = write!(detail, ", {trailing} bytes after the pixels");
        }
        detail
    }
}

/// `BM`, zero reserved fields, a known info header size, one plane, a known bit count, the pixel offset after the
/// headers, and for an uncompressed bitmap its padded rows inside the payload. The file size field is returned,
/// not checked.
fn bmp_layout(payload: &[u8]) -> Option<BmpLayout> {
    if !payload.starts_with(BMP_MAGIC) || payload.len() < BMP_FILE_HEADER_SIZE + size_of::<u32>() {
        return None;
    }
    let info_size = read_u32(payload, BMP_INFO_SIZE_OFFSET);
    if !BMP_INFO_HEADER_SIZES.contains(&info_size) || payload.len() < BMP_FILE_HEADER_SIZE + info_size as usize {
        return None;
    }
    let (width, height, planes, bit_count, compression) = if info_size == BMP_CORE_HEADER_SIZE {
        let (width, height) = (read_u16(payload, BMP_CORE_WIDTH_OFFSET), read_u16(payload, BMP_CORE_HEIGHT_OFFSET));
        (i64::from(width), i64::from(height), read_u16(payload, BMP_CORE_PLANES_OFFSET), read_u16(payload, BMP_CORE_BIT_COUNT_OFFSET), 0)
    } else {
        let signed = |offset| i64::from(read_u32(payload, offset) as i32);
        (
            signed(BMP_WIDTH_OFFSET),
            signed(BMP_HEIGHT_OFFSET),
            read_u16(payload, BMP_PLANES_OFFSET),
            read_u16(payload, BMP_BIT_COUNT_OFFSET),
            read_u32(payload, BMP_COMPRESSION_OFFSET),
        )
    };
    let pixels = read_u32(payload, BMP_PIXELS_OFFSET) as usize;
    if read_u32(payload, BMP_RESERVED_OFFSET) != 0
        || planes != BMP_PLANES
        || !BMP_BIT_COUNTS.contains(&bit_count)
        || pixels < BMP_FILE_HEADER_SIZE + info_size as usize
        || pixels > payload.len()
        || width <= 0
        || height == 0
    {
        return None;
    }
    let trailing = if BMP_UNCOMPRESSED.contains(&compression) {
        let row = (width as usize * bit_count as usize).div_ceil(BMP_ROW_ALIGN_BITS) * BMP_ROW_ALIGN_BITS / BITS_PER_BYTE;
        let end = pixels + row * height.unsigned_abs() as usize;
        Some(payload.len().checked_sub(end)?)
    } else {
        None
    };
    Some(BmpLayout { bit_count, size_field: read_u32(payload, BMP_LENGTH_OFFSET) as usize, trailing })
}

/// A zlib header followed by a deflate stream that inflates to its end, the Adler-32 included, with nothing after.
fn is_zlib_stream(payload: &[u8]) -> bool {
    let [cmf, flg, ..] = payload else {
        return false;
    };
    if cmf & ZLIB_METHOD_MASK != ZLIB_METHOD_DEFLATE
        || cmf >> ZLIB_WINDOW_SHIFT > ZLIB_MAX_WINDOW
        || (u16::from(*cmf) << BITS_PER_BYTE | u16::from(*flg)) % ZLIB_CHECK_DIVISOR != 0
        || flg & ZLIB_PRESET_DICTIONARY != 0
        || payload.len() <= ZLIB_HEADER_SIZE
    {
        return false;
    }
    let mut inflater = flate2::Decompress::new(true);
    let mut buffer = vec![0u8; ZLIB_PROBE_BUFFER_SIZE];
    loop {
        let (read, written) = (inflater.total_in(), inflater.total_out());
        match inflater.decompress(&payload[read as usize..], &mut buffer, flate2::FlushDecompress::None) {
            Ok(flate2::Status::StreamEnd) => return inflater.total_in() as usize == payload.len(),
            Ok(_) if inflater.total_in() == read && inflater.total_out() == written => return false,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

/// Where an MPEG audio stream without an ID3 tag starts: after the leading zero bytes, [`MPEG_FRAMES_CHECKED`]
/// frames chain, or fewer that end exactly with the payload.
fn mpeg_stream_start(payload: &[u8]) -> Option<usize> {
    let start = payload.iter().position(|&byte| byte != 0)?;
    let mut offset = start;
    let mut stream = None;
    for _ in 0..MPEG_FRAMES_CHECKED {
        if offset >= payload.len() {
            return (offset == payload.len()).then_some(start);
        }
        let (length, frame_stream) = mpeg_frame(payload.get(offset..)?)?;
        if stream.is_some_and(|stream| stream != frame_stream) {
            return None;
        }
        stream = Some(frame_stream);
        offset += length;
    }
    Some(start)
}

/// The length of the MPEG audio frame `bytes` starts with, and its version, layer and sample rate index.
fn mpeg_frame(bytes: &[u8]) -> Option<(usize, (u8, u8, u8))> {
    let &[sync, version_layer, rates] = bytes.first_chunk::<MPEG_HEADER_SIZE>()?;
    if sync != MPEG_SYNC_BYTE || version_layer & MPEG_SYNC_MASK != MPEG_SYNC_MASK {
        return None;
    }
    let version = (version_layer >> MPEG_VERSION_SHIFT) & MPEG_TWO_BITS;
    let layer = (version_layer >> MPEG_LAYER_SHIFT) & MPEG_TWO_BITS;
    let rate_table = match version {
        MPEG_VERSION_1 => 0,
        MPEG_VERSION_2 => 1,
        MPEG_VERSION_2_5 => 2,
        _ => return None,
    };
    let bitrate_table = match (version, layer) {
        (MPEG_VERSION_1, MPEG_LAYER_1) => 0,
        (MPEG_VERSION_1, MPEG_LAYER_2) => 1,
        (MPEG_VERSION_1, MPEG_LAYER_3) => 2,
        (_, MPEG_LAYER_1) => 3,
        (_, MPEG_LAYER_2 | MPEG_LAYER_3) => 4,
        _ => return None,
    };
    let bitrate_index = rates >> MPEG_BITRATE_SHIFT;
    let rate_index = (rates >> MPEG_SAMPLE_RATE_SHIFT) & MPEG_TWO_BITS;
    if bitrate_index == MPEG_FREE_BITRATE {
        return None;
    }
    let bitrate = MPEG_BITRATES_KBPS[bitrate_table].get(bitrate_index as usize)? * BITS_PER_KILOBIT;
    let rate = *MPEG_SAMPLE_RATES[rate_table].get(rate_index as usize)?;
    let padding = u32::from((rates >> MPEG_PADDING_SHIFT) & 1);
    let length = match layer {
        MPEG_LAYER_1 => (MPEG_LAYER_1_FACTOR * bitrate / rate + padding) * MPEG_LAYER_1_SLOT_BYTES,
        MPEG_LAYER_3 if version != MPEG_VERSION_1 => MPEG_LOW_RATE_LAYER_3_FACTOR * bitrate / rate + padding,
        _ => MPEG_FRAME_FACTOR * bitrate / rate + padding,
    };
    Some((length as usize, (version, layer, rate_index)))
}

/// UTF-8 (an optional BOM aside) without control characters other than tab and line ends: XML when it opens with
/// `<` after white space, text otherwise.
fn text_format(payload: &[u8]) -> Option<ContentFormat> {
    let text = std::str::from_utf8(payload.strip_prefix(UTF8_BOM).unwrap_or(payload)).ok()?;
    if text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')) {
        return None;
    }
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    Some(if trimmed.starts_with(XML_OPEN) { ContentFormat::Xml } else { ContentFormat::Text })
}

// endregion: --- Content formats

// region: --- TGA validation

const TGA_HEADER_SIZE: usize = 18;
const TGA_ID_LENGTH_OFFSET: usize = 0;
const TGA_COLOR_MAP_TYPE_OFFSET: usize = 1;
const TGA_IMAGE_TYPE_OFFSET: usize = 2;
const TGA_COLOR_MAP_FIRST_OFFSET: usize = 3;
const TGA_COLOR_MAP_LENGTH_OFFSET: usize = 5;
const TGA_COLOR_MAP_DEPTH_OFFSET: usize = 7;
const TGA_WIDTH_OFFSET: usize = 12;
const TGA_HEIGHT_OFFSET: usize = 14;
const TGA_DEPTH_OFFSET: usize = 16;
const TGA_DESCRIPTOR_OFFSET: usize = 17;
const TGA_NO_COLOR_MAP: u8 = 0;
const TGA_HAS_COLOR_MAP: u8 = 1;
const TGA_COLOR_MAPPED_TYPES: [u8; 2] = [1, 9];
const TGA_TRUE_COLOR_TYPES: [u8; 2] = [2, 10];
const TGA_GREY_TYPES: [u8; 2] = [3, 11];
const TGA_RLE_TYPES: [u8; 3] = [9, 10, 11];
const TGA_INDEX_DEPTHS: [u8; 2] = [8, 16];
const TGA_TRUE_COLOR_DEPTHS: [u8; 4] = [15, 16, 24, 32];
const TGA_GREY_DEPTHS: [u8; 2] = [8, 16];
const TGA_COLOR_MAP_DEPTHS: [u8; 4] = [15, 16, 24, 32];
/// Descriptor bits 0-3: the alpha (attribute) bits per pixel.
const TGA_ALPHA_BITS_MASK: u8 = 0x0f;
/// Descriptor bits 6 and 7: interleaving in TGA 1.0, zero in TGA 2.0 and in every real TGA.
const TGA_DESCRIPTOR_RESERVED: u8 = 0xc0;
/// An RLE packet header: the high bit marks a run of one repeated pixel, the low seven bits are the count less 1.
const TGA_RLE_RUN_FLAG: u8 = 0x80;
const TGA_RLE_COUNT_MASK: u8 = 0x7f;
/// The TGA 2.0 footer: the extension and developer area offsets, then the signature.
const TGA_FOOTER_SIGNATURE: &[u8] = b"TRUEVISION-XFILE.\0";
const TGA_FOOTER_SIZE: usize = 2 * size_of::<u32>() + TGA_FOOTER_SIGNATURE.len();
const TGA_DEVELOPER_OFFSET_IN_FOOTER: usize = size_of::<u32>();
/// The extension area starts with its size: 495 in TGA 2.0; the exporter of some EC and KR textures writes 494
/// (for an area of 495 bytes).
const TGA_EXTENSION_AREA_SIZES: [u16; 2] = [495, 494];
const TGA_EXTENSION_AREA_LEN: usize = 495;
/// The developer directory: a tag count, then per tag its id, offset and size.
const TGA_DEVELOPER_TAG_SIZE: usize = size_of::<u16>() + 2 * size_of::<u32>();

/// A valid TGA 2.0 footer after the pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TgaFooter {
    /// The size field of the extension area, when there is one.
    pub extension_size: Option<u16>,
    pub developer_area: bool,
}

/// The TGA validation of a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TgaVerdict {
    /// Header and pixels validate; the payload ends with the pixels or with a valid TGA 2.0 footer.
    Valid { image_type: u8, depth: u8, footer: Option<TgaFooter> },
    /// Header and pixels validate, followed by `bytes` that are not a valid footer (after one, when `footer`);
    /// `zeros` when they are all zero.
    Trailing { image_type: u8, depth: u8, footer: Option<TgaFooter>, bytes: usize, zeros: bool },
    /// The header fields are a TGA's but the data is not: why.
    BadData(&'static str),
    /// The header fields are not a TGA's: the first one that fails.
    BadHeader(&'static str),
}

impl TgaVerdict {
    /// Whether the header fields are a TGA's (a TGA candidate).
    pub fn header_is_valid(self) -> bool {
        !matches!(self, Self::BadHeader(_))
    }

    pub fn label(self) -> String {
        match self {
            Self::Valid { footer, .. } => format!("valid{}", footer_label(footer, ", ")),
            Self::Trailing { footer, bytes, zeros, .. } => {
                format!("{bytes} trailing {}bytes{}", if zeros { "zero " } else { "" }, footer_label(footer, " after a "))
            }
            Self::BadData(reason) => format!("not TGA: {reason}"),
            Self::BadHeader(reason) => format!("not TGA (header): {reason}"),
        }
    }
}

/// Validates `payload` as a TGA, strictly: an image type of 1, 2, 3, 9, 10 or 11; colour map fields consistent
/// with it (present for colour-mapped images; when absent, a zero first index and length, and an entry size of 0
/// or of a colour map depth, as the 24 of one KR system texture); a pixel depth the type allows, with no more
/// alpha bits than it and the reserved descriptor bits clear; a non-zero size; then the pixel data, uncompressed or
/// RLE-decoded to exactly width × height pixels, followed by nothing or by a TGA 2.0 footer whose extension and
/// developer areas lie between the pixels and the footer.
pub fn validate_tga(payload: &[u8]) -> TgaVerdict {
    if payload.len() < TGA_HEADER_SIZE {
        return TgaVerdict::BadHeader("shorter than the header");
    }
    let id_length = payload[TGA_ID_LENGTH_OFFSET] as usize;
    let color_map_type = payload[TGA_COLOR_MAP_TYPE_OFFSET];
    let image_type = payload[TGA_IMAGE_TYPE_OFFSET];
    let color_map_first = read_u16(payload, TGA_COLOR_MAP_FIRST_OFFSET);
    let color_map_length = read_u16(payload, TGA_COLOR_MAP_LENGTH_OFFSET);
    let color_map_depth = payload[TGA_COLOR_MAP_DEPTH_OFFSET];
    let (width, height) = (read_u16(payload, TGA_WIDTH_OFFSET) as usize, read_u16(payload, TGA_HEIGHT_OFFSET) as usize);
    let depth = payload[TGA_DEPTH_OFFSET];
    let descriptor = payload[TGA_DESCRIPTOR_OFFSET];

    let allowed_depths: &[u8] = if TGA_COLOR_MAPPED_TYPES.contains(&image_type) {
        &TGA_INDEX_DEPTHS
    } else if TGA_TRUE_COLOR_TYPES.contains(&image_type) {
        &TGA_TRUE_COLOR_DEPTHS
    } else if TGA_GREY_TYPES.contains(&image_type) {
        &TGA_GREY_DEPTHS
    } else {
        return TgaVerdict::BadHeader("image type");
    };
    match color_map_type {
        TGA_NO_COLOR_MAP if TGA_COLOR_MAPPED_TYPES.contains(&image_type) => {
            return TgaVerdict::BadHeader("a colour-mapped type without a colour map");
        }
        TGA_NO_COLOR_MAP
            if color_map_first != 0
                || color_map_length != 0
                || (color_map_depth != 0 && !TGA_COLOR_MAP_DEPTHS.contains(&color_map_depth)) =>
        {
            return TgaVerdict::BadHeader("colour map fields without a colour map");
        }
        TGA_NO_COLOR_MAP => {}
        TGA_HAS_COLOR_MAP if color_map_length == 0 || !TGA_COLOR_MAP_DEPTHS.contains(&color_map_depth) => {
            return TgaVerdict::BadHeader("colour map length or entry size");
        }
        TGA_HAS_COLOR_MAP => {}
        _ => return TgaVerdict::BadHeader("colour map type"),
    }
    if !allowed_depths.contains(&depth) {
        return TgaVerdict::BadHeader("pixel depth");
    }
    if descriptor & TGA_ALPHA_BITS_MASK > depth || descriptor & TGA_DESCRIPTOR_RESERVED != 0 {
        return TgaVerdict::BadHeader("image descriptor");
    }
    if width == 0 || height == 0 {
        return TgaVerdict::BadHeader("zero width or height");
    }

    let pixel_bytes = (depth as usize).div_ceil(BITS_PER_BYTE);
    let color_map_bytes = color_map_length as usize * (color_map_depth as usize).div_ceil(BITS_PER_BYTE);
    let data_start = TGA_HEADER_SIZE + id_length + color_map_bytes;
    if data_start > payload.len() {
        return TgaVerdict::BadData("the image id or colour map is truncated");
    }
    let pixels = width * height;
    let pixel_end = if TGA_RLE_TYPES.contains(&image_type) {
        match rle_end(payload, data_start, pixels, pixel_bytes) {
            Ok(end) => end,
            Err(reason) => return TgaVerdict::BadData(reason),
        }
    } else {
        let end = data_start + pixels * pixel_bytes;
        if end > payload.len() {
            return TgaVerdict::BadData("the pixel data is truncated");
        }
        end
    };
    if pixel_end == payload.len() {
        return TgaVerdict::Valid { image_type, depth, footer: None };
    }
    if let Some(footer) = footer_at_end(payload, pixel_end) {
        return TgaVerdict::Valid { image_type, depth, footer: Some(footer) };
    }
    // A valid footer may still sit right after the pixels or their extension area, with more bytes after it.
    let tail = &payload[pixel_end..];
    let footer_end = tail
        .windows(TGA_FOOTER_SIGNATURE.len())
        .rposition(|window| window == TGA_FOOTER_SIGNATURE)
        .map(|position| pixel_end + position + TGA_FOOTER_SIGNATURE.len());
    let footer = footer_end.and_then(|end| footer_at_end(&payload[..end], pixel_end));
    let rest = &payload[footer.and(footer_end).unwrap_or(pixel_end)..];
    TgaVerdict::Trailing { image_type, depth, footer, bytes: rest.len(), zeros: rest.iter().all(|&byte| byte == 0) }
}

/// `footer` and its areas after `lead`, for a label; nothing without a footer.
fn footer_label(footer: Option<TgaFooter>, lead: &str) -> String {
    let Some(footer) = footer else {
        return String::new();
    };
    let mut label = format!("{lead}footer");
    if let Some(size) = footer.extension_size {
        let _ = write!(label, ", extension area {size}");
    }
    if footer.developer_area {
        label.push_str(", developer area");
    }
    label
}

/// Where the RLE packets of `pixels` pixels starting at `start` end; an error when a packet runs past the last
/// pixel or the payload ends first.
fn rle_end(payload: &[u8], start: usize, pixels: usize, pixel_bytes: usize) -> Result<usize, &'static str> {
    let (mut offset, mut decoded) = (start, 0usize);
    while decoded < pixels {
        let Some(&packet) = payload.get(offset) else {
            return Err("the RLE data is truncated");
        };
        let count = (packet & TGA_RLE_COUNT_MASK) as usize + 1;
        if decoded + count > pixels {
            return Err("an RLE packet runs past the last pixel");
        }
        offset += 1 + if packet & TGA_RLE_RUN_FLAG != 0 { pixel_bytes } else { count * pixel_bytes };
        if offset > payload.len() {
            return Err("the RLE data is truncated");
        }
        decoded += count;
    }
    Ok(offset)
}

/// The TGA 2.0 footer ending `payload`, when its extension and developer areas (when their offsets are not zero)
/// lie between the pixels and the footer: the extension area with its size field
/// ([`TGA_EXTENSION_AREA_SIZES`]), the developer directory with its tags. Without either area, the footer follows
/// the pixels directly.
fn footer_at_end(payload: &[u8], pixel_end: usize) -> Option<TgaFooter> {
    if payload.len() < pixel_end + TGA_FOOTER_SIZE || !payload.ends_with(TGA_FOOTER_SIGNATURE) {
        return None;
    }
    let footer = payload.len() - TGA_FOOTER_SIZE;
    let extension = read_u32(payload, footer) as usize;
    let developer = read_u32(payload, footer + TGA_DEVELOPER_OFFSET_IN_FOOTER) as usize;
    let none = TgaFooter { extension_size: None, developer_area: false };
    if extension == 0 && developer == 0 {
        return (footer == pixel_end).then_some(none);
    }
    let in_range = |offset: usize, len: usize| offset >= pixel_end && offset + len <= footer;
    let extension_size = match extension {
        0 => None,
        offset if in_range(offset, TGA_EXTENSION_AREA_LEN) => {
            Some(read_u16(payload, offset)).filter(|size| TGA_EXTENSION_AREA_SIZES.contains(size))
        }
        _ => return None,
    };
    if extension != 0 && extension_size.is_none() {
        return None;
    }
    let developer_ok = developer == 0
        || (in_range(developer, size_of::<u16>())
            && in_range(developer, size_of::<u16>() + read_u16(payload, developer) as usize * TGA_DEVELOPER_TAG_SIZE));
    developer_ok.then_some(TgaFooter { extension_size, developer_area: developer != 0 })
}

// endregion: --- TGA validation

// region: --- Census

/// One DDS payload of a package.
#[derive(Debug, Clone)]
pub struct DdsCensusRecord {
    pub filename_hash: u64,
    pub name: Option<String>,
    pub compression: CompressionFlag,
    pub payload_len: usize,
    pub description: DdsDescription,
}

impl DdsCensusRecord {
    /// Bytes after the last surface (negative when the payload is short); None for formats of unknown layout.
    pub fn trailing_bytes(&self) -> Option<i64> {
        let description = &self.description;
        description.data_len.map(|data| self.payload_len as i64 - description.header_len as i64 - data as i64)
    }

    fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{:016X}", self.filename_hash),
        }
    }
}

/// The content of one entry.
#[derive(Debug, Clone)]
pub struct EntryContent {
    pub filename_hash: u64,
    pub name: Option<String>,
    /// The decompressed length.
    pub payload_len: usize,
    pub class: ContentClass,
}

impl EntryContent {
    /// The format the name declares by its extension.
    pub fn declared(&self) -> Option<ContentFormat> {
        self.name.as_deref().and_then(declared_format)
    }

    /// The name's extension says one format and the content is another, or the content is an image and the
    /// extension names no image of its format.
    pub fn name_disagrees(&self) -> bool {
        let actual = self.class.format;
        match (self.name.as_deref(), self.declared()) {
            (None, _) => false,
            (Some(_), Some(declared)) => declared != actual,
            (Some(_), None) => actual.is_image(),
        }
    }

    /// What the name says, for the report: the format, `.<extension>`, or `no extension`.
    pub fn declared_label(&self) -> String {
        match (self.declared(), self.name.as_deref().and_then(name_extension)) {
            (Some(format), _) => format.label().to_owned(),
            (None, Some(extension)) => format!(".{extension}"),
            (None, None) => "no extension".to_owned(),
        }
    }

    fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{:016X}", self.filename_hash),
        }
    }

    /// `name (hash, bytes)` for an example list.
    fn example(&self) -> String {
        match &self.name {
            Some(name) => format!("`{name}` {:016X} {} B", self.filename_hash, self.payload_len),
            None => format!("{:016X} {} B", self.filename_hash, self.payload_len),
        }
    }
}

/// The entries of one package.
#[derive(Debug, Clone, Default)]
pub struct PackageCensus {
    /// The package path relative to the scanned directory.
    pub path: PathBuf,
    pub entries: usize,
    pub dds: Vec<DdsCensusRecord>,
    /// Every decompressed entry, by content.
    pub contents: Vec<EntryContent>,
    /// Payloads holding a DDS header after their start: entry label and offset.
    pub embedded_dds: Vec<(String, usize)>,
    /// Entries that failed to decompress, or DDS payloads whose header does not parse: entry label and error.
    pub unreadable: Vec<(String, String)>,
}

impl PackageCensus {
    /// The install folder: the first component of the package path when it has several.
    pub fn install(&self) -> String {
        let mut components = self.path.components();
        match (components.next(), components.next()) {
            (Some(first), Some(_)) => first.as_os_str().to_string_lossy().into_owned(),
            _ => String::new(),
        }
    }

    /// The entries whose name disagrees with their content ([`EntryContent::name_disagrees`]), by what the name
    /// says and what the content is.
    pub fn name_mismatches(&self) -> BTreeMap<(String, ContentFormat), Vec<&EntryContent>> {
        let mut mismatches: BTreeMap<(String, ContentFormat), Vec<&EntryContent>> = BTreeMap::new();
        for entry in self.contents.iter().filter(|entry| entry.name_disagrees()) {
            mismatches.entry((entry.declared_label(), entry.class.format)).or_default().push(entry);
        }
        mismatches
    }

    /// The count of entries per content format.
    pub fn format_counts(&self) -> BTreeMap<ContentFormat, usize> {
        let mut counts = BTreeMap::new();
        for entry in &self.contents {
            *counts.entry(entry.class.format).or_default() += 1;
        }
        counts
    }
}

/// The entries of every package under a directory.
#[derive(Debug, Clone, Default)]
pub struct ImageCensus {
    pub packages: Vec<PackageCensus>,
}

/// Scans every `*.uop` under each of `roots` (recursively, in path order). A package is labelled with its path
/// relative to its root, after the root's folder name when there are several roots. `name_of` names an entry hash
/// when known.
pub fn census_directories(
    roots: &[PathBuf],
    name_of: &(dyn Fn(u64) -> Option<String> + Sync),
) -> eyre::Result<ImageCensus> {
    let mut packages = Vec::new();
    for root in roots {
        let paths = find_packages(root)?;
        let prefix = if roots.len() > 1 { root.file_name().map(PathBuf::from) } else { None };
        for path in paths {
            let mut package = census_package(&path, name_of).wrap_err_with(|| format!("scan {}", path.display()))?;
            let relative = path.strip_prefix(root).unwrap_or(&path);
            package.path = prefix.as_ref().map_or_else(|| relative.to_path_buf(), |prefix| prefix.join(relative));
            packages.push(package);
        }
    }
    Ok(ImageCensus { packages })
}

/// Every `*.uop` under `root`, recursively, in path order.
pub fn find_packages(root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_packages(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_packages(dir: &Path, out: &mut Vec<PathBuf>) -> eyre::Result<()> {
    for entry in std::fs::read_dir(dir).wrap_err_with(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_packages(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION)) {
            out.push(path);
        }
    }
    Ok(())
}

/// What one entry holds.
enum EntryScan {
    Read { content: EntryContent, dds: Option<Result<DdsCensusRecord, String>>, embedded_dds: Option<usize> },
    Unreadable(String, String),
    None,
}

/// Scans the entries of one package.
pub fn census_package(path: &Path, name_of: &(dyn Fn(u64) -> Option<String> + Sync)) -> eyre::Result<PackageCensus> {
    let package = UopPackage::load_with_mode(path, LoadMode::Lazy)?;
    let files: Vec<_> = package.iter_files().filter(|file| file.has_size()).collect();
    let scans: Vec<EntryScan> = files
        .par_iter()
        .map_init(
            || package.open_payload_reader(),
            |reader, file| {
                let hash = file.filename_hash();
                let name = name_of(hash);
                let payload = match reader {
                    Ok(reader) => package.unpack_file_by_hash_from_reader(hash, reader),
                    Err(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
                };
                let payload = match payload {
                    Ok(Some(payload)) => payload,
                    Ok(None) => return EntryScan::None,
                    Err(error) => {
                        return EntryScan::Unreadable(name.unwrap_or_else(|| format!("{hash:016X}")), error.to_string());
                    }
                };
                scan_entry(&payload, hash, name, file.compression())
            },
        )
        .collect();
    let mut census = PackageCensus { path: path.to_path_buf(), entries: files.len(), ..Default::default() };
    for scan in scans {
        match scan {
            EntryScan::Read { content, dds, embedded_dds } => {
                match dds {
                    Some(Ok(record)) => census.dds.push(record),
                    Some(Err(error)) => census.unreadable.push((content.label(), error)),
                    None => {}
                }
                if let Some(offset) = embedded_dds {
                    census.embedded_dds.push((content.label(), offset));
                }
                census.contents.push(content);
            }
            EntryScan::Unreadable(label, error) => census.unreadable.push((label, error)),
            EntryScan::None => {}
        }
    }
    census.dds.sort_by(|a, b| a.label().cmp(&b.label()));
    census.contents.sort_by(|a, b| (&a.name, a.filename_hash).cmp(&(&b.name, b.filename_hash)));
    Ok(census)
}

fn scan_entry(payload: &[u8], hash: u64, name: Option<String>, compression: CompressionFlag) -> EntryScan {
    let class = classify_content(payload);
    let dds = (class.format == ContentFormat::Dds).then(|| {
        DdsDescription::parse(payload)
            .map(|description| DdsCensusRecord {
                filename_hash: hash,
                name: name.clone(),
                compression,
                payload_len: payload.len(),
                description,
            })
            .map_err(|error| format!("{error}"))
    });
    let embedded_dds = if class.format == ContentFormat::Dds {
        None
    } else {
        payload.windows(EMBEDDED_DDS_PATTERN.len()).position(|window| window == EMBEDDED_DDS_PATTERN)
    };
    let content = EntryContent { filename_hash: hash, name, payload_len: payload.len(), class };
    EntryScan::Read { content, dds, embedded_dds }
}

// endregion: --- Census

// region: --- Report

/// How the mip count field relates to the image size: 0 (one level), 1, the full chain, the chain down to the
/// smaller side at 1, or another count.
const MIP_RULES: [&str; 5] = ["0", "1", "full", "to min side", "other"];

fn mip_rule(record: &DdsCensusRecord) -> &'static str {
    let description = &record.description;
    let (width, height) = (description.width(), description.height());
    match description.header.mipmap_count {
        0 => "0",
        1 => "1",
        count if count == full_mip_count(width, height) => "full",
        count if count == full_mip_count(width.min(height), width.min(height)) => "to min side",
        _ => "other",
    }
}

/// How `pitchOrLinearSize` relates to mip 0.
fn pitch_rule(record: &DdsCensusRecord) -> String {
    let description = &record.description;
    let value = description.header.pitch_or_linear_size;
    let (width, height) = (description.width(), description.height());
    let surface = description.format.surface_bytes(width, height);
    let row = description.format.surface_bytes(width, 1);
    if value == 0 {
        "0".to_owned()
    } else if description.format.is_block_compressed() && surface == Some(u64::from(value)) {
        "mip 0 bytes".to_owned()
    } else if !description.format.is_block_compressed() && row == Some(u64::from(value)) {
        "row bytes".to_owned()
    } else if surface == Some(u64::from(value)) {
        "mip 0 bytes".to_owned()
    } else {
        "other".to_owned()
    }
}

/// The reserved dwords: zero, an NVTT tag with its version, or the non-zero dwords.
fn reserved_rule(record: &DdsCensusRecord) -> String {
    let reserved = &record.description.header.reserved1;
    if reserved.iter().all(|&dword| dword == 0) {
        return "0".to_owned();
    }
    if reserved[NVTT_TAG_DWORD] == NVTT_TAG
        && reserved.iter().enumerate().all(|(index, &dword)| index >= NVTT_TAG_DWORD || dword == 0)
    {
        return format!("NVTT {:#x}", reserved[NVTT_VERSION_DWORD]);
    }
    reserved
        .iter()
        .enumerate()
        .filter(|(_, dword)| **dword != 0)
        .map(|(index, dword)| format!("[{index}]={dword:#x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Header fields beyond format, flags, caps, pitch and reserved dwords that are not zero.
fn extra_fields(record: &DdsCensusRecord) -> String {
    let header = &record.description.header;
    let pixel_format = &header.pixel_format;
    let mut extra = Vec::new();
    if header.depth != 0 && !record.description.is_volume() {
        extra.push(format!("depth {}", header.depth));
    }
    if pixel_format.flags.contains(dds::header::PixelFormatFlags::FOURCC)
        && (pixel_format.rgb_bit_count != 0
            || [pixel_format.r_bit_mask, pixel_format.g_bit_mask, pixel_format.b_bit_mask, pixel_format.a_bit_mask]
                .iter()
                .any(|&mask| mask != 0))
    {
        extra.push(format!(
            "FourCC with bit count {} masks {:x}/{:x}/{:x}/{:x}",
            pixel_format.rgb_bit_count,
            pixel_format.r_bit_mask,
            pixel_format.g_bit_mask,
            pixel_format.b_bit_mask,
            pixel_format.a_bit_mask
        ));
    }
    if pixel_format.size != crate::enhanced::dds_writer::DDS_PIXEL_FORMAT_STRUCT_SIZE {
        extra.push(format!("pixel format size {}", pixel_format.size));
    }
    if header.size != crate::enhanced::dds_writer::DDS_HEADER_STRUCT_SIZE {
        extra.push(format!("header size {}", header.size));
    }
    for (field, value) in [("caps3", header.caps3), ("caps4", header.caps4), ("reserved2", header.reserved2)] {
        if value != 0 {
            extra.push(format!("{field} {value:#x}"));
        }
    }
    if let Some(dx10) = &header.dx10 {
        extra.push(format!(
            "DX10 dim {} misc {:#x} array {} misc2 {:#x}",
            dx10.resource_dimension,
            dx10.misc_flag.bits(),
            dx10.array_size,
            dx10.misc_flags2
        ));
    }
    extra.join(", ")
}

fn is_power_of_two_image(record: &DdsCensusRecord) -> bool {
    record.description.width().is_power_of_two() && record.description.height().is_power_of_two()
}

/// One header convention: everything in a header but the size-dependent fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ConventionKey {
    format: String,
    flags: u32,
    caps: u32,
    caps2: u32,
    pixel_format_flags: u32,
    pitch: String,
    mips: &'static str,
    reserved: String,
    extra: String,
}

impl ConventionKey {
    fn of(record: &DdsCensusRecord) -> Self {
        let header = &record.description.header;
        Self {
            format: record.description.format.label(),
            flags: header.flags.bits(),
            caps: header.caps.bits(),
            caps2: header.caps2.bits(),
            pixel_format_flags: header.pixel_format.flags.bits(),
            pitch: pitch_rule(record),
            mips: mip_rule(record),
            reserved: reserved_rule(record),
            extra: extra_fields(record),
        }
    }
}

#[derive(Default)]
struct FormatStats {
    count: usize,
    mips: BTreeMap<&'static str, usize>,
    cube: usize,
    volume: usize,
    non_power_of_two: usize,
    min_size: Option<(u32, u32)>,
    max_size: Option<(u32, u32)>,
    trailing: usize,
    compression: BTreeMap<String, usize>,
}

#[derive(Default)]
struct ConventionStats {
    count: usize,
    packages: BTreeSet<String>,
    examples: Vec<String>,
}

impl ImageCensus {
    pub fn dds_count(&self) -> usize {
        self.packages.iter().map(|package| package.dds.len()).sum()
    }

    /// Every DDS record with its package path.
    pub fn records(&self) -> impl Iterator<Item = (&Path, &DdsCensusRecord)> {
        self.packages.iter().flat_map(|package| package.dds.iter().map(move |record| (package.path.as_path(), record)))
    }

    /// One tab-separated line per DDS: package, entry hash and name, compression, payload length, format, width,
    /// height, then the 31 header dwords after the magic (and the 5 DX10 dwords) in hex.
    pub fn records_tsv(&self) -> String {
        let mut out = String::from("package\thash\tname\tcompression\tpayload_len\tformat\twidth\theight\theader_dwords\n");
        for (path, record) in self.records() {
            let description = &record.description;
            let mut header = Vec::new();
            description.header.write(&mut header).expect("writing to memory");
            let dwords = header
                .chunks_exact(size_of::<u32>())
                .map(|dword| format!("{:08x}", u32::from_le_bytes(dword.try_into().expect("four bytes"))))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(
                out,
                "{}\t{:016X}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{dwords}",
                path.display(),
                record.filename_hash,
                record.name.as_deref().unwrap_or(""),
                record.compression,
                record.payload_len,
                description.format.label(),
                description.width(),
                description.height()
            );
        }
        out
    }

    /// The entries whose name disagrees with their content, in every package.
    pub fn mismatch_count(&self) -> usize {
        self.packages.iter().flat_map(|package| &package.contents).filter(|entry| entry.name_disagrees()).count()
    }

    /// The count of entries per content format, in every package.
    pub fn format_counts(&self) -> BTreeMap<ContentFormat, usize> {
        let mut counts = BTreeMap::new();
        for package in &self.packages {
            for (format, count) in package.format_counts() {
                *counts.entry(format).or_default() += count;
            }
        }
        counts
    }

    /// One tab-separated line per entry that is not plain binary: package, entry hash and name, payload length,
    /// what the name declares, the content format, its type detail and the TGA verdict of a TGA candidate.
    pub fn contents_tsv(&self) -> String {
        let mut out = String::from("package\thash\tname\tpayload_len\tdeclared\tcontent\tdetail\ttga\n");
        for package in &self.packages {
            for entry in package.contents.iter().filter(|entry| reported_content(entry)) {
                let tga = entry.class.tga.filter(|verdict| verdict.header_is_valid()).map(TgaVerdict::label);
                let _ = writeln!(
                    out,
                    "{}\t{:016X}\t{}\t{}\t{}\t{}\t{}\t{}",
                    package.path.display(),
                    entry.filename_hash,
                    entry.name.as_deref().unwrap_or(""),
                    entry.payload_len,
                    if entry.name.is_some() { entry.declared_label() } else { String::new() },
                    entry.class.format.label(),
                    entry.class.detail.as_deref().unwrap_or(""),
                    tga.unwrap_or_default()
                );
            }
        }
        out
    }

    /// The census report: totals, content per install and package, name/content disagreements, the TGA
    /// validation, then the DDS formats per package, header conventions and quirks.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let entries: usize = self.packages.iter().map(|package| package.entries).sum();
        let named = self.packages.iter().flat_map(|package| &package.contents).filter(|entry| entry.name.is_some()).count();
        let _ = writeln!(out, "# Image census\n");
        let _ = writeln!(
            out,
            "{} packages, {} entries ({} named by the dictionaries), classified by their decompressed content alone \
             (a magic, or a strict TGA validation), never by name. {} DDS payloads; {} named entries disagree with \
             their extension.\n",
            self.packages.len(),
            entries,
            named,
            self.dds_count(),
            self.mismatch_count()
        );
        self.write_contents(&mut out);
        self.write_mismatches(&mut out);
        self.write_tga_validation(&mut out);

        let _ = writeln!(out, "\n## DDS formats per package\n");
        let _ = writeln!(
            out,
            "| Package | Entries | Format | Count | Mip count field (0 / 1 / full / to min side / other) | Cube | Volume | Non-pow2 | Smallest | Largest | Trailing bytes | Compression |"
        );
        let _ = writeln!(out, "|---|---:|---|---:|---|---:|---:|---:|---|---|---:|---|");
        for package in &self.packages {
            if package.dds.is_empty() {
                continue;
            }
            let mut formats: BTreeMap<String, FormatStats> = BTreeMap::new();
            for record in &package.dds {
                let stats = formats.entry(record.description.format.label()).or_default();
                let description = &record.description;
                let size = (description.width(), description.height());
                let area = |size: (u32, u32)| u64::from(size.0) * u64::from(size.1);
                stats.count += 1;
                *stats.mips.entry(mip_rule(record)).or_default() += 1;
                stats.cube += usize::from(description.is_cube_map());
                stats.volume += usize::from(description.is_volume());
                stats.non_power_of_two += usize::from(!is_power_of_two_image(record));
                if stats.min_size.is_none_or(|min| area(size) < area(min)) {
                    stats.min_size = Some(size);
                }
                if stats.max_size.is_none_or(|max| area(size) > area(max)) {
                    stats.max_size = Some(size);
                }
                stats.trailing += usize::from(record.trailing_bytes().is_some_and(|bytes| bytes != 0));
                *stats.compression.entry(format!("{:?}", record.compression)).or_default() += 1;
            }
            for (format, stats) in formats {
                let mips = MIP_RULES
                    .map(|rule| stats.mips.get(rule).copied().unwrap_or(0).to_string())
                    .join(" / ");
                let size = |size: Option<(u32, u32)>| size.map_or(String::new(), |(w, h)| format!("{w}x{h}"));
                let compression =
                    stats.compression.iter().map(|(name, count)| format!("{name} {count}")).collect::<Vec<_>>().join(", ");
                let _ = writeln!(
                    out,
                    "| {} | {} | {format} | {} | {mips} | {} | {} | {} | {} | {} | {} | {compression} |",
                    package.path.display(),
                    package.entries,
                    stats.count,
                    stats.cube,
                    stats.volume,
                    stats.non_power_of_two,
                    size(stats.min_size),
                    size(stats.max_size),
                    stats.trailing
                );
            }
        }

        let _ = writeln!(out, "\n## Header conventions\n");
        let _ = writeln!(
            out,
            "Every header field but width, height, the mip count and `pitchOrLinearSize` (given as its relation to \
             mip 0). Flags, caps and caps2 in hex.\n"
        );
        let _ = writeln!(out, "| Format | Flags | Caps | Caps2 | PF flags | Pitch | Mips | Reserved1 | Other fields | Count | Packages | Examples |");
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---:|---|---|");
        let mut conventions: BTreeMap<ConventionKey, ConventionStats> = BTreeMap::new();
        for (path, record) in self.records() {
            let stats = conventions.entry(ConventionKey::of(record)).or_default();
            stats.count += 1;
            stats.packages.insert(package_label(path));
            if stats.examples.len() < EXAMPLES_KEPT {
                let description = &record.description;
                stats.examples.push(format!(
                    "{} {}x{} m{}",
                    record.label(),
                    description.width(),
                    description.height(),
                    description.mip_count
                ));
            }
        }
        for (key, stats) in &conventions {
            let _ = writeln!(
                out,
                "| {} | {:#x} | {:#x} | {:#x} | {:#x} | {} | {} | {} | {} | {} | {} | {} |",
                key.format,
                key.flags,
                key.caps,
                key.caps2,
                key.pixel_format_flags,
                key.pitch,
                key.mips,
                key.reserved,
                key.extra,
                stats.count,
                stats.packages.iter().cloned().collect::<Vec<_>>().join(", "),
                stats.examples.join("; ")
            );
        }

        let _ = writeln!(out, "\n## Payload length vs header\n");
        let mut mismatches: BTreeMap<(String, i64), ConventionStats> = BTreeMap::new();
        let mut unknown_layout = 0usize;
        for (path, record) in self.records() {
            match record.trailing_bytes() {
                None => unknown_layout += 1,
                Some(0) => {}
                Some(bytes) => {
                    let stats = mismatches.entry((record.description.format.label(), bytes)).or_default();
                    stats.count += 1;
                    stats.packages.insert(package_label(path));
                    if stats.examples.len() < EXAMPLES_KEPT {
                        stats.examples.push(record.label());
                    }
                }
            }
        }
        if mismatches.is_empty() {
            let _ = writeln!(out, "Every payload ends with its last surface.");
        } else {
            let _ = writeln!(out, "| Format | Bytes after the last surface | Count | Packages | Examples |");
            let _ = writeln!(out, "|---|---:|---:|---|---|");
            for ((format, bytes), stats) in &mismatches {
                let _ = writeln!(
                    out,
                    "| {format} | {bytes} | {} | {} | {} |",
                    stats.count,
                    stats.packages.iter().cloned().collect::<Vec<_>>().join(", "),
                    stats.examples.join("; ")
                );
            }
        }
        if unknown_layout > 0 {
            let _ = writeln!(out, "\n{unknown_layout} payloads have a format of unknown layout.");
        }

        let _ = writeln!(out, "\n## DDS inside other payloads\n");
        let embedded: Vec<_> = self.packages.iter().filter(|package| !package.embedded_dds.is_empty()).collect();
        if embedded.is_empty() {
            let _ = writeln!(out, "None.");
        } else {
            let _ = writeln!(out, "| Package | Count | Examples (entry @ offset) |");
            let _ = writeln!(out, "|---|---:|---|");
            for package in embedded {
                let examples = package
                    .embedded_dds
                    .iter()
                    .take(EXAMPLES_KEPT)
                    .map(|(label, offset)| format!("{label} @ {offset}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                let _ = writeln!(out, "| {} | {} | {examples} |", package.path.display(), package.embedded_dds.len());
            }
        }

        let unreadable: Vec<_> = self.packages.iter().filter(|package| !package.unreadable.is_empty()).collect();
        if !unreadable.is_empty() {
            let _ = writeln!(out, "\n## Unreadable entries\n");
            for package in unreadable {
                for (label, error) in package.unreadable.iter().take(EXAMPLES_KEPT) {
                    let _ = writeln!(out, "- {} {label}: {error}", package.path.display());
                }
                let _ = writeln!(out, "- {}: {} in all", package.path.display(), package.unreadable.len());
            }
        }
        out
    }
}

/// Entries the content listings show: everything but plain binary, and binary entries that a name calls an image
/// or whose header fields are a TGA's.
fn reported_content(entry: &EntryContent) -> bool {
    entry.class.format != ContentFormat::Binary
        || entry.declared().is_some_and(ContentFormat::is_image)
        || entry.class.tga.is_some_and(TgaVerdict::header_is_valid)
}

/// How the names of the entries of one content format relate to it.
#[derive(Default)]
struct NameStats {
    count: usize,
    named_as_such: usize,
    named_otherwise: usize,
    unnamed: usize,
}

impl NameStats {
    fn add(&mut self, entry: &EntryContent) {
        self.count += 1;
        match &entry.name {
            None => self.unnamed += 1,
            Some(_) if entry.declared() == Some(entry.class.format) => self.named_as_such += 1,
            Some(_) => self.named_otherwise += 1,
        }
    }
}

/// The first [`EXAMPLES_KEPT`] entries, named ones first.
fn examples<'a>(entries: impl IntoIterator<Item = &'a EntryContent>) -> String {
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_by_key(|entry| entry.name.is_none());
    entries.iter().take(EXAMPLES_KEPT).map(|entry| entry.example()).collect::<Vec<_>>().join("; ")
}

impl ImageCensus {
    /// Content formats per install, then the images per package with their type.
    fn write_contents(&self, out: &mut String) {
        let _ = writeln!(out, "## Content per install\n");
        let _ = writeln!(out, "Named as such: the name's extension is the content's format.\n");
        let _ = writeln!(out, "| Install | Content | Count | Named as such | Named otherwise | Unnamed |");
        let _ = writeln!(out, "|---|---|---:|---:|---:|---:|");
        let mut installs: BTreeMap<(String, ContentFormat), NameStats> = BTreeMap::new();
        for package in &self.packages {
            for entry in &package.contents {
                installs.entry((package.install(), entry.class.format)).or_default().add(entry);
            }
        }
        for ((install, format), stats) in &installs {
            let _ = writeln!(
                out,
                "| {install} | {} | {} | {} | {} | {} |",
                format.label(),
                stats.count,
                stats.named_as_such,
                stats.named_otherwise,
                stats.unnamed
            );
        }

        let _ = writeln!(out, "\n## Images per package\n");
        let _ = writeln!(out, "| Package | Content | Type | Count | Named as such | Named otherwise | Unnamed |");
        let _ = writeln!(out, "|---|---|---|---:|---:|---:|---:|");
        for package in &self.packages {
            let mut images: BTreeMap<(ContentFormat, String), NameStats> = BTreeMap::new();
            for entry in package.contents.iter().filter(|entry| entry.class.format.is_image()) {
                let detail = entry.class.detail.clone().unwrap_or_default();
                images.entry((entry.class.format, detail)).or_default().add(entry);
            }
            for ((format, detail), stats) in &images {
                let _ = writeln!(
                    out,
                    "| {} | {} | {detail} | {} | {} | {} | {} |",
                    package.path.display(),
                    format.label(),
                    stats.count,
                    stats.named_as_such,
                    stats.named_otherwise,
                    stats.unnamed
                );
            }
        }
    }

    /// Named entries whose extension disagrees with the content, then unnamed entries per content format.
    fn write_mismatches(&self, out: &mut String) {
        let _ = writeln!(out, "\n## Name vs content\n");
        let _ = writeln!(
            out,
            "Named entries whose extension names another format than their content, and images whose extension \
             names no image of their format. Examples: name, hash, decompressed size.\n"
        );
        let mismatches: Vec<_> = self.packages.iter().map(|package| (package, package.name_mismatches())).collect();
        if mismatches.iter().all(|(_, mismatches)| mismatches.is_empty()) {
            let _ = writeln!(out, "None.");
        } else {
            let _ = writeln!(out, "| Package | Name says | Content | Count | Examples |");
            let _ = writeln!(out, "|---|---|---|---:|---|");
            for (package, mismatches) in &mismatches {
                for ((declared, actual), entries) in mismatches {
                    let _ = writeln!(
                        out,
                        "| {} | {declared} | {} | {} | {} |",
                        package.path.display(),
                        actual.label(),
                        entries.len(),
                        examples(entries.iter().copied())
                    );
                }
            }
        }

        let _ = writeln!(out, "\n## Unnamed entries by content\n");
        let _ = writeln!(out, "Every content format but plain binary (counted per install above).\n");
        let _ = writeln!(out, "| Package | Content | Count | Examples |");
        let _ = writeln!(out, "|---|---|---:|---|");
        for package in &self.packages {
            let mut unnamed: BTreeMap<ContentFormat, Vec<&EntryContent>> = BTreeMap::new();
            for entry in package.contents.iter().filter(|entry| entry.name.is_none()) {
                if entry.class.format != ContentFormat::Binary {
                    unnamed.entry(entry.class.format).or_default().push(entry);
                }
            }
            for (format, entries) in &unnamed {
                let _ = writeln!(
                    out,
                    "| {} | {} | {} | {} |",
                    package.path.display(),
                    format.label(),
                    entries.len(),
                    examples(entries.iter().copied())
                );
            }
        }
    }

    /// The TGA verdicts of the entries named `.tga` and of every payload whose header fields are a TGA's.
    fn write_tga_validation(&self, out: &mut String) {
        let _ = writeln!(out, "\n## TGA validation\n");
        let _ = writeln!(
            out,
            "Entries named `.tga` and payloads whose 18 header bytes are a TGA's (type, colour map, depth, \
             descriptor, size), by the verdict on their data. A payload a magic identifies has no verdict.\n"
        );
        let _ = writeln!(out, "| Package | Verdict | Named .tga | Named otherwise | Unnamed | Examples |");
        let _ = writeln!(out, "|---|---|---:|---:|---:|---|");
        for package in &self.packages {
            let mut verdicts: BTreeMap<String, (usize, usize, usize, Vec<&EntryContent>)> = BTreeMap::new();
            for entry in &package.contents {
                let named_tga = entry.declared() == Some(ContentFormat::Tga);
                let verdict = match entry.class.tga {
                    Some(verdict) if named_tga || verdict.header_is_valid() => tga_verdict_group(verdict),
                    None if named_tga => format!("magic: {}", entry.class.format.label()),
                    _ => continue,
                };
                let stats = verdicts.entry(verdict).or_default();
                match (&entry.name, named_tga) {
                    (_, true) => stats.0 += 1,
                    (Some(_), false) => stats.1 += 1,
                    (None, false) => stats.2 += 1,
                }
                stats.3.push(entry);
            }
            for (verdict, (named_tga, named_otherwise, unnamed, entries)) in &verdicts {
                let _ = writeln!(
                    out,
                    "| {} | {verdict} | {named_tga} | {named_otherwise} | {unnamed} | {} |",
                    package.path.display(),
                    examples(entries.iter().copied())
                );
            }
        }
    }
}

/// The report group of a TGA verdict: trailing byte counts are grouped by their count.
fn tga_verdict_group(verdict: TgaVerdict) -> String {
    match verdict {
        TgaVerdict::Valid { image_type, depth, footer } => {
            format!("valid type {image_type} {depth} bit{}", footer_label(footer, ", "))
        }
        _ => verdict.label(),
    }
}

/// `folder/file.uop` of a package path relative to the scanned directory.
fn package_label(path: &Path) -> String {
    path.display().to_string()
}

// endregion: --- Report

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;
    use crate::enhanced::dds_writer::{DdsHeaderStyle, DdsTextureFormat as Format};
    use crate::uop_container::hash::hash_file_name_single;

    const SIDE: u16 = 2;
    const PIXELS: usize = SIDE as usize * SIDE as usize;
    const TRUE_COLOR: u8 = 2;
    const GREY: u8 = 3;
    const COLOR_MAPPED: u8 = 1;
    const TRUE_COLOR_RLE: u8 = 10;
    const DEPTH_32: u8 = 32;
    const DEPTH_24: u8 = 24;
    const DEPTH_8: u8 = 8;
    const ALPHA_8: u8 = 8;
    const BGRA: [u8; 4] = [1, 2, 3, 4];
    const TRAILING: usize = 3;
    const PADDING: usize = 32;
    /// An RLE packet: a run of `count` pixels, or `count` raw pixels.
    const fn run(count: u8) -> u8 {
        TGA_RLE_RUN_FLAG | (count - 1)
    }
    const fn raw(count: u8) -> u8 {
        count - 1
    }
    /// MPEG-1 layer III, 128 kbit/s, 44.1 kHz, no padding: 417-byte frames.
    const MPEG_HEADER: [u8; 4] = [0xff, 0xfb, 0x90, 0x00];
    const MPEG_FRAME_LEN: usize = 417;
    const MPEG_FREE_HEADER: [u8; 4] = [0xff, 0xfb, 0x00, 0x00];
    const LEADING_ZEROS: usize = 720;
    const BMP_24_ROW: usize = 8;
    const BITMAPINFOHEADER_SIZE: u32 = 40;
    const BMP_HEADERS: usize = BMP_FILE_HEADER_SIZE + BITMAPINFOHEADER_SIZE as usize;
    const RGB_BYTES: usize = 3;
    const MAP_ENTRIES: u8 = 2;
    const GIMP_PADDING: usize = 2;

    fn tga_header(image_type: u8, depth: u8, descriptor: u8) -> Vec<u8> {
        let mut header = vec![0u8; TGA_HEADER_SIZE];
        header[TGA_IMAGE_TYPE_OFFSET] = image_type;
        header[TGA_WIDTH_OFFSET..TGA_WIDTH_OFFSET + 2].copy_from_slice(&SIDE.to_le_bytes());
        header[TGA_HEIGHT_OFFSET..TGA_HEIGHT_OFFSET + 2].copy_from_slice(&SIDE.to_le_bytes());
        header[TGA_DEPTH_OFFSET] = depth;
        header[TGA_DESCRIPTOR_OFFSET] = descriptor;
        header
    }

    /// A 2x2 32-bit uncompressed TGA.
    fn tga_type_2() -> Vec<u8> {
        let mut tga = tga_header(TRUE_COLOR, DEPTH_32, ALPHA_8);
        (0..PIXELS).for_each(|_| tga.extend(BGRA));
        tga
    }

    /// A TGA 2.0 footer with these area offsets.
    fn footer(extension: u32, developer: u32) -> Vec<u8> {
        let mut footer = extension.to_le_bytes().to_vec();
        footer.extend(developer.to_le_bytes());
        footer.extend(TGA_FOOTER_SIGNATURE);
        footer
    }

    const PLAIN_FOOTER: TgaFooter = TgaFooter { extension_size: None, developer_area: false };

    #[test]
    fn uncompressed_tga_ends_with_its_pixels_or_a_footer() {
        let tga = tga_type_2();
        let valid = |footer| TgaVerdict::Valid { image_type: TRUE_COLOR, depth: DEPTH_32, footer };
        assert_eq!(validate_tga(&tga), valid(None));
        assert_eq!(classify_content(&tga).format, ContentFormat::Tga);
        assert_eq!(classify_content(&tga).detail.as_deref(), Some("TGA type 2, 32 bit"));

        let mut with_footer = tga.clone();
        with_footer.extend(footer(0, 0));
        assert_eq!(validate_tga(&with_footer), valid(Some(PLAIN_FOOTER)));

        for size in TGA_EXTENSION_AREA_SIZES {
            let mut with_extension = tga.clone();
            let offset = with_extension.len() as u32;
            let mut area = vec![0u8; TGA_EXTENSION_AREA_LEN];
            area[..2].copy_from_slice(&size.to_le_bytes());
            with_extension.extend(area);
            with_extension.extend(footer(offset, 0));
            let footer = TgaFooter { extension_size: Some(size), developer_area: false };
            assert_eq!(validate_tga(&with_extension), valid(Some(footer)));
        }

        let mut bad_extension = tga.clone();
        let offset = bad_extension.len() as u32;
        bad_extension.extend(vec![0u8; TGA_EXTENSION_AREA_LEN]);
        bad_extension.extend(footer(offset, 0));
        assert!(matches!(validate_tga(&bad_extension), TgaVerdict::Trailing { footer: None, .. }));

        let mut trailing = tga.clone();
        trailing.extend([1; TRAILING]);
        let verdict = validate_tga(&trailing);
        assert_eq!(
            verdict,
            TgaVerdict::Trailing { image_type: TRUE_COLOR, depth: DEPTH_32, footer: None, bytes: TRAILING, zeros: false }
        );
        assert_eq!(verdict.label(), "3 trailing bytes");
        assert_eq!(classify_content(&trailing).format, ContentFormat::TgaTrailing);

        let mut padded = with_footer.clone();
        padded.extend([0; PADDING]);
        let verdict = validate_tga(&padded);
        assert_eq!(
            verdict,
            TgaVerdict::Trailing {
                image_type: TRUE_COLOR,
                depth: DEPTH_32,
                footer: Some(PLAIN_FOOTER),
                bytes: PADDING,
                zeros: true
            }
        );
        assert_eq!(verdict.label(), "32 trailing zero bytes after a footer");

        let truncated = &tga[..tga.len() - 1];
        assert_eq!(validate_tga(truncated), TgaVerdict::BadData("the pixel data is truncated"));
        assert_eq!(classify_content(truncated).format, ContentFormat::Binary);
    }

    #[test]
    fn rle_tga_consumes_its_pixel_data_exactly() {
        let mut runs = tga_header(TRUE_COLOR_RLE, DEPTH_32, ALPHA_8);
        runs.push(run(PIXELS as u8));
        runs.extend(BGRA);
        let valid = TgaVerdict::Valid { image_type: TRUE_COLOR_RLE, depth: DEPTH_32, footer: None };
        assert_eq!(validate_tga(&runs), valid);

        let mut mixed = tga_header(TRUE_COLOR_RLE, DEPTH_32, ALPHA_8);
        mixed.push(raw(1));
        mixed.extend(BGRA);
        mixed.push(run(PIXELS as u8 - 1));
        mixed.extend(BGRA);
        assert_eq!(validate_tga(&mixed), valid);
        let mut with_footer = mixed.clone();
        with_footer.extend(footer(0, 0));
        assert_eq!(classify_content(&with_footer).format, ContentFormat::Tga);
        let mut trailing = mixed.clone();
        trailing.extend([1; TRAILING]);
        assert!(matches!(validate_tga(&trailing), TgaVerdict::Trailing { bytes: TRAILING, .. }));

        let truncated = &runs[..runs.len() - 1];
        assert_eq!(validate_tga(truncated), TgaVerdict::BadData("the RLE data is truncated"));
        let mut short_runs = tga_header(TRUE_COLOR_RLE, DEPTH_32, ALPHA_8);
        short_runs.push(run(1));
        short_runs.extend(BGRA);
        assert_eq!(validate_tga(&short_runs), TgaVerdict::BadData("the RLE data is truncated"));
        let mut overrun = tga_header(TRUE_COLOR_RLE, DEPTH_32, ALPHA_8);
        overrun.push(run(PIXELS as u8 + 1));
        overrun.extend(BGRA);
        assert_eq!(validate_tga(&overrun), TgaVerdict::BadData("an RLE packet runs past the last pixel"));
        assert_eq!(classify_content(&overrun).format, ContentFormat::Binary);
    }

    #[test]
    fn tga_header_fields_must_agree_with_the_type() {
        let mut map_fields_without_map = tga_type_2();
        map_fields_without_map[TGA_COLOR_MAP_LENGTH_OFFSET] = 1;
        assert_eq!(validate_tga(&map_fields_without_map), TgaVerdict::BadHeader("colour map fields without a colour map"));
        let mut stale_entry_size = tga_type_2();
        stale_entry_size[TGA_COLOR_MAP_DEPTH_OFFSET] = DEPTH_24;
        assert_eq!(classify_content(&stale_entry_size).format, ContentFormat::Tga);
        stale_entry_size[TGA_COLOR_MAP_DEPTH_OFFSET] = DEPTH_24 + 1;
        assert_eq!(validate_tga(&stale_entry_size), TgaVerdict::BadHeader("colour map fields without a colour map"));
        let mut mapped_without_map = tga_header(COLOR_MAPPED, DEPTH_8, 0);
        mapped_without_map.extend([0; PIXELS]);
        assert_eq!(validate_tga(&mapped_without_map), TgaVerdict::BadHeader("a colour-mapped type without a colour map"));
        let mut grey_24 = tga_header(GREY, DEPTH_24, 0);
        grey_24.extend([0; PIXELS * RGB_BYTES]);
        assert_eq!(validate_tga(&grey_24), TgaVerdict::BadHeader("pixel depth"));
        let mut too_much_alpha = tga_header(GREY, DEPTH_8, TGA_ALPHA_BITS_MASK);
        too_much_alpha.extend([0; PIXELS]);
        assert_eq!(validate_tga(&too_much_alpha), TgaVerdict::BadHeader("image descriptor"));
        let mut empty = tga_type_2();
        empty[TGA_WIDTH_OFFSET] = 0;
        assert_eq!(validate_tga(&empty[..TGA_HEADER_SIZE]), TgaVerdict::BadHeader("zero width or height"));
        assert_eq!(validate_tga(&[0; TGA_HEADER_SIZE - 1]), TgaVerdict::BadHeader("shorter than the header"));

        // A colour-mapped image: two 24-bit entries, then one 8-bit index per pixel.
        let mut mapped = tga_header(COLOR_MAPPED, DEPTH_8, 0);
        mapped[TGA_COLOR_MAP_TYPE_OFFSET] = TGA_HAS_COLOR_MAP;
        mapped[TGA_COLOR_MAP_LENGTH_OFFSET] = MAP_ENTRIES;
        mapped[TGA_COLOR_MAP_DEPTH_OFFSET] = DEPTH_24;
        mapped.extend([0; MAP_ENTRIES as usize * RGB_BYTES]);
        mapped.extend([0, 1, 1, 0]);
        assert_eq!(validate_tga(&mapped), TgaVerdict::Valid { image_type: COLOR_MAPPED, depth: DEPTH_8, footer: None });
    }

    fn dds() -> Vec<u8> {
        let mut dds = DdsHeaderStyle::Linear.convention(Format::A8R8G8B8).unwrap().header_bytes(1, 1).unwrap();
        dds.extend(BGRA);
        dds
    }

    fn png() -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::new(1, 1).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    }

    /// A 2x2 24-bit BMP: rows of 6 bytes padded to 8.
    fn bmp(size_field: u32, extra: usize) -> Vec<u8> {
        let mut bmp = BMP_MAGIC.to_vec();
        bmp.extend(size_field.to_le_bytes());
        bmp.extend(0u32.to_le_bytes());
        bmp.extend((BMP_HEADERS as u32).to_le_bytes());
        bmp.extend(BITMAPINFOHEADER_SIZE.to_le_bytes());
        bmp.extend(i32::from(SIDE).to_le_bytes());
        bmp.extend(i32::from(SIDE).to_le_bytes());
        bmp.extend(BMP_PLANES.to_le_bytes());
        bmp.extend(u16::from(DEPTH_24).to_le_bytes());
        bmp.resize(BMP_HEADERS + BMP_24_ROW * SIDE as usize + extra, 0);
        bmp
    }

    #[test]
    fn magics_identify_content_and_bmp_needs_consistent_sizes() {
        assert_eq!(classify_content(&dds()).format, ContentFormat::Dds);
        assert_eq!(classify_content(&png()).format, ContentFormat::Png);
        assert_eq!(classify_content(&[]).format, ContentFormat::Empty);

        let exact = BMP_HEADERS + BMP_24_ROW * SIDE as usize;
        let class = classify_content(&bmp(exact as u32, 0));
        assert_eq!((class.format, class.detail.as_deref()), (ContentFormat::Bmp, Some("BMP 24 bit")));
        let gimp = classify_content(&bmp((exact + GIMP_PADDING) as u32, GIMP_PADDING));
        assert_eq!(gimp.format, ContentFormat::Bmp);
        assert_eq!(gimp.detail.as_deref(), Some("BMP 24 bit, 2 bytes after the pixels"));
        assert_eq!(classify_content(&bmp((exact / 4) as u32, 0)).format, ContentFormat::BmpBadSize);
        let truncated = bmp(exact as u32, 0);
        assert_eq!(classify_content(&truncated[..exact - 1]).format, ContentFormat::Binary);
    }

    #[test]
    fn mpeg_frames_must_chain() {
        let frame = |header: [u8; 4]| {
            let mut frame = header.to_vec();
            frame.resize(MPEG_FRAME_LEN, 0);
            frame
        };
        let stream: Vec<u8> = (0..MPEG_FRAMES_CHECKED).flat_map(|_| frame(MPEG_HEADER)).collect();
        assert_eq!(classify_content(&stream).format, ContentFormat::Mp3);
        let mut padded = vec![0u8; LEADING_ZEROS];
        padded.extend(&stream);
        let class = classify_content(&padded);
        assert_eq!((class.format, class.detail.as_deref()), (ContentFormat::Mp3, Some("MP3 after 720 zero bytes")));
        assert_eq!(classify_content(&frame(MPEG_HEADER)).format, ContentFormat::Mp3);
        assert_eq!(classify_content(&frame(MPEG_HEADER)[..MPEG_FRAME_LEN - 1]).format, ContentFormat::Binary);
        let mut broken = frame(MPEG_HEADER);
        broken.extend(vec![1u8; MPEG_FRAME_LEN]);
        assert_eq!(classify_content(&broken).format, ContentFormat::Binary);
        let free: Vec<u8> = (0..MPEG_FRAMES_CHECKED).flat_map(|_| frame(MPEG_FREE_HEADER)).collect();
        assert_eq!(classify_content(&free).format, ContentFormat::Binary);
        assert_eq!(classify_content(b"ID3\x03").format, ContentFormat::Mp3);
    }

    #[test]
    fn zlib_streams_must_inflate_to_their_end_and_text_has_no_controls() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[BGRA; PADDING].concat()).unwrap();
        let stream = encoder.finish().unwrap();
        assert_eq!(classify_content(&stream).format, ContentFormat::Zlib);
        assert_eq!(classify_content(&stream[..stream.len() - 1]).format, ContentFormat::Binary);
        let mut extra = stream.clone();
        extra.push(0);
        assert_eq!(classify_content(&extra).format, ContentFormat::Binary);

        assert_eq!(classify_content(b"\xef\xbb\xbf  <?xml version=\"1.0\"?><a/>").format, ContentFormat::Xml);
        assert_eq!(classify_content(b"node key=1\n\tchild\r\n").format, ContentFormat::Text);
        assert_eq!(classify_content(b"text\0").format, ContentFormat::Binary);
    }

    #[test]
    fn declared_formats_come_from_the_last_extension() {
        assert_eq!(declared_format("data/a.b/x.TGA"), Some(ContentFormat::Tga));
        assert_eq!(declared_format("x.jpeg"), Some(ContentFormat::Jpeg));
        assert_eq!(declared_format("data/a.b/x"), None);
        assert_eq!(name_extension("data/a.b/x"), None);
        assert_eq!(name_extension("x.lua").as_deref(), Some("lua"));
    }

    const VERSION: u32 = 5;
    const BLOCK_SIZE: u32 = 100;
    const DDS_NAMED_TGA: &str = "build/test/dds.tga";
    const PNG_NAMED_TGA: &str = "build/test/png.tga";
    const TGA_NAMED_TGA: &str = "build/test/valid.tga";
    const TGA_NAMED_DDS: &str = "build/test/tga.dds";
    const TRUNCATED_RLE_TGA: &str = "build/test/truncated.tga";
    const TGA_NAMED_BIN: &str = "build/test/tga.bin";
    const UNNAMED_PNG: &str = "build/test/unnamed.png";

    /// A package of named and unnamed entries, scanned: the content decides the format, the names are compared.
    #[test]
    fn census_classifies_by_content_and_reports_disagreeing_names() {
        let dir = std::env::temp_dir().join(format!("uocf_image_census_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("images.uop");
        let mut truncated_rle = tga_header(TRUE_COLOR_RLE, DEPTH_32, ALPHA_8);
        truncated_rle.push(run(PIXELS as u8));
        let entries: [(&str, Vec<u8>); 7] = [
            (DDS_NAMED_TGA, dds()),
            (PNG_NAMED_TGA, png()),
            (TGA_NAMED_TGA, tga_type_2()),
            (TGA_NAMED_DDS, tga_type_2()),
            (TRUNCATED_RLE_TGA, truncated_rle),
            (TGA_NAMED_BIN, tga_type_2()),
            (UNNAMED_PNG, png()),
        ];
        let mut package = UopPackage::new(VERSION, BLOCK_SIZE).unwrap();
        for (name, payload) in &entries {
            package.add_file_from_memory(payload, name, CompressionFlag::Zlib).unwrap();
        }
        package.finalize_and_save(&path).unwrap();
        let names: BTreeMap<u64, &str> = entries
            .iter()
            .map(|(name, _)| (hash_file_name_single(name), *name))
            .filter(|(_, name)| *name != UNNAMED_PNG)
            .collect();
        let census = census_package(&path, &|hash| names.get(&hash).map(|name| name.to_string())).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let counts = census.format_counts();
        assert_eq!(counts[&ContentFormat::Dds], 1);
        assert_eq!(counts[&ContentFormat::Png], 2);
        assert_eq!(counts[&ContentFormat::Tga], 3);
        assert_eq!(counts[&ContentFormat::Binary], 1);
        assert_eq!(census.dds.len(), 1);

        let mismatches = census.name_mismatches();
        let names_of = |declared: &str, actual| -> Vec<&str> {
            mismatches[&(declared.to_owned(), actual)].iter().map(|entry| entry.name.as_deref().unwrap()).collect()
        };
        assert_eq!(mismatches.len(), 5, "{:?}", mismatches.keys().collect::<Vec<_>>());
        assert_eq!(names_of("TGA", ContentFormat::Dds), [DDS_NAMED_TGA]);
        assert_eq!(names_of("TGA", ContentFormat::Png), [PNG_NAMED_TGA]);
        assert_eq!(names_of("TGA", ContentFormat::Binary), [TRUNCATED_RLE_TGA]);
        assert_eq!(names_of("DDS", ContentFormat::Tga), [TGA_NAMED_DDS]);
        assert_eq!(names_of(".bin", ContentFormat::Tga), [TGA_NAMED_BIN]);

        let report = ImageCensus { packages: vec![census] }.to_markdown();
        assert!(report.contains("5 named entries disagree with their extension"), "{report}");
        assert!(report.contains("| TGA | DDS | 1 | `build/test/dds.tga`"), "{report}");
        assert!(report.contains("| not TGA: the RLE data is truncated | 1 | 0 | 0 |"), "{report}");
        assert!(report.contains("| magic: PNG | 1 | 0 | 0 |"), "{report}");
    }
}
