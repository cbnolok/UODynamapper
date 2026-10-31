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

/// One line of a `.def` translation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefTranslation {
    pub id: u32,
    pub targets: Vec<i32>,
    /// The third field (0 in every shipped line; its meaning is not known); `None` for an unclosed group.
    pub extra: Option<i32>,
    /// The group has no `}`: the targets run to the line end.
    pub unclosed: bool,
    /// The 1-based line number.
    pub line: usize,
}

impl DefTranslation {
    /// The targets naming a slot, in file order.
    pub fn slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.targets.iter().filter_map(|&target| u32::try_from(target).ok())
    }
}

/// The translations of a `Sound.def` or `Music.def`, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefTranslationTable {
    entries: BTreeMap<u32, DefTranslation>,
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
        for (index, raw) in text.split('\n').enumerate() {
            let line_number = index + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() {
                continue;
            }
            let entry = parse_line(line, line_number).map_err(|(column, message)| {
                eyre!("{source}:{line_number}:{column}: {message} (expected `id {{target[,target...]}} extra`)")
            })?;
            if let Some(previous) = entries.get(&entry.id) {
                eyre::bail!("{source}:{line_number}:1: id {} is already defined on line {}", entry.id, previous.line);
            }
            entries.insert(entry.id, entry);
        }
        Ok(Self { entries })
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
