//! Kingdom Reborn's sound tables in `Audio.uop`, read the way UOKR.exe reads them and strictly
//! (`scripts/dev-helpers/ec-research/ghidra-audio/results/uokr_audio.md`):
//!
//! - `data/audio/audio_sounds.csv` ([`KrSoundTable`], `MythAudio::ParseSoundsCsv`): MythAudio sound structures.
//! - `data/audio/audio_effects.csv` ([`KrEffectsTable`], `UOAudio::LoadEffectsTable`): `ID#` and a structure
//!   name. `ID#` is the server's sound id + 1 ([`KR_EFFECT_ID_OFFSET`]: "Mythic code assumes ID 0 to be invalid");
//!   the Enhanced Client has no such offset. A structure starting with a digit is a structure handle
//!   ([`KrEffectRow::structure_handle`]): `0x10000 | n` is the n-th sound structure the client accepted
//!   ([`KrSoundTable::sound_by_id`]). [`KrAudioTables::sound_for_server_id`] follows a server id to the structure
//!   and its file.
//!
//! Both are read with the client's CSV reader: a line ends at CR, LF or CRLF and holds at most
//! [`KR_MAX_LINE_CHARS`] characters; fields are split at commas, a `"` toggles quoting and is dropped, nothing is
//! trimmed. Here an unclosed `"`, a longer line or a blank line is refused.
//!
//! `audio_sounds.csv` rows, by their first field:
//!
//! - `!`: a section. The next field, upper-cased, selects it: `POLYGROUPS`, `ROLLOFF FACTOR`, `DEFAULT PARAMETER`,
//!   any other (the shipped `MASTER VOLUME`, `RATE PRESETS` and `SOUND DEFAULT PARAMETER VALUES`).
//! - `;`: a comment.
//! - `#`: a polygroup row, a rolloff row or a sound entry in the first three sections; skipped by the client in the
//!   others (so the shipped `s_default` row, in `SOUND DEFAULT PARAMETER VALUES`, is not read).
//! - empty or starting with a space: a sound entry.
//! - anything else: logged by the client ("invalid character found at beginning of line definition") and skipped;
//!   the header row `w,s,id,...` is one.
//!
//! A sound entry has [`KR_SOUND_FIELDS`] fields after the first ([`KrSoundRow`]), then any trailing ones (status,
//! usage, notes). Its name and file name hold at most [`KR_MAX_NAME_CHARS`] characters. Range fields (rate,
//! volume, pan, offset) are read as the client does: white space removed, `%`, `-` and `*` turned into spaces,
//! trimmed; empty is the default, else one value or `low high` ([`KrRange`]). A `-` before a value is refused here:
//! the client would read `-5` as 5. `*` or an empty field is the default of the distances, the integers and the
//! polygroup. A second structure of a name is skipped by the client ([`KrSoundRow::duplicate_of`]).
//!
//! The file is the CSV folder and the file name, `data/audio/` + the name with `\` turned into `/` and lowercased
//! ([`KrSoundRow::entry_name`]), as for the Enhanced Client's tables.
//!
//! [`KrSoundTable::to_text`] and [`KrEffectsTable::to_text`] write every field as read, quotes included, with each
//! line's own line end: the shipped tables come back byte for byte.
//!
//! [`KrAudioTables::with_sound`] adds a playable sound: a structure row at the end of `audio_sounds.csv`, named and
//! set as the shipped sound effects are ([`kr_structure_name`], [`KR_NEW_SOUND_PARAMETERS`]), and an
//! `audio_effects.csv` row of `ID#` server id + 1 naming it. Each table gets one line, ended like its first one, and
//! is read again with the rules above, so the client's limits (names, structures per file, line length) hold.

use std::collections::HashMap;
use std::str::FromStr;

use crate::enhanced::audio::{check_table_path, table_path_entry_name, SOUND_PATH_ROOT, TABLE_PATH_EXTENSION};

crate::eyre_imports!();

pub const KR_EFFECTS_TABLE_ENTRY: &str = "data/audio/audio_effects.csv";
/// The other MythAudio tables of Kingdom Reborn's `Audio.uop`, kept unread.
pub const KR_OTHER_TABLE_ENTRIES: [&str; 4] =
    ["data/audio/audio_groups.csv", "data/audio/audio_queues.csv", "data/audio/audio_loops.csv", "data/audio/audio_music.csv"];
/// `audio_effects.csv`'s `ID#` is the server's sound id plus this.
pub const KR_EFFECT_ID_OFFSET: u32 = 1;
/// The longest line the client's CSV reader holds.
pub const KR_MAX_LINE_CHARS: usize = 0x7fe;
/// The longest sound structure name and file name.
pub const KR_MAX_NAME_CHARS: usize = 0x50;
/// The most sound structure names the client takes.
pub const KR_MAX_SOUND_NAMES: usize = 4000;
/// The most structures one file may serve (it is loaded once).
pub const KR_MAX_STRUCTURES_PER_FILE: usize = 30;
/// The fields of a sound entry after the first.
pub const KR_SOUND_FIELDS: usize = 14;
/// The sound entry that sets the default parameters: it is no structure and gets no id.
pub const KR_DEFAULT_SOUND_NAME: &str = "s_default";
/// A structure handle is a kind bit over a 16-bit id; this kind is the sound structures'
/// (`MythAudio::FindStructureHandle`).
pub const KR_SOUND_HANDLE_KIND: u32 = 0x1_0000;
pub const KR_HANDLE_ID_MASK: u32 = 0xffff;
/// The first id the client gives a sound structure.
pub const KR_FIRST_SOUND_ID: u32 = 1;
/// The fields after the name and file name of a structure added here: those of 1,488 of the 1,561 shipped
/// structures (a rate drawn from 1.0-1.1, mono, the rest the defaults; the type field before the name is empty, a
/// sample).
pub const KR_NEW_SOUND_PARAMETERS: &str = "1.0-1.1,*,*,*,*,*,*,1,*,*,*";
/// The line end of a line added to a table without lines (the shipped tables end their lines with CRLF).
const DEFAULT_LINE_END: &str = "\r\n";
const FIELD_SEPARATOR: char = ',';
const QUOTE: char = '"';
const DIRECTIVE_MARK: char = '!';
const COMMENT_MARK: char = ';';
const ROW_MARK: char = '#';
const SPACE: char = ' ';
const DEFAULT_MARK: &str = "*";
/// The characters a range field turns into spaces.
const RANGE_SPACES: [char; 3] = ['%', '-', '*'];
const NEGATIVE_SIGN: char = '-';
const POLYGROUPS_SECTION: &str = "POLYGROUPS";
const ROLLOFF_SECTION: &str = "ROLLOFF FACTOR";
const DEFAULT_PARAMETER_SECTION: &str = "DEFAULT PARAMETER";

