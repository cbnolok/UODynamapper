//! Census of a client's audio ([`census_client`]): every entry of `Audio.uop`, every Classic sound slot
//! (`sound.mul` or `soundLegacyMUL.uop`), every file under `Music/`, probed by content and decoded once; and the
//! `Sound.def`, `Music.def` and `Music/Digital/Config.txt` parse results. [`AudioCensus::summary_markdown`] groups
//! the rows; [`AudioCensus::write_tsv`] lists them one per entry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use rayon::prelude::*;

use super::decode::{decode, decode_classic_pcm};
use super::{hex_prefix, name_extension, probe_audio, AudioContent, AudioFormat};
use crate::classic::audio_def::{DefTranslationTable, MUSIC_DEF_FILE_NAME, SOUND_DEF_FILE_NAME};
use crate::classic::music::{find_dir_ignoring_case, MusicConfig, MUSIC_DIR_NAME};
use crate::classic::sound::{SoundMap, SoundSlotSource, SOUND_HEADER_BYTES, SOUND_NAME_BYTES};
use crate::enhanced::audio::{AudioPackage, AudioTables, AUDIO_UOP_FILE_NAME};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash_dictionary::HashDictionary;
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

/// Decoded and expected lengths may differ by this many samples per channel before a row counts as a mismatch
/// (an MP3 decoder may drop or add one frame's worth at the edges).
pub const DECODE_TOLERANCE_FRAMES: u64 = 1152;
/// Bytes of one Classic PCM sample frame (mono, 16-bit).
const CLASSIC_FRAME_BYTES: usize = 2;

/// One decode of a row's payload.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeCheck {
    pub frames: u64,
    /// Samples per channel the probe counts.
    pub expected: Option<u64>,
    pub skipped_packets: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub error: Option<String>,
}

impl DecodeCheck {
    /// Decoded, with the probed rate and channels and within [`DECODE_TOLERANCE_FRAMES`] of the probed length.
    pub fn matches(&self, content: &AudioContent) -> bool {
        self.error.is_none()
            && self.expected.is_none_or(|expected| self.frames.abs_diff(expected) <= DECODE_TOLERANCE_FRAMES)
            && content.sample_rate().is_none_or(|rate| rate == self.sample_rate)
            && content.channels().is_none_or(|channels| channels == self.channels)
    }
}

/// One audio file or entry.
#[derive(Debug, Clone)]
pub struct CensusRow {
    /// The client folder's name.
    pub client: String,
    /// `Audio.uop`, `soundLegacyMUL.uop`, `sound.mul`, `Music`.
    pub container: String,
    /// The entry name, the slot id or the path under `Music/`; `None` for an unnamed entry.
    pub name: Option<String>,
    pub hash: Option<u64>,
    pub slot_id: Option<u32>,
    pub compression: Option<CompressionFlag>,
    pub stored_size: usize,
    pub size: usize,
    pub content: AudioContent,
    /// The bytes after the name in a Classic slot's header, in hex.
    pub classic_header_tail: Option<String>,
    pub decode: Option<DecodeCheck>,
}

impl CensusRow {
    /// The format the name's extension stands for, when it is not the content's. A Classic slot's name is the
    /// file its PCM came from (`d_frst01.wav`), so it is not checked.
    pub fn name_mismatch(&self) -> Option<AudioFormat> {
        if self.content.format() == AudioFormat::ClassicPcm {
            return None;
        }
        let expected = AudioFormat::of_extension(&name_extension(self.name.as_deref()?)?)?;
        (expected != self.content.format()).then_some(expected)
    }

    fn label(&self) -> String {
        match (&self.name, self.hash) {
            (Some(name), _) => name.clone(),
            (None, Some(hash)) => format!("{hash:016X}"),
            (None, None) => String::new(),
        }
    }
}

/// The census of one or more clients.
#[derive(Debug, Clone, Default)]
pub struct AudioCensus {
    pub rows: Vec<CensusRow>,
    /// Per client: what the definition files hold, or why they failed to parse.
    pub notes: Vec<(String, String)>,
}

