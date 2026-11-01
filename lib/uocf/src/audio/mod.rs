//! Audio in the client files, shared by the Classic, Kingdom Reborn and Enhanced readers: what a payload holds by
//! its bytes alone ([`probe_audio`]: MPEG audio with its tags and info header, RIFF WAVE with its chunks, OGG, Bink
//! video, text), decoding to 16-bit PCM ([`decode`]), the census of a client's audio files ([`census`]), the formats
//! the Enhanced Client is known to play ([`validate`]) and conversions to them ([`convert`]).
//! `docs/agent_notes/ec_audio_formats.md` summarizes what the shipped clients hold.

pub mod census;
pub mod convert;
pub mod decode;
pub mod mp3;
pub mod validate;
pub mod wav;

use crate::classic::sound::{BITS_PER_SAMPLE as CLASSIC_BITS, CHANNELS as CLASSIC_CHANNELS, SAMPLE_RATE as CLASSIC_RATE};
use mp3::{MpegAnalysis, MpegLayer};
use wav::WavAnalysis;

pub const OGG_MAGIC: &[u8] = b"OggS";
/// Bink video (`.bik`): the original Bink and Bink 2.
pub const BINK_MAGICS: [&[u8]; 2] = [b"BIK", b"KB2"];
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
/// Bytes of an unknown payload shown in hex.
pub const HEX_PREFIX_BYTES: usize = 16;
const BITS_PER_BYTE: u16 = 8;

/// The format of an audio payload, for grouping and reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AudioFormat {
    /// MPEG audio of one layer (Layer III is MP3).
    Mpeg(MpegLayer),
    Wav,
    /// A Classic sound slot: a name header and raw 22050 Hz mono 16-bit PCM (`sound.mul`, `soundLegacyMUL.uop`).
    ClassicPcm,
    Ogg,
    Bink,
    Text,
    Empty,
    Unknown,
}

impl AudioFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mpeg(MpegLayer::L3) => "MP3",
            Self::Mpeg(MpegLayer::L2) => "MP2",
            Self::Mpeg(MpegLayer::L1) => "MP1",
            Self::Wav => "WAV",
            Self::ClassicPcm => "Classic PCM",
            Self::Ogg => "OGG",
            Self::Bink => "Bink video",
            Self::Text => "text",
            Self::Empty => "empty",
            Self::Unknown => "unknown",
        }
    }

    /// The extension a file of this format usually has, lowercase.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Mpeg(MpegLayer::L3) => Some("mp3"),
            Self::Mpeg(MpegLayer::L2) => Some("mp2"),
            Self::Mpeg(MpegLayer::L1) => Some("mp1"),
            Self::Wav => Some("wav"),
            Self::Ogg => Some("ogg"),
            Self::Bink => Some("bik"),
            Self::ClassicPcm | Self::Text | Self::Empty | Self::Unknown => None,
        }
    }

    /// The format an extension (lowercase, without the dot) names, when it names an audio or video format.
    pub fn of_extension(extension: &str) -> Option<Self> {
        Some(match extension {
            "mp3" => Self::Mpeg(MpegLayer::L3),
            "mp2" => Self::Mpeg(MpegLayer::L2),
            "mp1" => Self::Mpeg(MpegLayer::L1),
            "wav" => Self::Wav,
            "ogg" => Self::Ogg,
            "bik" => Self::Bink,
            "txt" | "csv" | "def" => Self::Text,
            _ => return None,
        })
    }

    /// [`decode::decode`] turns it into PCM (when its codec parameters are supported).
    pub fn is_decodable(self) -> bool {
        matches!(self, Self::Mpeg(MpegLayer::L3) | Self::Wav | Self::ClassicPcm)
    }
}

/// The extension of a name or path, lowercase and without the dot.
pub fn name_extension(name: &str) -> Option<String> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file_name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).filter(|extension| !extension.is_empty())
}

/// What a payload holds.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioContent {
    Mpeg(MpegAnalysis),
    Wav(WavAnalysis),
    /// The PCM bytes after a Classic slot's header.
    ClassicPcm { pcm_len: usize },
    Ogg,
    Bink,
    Text,
    Empty,
    /// None of the above: the first [`HEX_PREFIX_BYTES`] bytes in hex.
    Unknown { hex_prefix: String },
}

impl AudioContent {
    pub fn format(&self) -> AudioFormat {
        match self {
            Self::Mpeg(analysis) => AudioFormat::Mpeg(analysis.first.layer),
            Self::Wav(_) => AudioFormat::Wav,
            Self::ClassicPcm { .. } => AudioFormat::ClassicPcm,
            Self::Ogg => AudioFormat::Ogg,
            Self::Bink => AudioFormat::Bink,
            Self::Text => AudioFormat::Text,
            Self::Empty => AudioFormat::Empty,
            Self::Unknown { .. } => AudioFormat::Unknown,
        }
    }

    pub fn sample_rate(&self) -> Option<u32> {
        match self {
            Self::Mpeg(analysis) => Some(analysis.first.sample_rate),
            Self::Wav(analysis) => analysis.format.map(|format| format.sample_rate),
            Self::ClassicPcm { .. } => Some(CLASSIC_RATE),
            _ => None,
        }
    }

