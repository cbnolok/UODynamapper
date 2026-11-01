//! `Sound.def` and `Music.def` of the Classic client: id translations, one per line, read strictly, with how the
//! client reads each line ([`DefTranslation::client_ignored`], [`DefTranslation::client_targets`]).
//!
//! The grammar, as the shipped files follow it (UO Gold to the current Classic client):
//!
//! ```text
//! file     = *(line EOL) [line]           ; EOL is CRLF or LF; the last line may lack it
//! line     = *WS ["#" *any]               ; blank or comment: skipped by the client
//!          | *WS entry | *WS dropped
//! entry    = id WS "{" targets "}" WS extra [WS ["#" *any]]
//! targets  = *WS int *(*WS "," *WS int) *WS
//! dropped  = id WS "{" *WS int 1*WS int *any ; two targets without a comma between them
//! id       = 1*DIGIT                      ; unique in the file
//! int      = ["-"] 1*DIGIT
//! WS       = 1*(SP | HTAB)
//! ```
//!
//! How the Classic client reads it (`UOClassicDefs::DefTable_ParseLine` in 7.0.103,
//! `scripts/dev-helpers/ec-research/ghidra-audio/results/classic_audio.md`):
//!
//! - Each line is read with `fgets` of 255 bytes, so lines of more than [`MAX_LINE_BYTES`] bytes are refused here.
//!   Numbers are read with `%hd`: ids and targets above [`MAX_DEF_VALUE`] are refused.
//! - After a target the client skips white space and wants `,` or `}`; anything else drops the whole line. Both
//!   releases ship `805 {331 0` (no `}`): the client drops it at `0`. Such a [`DefTranslation::client_ignored`]
//!   line is kept as written, and reported. A line whose group never closes and that the client would not drop is
//!   refused: the client's scan runs past the line end.
//! - A `-` before a target is skipped: `{-1}` is read as target 1. The 351 `{-1}` lines (ids 778 to 1440) are kept
//!   as written; [`DefTranslation::client_targets`] gives what the client reads.
//! - The first line of an id wins in the client; a second one is refused here.
//! - The third field is required, and read but not used for sounds.
//! - A line applies only when the id's own sound slot is missing (an empty slot, or 40 bytes or fewer); then the
//!   client picks one of the targets at random on every play.
//!
//! `Sound.def` (685 ids in UO Gold, 787 in the current client) gives sound ids without a slot of their own the
//! slot to play instead: `654 {487} 0`. Every shipped target list has one value and every third field is 0.
//! `Music.def` holds the single line `0 {0} 0`; **no client reads it** (no `music.def` string in UO Gold, Classic
//! 7.0.103, UOKR.exe or UOSA.exe): Classic music is chosen by `Music/Digital/Config.txt`.
//!
//! [`DefTranslationTable::to_text`] writes a table back: every line as read (blank and comment lines too), a line
//! set after parsing as `id {target, target} extra`, the file's line end (that of its first line; CRLF for a file
//! of one line) and its final line end or none. Every shipped `Sound.def` and `Music.def` round-trips byte for
//! byte.

use std::collections::BTreeMap;
use std::path::Path;

crate::eyre_imports!();

pub const SOUND_DEF_FILE_NAME: &str = "Sound.def";
pub const MUSIC_DEF_FILE_NAME: &str = "Music.def";
const GROUP_OPEN: char = '{';
const GROUP_CLOSE: char = '}';
const GROUP_SEPARATOR: char = ',';
const NEGATIVE_SIGN: char = '-';
const COMMENT_MARK: char = '#';
/// The longest line the client reads whole: `fgets` of 255 bytes holds 254 characters, the line end included.
pub const MAX_LINE_BYTES: usize = 253;
/// The largest id or target the client's `%hd` reads.
pub const MAX_DEF_VALUE: u32 = i16::MAX as u32;
/// The target the shipped `{-1}` lines name, as written.
pub const NO_TARGET: i32 = -1;
const CRLF: &str = "\r\n";
const LF: &str = "\n";
/// Between several targets of a group set after parsing.
const TARGET_JOINER: &str = ", ";

