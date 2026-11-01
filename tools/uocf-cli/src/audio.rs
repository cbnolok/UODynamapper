//! `uocf-cli audio`: the census of client audio, the list of the audio entries, exports of the files as they
//! are (and decoded to WAV) by id, name or all, and `Audio.uop` copies with entries replaced or added.

use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use clap::{ArgGroup, Subcommand};
use color_eyre::eyre::{self, eyre, WrapErr};
use uocf::audio::census::{census_client, AudioCensus};
use uocf::audio::validate::FormatPolicy;
use uocf::audio::decode::{decode, decode_classic_pcm};
use uocf::audio::AudioContent;
use uocf::classic::sound::{ClassicSound, SoundMap};
use uocf::enhanced::audio::{AudioEntry, AudioKind, AudioPackage, AudioTables, AUDIO_ENTRY_ROOT};
use uocf::enhanced::audio_edit::{AudioImportOptions, AudioPackageEditor, AudioPut, AudioTarget, NewAudio};
use uocf::uop_container::hash_dictionary::HashDictionary;

/// Where exports of unnamed `Audio.uop` entries and Classic slots go, under the output folder.
const UNNAMED_DIR: &str = "unnamed";
const CLASSIC_DIR: &str = "classic";
const RAW_EXTENSION: &str = "bin";
const CLASSIC_SLOT_EXTENSION: &str = "dat";
const WAV_EXTENSION: &str = "wav";
/// Appended to the stem of a decoded copy: `d_frst01.decoded.wav`.
const DECODED_SUFFIX: &str = ".decoded";

