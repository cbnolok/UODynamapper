//! The Classic remap def files read as the client reads them, strictly, and written back byte for byte.
//!
//! Traced in the 2024 Classic client (`scripts/dev-helpers/ec-research/ghidra-classic-defs`):
//!
//! - **`{targets}` files** ([`TargetsRecord`]): `gump.def`, `anim1.def` to `anim4.def`, `art.def`, `texterr.def`,
//!   `body.def`, `corpse.def`. A line is `ID {T1, T2, ...} VALUE`, then whitespace or `#` and a comment. The client
//!   picks one target at random; `VALUE` is the hue (0 in `anim1/2.def`). It reads a `-` in the braces as nothing,
//!   so `{-1}` (`anim2.def`) is target 1 ([`TargetsRecord::client_targets`]). The first line of an id wins.
//!   `art.def` supplies art ids that have no art; `body.def` does not remap a body below 0x800 that `bodyconv.def`
//!   maps.
//! - **`bodyconv.def`** ([`BodyConvRecord`]): numbers before the first `#`, the body and the body to use in each
//!   later animation file (`anim2.mul`, `anim3.mul`, ...; -1 for none). UO Gold reads 4 numbers, the 2024 client
//!   5 (it drops a line with fewer and does not read a sixth); the shipped files have 4 (Gold), 5 (ML, 7.0.20) or 6
//!   (2024). The last line of a body wins; the client's table has [`BODYCONV_BODIES`] rows.
//! - **`Equipconv.def`** ([`EquipConvRecord`]): body, equipment id, converted id, gump id and hue before the first
//!   `#`; **the client reads the hue as hexadecimal**. The last line of a body and equipment id wins.
//!
//! Strict where the client is lenient: text the client skips over (between the id and `{`, after `}`), a line it
//! would drop or misread, and a line longer than it reads at once ([`MAX_LINE_BYTES`]) are errors, with the line
//! and column ([`DefSyntaxError`]). The quoted comment lines of the shipped `bodyconv.def` and `Equipconv.def`
//! (`"# ..."`, from a spreadsheet export; the client reads no number in them) are comments.
//!
//! A file is kept line by line ([`DefDocument`]): an unchanged record is written as read, an edited or added one
//! as [`DefRecord::format`] writes it, so an unchanged file comes back byte for byte. The bytes are kept as
//! Latin-1, so any byte in a comment survives.

crate::eyre_imports!();
use std::path::Path;

use crate::utils::output_file::write_through_temp;

/// Bytes of a line the client reads at once: `fgets` into 255 bytes keeps 254, the line feed among them.
pub const MAX_LINE_BYTES: usize = 253;
/// Rows of the client's `bodyconv.def` table: bodies 0 to 2047.
pub const BODYCONV_BODIES: u32 = 0x800;
/// Numbers of a `bodyconv.def` line: the body and 3 (Gold) to 5 (2024 client) animation file bodies.
pub const BODYCONV_MIN_NUMBERS: usize = 4;
pub const BODYCONV_MAX_NUMBERS: usize = 6;
/// Fields of an `Equipconv.def` line.
pub const EQUIPCONV_FIELDS: usize = 5;
/// `bodyconv.def` and `Equipconv.def` value for "none" / "the id + 50,000".
pub const NONE: i32 = -1;
const COMMENT: char = '#';
const QUOTE: char = '"';
const CARRIAGE_RETURN: char = '\r';
const HEX_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;

/// A syntax error at a 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefSyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for DefSyntaxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for DefSyntaxError {}

/// A line of a def file being read: its characters and where an error is.
struct Cursor {
    chars: Vec<char>,
    at: usize,
    line: usize,
}

impl Cursor {
    fn new(text: &str, line: usize) -> Self {
        Self { chars: text.chars().collect(), at: 0, line }
    }

    fn error(&self, at: usize, message: impl Into<String>) -> DefSyntaxError {
        DefSyntaxError { line: self.line, column: at + 1, message: message.into() }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    /// A decimal number, `-` allowed when `signed`; `what` names it in errors.
    fn number(&mut self, signed: bool, what: &str) -> Result<i64, DefSyntaxError> {
        let start = self.at;
        if signed && self.peek() == Some('-') {
            self.at += 1;
        }
        let digits = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        if self.at == digits {
            let found = self.peek().map_or_else(|| "the end of the line".to_owned(), |c| format!("`{}`", c.escape_default()));
            return Err(self.error(start, format!("expected {what}, found {found}")));
        }
        let text: String = self.chars[start..self.at].iter().collect();
        text.parse::<i64>().map_err(|_| self.error(start, format!("{what} `{text}` is too large")))
    }

    fn rest(&self) -> String {
        self.chars[self.at..].iter().collect()
    }
}

fn in_range(cursor: &Cursor, at: usize, value: i64, range: std::ops::RangeInclusive<i64>, what: &str) -> Result<i64, DefSyntaxError> {
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(cursor.error(at, format!("{what} {value} is not in {} to {}", range.start(), range.end())))
    }
}