/// One entry line of a `.def` translation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefTranslation {
    pub id: u32,
    /// The targets as written (`-1` stays `-1`).
    pub targets: Vec<i32>,
    /// The third field (0 in every shipped line; not used for sounds); `None` for a group without `}`.
    pub extra: Option<i32>,
    /// The group has no `}`.
    pub unclosed: bool,
    /// Why the client drops the line; `None` when it reads it.
    pub client_ignored: Option<String>,
    /// The line as read, written back as it is; `None` for a line set after parsing.
    pub raw: Option<String>,
    /// The 1-based line number in the parsed file; 0 for an entry set after parsing.
    pub line: usize,
}

/// One line of the file, for writing it back.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DefLine {
    Entry(u32),
    /// A blank or comment line, as read.
    Skipped(String),
}

impl DefTranslation {
    /// The targets the client picks from (one at random on every play): the targets with the `-` dropped, as the
    /// client skips it. `None` when the client drops the line.
    pub fn client_targets(&self) -> Option<Vec<u32>> {
        self.client_ignored.is_none().then(|| self.targets.iter().map(|target| target.unsigned_abs()).collect())
    }

    /// The targets written with a `-`, which the client reads without it.
    pub fn negative_targets(&self) -> impl Iterator<Item = i32> + '_ {
        self.targets.iter().copied().filter(|&target| target < 0)
    }

    /// The line as the table writes it: as read, or `id {target, target} extra`.
    pub fn to_line(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let targets = self.targets.iter().map(i32::to_string).collect::<Vec<_>>().join(TARGET_JOINER);
        format!("{} {GROUP_OPEN}{targets}{GROUP_CLOSE} {}", self.id, self.extra.unwrap_or_default())
    }
}

/// The translations of a `Sound.def` or `Music.def`, by id, with the lines of the file they were read from.
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
            if line.len() > MAX_LINE_BYTES {
                eyre::bail!(
                    "{source}:{line_number}:{}: the line has {} bytes; the client reads {MAX_LINE_BYTES} and parses the rest \
                     as another line",
                    MAX_LINE_BYTES + 1,
                    line.len()
                );
            }
            let content = line.trim_start_matches([' ', '\t']);
            if content.is_empty() || content.starts_with(COMMENT_MARK) {
                lines.push(DefLine::Skipped(line.to_owned()));
                continue;
            }
            let mut entry = parse_line(line, line_number).map_err(|(column, message)| {
                eyre!("{source}:{line_number}:{column}: {message} (expected `id {{target[, target...]}} extra`)")
            })?;
            entry.raw = Some(line.to_owned());
            if let Some(previous) = entries.get(&entry.id) {
                eyre::bail!(
                    "{source}:{line_number}:1: id {} is already defined on line {} (the client keeps the first)",
                    entry.id,
                    previous.line
                );
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
                DefLine::Skipped(text) => text.clone(),
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

    /// Sets the group of `id`: in its line when it has one, else in a new line before the first entry of a greater
    /// id (after the last entry when none is greater). Ids and targets go up to [`MAX_DEF_VALUE`].
    pub fn set(&mut self, id: u32, targets: Vec<i32>, extra: i32) -> eyre::Result<()> {
        if targets.is_empty() {
            eyre::bail!("id {id}: a group needs a target");
        }
        if let Some(value) = std::iter::once(id).chain(targets.iter().map(|target| target.unsigned_abs())).find(|&value| value > MAX_DEF_VALUE) {
            eyre::bail!("id {id}: {value} is above {MAX_DEF_VALUE}, the largest value the client reads");
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
        self.entries.insert(id, DefTranslation { id, targets, extra: Some(extra), unclosed: false, client_ignored: None, raw: None, line });
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

    /// What the client reads differently from the text: one note per line it drops, and one per negative target
    /// value, with the ids that write it.
    pub fn client_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut negative: BTreeMap<i32, Vec<u32>> = BTreeMap::new();
        for entry in self.entries.values() {
            match &entry.client_ignored {
                Some(why) => notes.push(format!("line {}: `{}` is ignored by the client: {why}", entry.line, entry.to_line())),
                None => {
                    for target in entry.negative_targets() {
                        negative.entry(target).or_default().push(entry.id);
                    }
                }
            }
        }
        for (target, ids) in negative {
            notes.push(format!(
                "{} ids write the target {target}, which the client reads as {}: ids {}",
                ids.len(),
                target.unsigned_abs(),
                id_ranges(&ids)
            ));
        }
        notes
    }
}

/// Sorted ids as `a-b, c, ...`.
fn id_ranges(ids: &[u32]) -> String {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for &id in ids {
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == id => *end = id,
            _ => ranges.push((id, id)),
        }
    }
    ranges
        .iter()
        .map(|&(start, end)| if start == end { start.to_string() } else { format!("{start}-{end}") })
        .collect::<Vec<_>>()
        .join(", ")
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
        digits
            .parse()
            .ok()
            .filter(|&value| value <= MAX_DEF_VALUE)
            .ok_or_else(|| (column, format!("{what} `{digits}` is above {MAX_DEF_VALUE}, the largest the client reads")))
    }

    fn starts_number(&self) -> bool {
        self.peek().is_some_and(|c| c.is_ascii_digit() || c == NEGATIVE_SIGN)
    }

    fn signed(&mut self, what: &str, limit: u32) -> Result<i32, LineError> {
        let column = self.column();
        let start = self.position;
        if self.peek() == Some(NEGATIVE_SIGN) {
            self.position += NEGATIVE_SIGN.len_utf8();
        }
        if self.digits().is_empty() {
            return Err(self.error(format!("expected {what} (an integer)")));
        }
        let text = &self.line[start..self.position];
        text.parse::<i32>()
            .ok()
            .filter(|value| value.unsigned_abs() <= limit)
            .ok_or_else(|| (column, format!("{what} `{text}` does not fit the client's read")))
    }
}