#[derive(Subcommand, Debug)]
pub enum AudioCmd {
    /// Census of client audio: every Audio.uop entry, Classic sound slot (sound.mul or soundLegacyMUL.uop) and
    /// file under Music/, probed by content and decoded once; and the Sound.def, Music.def and Config.txt parse
    /// results. Writes a Markdown summary and, optionally, one TSV row per entry.
    Census {
        /// Client folder; repeat for several clients.
        #[arg(long = "client", required = true)]
        clients: Vec<PathBuf>,
        /// Hash dictionary (.dic) naming the Audio.uop entries the tables do not; repeat to merge several.
        #[arg(long)]
        dictionary: Vec<PathBuf>,
        /// The Markdown summary.
        #[arg(long)]
        output: PathBuf,
        /// One tab-separated row per entry.
        #[arg(long)]
        tsv: Option<PathBuf>,
    },
    /// List a client's audio: the Audio.uop table ids with their entries, or the Classic slots.
    List {
        #[arg(long)]
        client: PathBuf,
        /// Hash dictionary (.dic) naming the entries the tables do not; repeat to merge several.
        #[arg(long)]
        dictionary: Vec<PathBuf>,
    },
    /// Export audio as stored (the Audio.uop file, or the Classic slot with its header), and decoded to 16-bit
    /// PCM WAV with --wav. Named Audio.uop entries keep their path under data/audio/.
    #[command(group(
        ArgGroup::new("selector")
            .required(true)
            .args(["sound", "legacy_sound", "music", "name", "hash", "slot", "id", "all"])
    ))]
    Export {
        #[arg(long)]
        client: PathBuf,
        /// Audio.uop sound id (the sound table's MP3).
        #[arg(long)]
        sound: Option<u32>,
        /// Audio.uop sound id (the Classic sound's WAV under legacysounds/).
        #[arg(long)]
        legacy_sound: Option<u32>,
        /// Audio.uop music id.
        #[arg(long)]
        music: Option<u32>,
        /// Audio.uop entry name, such as data/audio/sounds/aaaenvir/d_frst01.mp3.
        #[arg(long)]
        name: Option<String>,
        /// Audio.uop entry hash, in hex.
        #[arg(long)]
        hash: Option<String>,
        /// Classic sound slot.
        #[arg(long)]
        slot: Option<u32>,
        /// Classic sound id, translated by Sound.def when it has no slot.
        #[arg(long)]
        id: Option<u32>,
        /// Every Audio.uop entry and Classic slot.
        #[arg(long)]
        all: bool,
        /// Hash dictionary (.dic) naming the entries the tables do not, for --all; repeat to merge several.
        #[arg(long)]
        dictionary: Vec<PathBuf>,
        /// Also write each file decoded to 16-bit PCM WAV.
        #[arg(long)]
        wav: bool,
        /// Output folder.
        #[arg(long)]
        output: PathBuf,
    },
    /// Write a copy of an Audio.uop with one entry replaced by a file. The entry keeps its compression flag; the
    /// file must be one the Enhanced Client's decoder (Miles 7.2c) plays (see --allow-unverified-format). The
    /// Enhanced Client never plays the legacysounds/ WAVs: replacing one changes nothing in it.
    #[command(group(ArgGroup::new("target").required(true).args(["sound", "legacy_sound", "music", "name"])))]
    Replace {
        #[command(flatten)]
        edit: EditArgs,
        /// Sound id (the server's): its MP3. For Kingdom Reborn, through audio_effects.csv (ID# = id + 1).
        #[arg(long)]
        sound: Option<u32>,
        /// Sound id: its Classic WAV under legacysounds/.
        #[arg(long)]
        legacy_sound: Option<u32>,
        /// Music id.
        #[arg(long)]
        music: Option<u32>,
        /// Entry name, such as data/audio/sounds/aaaenvir/d_frst01.mp3.
        #[arg(long)]
        name: Option<String>,
    },
    /// Write a copy of an Audio.uop with a new entry: MP3s stored, WAVs zlib-compressed, as the shipped ones. The
    /// client plays a sound only through a table row: a sound or music id without one gets it (--table-path).
    #[command(group(ArgGroup::new("target").required(true).args(["sound", "legacy_sound", "music", "name"])))]
    Add {
        #[command(flatten)]
        edit: EditArgs,
        /// Sound id (the server's): its MP3. For Kingdom Reborn, through audio_effects.csv (ID# = id + 1).
        #[arg(long)]
        sound: Option<u32>,
        /// Sound id: its Classic WAV under legacysounds/.
        #[arg(long)]
        legacy_sound: Option<u32>,
        /// Music id.
        #[arg(long)]
        music: Option<u32>,
        /// Entry name under data/audio/ (lowercase, `/` separators). An MP3 outside legacysounds/ must be one a
        /// table row already names (the client plays nothing else); add a new sound by id instead.
        #[arg(long, conflicts_with_all = ["table_path", "looping"])]
        name: Option<String>,
        /// The path of the new table row of an id without one: `Sounds\folder\file.mp3` for a sound (its MP3
        /// and its legacy WAV), `Music/file.mp3` for music. The Enhanced Client plays only the MP3, so a legacy
        /// sound takes no new row: add the MP3 with --sound first, then the WAV, which takes that row. For
        /// Kingdom Reborn, a server id without an audio_effects.csv row gets a sound structure in audio_sounds.csv
        /// (named as the shipped ones, `folder\file`, with their parameters) and its effects row (ID# = id + 1).
        #[arg(long)]
        table_path: Option<String>,
        /// A new music row loops.
        #[arg(long = "loop")]
        looping: bool,
    },
}

/// What `audio replace` and `audio add` share.
#[derive(clap::Args, Debug)]
pub struct EditArgs {
    /// The Audio.uop to read.
    #[arg(long)]
    uop: PathBuf,
    /// The Audio.uop to write: an absolute path, not the source.
    #[arg(long)]
    output: PathBuf,
    /// The file the entry will hold: MP3 for an .mp3 entry, WAV for a .wav one.
    #[arg(long)]
    input: PathBuf,
    /// Convert a file a WAV entry cannot take (another sample format, MP3) to 16-bit PCM WAV of its rate and
    /// channels. There is no MP3 encoder, and no resampling.
    #[arg(long)]
    convert: bool,
    /// Accept, with warnings, a file the Enhanced Client's decoder (Miles 7.2c) takes but whose playback was not
    /// verified (a resynchronised MP3 stream, a cut last frame, trailing bytes, PCM fields that disagree): for
    /// trying a file out in a test client, not for a release. A file Miles refuses, or of another kind than the
    /// entry's extension (a WAV for an .mp3), is refused all the same.
    #[arg(long)]
    allow_unverified_format: bool,
}

