//! Classic Client music: `Music/Digital/Config.txt`, the list of the MP3 files the music ids play, read strictly
//! ([`MusicConfig`]), and the files it names.
//!
//! The client's reading (`DigitalMusic_LoadConfig` and `DigitalMusic_Play` in UO Gold and 7.0.103;
//! `scripts/dev-helpers/ec-research/ghidra-audio/results/classic_audio.md`):
//!
//! - Lines are read with `fgets` of 512 bytes ([`CONFIG_MAX_LINE_BYTES`] before the line feed); trailing characters
//!   below `!` (spaces, tabs, CR) are trimmed; empty lines and lines starting with `#` are skipped.
//! - `strtok(" ")` splits the line at spaces (only spaces): the first token is the id, and every further token is
//!   stored for it, so the last one wins.
//! - That value is split at the first `,` into the name and the flag. The file is `Music/Digital/` and the name,
//!   with `.mp3` appended when its last 4 characters hold no `.`; any extension but `mp3` (any case) plays nothing.
//! - The music loops when the flag is exactly `loop` or `lop` ([`LOOP_FLAGS`], case-sensitive).
//!
//! The grammar read here:
//!
//! ```text
//! file  = *(line EOL) [line]          ; EOL is CRLF or LF; the last line may lack it
//! line  = "" | "#" *any | entry
//! entry = *SP id 1*(1*SP token) *WS   ; the last token is the value, the others are ignored by the client
//! id    = 1*DIGIT                     ; unique in the file
//! token = name ["," flag]
//! name  = 1*(any but SP, HTAB and ",")
//! flag  = 1*(any but SP and HTAB)     ; "loop" in every shipped line but one, `lop`, which loops too
//! WS    = SP | HTAB
//! ```
//!
//! Refused here: a tab inside a line (the client does not split at it), a name whose extension is not `mp3`, an id
//! that is not all digits (the client's `atol` would read its leading digits), and a second line of an id.
//! [`MusicConfig::client_notes`] lists the lines whose earlier tokens the client ignores.
//!
//! UO Gold's names have no extension (`turfin,loop`); the current client's have `.mp3`. UO Gold's line 10 reads
//! `9 britainpos,lop`.
//!
//! [`MusicConfig::to_text`] writes every line back as read (comments, blank lines, spacing and line ends too), so
//! an unchanged file comes back byte for byte; an entry set with [`MusicConfig::set`] is written `id name[,flag]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::utils::path::find_file_ignoring_case;

crate::eyre_imports!();

pub const MUSIC_DIR_NAME: &str = "Music";
pub const MUSIC_DIGITAL_DIR_NAME: &str = "Digital";
pub const MUSIC_CONFIG_FILE_NAME: &str = "Config.txt";
pub const MUSIC_EXTENSION: &str = ".mp3";
/// The flags that loop, compared exactly.
pub const LOOP_FLAGS: [&str; 2] = ["loop", "lop"];
/// The longest line `fgets` of 512 bytes reads whole, before its line feed.
pub const CONFIG_MAX_LINE_BYTES: usize = 510;
/// The characters [`MusicEntry::file_name`] looks for a `.` in.
const EXTENSION_WINDOW: usize = 4;
const FLAG_SEPARATOR: char = ',';
const TOKEN_SEPARATOR: char = ' ';
const COMMENT_MARK: char = '#';
/// Trailing characters below this are trimmed.
const FIRST_KEPT_CHAR: char = '!';

/// One line of `Config.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicEntry {
    pub id: u32,
    /// The name as written.
    pub name: String,
    /// The flag after the comma, as written.
    pub flag: Option<String>,
    /// The tokens before the last one, which the client ignores.
    pub ignored_tokens: Vec<String>,
    /// The 1-based line number (0 for an entry set with [`MusicConfig::set`]).
    pub line: usize,
    /// The line as read, written back while the entry is unchanged.
    raw: Option<String>,
}

impl MusicEntry {
    /// Whether the client loops it: the flag is exactly one of [`LOOP_FLAGS`].
    pub fn loops(&self) -> bool {
        self.flag.as_deref().is_some_and(|flag| LOOP_FLAGS.contains(&flag))
    }

    /// The file name in `Music/Digital`: the name, with `.mp3` added when its last 4 characters hold no `.`.
    pub fn file_name(&self) -> String {
        file_name_of(&self.name)
    }
}

