//! `keynames.txt`, the key names of the Classic client's macros, read as the 2024 client reads it, strictly, and
//! written back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicDataFiles::KeyNamesAndMacros_Load` and `KeyNames_ReadLine`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-text-tables`): the file is opened in text mode and
//! [`KEY_NAME_COUNT`] lines are read, line `n` naming scan code `n` (`MapVirtualKeyA`); a macro names its key by
//! these names. A line ends at a line feed or a carriage return, at most [`MAX_KEY_NAME_BYTES`] of it are kept, and
//! a file with fewer lines leaves the remaining names empty. The shipped file (the same on every client) has 89
//! lines, the first `NULL`.
//!
//! Strict where the client would read something else than an editor shows: a carriage return inside a line (the
//! client ends the line there), a NUL (it ends the name), a name longer than it keeps and a line after the last
//! one it reads are errors with the line and column ([`DefSyntaxError`]).

crate::eyre_imports!();
use std::path::Path;

use super::def_files::DefSyntaxError;
use crate::utils::output_file::write_through_temp;

pub const KEY_NAMES_FILE: &str = "keynames.txt";
/// Lines the client reads: scan codes 0 to 88.
pub const KEY_NAME_COUNT: usize = 89;
/// Bytes of a line the client keeps (its 512-byte buffer ends in two NULs).
pub const MAX_KEY_NAME_BYTES: usize = 510;
const LINE_FEED: char = '\n';
const CARRIAGE_RETURN: char = '\r';
const NUL: char = '\0';

/// `keynames.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyNamesFile {
    /// The names of scan codes 0 up to at most [`KEY_NAME_COUNT`] - 1, as the lines hold them.
    names: Vec<String>,
    /// Whether each line ends with a carriage return before its line feed.
    crlf: Vec<bool>,
    /// Whether the last line ends with a line feed.
    final_line_feed: bool,
}

fn check_name(name: &str, line: usize) -> Result<(), DefSyntaxError> {
    let error = |column: usize, message: String| DefSyntaxError { line, column, message };
    if let Some(at) = name.chars().position(|c| c == CARRIAGE_RETURN || c == LINE_FEED) {
        return Err(error(at + 1, "a carriage return inside a line (the client ends the line there)".to_owned()));
    }
    if let Some(at) = name.chars().position(|c| c == NUL) {
        return Err(error(at + 1, "a NUL inside a name (the client ends the name there)".to_owned()));
    }
    // A character is a byte: the file is read as Latin-1.
    if name.chars().count() > MAX_KEY_NAME_BYTES {
        return Err(error(MAX_KEY_NAME_BYTES + 1, format!("the client keeps {MAX_KEY_NAME_BYTES} bytes of a line")));
    }
    Ok(())
}

impl KeyNamesFile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
        let mut pieces: Vec<&str> = text.split(LINE_FEED).collect();
        let final_line_feed = pieces.last() == Some(&"");
        if final_line_feed {
            pieces.pop();
        }
        let (mut names, mut crlf) = (Vec::new(), Vec::new());
        for (index, piece) in pieces.into_iter().enumerate() {
            if index == KEY_NAME_COUNT {
                return Err(DefSyntaxError {
                    line: index + 1,
                    column: 1,
                    message: format!("the client reads {KEY_NAME_COUNT} key names; this line is not read"),
                });
            }
            let (name, ends_crlf) = piece.strip_suffix(CARRIAGE_RETURN).map_or((piece, false), |name| (name, true));
            check_name(name, index + 1)?;
            names.push(name.to_owned());
            crlf.push(ends_crlf);
        }
        Ok(Self { names, crlf, final_line_feed })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = String::new();
        let last = self.names.len().saturating_sub(1);
        for (index, (name, &crlf)) in self.names.iter().zip(&self.crlf).enumerate() {
            text.push_str(name);
            if crlf {
                text.push(CARRIAGE_RETURN);
            }
            if index != last || self.final_line_feed {
                text.push(LINE_FEED);
            }
        }
        text.chars().map(|c| c as u8).collect()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    /// The name of `scan_code` as the client reads it: empty past the file's lines.
    pub fn name(&self, scan_code: usize) -> Option<&str> {
        (scan_code < KEY_NAME_COUNT).then(|| self.names.get(scan_code).map_or("", String::as_str))
    }

    /// The lines of the file.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Names `scan_code`; empty lines are added up to it when the file is shorter, ending as the file's lines end.
    pub fn set_name(&mut self, scan_code: usize, name: &str) -> eyre::Result<()> {
        if scan_code >= KEY_NAME_COUNT {
            eyre::bail!("scan code {scan_code}: the client reads {KEY_NAME_COUNT} key names");
        }
        check_name(name, scan_code + 1).map_err(|error| eyre!("scan code {scan_code}: {}", error.message))?;
        if name.chars().any(|c| u32::from(c) > u32::from(u8::MAX)) {
            eyre::bail!("scan code {scan_code}: `{name}` has a character beyond Latin-1");
        }
        let crlf = self.crlf.first().copied().unwrap_or(true);
        if scan_code >= self.names.len() {
            // The old last line gets a line ending before the new ones.
            self.final_line_feed = true;
            self.names.resize(scan_code + 1, String::new());
            self.crlf.resize(scan_code + 1, crlf);
        }
        self.names[scan_code] = name.to_owned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_read_as_the_client_reads_them_and_write_back() {
        let text = "NULL\r\nEsc\r\n1\r\n";
        let mut file = KeyNamesFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        assert_eq!((file.name(1), file.name(5), file.name(KEY_NAME_COUNT)), (Some("Esc"), Some(""), None));
        file.set_name(1, "Escape").unwrap();
        file.set_name(4, "3").unwrap();
        assert_eq!(file.to_bytes(), b"NULL\r\nEscape\r\n1\r\n\r\n3\r\n");
        let unterminated = KeyNamesFile::from_bytes(b"NULL\nEsc").unwrap();
        assert_eq!(unterminated.to_bytes(), b"NULL\nEsc");
    }

    #[test]
    fn lines_the_client_reads_otherwise_are_errors() {
        let lone_return = KeyNamesFile::from_bytes(b"NULL\r\nE\rsc\r\n").unwrap_err();
        assert_eq!((lone_return.line, lone_return.column), (2, 2));
        assert!(KeyNamesFile::from_bytes(b"N\0ULL\r\n").unwrap_err().message.contains("NUL"));
        let too_many = "k\r\n".repeat(KEY_NAME_COUNT + 1);
        assert_eq!(KeyNamesFile::from_bytes(too_many.as_bytes()).unwrap_err().line, KEY_NAME_COUNT + 1);
        let long = "k".repeat(MAX_KEY_NAME_BYTES + 1);
        assert!(KeyNamesFile::from_bytes(long.as_bytes()).is_err());
        let mut file = KeyNamesFile::from_bytes(b"NULL\r\n").unwrap();
        assert!(file.set_name(KEY_NAME_COUNT, "x").is_err());
        assert!(file.set_name(0, "a\rb").is_err());
    }
}
