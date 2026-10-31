//! `Audio.uop` of the Enhanced Client (and of Kingdom Reborn, by name only): the sound and music tables and the
//! entries they name ([`AudioPackage`]).
//!
//! The package holds two tables, `data/audio/audio_sounds.csv` (`#ID,FilePath`) and `data/audio/audio_music.csv`
//! (`#ID,FilePath,Loop`), and the files. An entry's name is `data/audio/` and the table's path lowercased, with
//! `\` turned into `/`: sound 0, `Sounds\aaaenvir\d_frst01.mp3`, is `data/audio/sounds/aaaenvir/d_frst01.mp3`. Most
//! sounds also have the Classic sound of the same id as a WAV file, under `data/audio/legacysounds/` with the
//! `sounds/` folder dropped and `.wav` for `.mp3`: `data/audio/legacysounds/aaaenvir/d_frst01.wav`. Sound ids are the
//! Classic ids, the slots of `soundLegacyMUL.uop`. A few table paths name no entry (`flame04 .mp3`, the `pirates_`
//! rows); `docs/agent_notes/ec_audio_formats.md` lists them.
//!
//! Kingdom Reborn's tables have the same names but another layout (`w,s,id,filename,rate,...` rows, `;` comments,
//! music ids from 10000): [`AudioTables::KingdomReborn`] keeps them unread, and its entries are reached by name.
//!
//! Entries keep their stored bytes and compression flag ([`AudioEntry::stored`]), so a replacement can be written
//! with the original flag.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::audio::{probe_audio, AudioContent};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};
use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

pub const AUDIO_UOP_FILE_NAME: &str = "Audio.uop";
pub const SOUND_TABLE_ENTRY: &str = "data/audio/audio_sounds.csv";
pub const MUSIC_TABLE_ENTRY: &str = "data/audio/audio_music.csv";
pub const SOUND_TABLE_HEADER: &str = "#ID,FilePath";
pub const MUSIC_TABLE_HEADER: &str = "#ID,FilePath,Loop";
/// What every entry name starts with.
pub const AUDIO_ENTRY_ROOT: &str = "data/audio/";
/// The folder of the sound table's paths, lowercased.
pub const SOUNDS_FOLDER: &str = "sounds/";
pub const LEGACY_SOUNDS_ROOT: &str = "data/audio/legacysounds/";
pub const LEGACY_SOUND_EXTENSION: &str = "wav";
/// How Kingdom Reborn's tables start: the sound table's header, or the comment row of the music table.
const KR_SOUND_HEADER_START: &str = "w,s,id,";
const KR_COMMENT_MARK: char = ';';
const FIELD_SEPARATOR: char = ',';
const LOOP_OFF: &str = "0";
const LOOP_ON: &str = "1";

/// The entry name a table path stands for: [`AUDIO_ENTRY_ROOT`] and the path lowercased with `/` separators.
pub fn table_path_entry_name(path: &str) -> String {
    format!("{AUDIO_ENTRY_ROOT}{}", path.replace('\\', "/").to_ascii_lowercase())
}

/// The Classic sound's WAV beside a sound table path: under [`LEGACY_SOUNDS_ROOT`], without the `sounds/`
/// folder, with the `.wav` extension. `None` for a path outside `Sounds\`.
pub fn legacy_sound_entry_name(path: &str) -> Option<String> {
    let relative = path.replace('\\', "/").to_ascii_lowercase();
    let inner = relative.strip_prefix(SOUNDS_FOLDER)?;
    let stem = inner.rsplit_once('.').map_or(inner, |(stem, _)| stem);
    Some(format!("{LEGACY_SOUNDS_ROOT}{stem}.{LEGACY_SOUND_EXTENSION}"))
}

/// One row of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTableRow {
    pub id: u32,
    /// The path as written (`Sounds\aaaenvir\d_frst01.mp3`, `Music/turfin.mp3`).
    pub path: String,
    /// The music table's `Loop` field.
    pub loops: Option<bool>,
    /// The 1-based line number.
    pub line: usize,
}

