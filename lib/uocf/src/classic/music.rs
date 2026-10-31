//! Classic Client music: `Music/Digital/Config.txt`, the list of the MP3 files the music ids play, read strictly
//! ([`MusicConfig`]), and the files it names.
//!
//! The grammar, as UO Gold and the current client follow it:
//!
//! ```text
//! file  = *(line EOL) [line]          ; EOL is CRLF or LF; the last line may lack it
//! line  = "" | entry
//! entry = id WS name ["," flag] [WS]
//! id    = 1*DIGIT                     ; unique in the file
//! name  = 1*(any but WS and ",")      ; the file in Music/Digital, ".mp3" added when it has no extension
//! flag  = 1*(any but WS)              ; "loop" in every shipped line but one
//! WS    = 1*(SP | HTAB)
//! ```
//!
//! UO Gold's names have no extension (`turfin,loop`); the current client's have `.mp3`. UO Gold's line 10 reads
//! `9 britainpos,lop`: the flag is kept as written, and only `loop` (ignoring case) counts as looping.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

pub const MUSIC_DIR_NAME: &str = "Music";
pub const MUSIC_DIGITAL_DIR_NAME: &str = "Digital";
pub const MUSIC_CONFIG_FILE_NAME: &str = "Config.txt";
pub const MUSIC_EXTENSION: &str = ".mp3";
pub const LOOP_FLAG: &str = "loop";
const FLAG_SEPARATOR: char = ',';

/// One line of `Config.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicEntry {
    pub id: u32,
    /// The name as written.
    pub name: String,
    /// The flag after the comma, as written.
    pub flag: Option<String>,
    /// The 1-based line number.
    pub line: usize,
}

impl MusicEntry {
    pub fn loops(&self) -> bool {
        self.flag.as_deref().is_some_and(|flag| flag.eq_ignore_ascii_case(LOOP_FLAG))
    }

    /// The file name in `Music/Digital`: the name, with `.mp3` added when it has no extension.
    pub fn file_name(&self) -> String {
        if self.name.contains('.') { self.name.clone() } else { format!("{}{MUSIC_EXTENSION}", self.name) }
    }
}

/// `Music/Digital/Config.txt`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MusicConfig {
    /// The `Music/Digital` folder.
    pub directory: PathBuf,
    entries: BTreeMap<u32, MusicEntry>,
}

