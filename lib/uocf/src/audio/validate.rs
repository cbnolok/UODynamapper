//! Which audio the Enhanced Client plays: what its decoder, Miles Sound System 7.2c (`mss32.dll` and
//! `miles/mssmp3.asi`), accepts, checked before an entry is replaced or added ([`enhanced_format_report`]).
//!
//! The client hands the whole entry to `AIL_set_named_sample_file` with the entry name, and checks nothing itself.
//! Miles picks the decoder by the name's extension (a leading ID3v2 tag forces MP3), so a file of another kind than
//! its name ([`kind_mismatch`]) is refused under every policy. The rest of the rules come from the Miles code
//! (`scripts/dev-helpers/ec-research/ghidra-audio/results/miles_audio.md`); `docs/agent_notes/ec_audio_formats.md`
//! names the evidence of each:
//!
//! - **WAV** (the built-in parser): `WAVE` at +8; the chunk walk steps `size + (size & 1) + 8` from +12 within the
//!   RIFF size, so it must reach the `fmt ` and `data` chunks our walk finds (an odd chunk before them needs its
//!   pad byte); `fmt ` of 16 bytes or more (Miles reads the first 16); PCM of 8 or 16 bits, IMA ADPCM, or
//!   `WAVE_FORMAT_EXTENSIBLE` with `cbSize` ≥ 22, the PCM sub-format and 16 bits; mono or stereo; any rate; a `data`
//!   chunk inside the file (Miles does not clamp it).
//! - **MP3** (`mssmp3.asi`): Layer III, MPEG-1, 2 or 2.5; the first frame header Miles' byte scan locks onto
//!   ([`miles_mp3_seek`]: within 8 KiB after the last tag it skips) must be the stream's first frame; every frame of
//!   the first frame's version, layer and rate (the channel mode may change); any bitrate, VBR included; ID3v1 and
//!   ID3v2 tags.
//!
//! [`FormatReport`] sorts what differs: `refused` (Miles refuses it, or there is nothing to play), `unverified`
//! (the code read does not say; refused unless the caller asks for [`FormatPolicy::AllowUnverified`], for trying a
//! file out in a test client, never for a release) and `notes` (played, but unlike the shipped entries; always
//! accepted, reported as warnings).

use super::mp3::{
    ChannelMode, Id3v2Tag, MpegAnalysis, MpegLayer, MpegVersion, BITRATE_BAD, BITRATE_SHIFT, ID3V1_MAGIC, ID3V1_SIZE,
    ID3V2_HEADER_SIZE, ID3V2_MAGIC, LAYER_3_BITS, LAYER_SHIFT, MPEG_HEADER_SIZE, SAMPLE_RATE_RESERVED, SAMPLE_RATE_SHIFT, SYNC_BYTE,
    SYNC_MASK, TWO_BITS,
};
use super::wav::{
    WavAnalysis, CHUNK_HEADER_SIZE, DATA_CHUNK_ID, FMT_CHUNK_ID, KSDATAFORMAT_SUBTYPE_PCM, RIFF_HEADER_SIZE,
    WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IMA_ADPCM, WAVE_FORMAT_PCM,
};
use super::{name_extension, AudioContent, AudioFormat};
use crate::enhanced::audio::LEGACY_SOUNDS_ROOT;

crate::eyre_imports!();

