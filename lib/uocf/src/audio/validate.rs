//! Which audio the Enhanced Client is known to play: the streams its shipped `Audio.uop` holds
//! (`docs/agent_notes/ec_audio_formats.md`), checked before an entry is replaced or added
//! ([`enhanced_format_problems`]).
//!
//! The lists are what the reference release ships, not what the client's decoder accepts: that is a question for
//! the client code (the decoder and its limits are not known yet). Until then anything else is refused, unless the
//! caller asks for [`FormatPolicy::AllowUnverified`], which turns the problems into warnings: for trying a format
//! out in a test client, never for a release.

use std::ops::RangeInclusive;

use super::mp3::{ChannelMode, MpegAnalysis, MpegLayer, MpegVersion};
use super::wav::{WavAnalysis, FMT_EX_SIZE, FMT_PCM_SIZE, PCM_WAV_BITS_PER_SAMPLE, WAVE_FORMAT_PCM};
use super::{name_extension, AudioContent, AudioFormat};

crate::eyre_imports!();

/// The MPEG Layer III streams of the shipped `Audio.uop`: version, sample rate and channel mode.
pub const VERIFIED_MPEG_STREAMS: [(MpegVersion, u32, ChannelMode); 8] = [
    (MpegVersion::V1, 32_000, ChannelMode::Mono),
    (MpegVersion::V1, 44_100, ChannelMode::Mono),
    (MpegVersion::V1, 44_100, ChannelMode::Stereo),
    (MpegVersion::V1, 44_100, ChannelMode::JointStereo),
    (MpegVersion::V2, 16_000, ChannelMode::Mono),
    (MpegVersion::V2, 22_050, ChannelMode::Mono),
    (MpegVersion::V2, 22_050, ChannelMode::Stereo),
    (MpegVersion::V2, 22_050, ChannelMode::JointStereo),
];
/// The constant bitrates the shipped MP3s use run from 64 to 320 kbit/s (64, 128, 160, 192, 320).
pub const VERIFIED_MPEG_BITRATES_KBPS: RangeInclusive<u32> = 64..=320;
/// The PCM WAV streams of the shipped `Audio.uop`: sample rate and channels, all 16-bit. 22048 Hz is how most
/// legacy sounds label the Classic 22050 Hz PCM.
pub const VERIFIED_WAV_STREAMS: [(u32, u16); 5] = [(22_048, 1), (22_050, 1), (44_100, 1), (22_050, 2), (44_100, 2)];
pub const VERIFIED_WAV_BITS: u16 = PCM_WAV_BITS_PER_SAMPLE;
/// The `fmt ` chunk sizes of the shipped WAVs.
pub const VERIFIED_WAV_FMT_SIZES: [u32; 2] = [FMT_PCM_SIZE, FMT_EX_SIZE];

/// What to do with a format the shipped data does not show the client playing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FormatPolicy {
    /// Refuse it.
    #[default]
    VerifiedOnly,
    /// Accept it, reporting the problems as warnings (`--allow-unverified-format`).
    AllowUnverified,
}

impl FormatPolicy {
    /// Fails with `problems` under [`Self::VerifiedOnly`]; returns them as warnings under [`Self::AllowUnverified`].
    pub fn apply(self, what: &str, problems: Vec<String>) -> eyre::Result<Vec<String>> {
        if problems.is_empty() || self == Self::AllowUnverified {
            return Ok(problems);
        }
        eyre::bail!(
            "{what} is not in a format the shipped Audio.uop shows the client playing: {} (allow it with \
             --allow-unverified-format to try it out)",
            problems.join("; ")
        )
    }
}

/// The format an `Audio.uop` entry name asks for: MP3 for `.mp3`, WAV for `.wav`.
pub fn expected_format(name: &str) -> Option<AudioFormat> {
    match name_extension(name)?.as_str() {
        "mp3" => Some(AudioFormat::Mpeg(MpegLayer::L3)),
        "wav" => Some(AudioFormat::Wav),
        _ => None,
    }
}

/// Why `content`, stored as the entry `name`, is not like the shipped entries: empty when it is.
pub fn enhanced_format_problems(name: &str, content: &AudioContent) -> Vec<String> {
    let mut problems = Vec::new();
    match expected_format(name) {
        Some(expected) if expected != content.format() => {
            problems.push(format!("the name asks for {}, the file is {}", expected.label(), content.format().label()));
        }
        Some(_) => {}
        None => problems.push(format!("`{name}` has neither the .mp3 nor the .wav extension")),
    }
    match content {
        AudioContent::Mpeg(analysis) => problems.extend(mpeg_problems(analysis)),
        AudioContent::Wav(analysis) => problems.extend(wav_problems(analysis)),
        other => problems.push(format!("{} is neither MP3 nor WAV", other.format().label())),
    }
    problems
}