impl MusicConfig {
    /// `Music/Digital/Config.txt` of a client folder (names matched ignoring case); `Ok(None)` when absent.
    pub fn load(client_path: &Path) -> eyre::Result<Option<Self>> {
        let Some(music) = find_dir_ignoring_case(client_path, MUSIC_DIR_NAME) else {
            return Ok(None);
        };
        let Some(directory) = find_dir_ignoring_case(&music, MUSIC_DIGITAL_DIR_NAME) else {
            return Ok(None);
        };
        let Some(path) = find_file_ignoring_case(&directory, MUSIC_CONFIG_FILE_NAME) else {
            return Ok(None);
        };
        let text = std::fs::read_to_string(&path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
        let mut config = Self::parse(&path.display().to_string(), &text)?;
        config.directory = directory;
        Ok(Some(config))
    }

    /// Parses `text`; errors name `source`, the line and the column.
    pub fn parse(source: &str, text: &str) -> eyre::Result<Self> {
        let mut entries: BTreeMap<u32, MusicEntry> = BTreeMap::new();
        for (index, raw) in text.split('\n').enumerate() {
            let line_number = index + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() {
                continue;
            }
            let entry = parse_line(line, line_number)
                .map_err(|(column, message)| eyre!("{source}:{line_number}:{column}: {message} (expected `id name[,flag]`)"))?;
            if let Some(previous) = entries.get(&entry.id) {
                eyre::bail!("{source}:{line_number}:1: music id {} is already defined on line {}", entry.id, previous.line);
            }
            entries.insert(entry.id, entry);
        }
        Ok(Self { directory: PathBuf::new(), entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: u32) -> Option<&MusicEntry> {
        self.entries.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &MusicEntry> {
        self.entries.values()
    }

    /// The file an entry names, when it exists (matched ignoring case).
    pub fn file_path(&self, entry: &MusicEntry) -> Option<PathBuf> {
        find_file_ignoring_case(&self.directory, &entry.file_name())
    }
}

/// The folder `name` in `dir`, matched ignoring ASCII case.
pub fn find_dir_ignoring_case(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_str().is_some_and(|entry_name| entry_name.eq_ignore_ascii_case(name)))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn parse_line(line: &str, line_number: usize) -> Result<MusicEntry, (usize, String)> {
    let column_of = |byte: usize| line[..byte].chars().count() + 1;
    let found = |byte: usize| match line[byte..].chars().next() {
        Some(c) => format!("`{c}`"),
        None => "the end of the line".to_owned(),
    };
    let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(line.len());
    if digits_end == 0 {
        return Err((1, format!("expected the music id (decimal digits), found {}", found(0))));
    }
    let id = line[..digits_end].parse().map_err(|_| (1, format!("music id `{}` does not fit 32 bits", &line[..digits_end])))?;
    let name_start = line[digits_end..].find(|c: char| !is_space(c)).map_or(line.len(), |offset| digits_end + offset);
    if name_start == digits_end {
        return Err((column_of(digits_end), format!("expected white space before the name, found {}", found(digits_end))));
    }
    let name_end = line[name_start..].find(|c: char| is_space(c) || c == FLAG_SEPARATOR).map_or(line.len(), |offset| name_start + offset);
    if name_end == name_start {
        return Err((column_of(name_start), format!("expected the file name, found {}", found(name_start))));
    }
    let mut position = name_end;
    let mut flag = None;
    if line[position..].starts_with(FLAG_SEPARATOR) {
        let flag_start = position + FLAG_SEPARATOR.len_utf8();
        let flag_end = line[flag_start..].find(is_space).map_or(line.len(), |offset| flag_start + offset);
        if flag_end == flag_start {
            return Err((column_of(flag_start), format!("expected a flag after `{FLAG_SEPARATOR}`, found {}", found(flag_start))));
        }
        flag = Some(line[flag_start..flag_end].to_owned());
        position = flag_end;
    }
    let rest_start = line[position..].find(|c: char| !is_space(c)).map(|offset| position + offset);
    if let Some(rest_start) = rest_start {
        return Err((column_of(rest_start), format!("expected the end of the line, found {}", found(rest_start))));
    }
    Ok(MusicEntry { id, name: line[name_start..name_end].to_owned(), flag, line: line_number })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_shipped_dialects() {
        let config =
            MusicConfig::parse("Config.txt", "0 turfin,loop\r\n9 britainpos,lop\r\n64 SelimsBar.mp3                 \r\n101 Song.mp3")
                .unwrap();
        assert_eq!(config.len(), 4);
        let turfin = config.get(0).unwrap();
        assert!(turfin.loops());
        assert_eq!(turfin.file_name(), "turfin.mp3");
        let typo = config.get(9).unwrap();
        assert_eq!(typo.flag.as_deref(), Some("lop"));
        assert!(!typo.loops());
        let selim = config.get(64).unwrap();
        assert_eq!((selim.name.as_str(), selim.flag.as_ref()), ("SelimsBar.mp3", None));
        assert_eq!(config.get(101).unwrap().line, 4);
    }

    #[test]
    fn rejects_malformed_lines() {
        let error = |text: &str| MusicConfig::parse("Config.txt", text).unwrap_err().to_string();
        assert!(error("x turfin").contains("Config.txt:1:1: expected the music id"));
        assert!(error("0turfin").contains("1:2: expected white space"));
        assert!(error("0 turfin,").contains("1:10: expected a flag"));
        assert!(error("0 two words").contains("1:7: expected the end of the line"));
        assert!(error("0 a\n0 b").contains("already defined on line 1"));
    }
}
