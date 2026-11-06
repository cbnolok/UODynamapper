//! A text entry as an editor shows it ([`EntryText`]): decoded from its encoding (UTF-8, with or without a byte
//! order mark, or UTF-16 with one: the text kinds of [`classify_content`]; or Windows-1252 for a file that is not
//! UTF-8 but holds no control character, like the EC `GameData.uop` table whose `Prized` ends in an `A0`
//! no-break space), its line endings shown as `\n` when the file uses one kind of ending, and encoded back with
//! the same mark and endings. An unedited text encodes to the bytes it was read from.
//!
//! [`classify_content`]: crate::enhanced::image_census::classify_content

use crate::enhanced::image_census::{classify_content, ContentFormat};

crate::eyre_imports!();

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const UTF16_LE_BOM: &[u8] = b"\xff\xfe";
const UTF16_BE_BOM: &[u8] = b"\xfe\xff";
const UTF16_UNIT_BYTES: usize = 2;
const CRLF: &str = "\r\n";
const LF: char = '\n';
const CR: char = '\r';
const TAB: char = '\t';
const ASCII_DELETE: u8 = 0x7f;
const FIRST_WINDOWS_1252_HIGH: u8 = 0x80;
/// The characters of the bytes `0x80..=0x9F` in Windows-1252; `None` for the five it leaves undefined.
const WINDOWS_1252_HIGH: [Option<char>; 32] = [
    Some('\u{20ac}'), None, Some('\u{201a}'), Some('\u{0192}'), Some('\u{201e}'), Some('\u{2026}'), Some('\u{2020}'), Some('\u{2021}'),
    Some('\u{02c6}'), Some('\u{2030}'), Some('\u{0160}'), Some('\u{2039}'), Some('\u{0152}'), None, Some('\u{017d}'), None,
    None, Some('\u{2018}'), Some('\u{2019}'), Some('\u{201c}'), Some('\u{201d}'), Some('\u{2022}'), Some('\u{2013}'), Some('\u{2014}'),
    Some('\u{02dc}'), Some('\u{2122}'), Some('\u{0161}'), Some('\u{203a}'), Some('\u{0153}'), None, Some('\u{017e}'), Some('\u{0178}'),
];
/// The first byte past the Windows-1252 table: from here a byte is the Latin-1 character of its value.
const LATIN1_FROM: u8 = 0xa0;

/// How a text entry's characters are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    /// UTF-8 after the `EF BB BF` mark.
    Utf8Bom,
    /// UTF-16 little-endian after the `FF FE` mark.
    Utf16Le,
    /// UTF-16 big-endian after the `FE FF` mark.
    Utf16Be,
    /// One byte per character, for a file that is not UTF-8.
    Windows1252,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16Le => "UTF-16 LE with BOM",
            Self::Utf16Be => "UTF-16 BE with BOM",
            Self::Windows1252 => "Windows-1252",
        }
    }

    fn mark(self) -> &'static [u8] {
        match self {
            Self::Utf8 | Self::Windows1252 => &[],
            Self::Utf8Bom => UTF8_BOM,
            Self::Utf16Le => UTF16_LE_BOM,
            Self::Utf16Be => UTF16_BE_BOM,
        }
    }
}

/// The line endings of a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    CrLf,
    Lf,
    Cr,
    /// More than one kind: the text is shown and stored as written.
    Mixed,
    /// A single line: new lines are stored as typed.
    None,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::CrLf => "CRLF",
            Self::Lf => "LF",
            Self::Cr => "CR",
            Self::Mixed => "mixed line endings",
            Self::None => "no line ending",
        }
    }

    /// The endings `text` uses.
    pub fn of(text: &str) -> Self {
        let crlf = text.matches(CRLF).count();
        let lf = text.matches(LF).count() - crlf;
        let cr = text.matches(CR).count() - crlf;
        match (crlf > 0, lf > 0, cr > 0) {
            (false, false, false) => Self::None,
            (true, false, false) => Self::CrLf,
            (false, true, false) => Self::Lf,
            (false, false, true) => Self::Cr,
            _ => Self::Mixed,
        }
    }
}

/// A text entry decoded for editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryText {
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// The text, with `\n` for the line ending when it is [`LineEnding::CrLf`] or [`LineEnding::Cr`].
    pub text: String,
}