impl AudioTableRow {
    pub fn entry_name(&self) -> String {
        table_path_entry_name(&self.path)
    }
}

/// The tables of an `Audio.uop`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioTables {
    Enhanced { sounds: BTreeMap<u32, AudioTableRow>, music: BTreeMap<u32, AudioTableRow> },
    /// Kingdom Reborn's layout, not read.
    KingdomReborn,
    /// The package has no sound table.
    Missing,
}

/// Parses an Enhanced Client table: the `header` line, then `id,path` rows (`id,path,0|1` with `with_loop`), CRLF
/// or LF, empty lines skipped. Errors name `source`, the line and the column.
pub fn parse_table(source: &str, text: &str, header: &str, with_loop: bool) -> eyre::Result<BTreeMap<u32, AudioTableRow>> {
    let mut lines = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).enumerate();
    match lines.next() {
        Some((_, first)) if first == header => {}
        Some((_, first)) => eyre::bail!("{source}:1:1: expected the header `{header}`, found `{first}`"),
        None => eyre::bail!("{source}: empty table"),
    }
    let expected = if with_loop { "`id,path,0|1`" } else { "`id,path`" };
    let mut rows: BTreeMap<u32, AudioTableRow> = BTreeMap::new();
    for (index, line) in lines {
        let line_number = index + 1;
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
        let field_count = if with_loop { 3 } else { 2 };
        if fields.len() != field_count {
            eyre::bail!("{source}:{line_number}:1: {} fields, expected {expected}", fields.len());
        }
        let id = fields[0]
            .parse::<u32>()
            .ok()
            .filter(|_| fields[0].bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| eyre!("{source}:{line_number}:1: expected the id (decimal digits), found `{}`", fields[0]))?;
        let path_column = fields[0].len() + 2;
        if fields[1].is_empty() {
            eyre::bail!("{source}:{line_number}:{path_column}: expected a path");
        }
        let loops = if with_loop {
            let loop_column = path_column + fields[1].len() + 1;
            Some(match fields[2] {
                LOOP_OFF => false,
                LOOP_ON => true,
                other => eyre::bail!("{source}:{line_number}:{loop_column}: expected the loop flag 0 or 1, found `{other}`"),
            })
        } else {
            None
        };
        if let Some(previous) = rows.get(&id) {
            eyre::bail!("{source}:{line_number}:1: id {id} is already defined on line {}", previous.line);
        }
        rows.insert(id, AudioTableRow { id, path: fields[1].to_owned(), loops, line: line_number });
    }
    Ok(rows)
}

fn is_kingdom_reborn_table(text: &str) -> bool {
    let first = text.lines().next().unwrap_or_default();
    first.starts_with(KR_SOUND_HEADER_START) || first.starts_with(KR_COMMENT_MARK)
}

/// What an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioKind {
    /// The sound table's file (MP3).
    Sound,
    /// The Classic sound's WAV under `legacysounds/`.
    LegacySound,
    Music,
}

/// One package entry.
#[derive(Debug, Clone)]
pub struct AudioEntry {
    pub hash: u64,
    pub name: Option<String>,
    pub compression: CompressionFlag,
    /// The bytes as stored in the package (compressed when `compression` says so).
    pub stored: Arc<[u8]>,
    /// The file.
    pub payload: Arc<[u8]>,
    pub content: AudioContent,
}

/// An `Audio.uop`, loaded lazily.
pub struct AudioPackage {
    path: PathBuf,
    package: UopPackage,
    tables: AudioTables,
    /// The names the tables give, by hash.
    names: HashMap<u64, String>,
}

