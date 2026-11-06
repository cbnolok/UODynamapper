//! `helpindx.dat`, the topics of the Classic client's help gump, read as the 2024 client reads it, strictly, and
//! written back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicUiFiles::HelpIndex_Load`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-ui-files`): the file is opened in text mode and read with
//! `fgets` into 512 bytes. A line starting with `#` is a comment. Any other line must be shorter than 511 bytes
//! with its line feed; an optional `@` before it sets the topic's flag to 0 (1 otherwise; what the flag does was
//! not traced), then `strtok` splits it at tabs: the topic id and three cliloc numbers (`atol`; the row is dropped
//! unless all four are nonzero), then any number of linked topic ids (a tab ending the row makes the line feed one
//! more field, read as 0: [`HelpTopic::trailing_tab`]). The shipped file is the same on every
//! client (175 comment lines and the topics, CR LF).
//!
//! Strict where the client would read something else than an editor shows: on top of [`TextLines`]' rules
//! (Ctrl-Z, NUL, lone carriage return), a line `fgets` would split, an empty field (`strtok` skips it), a field
//! that is not decimal digits, optionally followed by spaces (where `atol` stops; the shipped file has one such
//! field), or past `i32`, and a row the client drops (a zero) are errors with the line and column
//! ([`DefSyntaxError`]). Rows keep their bytes; an edited row is written plainly.

crate::eyre_imports!();
use std::path::Path;

use crate::classic::def_files::DefSyntaxError;
use crate::classic::text_mode_lines::TextLines;
use crate::utils::output_file::write_through_temp;

pub const HELP_INDEX_FILE: &str = "helpindx.dat";
/// The `fgets` buffer, the terminating NUL included.
pub const HELP_LINE_BUFFER_BYTES: usize = 512;
/// The client takes a line whose length, its line feed included, is below this.
pub const HELP_LINE_LENGTH_LIMIT: usize = 511;
/// The longest line body the client takes: [`HELP_LINE_LENGTH_LIMIT`] less one, less the line feed.
pub const MAX_HELP_LINE_BYTES: usize = HELP_LINE_LENGTH_LIMIT - 2;
/// What starts a comment line.
pub const COMMENT_MARK: u8 = b'#';
/// What may start a topic row, setting its flag to 0.
pub const FLAG_MARK: u8 = b'@';
/// What separates the fields of a row.
pub const FIELD_SEPARATOR: u8 = b'\t';
/// Cliloc numbers of a topic.
pub const TOPIC_CLILOC_COUNT: usize = 3;
/// What may follow a field's digits (`atol` stops there).
const TRAILING_SPACE: u8 = b' ';

/// A topic row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpTopic {
    /// Whether the row starts with `@` (the client stores the topic's flag as 0 then).
    pub marked: bool,
    pub id: u32,
    /// The three cliloc numbers, as the row holds them.
    pub clilocs: [u32; TOPIC_CLILOC_COUNT],
    /// The linked topic ids.
    pub links: Vec<u32>,
    /// Whether the row ends with a tab (one shipped row does): the client then reads one more field, the line
    /// feed, as the link 0.
    pub trailing_tab: bool,
}

impl HelpTopic {
    /// The row's bytes: `@` when marked, then the fields separated by tabs.
    pub fn to_line(&self) -> Vec<u8> {
        let fields: Vec<String> =
            [self.id].iter().chain(&self.clilocs).chain(&self.links).map(u32::to_string).collect();
        let mut line = if self.marked { vec![FLAG_MARK] } else { Vec::new() };
        line.extend(fields.join(&char::from(FIELD_SEPARATOR).to_string()).into_bytes());
        if self.trailing_tab {
            line.push(FIELD_SEPARATOR);
        }
        line
    }

