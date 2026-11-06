//! `credits.dat`, the Classic client's scrolling credits, read as the 2024 client reads it, strictly, and written
//! back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicUiFiles::Credits_Load` and `Credits_ParseColour`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-ui-files`): the file is opened in text mode and read a line at
//! a time (`KeyNames_ReadLine`, [`MAX_CREDITS_LINE_BYTES`] bytes kept). When the line's first token (`strtok`
//! with [`FIRST_TOKEN_DELIMITERS`]) is `!`, the rest of the line (`strtok` with [`DIRECTIVE_DELIMITERS`]) is
//! directives, each a keyword and a value: `color`, `defaultcolor` (also the current colour), `font`,
//! `defaultfont` and `song`; the client skips any other token. Every other line, empty ones included, is a line
//! of text in the current colour and font. A colour is `#RRGGBB` or a decimal read as `0xRRGGBB`; a decimal with
//! a leading `-` is kept as `atol` reads it. The Gold client reads the same keywords.
//!
//! The text is bytes (the shipped files mix Windows-1252 and UTF-8). Strict where the client would read
//! something else than an editor shows: on top of [`TextLines`]' rules (Ctrl-Z, NUL, lone carriage return, a line
//! longer than the client keeps), an unknown directive keyword, a keyword without its value and a value the
//! client would misread are errors with the line and column ([`DefSyntaxError`]).

crate::eyre_imports!();
use std::path::Path;

use crate::classic::def_files::DefSyntaxError;
use crate::classic::text_mode_lines::{TextLines, check_line};
use crate::utils::output_file::write_through_temp;

pub const CREDITS_FILE: &str = "credits.dat";
/// Bytes of a line the client keeps (its 512-byte buffer ends in two NULs).
pub const MAX_CREDITS_LINE_BYTES: usize = 510;
/// What separates the first token of a line.
pub const FIRST_TOKEN_DELIMITERS: &[u8] = b" \t\n\r\x0c=";
/// What separates the tokens after a line's `!`.
pub const DIRECTIVE_DELIMITERS: &[u8] = b" \t\n\r\x0c";
/// The first token of a directive line.
pub const DIRECTIVE_MARK: &[u8] = b"!";
/// What starts a hexadecimal colour.
pub const HEX_COLOUR_MARK: u8 = b'#';
/// Hex digits of a `#RRGGBB` colour.
pub const HEX_COLOUR_DIGITS: usize = 6;
/// The largest decimal colour (`0xRRGGBB`).
pub const MAX_DECIMAL_COLOUR: u32 = 0xff_ffff;
const NEGATIVE_SIGN: u8 = b'-';
const HEX_RADIX: u32 = 16;
const DIRECTIVE_SEPARATOR: &str = " ";

fn too_long() -> String {
    format!("the client keeps {MAX_CREDITS_LINE_BYTES} bytes of a line")
}

/// A colour value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditsColour {
    /// `#RRGGBB`.
    Hex(u32),
    /// A decimal, read as `0xRRGGBB`.
    Decimal(u32),
    /// A decimal with a leading `-`, kept as `atol` reads it.
    Negative(i32),
}

impl CreditsColour {
    fn parse(token: &[u8]) -> Result<Self, String> {
        if let Some(digits) = token.strip_prefix(&[HEX_COLOUR_MARK]) {
            let upper_hex = |byte: &u8| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte);
            if digits.len() != HEX_COLOUR_DIGITS || !digits.iter().all(upper_hex) {
                return Err(format!(
                    "`{}`: the client reads {HEX_COLOUR_DIGITS} hex digits after `#` (upper case, as shipped) and any other character as 0",
                    String::from_utf8_lossy(token)
                ));
            }
            let text = std::str::from_utf8(digits).expect("ASCII digits");
            return Ok(Self::Hex(u32::from_str_radix(text, HEX_RADIX).expect("checked hex digits")));
        }
        if let Some(digits) = token.strip_prefix(&[NEGATIVE_SIGN]) {
            let value = parse_decimal(digits).and_then(|value| i32::try_from(value).ok()).ok_or_else(|| {
                format!("`{}`: not a decimal the client's atol reads whole", String::from_utf8_lossy(token))
            })?;
            return Ok(Self::Negative(-value));
        }
        match parse_decimal(token) {
            Some(value) if value <= u64::from(MAX_DECIMAL_COLOUR) => Ok(Self::Decimal(value as u32)),
            _ => Err(format!(
                "`{}`: not `#RRGGBB` or a decimal up to {MAX_DECIMAL_COLOUR} (0xRRGGBB)",
                String::from_utf8_lossy(token)
            )),
        }
    }

    fn render(self) -> String {
        match self {
            Self::Hex(value) => format!("#{value:0width$X}", width = HEX_COLOUR_DIGITS),
            Self::Decimal(value) => value.to_string(),
            Self::Negative(value) => value.to_string(),
        }
    }
}

