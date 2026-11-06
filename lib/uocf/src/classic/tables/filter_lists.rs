//! The chat word filter lists, read strictly and written back byte for byte:
//!
//! - `Uobscene.lst` (Classic clients, named `uobscene.lst` in `client.exe` of every era): UTF-16LE without a byte
//!   order mark, every code unit (line endings too) shifted up by [`UOBSCENE_SHIFT`]. The 2024 client reads it
//!   at start-up and writes it back itself (`ChatFilterLists_Load` / `ChatFilterLists_Save`, see
//!   `scripts/dev-helpers/ec-research/ghidra-classic-ui-files/`), each word followed by a shifted line feed
//!   (`0x17`): the shipped form.
//! - `Obscene.lst` (shipped next to it, named by no client): printable ASCII rotated by [`OBSCENE_ROTATION`]
//!   within the 95 characters from space to tilde ([`FilterListScheme::Rotated`]); carriage returns and line feeds
//!   are not rotated.
//! - `badwords.lst` (KR and EC; the EC exposes it to Lua as `WindowData.BadWordList`): UTF-16LE without a byte
//!   order mark, not scrambled.
//!
//! The schemes were found from the shipped data: the KR `badwords.lst` is `Uobscene.lst` unshifted, unit for unit,
//! and every `Obscene.lst` line rotated back is one of its words.
//!
//! `Uobscene.lst` follows the 2024 client's reader (`Uobscene_ReadWord`): one word a line, a line ends at a
//! carriage return or a line feed, and a carriage return and line feed in either order are one line end (kept per
//! line); an empty line holds no word (the client skips it). Its limits are errors that say what the client would
//! do instead: a word longer than [`UOBSCENE_WORD_UNITS_MAX`] units, more than [`UOBSCENE_WORDS_MAX`] words, a
//! last word without its line end. The other two readers were not traced, so their line rules are those of the
//! shipped files: a line ends at a line feed, optionally after a carriage return (kept per line), and whether the
//! last line ends is kept.
//!
//! Strict where an editor and a reader could disagree: a NUL, an odd byte count or unpaired surrogate in a UTF-16
//! list, in `Obscene.lst` and `badwords.lst` a carriage return that does not end a line, and in `Obscene.lst` a
//! byte that is neither printable ASCII nor a line ending are errors with the line and column
//! ([`DefSyntaxError`]).
//!
//! The words are kept in memory only; nothing here prints them.

crate::eyre_imports!();
use std::path::Path;

use super::def_files::DefSyntaxError;
use crate::utils::output_file::write_through_temp;

pub const UOBSCENE_FILE: &str = "Uobscene.lst";
pub const OBSCENE_FILE: &str = "Obscene.lst";
pub const BADWORDS_FILE: &str = "badwords.lst";
/// What `Uobscene.lst` adds to every UTF-16 code unit.
pub const UOBSCENE_SHIFT: u16 = 13;
/// The UTF-16 units of the client's `Uobscene.lst` word slot, the terminating NUL included.
pub const UOBSCENE_SLOT_UNITS: usize = 30;
/// The longest `Uobscene.lst` word the client's slot holds, in UTF-16 units.
pub const UOBSCENE_WORD_UNITS_MAX: usize = UOBSCENE_SLOT_UNITS - 1;
/// The `Uobscene.lst` words the client's table holds before it runs into the `conf_ignore.lst` table.
pub const UOBSCENE_WORDS_MAX: usize = 120;
/// What `Obscene.lst` adds to a printable character, modulo [`PRINTABLE_COUNT`].
pub const OBSCENE_ROTATION: u8 = 13;
/// The first printable ASCII character (space): the start of the `Obscene.lst` rotation.
pub const FIRST_PRINTABLE: u8 = b' ';
/// The last printable ASCII character (tilde).
pub const LAST_PRINTABLE: u8 = b'~';
/// Printable ASCII characters, space to tilde.
pub const PRINTABLE_COUNT: u8 = LAST_PRINTABLE - FIRST_PRINTABLE + 1;
const UTF16_UNIT_BYTES: usize = 2;
const LINE_FEED: char = '\n';
const CARRIAGE_RETURN: char = '\r';
const NUL: char = '\0';

/// How a filter list stores its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterListScheme {
    /// UTF-16LE, every code unit plus [`UOBSCENE_SHIFT`] (`Uobscene.lst`).
    ShiftedUtf16,
    /// Printable ASCII rotated by [`OBSCENE_ROTATION`] (`Obscene.lst`).
    Rotated,
    /// UTF-16LE (`badwords.lst`).
    Utf16,
}

