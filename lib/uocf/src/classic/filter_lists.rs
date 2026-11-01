//! The chat word filter lists, read strictly and written back byte for byte:
//!
//! - `Uobscene.lst` (Classic clients, named `uobscene.lst` in `client.exe` of every era): UTF-16LE without a byte
//!   order mark, every code unit (line feeds too) shifted up by [`UOBSCENE_SHIFT`].
//! - `Obscene.lst` (shipped next to it, named by no client): printable ASCII rotated by [`OBSCENE_ROTATION`]
//!   within the 95 characters from space to tilde ([`FilterListScheme::Rotated`]); carriage returns and line feeds
//!   are not rotated.
//! - `badwords.lst` (KR and EC; the EC exposes it to Lua as `WindowData.BadWordList`): UTF-16LE without a byte
//!   order mark, not scrambled.
//!
//! The schemes were found from the shipped data: the KR `badwords.lst` is `Uobscene.lst` unshifted, unit for unit,
//! and every `Obscene.lst` line rotated back is one of its words. The clients' readers were not traced, so the
//! line rules are those of the shipped files: one word a line, a line ends at a line feed, optionally after a
//! carriage return (kept per line), and whether the last line ends is kept.
//!
//! Strict where an editor and a reader could disagree: a carriage return that does not end a line, a NUL, an odd
//! byte count or unpaired surrogate in a UTF-16 list, and in `Obscene.lst` a byte that is neither printable ASCII
//! nor a line ending are errors with the line and column ([`DefSyntaxError`]).
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

    /// Whether the shipped list of this scheme ends its lines with a carriage return and line feed.
    fn shipped_crlf(self) -> bool {
        self == Self::Rotated
    }
}

fn rotate(byte: u8, by: u8) -> u8 {
    (byte - FIRST_PRINTABLE + by) % PRINTABLE_COUNT + FIRST_PRINTABLE
}

/// A filter list: its words, one a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterList {
    scheme: FilterListScheme,
    words: Vec<String>,
    /// Whether each line ends with a carriage return before its line feed.
    crlf: Vec<bool>,
    /// Whether the last line ends with a line feed.
    final_line_feed: bool,
}

impl FilterList {
    /// An empty list of `scheme`.
    pub fn new(scheme: FilterListScheme) -> Self {
        Self { scheme, words: Vec::new(), crlf: Vec::new(), final_line_feed: true }
    }

    pub fn from_bytes(scheme: FilterListScheme, bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let text = decode(scheme, bytes)?;
        let mut pieces: Vec<&str> = text.split(LINE_FEED).collect();
        let final_line_feed = pieces.last() == Some(&"");
        if final_line_feed {
            pieces.pop();
        }
        let mut list = Self { scheme, words: Vec::new(), crlf: Vec::new(), final_line_feed };
        for (index, piece) in pieces.into_iter().enumerate() {
            let (word, crlf) = piece.strip_suffix(CARRIAGE_RETURN).map_or((piece, false), |word| (word, true));
            check_word(scheme, word, index + 1)?;
            list.words.push(word.to_owned());
            list.crlf.push(crlf);
        }
        Ok(list)
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
        for (index, (word, &crlf)) in self.words.iter().zip(&self.crlf).enumerate() {
            text.push_str(word);
            if crlf {
                text.push(CARRIAGE_RETURN);
            }
            if index != last || self.final_line_feed {
                text.push(LINE_FEED);
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
        self.words[index] = word.to_owned();
        Ok(())
    }

    /// Adds `word` as a new last line, ending as the first line ends (the shipped ending in an empty list); the
    /// file still ends with a line feed only when it did.
    pub fn push(&mut self, word: &str) -> eyre::Result<()> {
        self.check_new_word(word, self.words.len() + 1)?;
        let crlf = self.crlf.first().copied().unwrap_or(self.scheme.shipped_crlf());
        // Every line but the last ends, so an unended last line gets its line feed and the new one stays unended.
        self.words.push(word.to_owned());
        self.crlf.push(crlf);
        Ok(())
    }

    /// Removes line `index`.
    pub fn remove(&mut self, index: usize) -> eyre::Result<String> {
        if index >= self.words.len() {
            eyre::bail!("line {}: the list has {} lines", index + 1, self.words.len());
        }
        self.crlf.remove(index);
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
            NUL => return Err(error(at + 1, "a NUL inside a word")),
            _ if scheme == FilterListScheme::Rotated && !(c.is_ascii() && (FIRST_PRINTABLE..=LAST_PRINTABLE).contains(&(c as u8))) => {
                return Err(error(at + 1, "Obscene.lst holds printable ASCII only"));
            }
            _ => {}
        }
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
