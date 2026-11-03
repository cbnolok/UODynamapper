//! Which audio the Enhanced Client plays: what its decoder, Miles Sound System 7.2c (`mss32.dll` and
//! `miles/mssmp3.asi`), accepts, checked before an entry is replaced or added ([`enhanced_format_report`]).
//!
//! The client hands the whole entry to `AIL_set_named_sample_file` with the entry name, and checks nothing itself.
//! Miles picks the decoder by the name's extension (a leading ID3v2 tag forces MP3), so a file of another kind than
//! its name ([`kind_mismatch`]) is refused under every policy. The rest of the rules come from the Miles code
//! (`scripts/dev-helpers/ec-research/ghidra-audio/results/miles_audio.md`); `docs/agent_notes/ec_audio_formats.md`
//! names the evidence of each:
//!
//! - **WAV** (the built-in parser): `WAVE` at +8; the chunk walk ([`miles_wav_walk`]) steps
//!   `size + (size & 1) + 8` from +12, each step must land below the RIFF size, and it must reach the `fmt ` and
//!   `data` chunks our walk finds (an odd chunk before them needs its pad byte); `fmt ` of 16 bytes or more (Miles
//!   reads the first 16); PCM of 8 or 16 bits, IMA ADPCM, or `WAVE_FORMAT_EXTENSIBLE` with `cbSize` ≥ 22, the PCM
//!   sub-format and 16 bits; mono or stereo; any rate; a `data` chunk inside the file (Miles takes its size as it
//!   stands, past the RIFF size too).
//! - **MP3** (`mssmp3.asi`): Layer III; the first frame header Miles' byte scan locks onto ([`miles_mp3_seek`]: after
//!   its exact skip of a leading ID3v2 tag, within 8 KiB after the last tag it skips) must be the stream's first
//!   frame, and its walk over the whole file ([`miles_mp3_frames`]) must decode the frames of the stream and no
//!   others; every frame of the first frame's version, layer and rate (the channel mode may change); any bitrate, VBR
//!   included; ID3v1 and ID3v2 tags.
//!
//! [`FormatReport`] sorts what differs: `refused` (Miles refuses or misreads it, or there is nothing to play),
//! `unverified` (the code read does not say, or Miles plays it wrongly: MPEG-2.5; refused unless the caller asks for
//! [`FormatPolicy::AllowUnverified`], for trying a file out in a test client, never for a release) and `notes`
//! (played, but unlike the shipped entries; always accepted, reported as warnings).

use super::mp3::{
    ChannelMode, FrameHeader, Id3v2Tag, MpegAnalysis, MpegLayer, MpegVersion, BITRATE_BAD, BITRATE_SHIFT, ID3V1_MAGIC, ID3V1_SIZE,
    ID3V2_HEADER_SIZE, ID3V2_MAGIC, LAYER_3_BITS, LAYER_SHIFT, MPEG_HEADER_SIZE, SAMPLE_RATE_RESERVED, SAMPLE_RATE_SHIFT, SYNC_BYTE,
    SYNC_MASK, TWO_BITS, VERSION_SHIFT,
};
use super::wav::{
    WavAnalysis, CHUNK_HEADER_SIZE, CHUNK_ID_SIZE, DATA_CHUNK_ID, FACT_CHUNK_ID, FMT_CHUNK_ID, KSDATAFORMAT_SUBTYPE_PCM,
    RIFF_HEADER_SIZE, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IMA_ADPCM, WAVE_FORMAT_MPEGLAYER3, WAVE_FORMAT_PCM,
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

/// What to do with what the Miles code read does not settle, and with MPEG-2.5, which Miles plays wrongly.
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
    /// Miles refuses or misreads it, or there is nothing to play: never stored.
    pub refused: Vec<String>,
    /// The Miles code read does not say how it plays, or Miles plays it wrongly (MPEG-2.5): for a test client only.
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
                "{what}: whether the Enhanced Client's decoder (Miles 7.2c) plays this as it should is not verified: {} (allow it with \
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

/// An ID3v2 tag Miles skips: both version bytes below [`MILES_ID3V2_VERSION_LIMIT`] and a syncsafe size. Nothing
/// else in the header counts: not the unsynchronisation, extended header or experimental flags.
fn miles_id3v2_tag(payload: &[u8], offset: usize) -> Option<Id3v2Tag> {
    Id3v2Tag::parse(payload, offset)
        .filter(|tag| tag.major < MILES_ID3V2_VERSION_LIMIT && tag.revision < MILES_ID3V2_VERSION_LIMIT)
}

/// `mssmp3.asi`'s frame seek (`SeekFrameHeader`), from the start of the stream and from frame to frame.
struct MilesScan<'a> {
    payload: &'a [u8],
    /// The next byte it reads.
    pos: usize,
    /// The version bits and sample rate index of the last Layer III header it saw.
    check: Option<(u8, u8)>,
}