impl AudioPackage {
    /// `Audio.uop` of a client folder (matched ignoring case); `Ok(None)` when absent.
    pub fn load(client_path: &Path) -> eyre::Result<Option<Self>> {
        find_file_ignoring_case(client_path, AUDIO_UOP_FILE_NAME).map(|path| Self::load_file(&path)).transpose()
    }

    pub fn load_file(path: &Path) -> eyre::Result<Self> {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy).wrap_err_with(|| format!("Failed to load {}", path.display()))?;
        let read_table = |name: &str| -> eyre::Result<Option<String>> {
            let Some(bytes) = package.unpack_file_by_hash(hash_file_name_single(name))? else {
                return Ok(None);
            };
            String::from_utf8(bytes).map(Some).map_err(|_| eyre!("{}:{name}: not UTF-8", path.display()))
        };
        let source = |name: &str| format!("{}:{name}", path.display());
        let tables = match read_table(SOUND_TABLE_ENTRY)? {
            None => AudioTables::Missing,
            Some(text) if is_kingdom_reborn_table(&text) => AudioTables::KingdomReborn,
            Some(text) => {
                let sounds = parse_table(&source(SOUND_TABLE_ENTRY), &text, SOUND_TABLE_HEADER, false)?;
                let music = match read_table(MUSIC_TABLE_ENTRY)? {
                    Some(text) => parse_table(&source(MUSIC_TABLE_ENTRY), &text, MUSIC_TABLE_HEADER, true)?,
                    None => BTreeMap::new(),
                };
                AudioTables::Enhanced { sounds, music }
            }
        };
        let mut names = HashMap::new();
        for name in [SOUND_TABLE_ENTRY, MUSIC_TABLE_ENTRY] {
            names.insert(hash_file_name_single(name), name.to_owned());
        }
        if let AudioTables::Enhanced { sounds, music } = &tables {
            let derived = sounds
                .values()
                .flat_map(|row| [Some(row.entry_name()), legacy_sound_entry_name(&row.path)])
                .chain(music.values().map(|row| Some(row.entry_name())))
                .flatten();
            for name in derived {
                let hash = hash_file_name_single(&name);
                if package.get_file_by_hash(hash).is_some() {
                    names.insert(hash, name);
                }
            }
        }
        Ok(Self { path: path.to_path_buf(), package, tables, names })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    pub fn tables(&self) -> &AudioTables {
        &self.tables
    }

    /// The names the tables give to entries of the package, by hash.
    pub fn known_names(&self) -> &HashMap<u64, String> {
        &self.names
    }

