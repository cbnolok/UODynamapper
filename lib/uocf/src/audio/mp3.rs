//! MPEG audio streams (`.mp3` and the other layers): the ID3v2 tags before the frames, the frame chain itself, the
//! Xing/Info (with its LAME extension) or VBRI header of the first frame, and the ID3v1/APE tags after the last
//! frame ([`analyze_mpeg`]). Every value is read from the bytes; nothing is taken from a name.

use std::collections::BTreeSet;

use super::validate::{miles_mp3_seek, MilesMp3Seek};

pub const ID3V2_MAGIC: &[u8] = b"ID3";
pub(crate) const ID3V2_HEADER_SIZE: usize = 10;
const ID3V2_MAJOR_OFFSET: usize = 3;
const ID3V2_REVISION_OFFSET: usize = 4;
const ID3V2_FLAGS_OFFSET: usize = 5;
const ID3V2_SIZE_OFFSET: usize = 6;
/// The ID3v2.4 flag announcing a 10-byte footer after the tag.
const ID3V2_FOOTER_FLAG: u8 = 0x10;
const ID3V2_FOOTER_SIZE: usize = 10;
/// ID3v2 sizes are "syncsafe": four bytes of seven bits each.
const SYNCSAFE_BITS: u32 = 7;
const SYNCSAFE_BYTE_LIMIT: u8 = 0x80;
const SYNCSAFE_BYTES: usize = 4;
pub const ID3V1_MAGIC: &[u8] = b"TAG";
pub const ID3V1_SIZE: usize = 128;
const APE_MAGIC: &[u8] = b"APETAGEX";
/// An APEv2 footer: the magic, version, tag size (footer included), item count, flags and 8 reserved bytes.
const APE_FOOTER_SIZE: usize = 32;
const APE_SIZE_OFFSET: usize = 12;

pub const MPEG_HEADER_SIZE: usize = 4;
pub(crate) const SYNC_BYTE: u8 = 0xff;
pub(crate) const SYNC_MASK: u8 = 0xe0;
const VERSION_SHIFT: u8 = 3;
pub(crate) const LAYER_SHIFT: u8 = 1;
pub(crate) const TWO_BITS: u8 = 0b11;
/// The layer bits of Layer III.
pub(crate) const LAYER_3_BITS: u8 = 0b01;
const FOUR_BITS: u8 = 0x0f;
const PROTECTION_ABSENT_BIT: u8 = 0x01;
pub(crate) const BITRATE_SHIFT: u8 = 4;
pub(crate) const SAMPLE_RATE_SHIFT: u8 = 2;
const PADDING_SHIFT: u8 = 1;
const CHANNEL_MODE_SHIFT: u8 = 6;
const BITRATE_FREE: u8 = 0;
pub(crate) const BITRATE_BAD: u8 = 15;
pub(crate) const SAMPLE_RATE_RESERVED: u8 = 3;
/// The 16-bit CRC after the header of a protected frame.
const CRC_SIZE: usize = 2;
const BITS_PER_KILOBIT: u32 = 1000;