/// A record of a def file.
pub trait DefRecord: Sized + Clone + PartialEq + std::fmt::Debug {
    /// Reads `line` (its line ending removed); `None` for a blank or comment line.
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError>;
    /// The line an edited or added record is written as, without a line ending.
    fn format(&self) -> String;
}

/// `ID {T1, T2, ...} VALUE [# comment]` of `gump.def`, `anim1-4.def`, `art.def`, `texterr.def`, `body.def`,
/// `corpse.def`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetsRecord {
    pub id: u16,
    /// As written; a `-` is read by the client as nothing ([`Self::client_targets`]).
    pub targets: Vec<i32>,
    /// The hue (0 in `anim1/2.def`).
    pub value: i32,
    /// The text after `#`.
    pub comment: Option<String>,
}

impl TargetsRecord {
    /// The targets as the client reads them: a written `-1` is 1.
    pub fn client_targets(&self) -> Vec<u16> {
        self.targets.iter().map(|target| target.unsigned_abs() as u16).collect()
    }
}

fn comment_suffix(comment: &Option<String>) -> String {
    comment.as_ref().map_or_else(String::new, |comment| format!(" {COMMENT}{comment}"))
}

impl DefRecord for TargetsRecord {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        let mut cursor = Cursor::new(line, line_number);
        cursor.skip_spaces();
        if matches!(cursor.peek(), None | Some(COMMENT)) {
            return Ok(None);
        }
        let at = cursor.at;
        let id = cursor.number(false, "an id")?;
        let id = in_range(&cursor, at, id, 0..=i64::from(u16::MAX), "the id")? as u16;
        cursor.skip_spaces();
        if cursor.peek() != Some('{') {
            return Err(cursor.error(cursor.at, "expected `{` after the id"));
        }
        cursor.at += 1;
        let mut targets = Vec::new();
        loop {
            cursor.skip_spaces();
            if targets.is_empty() && cursor.peek() == Some('}') {
                return Err(cursor.error(cursor.at, "a remap needs a target"));
            }
            let at = cursor.at;
            let target = cursor.number(true, "a target")?;
            let limit = i64::from(u16::MAX);
            targets.push(in_range(&cursor, at, target, -limit..=limit, "the target")? as i32);
            cursor.skip_spaces();
            match cursor.peek() {
                Some(',') => cursor.at += 1,
                Some('}') => {
                    cursor.at += 1;
                    break;
                }
                _ => return Err(cursor.error(cursor.at, "expected `,` or `}` after a target")),
            }
        }
        cursor.skip_spaces();
        let at = cursor.at;
        let value = cursor.number(true, "the value after `}`")?;
        let value = in_range(&cursor, at, value, i64::from(i32::MIN)..=i64::from(i32::MAX), "the value")? as i32;
        cursor.skip_spaces();
        let comment = match cursor.peek() {
            None => None,
            Some(COMMENT) => {
                cursor.at += 1;
                Some(cursor.rest())
            }
            Some(other) => return Err(cursor.error(cursor.at, format!("unexpected `{}` after the value", other.escape_default()))),
        };
        Ok(Some(Self { id, targets, value, comment }))
    }

    fn format(&self) -> String {
        let targets: Vec<String> = self.targets.iter().map(i32::to_string).collect();
        format!("{} {{{}}} {}{}", self.id, targets.join(", "), self.value, comment_suffix(&self.comment))
    }
}

/// Splits a `bodyconv.def` or `Equipconv.def` line at its first `#`: the fields and the comment, or `None` for a
/// line without fields (blank, a comment, a quoted comment).
fn split_fields(line: &str) -> Option<(&str, Option<String>)> {
    let (fields, comment) = match line.find(COMMENT) {
        Some(at) => (&line[..at], Some(line[at + COMMENT.len_utf8()..].to_owned())),
        None => (line, None),
    };
    let trimmed = fields.trim();
    (!trimmed.is_empty() && trimmed != QUOTE.to_string()).then_some((fields, comment))
}