    pub fn name_of(&self, hash: u64) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    /// Every entry hash, in package order.
    pub fn hashes(&self) -> impl Iterator<Item = u64> + '_ {
        self.package.iter_files().filter(|file| file.has_size()).map(|file| file.filename_hash())
    }

    pub fn sound_row(&self, id: u32) -> Option<&AudioTableRow> {
        match &self.tables {
            AudioTables::Enhanced { sounds, .. } => sounds.get(&id),
            _ => None,
        }
    }

    pub fn music_row(&self, id: u32) -> Option<&AudioTableRow> {
        match &self.tables {
            AudioTables::Enhanced { music, .. } => music.get(&id),
            _ => None,
        }
    }

    /// The entry name an id stands for, from the tables.
    pub fn entry_name_for(&self, kind: AudioKind, id: u32) -> Option<String> {
        match kind {
            AudioKind::Sound => self.sound_row(id).map(AudioTableRow::entry_name),
            AudioKind::LegacySound => self.sound_row(id).and_then(|row| legacy_sound_entry_name(&row.path)),
            AudioKind::Music => self.music_row(id).map(AudioTableRow::entry_name),
        }
    }

    /// The entry an id names; `Ok(None)` when the table has no row or the row names no entry.
    pub fn entry_for(&self, kind: AudioKind, id: u32) -> eyre::Result<Option<AudioEntry>> {
        match self.entry_name_for(kind, id) {
            Some(name) => self.entry_by_name(&name),
            None => Ok(None),
        }
    }

    pub fn entry_by_name(&self, name: &str) -> eyre::Result<Option<AudioEntry>> {
        let mut entry = self.entry_by_hash(hash_file_name_single(name))?;
        if let Some(entry) = &mut entry {
            entry.name = Some(name.to_owned());
        }
        Ok(entry)
    }

    /// The entry of `hash`, read from the package: its stored bytes, payload and content.
    pub fn entry_by_hash(&self, hash: u64) -> eyre::Result<Option<AudioEntry>> {
        let Some(file) = self.package.get_file_by_hash(hash) else {
            return Ok(None);
        };
        let mut file = file.clone();
        if file.data().is_none() && file.has_size() {
            let mut reader = self.package.open_payload_reader()?;
            file.load_data_from(&mut reader).wrap_err_with(|| format!("Failed to read entry {hash:016X} of {}", self.path.display()))?;
        }
        let stored = file.data().cloned().unwrap_or_else(|| Arc::from(Vec::new()));
        let payload = file.unpack_arc().wrap_err_with(|| format!("Failed to unpack entry {hash:016X} of {}", self.path.display()))?;
        let content = probe_audio(&payload);
        Ok(Some(AudioEntry {
            hash,
            name: self.name_of(hash).map(str::to_owned),
            compression: file.compression(),
            stored,
            payload,
            content,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_entry_names_from_table_paths() {
        assert_eq!(table_path_entry_name(r"Sounds\aaaenvir\d_frst01.mp3"), "data/audio/sounds/aaaenvir/d_frst01.mp3");
        assert_eq!(table_path_entry_name("Music/TribalWar.mp3"), "data/audio/music/tribalwar.mp3");
        assert_eq!(legacy_sound_entry_name(r"Sounds\aaaenvir\d_frst01.mp3").as_deref(), Some("data/audio/legacysounds/aaaenvir/d_frst01.wav"));
        assert_eq!(legacy_sound_entry_name("Music/turfin.mp3"), None);
    }

    #[test]
    fn parses_the_enhanced_tables_strictly() {
        let sounds = parse_table("s.csv", "#ID,FilePath\r\n0,Sounds\\a\\b.mp3\r\n7,Sounds\\x.mp3\r\n", SOUND_TABLE_HEADER, false).unwrap();
        assert_eq!(sounds.len(), 2);
        assert_eq!(sounds[&7].line, 3);
        let music = parse_table("m.csv", "#ID,FilePath,Loop\r\n0,Music/turfin.mp3,1\r\n1,Music/x.mp3,0", MUSIC_TABLE_HEADER, true).unwrap();
        assert_eq!((music[&0].loops, music[&1].loops), (Some(true), Some(false)));

        let error = |text: &str, header: &str, with_loop: bool| parse_table("t.csv", text, header, with_loop).unwrap_err().to_string();
        assert!(error("#ID,Path\n", SOUND_TABLE_HEADER, false).contains("t.csv:1:1: expected the header"));
        assert!(error("#ID,FilePath\nx,a.mp3\n", SOUND_TABLE_HEADER, false).contains("t.csv:2:1: expected the id"));
        assert!(error("#ID,FilePath\n1,a,b\n", SOUND_TABLE_HEADER, false).contains("3 fields"));
        assert!(error("#ID,FilePath,Loop\n1,a.mp3,2\n", MUSIC_TABLE_HEADER, true).contains("2:9: expected the loop flag"));
        assert!(error("#ID,FilePath\n1,a\n1,b\n", SOUND_TABLE_HEADER, false).contains("already defined on line 2"));
        assert!(is_kingdom_reborn_table("w,s,id,filename,rate\n"));
        assert!(is_kingdom_reborn_table(";,,Music start ID: 10000\n"));
        assert!(!is_kingdom_reborn_table("#ID,FilePath\r\n"));
    }
}