/// Bitrates in kbit/s by bitrate index 1..=14, per (version, layer) table.
const BITRATES_KBPS: [[u32; 14]; 5] = [
    [32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    [32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    [32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];
const BITRATE_TABLE_V1_L1: usize = 0;
const BITRATE_TABLE_V1_L2: usize = 1;
const BITRATE_TABLE_V1_L3: usize = 2;
const BITRATE_TABLE_V2_L1: usize = 3;
const BITRATE_TABLE_V2_L2_L3: usize = 4;
const SAMPLE_RATES_V1: [u32; 3] = [44_100, 48_000, 32_000];
const SAMPLE_RATES_V2: [u32; 3] = [22_050, 24_000, 16_000];
const SAMPLE_RATES_V2_5: [u32; 3] = [11_025, 12_000, 8_000];

const LAYER_1_SAMPLES: u32 = 384;
const LAYER_2_SAMPLES: u32 = 1152;
const LAYER_3_V1_SAMPLES: u32 = 1152;
const LAYER_3_V2_SAMPLES: u32 = 576;
/// Frame length = factor * bitrate / sample rate + padding (layer 1: in 4-byte slots).
const LAYER_1_FACTOR: u32 = 12;
const LAYER_1_SLOT_BYTES: u32 = 4;
const LAYER_2_3_FACTOR: u32 = 144;
const LAYER_3_V2_FACTOR: u32 = 72;

/// Layer 3 side information size, which the Xing/Info header follows.
const SIDE_INFO_V1_MONO: usize = 17;
const SIDE_INFO_V1_STEREO: usize = 32;
const SIDE_INFO_V2_MONO: usize = 9;
const SIDE_INFO_V2_STEREO: usize = 17;
const XING_MAGIC: &[u8] = b"Xing";
const INFO_MAGIC: &[u8] = b"Info";
const XING_FLAG_FRAMES: u32 = 0x1;
const XING_FLAG_BYTES: u32 = 0x2;
const XING_FLAG_TOC: u32 = 0x4;
const XING_FLAG_QUALITY: u32 = 0x8;
const XING_TOC_SIZE: usize = 100;
const XING_FIELD_SIZE: usize = 4;
/// The magic and the flags.
const XING_HEADER_SIZE: usize = 8;
/// The LAME extension starts with a 9-character encoder version; the encoder delay and padding (12 bits each)
/// are 21 bytes after its start.
const LAME_VERSION_SIZE: usize = 9;
const LAME_DELAY_OFFSET: usize = 21;
const LAME_DELAY_SIZE: usize = 3;
const LAME_PREFIX: &str = "LAME";
const TWELVE_BITS: u32 = 0xfff;
const NIBBLE_BITS: u32 = 4;
/// The VBRI header (Fraunhofer) sits 32 bytes after the frame header.
const VBRI_OFFSET: usize = MPEG_HEADER_SIZE + 32;
const VBRI_MAGIC: &[u8] = b"VBRI";
const VBRI_DELAY_OFFSET: usize = 6;
const VBRI_BYTES_OFFSET: usize = 10;
const VBRI_FRAMES_OFFSET: usize = 14;
const BITS_PER_BYTE: u32 = 8;

/// Frames that must chain from a candidate start for it to count as an MPEG stream (fewer when the payload ends
/// exactly after them). Frames chain when they share version, layer and sample rate ([`FrameHeader::chain_key`]).
pub const MPEG_FRAMES_TO_SYNC: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MpegVersion {
    V1,
    V2,
    V2_5,
}

impl MpegVersion {
    pub fn label(self) -> &'static str {
        match self {
            Self::V1 => "MPEG-1",
            Self::V2 => "MPEG-2",
            Self::V2_5 => "MPEG-2.5",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MpegLayer {
    L1,
    L2,
    L3,
}

impl MpegLayer {
    pub fn label(self) -> &'static str {
        match self {
            Self::L1 => "Layer I",
            Self::L2 => "Layer II",
            Self::L3 => "Layer III",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelMode {
    Stereo,
    JointStereo,
    DualChannel,
    Mono,
}

impl ChannelMode {
    pub fn channels(self) -> u16 {
        if self == Self::Mono { 1 } else { 2 }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stereo => "stereo",
            Self::JointStereo => "joint stereo",
            Self::DualChannel => "dual channel",
            Self::Mono => "mono",
        }
    }
}

/// One MPEG audio frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: MpegVersion,
    pub layer: MpegLayer,
    /// A 16-bit CRC follows the header.
    pub protected: bool,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub padding: bool,
    pub channel_mode: ChannelMode,
    /// The frame length, header included.
    pub length: usize,
}

impl FrameHeader {
    /// The header `bytes` start with: the sync, a known version and layer, a bitrate index other than free and bad,
    /// a known sample rate.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let &[sync, version_layer, rates, mode] = bytes.first_chunk::<MPEG_HEADER_SIZE>()?;
        if sync != SYNC_BYTE || version_layer & SYNC_MASK != SYNC_MASK {
            return None;
        }
        let version = match (version_layer >> VERSION_SHIFT) & TWO_BITS {
            0b11 => MpegVersion::V1,
            0b10 => MpegVersion::V2,
            0b00 => MpegVersion::V2_5,
            _ => return None,
        };
        let layer = match (version_layer >> LAYER_SHIFT) & TWO_BITS {
            0b11 => MpegLayer::L1,
            0b10 => MpegLayer::L2,
            LAYER_3_BITS => MpegLayer::L3,
            _ => return None,
        };
        let bitrate_index = rates >> BITRATE_SHIFT;
        let rate_index = (rates >> SAMPLE_RATE_SHIFT) & TWO_BITS;
        if bitrate_index == BITRATE_FREE || bitrate_index == BITRATE_BAD || rate_index == SAMPLE_RATE_RESERVED {
            return None;
        }
        let table = match (version, layer) {
            (MpegVersion::V1, MpegLayer::L1) => BITRATE_TABLE_V1_L1,
            (MpegVersion::V1, MpegLayer::L2) => BITRATE_TABLE_V1_L2,
            (MpegVersion::V1, MpegLayer::L3) => BITRATE_TABLE_V1_L3,
            (_, MpegLayer::L1) => BITRATE_TABLE_V2_L1,
            (_, MpegLayer::L2 | MpegLayer::L3) => BITRATE_TABLE_V2_L2_L3,
        };
        let bitrate_kbps = BITRATES_KBPS[table][usize::from(bitrate_index) - 1];
        let rates_of_version = match version {
            MpegVersion::V1 => SAMPLE_RATES_V1,
            MpegVersion::V2 => SAMPLE_RATES_V2,
            MpegVersion::V2_5 => SAMPLE_RATES_V2_5,
        };
        let sample_rate = rates_of_version[usize::from(rate_index)];
        let padding = (rates >> PADDING_SHIFT) & 1 == 1;
        let channel_mode = match mode >> CHANNEL_MODE_SHIFT {
            0b00 => ChannelMode::Stereo,
            0b01 => ChannelMode::JointStereo,
            0b10 => ChannelMode::DualChannel,
            _ => ChannelMode::Mono,
        };
        let bitrate = bitrate_kbps * BITS_PER_KILOBIT;
        let pad = u32::from(padding);
        let length = match layer {
            MpegLayer::L1 => (LAYER_1_FACTOR * bitrate / sample_rate + pad) * LAYER_1_SLOT_BYTES,
            MpegLayer::L3 if version != MpegVersion::V1 => LAYER_3_V2_FACTOR * bitrate / sample_rate + pad,
            _ => LAYER_2_3_FACTOR * bitrate / sample_rate + pad,
        } as usize;
        Some(Self {
            version,
            layer,
            protected: version_layer & PROTECTION_ABSENT_BIT == 0,
            bitrate_kbps,
            sample_rate,
            padding,
            channel_mode,
            length,
        })
    }

    /// Samples per channel in one frame.
    pub fn samples(&self) -> u32 {
        match (self.layer, self.version) {
            (MpegLayer::L1, _) => LAYER_1_SAMPLES,
            (MpegLayer::L2, _) => LAYER_2_SAMPLES,
            (MpegLayer::L3, MpegVersion::V1) => LAYER_3_V1_SAMPLES,
            (MpegLayer::L3, _) => LAYER_3_V2_SAMPLES,
        }
    }

    /// Version, layer, sample rate and channel mode: what the shipped streams keep along their frames.
    pub fn stream_key(&self) -> (MpegVersion, MpegLayer, u32, ChannelMode) {
        (self.version, self.layer, self.sample_rate, self.channel_mode)
    }

    /// Version, layer and sample rate: what every frame of one stream shares. The channel mode may change from frame
    /// to frame, as `mssmp3.asi` allows (it compares only these).
    pub fn chain_key(&self) -> (MpegVersion, MpegLayer, u32) {
        (self.version, self.layer, self.sample_rate)
    }

    /// Where a Xing/Info header starts: after the header, the CRC and the layer 3 side information.
    fn xing_offset(&self) -> usize {
        let side_info = match (self.version, self.channel_mode) {
            (MpegVersion::V1, ChannelMode::Mono) => SIDE_INFO_V1_MONO,
            (MpegVersion::V1, _) => SIDE_INFO_V1_STEREO,
            (_, ChannelMode::Mono) => SIDE_INFO_V2_MONO,
            _ => SIDE_INFO_V2_STEREO,
        };
        MPEG_HEADER_SIZE + if self.protected { CRC_SIZE } else { 0 } + side_info
    }
}

/// One ID3v2 tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id3v2Tag {
    pub offset: usize,
    pub major: u8,
    pub revision: u8,
    pub flags: u8,
    /// The whole tag: header, frames, padding and footer.
    pub length: usize,
}

impl Id3v2Tag {
    /// The tag `bytes` start with, when its size is syncsafe.
    pub fn parse(bytes: &[u8], offset: usize) -> Option<Self> {
        let header = bytes.get(offset..offset + ID3V2_HEADER_SIZE)?;
        if !header.starts_with(ID3V2_MAGIC) {
            return None;
        }
        let size_bytes = &header[ID3V2_SIZE_OFFSET..ID3V2_SIZE_OFFSET + SYNCSAFE_BYTES];
        if size_bytes.iter().any(|&byte| byte >= SYNCSAFE_BYTE_LIMIT) {
            return None;
        }
        let size = size_bytes.iter().fold(0usize, |size, &byte| (size << SYNCSAFE_BITS) | usize::from(byte));
        let flags = header[ID3V2_FLAGS_OFFSET];
        let footer = if flags & ID3V2_FOOTER_FLAG != 0 { ID3V2_FOOTER_SIZE } else { 0 };
        Some(Self {
            offset,
            major: header[ID3V2_MAJOR_OFFSET],
            revision: header[ID3V2_REVISION_OFFSET],
            flags,
            length: ID3V2_HEADER_SIZE + size + footer,
        })
    }

    /// The tag without its footer: the header and the size its size field gives.
    pub fn length_without_footer(&self) -> usize {
        self.length - if self.flags & ID3V2_FOOTER_FLAG != 0 { ID3V2_FOOTER_SIZE } else { 0 }
    }

    pub fn label(&self) -> String {
        format!("ID3v2.{}.{}", self.major, self.revision)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InfoHeaderKind {
    /// `Xing`: a VBR stream.
    Xing,
    /// `Info`: the same header in a CBR stream.
    Info,
    /// `VBRI` (Fraunhofer).
    Vbri,
}

impl InfoHeaderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Xing => "Xing",
            Self::Info => "Info",
            Self::Vbri => "VBRI",
        }
    }
}

/// The header in the first frame describing the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoHeader {
    pub kind: InfoHeaderKind,
    /// The frame count the header gives (without the header's own frame).
    pub frames: Option<u32>,
    pub bytes: Option<u32>,
    pub toc: bool,
    pub quality: Option<u32>,
    /// The encoder version of a LAME extension (`LAME3.99r`, `Lavc58.13`): 9 printable characters.
    pub encoder: Option<String>,
    /// The encoder delay and padding in samples (LAME extension, or the VBRI delay).
    pub delay: Option<u32>,
    pub padding: Option<u32>,
}

impl InfoHeader {
    /// The encoder version starts with `LAME`.
    pub fn is_lame(&self) -> bool {
        self.encoder.as_deref().is_some_and(|encoder| encoder.starts_with(LAME_PREFIX))
    }

    fn parse(frame: &[u8], header: &FrameHeader) -> Option<Self> {
        let xing_at = header.xing_offset();
        let magic = frame.get(xing_at..xing_at + XING_MAGIC.len())?;
        if magic == XING_MAGIC || magic == INFO_MAGIC {
            return Self::parse_xing(frame, xing_at, magic == XING_MAGIC);
        }
        if frame.get(VBRI_OFFSET..VBRI_OFFSET + VBRI_MAGIC.len()) == Some(VBRI_MAGIC) {
            let field = |offset: usize, size: usize| be_uint(frame.get(VBRI_OFFSET + offset..VBRI_OFFSET + offset + size)?);
            return Some(Self {
                kind: InfoHeaderKind::Vbri,
                frames: field(VBRI_FRAMES_OFFSET, XING_FIELD_SIZE),
                bytes: field(VBRI_BYTES_OFFSET, XING_FIELD_SIZE),
                toc: false,
                quality: None,
                encoder: None,
                delay: field(VBRI_DELAY_OFFSET, size_of::<u16>()),
                padding: None,
            });
        }
        None
    }

    fn parse_xing(frame: &[u8], start: usize, vbr: bool) -> Option<Self> {
        let flags = be_uint(frame.get(start + XING_MAGIC.len()..start + XING_HEADER_SIZE)?)?;
        let mut cursor = start + XING_HEADER_SIZE;
        let mut field = |present: bool, size: usize| -> Option<Option<&[u8]>> {
            if !present {
                return Some(None);
            }
            let bytes = frame.get(cursor..cursor + size)?;
            cursor += size;
            Some(Some(bytes))
        };
        let frames = field(flags & XING_FLAG_FRAMES != 0, XING_FIELD_SIZE)?.and_then(be_uint);
        let bytes = field(flags & XING_FLAG_BYTES != 0, XING_FIELD_SIZE)?.and_then(be_uint);
        let toc = field(flags & XING_FLAG_TOC != 0, XING_TOC_SIZE)?.is_some();
        let quality = field(flags & XING_FLAG_QUALITY != 0, XING_FIELD_SIZE)?.and_then(be_uint);
        let encoder = frame
            .get(cursor..cursor + LAME_VERSION_SIZE)
            .filter(|version| version.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' ' || *byte == 0))
            .map(|version| String::from_utf8_lossy(version).trim_end_matches(['\0', ' ']).to_owned())
            .filter(|version| !version.is_empty());
        let (delay, padding) = match (&encoder, frame.get(cursor + LAME_DELAY_OFFSET..cursor + LAME_DELAY_OFFSET + LAME_DELAY_SIZE)) {
            (Some(encoder), Some(&[high, middle, low])) if encoder.starts_with(LAME_PREFIX) => {
                let packed = (u32::from(high) << (2 * BITS_PER_BYTE)) | (u32::from(middle) << BITS_PER_BYTE) | u32::from(low);
                (Some(packed >> (3 * NIBBLE_BITS)), Some(packed & TWELVE_BITS))
            }
            _ => (None, None),
        };
        Some(Self {
            kind: if vbr { InfoHeaderKind::Xing } else { InfoHeaderKind::Info },
            frames,
            bytes,
            toc,
            quality,
            encoder,
            delay,
            padding,
        })
    }
}