/// The digits of `token` as a number, `None` unless it is all ASCII digits within `u64`.
fn parse_decimal(token: &[u8]) -> Option<u64> {
    if token.is_empty() || !token.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(token).ok()?.parse().ok()
}

/// A directive of a `!` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditsDirective {
    Color(CreditsColour),
    /// The default colour, also made the current one.
    DefaultColor(CreditsColour),
    Font(i32),
    DefaultFont(i32),
    /// A music track number.
    Song(i32),
}

impl CreditsDirective {
    const COLOR: &'static str = "color";
    const DEFAULT_COLOR: &'static str = "defaultcolor";
    const FONT: &'static str = "font";
    const DEFAULT_FONT: &'static str = "defaultfont";
    const SONG: &'static str = "song";

    fn render(self) -> String {
        let (keyword, value) = match self {
            Self::Color(colour) => (Self::COLOR, colour.render()),
            Self::DefaultColor(colour) => (Self::DEFAULT_COLOR, colour.render()),
            Self::Font(number) => (Self::FONT, number.to_string()),
            Self::DefaultFont(number) => (Self::DEFAULT_FONT, number.to_string()),
            Self::Song(number) => (Self::SONG, number.to_string()),
        };
        format!("{keyword}{DIRECTIVE_SEPARATOR}{value}")
    }
}

fn parse_number(token: &[u8]) -> Result<i32, String> {
    let negative = token.first() == Some(&NEGATIVE_SIGN);
    let digits = if negative { &token[1..] } else { token };
    let value = parse_decimal(digits)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("`{}`: not a decimal the client's atol reads whole", String::from_utf8_lossy(token)))?;
    Ok(if negative { -value } else { value })
}

/// What a line of `credits.dat` is to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreditsLine {
    /// A line of text (its bytes).
    Text(Vec<u8>),
    Directives(Vec<CreditsDirective>),
}

impl CreditsLine {
    /// The bytes of a directive line holding `directives`: `!`, then each keyword and value, space-separated.
    pub fn directive_line(directives: &[CreditsDirective]) -> Vec<u8> {
        let mut parts = vec![String::from_utf8(DIRECTIVE_MARK.to_vec()).expect("ASCII")];
        parts.extend(directives.iter().map(|directive| directive.render()));
        parts.join(DIRECTIVE_SEPARATOR).into_bytes()
    }