/// How far `mssmp3.asi` scans for the first frame sync, from where its scan (re)starts: the start of the stream, or
/// the end of a tag it skipped. It reads a byte while fewer than this many bytes lie between the window start and
/// it, so a header must start within `MILES_MP3_SYNC_WINDOW - (MPEG_HEADER_SIZE - 1)` bytes.
pub const MILES_MP3_SYNC_WINDOW: usize = 0x2000;
/// Miles takes a leading ID3v2 tag only when both version bytes are below this.
pub const MILES_ID3V2_VERSION_LIMIT: u8 = 0xff;
/// The PCM sample sizes the built-in WAV parser plays.
pub const MILES_PCM_BITS: [u16; 2] = [8, 16];
/// IMA ADPCM's sample size, which Miles decodes natively.
pub const MILES_IMA_ADPCM_BITS: u16 = 4;
/// `WAVE_FORMAT_EXTENSIBLE`: the smallest `cbSize`, and the one sample size (block align = channels × 2).
pub const MILES_EXTENSIBLE_MIN_CB_SIZE: u16 = 22;
pub const MILES_EXTENSIBLE_BITS: u16 = 16;
/// Mono and stereo; more channels take Miles' multichannel path, which was not read.
pub const MILES_MAX_CHANNELS: u16 = 2;
const BITS_PER_BYTE: u16 = 8;

/// The MPEG Layer III streams of the shipped `Audio.uop`: version, sample rate and channel mode. Others play; they
/// are reported as notes.
pub const SHIPPED_MPEG_STREAMS: [(MpegVersion, u32, ChannelMode); 8] = [
    (MpegVersion::V1, 32_000, ChannelMode::Mono),
    (MpegVersion::V1, 44_100, ChannelMode::Mono),
    (MpegVersion::V1, 44_100, ChannelMode::Stereo),
    (MpegVersion::V1, 44_100, ChannelMode::JointStereo),
    (MpegVersion::V2, 16_000, ChannelMode::Mono),
    (MpegVersion::V2, 22_050, ChannelMode::Mono),
    (MpegVersion::V2, 22_050, ChannelMode::Stereo),
    (MpegVersion::V2, 22_050, ChannelMode::JointStereo),
];
/// The stream of the legacy WAVs (`data/audio/legacysounds/`): the Classic client's playback format, which the
/// Classic tooling expects. Others are reported as notes.
pub const LEGACY_WAV_STREAM: (u32, u16, u16) = (22_048, 1, 16);

/// What to do with what the Miles code read does not settle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FormatPolicy {
    /// Refuse it.
    #[default]
    VerifiedOnly,
    /// Accept it, reporting it as warnings (`--allow-unverified-format`).
    AllowUnverified,
}

/// How a file compares with what the Enhanced Client plays (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatReport {
    /// Miles refuses it, or there is nothing to play: never stored.
    pub refused: Vec<String>,
    /// The Miles code read does not say whether it plays.
    pub unverified: Vec<String>,
    /// It plays, but differs from the shipped entries.
    pub notes: Vec<String>,
}

impl FormatReport {
    /// Nothing refused and nothing unverified.
    pub fn is_verified(&self) -> bool {
        self.refused.is_empty() && self.unverified.is_empty()
    }
}