impl FilterListScheme {
    /// The scheme of a shipped list by its file name, ignoring case.
    pub fn for_file_name(name: &str) -> Option<Self> {
        [(UOBSCENE_FILE, Self::ShiftedUtf16), (OBSCENE_FILE, Self::Rotated), (BADWORDS_FILE, Self::Utf16)]
            .into_iter()
            .find(|(file, _)| file.eq_ignore_ascii_case(name))
            .map(|(_, scheme)| scheme)
    }

    /// How the shipped list of this scheme ends its lines.
    fn shipped_line_end(self) -> LineEnd {
        if self == Self::Rotated { LineEnd::CrLf } else { LineEnd::Lf }
    }

    /// Whether the scheme's reader is the client's `Uobscene.lst` reader (lone carriage returns and line feed then
    /// carriage return pairs end lines; the word limits apply; the last line must end).
    fn client_word_reader(self) -> bool {
        self == Self::ShiftedUtf16
    }
}

/// How a line ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnd {
    Lf,
    CrLf,
    /// A lone carriage return (`Uobscene.lst` only).
    Cr,
    /// A line feed then a carriage return, one line end (`Uobscene.lst` only).
    LfCr,
}

impl LineEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
            Self::LfCr => "\n\r",
        }
    }
}

fn rotate(byte: u8, by: u8) -> u8 {
    (byte - FIRST_PRINTABLE + by) % PRINTABLE_COUNT + FIRST_PRINTABLE
}

/// A filter list: its words, one a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterList {
    scheme: FilterListScheme,
    /// The lines; an empty one holds no word.
    words: Vec<String>,
    /// How each line ends (the last one's only when [`Self::final_line_end`]).
    endings: Vec<LineEnd>,
    /// Whether the last line ends.
    final_line_end: bool,
}

impl FilterList {
    /// An empty list of `scheme`.
    pub fn new(scheme: FilterListScheme) -> Self {
        Self { scheme, words: Vec::new(), endings: Vec::new(), final_line_end: true }
    }

    pub fn from_bytes(scheme: FilterListScheme, bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let chars: Vec<char> = decode(scheme, bytes)?.chars().collect();
        let client = scheme.client_word_reader();
        let mut list = Self::new(scheme);
        let (mut start, mut at) = (0, 0);
        while at < chars.len() {
            let ending = match (chars[at], chars.get(at + 1).copied()) {
                (CARRIAGE_RETURN, Some(LINE_FEED)) => LineEnd::CrLf,
                (LINE_FEED, Some(CARRIAGE_RETURN)) if client => LineEnd::LfCr,
                (LINE_FEED, _) => LineEnd::Lf,
                (CARRIAGE_RETURN, _) if client => LineEnd::Cr,
                (CARRIAGE_RETURN, _) => {
                    let (line, column) = (list.words.len() + 1, at - start + 1);
                    return Err(DefSyntaxError { line, column, message: "a carriage return that does not end the line".to_owned() });
                }
                _ => {
                    at += 1;
                    continue;
                }
            };
            list.push_line(chars[start..at].iter().collect(), ending)?;
            at += ending.as_str().len();
            start = at;
        }
        if start < chars.len() {
            let line = list.words.len() + 1;
            if client {
                return Err(DefSyntaxError {
                    line,
                    column: chars.len() - start + 1,
                    message: "the last word has no line end: the client would read an extra character into it".to_owned(),
                });
            }
            list.push_line(chars[start..].iter().collect(), scheme.shipped_line_end())?;
            list.final_line_end = false;
        }
        Ok(list)
    }

    /// Adds a read line, checked.
    fn push_line(&mut self, word: String, ending: LineEnd) -> Result<(), DefSyntaxError> {
        let line = self.words.len() + 1;
        check_word(self.scheme, &word, line)?;
        if !word.is_empty() {
            self.check_word_count(self.word_count() + 1, line)?;
        }
        self.words.push(word);
        self.endings.push(ending);
        Ok(())
    }

    /// The non-empty lines.
    pub fn word_count(&self) -> usize {
        self.words.iter().filter(|word| !word.is_empty()).count()
    }

    fn check_word_count(&self, count: usize, line: usize) -> Result<(), DefSyntaxError> {
        if self.scheme.client_word_reader() && count > UOBSCENE_WORDS_MAX {
            return Err(DefSyntaxError {
                line,
                column: 1,
                message: format!(
                    "word {count}: the client's table holds {UOBSCENE_WORDS_MAX} words and would write this one over its conf_ignore.lst table"
                ),
            });
        }
        Ok(())
    }