fn decode_check(payload: &[u8], content: &AudioContent) -> Option<DecodeCheck> {
    if !content.format().is_decodable() {
        return None;
    }
    let expected = match content {
        AudioContent::Mpeg(analysis) => Some(analysis.samples()),
        AudioContent::Wav(analysis) => analysis.sample_frames(),
        AudioContent::ClassicPcm { pcm_len } => Some((pcm_len / CLASSIC_FRAME_BYTES) as u64),
        _ => None,
    };
    let decoded = match content {
        AudioContent::ClassicPcm { .. } => Ok(decode_classic_pcm(payload)),
        _ => decode(payload, content),
    };
    Some(match decoded {
        Ok(decoded) => DecodeCheck {
            frames: decoded.frames(),
            expected,
            skipped_packets: decoded.skipped_packets,
            sample_rate: decoded.sample_rate,
            channels: decoded.channels,
            error: None,
        },
        Err(error) => DecodeCheck { frames: 0, expected, skipped_packets: 0, sample_rate: 0, channels: 0, error: Some(error.to_string()) },
    })
}

fn client_label(client_path: &Path) -> String {
    client_path.file_name().map_or_else(|| client_path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The census of one client folder. `dictionary` names the `Audio.uop` entries the tables do not.
pub fn census_client(client_path: &Path, dictionary: Option<&HashDictionary>) -> eyre::Result<AudioCensus> {
    let client = client_label(client_path);
    let mut census = AudioCensus::default();

    if let Some(audio) = AudioPackage::load(client_path)? {
        let tables = match audio.tables() {
            AudioTables::Enhanced { sounds, music } => format!("Enhanced tables: {} sounds, {} music", sounds.len(), music.len()),
            AudioTables::KingdomReborn(kr) => format!(
                "Kingdom Reborn tables: {} sound structures, {} effects rows",
                kr.sounds.sounds().filter(|row| row.duplicate_of.is_none()).count(),
                kr.effects.as_ref().map_or(0, |effects| effects.rows().count())
            ),
            AudioTables::Missing => "no sound table".to_owned(),
        };
        census.notes.push((client.clone(), format!("{AUDIO_UOP_FILE_NAME}: {tables}")));
        for warning in audio.table_warnings() {
            census.notes.push((client.clone(), format!("{AUDIO_UOP_FILE_NAME}: warning: {warning}")));
        }
        let hashes: Vec<u64> = audio.hashes().collect();
        let rows: Vec<eyre::Result<CensusRow>> = hashes
            .par_iter()
            .map(|&hash| {
                let entry = audio.entry_by_hash(hash)?.ok_or_else(|| eyre!("entry {hash:016X} vanished"))?;
                let name = entry.name.clone().or_else(|| dictionary.and_then(|dictionary| dictionary.resolve(hash)).map(str::to_owned));
                let decode = decode_check(&entry.payload, &entry.content);
                Ok(CensusRow {
                    client: client.clone(),
                    container: AUDIO_UOP_FILE_NAME.to_owned(),
                    name,
                    hash: Some(hash),
                    slot_id: None,
                    compression: Some(entry.compression),
                    stored_size: entry.stored.len(),
                    size: entry.payload.len(),
                    content: entry.content,
                    classic_header_tail: None,
                    decode,
                })
            })
            .collect();
        census.rows.extend(rows.into_iter().collect::<eyre::Result<Vec<_>>>()?);
    }

    let classic_sounds = SoundMap::has_sound_files(client_path).then(|| SoundMap::load(client_path));
    match &classic_sounds {
        None => {}
        Some(Ok(sounds)) => {
            census.notes.push((client.clone(), format!("{}: {} slot ids scanned", sounds.source_label(), sounds.slot_count())));
            let container = sounds.source_label().to_owned();
            let slots: Vec<u32> = (0..sounds.slot_count() as u32).collect();
            let rows: Vec<eyre::Result<Option<CensusRow>>> = slots
                .par_iter()
                .map(|&slot_id| {
                    let Some((bytes, source)) = sounds.read_slot_bytes(slot_id)? else {
                        return Ok(None);
                    };
                    if bytes.is_empty() {
                        return Ok(None);
                    }
                    let (content, tail, pcm) = if bytes.len() > SOUND_HEADER_BYTES {
                        let pcm = &bytes[SOUND_HEADER_BYTES..];
                        (AudioContent::ClassicPcm { pcm_len: pcm.len() }, Some(hex_prefix(&bytes[SOUND_NAME_BYTES..SOUND_HEADER_BYTES])), pcm)
                    } else {
                        (AudioContent::Unknown { hex_prefix: hex_prefix(&bytes) }, None, &bytes[..0])
                    };
                    let name = crate::classic::sound::parse_sound_slot(slot_id, &bytes, source).map(|sound| sound.name);
                    let decode = decode_check(pcm, &content);
                    Ok(Some(CensusRow {
                        client: client.clone(),
                        container: container.clone(),
                        name,
                        hash: None,
                        slot_id: Some(slot_id),
                        compression: match source {
                            SoundSlotSource::Uop(compression) => Some(compression),
                            SoundSlotSource::Mul | SoundSlotSource::Verdata => None,
                        },
                        stored_size: bytes.len(),
                        size: bytes.len(),
                        content,
                        classic_header_tail: tail,
                        decode,
                    }))
                })
                .collect();
            census.rows.extend(rows.into_iter().collect::<eyre::Result<Vec<_>>>()?.into_iter().flatten());
        }
        Some(Err(error)) => census.notes.push((client.clone(), format!("Classic sounds: {error:#}"))),
    }
    if let Some(Ok(sounds)) = &classic_sounds {
        for note in sounds.translation_notes()? {
            census.notes.push((client.clone(), note));
        }
    }

    for name in [SOUND_DEF_FILE_NAME, MUSIC_DEF_FILE_NAME] {
        let Some(path) = find_file_ignoring_case(client_path, name) else { continue };
        let note = match DefTranslationTable::load(&path) {
            Ok(Some(table)) => {
                let ignored: Vec<usize> = table.iter().filter(|entry| entry.client_ignored.is_some()).map(|entry| entry.line).collect();
                let multi = table.iter().filter(|entry| entry.client_targets().is_some_and(|targets| targets.len() > 1)).count();
                let negative = table.iter().filter(|entry| entry.negative_targets().next().is_some()).count();
                let readers = if name == MUSIC_DEF_FILE_NAME { "; no client reads Music.def" } else { "" };
                format!(
                    "{name}: {} ids, {multi} with several targets (a random pick per play), {negative} with a `-` target, lines \
                     the client ignores {ignored:?}{readers}",
                    table.len()
                )
            }
            Ok(None) => continue,
            Err(error) => format!("{name}: {error:#}"),
        };
        census.notes.push((client.clone(), note));
    }
    match MusicConfig::load(client_path) {
        Ok(Some(config)) => {
            let missing: Vec<String> = config.iter().filter(|entry| config.file_path(entry).is_none()).map(|entry| entry.file_name()).collect();
            let flags: BTreeSet<String> = config.iter().filter_map(|entry| entry.flag.clone()).collect();
            for note in config.client_notes() {
                census.notes.push((client.clone(), format!("Config.txt {note}")));
            }
            let with_extension = config.iter().filter(|entry| entry.name.contains('.')).count();
            census.notes.push((
                client.clone(),
                format!(
                    "Config.txt: {} ids, {} looping, {with_extension} names with an extension, flags {flags:?}, missing files {missing:?}",
                    config.len(),
                    config.iter().filter(|entry| entry.loops()).count()
                ),
            ));
        }
        Ok(None) => {}
        Err(error) => census.notes.push((client.clone(), format!("Config.txt: {error:#}"))),
    }

    if let Some(music) = find_dir_ignoring_case(client_path, MUSIC_DIR_NAME) {
        let mut files = Vec::new();
        collect_files(&music, &mut files)?;
        let rows: Vec<eyre::Result<CensusRow>> = files
            .par_iter()
            .map(|path| {
                let bytes = std::fs::read(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
                let content = probe_audio(&bytes);
                let decode = decode_check(&bytes, &content);
                let relative = path.strip_prefix(client_path).unwrap_or(path).to_string_lossy().replace('\\', "/");
                Ok(CensusRow {
                    client: client.clone(),
                    container: MUSIC_DIR_NAME.to_owned(),
                    name: Some(relative),
                    hash: None,
                    slot_id: None,
                    compression: None,
                    stored_size: bytes.len(),
                    size: bytes.len(),
                    content,
                    classic_header_tail: None,
                    decode,
                })
            })
            .collect();
        census.rows.extend(rows.into_iter().collect::<eyre::Result<Vec<_>>>()?);
    }
    census.rows.sort_by(|a, b| (&a.container, a.slot_id, a.label()).cmp(&(&b.container, b.slot_id, b.label())));
    Ok(census)
}

fn collect_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> eyre::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .wrap_err_with(|| format!("Failed to list {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// Counts of a key, printed most frequent first.
#[derive(Default)]
struct Tally(BTreeMap<String, usize>);

impl Tally {
    fn add(&mut self, key: impl Into<String>) {
        *self.0.entry(key.into()).or_default() += 1;
    }

    fn render(&self) -> String {
        let mut pairs: Vec<_> = self.0.iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        pairs.iter().map(|(key, count)| format!("{key} ({count})")).collect::<Vec<_>>().join(", ")
    }
}

fn compression_label(compression: Option<CompressionFlag>) -> &'static str {
    match compression {
        None => "-",
        Some(CompressionFlag::None) => "none",
        Some(CompressionFlag::Zlib) => "zlib",
        Some(CompressionFlag::Mythic) => "mythic",
        Some(CompressionFlag::ZlibBwt) => "zlib+bwt",
    }
}

fn seconds_label(seconds: f64) -> String {
    format!("{seconds:.3}")
}

impl AudioCensus {
    pub fn extend(&mut self, other: AudioCensus) {
        self.rows.extend(other.rows);
        self.notes.extend(other.notes);
    }

    pub fn decode_failures(&self) -> impl Iterator<Item = &CensusRow> {
        self.rows.iter().filter(|row| row.decode.as_ref().is_some_and(|check| !check.matches(&row.content)))
    }

    /// The rows grouped by client and container: formats, compression, codec details, tags, chunk layouts,
    /// durations, unnamed entries, name/content mismatches, unknown payloads and decode failures.
    pub fn summary_markdown(&self) -> String {
        let mut groups: BTreeMap<(&str, &str), Vec<&CensusRow>> = BTreeMap::new();
        for row in &self.rows {
            groups.entry((&row.client, &row.container)).or_default().push(row);
        }
        let mut out = String::new();
        let clients: BTreeSet<&str> =
            self.notes.iter().map(|(client, _)| client.as_str()).chain(groups.keys().map(|(client, _)| *client)).collect();
        for client in clients {
            let _ = writeln!(out, "## {client}\n");
            for (_, note) in self.notes.iter().filter(|(note_client, _)| note_client == client) {
                let _ = writeln!(out, "- {note}");
            }
            let _ = writeln!(out);
            for ((_, container), rows) in groups.iter().filter(|((group_client, _), _)| *group_client == client) {
                let _ = writeln!(out, "### {container}: {} entries\n", rows.len());
                self.summarize_group(&mut out, rows);
            }
        }
        out
    }

    fn summarize_group(&self, out: &mut String, rows: &[&CensusRow]) {
        let mut formats = Tally::default();
        let mut compression = Tally::default();
        let mut by_format: BTreeMap<AudioFormat, Vec<&CensusRow>> = BTreeMap::new();
        for row in rows {
            formats.add(row.content.format().label());
            compression.add(format!("{} {}", row.content.format().label(), compression_label(row.compression)));
            by_format.entry(row.content.format()).or_default().push(row);
        }
        let _ = writeln!(out, "- formats: {}", formats.render());
        let _ = writeln!(out, "- compression: {}", compression.render());
        let unnamed = rows.iter().filter(|row| row.name.is_none()).count();
        let _ = writeln!(out, "- unnamed: {unnamed}");
        for (format, rows) in &by_format {
            let mut detail = Tally::default();
            let mut durations: Vec<f64> = Vec::new();
            for row in rows {
                detail.add(row.content.detail());
                durations.extend(row.content.duration_seconds());
            }
            let total: f64 = durations.iter().sum();
            let shortest = durations.iter().copied().fold(f64::INFINITY, f64::min);
            let longest = durations.iter().copied().fold(0.0, f64::max);
            let _ = writeln!(out, "- {}: {}", format.label(), detail.render());
            if !durations.is_empty() {
                let _ = writeln!(
                    out,
                    "  - duration: {} s in all, {}..{} s",
                    seconds_label(total),
                    seconds_label(shortest),
                    seconds_label(longest)
                );
            }
            match format {
                AudioFormat::Mpeg(_) => summarize_mpeg(out, rows),
                AudioFormat::Wav => summarize_wav(out, rows),
                AudioFormat::ClassicPcm => {
                    let mut tails = Tally::default();
                    rows.iter().filter_map(|row| row.classic_header_tail.clone()).for_each(|tail| tails.add(tail));
                    let _ = writeln!(out, "  - header bytes 32..40: {}", tails.render());
                }
                AudioFormat::Unknown => {
                    for row in rows {
                        let _ = writeln!(out, "  - {}: {} bytes, `{}`", row.label(), row.size, row.content.detail());
                    }
                }
                _ => {}
            }
        }
        let mismatches: Vec<String> = rows
            .iter()
            .filter_map(|row| row.name_mismatch().map(|expected| format!("{} (named {}, holds {})", row.label(), expected.label(), row.content.format().label())))
            .collect();
        let _ = writeln!(out, "- name/content mismatches: {}", if mismatches.is_empty() { "none".to_owned() } else { mismatches.join("; ") });
        let decoded = rows.iter().filter(|row| row.decode.is_some()).count();
        let failures: Vec<String> = rows
            .iter()
            .filter_map(|row| {
                let check = row.decode.as_ref()?;
                (!check.matches(&row.content)).then(|| match &check.error {
                    Some(error) => format!("{}: {error}", row.label()),
                    None => format!("{}: {} frames decoded, {:?} probed, {} Hz {} ch", row.label(), check.frames, check.expected, check.sample_rate, check.channels),
                })
            })
            .collect();
        let skipped: u32 = rows.iter().filter_map(|row| row.decode.as_ref()).map(|check| check.skipped_packets).sum();
        let exact = rows.iter().filter(|row| row.decode.as_ref().is_some_and(|check| check.expected == Some(check.frames))).count();
        let _ = writeln!(
            out,
            "- decoded: {decoded}, {exact} to exactly the probed length, {skipped} packets skipped, failures: {}\n",
            if failures.is_empty() { "none".to_owned() } else { failures.join("; ") }
        );
    }

    /// One row per entry, tab-separated, with a header line.
    pub fn write_tsv(&self, writer: &mut impl Write) -> eyre::Result<()> {
        writeln!(
            writer,
            "client\tcontainer\tname\thash\tslot\tcompression\tstored_size\tsize\tformat\tdetail\tsample_rate\tchannels\tbits\tduration\t\
             name_mismatch\tid3v2\tid3v1\tape\tinfo_header\tencoder\tdelay\tpadding\tleading_zero\tleading_junk\ttrailing\ttrailing_zero\t\
             resyncs\ttruncated_frame\tbitrates\twav_fmt_size\twav_chunks\twav_unpadded\twav_riff_size_ok\twav_trailing\twav_data_truncated\t\
             classic_header_tail\tdecode\tdecoded_frames\tprobed_frames\tskipped_packets"
        )?;
        for row in &self.rows {
            let mut fields: Vec<String> = vec![
                row.client.clone(),
                row.container.clone(),
                row.name.clone().unwrap_or_default(),
                row.hash.map(|hash| format!("{hash:016X}")).unwrap_or_default(),
                row.slot_id.map(|slot| slot.to_string()).unwrap_or_default(),
                compression_label(row.compression).to_owned(),
                row.stored_size.to_string(),
                row.size.to_string(),
                row.content.format().label().to_owned(),
                row.content.detail(),
                row.content.sample_rate().map(|rate| rate.to_string()).unwrap_or_default(),
                row.content.channels().map(|channels| channels.to_string()).unwrap_or_default(),
                row.content.bits_per_sample().map(|bits| bits.to_string()).unwrap_or_default(),
                row.content.duration_seconds().map(seconds_label).unwrap_or_default(),
                row.name_mismatch().map(|format| format.label().to_owned()).unwrap_or_default(),
            ];
            let mpeg = match &row.content {
                AudioContent::Mpeg(analysis) => {
                    let info = analysis.info_header.as_ref();
                    vec![
                        analysis.id3v2.iter().map(|tag| format!("{}:{}", tag.label(), tag.length)).collect::<Vec<_>>().join(" "),
                        analysis.tail.id3v1.to_string(),
                        analysis.tail.ape.to_string(),
                        info.map(|info| info.kind.label().to_owned()).unwrap_or_default(),
                        info.and_then(|info| info.encoder.clone()).unwrap_or_default(),
                        info.and_then(|info| info.delay).map(|delay| delay.to_string()).unwrap_or_default(),
                        info.and_then(|info| info.padding).map(|padding| padding.to_string()).unwrap_or_default(),
                        analysis.leading_zero_bytes.to_string(),
                        analysis.leading_junk_bytes.to_string(),
                        analysis.tail.trailing_bytes.to_string(),
                        analysis.tail.trailing_zero.to_string(),
                        analysis.resyncs.to_string(),
                        analysis.truncated_last_frame.to_string(),
                        analysis.bitrates.iter().map(u32::to_string).collect::<Vec<_>>().join(" "),
                    ]
                }
                _ => vec![String::new(); 14],
            };
            fields.extend(mpeg);
            let wav = match &row.content {
                AudioContent::Wav(analysis) => vec![
                    analysis.format.map(|format| format.chunk_size.to_string()).unwrap_or_default(),
                    analysis.chunks.iter().map(|chunk| chunk.id_label()).collect::<Vec<_>>().join(" "),
                    analysis.unpadded_chunks().to_string(),
                    analysis.riff_size_matches().to_string(),
                    analysis.trailing_bytes.to_string(),
                    analysis.data_truncated.to_string(),
                ],
                _ => vec![String::new(); 6],
            };
            fields.extend(wav);
            fields.push(row.classic_header_tail.clone().unwrap_or_default());
            match &row.decode {
                Some(check) => fields.extend([
                    match &check.error {
                        Some(error) => format!("error: {error}"),
                        None if check.matches(&row.content) => "ok".to_owned(),
                        None => "mismatch".to_owned(),
                    },
                    check.frames.to_string(),
                    check.expected.map(|expected| expected.to_string()).unwrap_or_default(),
                    check.skipped_packets.to_string(),
                ]),
                None => fields.extend(vec![String::new(); 4]),
            }
            let line: Vec<String> = fields.into_iter().map(|field| field.replace(['\t', '\n', '\r'], " ")).collect();
            writeln!(writer, "{}", line.join("\t"))?;
        }
        Ok(())
    }
}

fn summarize_mpeg(out: &mut String, rows: &[&CensusRow]) {
    let mut bitrate = Tally::default();
    let mut id3v2 = Tally::default();
    let mut info = Tally::default();
    let mut encoder = Tally::default();
    let (mut id3v1, mut ape, mut leading_zero, mut leading_junk, mut trailing, mut trailing_nonzero, mut resyncs, mut truncated, mut inconsistent) =
        (0, 0, 0, 0, 0, 0, 0, 0, 0);
    for row in rows {
        let AudioContent::Mpeg(analysis) = &row.content else { continue };
        bitrate.add(if analysis.is_cbr() { "CBR" } else { "VBR" });
        id3v2.add(if analysis.id3v2.is_empty() { "none".to_owned() } else { analysis.id3v2.iter().map(|tag| tag.label()).collect::<Vec<_>>().join("+") });
        info.add(analysis.info_header.as_ref().map_or("none", |header| header.kind.label()));
        if let Some(name) = analysis.info_header.as_ref().and_then(|header| header.encoder.clone()) {
            encoder.add(name);
        }
        id3v1 += usize::from(analysis.tail.id3v1);
        ape += usize::from(analysis.tail.ape);
        leading_zero += usize::from(analysis.leading_zero_bytes > 0);
        leading_junk += usize::from(analysis.leading_junk_bytes > 0);
        trailing += usize::from(analysis.tail.trailing_bytes > 0);
        trailing_nonzero += usize::from(!analysis.tail.trailing_zero);
        resyncs += usize::from(analysis.resyncs > 0);
        truncated += usize::from(analysis.truncated_last_frame);
        inconsistent += usize::from(analysis.inconsistent_frames > 0);
    }
    let _ = writeln!(out, "  - bitrate mode: {}", bitrate.render());
    let _ = writeln!(out, "  - ID3v2: {}; ID3v1: {id3v1}; APE: {ape}", id3v2.render());
    let _ = writeln!(out, "  - info header: {}; encoder: {}", info.render(), if encoder.0.is_empty() { "none".to_owned() } else { encoder.render() });
    let _ = writeln!(
        out,
        "  - leading zero bytes: {leading_zero}; leading other bytes: {leading_junk}; bytes after the last frame: {trailing} \
         ({trailing_nonzero} not all zero); resynced: {resyncs}; last frame cut: {truncated}; frames of another stream: {inconsistent}"
    );
}

fn summarize_wav(out: &mut String, rows: &[&CensusRow]) {
    let mut fmt_size = Tally::default();
    let mut layout = Tally::default();
    let (mut unpadded, mut riff_mismatch, mut trailing, mut truncated, mut inconsistent) = (0, 0, 0, 0, 0);
    for row in rows {
        let AudioContent::Wav(analysis) = &row.content else { continue };
        fmt_size.add(analysis.format.map_or("none".to_owned(), |format| format.chunk_size.to_string()));
        layout.add(analysis.chunks.iter().map(|chunk| chunk.id_label().trim_end().to_owned()).collect::<Vec<_>>().join(" "));
        unpadded += usize::from(analysis.unpadded_chunks() > 0);
        riff_mismatch += usize::from(!analysis.riff_size_matches());
        trailing += usize::from(analysis.trailing_bytes > 0);
        truncated += usize::from(analysis.data_truncated);
        inconsistent += usize::from(analysis.format.is_some_and(|format| !format.is_consistent()));
    }
    let _ = writeln!(out, "  - fmt chunk size: {}", fmt_size.render());
    let _ = writeln!(out, "  - chunk layouts: {}", layout.render());
    let _ = writeln!(
        out,
        "  - odd chunks without their pad byte: {unpadded}; RIFF size not the file's: {riff_mismatch}; bytes after the last \
         chunk: {trailing}; data cut short: {truncated}; byte rate or block align inconsistent: {inconsistent}"
    );
}
