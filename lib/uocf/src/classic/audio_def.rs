//! `Sound.def` and `Music.def` of the Classic client: id translations, one per line, read strictly.
//!
//! The grammar, as the shipped files follow it (UO Gold to the current Classic client):
//!
//! ```text
//! file     = *(line EOL) [line]           ; EOL is CRLF or LF; the last line may lack it
//! line     = "" | entry | unclosed
//! entry    = id WS "{" targets "}" WS extra [WS]
//! unclosed = id WS "{" targets [WS]        ; no "}" and no extra: see below
//! targets  = *WS int *(sep int) *WS
//! sep      = 1*WS | *WS "," *WS
//! id       = 1*DIGIT                      ; unique in the file
//! int      = ["-"] 1*DIGIT                ; a target of -1 names no slot
//! WS       = 1*(SP | HTAB)
//! ```
//!
//! `Sound.def` (685 ids in UO Gold, 787 in the current client) gives sound ids without a slot of their own the
//! slot to play instead: `654 {487} 0`. Every closed target list has one value, every `extra` is 0, and the ids
//! from 778 on map to `{-1}`. Both releases ship one line without its `}`, `805 {331 0`: it is read as an
//! [`DefTranslation::unclosed`] group of every integer up to the line end (331 and 0), the first slot of which is
//! the one played; how the client reads it is not known. `Music.def` holds the single line `0 {0} 0` without a
//! line end. Comments, blank lines with spaces and anything else are errors: the shipped files have none.
//!
//! [`DefTranslationTable::to_text`] writes a table back in the shipped form: lines in file order (blank lines kept),
//! `id {target} extra` with `, ` between several targets, an unclosed group as read (`805 {331 0`), the file's line
//! end (that of its first line; CRLF for a file of one line) and its final line end or none. Every shipped
//! `Sound.def` and `Music.def` round-trips byte for byte; other spacing is normalized.

use std::collections::BTreeMap;
use std::path::Path;

crate::eyre_imports!();

pub const SOUND_DEF_FILE_NAME: &str = "Sound.def";
pub const MUSIC_DEF_FILE_NAME: &str = "Music.def";
const GROUP_OPEN: char = '{';
const GROUP_CLOSE: char = '}';
const GROUP_SEPARATOR: char = ',';
const NEGATIVE_SIGN: char = '-';
/// The target naming no slot.
pub const NO_TARGET: i32 = -1;
const CRLF: &str = "\r\n";
const LF: &str = "\n";
/// Between several targets of a closed group, and of an unclosed one.
const TARGET_JOINER: &str = ", ";
const UNCLOSED_TARGET_JOINER: &str = " ";

/// One line of a `.def` translation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefTranslation {
    pub id: u32,
    pub targets: Vec<i32>,
    /// The third field (0 in every shipped line; its meaning is not known); `None` for an unclosed group.
    pub extra: Option<i32>,
    /// The group has no `}`: the targets run to the line end.
    pub unclosed: bool,
    /// The 1-based line number in the parsed file; 0 for an entry set after parsing.
    pub line: usize,
}

/// One line of the file, for writing it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefLine {
    Entry(u32),
    Blank,
}

impl DefTranslation {
    /// The targets naming a slot, in file order.
    pub fn slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.targets.iter().filter_map(|&target| u32::try_from(target).ok())
    }

    /// The line as the table writes it.
    pub fn to_line(&self) -> String {
        let join = |joiner: &str| self.targets.iter().map(i32::to_string).collect::<Vec<_>>().join(joiner);
        match self.extra {
            Some(extra) if !self.unclosed => format!("{} {GROUP_OPEN}{}{GROUP_CLOSE} {extra}", self.id, join(TARGET_JOINER)),
            _ => format!("{} {GROUP_OPEN}{}", self.id, join(UNCLOSED_TARGET_JOINER)),
        }
    }
}

/// The translations of a `Sound.def` or `Music.def`, by id, with the layout of the file they were read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefTranslationTable {
    entries: BTreeMap<u32, DefTranslation>,
    lines: Vec<DefLine>,
    line_end: &'static str,
    final_line_end: bool,
}

impl Default for DefTranslationTable {
    fn default() -> Self {
        Self { entries: BTreeMap::new(), lines: Vec::new(), line_end: CRLF, final_line_end: true }
    }
}