impl<'a> MilesScan<'a> {
    /// `SetNamedSampleFile_Impl` first moves the start of the image past a leading ID3v2 tag
    /// ([`Id3v2Tag::miles_leading_skip`]). Past the end of the file there is nothing to read.
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, pos: miles_id3v2_tag(payload, 0).map_or(0, |tag| tag.miles_leading_skip()), check: None }
    }

    /// The next frame header, simulating the byte scan:
    ///
    /// - it reads one byte at a time while the byte is at most [`MILES_MP3_SYNC_WINDOW`] bytes past the window start
    ///   (a header starting `scanned` bytes in is found when `scanned + MPEG_HEADER_SIZE - 1 <= MILES_MP3_SYNC_WINDOW`);
    /// - when the last three bytes read are `TAG`, it skips the rest of the 128-byte ID3v1 tag and restarts the scan
    ///   and the window, even when the file ends first;
    /// - when they are `ID3`, it reads the 7 bytes left of the tag header: a valid tag is skipped, with 10 more bytes
    ///   when its footer flag is set, and the scan and the window restart; an invalid one is passed over in the same
    ///   window. A tag that runs past the file ends the stream;
    /// - when the last four bytes read are a frame sync (`0xff`, then three set bits), it takes them for a frame if
    ///   they are Layer III with a bitrate index other than 15 and a sample rate index other than 3; otherwise it
    ///   scans on. Nothing checks that the next frame follows;
    /// - after the first, a header whose version or sample rate differs from the last Layer III header's is passed
    ///   over and becomes what the next one is compared with.
    fn next_header(&mut self) -> MilesMp3Seek {
        const NOTHING_READ: [u8; MPEG_HEADER_SIZE] = [0; MPEG_HEADER_SIZE];
        let mut window_start = self.pos;
        // The last four bytes read, oldest first.
        let mut read = NOTHING_READ;
        loop {
            if self.pos - window_start > MILES_MP3_SYNC_WINDOW {
                return MilesMp3Seek::NotFound { window_start };
            }
            let Some(&byte) = self.payload.get(self.pos) else { return MilesMp3Seek::NotFound { window_start } };
            self.pos += 1;
            read = [read[1], read[2], read[3], byte];
            let last_three = &read[1..];
            if last_three == ID3V1_MAGIC {
                self.pos = self.pos - ID3V1_MAGIC.len() + ID3V1_SIZE;
                window_start = self.pos;
                read = NOTHING_READ;
            } else if last_three == ID3V2_MAGIC {
                let tag_offset = self.pos - ID3V2_MAGIC.len();
                match miles_id3v2_tag(self.payload, tag_offset) {
                    Some(tag) => {
                        self.pos = tag_offset + tag.length;
                        window_start = self.pos;
                        read = NOTHING_READ;
                    }
                    None if self.payload.len() < tag_offset + ID3V2_HEADER_SIZE => return MilesMp3Seek::NotFound { window_start },
                    None => self.pos = tag_offset + ID3V2_HEADER_SIZE,
                }
            } else if read[0] == SYNC_BYTE && read[1] & SYNC_MASK == SYNC_MASK {
                let layer_3 = (read[1] >> LAYER_SHIFT) & TWO_BITS == LAYER_3_BITS;
                let bitrate_known = read[2] >> BITRATE_SHIFT != BITRATE_BAD;
                let rate_index = (read[2] >> SAMPLE_RATE_SHIFT) & TWO_BITS;
                if layer_3 && bitrate_known && rate_index != SAMPLE_RATE_RESERVED {
                    let key = ((read[1] >> VERSION_SHIFT) & TWO_BITS, rate_index);
                    if self.check.replace(key).is_none_or(|last| last == key) {
                        return MilesMp3Seek::Frame(self.pos - MPEG_HEADER_SIZE);
                    }
                }
            }
        }
    }
}

/// The first frame header `mssmp3.asi` locks onto ([`MilesScan`]).
pub fn miles_mp3_seek(payload: &[u8]) -> MilesMp3Seek {
    MilesScan::new(payload).next_header()
}