fn be_uint(bytes: &[u8]) -> Option<u32> {
    (bytes.len() <= size_of::<u32>()).then(|| bytes.iter().fold(0u32, |value, &byte| (value << BITS_PER_BYTE) | u32::from(byte)))
}

/// What follows the last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamTail {
    pub id3v1: bool,
    /// An APEv2 tag (its footer, before an ID3v1 tag or at the end).
    pub ape: bool,
    /// Bytes after the last frame that are no tag.
    pub trailing_bytes: usize,
    /// Every trailing byte is zero.
    pub trailing_zero: bool,
}

/// An MPEG audio payload, frame by frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MpegAnalysis {
    pub id3v2: Vec<Id3v2Tag>,
    /// Zero bytes between the ID3v2 tags (or the start) and the first frame.
    pub leading_zero_bytes: usize,
    /// Other bytes there (an ID3v2 tag whose size is short, junk).
    pub leading_junk_bytes: usize,
    pub first_frame_offset: usize,
    /// The first frame's header: version, layer, rate and channel mode of the stream.
    pub first: FrameHeader,
    /// Every frame, the info header's included.
    pub frames: u32,
    pub info_header: Option<InfoHeader>,
    /// Frames whose version, layer, sample rate or channel mode differ from the first frame's.
    pub inconsistent_frames: u32,
    /// Of those, the frames whose version, layer or sample rate differ (a channel mode change is not counted).
    pub inconsistent_rate_frames: u32,
    /// The bitrates of the audio frames (the info header's frame excluded), in kbit/s.
    pub bitrates: BTreeSet<u32>,
    /// Frames with a CRC.
    pub protected_frames: u32,
    /// Times the frame chain broke inside the stream and resumed after skipped bytes.
    pub resyncs: u32,
    pub resync_bytes: usize,
    /// The last frame runs past the end of the payload.
    pub truncated_last_frame: bool,
    pub tail: StreamTail,
    /// The payload length.
    pub length: usize,
    /// Bytes inside the frames (the info frame included).
    pub frame_bytes: usize,
    /// Where `mssmp3.asi` starts decoding ([`miles_mp3_seek`]).
    pub miles_seek: MilesMp3Seek,
}