    /// Reads `path`, with the scheme its file name names.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        let scheme = FilterListScheme::for_file_name(&name)
            .ok_or_else(|| eyre!("{}: not {UOBSCENE_FILE}, {OBSCENE_FILE} or {BADWORDS_FILE}", path.display()))?;
        Self::load_as(scheme, path)
    }

    pub fn load_as(scheme: FilterListScheme, path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(scheme, &bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = String::new();
        let last = self.words.len().saturating_sub(1);
        for (index, (word, ending)) in self.words.iter().zip(&self.endings).enumerate() {
            text.push_str(word);
            if index != last || self.final_line_end {
                text.push_str(ending.as_str());
            }
        }
        encode(self.scheme, &text)
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    pub fn scheme(&self) -> FilterListScheme {
        self.scheme
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Replaces the word of line `index`.
    pub fn set(&mut self, index: usize, word: &str) -> eyre::Result<()> {
        if index >= self.words.len() {
            eyre::bail!("line {}: the list has {} lines", index + 1, self.words.len());
        }
        self.check_new_word(word, index + 1)?;
        if self.words[index].is_empty() && !word.is_empty() {
            self.check_word_count(self.word_count() + 1, index + 1).map_err(|error| eyre!("line {}: {}", index + 1, error.message))?;
        }
        self.words[index] = word.to_owned();
        Ok(())
    }

    /// Adds `word` as a new last line, ending as the first line ends (the shipped ending in an empty list); the
    /// file still ends with a line end only when it did.
    pub fn push(&mut self, word: &str) -> eyre::Result<()> {
        let line = self.words.len() + 1;
        self.check_new_word(word, line)?;
        if !word.is_empty() {
            self.check_word_count(self.word_count() + 1, line).map_err(|error| eyre!("line {line}: {}", error.message))?;
        }
        let ending = self.endings.first().copied().unwrap_or(self.scheme.shipped_line_end());
        // Every line but the last ends, so an unended last line gets its line end and the new one stays unended.
        self.words.push(word.to_owned());
        self.endings.push(ending);
        Ok(())
    }

    /// Removes line `index`.
    pub fn remove(&mut self, index: usize) -> eyre::Result<String> {
        if index >= self.words.len() {
            eyre::bail!("line {}: the list has {} lines", index + 1, self.words.len());
        }
        self.endings.remove(index);
        Ok(self.words.remove(index))
    }

    fn check_new_word(&self, word: &str, line: usize) -> eyre::Result<()> {
        if word.contains(CARRIAGE_RETURN) {
            eyre::bail!("line {line}: a carriage return inside a word");
        }
        if word.contains(LINE_FEED) {
            eyre::bail!("line {line}: a line feed inside a word");
        }
        check_word(self.scheme, word, line).map_err(|error| eyre!("line {line}: {}", error.message))
    }
}

fn check_word(scheme: FilterListScheme, word: &str, line: usize) -> Result<(), DefSyntaxError> {
    let error = |column: usize, message: &str| DefSyntaxError { line, column, message: message.to_owned() };
    for (at, c) in word.chars().enumerate() {
        match c {
            CARRIAGE_RETURN => return Err(error(at + 1, "a carriage return that does not end the line")),
            NUL if scheme.client_word_reader() => return Err(error(at + 1, "a NUL inside a word: the client would cut the word there")),
            NUL => return Err(error(at + 1, "a NUL inside a word")),
            _ if scheme == FilterListScheme::Rotated && !(c.is_ascii() && (FIRST_PRINTABLE..=LAST_PRINTABLE).contains(&(c as u8))) => {
                return Err(error(at + 1, "Obscene.lst holds printable ASCII only"));
            }
            _ => {}
        }
    }
    let units = word.encode_utf16().count();
    if scheme.client_word_reader() && units > UOBSCENE_WORD_UNITS_MAX {
        return Err(error(
            UOBSCENE_WORD_UNITS_MAX + 1,
            &format!(
                "a word of {units} UTF-16 units: the client's {UOBSCENE_SLOT_UNITS}-unit slot holds {UOBSCENE_WORD_UNITS_MAX} and its NUL, and the client would run this word into the next one"
            ),
        ));
    }
    Ok(())
}

/// The text of `bytes`: line endings as they are, the words unscrambled.
fn decode(scheme: FilterListScheme, bytes: &[u8]) -> Result<String, DefSyntaxError> {
    match scheme {
        FilterListScheme::Rotated => {
            let (mut line, mut column) = (1, 1);
            let mut text = String::with_capacity(bytes.len());
            for &byte in bytes {
                let c = match byte {
                    b'\n' => {
                        line += 1;
                        column = 0;
                        LINE_FEED
                    }
                    b'\r' => CARRIAGE_RETURN,
                    FIRST_PRINTABLE..=LAST_PRINTABLE => char::from(rotate(byte, PRINTABLE_COUNT - OBSCENE_ROTATION)),
                    _ => {
                        return Err(DefSyntaxError {
                            line,
                            column,
                            message: format!("byte {byte:#04x} is neither printable ASCII nor a line ending"),
                        });
                    }
                };
                text.push(c);
                column += 1;
            }
            Ok(text)
        }
        FilterListScheme::ShiftedUtf16 | FilterListScheme::Utf16 => {
            if bytes.len() % UTF16_UNIT_BYTES != 0 {
                return Err(DefSyntaxError { line: 1, column: 1, message: "an odd byte count: not UTF-16".to_owned() });
            }
            let shift = if scheme == FilterListScheme::ShiftedUtf16 { UOBSCENE_SHIFT } else { 0 };
            let units: Vec<u16> = bytes
                .chunks_exact(UTF16_UNIT_BYTES)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]).wrapping_sub(shift))
                .collect();
            let mut text = String::with_capacity(units.len());
            let (mut line, mut column) = (1, 1);
            for decoded in char::decode_utf16(units) {
                let c = decoded.map_err(|_| DefSyntaxError { line, column, message: "an unpaired UTF-16 surrogate".to_owned() })?;
                if c == LINE_FEED {
                    line += 1;
                    column = 0;
                }
                column += 1;
                text.push(c);
            }
            Ok(text)
        }
    }
}