impl EditArgs {
    fn options(&self) -> AudioImportOptions {
        let policy = if self.allow_unverified_format { FormatPolicy::AllowUnverified } else { FormatPolicy::VerifiedOnly };
        AudioImportOptions { policy, convert: self.convert }
    }

    /// Loads the package and the file, applies `put`, writes the copy and reports.
    fn apply(&self, put: impl FnOnce(&mut AudioPackageEditor, &[u8], AudioImportOptions) -> eyre::Result<AudioPut>) -> eyre::Result<()> {
        eyre::ensure!(self.output.is_absolute(), "--output must be an absolute path: {}", self.output.display());
        let file = fs::read(&self.input).wrap_err_with(|| format!("Failed to read {}", self.input.display()))?;
        let mut editor = AudioPackageEditor::load(&self.uop)?;
        let result = put(&mut editor, &file, self.options())?;
        for warning in &result.warnings {
            log::warn!("{}: {warning}", result.name);
        }
        editor.save_as(&self.output)?;
        log::info!(
            "{:?} {} ({:016X}, {:?}{}){} in {}",
            result.write,
            result.name,
            result.hash,
            result.compression,
            if result.converted { ", converted to 16-bit PCM WAV" } else { "" },
            if result.tables_with_new_rows.is_empty() { String::new() } else { format!(", with a new row in {}", result.tables_with_new_rows.join(" and ")) },
            self.output.display()
        );
        Ok(())
    }
}

/// The entry kind and id of the `--sound`, `--legacy-sound` or `--music` given.
fn id_target(sound: Option<u32>, legacy_sound: Option<u32>, music: Option<u32>) -> Option<(AudioKind, u32)> {
    match (sound, legacy_sound, music) {
        (Some(id), ..) => Some((AudioKind::Sound, id)),
        (_, Some(id), _) => Some((AudioKind::LegacySound, id)),
        (.., Some(id)) => Some((AudioKind::Music, id)),
        _ => None,
    }
}

fn load_dictionaries(paths: &[PathBuf]) -> eyre::Result<HashDictionary> {
    let mut names = HashDictionary::new();
    for path in paths {
        names.merge(HashDictionary::load(path)?);
    }
    Ok(names)
}

pub fn run(cmd: AudioCmd) -> eyre::Result<()> {
    match cmd {
        AudioCmd::Census { clients, dictionary, output, tsv } => {
            let names = load_dictionaries(&dictionary)?;
            let mut census = AudioCensus::default();
            for client in &clients {
                census.extend(census_client(client, Some(&names))?);
            }
            fs::write(&output, census.summary_markdown()).wrap_err_with(|| format!("Failed to write {}", output.display()))?;
            if let Some(path) = tsv {
                let file = fs::File::create(&path).wrap_err_with(|| format!("Failed to create {}", path.display()))?;
                census.write_tsv(&mut BufWriter::new(file))?;
            }
            let failures = census.decode_failures().count();
            log::info!("{} audio entries, {failures} decode failures; summary in {}", census.rows.len(), output.display());
            Ok(())
        }
        AudioCmd::List { client, dictionary } => list(&client, &load_dictionaries(&dictionary)?),
        AudioCmd::Export { client, sound, legacy_sound, music, name, hash, slot, id, all, dictionary, wav, output } => {
            let selection = match (sound, legacy_sound, music, name, hash, slot, id) {
                (Some(id), ..) => Selection::Enhanced(AudioKind::Sound, id),
                (_, Some(id), ..) => Selection::Enhanced(AudioKind::LegacySound, id),
                (_, _, Some(id), ..) => Selection::Enhanced(AudioKind::Music, id),
                (_, _, _, Some(name), ..) => Selection::Name(name),
                (_, _, _, _, Some(hash), ..) => {
                    Selection::Hash(u64::from_str_radix(hash.trim_start_matches("0x"), 16).wrap_err_with(|| format!("Not a hex hash: {hash}"))?)
                }
                (.., Some(slot), _) => Selection::Slot(slot),
                (.., Some(id)) => Selection::ClassicId(id),
                _ if all => Selection::All(load_dictionaries(&dictionary)?),
                _ => unreachable!("clap requires one selector"),
            };
            export(&client, selection, wav, &output)
        }
        AudioCmd::Replace { edit, sound, legacy_sound, music, name } => {
            let target = match (id_target(sound, legacy_sound, music), name) {
                (Some((kind, id)), _) => AudioTarget::Id(kind, id),
                (None, Some(name)) => AudioTarget::Name(name),
                (None, None) => unreachable!("clap requires one target"),
            };
            edit.apply(|editor, file, options| editor.replace(&target, file, options))
        }
        AudioCmd::Add { edit, sound, legacy_sound, music, name, table_path, looping } => {
            let new = match (id_target(sound, legacy_sound, music), name) {
                (Some((kind, id)), _) => NewAudio::Id { kind, id, path: table_path, loops: looping.then_some(true) },
                (None, Some(name)) => NewAudio::Name(name),
                (None, None) => unreachable!("clap requires one target"),
            };
            edit.apply(|editor, file, options| editor.add(&new, file, options))
        }
    }
}