/// The whitespace-separated tokens of `fields` with their 0-based columns.
fn tokens(fields: &str) -> Vec<(usize, &str)> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (at, c) in fields.char_indices().chain(std::iter::once((fields.len(), ' '))) {
        match (c.is_whitespace(), start) {
            (true, Some(from)) => {
                tokens.push((from, &fields[from..at]));
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    tokens
}

fn token_number(line_number: usize, (at, token): (usize, &str), range: std::ops::RangeInclusive<i64>, what: &str) -> Result<i64, DefSyntaxError> {
    let mut cursor = Cursor::new(token, line_number);
    let number = cursor.number(true, what).map_err(|error| DefSyntaxError { column: error.column + at, ..error })?;
    if cursor.at != cursor.chars.len() {
        return Err(DefSyntaxError { line: line_number, column: at + cursor.at + 1, message: format!("{what} `{token}` is not a number") });
    }
    if !range.contains(&number) {
        return Err(DefSyntaxError { line: line_number, column: at + 1, message: format!("{what} {number} is not in {} to {}", range.start(), range.end()) });
    }
    Ok(number)
}

/// A `bodyconv.def` line: the body and the body to use in `anim2.mul`, `anim3.mul`, ... ([`NONE`] for none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyConvRecord {
    pub body: u16,
    /// 3 to 5 bodies, one per later animation file.
    pub anim_file_bodies: Vec<i32>,
    pub comment: Option<String>,
}

impl DefRecord for BodyConvRecord {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        let Some((fields, comment)) = split_fields(line) else { return Ok(None) };
        let tokens = tokens(fields);
        if !(BODYCONV_MIN_NUMBERS..=BODYCONV_MAX_NUMBERS).contains(&tokens.len()) {
            return Err(DefSyntaxError {
                line: line_number,
                column: tokens.first().map_or(1, |(at, _)| at + 1),
                message: format!(
                    "a bodyconv line has {BODYCONV_MIN_NUMBERS} to {BODYCONV_MAX_NUMBERS} numbers (the body and one per \
                     later animation file), not {}",
                    tokens.len()
                ),
            });
        }
        let body = token_number(line_number, tokens[0], 0..=i64::from(BODYCONV_BODIES) - 1, "the body")? as u16;
        let anim_file_bodies = tokens[1..]
            .iter()
            .map(|&token| token_number(line_number, token, i64::from(NONE)..=i64::from(u16::MAX), "an animation file body").map(|n| n as i32))
            .collect::<Result<_, _>>()?;
        Ok(Some(Self { body, anim_file_bodies, comment }))
    }

    fn format(&self) -> String {
        let mut line = self.body.to_string();
        for body in &self.anim_file_bodies {
            line.push('\t');
            line.push_str(&body.to_string());
        }
        if let Some(comment) = &self.comment {
            line.push_str(&format!("\t{COMMENT}{comment}"));
        }
        line
    }
}

/// An `Equipconv.def` line: on `body`, `equipment` is drawn as `converted` with `gump` (0: the equipment id +
/// 50,000; [`NONE`]: the converted id + 50,000; else the gump) and `hue`. The shipped files have a `converted` of
/// [`NONE`] too (the gargoyle ankh necklace, with its own gump).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipConvRecord {
    pub body: u16,
    pub equipment: u16,
    pub converted: i32,
    pub gump: i32,
    /// Written in hexadecimal, as the client reads it.
    pub hue: u32,
    pub comment: Option<String>,
}

impl DefRecord for EquipConvRecord {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        let Some((fields, comment)) = split_fields(line) else { return Ok(None) };
        let tokens = tokens(fields);
        if tokens.len() != EQUIPCONV_FIELDS {
            return Err(DefSyntaxError {
                line: line_number,
                column: tokens.get(EQUIPCONV_FIELDS).or(tokens.first()).map_or(1, |(at, _)| at + 1),
                message: format!(
                    "an Equipconv line has {EQUIPCONV_FIELDS} fields (body, equipment, converted id, gump, hue), not {}",
                    tokens.len()
                ),
            });
        }
        let id = |index: usize, what: &str| token_number(line_number, tokens[index], 0..=i64::from(u16::MAX), what).map(|n| n as u16);
        let (body, equipment) = (id(0, "the body")?, id(1, "the equipment id")?);
        let id_or_none = |index: usize, what: &str| {
            token_number(line_number, tokens[index], i64::from(NONE)..=i64::from(u16::MAX), what).map(|n| n as i32)
        };
        let (converted, gump) = (id_or_none(2, "the converted id")?, id_or_none(3, "the gump")?);
        let (at, hue) = tokens[4];
        let digits = hue.strip_prefix(HEX_PREFIX).unwrap_or(hue);
        let hue = u32::from_str_radix(digits, HEX_RADIX).ok().filter(|_| !digits.starts_with('+')).ok_or_else(|| DefSyntaxError {
            line: line_number,
            column: at + 1,
            message: format!("the hue `{hue}` is no hexadecimal number (the client reads it as one)"),
        })?;
        Ok(Some(Self { body, equipment, converted, gump, hue, comment }))
    }

    fn format(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{:x}{}", self.body, self.equipment, self.converted, self.gump, self.hue,
            self.comment.as_ref().map_or_else(String::new, |comment| format!("\t{COMMENT}{comment}")))
    }
}