/// One field as written (quotes included) and as the client reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvField {
    pub raw: String,
    pub value: String,
}

/// One line, split the way the client's reader splits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvLine {
    /// 1-based.
    pub line: usize,
    pub fields: Vec<CsvField>,
    /// `\r\n`, `\n`, `\r`, or empty for a last line without one.
    pub line_end: String,
}

impl CsvLine {
    /// The value of field `index`, empty past the last.
    pub fn value(&self, index: usize) -> &str {
        self.fields.get(index).map_or("", |field| field.value.as_str())
    }

    fn to_text(&self) -> String {
        let fields: Vec<&str> = self.fields.iter().map(|field| field.raw.as_str()).collect();
        format!("{}{}", fields.join(&FIELD_SEPARATOR.to_string()), self.line_end)
    }
}

/// The text of `lines` with `row` added as a last line, ended like the first line; a last line without a line end
/// gets one.
fn text_with_line<'a>(lines: impl IntoIterator<Item = &'a CsvLine>, row: &str) -> String {
    let mut text = String::new();
    let mut line_end = None;
    for line in lines {
        line_end.get_or_insert_with(|| line.line_end.clone());
        text.push_str(&line.to_text());
    }
    let line_end = line_end.filter(|end| !end.is_empty()).unwrap_or_else(|| DEFAULT_LINE_END.to_owned());
    if !text.is_empty() && !text.ends_with(['\r', '\n']) {
        text.push_str(&line_end);
    }
    text.push_str(row);
    text.push_str(&line_end);
    text
}

/// The structure name a sound added here gets, as the shipped ones have: the table path without `Sounds\` and
/// the extension (`Sounds\animals\bear5.mp3` is `animals\bear5`).
pub fn kr_structure_name(path: &str) -> String {
    let without_root = match path.get(..SOUND_PATH_ROOT.len()) {
        Some(root) if root.eq_ignore_ascii_case(SOUND_PATH_ROOT) => &path[SOUND_PATH_ROOT.len()..],
        _ => path,
    };
    let extension = without_root.len().saturating_sub(TABLE_PATH_EXTENSION.len());
    match without_root.get(extension..) {
        Some(end) if end.eq_ignore_ascii_case(TABLE_PATH_EXTENSION) => without_root[..extension].to_owned(),
        _ => without_root.to_owned(),
    }
}

/// The lines of `text`, each with its line end. Errors name `source`, the line and the column.
fn split_lines(source: &str, text: &str) -> eyre::Result<Vec<CsvLine>> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let line_number = lines.len() + 1;
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line_end_len = match rest[end..].as_bytes() {
            [b'\r', b'\n', ..] => 2,
            [] => 0,
            _ => 1,
        };
        let line = &rest[..end];
        if line.chars().count() > KR_MAX_LINE_CHARS {
            eyre::bail!("{source}:{line_number}:{}: the line is longer than the {KR_MAX_LINE_CHARS} characters the client reads", KR_MAX_LINE_CHARS + 1);
        }
        if line.is_empty() {
            eyre::bail!("{source}:{line_number}:1: a blank line (the client reads it as a sound entry without a name)");
        }
        lines.push(CsvLine { line: line_number, fields: split_fields(source, line_number, line)?, line_end: rest[end..end + line_end_len].to_owned() });
        rest = &rest[end + line_end_len..];
    }
    Ok(lines)
}

fn split_fields(source: &str, line_number: usize, line: &str) -> eyre::Result<Vec<CsvField>> {
    let mut fields = Vec::new();
    let mut field = CsvField { raw: String::new(), value: String::new() };
    let mut quoted_at = None;
    for (column, c) in line.chars().enumerate() {
        match c {
            QUOTE => {
                quoted_at = if quoted_at.is_some() { None } else { Some(column + 1) };
                field.raw.push(c);
            }
            FIELD_SEPARATOR if quoted_at.is_none() => fields.push(std::mem::replace(&mut field, CsvField { raw: String::new(), value: String::new() })),
            _ => {
                field.raw.push(c);
                field.value.push(c);
            }
        }
    }
    if let Some(column) = quoted_at {
        eyre::bail!("{source}:{line_number}:{column}: the `{QUOTE}` is not closed (the client reads the rest of the line as one field)");
    }
    fields.push(field);
    Ok(fields)
}

/// A range field: the default (`*`, empty), one value, or `low high` (`1.0-1.1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KrRange<T> {
    Default,
    Value(T),
    Span(T, T),
}