impl FormatPolicy {
    /// Fails on anything `report` refuses, and on what is unverified under [`Self::VerifiedOnly`]; returns the
    /// warnings: the unverified problems let through, then the notes.
    pub fn apply(self, what: &str, report: FormatReport) -> eyre::Result<Vec<String>> {
        if !report.refused.is_empty() {
            eyre::bail!("{what} is not a file the Enhanced Client's decoder (Miles 7.2c) plays: {}", report.refused.join("; "))
        }
        if !report.unverified.is_empty() && self == Self::VerifiedOnly {
            eyre::bail!(
                "{what}: whether the Enhanced Client's decoder (Miles 7.2c) plays this is not verified: {} (allow it with \
                 --allow-unverified-format to try it out)",
                report.unverified.join("; ")
            )
        }
        Ok(report.unverified.into_iter().chain(report.notes).collect())
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

/// Why `content` cannot be stored as the entry `name` under any policy: the name has neither audio extension, or
/// the file is of another kind than the extension says.
pub fn kind_mismatch(name: &str, content: &AudioContent) -> Option<String> {
    match expected_format(name) {
        Some(expected) if expected != content.format() => {
            Some(format!("the name asks for {}, the file is {}", expected.label(), content.format().label()))
        }
        Some(_) => None,
        None => Some(format!("`{name}` has neither the .mp3 nor the .wav extension")),
    }
}

/// How `content`, stored as the entry `name`, compares with what the Enhanced Client plays. The first refusal is
/// the [`kind_mismatch`] when there is one.
pub fn enhanced_format_report(name: &str, content: &AudioContent) -> FormatReport {
    let mut report = FormatReport { refused: kind_mismatch(name, content).into_iter().collect(), ..FormatReport::default() };
    let legacy = name.starts_with(LEGACY_SOUNDS_ROOT);
    match content {
        AudioContent::Mpeg(analysis) => mpeg_report(analysis, &mut report),
        AudioContent::Wav(analysis) => wav_report(analysis, legacy, &mut report),
        other => report.refused.push(format!("{} is neither MP3 nor WAV", other.format().label())),
    }
    if legacy {
        report.notes.push(format!(
            "the Enhanced Client never plays `{LEGACY_SOUNDS_ROOT}` entries: it plays the MP3 files audio_sounds.csv names"
        ));
    }
    report
}

/// Where `mssmp3.asi` starts decoding an MP3 ([`miles_mp3_seek`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilesMp3Seek {
    /// The offset of the frame header it locks onto.
    Frame(usize),
    /// It finds no header: within [`MILES_MP3_SYNC_WINDOW`] bytes of `window_start`, or before the payload ends.
    NotFound { window_start: usize },
}

/// An ID3v2 tag Miles skips: both version bytes below [`MILES_ID3V2_VERSION_LIMIT`] and a syncsafe size.
fn miles_id3v2_tag(payload: &[u8], offset: usize) -> Option<Id3v2Tag> {
    Id3v2Tag::parse(payload, offset)
        .filter(|tag| tag.major < MILES_ID3V2_VERSION_LIMIT && tag.revision < MILES_ID3V2_VERSION_LIMIT)
}

/// The first frame header `mssmp3.asi` locks onto, simulating its byte scan (`SeekFrameHeader`, after
/// `SetNamedSampleFile_Impl` skipped a leading ID3v2 tag without its footer):
///
/// - it reads one byte at a time while the byte is at most [`MILES_MP3_SYNC_WINDOW`] bytes past the window start
///   (a header starting `scanned` bytes in is found when `scanned + MPEG_HEADER_SIZE - 1 <= MILES_MP3_SYNC_WINDOW`);
/// - when the last three bytes read are `TAG`, it skips the rest of the 128-byte ID3v1 tag and restarts the window;
/// - when they are `ID3`, it reads the 7 bytes left of the tag header: a valid tag is skipped (with its footer) and
///   the window restarts, an invalid one is passed over in the same window;
/// - when the last four bytes read are a frame sync (`0xff`, then three set bits), it takes them for the first
///   frame if they are Layer III with a bitrate index other than 15 and a sample rate index other than 3; otherwise
///   it scans on. Nothing checks that the next frame follows.
pub fn miles_mp3_seek(payload: &[u8]) -> MilesMp3Seek {
    let mut pos = miles_id3v2_tag(payload, 0).map_or(0, |tag| tag.length_without_footer());
    let mut window_start = pos;
    // The last four bytes read, oldest first.
    let mut read = [0u8; MPEG_HEADER_SIZE];
    loop {
        if pos - window_start > MILES_MP3_SYNC_WINDOW {
            return MilesMp3Seek::NotFound { window_start };
        }
        let Some(&byte) = payload.get(pos) else { return MilesMp3Seek::NotFound { window_start } };
        pos += 1;
        read = [read[1], read[2], read[3], byte];
        let last_three = &read[1..];
        if last_three == ID3V1_MAGIC {
            pos = pos - ID3V1_MAGIC.len() + ID3V1_SIZE;
            window_start = pos;
        } else if last_three == ID3V2_MAGIC {
            let tag_offset = pos - ID3V2_MAGIC.len();
            match miles_id3v2_tag(payload, tag_offset) {
                Some(tag) => {
                    pos = tag_offset + tag.length;
                    window_start = pos;
                }
                None if payload.len() < tag_offset + ID3V2_HEADER_SIZE => return MilesMp3Seek::NotFound { window_start },
                None => pos = tag_offset + ID3V2_HEADER_SIZE,
            }
        } else if read[0] == SYNC_BYTE && read[1] & SYNC_MASK == SYNC_MASK {
            let layer_3 = (read[1] >> LAYER_SHIFT) & TWO_BITS == LAYER_3_BITS;
            let bitrate_known = read[2] >> BITRATE_SHIFT != BITRATE_BAD;
            let rate_known = (read[2] >> SAMPLE_RATE_SHIFT) & TWO_BITS != SAMPLE_RATE_RESERVED;
            if layer_3 && bitrate_known && rate_known {
                return MilesMp3Seek::Frame(pos - MPEG_HEADER_SIZE);
            }
        }
    }
}

fn mpeg_report(analysis: &MpegAnalysis, report: &mut FormatReport) {
    let first = &analysis.first;
    if first.layer != MpegLayer::L3 {
        report.refused.push(format!("{} (mssmp3.asi decodes Layer III only)", first.layer.label()));
    }
    match analysis.miles_seek {
        MilesMp3Seek::Frame(offset) if offset == analysis.first_frame_offset => {}
        MilesMp3Seek::Frame(offset) => report.refused.push(format!(
            "mssmp3.asi's byte scan takes the bytes at {offset} for the first frame, not the stream's first frame at {} \
             (it would decode from there)",
            analysis.first_frame_offset
        )),
        MilesMp3Seek::NotFound { window_start } => report.refused.push(format!(
            "the first frame is {} bytes in (mssmp3.asi scans {MILES_MP3_SYNC_WINDOW} bytes for it, from byte \
             {window_start}, after the last tag it skips)",
            analysis.first_frame_offset.saturating_sub(window_start)
        )),
    }
    if analysis.audio_frames() == 0 {
        report.refused.push("no audio frames".to_owned());
    }
    if analysis.inconsistent_rate_frames > 0 {
        report.refused.push(format!(
            "{} frames of another version, layer or sample rate than the first (mssmp3.asi takes only frames like the first)",
            analysis.inconsistent_rate_frames
        ));
    }
    let channel_mode_changes = analysis.inconsistent_frames - analysis.inconsistent_rate_frames;
    let unverified = [
        (analysis.resyncs > 0, format!("{} breaks in the frame chain", analysis.resyncs)),
        (analysis.truncated_last_frame, "the last frame is cut".to_owned()),
        (analysis.tail.trailing_bytes > 0, format!("{} bytes that are no tag after the last frame", analysis.tail.trailing_bytes)),
        (analysis.tail.ape, "an APE tag (Miles skips ID3 tags only)".to_owned()),
    ];
    report.unverified.extend(unverified.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem));
    if first.layer == MpegLayer::L3 && !SHIPPED_MPEG_STREAMS.contains(&(first.version, first.sample_rate, first.channel_mode)) {
        report.notes.push(format!("{} is not among the shipped streams", analysis.stream_label()));
    }
    if channel_mode_changes > 0 {
        // mssmp3.asi compares version, layer and sample rate only; no shipped stream changes its channel mode.
        report.notes.push(format!("{channel_mode_changes} frames of another channel mode than the first (no shipped stream has one)"));
    }
}

