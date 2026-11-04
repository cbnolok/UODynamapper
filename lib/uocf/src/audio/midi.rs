//! Standard MIDI files, by what they declare: the header, each track's name, events, notes and programs, and the
//! length the tempo events give. Nothing here plays them.
//!
//! The Classic clients name `.mid` tracks in `music\` as a fallback when there is no digital music, but none of the
//! reference clients ships one (`docs/agent_notes/ec_audio_formats.md`), so this reads the SMF format itself, not a
//! client's use of it.
//!
//! ```text
//! file  = "MThd" u32be(6..) u16be(format) u16be(tracks) u16be(division) *track
//! track = "MTrk" u32be(size) *(vlq(delta) event)
//! event = meta | sysex | voice                ; a voice event may leave out its status (running status)
//! meta  = 0xFF type vlq(size) data
//! sysex = (0xF0 | 0xF7) vlq(size) data
//! ```

use std::collections::BTreeSet;

crate::eyre_imports!();

pub const HEADER_MAGIC: &[u8; 4] = b"MThd";
pub const TRACK_MAGIC: &[u8; 4] = b"MTrk";
const CHUNK_HEADER_BYTES: usize = 8;
const HEADER_MIN_BYTES: usize = 6;
/// The bit of the division field that marks SMPTE timing.
const DIVISION_SMPTE_BIT: u16 = 0x8000;
const VLQ_MAX_BYTES: usize = 4;
const VLQ_CONTINUE_BIT: u8 = 0x80;
const VLQ_VALUE_MASK: u8 = 0x7f;
/// Below this a byte is a data byte, so the event reuses the last status.
const STATUS_BIT: u8 = 0x80;
const META_EVENT: u8 = 0xff;
const SYSEX_EVENT: u8 = 0xf0;
const SYSEX_ESCAPE_EVENT: u8 = 0xf7;
const META_TRACK_NAME: u8 = 0x03;
const META_END_OF_TRACK: u8 = 0x2f;
const META_SET_TEMPO: u8 = 0x51;
const SET_TEMPO_BYTES: usize = 3;
const VOICE_KIND_MASK: u8 = 0xf0;
const NOTE_ON: u8 = 0x90;
const PROGRAM_CHANGE: u8 = 0xc0;
const CHANNEL_PRESSURE: u8 = 0xd0;
/// The tempo until the first Set Tempo event: 120 beats per minute.
const DEFAULT_MICROSECONDS_PER_QUARTER: u32 = 500_000;
const MICROSECONDS_PER_SECOND: f64 = 1_000_000.0;
const MICROSECONDS_PER_MINUTE: f64 = 60_000_000.0;
const FORMAT_INDEPENDENT_SEQUENCES: u16 = 2;

/// The header's division field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiDivision {
    TicksPerQuarter(u16),
    Smpte { frames_per_second: u8, ticks_per_frame: u8 },
}

/// One track.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MidiTrack {
    pub name: Option<String>,
    pub events: u32,
    /// Note On events of a velocity above zero.
    pub notes: u32,
    /// The programs (General MIDI instruments, 0-based) of its Program Change events.
    pub programs: BTreeSet<u8>,
    /// The tick of its last event.
    pub ticks: u64,
}

/// What a file declares.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiAnalysis {
    pub format: u16,
    pub division: MidiDivision,
    pub tracks: Vec<MidiTrack>,
    /// The length by the tempo events. `None` for format 2 (independent sequences have no common length).
    pub duration_seconds: Option<f64>,
    /// Beats per minute of the first tempo in the file (120 without one).
    pub first_tempo_bpm: f64,
}

impl MidiAnalysis {
    pub fn notes(&self) -> u64 {
        self.tracks.iter().map(|track| u64::from(track.notes)).sum()
    }
}

/// `Some` when the payload starts as a Standard MIDI file.
pub fn is_midi(payload: &[u8]) -> bool {
    payload.starts_with(HEADER_MAGIC)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> eyre::Result<&'a [u8]> {
        let end = self.at.checked_add(count).filter(|&end| end <= self.bytes.len());
        let Some(end) = end else {
            eyre::bail!("needs {count} bytes at offset {}, of {}", self.at, self.bytes.len());
        };
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> eyre::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn u16(&mut self) -> eyre::Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> eyre::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn vlq(&mut self) -> eyre::Result<u32> {
        let mut value = 0u32;
        for _ in 0..VLQ_MAX_BYTES {
            let byte = self.u8()?;
            value = (value << 7) | u32::from(byte & VLQ_VALUE_MASK);
            if byte & VLQ_CONTINUE_BIT == 0 {
                return Ok(value);
            }
        }
        eyre::bail!("a variable-length number longer than {VLQ_MAX_BYTES} bytes before offset {}", self.at);
    }
}

/// A Set Tempo event at a tick of the file's time line.
struct TempoChange {
    tick: u64,
    microseconds_per_quarter: u32,
}