impl EntryText {
    /// The text of `bytes` when [`classify_content`] counts them as text, XML or UTF-16 text, or when they are
    /// Windows-1252 text ([`windows_1252_text`]).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (encoding, raw) = match classify_content(bytes).format {
            ContentFormat::Text | ContentFormat::Xml => match bytes.strip_prefix(UTF8_BOM) {
                Some(body) => (TextEncoding::Utf8Bom, std::str::from_utf8(body).ok()?.to_owned()),
                None => (TextEncoding::Utf8, std::str::from_utf8(bytes).ok()?.to_owned()),
            },
            ContentFormat::Utf16Text => {
                let (encoding, body) = match bytes.strip_prefix(UTF16_LE_BOM) {
                    Some(body) => (TextEncoding::Utf16Le, body),
                    None => (TextEncoding::Utf16Be, bytes.strip_prefix(UTF16_BE_BOM)?),
                };
                let units = body.chunks_exact(UTF16_UNIT_BYTES).map(|unit| {
                    let pair = [unit[0], unit[1]];
                    if encoding == TextEncoding::Utf16Le { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) }
                });
                (encoding, char::decode_utf16(units).collect::<Result<String, _>>().ok()?)
            }
            _ => (TextEncoding::Windows1252, windows_1252_text(bytes)?),
        };
        let line_ending = LineEnding::of(&raw);
        let text = match line_ending {
            LineEnding::CrLf => raw.replace(CRLF, "\n"),
            LineEnding::Cr => raw.replace(CR, "\n"),
            LineEnding::Lf | LineEnding::Mixed | LineEnding::None => raw,
        };
        Some(Self { encoding, line_ending, text })
    }

    /// "UTF-8 with BOM, CRLF".
    pub fn label(&self) -> String {
        format!("{}, {}", self.encoding.label(), self.line_ending.label())
    }

    /// `edited` stored like this text: its mark, encoding and (uniform) line ending. A `\r\n` or `\r` typed into a
    /// text of one ending becomes that ending too. Fails on a character Windows-1252 cannot hold, naming its line
    /// and column.
    pub fn encode(&self, edited: &str) -> eyre::Result<Vec<u8>> {
        let stored = match self.line_ending {
            LineEnding::CrLf => edited.replace(CRLF, "\n").replace(CR, "\n").replace(LF, CRLF),
            LineEnding::Cr => edited.replace(CRLF, "\n").replace(LF, "\r"),
            LineEnding::Lf => edited.replace(CRLF, "\n").replace(CR, "\n"),
            LineEnding::Mixed | LineEnding::None => edited.to_owned(),
        };
        let mut bytes = self.encoding.mark().to_vec();
        match self.encoding {
            TextEncoding::Utf8 | TextEncoding::Utf8Bom => bytes.extend_from_slice(stored.as_bytes()),
            TextEncoding::Utf16Le => bytes.extend(stored.encode_utf16().flat_map(u16::to_le_bytes)),
            TextEncoding::Utf16Be => bytes.extend(stored.encode_utf16().flat_map(u16::to_be_bytes)),
            TextEncoding::Windows1252 => {
                for (line, text) in edited.split(LF).enumerate() {
                    if let Some((column, c)) = text.chars().enumerate().find(|&(_, c)| windows_1252_byte(c).is_none()) {
                        eyre::bail!("line {}, column {}: Windows-1252 has no {c:?} (U+{:04X})", line + 1, column + 1, c as u32);
                    }
                }
                bytes.extend(stored.chars().filter_map(windows_1252_byte));
            }
        }
        Ok(bytes)
    }

    /// [`Self::encode`] of the text as decoded.
    pub fn encode_unchanged(&self) -> Vec<u8> {
        self.encode(&self.text).expect("a decoded text encodes")
    }
}

/// The Windows-1252 text of `bytes` that are not UTF-8 and hold no control character but tab, CR and LF, nor a byte
/// the code page leaves undefined.
pub fn windows_1252_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let text: String = bytes.iter().map(|&byte| windows_1252_char(byte)).collect::<Option<_>>()?;
    let control = text.chars().any(|c| c.is_control() && !matches!(c, TAB | LF | CR));
    (!control).then_some(text)
}