/// Reads a range the client's way (see the module documentation); `Err` says why the text is refused.
pub fn parse_range<T: FromStr>(text: &str) -> Result<KrRange<T>, String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.starts_with(NEGATIVE_SIGN) && compact[NEGATIVE_SIGN.len_utf8()..].starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return Err(format!("`{text}`: a value cannot be negative (the client reads `-5` as 5)"));
    }
    let spaced: String = compact.chars().map(|c| if RANGE_SPACES.contains(&c) { SPACE } else { c }).collect();
    let parts: Vec<&str> = spaced.split_whitespace().collect();
    let number = |part: &str| part.parse::<T>().map_err(|_| format!("`{text}`: `{part}` is not a number"));
    match parts.as_slice() {
        [] => Ok(KrRange::Default),
        [value] => Ok(KrRange::Value(number(value)?)),
        [low, high] => Ok(KrRange::Span(number(low)?, number(high)?)),
        _ => Err(format!("`{text}`: a range is one value or two")),
    }
}

/// `*` or empty is the default (`None`), else a number.
fn parse_optional<T: FromStr>(text: &str) -> Result<Option<T>, String> {
    if text.is_empty() || text == DEFAULT_MARK {
        return Ok(None);
    }
    text.parse().map(Some).map_err(|_| format!("`{text}` is not a number, `{DEFAULT_MARK}` or empty"))
}

/// One sound entry of `audio_sounds.csv`.
#[derive(Debug, Clone, PartialEq)]
pub struct KrSoundRow {
    pub line: usize,
    /// 0 (or default) = sample, 1 = stream.
    pub type_id: Option<i32>,
    pub name: String,
    pub filename: String,
    pub rate: KrRange<f64>,
    pub volume: KrRange<f64>,
    pub pan: KrRange<f64>,
    pub offset: KrRange<i32>,
    pub min_range: Option<f64>,
    pub max_range: Option<f64>,
    /// Read, then ignored by the client.
    pub repeat: Option<i32>,
    pub mono: Option<i32>,
    pub polygroup: Option<String>,
    pub priority: Option<i32>,
    pub streamed: Option<i32>,
    /// The line of the first structure of this name, when this one comes later: the client skips it.
    pub duplicate_of: Option<usize>,
}

impl KrSoundRow {
    /// The `Audio.uop` entry the file name stands for.
    pub fn entry_name(&self) -> String {
        table_path_entry_name(&self.filename)
    }
}

/// A section of `audio_sounds.csv`, which says what its `#` rows are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KrSection {
    Polygroups,
    RolloffFactor,
    DefaultParameter,
    /// Any other name: its `#` rows are skipped.
    Other,
}

impl KrSection {
    fn of_name(name: &str) -> Self {
        match name.to_ascii_uppercase().as_str() {
            POLYGROUPS_SECTION => Self::Polygroups,
            ROLLOFF_SECTION => Self::RolloffFactor,
            DEFAULT_PARAMETER_SECTION => Self::DefaultParameter,
            _ => Self::Other,
        }
    }
}

/// What a line of `audio_sounds.csv` is to the client.
#[derive(Debug, Clone, PartialEq)]
pub enum KrSoundLine {
    Section { name: String, section: KrSection },
    Comment,
    /// A `#` row of the polygroup or rolloff section, kept as its fields.
    Parameter(KrSection),
    Sound(KrSoundRow),
    /// A line the client skips, and why.
    Skipped(String),
}

fn sound_row(line: &CsvLine, names: &HashMap<String, usize>) -> Result<KrSoundRow, (usize, String)> {
    let fields = KR_SOUND_FIELDS + 1;
    if line.fields.len() < fields {
        return Err((1, format!("{} fields, a sound entry has {fields}", line.fields.len())));
    }
    // The 1-based column of field `index`.
    let column = |index: usize| line.fields[..index].iter().map(|field| field.raw.chars().count() + 1).sum::<usize>() + 1;
    let field = |index: usize| line.value(index);
    let at = |index: usize| move |message: String| (column(index), message);
    let name = field(2).to_owned();
    let filename = field(3).to_owned();
    for (index, what, text) in [(2, "name", &name), (3, "file name", &filename)] {
        if text.is_empty() {
            return Err((column(index), format!("a sound entry needs a {what}")));
        }
        if text.chars().count() > KR_MAX_NAME_CHARS {
            return Err((column(index), format!("the {what} is longer than {KR_MAX_NAME_CHARS} characters")));
        }
    }
    let polygroup = match field(12) {
        "" | DEFAULT_MARK => None,
        group => Some(group.to_owned()),
    };
    Ok(KrSoundRow {
        line: line.line,
        type_id: parse_optional(field(1)).map_err(at(1))?,
        rate: parse_range(field(4)).map_err(at(4))?,
        volume: parse_range(field(5)).map_err(at(5))?,
        pan: parse_range(field(6)).map_err(at(6))?,
        offset: parse_range(field(7)).map_err(at(7))?,
        min_range: parse_optional(field(8)).map_err(at(8))?,
        max_range: parse_optional(field(9)).map_err(at(9))?,
        repeat: parse_optional(field(10)).map_err(at(10))?,
        mono: parse_optional(field(11)).map_err(at(11))?,
        polygroup,
        priority: parse_optional(field(13)).map_err(at(13))?,
        streamed: parse_optional(field(14)).map_err(at(14))?,
        duplicate_of: names.get(&name).copied(),
        name,
        filename,
    })
}