fn read_track(bytes: &[u8], tempo: &mut Vec<TempoChange>) -> eyre::Result<MidiTrack> {
    let mut cursor = Cursor { bytes, at: 0 };
    let mut track = MidiTrack::default();
    let mut running_status = None;
    while cursor.peek().is_some() {
        track.ticks += u64::from(cursor.vlq()?);
        let first = cursor.peek().ok_or_else(|| eyre!("a track ends after a delta time"))?;
        let status = if first & STATUS_BIT != 0 {
            cursor.at += 1;
            first
        } else {
            running_status.ok_or_else(|| eyre!("a data byte at offset {} with no status before it", cursor.at))?
        };
        track.events += 1;
        match status {
            META_EVENT => {
                running_status = None;
                let kind = cursor.u8()?;
                let size = cursor.vlq()? as usize;
                let data = cursor.take(size)?;
                match kind {
                    META_TRACK_NAME if track.name.is_none() => track.name = Some(String::from_utf8_lossy(data).trim().to_owned()),
                    META_SET_TEMPO if size == SET_TEMPO_BYTES => {
                        let microseconds_per_quarter = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                        tempo.push(TempoChange { tick: track.ticks, microseconds_per_quarter });
                    }
                    META_END_OF_TRACK => break,
                    _ => {}
                }
            }
            SYSEX_EVENT | SYSEX_ESCAPE_EVENT => {
                running_status = None;
                let size = cursor.vlq()? as usize;
                cursor.take(size)?;
            }
            _ => {
                running_status = Some(status);
                let kind = status & VOICE_KIND_MASK;
                match kind {
                    PROGRAM_CHANGE => {
                        track.programs.insert(cursor.u8()?);
                    }
                    CHANNEL_PRESSURE => {
                        cursor.u8()?;
                    }
                    NOTE_ON => {
                        cursor.u8()?;
                        if cursor.u8()? > 0 {
                            track.notes += 1;
                        }
                    }
                    _ => {
                        cursor.take(2)?;
                    }
                }
            }
        }
    }
    Ok(track)
}

/// The seconds of `ticks` ticks at `ticks_per_quarter`, following the tempo changes.
fn timed_duration(ticks: u64, ticks_per_quarter: u16, tempo: &mut [TempoChange]) -> f64 {
    tempo.sort_by_key(|change| change.tick);
    let seconds_of = |ticks: u64, microseconds_per_quarter: u32| {
        ticks as f64 * f64::from(microseconds_per_quarter) / f64::from(ticks_per_quarter.max(1)) / MICROSECONDS_PER_SECOND
    };
    let (mut seconds, mut last_tick, mut current) = (0.0, 0, DEFAULT_MICROSECONDS_PER_QUARTER);
    for change in tempo.iter().take_while(|change| change.tick <= ticks) {
        seconds += seconds_of(change.tick - last_tick, current);
        last_tick = change.tick;
        current = change.microseconds_per_quarter;
    }
    seconds + seconds_of(ticks - last_tick, current)
}