impl MpegAnalysis {
    /// Frames holding audio: all but the info header's.
    pub fn audio_frames(&self) -> u32 {
        self.frames - u32::from(self.info_header.is_some())
    }

    /// Samples per channel the audio frames hold (before any encoder delay and padding is trimmed).
    pub fn samples(&self) -> u64 {
        u64::from(self.audio_frames()) * u64::from(self.first.samples())
    }

    pub fn duration_seconds(&self) -> f64 {
        self.samples() as f64 / f64::from(self.first.sample_rate)
    }

    /// Every audio frame has the same bitrate.
    pub fn is_cbr(&self) -> bool {
        self.bitrates.len() <= 1
    }

    /// The average bitrate over the audio frames in kbit/s, from their bytes and duration.
    pub fn average_bitrate_kbps(&self) -> f64 {
        let seconds = self.duration_seconds();
        if seconds <= 0.0 {
            return 0.0;
        }
        let info_frame = if self.info_header.is_some() { self.first.length } else { 0 };
        (self.frame_bytes.saturating_sub(info_frame) as f64 * f64::from(BITS_PER_BYTE)) / seconds / f64::from(BITS_PER_KILOBIT)
    }

    /// `MPEG-1 Layer III 44100 Hz joint stereo`.
    pub fn stream_label(&self) -> String {
        format!(
            "{} {} {} Hz {}",
            self.first.version.label(),
            self.first.layer.label(),
            self.first.sample_rate,
            self.first.channel_mode.label()
        )
    }