/// Kingdom Reborn's `data/audio/audio_sounds.csv`.
#[derive(Debug, Clone, PartialEq)]
pub struct KrSoundTable {
    lines: Vec<(CsvLine, KrSoundLine)>,
    /// The first structure of each name: its index in `lines`.
    by_name: HashMap<String, usize>,
    /// The structures the client gives an id, in id order from [`KR_FIRST_SOUND_ID`]: their index in `lines`.
    by_id: Vec<usize>,
}

impl KrSoundTable {
    /// Parses `text`; errors name `source`, the line and the column.
    pub fn parse(source: &str, text: &str) -> eyre::Result<Self> {
        let mut lines = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        let mut by_id = Vec::new();
        let mut names_by_line: HashMap<String, usize> = HashMap::new();
        let mut section = None;
        let mut per_file: HashMap<String, usize> = HashMap::new();
        for line in split_lines(source, text)? {
            let first = line.value(0);
            let kind = match first.chars().next() {
                Some(DIRECTIVE_MARK) => {
                    let name = line.value(1).to_owned();
                    let parsed = KrSection::of_name(&name);
                    section = Some(name.clone());
                    KrSoundLine::Section { name, section: parsed }
                }
                Some(COMMENT_MARK) => KrSoundLine::Comment,
                Some(ROW_MARK) => match section.as_deref().map(KrSection::of_name) {
                    Some(KrSection::Polygroups) => KrSoundLine::Parameter(KrSection::Polygroups),
                    Some(KrSection::RolloffFactor) => KrSoundLine::Parameter(KrSection::RolloffFactor),
                    Some(KrSection::DefaultParameter) => KrSoundLine::Sound(Self::parse_sound(source, &line, &names_by_line)?),
                    Some(KrSection::Other) => KrSoundLine::Skipped(format!(
                        "the client reads no `{ROW_MARK}` rows in the `{}` section",
                        section.as_deref().unwrap_or_default()
                    )),
                    None => KrSoundLine::Skipped(format!("a `{ROW_MARK}` row before any section")),
                },
                None | Some(SPACE) => KrSoundLine::Sound(Self::parse_sound(source, &line, &names_by_line)?),
                Some(other) => KrSoundLine::Skipped(format!("the client skips a line that starts with `{other}`")),
            };
            if let KrSoundLine::Sound(row) = &kind {
                if row.duplicate_of.is_none() {
                    if by_name.len() == KR_MAX_SOUND_NAMES {
                        eyre::bail!("{source}:{}:1: more than the {KR_MAX_SOUND_NAMES} sound names the client takes", line.line);
                    }
                    by_name.insert(row.name.clone(), lines.len());
                    if row.name != KR_DEFAULT_SOUND_NAME {
                        by_id.push(lines.len());
                    }
                    names_by_line.insert(row.name.clone(), line.line);
                    let users = per_file.entry(row.entry_name()).or_default();
                    *users += 1;
                    if *users > KR_MAX_STRUCTURES_PER_FILE {
                        eyre::bail!(
                            "{source}:{}:1: `{}` serves more than the {KR_MAX_STRUCTURES_PER_FILE} structures a file may serve",
                            line.line,
                            row.filename
                        );
                    }
                }
            }
            lines.push((line, kind));
        }
        Ok(Self { lines, by_name, by_id })
    }

    fn parse_sound(source: &str, line: &CsvLine, names: &HashMap<String, usize>) -> eyre::Result<KrSoundRow> {
        sound_row(line, names).map_err(|(column, message)| eyre!("{source}:{}:{column}: {message}", line.line))
    }

    /// The text of the table: every line as read (see the module documentation).
    pub fn to_text(&self) -> String {
        self.lines.iter().map(|(line, _)| line.to_text()).collect()
    }

    pub fn lines(&self) -> impl Iterator<Item = (&CsvLine, &KrSoundLine)> {
        self.lines.iter().map(|(line, kind)| (line, kind))
    }

    /// Every sound entry, the skipped duplicates included.
    pub fn sounds(&self) -> impl Iterator<Item = &KrSoundRow> {
        self.lines.iter().filter_map(|(_, kind)| match kind {
            KrSoundLine::Sound(row) => Some(row),
            _ => None,
        })
    }

    /// The structure the client takes for `name`: the first of that name.
    pub fn sound(&self, name: &str) -> Option<&KrSoundRow> {
        match &self.lines[*self.by_name.get(name)?].1 {
            KrSoundLine::Sound(row) => Some(row),
            _ => None,
        }
    }

    /// The structure of id `id`: `ParseSoundsCsv` numbers the structures it accepts in their order, from
    /// [`KR_FIRST_SOUND_ID`], leaving out `s_default` and the duplicates.
    pub fn sound_by_id(&self, id: u32) -> Option<&KrSoundRow> {
        let index = usize::try_from(id.checked_sub(KR_FIRST_SOUND_ID)?).ok()?;
        match &self.lines[*self.by_id.get(index)?].1 {
            KrSoundLine::Sound(row) => Some(row),
            _ => None,
        }
    }

    /// The structure a handle names, when it is a sound structure's.
    pub fn sound_by_handle(&self, handle: u32) -> Option<&KrSoundRow> {
        (handle & !KR_HANDLE_ID_MASK == KR_SOUND_HANDLE_KIND).then(|| self.sound_by_id(handle & KR_HANDLE_ID_MASK)).flatten()
    }