/// Reads a Standard MIDI file. Chunks of other kinds are skipped, as the format asks.
pub fn analyze_midi(payload: &[u8]) -> eyre::Result<MidiAnalysis> {
    let mut cursor = Cursor { bytes: payload, at: 0 };
    if !is_midi(payload) {
        eyre::bail!("not a Standard MIDI file: no {} chunk at the start", String::from_utf8_lossy(HEADER_MAGIC));
    }
    cursor.take(HEADER_MAGIC.len())?;
    let header_size = cursor.u32()? as usize;
    if header_size < HEADER_MIN_BYTES {
        eyre::bail!("a header chunk of {header_size} bytes; the format has {HEADER_MIN_BYTES}");
    }
    let header = cursor.take(header_size)?;
    let mut header = Cursor { bytes: header, at: 0 };
    let format = header.u16()?;
    let declared_tracks = header.u16()?;
    let division_field = header.u16()?;
    let division = if division_field & DIVISION_SMPTE_BIT != 0 {
        let [frames, ticks] = division_field.to_be_bytes();
        MidiDivision::Smpte { frames_per_second: (frames as i8).unsigned_abs(), ticks_per_frame: ticks }
    } else {
        MidiDivision::TicksPerQuarter(division_field)
    };

    let mut tracks = Vec::with_capacity(usize::from(declared_tracks));
    let mut tempo = Vec::new();
    while cursor.at + CHUNK_HEADER_BYTES <= payload.len() {
        let kind = cursor.take(TRACK_MAGIC.len())?;
        let size = cursor.u32()? as usize;
        let chunk = cursor.take(size).wrap_err_with(|| format!("chunk {}", String::from_utf8_lossy(kind)))?;
        if kind == TRACK_MAGIC {
            tracks.push(read_track(chunk, &mut tempo).wrap_err_with(|| format!("track {}", tracks.len()))?);
        }
    }
    if tracks.len() != usize::from(declared_tracks) {
        eyre::bail!("the header declares {declared_tracks} tracks and the file has {}", tracks.len());
    }

    let last_tick = tracks.iter().map(|track| track.ticks).max().unwrap_or(0);
    let first_tempo = tempo.iter().min_by_key(|change| change.tick).map_or(DEFAULT_MICROSECONDS_PER_QUARTER, |change| change.microseconds_per_quarter);
    let duration_seconds = match division {
        _ if format == FORMAT_INDEPENDENT_SEQUENCES => None,
        MidiDivision::TicksPerQuarter(ticks_per_quarter) => Some(timed_duration(last_tick, ticks_per_quarter, &mut tempo)),
        MidiDivision::Smpte { frames_per_second, ticks_per_frame } => {
            Some(last_tick as f64 / (f64::from(frames_per_second) * f64::from(ticks_per_frame)).max(1.0))
        }
    };
    Ok(MidiAnalysis {
        format,
        division,
        tracks,
        duration_seconds,
        first_tempo_bpm: MICROSECONDS_PER_MINUTE / f64::from(first_tempo.max(1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICKS_PER_QUARTER: u16 = 480;
    const HALF_SECOND_TEMPO: [u8; 3] = [0x07, 0xa1, 0x20];
    const QUARTER_SECOND_TEMPO: [u8; 3] = [0x03, 0xd0, 0x90];
    const TOLERANCE: f64 = 1e-9;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_vec();
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    fn file(format: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut header = format.to_be_bytes().to_vec();
        header.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        header.extend_from_slice(&TICKS_PER_QUARTER.to_be_bytes());
        let mut bytes = chunk(HEADER_MAGIC, &header);
        for track in tracks {
            bytes.extend_from_slice(&chunk(TRACK_MAGIC, track));
        }
        bytes
    }

    /// A tempo of 500,000 microseconds, a program, then two notes one quarter apart with running status.
    fn conductor_and_notes() -> Vec<u8> {
        let mut track = vec![0x00, META_EVENT, META_TRACK_NAME, 0x04];
        track.extend_from_slice(b"Lute");
        track.extend_from_slice(&[0x00, META_EVENT, META_SET_TEMPO, 0x03]);
        track.extend_from_slice(&HALF_SECOND_TEMPO);
        track.extend_from_slice(&[0x00, 0xc1, 24]);
        track.extend_from_slice(&[0x00, 0x91, 60, 100]);
        // 480 ticks later, in running status: a note off as velocity 0, then a second note.
        track.extend_from_slice(&[0x83, 0x60, 60, 0]);
        track.extend_from_slice(&[0x00, 62, 90]);
        track.extend_from_slice(&[0x83, 0x60, 0x81, 62, 0]);
        track.extend_from_slice(&[0x00, META_EVENT, META_END_OF_TRACK, 0x00]);
        track
    }

    #[test]
    fn reads_the_header_tracks_and_length() {
        let analysis = analyze_midi(&file(1, &[&conductor_and_notes()])).unwrap();
        assert_eq!((analysis.format, analysis.division), (1, MidiDivision::TicksPerQuarter(TICKS_PER_QUARTER)));
        let track = &analysis.tracks[0];
        assert_eq!(track.name.as_deref(), Some("Lute"));
        assert_eq!(track.notes, 2);
        assert_eq!(track.programs, BTreeSet::from([24]));
        assert_eq!(track.ticks, 960);
        assert!((analysis.duration_seconds.unwrap() - 1.0).abs() < TOLERANCE);
        assert_eq!(analysis.first_tempo_bpm, 120.0);
    }

    #[test]
    fn a_tempo_change_shortens_what_follows_it() {
        let mut track = vec![0x00, META_EVENT, META_SET_TEMPO, 0x03];
        track.extend_from_slice(&HALF_SECOND_TEMPO);
        track.extend_from_slice(&[0x83, 0x60, META_EVENT, META_SET_TEMPO, 0x03]);
        track.extend_from_slice(&QUARTER_SECOND_TEMPO);
        track.extend_from_slice(&[0x83, 0x60, META_EVENT, META_END_OF_TRACK, 0x00]);
        let analysis = analyze_midi(&file(1, &[&track])).unwrap();
        assert!((analysis.duration_seconds.unwrap() - 0.75).abs() < TOLERANCE);
    }

    #[test]
    fn independent_sequences_have_no_common_length() {
        let analysis = analyze_midi(&file(FORMAT_INDEPENDENT_SEQUENCES, &[&conductor_and_notes()])).unwrap();
        assert_eq!(analysis.duration_seconds, None);
    }

    #[test]
    fn refuses_what_is_not_a_whole_midi_file() {
        assert!(analyze_midi(b"RIFF....").is_err());
        let whole = file(1, &[&conductor_and_notes()]);
        assert!(analyze_midi(&whole[..whole.len() - 3]).is_err(), "a cut track");
        let mut wrong_count = whole.clone();
        wrong_count[11] = 2;
        assert!(analyze_midi(&wrong_count).is_err(), "more tracks declared than present");
        assert!(analyze_midi(&file(1, &[&[0x00, 0x40, 0x00]])).is_err(), "a data byte with no status");
    }
}