    /// `CBR 128` or `VBR 32-160`.
    pub fn bitrate_label(&self) -> String {
        match (self.bitrates.first(), self.bitrates.last()) {
            (Some(min), Some(max)) if min == max => format!("CBR {min}"),
            (Some(min), Some(max)) => format!("VBR {min}-{max}"),
            _ => "no audio frames".to_owned(),
        }
    }
}

/// The frames chaining from `start` until `MPEG_FRAMES_TO_SYNC` are seen or the payload ends exactly after a frame.
fn chains_from(payload: &[u8], start: usize) -> Option<FrameHeader> {
    let first = FrameHeader::parse(payload.get(start..)?)?;
    let mut offset = start;
    for _ in 0..MPEG_FRAMES_TO_SYNC {
        if offset == payload.len() {
            return Some(first);
        }
        let header = FrameHeader::parse(payload.get(offset..)?)?;
        if header.chain_key() != first.chain_key() {
            return None;
        }
        offset += header.length;
        if offset > payload.len() {
            return None;
        }
    }
    Some(first)
}

/// Where the next frame chain starts at or after `from`.
fn resync(payload: &[u8], from: usize, key: Option<(MpegVersion, MpegLayer, u32)>) -> Option<usize> {
    (from..payload.len().saturating_sub(MPEG_HEADER_SIZE - 1))
        .filter(|&offset| payload[offset] == SYNC_BYTE)
        .find(|&offset| chains_from(payload, offset).is_some_and(|header| key.is_none_or(|key| header.chain_key() == key)))
}