    /// The table with a sound structure `name` playing `filename` (a table path) added at the end, with
    /// [`KR_NEW_SOUND_PARAMETERS`], read again as [`Self::parse`] reads it. Refused when the name is taken, or when
    /// the table would break one of the client's limits.
    pub fn with_sound(&self, source: &str, name: &str, filename: &str) -> eyre::Result<Self> {
        if let Some(row) = self.sound(name) {
            eyre::bail!("{source}: the sound structure `{name}` is already defined on line {}", row.line);
        }
        for (what, text) in [("name", name), ("file name", filename)] {
            if text.chars().count() > KR_MAX_NAME_CHARS {
                eyre::bail!("{source}: the {what} `{text}` is longer than the {KR_MAX_NAME_CHARS} characters the client reads");
            }
        }
        let row = format!("{FIELD_SEPARATOR}{FIELD_SEPARATOR}{name}{FIELD_SEPARATOR}{filename}{FIELD_SEPARATOR}{KR_NEW_SOUND_PARAMETERS}");
        let table = Self::parse(source, &text_with_line(self.lines.iter().map(|(line, _)| line), &row))?;
        match table.lines.last() {
            Some((_, KrSoundLine::Sound(added))) if added.name == name && added.filename == filename && added.duplicate_of.is_none() => Ok(table),
            _ => eyre::bail!("{source}: the added line does not read back as the sound structure `{name}`"),
        }
    }

    /// The lines the client skips, and the structures it skips as duplicates, one note each.
    pub fn client_notes(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|(line, kind)| match kind {
                KrSoundLine::Skipped(why) => Some(format!("line {}: skipped: {why}", line.line)),
                KrSoundLine::Sound(KrSoundRow { name, duplicate_of: Some(first), .. }) => {
                    Some(format!("line {}: skipped: the structure `{name}` is already defined on line {first}", line.line))
                }
                _ => None,
            })
            .collect()
    }
}

/// One row of `audio_effects.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrEffectRow {
    pub line: usize,
    /// `ID#`: the server's sound id + [`KR_EFFECT_ID_OFFSET`].
    pub id: u32,
    /// The sound structure (or group, queue, loop) name; a value starting with a digit is a structure handle
    /// ([`Self::structure_handle`]).
    pub structure: String,
    /// The line of the first row of this id, when this one comes later. Which one the client keeps was not
    /// traced; [`KrEffectsTable::by_server_id`] takes the first.
    pub duplicate_of: Option<usize>,
}

impl KrEffectRow {
    /// The server's sound id; `None` for `ID#` 0.
    pub fn server_id(&self) -> Option<u32> {
        self.id.checked_sub(KR_EFFECT_ID_OFFSET)
    }

    /// The structure handle a structure starting with a digit is read as: `atol` of it (the leading digits, up to
    /// `i32::MAX`), as `LoadEffectsTable` stores it. `None` for a name.
    pub fn structure_handle(&self) -> Option<u32> {
        if !self.structure.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let max = u64::from(i32::MAX.unsigned_abs());
        let value = self
            .structure
            .bytes()
            .take_while(u8::is_ascii_digit)
            .fold(0u64, |value, digit| (value * DECIMAL_RADIX + u64::from(digit - b'0')).min(max));
        Some(u32::try_from(value).expect("at most i32::MAX"))
    }
}

const DECIMAL_RADIX: u64 = 10;

/// Kingdom Reborn's `data/audio/audio_effects.csv`: `;` comment rows and `,ID#,name[,...]` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrEffectsTable {
    lines: Vec<(CsvLine, Option<KrEffectRow>)>,
    by_id: HashMap<u32, usize>,
}

impl KrEffectsTable {
    /// Parses `text`; errors name `source`, the line and the column.
    pub fn parse(source: &str, text: &str) -> eyre::Result<Self> {
        let mut lines = Vec::new();
        let mut by_id: HashMap<u32, usize> = HashMap::new();
        for line in split_lines(source, text)? {
            let row = match line.value(0).chars().next() {
                Some(COMMENT_MARK) => None,
                None => {
                    let id_text = line.value(1);
                    let column = line.fields[0].raw.chars().count() + 2;
                    let id = id_text
                        .parse::<u32>()
                        .ok()
                        .filter(|_| id_text.bytes().all(|byte| byte.is_ascii_digit()))
                        .ok_or_else(|| eyre!("{source}:{}:{column}: expected the ID# (decimal digits), found `{id_text}`", line.line))?;
                    let structure = line.value(2).to_owned();
                    if structure.is_empty() {
                        eyre::bail!("{source}:{}:{}: expected a structure name", line.line, column + id_text.chars().count() + 1);
                    }
                    let duplicate_of = by_id.get(&id).map(|&index| lines_line(&lines, index));
                    if duplicate_of.is_none() {
                        by_id.insert(id, lines.len());
                    }
                    Some(KrEffectRow { line: line.line, id, structure, duplicate_of })
                }
                Some(other) => eyre::bail!("{source}:{}:1: expected `{COMMENT_MARK}` or an empty first field, found `{other}`", line.line),
            };
            lines.push((line, row));
        }
        Ok(Self { lines, by_id })
    }

    /// The text of the table: every line as read.
    pub fn to_text(&self) -> String {
        self.lines.iter().map(|(line, _)| line.to_text()).collect()
    }

    pub fn rows(&self) -> impl Iterator<Item = &KrEffectRow> {
        self.lines.iter().filter_map(|(_, row)| row.as_ref())
    }

    /// The row of a server sound id (`ID#` = id + 1); the first when there are several.
    pub fn by_server_id(&self, server_id: u32) -> Option<&KrEffectRow> {
        let id = server_id.checked_add(KR_EFFECT_ID_OFFSET)?;
        self.lines[*self.by_id.get(&id)?].1.as_ref()
    }