/// The offsets of the frames `mssmp3.asi` decodes, in order: after each it scans on from the frame's end, computed as
/// ours is, or from the end of the side information when that lies further. A header whose side information is cut
/// ends the stream without a frame. Free-format and reserved-version headers it takes too, but no frame length of
/// ours follows from them: the walk ends with such a frame.
pub fn miles_mp3_frames(payload: &[u8]) -> Vec<usize> {
    let mut scan = MilesScan::new(payload);
    let mut frames = Vec::new();
    while let MilesMp3Seek::Frame(offset) = scan.next_header() {
        let Some(header) = FrameHeader::parse(&payload[offset..]) else {
            frames.push(offset);
            break;
        };
        if offset + header.xing_offset() > payload.len() {
            break;
        }
        frames.push(offset);
        scan.pos = offset + header.length.max(header.xing_offset());
    }
    frames
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
            "{} frames of another version, layer or sample rate than the first (mssmp3.asi passes over a frame unlike the last, and goes on at the new values while the playback rate stays the first's)",
            analysis.inconsistent_rate_frames
        ));
    }
    if analysis.miles_seek == MilesMp3Seek::Frame(analysis.first_frame_offset) && analysis.inconsistent_rate_frames == 0 {
        if let Some(offset) = analysis.miles_stray_frame {
            report.refused.push(format!("mssmp3.asi takes the bytes at {offset}, which are no frame of the stream, for a frame (it would decode them)"));
        }
        if let Some(offset) = analysis.miles_missed_frame {
            report.refused.push(format!(
                "mssmp3.asi never decodes the frame at {offset} (more than {MILES_MP3_SYNC_WINDOW} bytes without a frame end the stream, and a stray header of another rate makes it pass over the next frame)"
            ));
        }
    }
    if first.layer == MpegLayer::L3 && first.version == MpegVersion::V2_5 {
        report.unverified.push(format!(
            "MPEG-2.5 ({} Hz): mssmp3.asi decodes it with the band tables of MPEG-2, so expect artefacts; no shipped entry is MPEG-2.5",
            first.sample_rate
        ));
    }
    let channel_mode_changes = analysis.inconsistent_frames - analysis.inconsistent_rate_frames;
    let notes = [
        (analysis.resyncs > 0, format!(
            "{} breaks in the frame chain, {} bytes in all (mssmp3.asi scans over them; the frame after one may come out silent)",
            analysis.resyncs, analysis.resync_bytes
        )),
        (analysis.truncated_last_frame, "the last frame is cut (mssmp3.asi decodes it, partly from stale bytes: a short glitch is possible)".to_owned()),
        (analysis.tail.trailing_bytes > 0, format!("{} bytes that are no tag after the last frame (mssmp3.asi scans over them)", analysis.tail.trailing_bytes)),
        (analysis.tail.ape, "an APE tag (mssmp3.asi skips ID3 tags only and scans over it)".to_owned()),
    ];
    report.notes.extend(notes.into_iter().filter(|(found, _)| *found).map(|(_, note)| note));
    if first.layer == MpegLayer::L3 && first.version != MpegVersion::V2_5 && !SHIPPED_MPEG_STREAMS.contains(&(first.version, first.sample_rate, first.channel_mode)) {
        report.notes.push(format!("{} is not among the shipped streams", analysis.stream_label()));
    }
    if channel_mode_changes > 0 {
        // mssmp3.asi compares version, layer and sample rate only; no shipped stream changes its channel mode.
        report.notes.push(format!("{channel_mode_changes} frames of another channel mode than the first (no shipped stream has one)"));
    }
}

/// How one search of Miles' chunk walk ends ([`miles_wav_walk`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilesChunkSearch {
    /// The offset of the chunk it takes.
    Found(usize),
    /// The next chunk would start at or past the RIFF size.
    RiffEnd,
    /// The chunk header at this offset is not all inside the file, and Miles reads it all the same.
    PastFile(u32),
    /// The size of the chunk at this offset steps back or in place: the walk reads outside the file or never ends.
    Stuck(usize),
}

/// What `WavInfo_Impl` finds, walking the file for each chunk it searches, from the start each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilesWavWalk {
    pub fmt: MilesChunkSearch,
    pub data: MilesChunkSearch,
    /// Searched only for IMA ADPCM. Not finding it within the RIFF size is fine.
    pub fact: MilesChunkSearch,
}

/// One search of `WavInfo_Impl`'s walk. It looks at the chunk at +12 before any bound, compares ids ignoring case,
/// steps `size + (size & 1) + 8` in 32 bits, and stops when the step lands at or past the RIFF size, as a signed
/// 32-bit difference of the two (so a RIFF size more than 2 GiB past the step stops it too). It never looks at the
/// length of the file, which it is not given.
fn miles_find_chunk(payload: &[u8], riff_size: u32, id: [u8; CHUNK_ID_SIZE]) -> MilesChunkSearch {
    let mut offset = RIFF_HEADER_SIZE as u32;
    loop {
        let Some(header) = payload.get(offset as usize..).and_then(|rest| rest.first_chunk::<CHUNK_HEADER_SIZE>()) else {
            return MilesChunkSearch::PastFile(offset);
        };
        if header[..CHUNK_ID_SIZE].eq_ignore_ascii_case(&id) {
            return MilesChunkSearch::Found(offset as usize);
        }
        let size = u32::from_le_bytes(header[CHUNK_ID_SIZE..].try_into().expect("four bytes"));
        let next = offset.wrapping_add(size).wrapping_add(size & 1).wrapping_add(CHUNK_HEADER_SIZE as u32);
        if next.wrapping_sub(riff_size) as i32 >= 0 {
            return MilesChunkSearch::RiffEnd;
        }
        if next <= offset {
            return MilesChunkSearch::Stuck(offset as usize);
        }
        offset = next;
    }
}

/// Miles' chunk walk over a RIFF payload whose size field is `riff_size`.
pub fn miles_wav_walk(payload: &[u8], riff_size: u32) -> MilesWavWalk {
    MilesWavWalk {
        fmt: miles_find_chunk(payload, riff_size, FMT_CHUNK_ID),
        data: miles_find_chunk(payload, riff_size, DATA_CHUNK_ID),
        fact: miles_find_chunk(payload, riff_size, FACT_CHUNK_ID),
    }
}