/// The offset of the chunk Miles' walk finds for `id`: from +12, `size + (size & 1) + 8` per chunk, ids compared
/// ignoring case, within the RIFF size and the payload. The walk is run over the chunks our walk found: where it
/// lands on no chunk of ours (the byte after an unpadded odd chunk, past the end), it finds nothing.
fn miles_find_chunk(analysis: &WavAnalysis, id: [u8; 4]) -> Option<usize> {
    let bound = (analysis.riff_size as usize).saturating_add(CHUNK_HEADER_SIZE).min(analysis.length);
    let mut offset = RIFF_HEADER_SIZE;
    while offset + CHUNK_HEADER_SIZE <= bound {
        let chunk = analysis.chunks.iter().find(|chunk| chunk.offset == offset)?;
        if chunk.id.eq_ignore_ascii_case(&id) {
            return Some(offset);
        }
        offset = offset.checked_add(CHUNK_HEADER_SIZE + chunk.size as usize + (chunk.size as usize & 1))?;
    }
    None
}

/// Why Miles' walk does not find the `fmt ` and `data` chunks ours finds; `None` when it does. An unpadded odd
/// chunk sends Miles to the byte after it, so it matters only before `fmt ` or `data`.
fn miles_walk_problem(analysis: &WavAnalysis) -> Option<String> {
    for (id, label) in [(FMT_CHUNK_ID, "fmt "), (DATA_CHUNK_ID, "data")] {
        let Some(ours) = analysis.chunks.iter().find(|chunk| chunk.id == id).map(|chunk| chunk.offset) else { continue };
        if miles_find_chunk(analysis, id) != Some(ours) {
            let unpadded = analysis.chunks.iter().any(|chunk| chunk.unpadded && chunk.offset < ours);
            let why = if unpadded { "an odd chunk before it has no pad byte" } else { "the RIFF size ends before it" };
            return Some(format!("Miles' chunk walk misses the `{label}` chunk: {why}"));
        }
    }
    None
}