    /// The table with a row of `ID#` `server_id` + [`KR_EFFECT_ID_OFFSET`] naming `structure` added at the end,
    /// read again as [`Self::parse`] reads it. Refused when the id has a row, or when the name starts with a digit
    /// (the client would read it as a structure handle).
    pub fn with_row(&self, source: &str, server_id: u32, structure: &str) -> eyre::Result<Self> {
        let id = server_id.checked_add(KR_EFFECT_ID_OFFSET).ok_or_else(|| eyre!("{source}: server id {server_id} has no ID#"))?;
        if let Some(row) = self.by_server_id(server_id) {
            eyre::bail!("{source}: server id {server_id} (ID# {id}) already has the row on line {}", row.line);
        }
        if structure.is_empty() || structure.starts_with(|c: char| c.is_ascii_digit()) {
            eyre::bail!("{source}: `{structure}`: a structure name must not be empty or start with a digit (the client reads that as a structure handle)");
        }
        let row = format!("{FIELD_SEPARATOR}{id}{FIELD_SEPARATOR}{structure}");
        let table = Self::parse(source, &text_with_line(self.lines.iter().map(|(line, _)| line), &row))?;
        match table.lines.last() {
            Some((_, Some(added))) if added.id == id && added.structure == structure => Ok(table),
            _ => eyre::bail!("{source}: the added line does not read back as ID# {id} naming `{structure}`"),
        }
    }

    /// The rows of an id defined again, one note each.
    pub fn client_notes(&self) -> Vec<String> {
        self.rows()
            .filter_map(|row| {
                row.duplicate_of.map(|first| {
                    format!(
                        "line {}: ID# {} is already defined on line {first}; the first row is used here (which one the client \
                         keeps was not traced)",
                        row.line, row.id
                    )
                })
            })
            .collect()
    }
}

fn lines_line(lines: &[(CsvLine, Option<KrEffectRow>)], index: usize) -> usize {
    lines[index].0.line
}

/// Kingdom Reborn's tables of an `Audio.uop`.
#[derive(Debug, Clone, PartialEq)]
pub struct KrAudioTables {
    pub sounds: KrSoundTable,
    /// `None` when the package has no `audio_effects.csv`.
    pub effects: Option<KrEffectsTable>,
}

impl KrAudioTables {
    /// The effects row of a server sound id, and the sound structure it names when it is one.
    pub fn sound_for_server_id(&self, server_id: u32) -> Option<(&KrEffectRow, Option<&KrSoundRow>)> {
        let effect = self.effects.as_ref()?.by_server_id(server_id)?;
        Some((effect, self.structure_sound(effect)))
    }

    /// The sound structure an effects row names: by handle when it starts with a digit, else by name.
    pub fn structure_sound(&self, effect: &KrEffectRow) -> Option<&KrSoundRow> {
        match effect.structure_handle() {
            Some(handle) => self.sounds.sound_by_handle(handle),
            None => self.sounds.sound(&effect.structure),
        }
    }

    /// The tables with a new sound for the server id `server_id` playing the table path `path` (`Sounds\...mp3`):
    /// a structure named [`kr_structure_name`] in `audio_sounds.csv` ([`KrSoundTable::with_sound`]) and its
    /// `audio_effects.csv` row ([`KrEffectsTable::with_row`]). `source` names the package in errors.
    pub fn with_sound(&self, source: &str, server_id: u32, path: &str) -> eyre::Result<Self> {
        use crate::enhanced::audio::SOUND_TABLE_ENTRY;
        let Some(effects) = &self.effects else {
            eyre::bail!("{source}: the package has no {KR_EFFECTS_TABLE_ENTRY}, through which the client plays a server id");
        };
        check_table_path(path, false)?;
        let name = kr_structure_name(path);
        if name == KR_DEFAULT_SOUND_NAME {
            eyre::bail!("`{path}`: the structure would be named `{KR_DEFAULT_SOUND_NAME}`, the default-parameter row");
        }
        let effects = effects.with_row(&format!("{source}: {KR_EFFECTS_TABLE_ENTRY}"), server_id, &name)?;
        let sounds = self.sounds.with_sound(&format!("{source}: {SOUND_TABLE_ENTRY}"), &name, path)?;
        Ok(Self { sounds, effects: Some(effects) })
    }