/// A line of a def file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefLine<R> {
    /// A blank or comment line, as read (its carriage return kept).
    Other(String),
    /// A record, and the line it was read from (`None` for an added one).
    Record { record: R, read_as: Option<String> },
}

/// A def file line by line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefDocument<R> {
    /// The lines between line feeds: the last is empty when the file ends with one.
    pub lines: Vec<DefLine<R>>,
}

pub type TargetsDefFile = DefDocument<TargetsRecord>;
pub type BodyConvDefFile = DefDocument<BodyConvRecord>;
pub type EquipConvDefFile = DefDocument<EquipConvRecord>;

impl<R: DefRecord> DefDocument<R> {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
        let mut lines = Vec::new();
        for (index, line) in text.split('\n').enumerate() {
            let content = line.strip_suffix(CARRIAGE_RETURN).unwrap_or(line);
            if content.len() > MAX_LINE_BYTES {
                return Err(DefSyntaxError {
                    line: index + 1,
                    column: MAX_LINE_BYTES + 1,
                    message: format!("the client reads lines of up to {MAX_LINE_BYTES} bytes; the rest would be a line of its own"),
                });
            }
            lines.push(match R::parse(content, index + 1)? {
                Some(record) => DefLine::Record { record, read_as: Some(line.to_owned()) },
                None => DefLine::Other(line.to_owned()),
            });
        }
        Ok(Self { lines })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    /// Whether lines end with a carriage return and a line feed (the first line decides; the shipped files do).
    fn crlf(&self) -> bool {
        self.lines.iter().find_map(|line| match line {
            DefLine::Other(text) | DefLine::Record { read_as: Some(text), .. } => Some(text.ends_with(CARRIAGE_RETURN)),
            DefLine::Record { read_as: None, .. } => None,
        }).unwrap_or(true)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let crlf = self.crlf();
        let last = self.lines.len().saturating_sub(1);
        let mut text = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            match line {
                DefLine::Other(read) => text.push_str(read),
                DefLine::Record { record, read_as: Some(read) }
                    if R::parse(read.strip_suffix(CARRIAGE_RETURN).unwrap_or(read), index + 1).ok().flatten().as_ref() == Some(record) =>
                {
                    text.push_str(read);
                }
                DefLine::Record { record, read_as } => {
                    text.push_str(&record.format());
                    let ended = read_as.as_ref().map_or(crlf && index != last, |read| read.ends_with(CARRIAGE_RETURN));
                    if ended {
                        text.push(CARRIAGE_RETURN);
                    }
                }
            }
            if index != last {
                text.push('\n');
            }
        }
        text.chars().map(|c| c as u8).collect()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    pub fn records(&self) -> impl Iterator<Item = &R> {
        self.lines.iter().filter_map(|line| match line {
            DefLine::Record { record, .. } => Some(record),
            DefLine::Other(_) => None,
        })
    }

    pub fn records_mut(&mut self) -> impl Iterator<Item = &mut R> {
        self.lines.iter_mut().filter_map(|line| match line {
            DefLine::Record { record, .. } => Some(record),
            DefLine::Other(_) => None,
        })
    }

    /// Adds a record after the last line (before the empty one a final line feed leaves).
    pub fn push(&mut self, record: R) {
        let crlf = self.crlf();
        let at = match self.lines.last() {
            Some(DefLine::Other(text)) if text.is_empty() => self.lines.len() - 1,
            _ => self.lines.len(),
        };
        // The line before it gets the line ending the file uses, when it had none (the file's last line).
        if at == self.lines.len() && crlf {
            if let Some(DefLine::Other(text) | DefLine::Record { read_as: Some(text), .. }) = self.lines.last_mut() {
                if !text.ends_with(CARRIAGE_RETURN) {
                    text.push(CARRIAGE_RETURN);
                }
            }
        }
        self.lines.insert(at, DefLine::Record { record, read_as: None });
    }