/// The tags at the end of the payload: an ID3v1 tag in the last 128 bytes, an APEv2 footer before it or at the end.
/// A tag counts only when it starts at or after `not_before` (the end of the first frame): bytes inside the first
/// frame, or before it, are no tail tag. Returns where the tags start.
fn tail_tags(payload: &[u8], not_before: usize) -> (usize, bool, bool) {
    let mut end = payload.len();
    let id3v1 = end >= not_before.saturating_add(ID3V1_SIZE) && payload[end - ID3V1_SIZE..].starts_with(ID3V1_MAGIC);
    if id3v1 {
        end -= ID3V1_SIZE;
    }
    let mut ape = false;
    if end >= not_before.saturating_add(APE_FOOTER_SIZE) && payload[end - APE_FOOTER_SIZE..].starts_with(APE_MAGIC) {
        let footer = &payload[end - APE_FOOTER_SIZE..end];
        let size = u32::from_le_bytes(footer[APE_SIZE_OFFSET..APE_SIZE_OFFSET + size_of::<u32>()].try_into().expect("four bytes")) as usize;
        if size >= APE_FOOTER_SIZE && not_before.saturating_add(size) <= end {
            ape = true;
            end -= size;
            // An APEv2 tag may have a header too, of the footer's size, which the size field leaves out.
            if end >= not_before.saturating_add(APE_FOOTER_SIZE) && payload[end - APE_FOOTER_SIZE..].starts_with(APE_MAGIC) {
                end -= APE_FOOTER_SIZE;
            }
        }
    }
    (end, id3v1, ape)
}