fn windows_1252_char(byte: u8) -> Option<char> {
    match byte {
        ASCII_DELETE => None,
        FIRST_WINDOWS_1252_HIGH..LATIN1_FROM => WINDOWS_1252_HIGH[usize::from(byte - FIRST_WINDOWS_1252_HIGH)],
        _ => Some(char::from(byte)),
    }
}

fn windows_1252_byte(c: char) -> Option<u8> {
    match u8::try_from(c as u32) {
        Ok(byte) if byte != ASCII_DELETE && !(FIRST_WINDOWS_1252_HIGH..LATIN1_FROM).contains(&byte) => Some(byte),
        _ => WINDOWS_1252_HIGH
            .iter()
            .position(|&high| high == Some(c))
            .map(|index| FIRST_WINDOWS_1252_HIGH + index as u8),
    }
}

/// The text of a text entry, or why it is none.
pub fn decode_entry_text(bytes: &[u8]) -> eyre::Result<EntryText> {
    EntryText::decode(bytes).ok_or_else(|| eyre!("the entry is {}, not text", classify_content(bytes).format.label()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> EntryText {
        let text = EntryText::decode(bytes).unwrap();
        assert_eq!(text.encode_unchanged(), bytes, "{}", text.label());
        text
    }

    #[test]
    fn keeps_marks_and_line_endings() {
        let crlf = round_trip(b"\xef\xbb\xbfa,b\r\n1,2\r\n");
        assert_eq!((crlf.encoding, crlf.line_ending, crlf.text.as_str()), (TextEncoding::Utf8Bom, LineEnding::CrLf, "a,b\n1,2\n"));
        assert_eq!(crlf.encode("a,b\n1,2\n3,4").unwrap(), b"\xef\xbb\xbfa,b\r\n1,2\r\n3,4");
        assert_eq!(crlf.encode("x\r\ny\rz").unwrap(), b"\xef\xbb\xbfx\r\ny\r\nz", "typed endings become the file's");
        let lf = round_trip(b"-- lua\nfunction f() end\n");
        assert_eq!((lf.encoding, lf.line_ending), (TextEncoding::Utf8, LineEnding::Lf));
        assert_eq!(lf.encode("a\r\nb").unwrap(), b"a\nb");
        let cr = round_trip(b"a\rb\r");
        assert_eq!((cr.line_ending, cr.text.as_str()), (LineEnding::Cr, "a\nb\n"));
        let mixed = round_trip(b"a\r\nb\nc\rd");
        assert_eq!((mixed.line_ending, mixed.text.as_str()), (LineEnding::Mixed, "a\r\nb\nc\rd"));
        assert_eq!(round_trip(b"<?xml version=\"1.0\"?><a/>").line_ending, LineEnding::None);
    }

    #[test]
    fn reads_utf16_with_its_mark() {
        let little: Vec<u8> = UTF16_LE_BOM.iter().copied().chain("é\r\nz".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let text = round_trip(&little);
        assert_eq!((text.encoding, text.line_ending, text.text.as_str()), (TextEncoding::Utf16Le, LineEnding::CrLf, "é\nz"));
        let big: Vec<u8> = UTF16_BE_BOM.iter().copied().chain("q\n".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(round_trip(&big).encoding, TextEncoding::Utf16Be);
        assert!(EntryText::decode(b"\x00\x01\x02binary").is_none());
        assert!(decode_entry_text(b"").unwrap_err().to_string().contains("not text"));
    }

    #[test]
    fn reads_windows_1252_when_not_utf8() {
        let text = round_trip(b"110,Prized\xa0,0\r\n\x80\x9f\r\n");
        assert_eq!((text.encoding, text.text.as_str()), (TextEncoding::Windows1252, "110,Prized\u{a0},0\n\u{20ac}\u{178}\n"));
        assert_eq!(text.encode("é€\n").unwrap(), b"\xe9\x80\r\n");
        let error = text.encode("ok\nnot \u{4e00}").unwrap_err().to_string();
        assert!(error.contains("line 2, column 5"), "{error}");
        assert!(EntryText::decode(b"undefined \x81 byte").is_none());
        assert!(EntryText::decode(b"control \x01 \xa0").is_none());
    }
}