    /// Removes the records `remove` selects; returns how many.
    pub fn remove_where(&mut self, mut remove: impl FnMut(&R) -> bool) -> usize {
        let before = self.lines.len();
        self.lines.retain(|line| !matches!(line, DefLine::Record { record, .. } if remove(record)));
        before - self.lines.len()
    }
}

impl TargetsDefFile {
    /// The record of `id` the client uses: the first.
    pub fn resolve(&self, id: u16) -> Option<&TargetsRecord> {
        self.records().find(|record| record.id == id)
    }
}

impl BodyConvDefFile {
    /// The record of `body` the client uses: the last.
    pub fn resolve(&self, body: u16) -> Option<&BodyConvRecord> {
        self.records().filter(|record| record.body == body).last()
    }
}

impl EquipConvDefFile {
    /// The record of `body` and `equipment` the client uses: the last.
    pub fn resolve(&self, body: u16, equipment: u16) -> Option<&EquipConvRecord> {
        self.records().filter(|record| record.body == body && record.equipment == equipment).last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error<R: DefRecord>(text: &str) -> DefSyntaxError {
        DefDocument::<R>::from_bytes(text.as_bytes()).unwrap_err()
    }

    #[test]
    fn targets_lines_read_as_the_client_reads_them_and_write_back() {
        let text = "# Format\r\n\r\n46 {12, 59} 1106\r\n35\t{-1}\t0 # none\r\n770 {401} 2308 \r\n46 {1} 2\r\n";
        let mut file = TargetsDefFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        assert_eq!(file.resolve(46).unwrap().targets, vec![12, 59], "the first line of an id wins");
        let none = file.resolve(35).unwrap();
        assert_eq!((none.client_targets(), none.comment.as_deref()), (vec![1], Some(" none")));
        file.records_mut().find(|record| record.id == 770).unwrap().value = 0;
        file.push(TargetsRecord { id: 9, targets: vec![1, 2], value: 0, comment: None });
        let written = String::from_utf8(file.to_bytes()).unwrap();
        assert!(written.ends_with("770 {401} 0\r\n46 {1} 2\r\n9 {1, 2} 0\r\n"), "{written:?}");

        assert_eq!(error::<TargetsRecord>("1 x {2} 0").column, 3);
        assert_eq!(error::<TargetsRecord>("1 {2 3} 0").message, "expected `,` or `}` after a target");
        assert_eq!(error::<TargetsRecord>("1 {} 0").message, "a remap needs a target");
        assert_eq!(error::<TargetsRecord>("1 {2}").message, "expected the value after `}`, found the end of the line");
        assert_eq!(error::<TargetsRecord>("1 {2} 0 x").column, 9);
        assert_eq!(error::<TargetsRecord>("\n70000 {2} 0").line, 2);
        assert!(error::<TargetsRecord>(&format!("1 {{2}} 0 #{}", "x".repeat(MAX_LINE_BYTES))).message.contains("up to"));
    }

    #[test]
    fn bodyconv_and_equipconv_lines_read_and_write_back() {
        let text = "\"# low level\"\t\t\r\n# c\t\r\n157\t1\t-1\t-1\t-1\t-1\r\n547\t2\t-1\t-1\t-1 #first\r\n547\t3\t-1\t-1\t-1";
        let mut file = BodyConvDefFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        assert_eq!(file.resolve(547).unwrap().anim_file_bodies[0], 3, "the last line of a body wins");
        file.push(BodyConvRecord { body: 1, anim_file_bodies: vec![NONE, 5, NONE, NONE], comment: Some(" new".into()) });
        assert!(file.to_bytes().ends_with(b"547\t3\t-1\t-1\t-1\r\n1\t-1\t5\t-1\t-1\t# new"));
        assert!(error::<BodyConvRecord>("1 2 3").message.contains("not 3"));
        assert_eq!(error::<BodyConvRecord>("2048 1 -1 -1 -1").column, 1);
        assert_eq!(error::<BodyConvRecord>("1 2 x -1 -1").column, 5);

        let text = "401\t1249 1250 61250\t0\t#\tHuman M to F\r\n606\t404\t962\t-1\t1a\r\n";
        let file = EquipConvDefFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        let record = file.resolve(606, 404).unwrap();
        assert_eq!((record.gump, record.hue), (NONE, 0x1a));
        assert_eq!(record.format(), "606\t404\t962\t-1\t1a");
        assert!(error::<EquipConvRecord>("1 2 3 4").message.contains("not 4"));
        assert!(error::<EquipConvRecord>("1 2 3 4 g").message.contains("hexadecimal"));
    }
}