/// The MPEG audio stream of `payload`, when one starts after its ID3v2 tags and leading bytes: at least
/// [`MPEG_FRAMES_TO_SYNC`] frames chain (or fewer that end exactly with the payload). Leading bytes are accepted
/// only when they are zeros or follow an ID3v2 tag; a payload whose first frame is found only after other bytes is
/// not taken for MPEG audio.
pub fn analyze_mpeg(payload: &[u8]) -> Option<MpegAnalysis> {
    let mut id3v2 = Vec::new();
    let mut offset = 0;
    while let Some(tag) = Id3v2Tag::parse(payload, offset) {
        offset += tag.length;
        id3v2.push(tag);
    }
    let after_tags = offset.min(payload.len());
    let mut start = after_tags + payload[after_tags..].iter().take_while(|&&byte| byte == 0).count();
    if chains_from(payload, start).is_none() {
        if id3v2.is_empty() {
            return None;
        }
        start = resync(payload, after_tags, None)?;
    }
    let leading = &payload[after_tags..start];
    let (leading_zero_bytes, leading_junk_bytes) =
        if leading.iter().all(|&byte| byte == 0) { (leading.len(), 0) } else { (0, leading.len()) };
    let first = FrameHeader::parse(&payload[start..]).expect("the chain starts with a frame");
    let (tags_start, id3v1, ape) = tail_tags(payload, start.saturating_add(first.length).min(payload.len()));
    let key = first.stream_key();

    let mut analysis = MpegAnalysis {
        id3v2,
        leading_zero_bytes,
        leading_junk_bytes,
        first_frame_offset: start,
        first,
        frames: 0,
        info_header: None,
        inconsistent_frames: 0,
        inconsistent_rate_frames: 0,
        bitrates: BTreeSet::new(),
        protected_frames: 0,
        resyncs: 0,
        resync_bytes: 0,
        truncated_last_frame: false,
        tail: StreamTail { id3v1, ape, ..Default::default() },
        length: payload.len(),
        frame_bytes: 0,
        miles_seek: miles_mp3_seek(payload),
    };
    let mut offset = start;
    while offset < tags_start {
        let Some(header) = FrameHeader::parse(&payload[offset..tags_start]) else {
            // The chain broke: resume at the next chain of the same stream, or stop at the tail.
            if payload[offset..tags_start].iter().all(|&byte| byte == 0) {
                break;
            }
            match resync(&payload[..tags_start], offset + 1, Some(first.chain_key())) {
                Some(next) => {
                    analysis.resyncs += 1;
                    analysis.resync_bytes += next - offset;
                    offset = next;
                    continue;
                }
                None => break,
            }
        };
        let frame_end = offset + header.length;
        if analysis.frames == 0 {
            analysis.info_header = InfoHeader::parse(&payload[offset..frame_end.min(payload.len())], &header);
        }
        if header.stream_key() != key {
            analysis.inconsistent_frames += 1;
            if (header.version, header.layer, header.sample_rate) != (first.version, first.layer, first.sample_rate) {
                analysis.inconsistent_rate_frames += 1;
            }
        }
        if analysis.frames > 0 || analysis.info_header.is_none() {
            analysis.bitrates.insert(header.bitrate_kbps);
        }
        if header.protected {
            analysis.protected_frames += 1;
        }
        analysis.frames += 1;
        if frame_end > tags_start {
            analysis.truncated_last_frame = true;
            analysis.frame_bytes += tags_start - offset;
            offset = tags_start;
            break;
        }
        analysis.frame_bytes += header.length;
        offset = frame_end;
    }
    let trailing = &payload[offset..tags_start];
    analysis.tail.trailing_bytes = trailing.len();
    analysis.tail.trailing_zero = trailing.iter().all(|&byte| byte == 0);
    Some(analysis)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// MPEG-1 Layer III, 128 kbit/s, 44100 Hz, no CRC, no padding, joint stereo: a 417-byte frame.
    pub(crate) const MPEG1_L3_128K_HEADER: [u8; 4] = [0xff, 0xfb, 0x90, 0x40];
    pub(crate) const MPEG1_L3_128K_FRAME: usize = 417;
    /// MPEG-2 Layer III, 64 kbit/s, 22050 Hz, mono: a 208-byte frame.
    pub(crate) const MPEG2_L3_64K_MONO_HEADER: [u8; 4] = [0xff, 0xf3, 0x80, 0xc0];
    pub(crate) const MPEG2_L3_64K_MONO_FRAME: usize = 208;

    pub(crate) fn frames(header: [u8; 4], frame_len: usize, count: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(frame_len * count);
        for _ in 0..count {
            let start = bytes.len();
            bytes.extend_from_slice(&header);
            bytes.resize(start + frame_len, 0);
        }
        bytes
    }

    #[test]
    fn frame_header_fields_and_length() {
        let header = FrameHeader::parse(&MPEG1_L3_128K_HEADER).unwrap();
        assert_eq!(header.version, MpegVersion::V1);
        assert_eq!(header.layer, MpegLayer::L3);
        assert_eq!((header.bitrate_kbps, header.sample_rate), (128, 44_100));
        assert_eq!(header.channel_mode, ChannelMode::JointStereo);
        assert_eq!(header.length, MPEG1_L3_128K_FRAME);
        assert_eq!(header.samples(), LAYER_3_V1_SAMPLES);
        let mono = FrameHeader::parse(&MPEG2_L3_64K_MONO_HEADER).unwrap();
        assert_eq!((mono.version, mono.sample_rate, mono.channel_mode), (MpegVersion::V2, 22_050, ChannelMode::Mono));
        assert_eq!(mono.length, MPEG2_L3_64K_MONO_FRAME);
        assert_eq!(mono.samples(), LAYER_3_V2_SAMPLES);
        assert!(FrameHeader::parse(&[0xff, 0xfb, 0xf0, 0x40]).is_none(), "bad bitrate index");
        assert!(FrameHeader::parse(&[0xff, 0xfb, 0x9c, 0x40]).is_none(), "reserved sample rate");
    }

    #[test]
    fn stream_with_id3_tags_info_header_and_trailing_zeros() {
        let mut payload = b"ID3\x03\x00\x00\x00\x00\x00\x04abcd".to_vec();
        payload.extend_from_slice(&[0, 0]);
        let mut stream = frames(MPEG1_L3_128K_HEADER, MPEG1_L3_128K_FRAME, 5);
        let xing_at = MPEG_HEADER_SIZE + SIDE_INFO_V1_STEREO;
        stream[xing_at..xing_at + 4].copy_from_slice(b"Info");
        stream[xing_at + 4..xing_at + 8].copy_from_slice(&XING_FLAG_FRAMES.to_be_bytes());
        stream[xing_at + 8..xing_at + 12].copy_from_slice(&4u32.to_be_bytes());
        stream[xing_at + 12..xing_at + 12 + LAME_VERSION_SIZE].copy_from_slice(b"LAME3.99r");
        let delay_at = xing_at + 12 + LAME_DELAY_OFFSET;
        stream[delay_at..delay_at + 3].copy_from_slice(&[0x24, 0x00, 0x48]);
        payload.extend_from_slice(&stream);
        payload.extend_from_slice(&[0, 0, 0]);
        let mut tag = vec![0u8; ID3V1_SIZE];
        tag[..3].copy_from_slice(ID3V1_MAGIC);
        payload.extend_from_slice(&tag);

        let analysis = analyze_mpeg(&payload).unwrap();
        assert_eq!(analysis.id3v2.len(), 1);
        assert_eq!(analysis.id3v2[0].label(), "ID3v2.3.0");
        assert_eq!(analysis.leading_zero_bytes, 2);
        assert_eq!(analysis.frames, 5);
        assert_eq!(analysis.audio_frames(), 4);
        let info = analysis.info_header.as_ref().unwrap();
        assert_eq!(info.kind, InfoHeaderKind::Info);
        assert_eq!(info.frames, Some(4));
        assert!(info.is_lame());
        assert_eq!((info.delay, info.padding), (Some(576), Some(72)));
        assert!(analysis.tail.id3v1);
        assert_eq!(analysis.tail.trailing_bytes, 3);
        assert!(analysis.tail.trailing_zero);
        assert!(analysis.is_cbr());
        assert_eq!(analysis.bitrate_label(), "CBR 128");
        assert_eq!(analysis.samples(), 4 * u64::from(LAYER_3_V1_SAMPLES));
    }

    #[test]
    fn resyncs_inside_the_stream_and_rejects_text() {
        let mut payload = frames(MPEG2_L3_64K_MONO_HEADER, MPEG2_L3_64K_MONO_FRAME, 4);
        payload.extend_from_slice(b"junk");
        payload.extend_from_slice(&frames(MPEG2_L3_64K_MONO_HEADER, MPEG2_L3_64K_MONO_FRAME, 4));
        let analysis = analyze_mpeg(&payload).unwrap();
        assert_eq!(analysis.frames, 8);
        assert_eq!((analysis.resyncs, analysis.resync_bytes), (1, 4));
        assert!(analyze_mpeg(b"plain text, no frames").is_none());
        assert!(analyze_mpeg(&MPEG1_L3_128K_HEADER).is_none(), "a lone header that runs past the end");
    }

    #[test]
    fn channel_mode_changes_keep_the_chain() {
        // MPEG-2 Layer III, 64 kbit/s, 22050 Hz, joint stereo: the mono header's frame length.
        const MPEG2_L3_64K_JOINT_HEADER: [u8; 4] = [0xff, 0xf3, 0x80, 0x40];
        let mut payload = frames(MPEG2_L3_64K_MONO_HEADER, MPEG2_L3_64K_MONO_FRAME, 1);
        payload.extend(frames(MPEG2_L3_64K_JOINT_HEADER, MPEG2_L3_64K_MONO_FRAME, 3));
        let analysis = analyze_mpeg(&payload).unwrap();
        assert_eq!((analysis.first_frame_offset, analysis.frames, analysis.resyncs), (0, 4, 0));
        assert_eq!((analysis.inconsistent_frames, analysis.inconsistent_rate_frames), (3, 0));
    }

    /// Tags at the end that would start before the end of the first frame are no tags (these panicked).
    #[test]
    fn tail_tags_inside_the_first_frame_are_ignored() {
        /// MPEG-2.5 Layer III, 8 kbit/s, 8000 Hz, mono: a 72-byte frame.
        const MPEG25_L3_8K_MONO_HEADER: [u8; 4] = [0xff, 0xe3, 0x18, 0xc0];
        const MPEG25_L3_8K_MONO_FRAME: usize = 72;
        const LEADING_ZEROS: usize = 4;
        const ID3V2_BODY: u8 = 100;

        // Zeros, three frames, and an APE footer in their last bytes whose size reaches back past the first frame.
        let mut ape = vec![0u8; LEADING_ZEROS];
        ape.extend(frames(MPEG2_L3_64K_MONO_HEADER, MPEG2_L3_64K_MONO_FRAME, MPEG_FRAMES_TO_SYNC));
        let footer_at = ape.len() - APE_FOOTER_SIZE;
        let size = (ape.len() - LEADING_ZEROS) as u32;
        ape[footer_at..footer_at + APE_MAGIC.len()].copy_from_slice(APE_MAGIC);
        ape[footer_at + APE_SIZE_OFFSET..footer_at + APE_SIZE_OFFSET + size_of::<u32>()].copy_from_slice(&size.to_le_bytes());
        let analysis = analyze_mpeg(&ape).unwrap();
        assert_eq!((analysis.first_frame_offset, analysis.frames), (LEADING_ZEROS, MPEG_FRAMES_TO_SYNC as u32));
        assert!(!analysis.tail.ape);

        // An ID3v2 tag, then one frame ending the payload; `TAG` 128 bytes from the end lies inside the ID3v2 tag.
        let mut id3v1 = b"ID3\x03\x00\x00\x00\x00\x00".to_vec();
        id3v1.push(ID3V2_BODY);
        id3v1.resize(ID3V2_HEADER_SIZE + usize::from(ID3V2_BODY), 0);
        id3v1.extend(frames(MPEG25_L3_8K_MONO_HEADER, MPEG25_L3_8K_MONO_FRAME, 1));
        let tag_at = id3v1.len() - ID3V1_SIZE;
        id3v1[tag_at..tag_at + ID3V1_MAGIC.len()].copy_from_slice(ID3V1_MAGIC);
        let analysis = analyze_mpeg(&id3v1).unwrap();
        assert_eq!(analysis.first.version, MpegVersion::V2_5);
        assert_eq!((analysis.first_frame_offset, analysis.frames), (ID3V2_HEADER_SIZE + usize::from(ID3V2_BODY), 1));
        assert!(!analysis.tail.id3v1);
        assert_eq!(analysis.tail.trailing_bytes, 0);
    }
}