fn mpeg_problems(analysis: &MpegAnalysis) -> Vec<String> {
    let first = &analysis.first;
    let mut problems = Vec::new();
    if first.layer != MpegLayer::L3 {
        problems.push(format!("{} (only Layer III ships)", first.layer.label()));
    }
    if !VERIFIED_MPEG_STREAMS.contains(&(first.version, first.sample_rate, first.channel_mode)) {
        problems.push(format!("{} (not among the shipped streams)", analysis.stream_label()));
    }
    if !analysis.is_cbr() {
        problems.push(format!("{} (every shipped MP3 has a constant bitrate)", analysis.bitrate_label()));
    }
    if let Some(bitrate) = analysis.bitrates.iter().find(|bitrate| !VERIFIED_MPEG_BITRATES_KBPS.contains(bitrate)) {
        problems.push(format!(
            "{bitrate} kbit/s (the shipped bitrates run from {} to {})",
            VERIFIED_MPEG_BITRATES_KBPS.start(),
            VERIFIED_MPEG_BITRATES_KBPS.end()
        ));
    }
    if analysis.audio_frames() == 0 {
        problems.push("no audio frames".to_owned());
    }
    let defects = [
        (analysis.inconsistent_frames > 0, format!("{} frames of another stream", analysis.inconsistent_frames)),
        (analysis.resyncs > 0, format!("{} breaks in the frame chain", analysis.resyncs)),
        (analysis.truncated_last_frame, "the last frame is cut".to_owned()),
        (analysis.leading_junk_bytes > 0, format!("{} bytes that are no frame before the first", analysis.leading_junk_bytes)),
        (analysis.tail.trailing_bytes > 0, format!("{} bytes that are no tag after the last frame", analysis.tail.trailing_bytes)),
        (analysis.tail.ape, "an APE tag".to_owned()),
    ];
    problems.extend(defects.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem));
    problems
}

fn wav_problems(analysis: &WavAnalysis) -> Vec<String> {
    let Some(format) = analysis.format else {
        return vec!["no fmt chunk".to_owned()];
    };
    let mut problems = Vec::new();
    if format.format_tag != WAVE_FORMAT_PCM || format.bits_per_sample != VERIFIED_WAV_BITS {
        problems.push(format!("{} (every shipped WAV is plain {VERIFIED_WAV_BITS}-bit PCM)", format.label()));
    } else if !VERIFIED_WAV_STREAMS.contains(&(format.sample_rate, format.channels)) {
        problems.push(format!("{} (not among the shipped streams)", format.label()));
    }
    if !format.is_consistent() {
        problems.push("byte rate or block align do not follow from the format".to_owned());
    }
    if !VERIFIED_WAV_FMT_SIZES.contains(&format.chunk_size) {
        problems.push(format!("a fmt chunk of {} bytes", format.chunk_size));
    }
    let position = |id| analysis.chunks.iter().position(|chunk| chunk.id == id);
    match (position(super::wav::FMT_CHUNK_ID), position(super::wav::DATA_CHUNK_ID)) {
        (_, None) => problems.push("no data chunk".to_owned()),
        (Some(fmt), Some(data)) if data < fmt => problems.push("the data chunk comes before fmt".to_owned()),
        _ => {}
    }
    let block_align = usize::from(format.block_align);
    let defects = [
        (analysis.data_len == 0, "no samples".to_owned()),
        (analysis.data_truncated, "the data chunk is cut".to_owned()),
        (block_align > 0 && analysis.data_len % block_align != 0, "the data is not whole sample frames".to_owned()),
        (!analysis.riff_size_matches(), "the RIFF size is not the file's".to_owned()),
        (analysis.trailing_bytes > 0, format!("{} bytes after the last chunk", analysis.trailing_bytes)),
    ];
    problems.extend(defects.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem));
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{mp3, probe_audio, wav};

    const MP3_NAME: &str = "data/audio/sounds/test/a.mp3";
    const WAV_NAME: &str = "data/audio/legacysounds/test/a.wav";

    #[test]
    fn accepts_the_shipped_kinds_of_stream() {
        let mpeg2 = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        assert_eq!(enhanced_format_problems(MP3_NAME, &probe_audio(&mpeg2)), Vec::<String>::new());
        let mpeg1 = mp3::tests::frames(mp3::tests::MPEG1_L3_128K_HEADER, mp3::tests::MPEG1_L3_128K_FRAME, 4);
        let problems = enhanced_format_problems(MP3_NAME, &probe_audio(&mpeg1));
        // The synthetic MPEG-1 frames are 44100 Hz joint stereo, a shipped stream.
        assert!(problems.is_empty(), "{problems:?}");
        let pcm = wav::encode_pcm_wav(22_050, 1, &[0, 0, 1, 0]);
        assert!(enhanced_format_problems(WAV_NAME, &probe_audio(&pcm)).is_empty());
        let with_chunks = wav::tests::sample_wav(&[0, 0]);
        assert!(enhanced_format_problems(WAV_NAME, &probe_audio(&with_chunks)).is_empty(), "18-byte fmt and an odd LIST");
    }

    #[test]
    fn reports_what_differs_and_applies_the_policy() {
        let wav_48k = wav::encode_pcm_wav(48_000, 1, &[0, 0]);
        let problems = enhanced_format_problems(WAV_NAME, &probe_audio(&wav_48k));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("48000 Hz"));

        let mismatch = enhanced_format_problems(MP3_NAME, &probe_audio(&wav::encode_pcm_wav(22_050, 1, &[0, 0])));
        assert!(mismatch[0].contains("the name asks for MP3, the file is WAV"), "{mismatch:?}");
        assert!(enhanced_format_problems("data/audio/a.ogg", &AudioContent::Ogg).len() == 2);

        let mut cut = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        cut.truncate(cut.len() - 1);
        assert!(enhanced_format_problems(MP3_NAME, &probe_audio(&cut)).iter().any(|problem| problem.contains("cut")));

        let error = FormatPolicy::VerifiedOnly.apply("x.wav", problems.clone()).unwrap_err().to_string();
        assert!(error.contains("--allow-unverified-format"), "{error}");
        assert_eq!(FormatPolicy::AllowUnverified.apply("x.wav", problems.clone()).unwrap(), problems);
        assert!(FormatPolicy::VerifiedOnly.apply("x.wav", Vec::new()).unwrap().is_empty());
    }
}