impl DefTranslationTable {
    /// Reads `path`; `Ok(None)` when it does not exist.
    pub fn load(path: &Path) -> eyre::Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
        Self::parse(&path.display().to_string(), &text).map(Some)
    }

    /// Parses `text`; errors name `source`, the line and the column.
    pub fn parse(source: &str, text: &str) -> eyre::Result<Self> {
        let mut entries: BTreeMap<u32, DefTranslation> = BTreeMap::new();
        let mut lines = Vec::new();
        let final_line_end = text.ends_with(LF);
        let body = text.strip_suffix(LF).unwrap_or(text);
        let line_end = match text.find(LF) {
            Some(end) if !text[..end].ends_with('\r') => LF,
            _ => CRLF,
        };
        for (index, raw) in body.split(LF).enumerate().filter(|_| !text.is_empty()) {
            let line_number = index + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() {
                lines.push(DefLine::Blank);
                continue;
            }
            let entry = parse_line(line, line_number).map_err(|(column, message)| {
                eyre!("{source}:{line_number}:{column}: {message} (expected `id {{target[,target...]}} extra`)")
            })?;
            if let Some(previous) = entries.get(&entry.id) {
                eyre::bail!("{source}:{line_number}:1: id {} is already defined on line {}", entry.id, previous.line);
            }
            lines.push(DefLine::Entry(entry.id));
            entries.insert(entry.id, entry);
        }
        Ok(Self { entries, lines, line_end, final_line_end })
    }

    /// The text of the table (see the module documentation).
    pub fn to_text(&self) -> String {
        let lines: Vec<String> = self
            .lines
            .iter()
            .map(|line| match line {
                DefLine::Entry(id) => self.entries[id].to_line(),
                DefLine::Blank => String::new(),
            })
            .collect();
        let mut text = lines.join(self.line_end);
        if self.final_line_end && !lines.is_empty() {
            text.push_str(self.line_end);
        }
        text
    }

    /// Writes [`Self::to_text`] to `path`.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        std::fs::write(path, self.to_text()).wrap_err_with(|| format!("Failed to write {}", path.display()))
    }

    /// Sets the closed group of `id`: in its line when it has one, else in a new line before the first entry of a
    /// greater id (after the last entry when none is greater).
    pub fn set(&mut self, id: u32, targets: Vec<i32>, extra: i32) -> eyre::Result<()> {
        if targets.is_empty() {
            eyre::bail!("id {id}: a group needs a target");
        }
        let line = self.entries.get(&id).map_or(0, |entry| entry.line);
        if !self.entries.contains_key(&id) {
            let position = self
                .lines
                .iter()
                .position(|line| matches!(line, DefLine::Entry(other) if *other > id))
                .or_else(|| self.lines.iter().rposition(|line| matches!(line, DefLine::Entry(_))).map(|last| last + 1))
                .unwrap_or(0);
            self.lines.insert(position, DefLine::Entry(id));
        }
        self.entries.insert(id, DefTranslation { id, targets, extra: Some(extra), unclosed: false, line });
        Ok(())
    }

    /// Removes the line of `id`; returns whether it had one.
    pub fn remove(&mut self, id: u32) -> bool {
        self.lines.retain(|line| *line != DefLine::Entry(id));
        self.entries.remove(&id).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: u32) -> Option<&DefTranslation> {
        self.entries.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DefTranslation> {
        self.entries.values()
    }
}

/// A cursor over one line, with 1-based columns for errors.
struct Cursor<'a> {
    line: &'a str,
    position: usize,
}

type LineError = (usize, String);

impl<'a> Cursor<'a> {
    fn column(&self) -> usize {
        self.line[..self.position].chars().count() + 1
    }

    fn peek(&self) -> Option<char> {
        self.line[self.position..].chars().next()
    }

    fn error(&self, message: impl Into<String>) -> LineError {
        let found = match self.peek() {
            Some(c) => format!("`{c}`"),
            None => "the end of the line".to_owned(),
        };
        (self.column(), format!("{}, found {found}", message.into()))
    }

    fn skip_space(&mut self) -> usize {
        let start = self.position;
        while let Some(c @ (' ' | '\t')) = self.peek() {
            self.position += c.len_utf8();
        }
        self.position - start
    }

    fn require_space(&mut self, what: &str) -> Result<(), LineError> {
        if self.skip_space() == 0 { Err(self.error(format!("expected white space before {what}"))) } else { Ok(()) }
    }

    fn expect(&mut self, wanted: char) -> Result<(), LineError> {
        if self.peek() == Some(wanted) {
            self.position += wanted.len_utf8();
            Ok(())
        } else {
            Err(self.error(format!("expected `{wanted}`")))
        }
    }

    fn digits(&mut self) -> &'a str {
        let start = self.position;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.position += 1;
        }
        &self.line[start..self.position]
    }

    fn unsigned(&mut self, what: &str) -> Result<u32, LineError> {
        let column = self.column();
        let digits = self.digits();
        if digits.is_empty() {
            return Err(self.error(format!("expected {what} (decimal digits)")));
        }
        digits.parse().map_err(|_| (column, format!("{what} `{digits}` does not fit 32 bits")))
    }

    fn signed(&mut self, what: &str) -> Result<i32, LineError> {
        let column = self.column();
        let negative = self.peek() == Some(NEGATIVE_SIGN);
        if negative {
            self.position += NEGATIVE_SIGN.len_utf8();
        }
        let digits = self.digits();
        if digits.is_empty() {
            return Err(self.error(format!("expected {what} (an integer)")));
        }
        let text = &self.line[column - 1..self.position];
        text.parse().map_err(|_| (column, format!("{what} `{text}` does not fit 32 bits")))
    }
}