    /// Reads the body of row `line` (1-based).
    fn parse(body: &[u8], line: usize) -> Result<Self, DefSyntaxError> {
        let error = |column: usize, message: String| DefSyntaxError { line, column, message };
        let marked = body.first() == Some(&FLAG_MARK);
        let start = usize::from(marked);
        let trailing_tab = body.len() > start && body.last() == Some(&FIELD_SEPARATOR);
        let fields = &body[start..body.len() - usize::from(trailing_tab)];
        let mut numbers = Vec::new();
        let mut column = start + 1;
        for field in fields.split(|&byte| byte == FIELD_SEPARATOR) {
            let digits = field.iter().rposition(|&byte| byte != TRAILING_SPACE).map_or(&field[..0], |last| &field[..=last]);
            let text = std::str::from_utf8(digits).unwrap_or_default();
            if field.is_empty() {
                return Err(error(column, "an empty field: strtok skips it, shifting the fields after it".to_owned()));
            }
            let plain = !digits.is_empty() && digits.iter().all(u8::is_ascii_digit);
            let Some(number) = text.parse::<u32>().ok().filter(|_| plain).filter(|&number| i32::try_from(number).is_ok()) else {
                return Err(error(column, format!("`{}`: not a decimal (and trailing spaces) the client's atol reads whole", String::from_utf8_lossy(field))));
            };
            numbers.push(number);
            column += field.len() + 1;
        }
        let required = 1 + TOPIC_CLILOC_COUNT;
        if numbers.len() < required || numbers[..required].contains(&0) {
            return Err(error(
                start + 1,
                format!("a row needs a topic id and {TOPIC_CLILOC_COUNT} cliloc numbers, all nonzero; the client drops this one"),
            ));
        }
        let clilocs = [numbers[1], numbers[2], numbers[3]];
        Ok(Self { marked, id: numbers[0], clilocs, links: numbers[required..].to_vec(), trailing_tab })
    }
}

/// A line of `helpindx.dat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpIndexLine {
    /// A comment, `#` included.
    Comment(Vec<u8>),
    Topic(HelpTopic),
}

impl HelpIndexLine {
    fn parse(body: &[u8], line: usize) -> Result<Self, DefSyntaxError> {
        if body.first() == Some(&COMMENT_MARK) {
            return Ok(Self::Comment(body.to_vec()));
        }
        HelpTopic::parse(body, line).map(Self::Topic)
    }

    fn to_line(&self) -> Vec<u8> {
        match self {
            Self::Comment(bytes) => bytes.clone(),
            Self::Topic(topic) => topic.to_line(),
        }
    }
}

fn too_long() -> String {
    format!(
        "the client takes a line shorter than {HELP_LINE_LENGTH_LIMIT} bytes with its line feed (fgets reads {HELP_LINE_BUFFER_BYTES} with the NUL)"
    )
}

/// `helpindx.dat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpIndexFile {
    text: TextLines,
    lines: Vec<HelpIndexLine>,
}

