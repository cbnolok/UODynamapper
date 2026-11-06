//! The lines of a Classic text file the client opens in text mode (`"r"` / `"rt"`), as bytes: split strictly
//! where the C runtime or the client's line reader would read something else than an editor shows, and joined
//! back byte for byte.
//!
//! A line ends at a line feed, optionally after a carriage return (kept per line); whether the last line ends is
//! kept. Errors ([`DefSyntaxError`], with the line and column): a Ctrl-Z (text mode stops reading there), a NUL
//! (the client's C strings end there), a carriage return not before a line feed, and a line longer than the
//! reader keeps.

use super::def_files::DefSyntaxError;

const LINE_FEED: u8 = b'\n';
const CARRIAGE_RETURN: u8 = b'\r';
const NUL: u8 = 0;
/// End of file for the C runtime's text mode.
const CTRL_Z: u8 = 0x1a;

/// The lines of a text file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextLines {
    pub lines: Vec<Vec<u8>>,
    /// Whether each line ends with a carriage return before its line feed.
    pub crlf: Vec<bool>,
    /// Whether the last line ends with a line feed.
    pub final_line_feed: bool,
}

impl TextLines {
    /// Splits `bytes`; a line longer than `max_line_bytes` is an error saying `too_long` (what the client does).
    pub fn split(bytes: &[u8], max_line_bytes: usize, too_long: &str) -> Result<Self, DefSyntaxError> {
        let mut pieces: Vec<&[u8]> = bytes.split(|&byte| byte == LINE_FEED).collect();
        let final_line_feed = pieces.last().is_some_and(|piece| piece.is_empty());
        if final_line_feed {
            pieces.pop();
        }
        let mut text = Self { lines: Vec::new(), crlf: Vec::new(), final_line_feed };
        let count = pieces.len();
        for (index, piece) in pieces.into_iter().enumerate() {
            let line = index + 1;
            // An unended last line has no line end to hold a carriage return.
            let ends = index + 1 < count || final_line_feed;
            let stripped = if ends { piece.strip_suffix(&[CARRIAGE_RETURN]) } else { None };
            let (body, crlf) = stripped.map_or((piece, false), |body| (body, true));
            check_line(body, line, max_line_bytes, too_long)?;
            text.lines.push(body.to_vec());
            text.crlf.push(crlf);
        }
        Ok(text)
    }

    pub fn join(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let last = self.lines.len().saturating_sub(1);
        for (index, (line, &crlf)) in self.lines.iter().zip(&self.crlf).enumerate() {
            bytes.extend_from_slice(line);
            if index != last || self.final_line_feed {
                if crlf {
                    bytes.push(CARRIAGE_RETURN);
                }
                bytes.push(LINE_FEED);
            }
        }
        bytes
    }

    /// Adds a last line, ending as the first line ends (CR LF in an empty file); an unended last line gets its
    /// line end (the first line's) and the new one stays unended.
    pub fn push(&mut self, line: Vec<u8>) {
        let crlf = self.crlf.first().copied().unwrap_or(true);
        if !self.final_line_feed && let Some(last) = self.crlf.last_mut() {
            *last = crlf;
        }
        self.lines.push(line);
        self.crlf.push(crlf);
    }

    pub fn insert(&mut self, index: usize, line: Vec<u8>) {
        let crlf = self.crlf.first().copied().unwrap_or(true);
        self.lines.insert(index, line);
        self.crlf.insert(index, crlf);
    }

    pub fn remove(&mut self, index: usize) -> Vec<u8> {
        self.crlf.remove(index);
        self.lines.remove(index)
    }
}

/// Checks the body of line `line` (1-based).
pub(crate) fn check_line(body: &[u8], line: usize, max_line_bytes: usize, too_long: &str) -> Result<(), DefSyntaxError> {
    let error = |column: usize, message: String| DefSyntaxError { line, column, message };
    for (at, &byte) in body.iter().enumerate() {
        match byte {
            CTRL_Z => return Err(error(at + 1, "a Ctrl-Z byte: the client's text-mode read stops there".to_owned())),
            NUL => return Err(error(at + 1, "a NUL: the client's string ends there".to_owned())),
            CARRIAGE_RETURN => return Err(error(at + 1, "a carriage return not before a line feed".to_owned())),
            LINE_FEED => return Err(error(at + 1, "a line feed inside a line".to_owned())),
            _ => {}
        }
    }
    if body.len() > max_line_bytes {
        return Err(error(max_line_bytes + 1, format!("a line of {} bytes: {too_long}", body.len())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 4;

    #[test]
    fn lines_split_and_join_back_byte_for_byte() {
        for bytes in [&b"ab\r\ncd\n"[..], b"ab\r\ncd", b"", b"\n\n"] {
            let text = TextLines::split(bytes, MAX, "too long").unwrap();
            assert_eq!(text.join(), bytes);
        }
        let mut text = TextLines::split(b"a\nb", MAX, "").unwrap();
        text.push(b"c".to_vec());
        assert_eq!(text.join(), b"a\nb\nc");
    }

    #[test]
    fn what_the_client_reads_otherwise_is_an_error() {
        let lone = TextLines::split(b"ab\r\nc\rd\r\n", MAX, "").unwrap_err();
        assert_eq!((lone.line, lone.column), (2, 2));
        assert!(TextLines::split(b"a\x1ab", MAX, "").unwrap_err().message.contains("Ctrl-Z"));
        assert!(TextLines::split(b"a\0b", MAX, "").is_err());
        assert!(TextLines::split(b"a\r", MAX, "").is_err(), "a carriage return ending the file ends no line");
        let long = TextLines::split(b"abcde\n", MAX, "kept short").unwrap_err();
        assert_eq!(long.column, MAX + 1);
        assert!(long.message.contains("kept short"));
    }
}