enum Selection {
    Enhanced(AudioKind, u32),
    Name(String),
    Hash(u64),
    Slot(u32),
    ClassicId(u32),
    All(HashDictionary),
}

fn list(client: &Path, names: &HashDictionary) -> eyre::Result<()> {
    if let Some(audio) = AudioPackage::load(client)? {
        println!("{}:", audio.path().display());
        if let AudioTables::Enhanced { sounds, music } = audio.tables() {
            for (kind, rows) in [(AudioKind::Sound, sounds), (AudioKind::Music, music)] {
                for row in rows.values() {
                    let kinds: &[AudioKind] = if kind == AudioKind::Sound { &[AudioKind::Sound, AudioKind::LegacySound] } else { &[kind] };
                    for &entry_kind in kinds {
                        let found = audio.entry_for(entry_kind, row.id)?;
                        let name = audio.entry_name_for(entry_kind, row.id).unwrap_or_default();
                        match found {
                            Some(entry) => println!("{entry_kind:?}\t{}\t{name}\t{}", row.id, describe(&entry.content)),
                            None => println!("{entry_kind:?}\t{}\t{name}\tno entry", row.id),
                        }
                    }
                }
            }
        } else {
            if let AudioTables::KingdomReborn(kr) = audio.tables() {
                for effect in kr.effects.iter().flat_map(|effects| effects.rows()) {
                    let Some(server_id) = effect.server_id() else { continue };
                    let name = audio.entry_name_for(AudioKind::Sound, server_id).unwrap_or_default();
                    let found = match audio.entry_for(AudioKind::Sound, server_id)? {
                        Some(entry) => describe(&entry.content),
                        None => "no entry".to_owned(),
                    };
                    println!("Sound\t{server_id}\tID# {}\t{}\t{name}\t{found}", effect.id, effect.structure);
                }
            }
            for hash in audio.hashes() {
                let name = audio.name_of(hash).or_else(|| names.resolve(hash)).unwrap_or_default();
                let entry = audio.entry_by_hash(hash)?.ok_or_else(|| eyre!("entry {hash:016X} vanished"))?;
                println!("{hash:016X}\t{name}\t{}", describe(&entry.content));
            }
        }
        for warning in audio.table_warnings() {
            eprintln!("warning: {warning}");
        }
    }
    if SoundMap::has_sound_files(client) {
        let sounds = SoundMap::load(client)?;
        println!("{}:", sounds.source_label());
        for slot in 0..sounds.slot_count() as u32 {
            if let Some(sound) = sounds.read_slot(slot)? {
                println!("slot\t{slot}\t{}\t{:.3} s", sound.name, sound.duration_seconds());
            }
        }
    }
    Ok(())
}

fn describe(content: &AudioContent) -> String {
    let duration = content.duration_seconds().map(|seconds| format!(", {seconds:.3} s")).unwrap_or_default();
    format!("{}: {}{duration}", content.format().label(), content.detail())
}

fn write_file(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, bytes).wrap_err_with(|| format!("Failed to write {}", path.display()))
}

/// `stem.decoded.wav` beside `path`.
fn decoded_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{stem}{DECODED_SUFFIX}.{WAV_EXTENSION}"))
}