fn encode(scheme: FilterListScheme, text: &str) -> Vec<u8> {
    match scheme {
        FilterListScheme::Rotated => text
            .bytes()
            .map(|byte| if (FIRST_PRINTABLE..=LAST_PRINTABLE).contains(&byte) { rotate(byte, OBSCENE_ROTATION) } else { byte })
            .collect(),
        FilterListScheme::ShiftedUtf16 | FilterListScheme::Utf16 => {
            let shift = if scheme == FilterListScheme::ShiftedUtf16 { UOBSCENE_SHIFT } else { 0 };
            text.encode_utf16().flat_map(|unit| unit.wrapping_add(shift).to_le_bytes()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, shift: u16) -> Vec<u8> {
        text.encode_utf16().flat_map(|unit| unit.wrapping_add(shift).to_le_bytes()).collect()
    }

    #[test]
    fn each_scheme_reads_and_writes_back_byte_for_byte() {
        let shifted = utf16("alpha\nbeta\n", UOBSCENE_SHIFT);
        let list = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &shifted).unwrap();
        assert_eq!(list.words(), ["alpha", "beta"]);
        assert_eq!(list.to_bytes(), shifted);
        let plain = utf16("alpha\r\nbeta", 0);
        let list = FilterList::from_bytes(FilterListScheme::Utf16, &plain).unwrap();
        assert_eq!(list.words(), ["alpha", "beta"]);
        assert_eq!(list.to_bytes(), plain);
        // 'a' + 13 = 'n'; 'z' + 13 wraps past '~' to '('; ' ' + 13 = '-'.
        let rotated = b"n(-\r\n";
        let list = FilterList::from_bytes(FilterListScheme::Rotated, rotated).unwrap();
        assert_eq!(list.words(), ["az "]);
        assert_eq!(list.to_bytes(), rotated);
    }

    #[test]
    fn edits_keep_the_line_endings() {
        let mut list = FilterList::from_bytes(FilterListScheme::Rotated, b"n\r\n").unwrap();
        list.push("b").unwrap();
        assert_eq!(list.to_bytes(), b"n\r\no\r\n");
        list.set(0, "c").unwrap();
        assert_eq!(list.remove(1).unwrap(), "b");
        assert_eq!(list.to_bytes(), b"p\r\n");
        let mut unterminated = FilterList::from_bytes(FilterListScheme::Utf16, &utf16("a", 0)).unwrap();
        unterminated.push("b").unwrap();
        assert_eq!(unterminated.to_bytes(), utf16("a\nb", 0));
        let mut empty = FilterList::new(FilterListScheme::ShiftedUtf16);
        empty.push("a").unwrap();
        assert_eq!(empty.to_bytes(), utf16("a\n", UOBSCENE_SHIFT));
    }

    /// What `ChatFilterLists_Save` writes: every word's units plus 13, then `0x17` (a line feed plus 13).
    fn client_written(words: &[&str]) -> Vec<u8> {
        const SHIFTED_LINE_FEED: u16 = 0x17;
        let mut bytes = Vec::new();
        for word in words {
            bytes.extend(word.encode_utf16().flat_map(|unit| (unit + UOBSCENE_SHIFT).to_le_bytes()));
            bytes.extend(SHIFTED_LINE_FEED.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_client_written_uobscene_list_reads_and_writes_back_byte_for_byte() {
        let bytes = client_written(&["alpha", "beta", "\u{e9}t\u{e9}"]);
        let mut list = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &bytes).unwrap();
        assert_eq!(list.words(), ["alpha", "beta", "\u{e9}t\u{e9}"]);
        assert_eq!(list.to_bytes(), bytes);
        list.push("gamma").unwrap();
        assert_eq!(list.to_bytes(), client_written(&["alpha", "beta", "\u{e9}t\u{e9}", "gamma"]));
        assert!(FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &[]).unwrap().words().is_empty());
    }

    #[test]
    fn uobscene_takes_every_line_end_the_client_reads() {
        // A lone CR, CR LF, LF CR, LF, and an empty line (CR CR is two ends), as the client reads them.
        let text = "a\rb\r\nc\n\rd\n\r\re\n";
        let bytes = utf16(text, UOBSCENE_SHIFT);
        let list = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &bytes).unwrap();
        assert_eq!(list.words(), ["a", "b", "c", "d", "", "e"]);
        assert_eq!(list.word_count(), 5);
        assert_eq!(list.to_bytes(), bytes);
        // The untraced readers keep the shipped rules.
        assert!(FilterList::from_bytes(FilterListScheme::Utf16, &utf16("a\rb\n", 0)).is_err());
    }

    #[test]
    fn uobscene_limits_say_what_the_client_would_do() {
        let longest = "x".repeat(UOBSCENE_WORD_UNITS_MAX);
        assert!(FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &client_written(&[&longest])).is_ok());
        let long = "x".repeat(UOBSCENE_WORD_UNITS_MAX + 1);
        let error = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &client_written(&[&long])).unwrap_err();
        assert!(error.message.contains("run this word into the next one"), "{}", error.message);
        let full: Vec<&str> = std::iter::repeat_n("w", UOBSCENE_WORDS_MAX).collect();
        let mut list = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &client_written(&full)).unwrap();
        assert!(list.push("").is_ok(), "an empty line holds no word");
        assert!(list.push("w").is_err());
        let over: Vec<&str> = std::iter::repeat_n("w", UOBSCENE_WORDS_MAX + 1).collect();
        let error = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &client_written(&over)).unwrap_err();
        assert_eq!(error.line, UOBSCENE_WORDS_MAX + 1);
        let unended = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &utf16("a\nbc", UOBSCENE_SHIFT)).unwrap_err();
        assert_eq!((unended.line, unended.column), (2, 3));
        assert!(unended.message.contains("extra character"));
        let nul = FilterList::from_bytes(FilterListScheme::ShiftedUtf16, &utf16("a\0b\n", UOBSCENE_SHIFT)).unwrap_err();
        assert!(nul.message.contains("cut the word"));
        let mut empty = FilterList::new(FilterListScheme::ShiftedUtf16);
        assert!(empty.push(&long).is_err());
    }

    #[test]
    fn what_a_reader_could_read_otherwise_is_an_error() {
        let lone_return = FilterList::from_bytes(FilterListScheme::Utf16, &utf16("a\nb\rc\n", 0)).unwrap_err();
        assert_eq!((lone_return.line, lone_return.column), (2, 2));
        assert!(FilterList::from_bytes(FilterListScheme::Utf16, &utf16("a\0b\n", 0)).is_err());
        assert!(FilterList::from_bytes(FilterListScheme::Utf16, b"a\0b").is_err());
        let surrogate = 0xd800u16.to_le_bytes();
        assert!(FilterList::from_bytes(FilterListScheme::Utf16, &surrogate).is_err());
        let control = FilterList::from_bytes(FilterListScheme::Rotated, b"n\r\n\t").unwrap_err();
        assert_eq!((control.line, control.column), (2, 1));
        let mut list = FilterList::new(FilterListScheme::Rotated);
        assert!(list.push("\u{e9}").is_err());
        assert!(list.push("a\nb").is_err());
        assert!(list.set(0, "a").is_err());
        assert_eq!(FilterListScheme::for_file_name("uobscene.LST"), Some(FilterListScheme::ShiftedUtf16));
        assert_eq!(FilterListScheme::for_file_name("words.lst"), None);
    }
}