    /// The structures no effects row reaches and the effects rows naming no sound structure, as counts, and the
    /// notes of both tables.
    pub fn client_notes(&self) -> Vec<String> {
        let mut notes: Vec<String> = self.sounds.client_notes().into_iter().map(|note| format!("audio_sounds.csv {note}")).collect();
        if let Some(effects) = &self.effects {
            notes.extend(effects.client_notes().into_iter().map(|note| format!("audio_effects.csv {note}")));
            for row in effects.rows().filter(|row| row.duplicate_of.is_none() && self.structure_sound(row).is_none()) {
                notes.push(format!(
                    "audio_effects.csv line {}: ID# {} (server id {}) names `{}`, which is no sound structure",
                    row.line,
                    row.id,
                    row.server_id().map_or_else(|| "none".to_owned(), |id| id.to_string()),
                    row.structure
                ));
            }
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUNDS: &str = "w,s,id,filename,rate,vol,pan,offset,r1,r2,,,polygroup,type,,status,usage,notes\r\n\
        ;,\"structure name, with a comma\",,\r\n\
        !,ROLLOFF FACTOR,,\r\n\
        #,2,,\r\n\
        !,SOUND DEFAULT PARAMETER VALUES,,\r\n\
        #,,s_default,,1,0.99,0.5,0,150,1200,50,0,,1,1,,,\r\n\
        ,,aaaenvir\\d_frst01,Sounds\\aaaenvir\\Env_Forest.mp3,1.0-1.1,*,*,*,*,*,*,1,*,*,*,,,\r\n\
        ,,turfin,Music/turfin.mp3,100%,*,*,*,*,*,*,*,*,*,*\r\n\
        ,,turfin,Music/other.mp3,1.0,*,*,*,*,*,*,*,*,*,*\n";
    const EFFECTS: &str = ";,ID#,description,sound,notes\r\n,1,aaaenvir\\d_frst01,,\"a note, with a comma\"\r\n,2,l_turfin\r\n,2,turfin";

    #[test]
    fn reads_the_sounds_the_way_the_client_does() {
        let table = KrSoundTable::parse("audio_sounds.csv", SOUNDS).unwrap();
        assert_eq!(table.to_text(), SOUNDS, "written back byte for byte");
        let sounds: Vec<&KrSoundRow> = table.sounds().collect();
        assert_eq!(sounds.len(), 3, "s_default is in a section whose `#` rows are skipped");
        let forest = table.sound("aaaenvir\\d_frst01").unwrap();
        assert_eq!(forest.entry_name(), "data/audio/sounds/aaaenvir/env_forest.mp3");
        assert_eq!((forest.rate, forest.volume, forest.mono, forest.polygroup.clone()), (KrRange::Span(1.0, 1.1), KrRange::Default, Some(1), None));
        assert_eq!(table.sound("turfin").unwrap().rate, KrRange::Value(100.0));
        assert_eq!(table.sound("turfin").unwrap().filename, "Music/turfin.mp3", "the first structure of a name wins");
        let notes = table.client_notes();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].starts_with("line 1: skipped: the client skips a line that starts with `w`"), "{notes:?}");
        assert!(notes[1].contains("line 6: skipped: the client reads no `#` rows in the `SOUND DEFAULT PARAMETER VALUES`"), "{notes:?}");
        assert!(notes[2].contains("line 9: skipped: the structure `turfin` is already defined on line 8"), "{notes:?}");
        assert!(matches!(table.lines().nth(3).unwrap().1, KrSoundLine::Parameter(KrSection::RolloffFactor)));
    }

    #[test]
    fn follows_a_server_id_to_its_file() {
        let tables = KrAudioTables {
            sounds: KrSoundTable::parse("audio_sounds.csv", SOUNDS).unwrap(),
            effects: Some(KrEffectsTable::parse("audio_effects.csv", EFFECTS).unwrap()),
        };
        let effects = tables.effects.as_ref().unwrap();
        assert_eq!(effects.to_text(), EFFECTS);
        let (effect, sound) = tables.sound_for_server_id(0).unwrap();
        assert_eq!((effect.id, effect.server_id()), (1, Some(0)));
        assert_eq!(sound.unwrap().entry_name(), "data/audio/sounds/aaaenvir/env_forest.mp3");
        let (loop_row, none) = tables.sound_for_server_id(1).unwrap();
        assert_eq!((loop_row.structure.as_str(), none), ("l_turfin", None), "the first row of ID# 2 is taken");
        assert!(tables.sound_for_server_id(2).is_none());
        let notes = tables.client_notes();
        assert!(notes.iter().any(|note| note.contains("ID# 2 is already defined on line 3")), "{notes:?}");

        // A structure starting with a digit is a handle: 0x10000 | n, the n-th structure the client gave an id
        // (s_default and the second `turfin` get none). 65537 is the forest, 65538 `turfin`, 65539 nothing.
        let by_handle = KrAudioTables {
            sounds: tables.sounds.clone(),
            effects: Some(KrEffectsTable::parse("e.csv", ",1,65537\r\n,2,65538x\r\n,3,65539\r\n,4,1\r\n,5,99999999999").unwrap()),
        };
        let file = |server_id| by_handle.sound_for_server_id(server_id).unwrap().1.map(|row| row.filename.as_str());
        assert_eq!(file(0), Some("Sounds\\aaaenvir\\Env_Forest.mp3"));
        assert_eq!(file(1), Some("Music/turfin.mp3"), "atol stops at the first non-digit");
        assert_eq!((file(2), file(3), file(4)), (None, None, None), "past the last id, another kind, LONG_MAX");
        assert_eq!(by_handle.effects.as_ref().unwrap().by_server_id(4).unwrap().structure_handle(), Some(i32::MAX.unsigned_abs()));
        assert!(notes.iter().any(|note| note.contains("names `l_turfin`, which is no sound structure")), "{notes:?}");
    }