fn parse_line(line: &str, line_number: usize) -> Result<DefTranslation, LineError> {
    let mut cursor = Cursor { line, position: 0 };
    let id = cursor.unsigned("the id")?;
    cursor.require_space("the target group")?;
    cursor.expect(GROUP_OPEN)?;
    let mut targets = Vec::new();
    cursor.skip_space();
    loop {
        targets.push(cursor.signed("a target")?);
        let spaced = cursor.skip_space() > 0;
        match cursor.peek() {
            Some(GROUP_SEPARATOR) => {
                cursor.position += GROUP_SEPARATOR.len_utf8();
                cursor.skip_space();
            }
            Some(GROUP_CLOSE) => {
                cursor.position += GROUP_CLOSE.len_utf8();
                break;
            }
            None => return Ok(DefTranslation { id, targets, extra: None, unclosed: true, line: line_number }),
            Some(_) if spaced => {}
            Some(_) => return Err(cursor.error(format!("expected `{GROUP_SEPARATOR}`, `{GROUP_CLOSE}` or white space"))),
        }
    }
    cursor.require_space("the extra value")?;
    let extra = cursor.signed("the extra value")?;
    cursor.skip_space();
    if cursor.peek().is_some() {
        return Err(cursor.error("expected the end of the line"));
    }
    Ok(DefTranslation { id, targets, extra: Some(extra), unclosed: false, line: line_number })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shipped_grammar() {
        let table =
            DefTranslationTable::parse("Sound.def", "654 {487} 0\r\n778 {-1} 0\r\n7 {1, 2,3 4} -4\r\n\r\n805 {331 0\r\n").unwrap();
        assert_eq!(table.len(), 4);
        assert_eq!(table.get(654).unwrap().targets, [487]);
        assert_eq!(table.get(778).unwrap().slots().count(), 0);
        let multi = table.get(7).unwrap();
        assert_eq!((multi.targets.as_slice(), multi.extra, multi.line), (&[1, 2, 3, 4][..], Some(-4), 3));
        let unclosed = table.get(805).unwrap();
        assert_eq!((unclosed.targets.as_slice(), unclosed.extra, unclosed.unclosed), (&[331, 0][..], None, true));
        let music = DefTranslationTable::parse("Music.def", "0 {0} 0").unwrap();
        assert_eq!(music.get(0).unwrap().targets, [0]);
    }

    #[test]
    fn writes_the_shipped_layout_back() {
        for text in [
            "654 {487} 0\r\n778 {-1} 0\r\n805 {331 0\r\n1440 {-1} 0\r\n\r\n",
            "654 {487} 0\r\n805 {331 0\r\n",
            "0 {0} 0",
            "1 {2, 3} 0\n2 {4} 1\n",
            "",
        ] {
            assert_eq!(DefTranslationTable::parse("Sound.def", text).unwrap().to_text(), text);
        }
        let spaced = DefTranslationTable::parse("Sound.def", "1  {2 ,3}\t0\r\n").unwrap();
        assert_eq!(spaced.to_text(), "1 {2, 3} 0\r\n", "other spacing is normalized");

        let mut table = DefTranslationTable::parse("Sound.def", "10 {1} 0\r\n30 {3} 0\r\n\r\n").unwrap();
        table.set(20, vec![2], 0).unwrap();
        table.set(40, vec![NO_TARGET], 0).unwrap();
        table.set(5, vec![0, 9], 0).unwrap();
        table.set(10, vec![11], 1).unwrap();
        assert!(table.remove(30) && !table.remove(30));
        assert_eq!(table.to_text(), "5 {0, 9} 0\r\n10 {11} 1\r\n20 {2} 0\r\n40 {-1} 0\r\n\r\n");
        assert!(table.set(1, Vec::new(), 0).is_err());
        let mut empty = DefTranslationTable::default();
        empty.set(0, vec![0], 0).unwrap();
        assert_eq!(empty.to_text(), "0 {0} 0\r\n");
    }

    #[test]
    fn rejects_anything_else_with_line_and_column() {
        let error = |text: &str| DefTranslationTable::parse("Sound.def", text).unwrap_err().to_string();
        assert!(error("1 {2} 0\n# comment\n").starts_with("Sound.def:2:1: expected the id"), "{}", error("# x"));
        assert!(error("1 {2x} 0").starts_with("Sound.def:1:5:"), "{}", error("1 {2x} 0"));
        assert!(error("1 {2 x").starts_with("Sound.def:1:6: expected a target"), "{}", error("1 {2 x"));
        assert!(error("1 {2} 0 x").contains("1:9: expected the end of the line"), "{}", error("1 {2} 0 x"));
        assert!(error("1 {} 0").contains("expected a target"), "{}", error("1 {} 0"));
        assert!(error("1{2} 0").contains("1:2: expected white space"), "{}", error("1{2} 0"));
        assert!(error("1 {2} 0\n1 {3} 0").contains("already defined on line 1"));
        assert!(error("  \n").contains("1:1:"), "a blank line with spaces is not empty");
    }
}