fn parse_line(line: &str, line_number: usize) -> Result<DefTranslation, LineError> {
    let mut cursor = Cursor { line, position: 0 };
    cursor.skip_space();
    let id = cursor.unsigned("the id")?;
    cursor.require_space("the target group")?;
    cursor.expect(GROUP_OPEN)?;
    let mut targets = Vec::new();
    cursor.skip_space();
    let entry = |targets, extra, unclosed, client_ignored| DefTranslation {
        id,
        targets,
        extra,
        unclosed,
        client_ignored,
        raw: None,
        line: line_number,
    };
    loop {
        targets.push(cursor.signed("a target", MAX_DEF_VALUE)?);
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
            Some(_) if spaced && cursor.starts_number() => {
                // The client wants `,` or `}` here and drops the line; the rest is kept as written.
                let previous = targets[targets.len() - 1];
                let next_column = cursor.column();
                let rest = &line[cursor.position..];
                let next: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == NEGATIVE_SIGN).collect();
                let why = format!("`{next}` (column {next_column}) follows the target {previous} without a comma");
                let unclosed = !rest.contains(GROUP_CLOSE);
                return Ok(entry(targets, None, unclosed, Some(why)));
            }
            None => {
                return Err(cursor.error(format!(
                    "expected `{GROUP_SEPARATOR}` or `{GROUP_CLOSE}` (the client's scan for them runs past the line end)"
                )));
            }
            Some(_) => return Err(cursor.error(format!("expected `{GROUP_SEPARATOR}` or `{GROUP_CLOSE}`"))),
        }
    }
    cursor.require_space("the extra value")?;
    let extra = cursor.signed("the extra value", i32::MAX.unsigned_abs())?;
    let spaced = cursor.skip_space() > 0;
    match cursor.peek() {
        None => {}
        Some(COMMENT_MARK) if spaced => {}
        Some(_) => return Err(cursor.error(format!("expected the end of the line or a `{COMMENT_MARK}` comment"))),
    }
    Ok(entry(targets, Some(extra), false, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shipped_grammar_as_the_client_reads_it() {
        let text = "654 {487} 0\r\n778 {-1} 0\r\n7 {1, 2 ,3} -4\r\n\r\n805 {331 0\r\n";
        let table = DefTranslationTable::parse("Sound.def", text).unwrap();
        assert_eq!(table.len(), 4);
        assert_eq!(table.get(654).unwrap().client_targets(), Some(vec![487]));
        let negative = table.get(778).unwrap();
        assert_eq!((negative.targets.as_slice(), negative.client_targets()), (&[NO_TARGET][..], Some(vec![1])));
        let multi = table.get(7).unwrap();
        assert_eq!((multi.targets.as_slice(), multi.extra, multi.line), (&[1, 2, 3][..], Some(-4), 3));
        let dropped = table.get(805).unwrap();
        assert_eq!((dropped.targets.as_slice(), dropped.extra, dropped.unclosed), (&[331][..], None, true));
        assert_eq!(dropped.client_targets(), None);
        let notes = table.client_notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("`805 {331 0` is ignored by the client: `0` (column 10) follows the target 331"), "{notes:?}");
        assert_eq!(notes[1], "1 ids write the target -1, which the client reads as 1: ids 778");
        let many = DefTranslationTable::parse("Sound.def", "1 {-1} 0\n2 {-1} 0\n3 {-1} 0\n5 {-1} 0\n").unwrap();
        assert_eq!(many.client_notes(), ["4 ids write the target -1, which the client reads as 1: ids 1-3, 5"]);
        let closed = DefTranslationTable::parse("Sound.def", "9 {1 2} 0").unwrap();
        assert!(closed.get(9).unwrap().client_ignored.is_some() && !closed.get(9).unwrap().unclosed);
        let music = DefTranslationTable::parse("Music.def", "0 {0} 0").unwrap();
        assert_eq!(music.get(0).unwrap().targets, [0]);
        let skipped = DefTranslationTable::parse("Sound.def", "# note\r\n  \r\n  5 {6} 0 # why\r\n").unwrap();
        assert_eq!((skipped.len(), skipped.get(5).unwrap().line), (1, 3));
    }

    #[test]
    fn writes_every_line_back_as_read() {
        for text in [
            "654 {487} 0\r\n778 {-1} 0\r\n805 {331 0\r\n1440 {-1} 0\r\n\r\n",
            "654 {487} 0\r\n805 {331 0\r\n",
            "0 {0} 0",
            "1 {2, 3} 0\n2 {4} 1\n",
            "1  {2 ,3}\t0\r\n# comment\r\n \t\r\n",
            "",
        ] {
            assert_eq!(DefTranslationTable::parse("Sound.def", text).unwrap().to_text(), text);
        }
        let mut table = DefTranslationTable::parse("Sound.def", "10 {1} 0\r\n30 {3} 0\r\n\r\n").unwrap();
        table.set(20, vec![2], 0).unwrap();
        table.set(40, vec![NO_TARGET], 0).unwrap();
        table.set(5, vec![0, 9], 0).unwrap();
        table.set(10, vec![11], 1).unwrap();
        assert!(table.remove(30) && !table.remove(30));
        assert_eq!(table.to_text(), "5 {0, 9} 0\r\n10 {11} 1\r\n20 {2} 0\r\n40 {-1} 0\r\n\r\n");
        assert!(table.set(1, Vec::new(), 0).is_err());
        assert!(table.set(1, vec![i32::from(i16::MAX) + 1], 0).unwrap_err().to_string().contains("above"));
        let mut empty = DefTranslationTable::default();
        empty.set(0, vec![0], 0).unwrap();
        assert_eq!(empty.to_text(), "0 {0} 0\r\n");
    }

    #[test]
    fn rejects_anything_else_with_line_and_column() {
        let error = |text: &str| DefTranslationTable::parse("Sound.def", text).unwrap_err().to_string();
        assert!(error("1 {2} 0\nx\n").starts_with("Sound.def:2:1: expected the id"), "{}", error("1 {2} 0\nx\n"));
        assert!(error("1 {2x} 0").starts_with("Sound.def:1:5:"), "{}", error("1 {2x} 0"));
        assert!(error("1 {2").contains("1:5: expected `,` or `}` (the client's scan"), "{}", error("1 {2"));
        assert!(error("1 {2, x").contains("1:7: expected a target"), "{}", error("1 {2, x"));
        assert!(error("1 {2} 0 x").contains("1:9: expected the end of the line"), "{}", error("1 {2} 0 x"));
        assert!(error("1 {2} 0#x").contains("1:8:"), "{}", error("1 {2} 0#x"));
        assert!(error("1 {} 0").contains("expected a target"), "{}", error("1 {} 0"));
        assert!(error("1 {2}").contains("expected white space before the extra value"), "{}", error("1 {2}"));
        assert!(error("1{2} 0").contains("1:2: expected white space"), "{}", error("1{2} 0"));
        assert!(error("1 {2} 0\n1 {3} 0").contains("already defined on line 1 (the client keeps the first)"));
        assert!(error("40000 {1} 0").contains("above 32767"), "{}", error("40000 {1} 0"));
        assert!(error("1 {40000} 0").contains("does not fit"), "{}", error("1 {40000} 0"));
        let long = format!("1 {{2}} 0 #{}", "x".repeat(MAX_LINE_BYTES));
        assert!(error(&long).contains(&format!("1:{}: the line has", MAX_LINE_BYTES + 1)), "{}", error(&long));
    }
}