    /// Reads the body of line `line` (1-based) as the client does.
    fn parse(body: &[u8], line: usize) -> Result<Self, DefSyntaxError> {
        let Some(tokens) = directive_tokens(body) else {
            return Ok(Self::Text(body.to_vec()));
        };
        let error = |(column, message): (usize, String)| DefSyntaxError { line, column, message };
        let mut directives = Vec::new();
        let mut tokens = tokens.into_iter();
        while let Some((column, keyword)) = tokens.next() {
            let mut value = || {
                tokens.next().ok_or_else(|| (column, format!("`{}` without its value", String::from_utf8_lossy(keyword))))
            };
            let directive = match keyword {
                k if k == CreditsDirective::COLOR.as_bytes() => {
                    let (at, token) = value().map_err(error)?;
                    CreditsDirective::Color(CreditsColour::parse(token).map_err(|message| error((at, message)))?)
                }
                k if k == CreditsDirective::DEFAULT_COLOR.as_bytes() => {
                    let (at, token) = value().map_err(error)?;
                    CreditsDirective::DefaultColor(CreditsColour::parse(token).map_err(|message| error((at, message)))?)
                }
                k if k == CreditsDirective::FONT.as_bytes() => {
                    let (at, token) = value().map_err(error)?;
                    CreditsDirective::Font(parse_number(token).map_err(|message| error((at, message)))?)
                }
                k if k == CreditsDirective::DEFAULT_FONT.as_bytes() => {
                    let (at, token) = value().map_err(error)?;
                    CreditsDirective::DefaultFont(parse_number(token).map_err(|message| error((at, message)))?)
                }
                k if k == CreditsDirective::SONG.as_bytes() => {
                    let (at, token) = value().map_err(error)?;
                    CreditsDirective::Song(parse_number(token).map_err(|message| error((at, message)))?)
                }
                _ => {
                    return Err(error((
                        column,
                        format!("`{}`: not a directive keyword; the client skips it", String::from_utf8_lossy(keyword)),
                    )));
                }
            };
            directives.push(directive);
        }
        Ok(Self::Directives(directives))
    }
}

/// The tokens after the `!` of a directive line, each with its 1-based column; `None` for a text line.
fn directive_tokens(body: &[u8]) -> Option<Vec<(usize, &[u8])>> {
    let is_first_delimiter = |byte: &u8| FIRST_TOKEN_DELIMITERS.contains(byte);
    let start = body.iter().position(|byte| !is_first_delimiter(byte))?;
    let end = body[start..].iter().position(is_first_delimiter).map_or(body.len(), |length| start + length);
    if &body[start..end] != DIRECTIVE_MARK {
        return None;
    }
    // strtok resumes after the delimiter that ended the first token.
    let mut tokens = Vec::new();
    let mut at = end + 1;
    while at < body.len() {
        if DIRECTIVE_DELIMITERS.contains(&body[at]) {
            at += 1;
            continue;
        }
        let length = body[at..].iter().position(|byte| DIRECTIVE_DELIMITERS.contains(byte)).unwrap_or(body.len() - at);
        tokens.push((at + 1, &body[at..at + length]));
        at += length;
    }
    Some(tokens)
}

/// `credits.dat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditsFile {
    text: TextLines,
    lines: Vec<CreditsLine>,
}