    pub fn channels(&self) -> Option<u16> {
        match self {
            Self::Mpeg(analysis) => Some(analysis.first.channel_mode.channels()),
            Self::Wav(analysis) => analysis.format.map(|format| format.channels),
            Self::ClassicPcm { .. } => Some(CLASSIC_CHANNELS),
            _ => None,
        }
    }

    /// Bits per sample of PCM content.
    pub fn bits_per_sample(&self) -> Option<u16> {
        match self {
            Self::Wav(analysis) => analysis.format.map(|format| format.bits_per_sample),
            Self::ClassicPcm { .. } => Some(CLASSIC_BITS),
            _ => None,
        }
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        match self {
            Self::Mpeg(analysis) => Some(analysis.duration_seconds()),
            Self::Wav(analysis) => analysis.duration_seconds(),
            Self::ClassicPcm { pcm_len } => {
                let frame_bytes = u64::from(CLASSIC_CHANNELS) * u64::from(CLASSIC_BITS / BITS_PER_BYTE);
                Some((*pcm_len as u64 / frame_bytes) as f64 / f64::from(CLASSIC_RATE))
            }
            _ => None,
        }
    }

    /// The codec in a few words: `MPEG-1 Layer III 44100 Hz joint stereo, CBR 128`, `PCM 22050 Hz mono 16 bit`.
    pub fn detail(&self) -> String {
        match self {
            Self::Mpeg(analysis) => format!("{}, {}", analysis.stream_label(), analysis.bitrate_label()),
            Self::Wav(analysis) => match analysis.format {
                Some(format) => format.label(),
                None => "no fmt chunk".to_owned(),
            },
            Self::ClassicPcm { .. } => format!("PCM {CLASSIC_RATE} Hz mono {CLASSIC_BITS} bit"),
            Self::Unknown { hex_prefix } => hex_prefix.clone(),
            other => other.format().label().to_owned(),
        }
    }
}

/// What a standalone audio file or package entry holds, by its bytes alone: an MPEG stream (after ID3v2 tags or
/// zero bytes, [`mp3::analyze_mpeg`]), `RIFF....WAVE`, `OggS`, Bink, UTF-8 text, or unknown. A Classic sound slot
/// has no magic; its readers know it by where it is stored (`crate::classic::sound`).
pub fn probe_audio(payload: &[u8]) -> AudioContent {
    if payload.is_empty() {
        return AudioContent::Empty;
    }
    if let Some(analysis) = wav::analyze_wav(payload) {
        return AudioContent::Wav(analysis);
    }
    if payload.starts_with(OGG_MAGIC) {
        return AudioContent::Ogg;
    }
    if BINK_MAGICS.iter().any(|magic| payload.starts_with(magic)) {
        return AudioContent::Bink;
    }
    if let Some(analysis) = mp3::analyze_mpeg(payload) {
        return AudioContent::Mpeg(analysis);
    }
    if is_text(payload) {
        return AudioContent::Text;
    }
    AudioContent::Unknown { hex_prefix: hex_prefix(payload) }
}

/// UTF-8 (an optional BOM aside) without control characters other than tab and line ends.
fn is_text(payload: &[u8]) -> bool {
    std::str::from_utf8(payload.strip_prefix(UTF8_BOM).unwrap_or(payload))
        .is_ok_and(|text| !text.trim().is_empty() && !text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')))
}

/// The first [`HEX_PREFIX_BYTES`] bytes in hex, space-separated.
pub fn hex_prefix(payload: &[u8]) -> String {
    payload.iter().take(HEX_PREFIX_BYTES).map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_by_content() {
        let wav = wav::encode_pcm_wav(CLASSIC_RATE, CLASSIC_CHANNELS, &[0, 0, 1, 0]);
        let content = probe_audio(&wav);
        assert_eq!(content.format(), AudioFormat::Wav);
        assert_eq!((content.sample_rate(), content.channels(), content.bits_per_sample()), (Some(CLASSIC_RATE), Some(1), Some(16)));
        let stream = mp3::tests::frames(mp3::tests::MPEG1_L3_128K_HEADER, mp3::tests::MPEG1_L3_128K_FRAME, 4);
        assert_eq!(probe_audio(&stream).format(), AudioFormat::Mpeg(MpegLayer::L3));
        assert_eq!(probe_audio(b"OggS\0\x02").format(), AudioFormat::Ogg);
        assert_eq!(probe_audio(b"BIKi....").format(), AudioFormat::Bink);
        assert_eq!(probe_audio(b"axe01.mp3\r\n").format(), AudioFormat::Text);
        assert_eq!(probe_audio(&[]).format(), AudioFormat::Empty);
        assert_eq!(probe_audio(&[0x01, 0x02, 0xfe]), AudioContent::Unknown { hex_prefix: "01 02 fe".to_owned() });
    }

    #[test]
    fn extension_formats() {
        assert_eq!(name_extension("data/audio/sounds/x/flame04 .mp3").as_deref(), Some("mp3"));
        assert_eq!(AudioFormat::of_extension("wav"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::Mpeg(MpegLayer::L3).extension(), Some("mp3"));
    }
}