impl HelpIndexFile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let text = TextLines::split(bytes, MAX_HELP_LINE_BYTES, &too_long())?;
        let lines = text.lines.iter().enumerate().map(|(index, body)| HelpIndexLine::parse(body, index + 1)).collect::<Result<_, _>>()?;
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

    pub fn lines(&self) -> &[HelpIndexLine] {
        &self.lines
    }

    pub fn topics(&self) -> impl Iterator<Item = &HelpTopic> {
        self.lines.iter().filter_map(|line| match line {
            HelpIndexLine::Topic(topic) => Some(topic),
            HelpIndexLine::Comment(_) => None,
        })
    }

    /// Checks `line` as line `index` would read and returns its bytes.
    fn checked(line: &HelpIndexLine, index: usize) -> eyre::Result<Vec<u8>> {
        let bytes = line.to_line();
        crate::classic::text_mode_lines::check_line(&bytes, index + 1, MAX_HELP_LINE_BYTES, &too_long())
            .and_then(|()| HelpIndexLine::parse(&bytes, index + 1))
            .and_then(|read| {
                (read == *line).then_some(()).ok_or_else(|| DefSyntaxError {
                    line: index + 1,
                    column: 1,
                    message: "a comment must start with `#`".to_owned(),
                })
            })
            .map_err(|error| eyre!("line {}: {}", index + 1, error.message))?;
        Ok(bytes)
    }

    /// Replaces line `index`.
    pub fn set_line(&mut self, index: usize, line: HelpIndexLine) -> eyre::Result<()> {
        if index >= self.lines.len() {
            eyre::bail!("line {}: the file has {} lines", index + 1, self.lines.len());
        }
        self.text.lines[index] = Self::checked(&line, index)?;
        self.lines[index] = line;
        Ok(())
    }

    /// Inserts `line` at `index` (the line count to append), ending as the first line ends.
    pub fn insert_line(&mut self, index: usize, line: HelpIndexLine) -> eyre::Result<()> {
        if index > self.lines.len() {
            eyre::bail!("line {}: the file has {} lines", index + 1, self.lines.len());
        }
        let bytes = Self::checked(&line, index)?;
        if index == self.lines.len() {
            self.text.push(bytes);
        } else {
            self.text.insert(index, bytes);
        }
        self.lines.insert(index, line);
        Ok(())
    }

    pub fn remove_line(&mut self, index: usize) -> eyre::Result<HelpIndexLine> {
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

    const SAMPLE: &[u8] = b"# Movement\r\n1\t3010030\t3010030\t3010100\r\n2\t3010031\t3010030\t3010101\t3\t4\r\n@3\t3010032\t3010030\t3010102\r\n";

    #[test]
    fn the_help_index_reads_as_the_client_reads_it_and_writes_back() {
        let file = HelpIndexFile::from_bytes(SAMPLE).unwrap();
        assert_eq!(file.to_bytes(), SAMPLE);
        let topics: Vec<_> = file.topics().collect();
        assert_eq!(topics.len(), 3);
        assert_eq!(*topics[1], HelpTopic { marked: false, id: 2, clilocs: [3010031, 3010030, 3010101], links: vec![3, 4], trailing_tab: false });
        assert!(topics[2].marked);
        assert_eq!(file.lines()[0], HelpIndexLine::Comment(b"# Movement".to_vec()));
        let spaced = b"@92 \t1049551\t3010030\t1049552\r\n";
        let file = HelpIndexFile::from_bytes(spaced).unwrap();
        assert_eq!(file.topics().next().unwrap().id, 92);
        assert_eq!(file.to_bytes(), spaced);
        let tab_ended = b"@112\t1061662\t3010030\t1061663\t\r\n";
        let file = HelpIndexFile::from_bytes(tab_ended).unwrap();
        let topic = file.topics().next().unwrap();
        assert!(topic.trailing_tab && topic.links.is_empty());
        assert_eq!(topic.to_line(), tab_ended[..tab_ended.len() - 2]);
    }

    #[test]
    fn edits_read_back() {
        let mut file = HelpIndexFile::from_bytes(SAMPLE).unwrap();
        let topic = HelpTopic { marked: true, id: 9, clilocs: [1, 2, 3], links: vec![1], trailing_tab: false };
        file.insert_line(2, HelpIndexLine::Topic(topic.clone())).unwrap();
        let back = HelpIndexFile::from_bytes(&file.to_bytes()).unwrap();
        assert_eq!(back.lines()[2], HelpIndexLine::Topic(topic));
        let row = b"\r\n@9\t1\t2\t3\t1\r\n";
        assert!(back.to_bytes().windows(row.len()).any(|window| window == row));
        file.remove_line(2).unwrap();
        assert_eq!(file.to_bytes(), SAMPLE);
        assert!(file.set_line(0, HelpIndexLine::Comment(b"no mark".to_vec())).is_err());
        let zero = HelpTopic { marked: false, id: 0, clilocs: [1, 2, 3], links: Vec::new(), trailing_tab: false };
        assert!(file.set_line(1, HelpIndexLine::Topic(zero)).is_err());
    }

    #[test]
    fn what_the_client_reads_otherwise_is_an_error() {
        for (bytes, column) in [
            (&b"1\t2\t3"[..], 1),
            (b"1\t2\t0\t4", 1),
            (b"@1\t\t2\t3\t4", 4),
            (b"1\t2\t3\t 4", 7),
            (b"1\t2\t3\t2147483648", 7),
            (b"1\t2\t3\t4x", 7),
            (b"1\t2\t3\t4\t\t", 9),
            (b"", 1),
        ] {
            let error = HelpIndexFile::from_bytes(&[bytes, b"\n"].concat()).unwrap_err();
            assert_eq!((error.line, error.column), (1, column), "{}", String::from_utf8_lossy(bytes));
        }
        let long = [b"#".as_slice(), &vec![b'a'; MAX_HELP_LINE_BYTES]].concat();
        assert!(HelpIndexFile::from_bytes(&long).is_err());
    }
}