/// One line of the file, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigLine {
    /// A blank or comment line, as read (the CR of a CRLF end too).
    Skipped(String),
    Entry(u32),
}

fn file_name_of(name: &str) -> String {
    let tail_start = name.char_indices().rev().nth(EXTENSION_WINDOW - 1).map_or(0, |(index, _)| index);
    if name[tail_start..].contains('.') { name.to_owned() } else { format!("{name}{MUSIC_EXTENSION}") }
}

/// `Music/Digital/Config.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicConfig {
    /// The `Music/Digital` folder.
    pub directory: PathBuf,
    entries: BTreeMap<u32, MusicEntry>,
    lines: Vec<ConfigLine>,
    /// Whether the lines end in CRLF (the shipped files' ends), as the first one does.
    crlf: bool,
}

impl Default for MusicConfig {
    fn default() -> Self {
        Self { directory: PathBuf::new(), entries: BTreeMap::new(), lines: Vec::new(), crlf: true }
    }
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
        let mut lines = Vec::new();
        let crlf = text.split_once('\n').is_none_or(|(first, _)| first.ends_with('\r'));
        for (index, raw) in text.split('\n').enumerate() {
            let line_number = index + 1;
            if raw.len() > CONFIG_MAX_LINE_BYTES {
                eyre::bail!(
                    "{source}:{line_number}:{}: the line has {} bytes; the client reads {CONFIG_MAX_LINE_BYTES} and the rest as \
                     another line",
                    CONFIG_MAX_LINE_BYTES + 1,
                    raw.len()
                );
            }
            let line = raw.trim_end_matches(|c: char| c < FIRST_KEPT_CHAR);
            if line.is_empty() || line.starts_with(COMMENT_MARK) {
                lines.push(ConfigLine::Skipped(raw.to_owned()));
                continue;
            }
            let mut entry = parse_line(line, line_number)
                .map_err(|(column, message)| eyre!("{source}:{line_number}:{column}: {message} (expected `id name[,flag]`)"))?;
            if let Some(previous) = entries.get(&entry.id) {
                eyre::bail!(
                    "{source}:{line_number}:1: music id {} is already defined on line {} (the client would take this line's name)",
                    entry.id,
                    previous.line
                );
            }
            entry.raw = Some(raw.to_owned());
            lines.push(ConfigLine::Entry(entry.id));
            entries.insert(entry.id, entry);
        }
        Ok(Self { directory: PathBuf::new(), entries, lines, crlf })
    }

    /// The text of the file: every line as read, an entry set since as `id name[,flag]`.
    pub fn to_text(&self) -> String {
        let lines: Vec<String> = self
            .lines
            .iter()
            .map(|line| match line {
                ConfigLine::Skipped(text) => text.clone(),
                ConfigLine::Entry(id) => {
                    let entry = &self.entries[id];
                    entry.raw.clone().unwrap_or_else(|| {
                        let flag = entry.flag.as_ref().map_or_else(String::new, |flag| format!("{FLAG_SEPARATOR}{flag}"));
                        let end = if self.crlf { "\r" } else { "" };
                        format!("{}{TOKEN_SEPARATOR}{}{flag}{end}", entry.id, entry.name)
                    })
                }
            })
            .collect();
        lines.join("\n")
    }

    /// Writes [`Self::to_text`] to `path`, through a temporary file in its folder.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let text = self.to_text();
        crate::utils::output_file::write_through_temp(path, |temp| {
            std::fs::write(temp, &text).wrap_err_with(|| format!("write {}", temp.display()))
        })
    }

    /// Sets the file of music `id`: in its line when it has one, else in a new line before the first entry of a
    /// greater id (after the last entry when none is greater). The name and the flag are checked as the parser
    /// checks them.
    pub fn set(&mut self, id: u32, name: &str, flag: Option<&str>) -> eyre::Result<()> {
        let value = match flag {
            Some(flag) => format!("{name}{FLAG_SEPARATOR}{flag}"),
            None => name.to_owned(),
        };
        if value.contains(TOKEN_SEPARATOR) {
            eyre::bail!("music id {id}: `{value}` holds a space, and the client splits the line at spaces");
        }
        let line = format!("{id}{TOKEN_SEPARATOR}{value}");
        let mut entry = parse_line(&line, 0).map_err(|(column, message)| eyre!("music id {id}: `{line}`:{column}: {message}"))?;
        if entry.name != name || entry.flag.as_deref() != flag {
            eyre::bail!("music id {id}: `{line}` reads back as name `{}` and flag {:?}", entry.name, entry.flag);
        }
        if line.len() > CONFIG_MAX_LINE_BYTES {
            eyre::bail!("music id {id}: the line has {} bytes; the client reads {CONFIG_MAX_LINE_BYTES}", line.len());
        }
        match self.entries.get(&id) {
            Some(previous) => entry.line = previous.line,
            None => {
                let position = self
                    .lines
                    .iter()
                    .position(|line| matches!(line, ConfigLine::Entry(other) if *other > id))
                    .or_else(|| self.lines.iter().rposition(|line| matches!(line, ConfigLine::Entry(_))).map(|last| last + 1))
                    .unwrap_or(self.lines.len());
                self.lines.insert(position, ConfigLine::Entry(id));
            }
        }
        self.entries.insert(id, entry);
        Ok(())
    }

    /// Removes the line of `id`; returns whether it had one.
    pub fn remove(&mut self, id: u32) -> bool {
        self.lines.retain(|line| *line != ConfigLine::Entry(id));
        self.entries.remove(&id).is_some()
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

    /// The lines whose earlier tokens the client ignores, one note each.
    pub fn client_notes(&self) -> Vec<String> {
        self.entries
            .values()
            .filter(|entry| !entry.ignored_tokens.is_empty())
            .map(|entry| {
                format!("line {}: music id {}: the client takes the last token and ignores {:?}", entry.line, entry.id, entry.ignored_tokens)
            })
            .collect()
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

fn parse_line(line: &str, line_number: usize) -> Result<MusicEntry, (usize, String)> {
    let column_of = |byte: usize| line[..byte].chars().count() + 1;
    let found = |byte: usize| match line[byte..].chars().next() {
        Some(c) => format!("`{c}`"),
        None => "the end of the line".to_owned(),
    };
    if let Some(tab) = line.find('\t') {
        return Err((column_of(tab), "a tab: the client splits the line at spaces only".to_owned()));
    }
    let id_start = line.find(|c: char| c != TOKEN_SEPARATOR).unwrap_or(line.len());
    let digits_end = line[id_start..].find(|c: char| !c.is_ascii_digit()).map_or(line.len(), |offset| id_start + offset);
    if digits_end == id_start {
        return Err((column_of(id_start), format!("expected the music id (decimal digits), found {}", found(id_start))));
    }
    let id = line[id_start..digits_end].parse().map_err(|_| (column_of(id_start), format!("music id `{}` does not fit 32 bits", &line[id_start..digits_end])))?;
    if !line[digits_end..].starts_with(TOKEN_SEPARATOR) {
        return Err((column_of(digits_end), format!("expected a space before the name, found {}", found(digits_end))));
    }
    // The tokens and their byte offsets.
    let mut tokens: Vec<(usize, &str)> = Vec::new();
    let mut position = digits_end;
    while position < line.len() {
        let start = line[position..].find(|c: char| c != TOKEN_SEPARATOR).map_or(line.len(), |offset| position + offset);
        let end = line[start..].find(TOKEN_SEPARATOR).map_or(line.len(), |offset| start + offset);
        if start < end {
            tokens.push((start, &line[start..end]));
        }
        position = end;
    }
    let Some(&(value_start, value)) = tokens.last() else {
        return Err((column_of(line.len()), "expected the file name, found the end of the line".to_owned()));
    };
    let (name, flag) = match value.split_once(FLAG_SEPARATOR) {
        Some((name, flag)) => (name, Some(flag)),
        None => (value, None),
    };
    if name.is_empty() {
        return Err((column_of(value_start), format!("expected the file name, found {}", found(value_start))));
    }
    if flag == Some("") {
        let flag_start = value_start + name.len() + FLAG_SEPARATOR.len_utf8();
        return Err((column_of(flag_start), format!("expected a flag after `{FLAG_SEPARATOR}`, found {}", found(flag_start))));
    }
    let file_name = file_name_of(name);
    let extension_ok = file_name.rsplit_once('.').is_some_and(|(_, extension)| format!(".{extension}").eq_ignore_ascii_case(MUSIC_EXTENSION));
    if !extension_ok {
        return Err((column_of(value_start), format!("`{file_name}`: the client plays {MUSIC_EXTENSION} files only")));
    }
    Ok(MusicEntry {
        id,
        name: name.to_owned(),
        flag: flag.map(str::to_owned),
        ignored_tokens: tokens[..tokens.len() - 1].iter().map(|(_, token)| (*token).to_owned()).collect(),
        line: line_number,
        raw: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_shipped_dialects_as_the_client_reads_them() {
        let config = MusicConfig::parse(
            "Config.txt",
            "0 turfin,loop\r\n9 britainpos,lop\r\n64 SelimsBar.mp3                 \r\n101 Song.mp3\r\n# note\r\n5 Loud,Loop\r\n6 old new,loop\r\n7 v1.final,loop",
        )
        .unwrap();
        assert_eq!(config.len(), 7);
        let turfin = config.get(0).unwrap();
        assert!(turfin.loops());
        assert_eq!(turfin.file_name(), "turfin.mp3");
        let typo = config.get(9).unwrap();
        assert_eq!(typo.flag.as_deref(), Some("lop"));
        assert!(typo.loops(), "the client loops `lop` too");
        assert!(!config.get(5).unwrap().loops(), "the flags are case-sensitive");
        let selim = config.get(64).unwrap();
        assert_eq!((selim.name.as_str(), selim.flag.as_ref()), ("SelimsBar.mp3", None));
        assert_eq!(config.get(101).unwrap().line, 4);
        let last = config.get(6).unwrap();
        assert_eq!((last.name.as_str(), last.ignored_tokens.as_slice()), ("new", &["old".to_owned()][..]));
        assert_eq!(config.get(7).unwrap().file_name(), "v1.final.mp3", "no `.` in the last 4 characters");
        let notes = config.client_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("line 7: music id 6: the client takes the last token"), "{notes:?}");
    }

    #[test]
    fn writes_every_line_back_and_edits_only_its_own() {
        let text = "# music\r\n0 turfin,loop\r\n\r\n9 britainpos,lop   \r\n64 SelimsBar.mp3\r\n";
        let mut config = MusicConfig::parse("Config.txt", text).unwrap();
        assert_eq!(config.to_text(), text);
        config.set(9, "britainpos", Some("loop")).unwrap();
        config.set(5, "new.mp3", None).unwrap();
        config.set(70, "last", Some("loop")).unwrap();
        assert!(config.remove(64));
        let edited = config.to_text();
        assert_eq!(edited, "# music\r\n0 turfin,loop\r\n\r\n5 new.mp3\r\n9 britainpos,loop\r\n70 last,loop\r\n");
        assert_eq!(MusicConfig::parse("Config.txt", &edited).unwrap().get(9).unwrap().flag.as_deref(), Some("loop"));
        let mut lf = MusicConfig::parse("Config.txt", "1 a\n").unwrap();
        lf.set(2, "b", None).unwrap();
        assert_eq!(lf.to_text(), "1 a\n2 b\n", "the new line ends as the first one does");
        assert!(config.set(1, "two words", None).is_err());
        assert!(config.set(1, "song.wav", None).is_err());
        assert!(config.set(1, "song", Some("")).is_err());
        assert!(config.set(1, "", Some("loop")).is_err());
    }

    #[test]
    fn rejects_malformed_lines() {
        let error = |text: &str| MusicConfig::parse("Config.txt", text).unwrap_err().to_string();
        assert!(error("x turfin").contains("Config.txt:1:1: expected the music id"));
        assert!(error("0turfin").contains("1:2: expected a space"));
        assert!(error("0 turfin,").contains("1:10: expected a flag"));
        assert!(error("0 ,loop").contains("1:3: expected the file name"));
        assert!(error("0\tturfin").contains("1:2: a tab"), "{}", error("0\tturfin"));
        assert!(error("0 song.wav").contains("1:3: `song.wav`: the client plays .mp3 files only"), "{}", error("0 song.wav"));
        assert!(error("0 a\n0 b").contains("already defined on line 1"));
        let long = format!("0 {}.mp3", "x".repeat(CONFIG_MAX_LINE_BYTES));
        assert!(error(&long).contains("1:511: the line has"), "{}", error(&long));
    }
}