/// Why Miles' walk does not take the `fmt ` and `data` chunks ours finds, and for IMA ADPCM why its search for `fact`
/// does not end. An unpadded odd chunk sends Miles to the byte after it, so it matters only before `fmt ` or `data`.
fn miles_walk_problems(analysis: &WavAnalysis, ima_adpcm: bool) -> Vec<String> {
    let walk = analysis.miles_walk;
    let mut problems = Vec::new();
    for (id, label, search) in [(FMT_CHUNK_ID, "fmt ", walk.fmt), (DATA_CHUNK_ID, "data", walk.data)] {
        let Some(ours) = analysis.chunks.iter().find(|chunk| chunk.id == id).map(|chunk| chunk.offset) else { continue };
        if search == MilesChunkSearch::Found(ours) {
            continue;
        }
        let why = if analysis.chunks.iter().any(|chunk| chunk.unpadded && chunk.offset < ours) {
            "an odd chunk before it has no pad byte".to_owned()
        } else {
            match search {
                MilesChunkSearch::Found(other) => format!("it takes the bytes at {other} for it"),
                MilesChunkSearch::RiffEnd => format!("the RIFF size, {}, ends before it", analysis.riff_size),
                MilesChunkSearch::PastFile(offset) => format!("it runs past the end of the file, to {offset}"),
                MilesChunkSearch::Stuck(offset) => format!("the size of the chunk at {offset} steps back or in place"),
            }
        };
        problems.push(format!("Miles' chunk walk misses the `{label}` chunk at {ours}: {why}"));
    }
    match walk.fact {
        MilesChunkSearch::PastFile(offset) if ima_adpcm => {
            problems.push(format!("Miles' search for an IMA ADPCM `fact` chunk runs past the end of the file, to {offset}"));
        }
        MilesChunkSearch::Stuck(offset) if ima_adpcm => {
            problems.push(format!("Miles' search for an IMA ADPCM `fact` chunk never ends: the size of the chunk at {offset} steps back or in place"));
        }
        _ => {}
    }
    problems
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
    if tag == WAVE_FORMAT_MPEGLAYER3 {
        report.refused.push(format!("{}: mssmp3.asi would take its tag, but MP3 data is stored as an .mp3 entry, never in a WAV", format.label()));
    } else if !playable && tag != WAVE_FORMAT_EXTENSIBLE {
        report.refused.push(format!("{} (Miles plays PCM of 8 or 16 bits and IMA ADPCM)", format.label()));
    }
    if format.channels == 0 || format.channels > MILES_MAX_CHANNELS {
        report.refused.push(format!("{} channels (mono or stereo only)", format.channels));
    }
    if format.sample_rate == 0 {
        report.refused.push("a sample rate of 0".to_owned());
    }
    if tag == WAVE_FORMAT_IMA_ADPCM && format.block_align == 0 {
        report.refused.push("IMA ADPCM with a block align of 0 (without a `fact` chunk Miles divides by it)".to_owned());
    }
    match analysis.data_offset {
        None => report.refused.push("no data chunk".to_owned()),
        Some(_) if analysis.data_truncated => report.refused.push("the data chunk runs past the file (Miles does not clamp it)".to_owned()),
        Some(_) if analysis.data_len == 0 => report.refused.push("no samples".to_owned()),
        Some(_) => {}
    }
    report.refused.extend(miles_walk_problems(analysis, tag == WAVE_FORMAT_IMA_ADPCM && bits == MILES_IMA_ADPCM_BITS));
    // What Miles mixes per sample frame: it reads neither the byte rate nor, for PCM, the block align.
    let frame_bytes = usize::from(format.channels) * usize::from(bits.div_ceil(BITS_PER_BYTE));
    let pcm = format.effective_tag() == WAVE_FORMAT_PCM;
    if pcm && frame_bytes > 0 && analysis.data_len % frame_bytes != 0 {
        report.unverified.push("the data is not whole sample frames (Miles' mixer would read the last one past the data)".to_owned());
    }
    let riff_end = analysis.riff_size as usize + CHUNK_HEADER_SIZE;
    let data_end = analysis.data_offset.map_or(0, |offset| offset + analysis.data_len);
    let notes = [
        (pcm && !format.is_consistent(), "byte rate or block align do not follow from the format (Miles reads neither for PCM)".to_owned()),
        (riff_end > analysis.length, format!(
            "the RIFF size runs {} bytes past the end of the file (Miles only bounds its chunk walk with it)",
            riff_end.saturating_sub(analysis.length)
        )),
        (riff_end < analysis.length, format!("{} bytes after the end the RIFF size gives", analysis.length.saturating_sub(riff_end))),
        (data_end > riff_end, format!(
            "the data chunk runs {} bytes past the RIFF size (Miles takes the chunk's own size and plays them)",
            data_end.saturating_sub(riff_end)
        )),
        (analysis.trailing_bytes > 0 && analysis.length - analysis.trailing_bytes < riff_end, format!("{} bytes after the last chunk", analysis.trailing_bytes)),
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

    const JUNK: [u8; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    /// The ID3v2 header's flags byte.
    const ID3V2_FLAGS_AT: usize = 5;
    const ID3V2_UNSYNCHRONISED: u8 = 0x80;
    const ID3V2_EXTENDED_HEADER: u8 = 0x40;
    const ID3V2_FOOTER: u8 = 0x10;
    const ID3V2_FOOTER_LEN: usize = 10;
    /// The size whose lowest size byte is 118 and next size byte 1: Miles' sum carries, and the carry is lost.
    const CARRY_LOST_BODY: usize = (1 << 7) | 118;
    const CARRY_LOST_SHORTFALL: usize = 128;

    fn tag_with_flags(body: usize, flags: u8) -> Vec<u8> {
        let mut tag = id3v2(body);
        tag[ID3V2_FLAGS_AT] = flags;
        tag
    }

    #[test]
    fn the_leading_skip_is_the_one_miles_computes() {
        let skip = |body: usize| Id3v2Tag::parse(&id3v2(body), 0).unwrap().miles_leading_skip();
        assert_eq!(skip(0), ID3V2_HEADER_SIZE);
        assert_eq!(skip(117), ID3V2_HEADER_SIZE + 117, "no carry");
        assert_eq!(skip(118), ID3V2_HEADER_SIZE + 118, "a carry, and the next size byte is even: the OR keeps it");
        assert_eq!(skip((2 << 7) | 118), ID3V2_HEADER_SIZE + ((2 << 7) | 118), "the same");
        assert_eq!(skip(CARRY_LOST_BODY), ID3V2_HEADER_SIZE + CARRY_LOST_BODY - CARRY_LOST_SHORTFALL, "the carry is lost");
        assert_eq!(skip((3 << 7) | 127), ID3V2_HEADER_SIZE + ((3 << 7) | 127) - CARRY_LOST_SHORTFALL, "the same");
        let footer = tag_with_flags(CARRY_LOST_BODY, ID3V2_FOOTER);
        assert_eq!(Id3v2Tag::parse(&footer, 0).unwrap().miles_leading_skip(), skip(CARRY_LOST_BODY), "the footer flag is not read");
    }

    #[test]
    fn a_short_leading_skip_shows_miles_the_end_of_the_tag() {
        let tag_len = ID3V2_HEADER_SIZE + CARRY_LOST_BODY;
        // Zeros in the bytes the skip leaves, as in 157 shipped files: the same first frame.
        let padded = [id3v2(CARRY_LOST_BODY), stream()].concat();
        assert_eq!(miles_mp3_seek(&padded), MilesMp3Seek::Frame(tag_len));
        assert_eq!(report(MP3_NAME, &padded), FormatReport::default());

        // A frame header there, which a generic parse skips with the tag: Miles decodes from it.
        let mut in_tag = padded.clone();
        let at = tag_len - CARRY_LOST_SHORTFALL + JUNK.len();
        in_tag[at..at + MPEG_HEADER_SIZE].copy_from_slice(&mp3::tests::MPEG2_L3_64K_MONO_HEADER);
        assert_eq!(miles_mp3_seek(&in_tag), MilesMp3Seek::Frame(at));
        assert_eq!(mp3::analyze_mpeg(&in_tag).unwrap().first_frame_offset, tag_len);
        let found = report(MP3_NAME, &in_tag);
        assert!(found.refused.iter().any(|problem| problem.contains("takes the bytes at")), "{found:?}");

        // `TAG` at the very end of the tag: Miles skips 125 bytes into the first frame and locks onto the second.
        let mut id3v1_in_tag = padded.clone();
        id3v1_in_tag[tag_len - ID3V1_MAGIC.len()..tag_len].copy_from_slice(ID3V1_MAGIC);
        assert_eq!(miles_mp3_seek(&id3v1_in_tag), MilesMp3Seek::Frame(tag_len + mp3::tests::MPEG2_L3_64K_MONO_FRAME));
        assert!(!report(MP3_NAME, &id3v1_in_tag).refused.is_empty());
    }

    #[test]
    fn id3_flags_and_repeated_tags_as_miles_reads_them() {
        const TAG_BODY: usize = 64;
        let first = |tag: &[u8]| tag.len();
        // The unsynchronisation and extended header flags change nothing: the size field is all it reads.
        for flags in [ID3V2_UNSYNCHRONISED, ID3V2_EXTENDED_HEADER] {
            let tag = tag_with_flags(TAG_BODY, flags);
            let file = [tag.clone(), stream()].concat();
            assert_eq!(miles_mp3_seek(&file), MilesMp3Seek::Frame(first(&tag)));
            assert!(report(MP3_NAME, &file).is_verified());
        }
        // A leading footer is not skipped: its 10 bytes are scanned, and hold no sync. They count in the window.
        let mut footer = tag_with_flags(TAG_BODY, ID3V2_FOOTER);
        footer.resize(footer.len() + ID3V2_FOOTER_LEN, 0);
        let file = [footer.clone(), stream()].concat();
        assert_eq!(miles_mp3_seek(&file), MilesMp3Seek::Frame(first(&footer)));
        let last_start = MILES_MP3_SYNC_WINDOW - (MPEG_HEADER_SIZE - 1);
        let in_window = [footer.clone(), vec![0; last_start - ID3V2_FOOTER_LEN], stream()].concat();
        assert!(report(MP3_NAME, &in_window).refused.is_empty());
        let past_window = [footer.clone(), vec![0; last_start - ID3V2_FOOTER_LEN + 1], stream()].concat();
        assert!(report(MP3_NAME, &past_window).refused.iter().any(|problem| problem.contains("scans")));
        // A later tag's footer is skipped, so its window starts after it.
        let later = [id3v2(TAG_BODY), footer.clone(), vec![0; last_start], stream()].concat();
        assert!(report(MP3_NAME, &later).is_verified(), "{:?}", report(MP3_NAME, &later));
        // Repeated tags after a short leading skip: it finds the second in the bytes it scans.
        let repeated = [id3v2(CARRY_LOST_BODY), id3v2(TAG_BODY), id3v2(CARRY_LOST_BODY), stream()].concat();
        assert_eq!(miles_mp3_seek(&repeated), MilesMp3Seek::Frame(repeated.len() - stream().len()));
        assert!(report(MP3_NAME, &repeated).is_verified());
    }

    #[test]
    fn id3_headers_miles_does_not_take_and_sizes_past_the_file() {
        const LONG_BODY: usize = 0x3000;
        const VERSION_AT: usize = 3;
        const SIZE_AT: usize = 6;
        // Version byte 0xff: our parse takes the tag, Miles does not, and scans its body within one window.
        let mut version = id3v2(LONG_BODY);
        version[VERSION_AT] = MILES_ID3V2_VERSION_LIMIT;
        let file = [version, stream()].concat();
        assert_eq!(miles_mp3_seek(&file), MilesMp3Seek::NotFound { window_start: 0 });
        assert!(report(MP3_NAME, &file).refused.iter().any(|problem| problem.contains("scans")));
        // A size that is not syncsafe: the header is passed over and the scan goes on after it.
        let mut not_syncsafe = id3v2(0);
        not_syncsafe[SIZE_AT] = 0x80;
        let file = [not_syncsafe.clone(), stream()].concat();
        assert_eq!(miles_mp3_seek(&file), MilesMp3Seek::Frame(not_syncsafe.len()));
        // A leading tag past the end of the file: Miles' pointer leaves the file; there is no stream for us either.
        let mut past = id3v2(0);
        past[SIZE_AT + 1] = 1;
        let file = [past, stream()].concat();
        assert!(matches!(miles_mp3_seek(&file), MilesMp3Seek::NotFound { .. }));
        assert!(!report(MP3_NAME, &file).refused.is_empty());
        // A later tag past the end ends the stream at once.
        let second = [id3v2(0), vec![b'I', b'D', b'3', 3, 0, 0, 0, 1, 0, 0]].concat();
        assert!(matches!(miles_mp3_seek(&second), MilesMp3Seek::NotFound { .. }));
    }

    #[test]
    fn the_whole_walk_settles_junk_tails_and_breaks() {
        // Junk between frames and after the last, without a sync: scanned over.
        let broken = [stream(), JUNK.to_vec(), stream()].concat();
        let found = report(MP3_NAME, &broken);
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("breaks in the frame chain")), "{found:?}");
        let trailing = [stream(), JUNK.to_vec()].concat();
        let found = report(MP3_NAME, &trailing);
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("no tag")), "{found:?}");
        // An APE tag: junk to Miles.
        const APE_FOOTER_LEN: usize = 32;
        const APE_SIZE_AT: usize = 12;
        let mut ape = vec![0u8; APE_FOOTER_LEN];
        ape[..8].copy_from_slice(b"APETAGEX");
        ape[APE_SIZE_AT] = APE_FOOTER_LEN as u8;
        let found = report(MP3_NAME, &[stream(), ape].concat());
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("APE")), "{found:?}");
        // An ID3v1 tag holding a frame header is skipped whole.
        let mut id3v1 = vec![0u8; ID3V1_SIZE];
        id3v1[..ID3V1_MAGIC.len()].copy_from_slice(ID3V1_MAGIC);
        id3v1[JUNK.len()..JUNK.len() + MPEG_HEADER_SIZE].copy_from_slice(&mp3::tests::MPEG2_L3_64K_MONO_HEADER);
        assert_eq!(report(MP3_NAME, &[stream(), id3v1].concat()), FormatReport::default());

        // A lone Layer III header in the tail, far enough from the end to hold its side information: decoded.
        let stray_at = stream().len() + JUNK.len();
        let stray = [stream(), JUNK.to_vec(), mp3::tests::MPEG2_L3_64K_MONO_HEADER.to_vec(), vec![1; 64]].concat();
        let analysis = mp3::analyze_mpeg(&stray).unwrap();
        assert_eq!(analysis.miles_stray_frame, Some(stray_at));
        let found = report(MP3_NAME, &stray);
        assert!(found.refused.iter().any(|problem| problem.contains("no frame of the stream")), "{found:?}");
        // The same header with its side information cut ends the stream without a frame.
        let cut_header = [stream(), JUNK.to_vec(), mp3::tests::MPEG2_L3_64K_MONO_HEADER.to_vec()].concat();
        assert_eq!(mp3::analyze_mpeg(&cut_header).unwrap().miles_stray_frame, None);

        // A free-format header there is taken too, and no length of ours follows from it.
        const MPEG2_L3_FREE_MONO_HEADER: [u8; 4] = [0xff, 0xf3, 0x00, 0xc0];
        let free = [stream(), JUNK.to_vec(), MPEG2_L3_FREE_MONO_HEADER.to_vec(), vec![1; 64]].concat();
        assert_eq!(mp3::analyze_mpeg(&free).unwrap().miles_stray_frame, Some(stray_at));

        // More junk than the window: the stream ends there and the rest never plays.
        let gap = [stream(), vec![1; MILES_MP3_SYNC_WINDOW + 1], stream()].concat();
        let analysis = mp3::analyze_mpeg(&gap).unwrap();
        assert_eq!(analysis.resyncs, 1);
        assert_eq!(analysis.miles_missed_frame, Some(stream().len() + MILES_MP3_SYNC_WINDOW + 1));
        let found = report(MP3_NAME, &gap);
        assert!(found.refused.iter().any(|problem| problem.contains("never decodes")), "{found:?}");

        // A header of another rate in the junk: passed over, and the next real frame with it.
        const MPEG2_L3_24K_HZ_HEADER: [u8; 4] = [0xff, 0xf3, 0x84, 0xc0];
        let other_rate = [stream(), JUNK.to_vec(), MPEG2_L3_24K_HZ_HEADER.to_vec(), JUNK.to_vec(), stream()].concat();
        let found = report(MP3_NAME, &other_rate);
        assert!(found.refused.iter().any(|problem| problem.contains("never decodes")), "{found:?}");
    }

    #[test]
    fn mpeg_2_5_is_for_a_test_client_only() {
        /// MPEG-2.5 Layer III, 8 kbit/s, 8000 Hz, mono: a 72-byte frame.
        const MPEG25_L3_8K_MONO_HEADER: [u8; 4] = [0xff, 0xe3, 0x18, 0xc0];
        const MPEG25_L3_8K_MONO_FRAME: usize = 72;
        let file = mp3::tests::frames(MPEG25_L3_8K_MONO_HEADER, MPEG25_L3_8K_MONO_FRAME, 4);
        let found = report(MP3_NAME, &file);
        assert!(found.refused.is_empty() && found.notes.is_empty(), "{found:?}");
        assert_eq!(found.unverified.len(), 1);
        assert!(found.unverified[0].contains("MPEG-2.5 (8000 Hz)"), "{found:?}");
        assert!(FormatPolicy::VerifiedOnly.apply("x.mp3", found.clone()).unwrap_err().to_string().contains("--allow-unverified-format"));
        assert_eq!(FormatPolicy::AllowUnverified.apply("x.mp3", found.clone()).unwrap(), found.unverified);
    }

    /// `fmt ` (16 bytes) then `data`: where `data` starts.
    const CANONICAL_DATA_AT: usize = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + wav::FMT_PCM_SIZE as usize;
    const RIFF_SIZE_AT: usize = CHUNK_HEADER_SIZE / 2;

    fn with_riff_size(mut file: Vec<u8>, size: u32) -> Vec<u8> {
        file[RIFF_SIZE_AT..RIFF_SIZE_AT + size_of::<u32>()].copy_from_slice(&size.to_le_bytes());
        file
    }

    #[test]
    fn the_riff_size_bounds_the_walk_as_in_miles() {
        const SAMPLES: [u8; 4] = [0; 4];
        let canonical = || wav::encode_pcm_wav(48_000, 1, &SAMPLES);
        // A `data` chunk starting at the RIFF size, or past it: the step lands at the bound and the walk stops.
        for size in [CANONICAL_DATA_AT as u32, CANONICAL_DATA_AT as u32 - 1] {
            let found = report(OTHER_WAV_NAME, &with_riff_size(canonical(), size));
            assert!(found.refused.iter().any(|problem| problem.contains("the RIFF size") && problem.contains("`data`")), "{size}: {found:?}");
        }
        // One byte more and it is found, though its body lies past the RIFF size: Miles plays all of it.
        let beyond = with_riff_size(canonical(), CANONICAL_DATA_AT as u32 + 1);
        let found = report(OTHER_WAV_NAME, &beyond);
        assert!(found.is_verified(), "{found:?}");
        assert!(found.notes.iter().any(|note| note.contains("data chunk runs")), "{found:?}");
        assert_eq!(wav::analyze_wav(&beyond).unwrap().miles_walk.data, MilesChunkSearch::Found(CANONICAL_DATA_AT));
        // A size past the end of the file: the walk stops at `data`, so it plays; reported.
        let long = with_riff_size(canonical(), canonical().len() as u32);
        let found = report(OTHER_WAV_NAME, &long);
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("past the end of the file")), "{found:?}");
        // A `data` chunk past the end of the file is refused, RIFF size or not.
        let mut cut = canonical();
        cut.truncate(cut.len() - 1);
        assert!(report(OTHER_WAV_NAME, &cut).refused.iter().any(|problem| problem.contains("runs past the file")));
        // Bytes past the RIFF size: Miles never looks at them.
        let extra = [canonical(), JUNK.to_vec()].concat();
        let found = report(OTHER_WAV_NAME, &extra);
        assert!(found.is_verified() && found.notes.len() == 1 && found.notes[0].contains("after the end the RIFF size gives"), "{found:?}");
        // A chunk past the RIFF size that is not `fmt ` or `data` is ignored.
        let mut listed = canonical();
        listed.extend_from_slice(b"LIST\0\0\0\0");
        assert!(report(OTHER_WAV_NAME, &listed).is_verified());

        // The signed difference: a RIFF size more than 2 GiB past the step stops the walk at once.
        const FAR_RIFF_SIZE: u32 = 0xffff_fff0;
        const NEAR_HALF_RIFF_SIZE: u32 = 0x8000_0000;
        let file = canonical();
        assert_eq!(miles_wav_walk(&file, FAR_RIFF_SIZE).data, MilesChunkSearch::RiffEnd);
        assert_eq!(miles_wav_walk(&file, NEAR_HALF_RIFF_SIZE).data, MilesChunkSearch::Found(CANONICAL_DATA_AT));
        let far = report(OTHER_WAV_NAME, &with_riff_size(canonical(), FAR_RIFF_SIZE));
        assert!(far.refused.iter().any(|problem| problem.contains("the RIFF size")), "{far:?}");
    }

    #[test]
    fn the_walk_runs_off_the_file_or_never_ends() {
        const SAMPLES: [u8; 2] = [0; 2];
        const FMT_FIELDS: usize = wav::FMT_PCM_SIZE as usize;
        let canonical = wav::encode_pcm_wav(22_050, 1, &SAMPLES);
        // Ids compare ignoring case.
        assert_eq!(miles_find_chunk(&canonical, canonical.len() as u32, *b"DATA"), MilesChunkSearch::Found(CANONICAL_DATA_AT));
        // The RIFF size leaves room, but the file ends before a chunk header: Miles reads on.
        let no_such_chunk = *b"none";
        let size = (canonical.len() + ID3V1_SIZE) as u32;
        assert_eq!(miles_find_chunk(&canonical, size, no_such_chunk), MilesChunkSearch::PastFile(canonical.len() as u32));
        // A size that wraps the step to nothing, and one that steps back.
        let chunk_with_size = |size: u32| {
            let mut file = canonical[..RIFF_HEADER_SIZE].to_vec();
            file.extend_from_slice(b"JUNK");
            file.extend_from_slice(&size.to_le_bytes());
            file.extend_from_slice(&canonical[RIFF_HEADER_SIZE..]);
            with_riff_size(file, canonical.len() as u32)
        };
        const STEP_ZERO: u32 = 0u32.wrapping_sub(CHUNK_HEADER_SIZE as u32);
        const STEP_BACK: u32 = STEP_ZERO - 2;
        for size in [STEP_ZERO, STEP_BACK] {
            let file = chunk_with_size(size);
            assert_eq!(miles_find_chunk(&file, file.len() as u32, DATA_CHUNK_ID), MilesChunkSearch::Stuck(RIFF_HEADER_SIZE));
        }

        // IMA ADPCM also searches `fact`: over the whole RIFF size when there is none.
        let ima = wav::tests::wav_with_format(WAVE_FORMAT_IMA_ADPCM, MILES_IMA_ADPCM_BITS, &[0; 4]);
        assert_eq!(wav::analyze_wav(&ima).unwrap().miles_walk.fact, MilesChunkSearch::RiffEnd);
        assert!(report(OTHER_WAV_NAME, &ima).refused.is_empty());
        let long = with_riff_size(ima.clone(), ima.len() as u32 + FMT_FIELDS as u32);
        let found = report(OTHER_WAV_NAME, &long);
        assert!(found.refused.iter().any(|problem| problem.contains("`fact` chunk runs past")), "{found:?}");
        // A block align of 0: Miles divides by it to count the samples.
        const BLOCK_ALIGN_AT: usize = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + 12;
        let mut no_block = ima.clone();
        no_block[BLOCK_ALIGN_AT..BLOCK_ALIGN_AT + size_of::<u16>()].fill(0);
        assert!(report(OTHER_WAV_NAME, &no_block).refused.iter().any(|problem| problem.contains("block align of 0")));
        // The same RIFF size on PCM never searches `fact`.
        let pcm = wav::encode_pcm_wav(22_050, 1, &SAMPLES);
        let long = with_riff_size(pcm.clone(), pcm.len() as u32 + FMT_FIELDS as u32);
        assert!(report(OTHER_WAV_NAME, &long).refused.is_empty());
    }

    #[test]
    fn what_moved_out_of_unverified_for_wav() {
        const BLOCK_ALIGN_AT: usize = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + 12;
        const BYTE_RATE_AT: usize = RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + 8;
        // Miles reads neither field for PCM: a note.
        let mut odd_fields = wav::encode_pcm_wav(22_050, 1, &[0; 4]);
        odd_fields[BYTE_RATE_AT] ^= 1;
        odd_fields[BLOCK_ALIGN_AT] = 3;
        let found = report(OTHER_WAV_NAME, &odd_fields);
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("byte rate")), "{found:?}");
        // Whole frames are counted from the channels and bits, as the mixer reads, not from the block align.
        let mut partial = wav::encode_pcm_wav(22_050, 1, &[0; 3]);
        partial[BLOCK_ALIGN_AT] = 1;
        let found = report(OTHER_WAV_NAME, &partial);
        assert!(found.unverified.iter().any(|problem| problem.contains("whole sample frames")), "{found:?}");
        // MP3 data in a WAV: mssmp3.asi would take the tag; never stored so here.
        let mp3_in_wav = wav::tests::wav_with_format(wav::WAVE_FORMAT_MPEGLAYER3, 0, &[0; 4]);
        assert!(report(OTHER_WAV_NAME, &mp3_in_wav).refused.iter().any(|problem| problem.contains("mssmp3.asi would take its tag")));
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
        assert!(found.is_verified() && found.notes.iter().any(|note| note.contains("cut")), "plays, with a possible glitch: {found:?}");
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