    #[test]
    fn refuses_what_the_client_would_misread() {
        let error = |text: &str| KrSoundTable::parse("s.csv", text).unwrap_err().to_string();
        let row = |rate: &str| format!(",,a,a.mp3,{rate},*,*,*,*,*,*,*,*,*,*\r\n");
        assert!(error(&row("-5")).contains("s.csv:1:11: `-5`: a value cannot be negative"), "{}", error(&row("-5")));
        assert!(error(&row("fast")).contains("`fast` is not a number"), "{}", error(&row("fast")));
        assert!(error(&row("1-2-3")).contains("one value or two"), "{}", error(&row("1-2-3")));
        assert_eq!(parse_range::<i32>("1 2"), Ok(KrRange::Value(12)), "white space is removed first");
        assert!(error(",,a,a.mp3,1\r\n").contains("s.csv:1:1: 5 fields, a sound entry has 15"));
        assert!(error(",,,a.mp3,1,*,*,*,*,*,*,*,*,*,*").contains("s.csv:1:3: a sound entry needs a name"));
        assert!(error(&format!(",,a,{}.mp3,1,*,*,*,*,*,*,*,*,*,*", "x".repeat(KR_MAX_NAME_CHARS))).contains("longer than 80"));
        assert!(error(",,a,a.mp3,1,*,*,*,*,*,*,yes,*,*,*").contains("s.csv:1:25: `yes` is not a number"), "{}", error(",,a,a.mp3,1,*,*,*,*,*,*,yes,*,*,*"));
        assert!(error(";,\"open\r\n").contains("s.csv:1:3: the `\"` is not closed"));
        assert!(error(";,a\r\n\r\n;,b").contains("s.csv:2:1: a blank line"));
        assert!(error(&format!(";{}", "x".repeat(KR_MAX_LINE_CHARS))).contains("longer than the 2046 characters"));
        assert_eq!(parse_range::<i32>("90 - 110"), Ok(KrRange::Span(90, 110)));
        assert_eq!(parse_range::<f64>("*"), Ok(KrRange::Default));
        let effects = |text: &str| KrEffectsTable::parse("e.csv", text).unwrap_err().to_string();
        assert!(effects(",x,a").contains("e.csv:1:2: expected the ID#"), "{}", effects(",x,a"));
        assert!(effects(",1,").contains("e.csv:1:4: expected a structure name"), "{}", effects(",1,"));
        assert!(effects("w,1,a").contains("found `w`"));
    }

    #[test]
    fn adds_a_sound_structure_and_its_effects_row_within_the_clients_limits() {
        let tables = KrAudioTables {
            sounds: KrSoundTable::parse("audio_sounds.csv", SOUNDS).unwrap(),
            effects: Some(KrEffectsTable::parse("audio_effects.csv", EFFECTS).unwrap()),
        };
        assert_eq!(kr_structure_name("Sounds\\animals\\bear5.mp3"), "animals\\bear5");
        assert_eq!(kr_structure_name("sounds\\Bell.MP3"), "Bell");
        let added = tables.with_sound("Audio.uop", 9, "Sounds\\custom\\Horn.mp3").unwrap();
        let sounds = added.sounds.to_text();
        // SOUNDS ends with LF: the new line ends like the first one, CRLF; EFFECTS has no last line end.
        assert_eq!(sounds, format!("{SOUNDS},,custom\\Horn,Sounds\\custom\\Horn.mp3,{KR_NEW_SOUND_PARAMETERS}\r\n"));
        assert_eq!(added.effects.as_ref().unwrap().to_text(), format!("{EFFECTS}\r\n,10,custom\\Horn\r\n"));
        let (effect, sound) = added.sound_for_server_id(9).unwrap();
        assert_eq!((effect.id, sound.unwrap().entry_name()), (10, "data/audio/sounds/custom/horn.mp3".to_owned()));
        let horn = added.sounds.sound("custom\\Horn").unwrap();
        assert_eq!((horn.rate, horn.mono, horn.type_id), (KrRange::Span(1.0, 1.1), Some(1), None));
        assert!(added.client_notes().len() == tables.client_notes().len(), "{:?}", added.client_notes());

        let error = |server_id: u32, path: &str| format!("{:#}", tables.with_sound("Audio.uop", server_id, path).unwrap_err());
        assert!(error(0, "Sounds\\x.mp3").contains("server id 0 (ID# 1) already has the row on line 2"), "{}", error(0, "Sounds\\x.mp3"));
        assert!(error(9, "Music/x.mp3").contains("start with `Sounds\\`"), "{}", error(9, "Music/x.mp3"));
        assert!(error(9, "Sounds\\1st.mp3").contains("must not be empty or start with a digit"));
        assert!(error(9, "Sounds\\s_default.mp3").contains("default-parameter row"));
        assert!(error(9, "Sounds\\aaaenvir\\d_frst01.mp3").contains("`aaaenvir\\d_frst01` is already defined on line 7"), "{}", error(9, "Sounds\\aaaenvir\\d_frst01.mp3"));
        let long = format!("Sounds\\{}.mp3", "x".repeat(KR_MAX_NAME_CHARS));
        assert!(error(9, &long).contains("longer than the 80 characters"), "{}", error(9, &long));
        assert!(KrAudioTables { effects: None, ..tables.clone() }.with_sound("Audio.uop", 9, "Sounds\\a.mp3").unwrap_err().to_string().contains("no data/audio/audio_effects.csv"));

        // One file serves at most KR_MAX_STRUCTURES_PER_FILE structures, and the client takes at most
        // KR_MAX_SOUND_NAMES names.
        let shared: String = (0..KR_MAX_STRUCTURES_PER_FILE).map(|n| format!(",,s{n},Sounds\\shared.mp3,1,*,*,*,*,*,*,*,*,*,*\r\n")).collect();
        let full_file = KrSoundTable::parse("s.csv", &shared).unwrap();
        let error = full_file.with_sound("s.csv", "one_more", "Sounds\\shared.mp3").unwrap_err().to_string();
        assert!(error.contains("serves more than the 30 structures"), "{error}");
        let many: String = (0..KR_MAX_SOUND_NAMES).map(|n| format!(",,s{n},Sounds\\f{n}.mp3,1,*,*,*,*,*,*,*,*,*,*\r\n")).collect();
        let full = KrSoundTable::parse("s.csv", &many).unwrap();
        let error = full.with_sound("s.csv", "one_more", "Sounds\\one_more.mp3").unwrap_err().to_string();
        assert!(error.contains("more than the 4000 sound names"), "{error}");
        assert_eq!(text_with_line([], "a"), "a\r\n", "an empty table");
    }
}