/// Keeps letters, digits, `-`, `_` and `.`; anything else becomes `_`.
fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect()
}

fn export_entry(entry: &AudioEntry, wav: bool, output: &Path) -> eyre::Result<()> {
    let relative = match &entry.name {
        Some(name) => {
            let inner = name.strip_prefix(AUDIO_ENTRY_ROOT).unwrap_or(name);
            inner.split('/').filter(|part| !part.is_empty() && *part != "..").map(sanitize).collect::<PathBuf>()
        }
        None => {
            let extension = entry.content.format().extension().unwrap_or(RAW_EXTENSION);
            Path::new(UNNAMED_DIR).join(format!("{:016X}.{extension}", entry.hash))
        }
    };
    let path = output.join(relative);
    write_file(&path, &entry.payload)?;
    if wav && entry.content.format().is_decodable() {
        let decoded = decode(&entry.payload, &entry.content).wrap_err_with(|| format!("Failed to decode {}", path.display()))?;
        write_file(&decoded_path(&path), &decoded.to_wav())?;
    }
    Ok(())
}

fn export_classic(sound: &ClassicSound, wav: bool, output: &Path) -> eyre::Result<()> {
    let stem = format!("{:05}_{}", sound.slot_id, sanitize(&sound.name));
    let path = output.join(CLASSIC_DIR).join(format!("{stem}.{CLASSIC_SLOT_EXTENSION}"));
    write_file(&path, &sound.slot_bytes())?;
    if wav {
        write_file(&path.with_extension(WAV_EXTENSION), &decode_classic_pcm(&sound.pcm_data).to_wav())?;
    }
    Ok(())
}

fn export(client: &Path, selection: Selection, wav: bool, output: &Path) -> eyre::Result<()> {
    let enhanced = || -> eyre::Result<AudioPackage> {
        AudioPackage::load(client)?.ok_or_else(|| eyre!("No Audio.uop in {}", client.display()))
    };
    let classic = || SoundMap::load(client);
    let mut exported = 0usize;
    match selection {
        Selection::Enhanced(kind, id) => {
            let audio = enhanced()?;
            let entry = audio.entry_for(kind, id)?.ok_or_else(|| eyre!("No {kind:?} entry for id {id}"))?;
            export_entry(&entry, wav, output)?;
            exported += 1;
        }
        Selection::Name(name) => {
            let entry = enhanced()?.entry_by_name(&name)?.ok_or_else(|| eyre!("No entry named {name}"))?;
            export_entry(&entry, wav, output)?;
            exported += 1;
        }
        Selection::Hash(hash) => {
            let entry = enhanced()?.entry_by_hash(hash)?.ok_or_else(|| eyre!("No entry {hash:016X}"))?;
            export_entry(&entry, wav, output)?;
            exported += 1;
        }
        Selection::Slot(slot) => {
            let sound = classic()?.read_slot(slot)?.ok_or_else(|| eyre!("No sound at slot {slot}"))?;
            export_classic(&sound, wav, output)?;
            exported += 1;
        }
        Selection::ClassicId(id) => {
            let lookup = classic()?.read_id(id)?.ok_or_else(|| eyre!("No sound for id {id}"))?;
            export_classic(&lookup.sound, wav, output)?;
            exported += 1;
        }
        Selection::All(names) => {
            if let Some(audio) = AudioPackage::load(client)? {
                for hash in audio.hashes().collect::<Vec<_>>() {
                    let mut entry = audio.entry_by_hash(hash)?.ok_or_else(|| eyre!("entry {hash:016X} vanished"))?;
                    if entry.name.is_none() {
                        entry.name = names.resolve(hash).map(str::to_owned);
                    }
                    export_entry(&entry, wav, output)?;
                    exported += 1;
                }
            }
            if SoundMap::has_sound_files(client) {
                let sounds = classic()?;
                for slot in 0..sounds.slot_count() as u32 {
                    if let Some(sound) = sounds.read_slot(slot)? {
                        export_classic(&sound, wav, output)?;
                        exported += 1;
                    }
                }
            }
        }
    }
    log::info!("Exported {exported} audio files to {}", output.display());
    Ok(())
}