fn wav_report(analysis: &WavAnalysis, legacy: bool, report: &mut FormatReport) {
    let Some(format) = analysis.format else {
        report.refused.push("no fmt chunk".to_owned());
        return;
    };
    let tag = format.format_tag;
    let bits = format.bits_per_sample;
    let playable = match tag {
        WAVE_FORMAT_PCM => MILES_PCM_BITS.contains(&bits),
        WAVE_FORMAT_IMA_ADPCM => bits == MILES_IMA_ADPCM_BITS,
        WAVE_FORMAT_EXTENSIBLE => {
            let bad = [
                (format.cb_size.is_none_or(|size| size < MILES_EXTENSIBLE_MIN_CB_SIZE), format!("a cbSize below {MILES_EXTENSIBLE_MIN_CB_SIZE}")),
                (format.sub_format_guid != Some(KSDATAFORMAT_SUBTYPE_PCM), "a sub-format other than PCM".to_owned()),
                (bits != MILES_EXTENSIBLE_BITS || format.block_align != format.channels * (MILES_EXTENSIBLE_BITS / BITS_PER_BYTE),
                    format!("Miles takes {MILES_EXTENSIBLE_BITS}-bit samples only")),
            ];
            let problems: Vec<String> = bad.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem).collect();
            if !problems.is_empty() {
                report.refused.push(format!("extensible format with {}", problems.join(", ")));
            }
            problems.is_empty()
        }
        _ => false,
    };
    if !playable && tag != WAVE_FORMAT_EXTENSIBLE {
        report.refused.push(format!("{} (Miles plays PCM of 8 or 16 bits and IMA ADPCM)", format.label()));
    }
    if format.channels == 0 || format.channels > MILES_MAX_CHANNELS {
        report.refused.push(format!("{} channels (mono or stereo only)", format.channels));
    }
    if format.sample_rate == 0 {
        report.refused.push("a sample rate of 0".to_owned());
    }
    match analysis.data_offset {
        None => report.refused.push("no data chunk".to_owned()),
        Some(_) if analysis.data_truncated => report.refused.push("the data chunk runs past the file (Miles does not clamp it)".to_owned()),
        Some(_) if analysis.data_len == 0 => report.refused.push("no samples".to_owned()),
        Some(_) => {}
    }
    report.refused.extend(miles_walk_problem(analysis));
    let block_align = usize::from(format.block_align);
    let pcm = format.effective_tag() == WAVE_FORMAT_PCM;
    let unverified = [
        (pcm && !format.is_consistent(), "byte rate or block align do not follow from the format".to_owned()),
        (pcm && block_align > 0 && analysis.data_len % block_align != 0, "the data is not whole sample frames".to_owned()),
    ];
    report.unverified.extend(unverified.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem));
    let notes = [
        (!analysis.riff_size_matches(), "the RIFF size is not the file's".to_owned()),
        (analysis.trailing_bytes > 0, format!("{} bytes after the last chunk", analysis.trailing_bytes)),
        (legacy && (format.sample_rate, format.channels, bits) != LEGACY_WAV_STREAM, {
            let (rate, channels, bits) = LEGACY_WAV_STREAM;
            format!("{} (the legacy WAVs, like Classic sounds, are {rate} Hz, {channels} channel, {bits}-bit)", format.label())
        }),
    ];
    report.notes.extend(notes.into_iter().filter(|(found, _)| *found).map(|(_, problem)| problem));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{mp3, probe_audio, wav};

    const MP3_NAME: &str = "data/audio/sounds/test/a.mp3";
    const WAV_NAME: &str = "data/audio/legacysounds/test/a.wav";
    const OTHER_WAV_NAME: &str = "data/audio/test/a.wav";

    fn report(name: &str, file: &[u8]) -> FormatReport {
        enhanced_format_report(name, &probe_audio(file))
    }

    #[test]
    fn accepts_what_miles_plays() {
        let mpeg2 = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        assert_eq!(report(MP3_NAME, &mpeg2), FormatReport::default());
        let mpeg1 = mp3::tests::frames(mp3::tests::MPEG1_L3_128K_HEADER, mp3::tests::MPEG1_L3_128K_FRAME, 4);
        assert!(report(MP3_NAME, &mpeg1).is_verified(), "44100 Hz joint stereo, a shipped stream");
        // MPEG-2 Layer III, 32 kbit/s, 22050 Hz, mono: a 104-byte frame.
        const MPEG2_L3_32K_MONO_HEADER: [u8; 4] = [0xff, 0xf3, 0x40, 0xc0];
        const MPEG2_L3_32K_MONO_FRAME: usize = 104;
        let mut vbr = mpeg2.clone();
        vbr.extend(mp3::tests::frames(MPEG2_L3_32K_MONO_HEADER, MPEG2_L3_32K_MONO_FRAME, 3));
        assert!(report(MP3_NAME, &vbr).is_verified(), "VBR: Miles does not compare bitrates");

        let pcm = wav::encode_pcm_wav(22_048, 1, &[0, 0, 1, 0]);
        assert_eq!(report(WAV_NAME, &pcm).refused, Vec::<String>::new());
        assert_eq!(report(WAV_NAME, &pcm).notes.len(), 1, "the EC does not play legacy WAVs");
        assert_eq!(report(OTHER_WAV_NAME, &wav::encode_pcm_wav(48_000, 2, &[0, 0, 0, 0])), FormatReport::default(), "any rate");
        let eight_bit = wav::tests::wav_with_format(WAVE_FORMAT_PCM, 8, &[0x80, 0x80]);
        assert!(report(OTHER_WAV_NAME, &eight_bit).is_verified());
        let ima = wav::tests::wav_with_format(WAVE_FORMAT_IMA_ADPCM, MILES_IMA_ADPCM_BITS, &[0; 4]);
        assert!(report(OTHER_WAV_NAME, &ima).refused.is_empty(), "{:?}", report(OTHER_WAV_NAME, &ima));
        let with_chunks = wav::tests::sample_wav(&[0, 0]);
        let found = report(OTHER_WAV_NAME, &with_chunks);
        assert!(found.refused.iter().any(|problem| problem.contains("no pad byte")), "an unpadded LIST before data: {found:?}");
        // fmt, data, then an odd `bext` without its pad byte and a `cart`: as 110 shipped legacy WAVs.
        let mut last = wav::encode_pcm_wav(22_050, 1, &[0, 0]);
        last.extend_from_slice(b"bext\x03\0\0\0abccart\0\0\0\0");
        let riff_size = (last.len() - CHUNK_HEADER_SIZE) as u32;
        last[CHUNK_HEADER_SIZE / 2..CHUNK_HEADER_SIZE].copy_from_slice(&riff_size.to_le_bytes());
        assert_eq!(crate::audio::wav::analyze_wav(&last).unwrap().unpadded_chunks(), 1);
        let found = report(OTHER_WAV_NAME, &last);
        assert!(found.is_verified(), "an odd chunk after fmt and data is never walked: {found:?}");
    }

    fn stream() -> Vec<u8> {
        mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4)
    }

    /// An ID3v2.3 tag of `body` bytes after its header.
    fn id3v2(body: usize) -> Vec<u8> {
        const SYNCSAFE_BITS: u32 = 7;
        const SYNCSAFE_MASK: usize = 0x7f;
        let size: Vec<u8> = (0..4).rev().map(|byte| ((body >> (SYNCSAFE_BITS * byte)) & SYNCSAFE_MASK) as u8).collect();
        let mut tag = [&b"ID3\x03\x00\x00"[..], &size].concat();
        tag.resize(ID3V2_HEADER_SIZE + body, 0);
        tag
    }

    #[test]
    fn the_sync_window_ends_where_the_last_header_byte_must_be_read() {
        let last_start = MILES_MP3_SYNC_WINDOW - (MPEG_HEADER_SIZE - 1);
        for (zeros, found) in [(last_start, true), (last_start + 1, false), (MILES_MP3_SYNC_WINDOW, false), (MILES_MP3_SYNC_WINDOW + 1, false)] {
            let payload = [vec![0; zeros], stream()].concat();
            let expected = if found { MilesMp3Seek::Frame(zeros) } else { MilesMp3Seek::NotFound { window_start: 0 } };
            assert_eq!(miles_mp3_seek(&payload), expected, "{zeros:#x} zero bytes");
            assert_eq!(report(MP3_NAME, &payload).refused.is_empty(), found, "{zeros:#x} zero bytes");
        }
        // After a leading tag, the window starts at its end.
        let tagged = [id3v2(MILES_MP3_SYNC_WINDOW), vec![0; last_start], stream()].concat();
        assert!(report(MP3_NAME, &tagged).is_verified());
    }

    #[test]
    fn the_window_restarts_after_every_tag_miles_skips() {
        const TAG_BODY: usize = 0x3000;
        const GAP: usize = 0x1000;
        // Two tags: Miles skips the second as it scans, and counts the window from its end.
        let stacked = [id3v2(TAG_BODY), id3v2(TAG_BODY), vec![0; GAP], stream()].concat();
        let first = 2 * (ID3V2_HEADER_SIZE + TAG_BODY) + GAP;
        assert_eq!(miles_mp3_seek(&stacked), MilesMp3Seek::Frame(first));
        assert!(report(MP3_NAME, &stacked).is_verified(), "{:?}", report(MP3_NAME, &stacked));
        // An ID3v1 tag ahead of the stream restarts it too.
        let mut id3v1 = vec![0u8; ID3V1_SIZE];
        id3v1[..ID3V1_MAGIC.len()].copy_from_slice(ID3V1_MAGIC);
        let after_id3v1 = [vec![0; GAP], id3v1, vec![0; GAP], stream()].concat();
        assert_eq!(miles_mp3_seek(&after_id3v1), MilesMp3Seek::Frame(2 * GAP + ID3V1_SIZE));
    }

    #[test]
    fn a_false_sync_before_the_stream_is_refused() {
        const JUNK: [u8; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        // A lone Layer III header in the junk after the tag: our resync passes over it, Miles decodes from it.
        let false_sync = [id3v2(0), JUNK.to_vec(), mp3::tests::MPEG2_L3_64K_MONO_HEADER.to_vec(), JUNK.to_vec(), stream()].concat();
        let analysis = mp3::analyze_mpeg(&false_sync).unwrap();
        assert_eq!(analysis.first_frame_offset, ID3V2_HEADER_SIZE + 2 * JUNK.len() + MPEG_HEADER_SIZE);
        assert_eq!(analysis.miles_seek, MilesMp3Seek::Frame(ID3V2_HEADER_SIZE + JUNK.len()));
        let found = report(MP3_NAME, &false_sync);
        assert!(found.refused.iter().any(|problem| problem.contains("takes the bytes at")), "{found:?}");
        // A Layer II sync there is passed over: Miles locks onto the stream.
        const LAYER_2_HEADER: [u8; 4] = [0xff, 0xfd, 0x90, 0x40];
        let layer_2 = [id3v2(0), JUNK.to_vec(), LAYER_2_HEADER.to_vec(), JUNK.to_vec(), stream()].concat();
        assert!(report(MP3_NAME, &layer_2).refused.is_empty(), "{:?}", report(MP3_NAME, &layer_2));
    }

    #[test]
    fn refuses_what_miles_refuses_and_applies_the_policy() {
        let float = wav::tests::wav_with_format(wav::WAVE_FORMAT_IEEE_FLOAT, 32, &0.5f32.to_le_bytes());
        assert!(report(OTHER_WAV_NAME, &float).refused[0].contains("IEEE float"));
        let mismatch = report(MP3_NAME, &wav::encode_pcm_wav(22_050, 1, &[0, 0]));
        assert!(mismatch.refused[0].contains("the name asks for MP3, the file is WAV"), "{mismatch:?}");
        assert_eq!(enhanced_format_report("data/audio/a.ogg", &AudioContent::Ogg).refused.len(), 2);
        assert!(kind_mismatch("data/audio/a.ogg", &AudioContent::Ogg).is_some());

        let mut cut = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        cut.truncate(cut.len() - 1);
        let found = report(MP3_NAME, &cut);
        assert!(found.refused.is_empty() && found.unverified.iter().any(|problem| problem.contains("cut")), "{found:?}");
        let mut late = vec![0u8; MILES_MP3_SYNC_WINDOW + 1];
        late.extend(mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4));
        assert!(report(MP3_NAME, &late).refused.iter().any(|problem| problem.contains("scans")));
        let mut mixed = mp3::tests::frames(mp3::tests::MPEG2_L3_64K_MONO_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 4);
        // A channel mode change plays (mssmp3.asi does not compare it): a note.
        const MPEG2_L3_64K_JOINT_HEADER: [u8; 4] = [0xff, 0xf3, 0x80, 0x40];
        let mut joint = mixed.clone();
        joint.extend(mp3::tests::frames(MPEG2_L3_64K_JOINT_HEADER, mp3::tests::MPEG2_L3_64K_MONO_FRAME, 1));
        let found = report(MP3_NAME, &joint);
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("channel mode")), "{found:?}");
        mixed.extend(mp3::tests::frames(mp3::tests::MPEG1_L3_128K_HEADER, mp3::tests::MPEG1_L3_128K_FRAME, 1));
        assert!(report(MP3_NAME, &mixed).refused.iter().any(|problem| problem.contains("another version")));

        let refused = FormatReport { refused: vec!["x".to_owned()], ..FormatReport::default() };
        assert!(FormatPolicy::AllowUnverified.apply("x.wav", refused).unwrap_err().to_string().contains("Miles 7.2c"));
        let unverified = FormatReport { unverified: vec!["u".to_owned()], notes: vec!["n".to_owned()], ..FormatReport::default() };
        let error = FormatPolicy::VerifiedOnly.apply("x.wav", unverified.clone()).unwrap_err().to_string();
        assert!(error.contains("--allow-unverified-format"), "{error}");
        assert_eq!(FormatPolicy::AllowUnverified.apply("x.wav", unverified).unwrap(), ["u", "n"]);
        let notes = FormatReport { notes: vec!["n".to_owned()], ..FormatReport::default() };
        assert_eq!(FormatPolicy::VerifiedOnly.apply("x.wav", notes).unwrap(), ["n"]);
    }
}