impl CreditsFile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let text = TextLines::split(bytes, MAX_CREDITS_LINE_BYTES, &too_long())?;
        let lines = text.lines.iter().enumerate().map(|(index, body)| CreditsLine::parse(body, index + 1)).collect::<Result<_, _>>()?;
        Ok(Self { text, lines })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.text.join()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    /// The lines as the client reads them.
    pub fn lines(&self) -> &[CreditsLine] {
        &self.lines
    }

    /// The bytes of line `index`.
    pub fn line_bytes(&self, index: usize) -> Option<&[u8]> {
        self.text.lines.get(index).map(Vec::as_slice)
    }

    fn checked(bytes: &[u8], line: usize) -> eyre::Result<CreditsLine> {
        check_line(bytes, line, MAX_CREDITS_LINE_BYTES, &too_long()).map_err(|error| eyre!("line {line}: {}", error.message))?;
        CreditsLine::parse(bytes, line).map_err(|error| eyre!("line {line}, column {}: {}", error.column, error.message))
    }

    /// Replaces line `index` with `bytes`, read as the client reads it.
    pub fn set_line(&mut self, index: usize, bytes: &[u8]) -> eyre::Result<()> {
        if index >= self.lines.len() {
            eyre::bail!("line {}: the file has {} lines", index + 1, self.lines.len());
        }
        self.lines[index] = Self::checked(bytes, index + 1)?;
        self.text.lines[index] = bytes.to_vec();
        Ok(())
    }

    /// Inserts `bytes` as line `index` (the line count to append), ending as the first line ends.
    pub fn insert_line(&mut self, index: usize, bytes: &[u8]) -> eyre::Result<()> {
        if index > self.lines.len() {
            eyre::bail!("line {}: the file has {} lines", index + 1, self.lines.len());
        }
        let line = Self::checked(bytes, index + 1)?;
        if index == self.lines.len() {
            self.text.push(bytes.to_vec());
        } else {
            self.text.insert(index, bytes.to_vec());
        }
        self.lines.insert(index, line);
        Ok(())
    }

    pub fn remove_line(&mut self, index: usize) -> eyre::Result<CreditsLine> {
        if index >= self.lines.len() {
            eyre::bail!("line {}: the file has {} lines", index + 1, self.lines.len());
        }
        self.text.remove(index);
        Ok(self.lines.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"! song 49 song 50\r\n! defaultfont 0\r\n! defaultcolor #9999FF\r\n\r\nUltima Online\r\n! color 255 color -3\r\n\xa9 Team";

    #[test]
    fn credits_read_as_the_client_reads_them_and_write_back() {
        let file = CreditsFile::from_bytes(SAMPLE).unwrap();
        assert_eq!(file.to_bytes(), SAMPLE);
        assert_eq!(file.lines()[0], CreditsLine::Directives(vec![CreditsDirective::Song(49), CreditsDirective::Song(50)]));
        assert_eq!(file.lines()[2], CreditsLine::Directives(vec![CreditsDirective::DefaultColor(CreditsColour::Hex(0x9999ff))]));
        assert_eq!(file.lines()[3], CreditsLine::Text(Vec::new()));
        assert_eq!(
            file.lines()[5],
            CreditsLine::Directives(vec![
                CreditsDirective::Color(CreditsColour::Decimal(255)),
                CreditsDirective::Color(CreditsColour::Negative(-3)),
            ])
        );
        assert_eq!(file.lines()[6], CreditsLine::Text(b"\xa9 Team".to_vec()));
        // `=` ends the first token only; `!x` is text.
        let file = CreditsFile::from_bytes(b" !=font 2\n!x\n").unwrap();
        assert_eq!(file.lines(), [CreditsLine::Directives(vec![CreditsDirective::Font(2)]), CreditsLine::Text(b"!x".to_vec())]);
    }

    #[test]
    fn edits_read_back() {
        let mut file = CreditsFile::from_bytes(SAMPLE).unwrap();
        let directives = [CreditsDirective::Color(CreditsColour::Hex(0xffff00)), CreditsDirective::Song(7)];
        let line = CreditsLine::directive_line(&directives);
        assert_eq!(line, b"! color #FFFF00 song 7");
        file.insert_line(4, &line).unwrap();
        file.insert_line(file.lines().len(), b"The end").unwrap();
        let back = CreditsFile::from_bytes(&file.to_bytes()).unwrap();
        assert_eq!(back.lines()[4], CreditsLine::Directives(directives.to_vec()));
        assert_eq!(back.line_bytes(8), Some(&b"The end"[..]));
        assert!(back.to_bytes().ends_with(b"Team\r\nThe end"), "the old last line got its line end: {:?}", String::from_utf8_lossy(&back.to_bytes()));
        file.remove_line(8).unwrap();
        file.remove_line(4).unwrap();
        assert_eq!(file.to_bytes(), SAMPLE);
        assert!(file.set_line(0, b"! colour #FFFFFF").is_err());
    }

    #[test]
    fn what_the_client_reads_otherwise_is_an_error() {
        for (bytes, column) in [
            (&b"! color"[..], 3),
            (b"! colour #FFFFFF", 3),
            (b"! color #FFFFF", 9),
            (b"! color #ffffff", 9),
            (b"! color 16777216", 9),
            (b"! font 1x", 8),
            (b"! song +3", 8),
        ] {
            let error = CreditsFile::from_bytes(bytes).unwrap_err();
            assert_eq!((error.line, error.column), (1, column), "{}", String::from_utf8_lossy(bytes));
        }
        assert!(CreditsFile::from_bytes(b"a\rb").is_err());
        let long = vec![b'a'; MAX_CREDITS_LINE_BYTES + 1];
        assert!(CreditsFile::from_bytes(&long).is_err());
    }
}
