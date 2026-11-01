//! The custom-house tables (`walls.txt`, `floors.txt`, `doors.txt`, `misc.txt`, `stairs.txt`, `teleprts.txt`,
//! `roof.txt`) and `suppinfo.txt`, read as the 2024 Classic client reads them, strictly, and written back byte
//! for byte.
//!
//! Traced in the 2024 Classic client (`scripts/dev-helpers/ec-research/ghidra-classic-text-tables`):
//!
//! - The file is opened in text mode and read with `fgets(buffer, 255)`, so a line of more than
//!   [`MAX_LINE_BYTES`] bytes would be read as two. Lines 1 and 2 (the column types and names) are skipped, every
//!   other line is a row.
//! - **House tables** ([`HouseRow`]): `sscanf("%d %d ...")` with one `%d` per number column; the row is kept when
//!   at least the table's minimum is read (the last one or two columns may be left out, they are then 0), else
//!   dropped. Text after the numbers (the shipped `Comment` column) is not read.
//! - **`suppinfo.txt`** ([`SuppInfoRecord`]): `sscanf("%s %d ...")`, a word (the shipped files write `\`, `./`,
//!   `v`, `o`, `*` ...) into 32 bytes, the tile and 14 flags (the client keeps each flag as `!= 0`); a line with
//!   fewer than all 16 is dropped.
//!
//! Strict where the client is lenient: a row it would drop for missing numbers, a number running into other text
//! (`12x` reads as 12), a number out of 32 bits, a `suppinfo.txt` word longer than the client's buffer and a line
//! longer than it reads at once are errors with the line and column ([`DefSyntaxError`]). A line without any
//! number is kept as read: the client drops it as well (the `doors.txt` names row, which comes third, and the
//! section notes of UO Gold's and ML's `suppinfo.txt`).
//!
//! The files are kept line by line in a [`DefDocument`]: an unchanged row is written as read, an edited or added
//! one as its numbers and comment separated by tabs.

use super::def_files::{DefDocument, DefRecord, DefSyntaxError};

/// Bytes of a line the client reads at once, as for the def files.
pub use super::def_files::MAX_LINE_BYTES;

/// Lines at the start of a table the client skips: the column types and the column names.
pub const HEADER_LINES: usize = 2;

pub const WALLS_FILE: &str = "walls.txt";
pub const FLOORS_FILE: &str = "floors.txt";
pub const DOORS_FILE: &str = "doors.txt";
pub const MISC_FILE: &str = "misc.txt";
pub const STAIRS_FILE: &str = "stairs.txt";
pub const TELEPORTERS_FILE: &str = "teleprts.txt";
pub const ROOF_FILE: &str = "roof.txt";
pub const SUPPINFO_FILE: &str = "suppinfo.txt";

/// Number columns of each table, and the fewest the client keeps a row with.
pub const WALLS_COLUMNS: usize = 18;
pub const WALLS_MIN_COLUMNS: usize = 17;
pub const FLOORS_COLUMNS: usize = 18;
pub const FLOORS_MIN_COLUMNS: usize = 17;
pub const DOORS_COLUMNS: usize = 10;
pub const DOORS_MIN_COLUMNS: usize = 9;
pub const MISC_COLUMNS: usize = 12;
pub const MISC_MIN_COLUMNS: usize = 11;
pub const STAIRS_COLUMNS: usize = 15;
pub const STAIRS_MIN_COLUMNS: usize = 14;
pub const TELEPORTERS_COLUMNS: usize = 18;
pub const TELEPORTERS_MIN_COLUMNS: usize = 17;
pub const ROOF_COLUMNS: usize = 20;
pub const ROOF_MIN_COLUMNS: usize = 18;
/// Flags of a `suppinfo.txt` row after the tile.
pub const SUPPINFO_FLAGS: usize = 14;
/// Bytes of the client's buffer for the `suppinfo.txt` word, its terminating NUL among them.
pub const SUPPINFO_WORD_BUFFER_BYTES: usize = 32;

const COLUMN_SEPARATOR: char = '\t';

/// The whitespace `sscanf` skips (`isspace` in the C locale).
fn is_c_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{0B}' | '\u{0C}')
}

/// A line being read: its characters and where an error is.
struct Scanner {
    chars: Vec<char>,
    at: usize,
    line: usize,
}

impl Scanner {
    fn new(text: &str, line: usize) -> Self {
        Self { chars: text.chars().collect(), at: 0, line }
    }

    fn error(&self, at: usize, message: impl Into<String>) -> DefSyntaxError {
        DefSyntaxError { line: self.line, column: at + 1, message: message.into() }
    }

    fn skip_spaces(&mut self) {
        while self.chars.get(self.at).copied().is_some_and(is_c_space) {
            self.at += 1;
        }
    }

    /// `%d`: whitespace, a sign, digits. `None` (nothing consumed) when no digit comes; an error when the number
    /// does not fit 32 bits or runs into other text.
    fn number(&mut self) -> Result<Option<i32>, DefSyntaxError> {
        let start = self.at;
        self.skip_spaces();
        let number_start = self.at;
        let mut at = self.at;
        if matches!(self.chars.get(at), Some('+' | '-')) {
            at += 1;
        }
        let digits = at;
        while self.chars.get(at).is_some_and(char::is_ascii_digit) {
            at += 1;
        }
        if at == digits {
            self.at = start;
            return Ok(None);
        }
        let text: String = self.chars[number_start..at].iter().collect();
        let value = text.parse::<i32>().map_err(|_| self.error(number_start, format!("the number `{text}` does not fit 32 bits")))?;
        if let Some(&next) = self.chars.get(at).filter(|&&next| !is_c_space(next)) {
            return Err(self.error(at, format!("`{}` runs into the number `{text}` (the client reads the number and stops)", next.escape_default())));
        }
        self.at = at;
        Ok(Some(value))
    }

    /// The text after the spaces that follow what was read.
    fn comment(&mut self) -> String {
        self.skip_spaces();
        self.chars[self.at..].iter().collect()
    }
}

/// `values` and `comment` separated by tabs.
fn format_row(prefix: Option<&str>, values: &[i32], comment: &str) -> String {
    let mut fields: Vec<String> = prefix.map(str::to_owned).into_iter().collect();
    fields.extend(values.iter().map(i32::to_string));
    if !comment.is_empty() {
        fields.push(comment.to_owned());
    }
    fields.join(&COLUMN_SEPARATOR.to_string())
}

/// A row of a house table with `COLUMNS` number columns, `MIN_COLUMNS` of which the client needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HouseRow<const COLUMNS: usize, const MIN_COLUMNS: usize> {
    /// `MIN_COLUMNS` to `COLUMNS` numbers, in column order; the client reads a missing one as 0.
    pub values: Vec<i32>,
    /// The text after the numbers, not read by the client (the `Comment` column); empty for none. With fewer than
    /// `COLUMNS` numbers it must not start with a number, which the client would read as the next column.
    pub comment: String,
}

impl<const COLUMNS: usize, const MIN_COLUMNS: usize> HouseRow<COLUMNS, MIN_COLUMNS> {
    /// The values as the client holds them: the missing columns are 0.
    pub fn client_values(&self) -> [i32; COLUMNS] {
        std::array::from_fn(|column| self.values.get(column).copied().unwrap_or(0))
    }
}

impl<const COLUMNS: usize, const MIN_COLUMNS: usize> DefRecord for HouseRow<COLUMNS, MIN_COLUMNS> {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        if line_number <= HEADER_LINES {
            return Ok(None);
        }
        let mut scanner = Scanner::new(line, line_number);
        let mut values = Vec::new();
        while values.len() < COLUMNS {
            match scanner.number()? {
                Some(value) => values.push(value),
                None => break,
            }
        }
        if values.is_empty() {
            return Ok(None);
        }
        if values.len() < MIN_COLUMNS {
            scanner.skip_spaces();
            return Err(scanner.error(scanner.at, format!(
                "a row needs at least {MIN_COLUMNS} of its {COLUMNS} numbers (the client drops it), found {}", values.len()
            )));
        }
        Ok(Some(Self { values, comment: scanner.comment() }))
    }

    fn format(&self) -> String {
        format_row(None, &self.values, &self.comment)
    }
}

pub type WallsRow = HouseRow<WALLS_COLUMNS, WALLS_MIN_COLUMNS>;
pub type FloorsRow = HouseRow<FLOORS_COLUMNS, FLOORS_MIN_COLUMNS>;
pub type DoorsRow = HouseRow<DOORS_COLUMNS, DOORS_MIN_COLUMNS>;
pub type MiscRow = HouseRow<MISC_COLUMNS, MISC_MIN_COLUMNS>;
pub type StairsRow = HouseRow<STAIRS_COLUMNS, STAIRS_MIN_COLUMNS>;
pub type TeleportersRow = HouseRow<TELEPORTERS_COLUMNS, TELEPORTERS_MIN_COLUMNS>;
pub type RoofRow = HouseRow<ROOF_COLUMNS, ROOF_MIN_COLUMNS>;

pub type WallsFile = DefDocument<WallsRow>;
pub type FloorsFile = DefDocument<FloorsRow>;
pub type DoorsFile = DefDocument<DoorsRow>;
pub type MiscFile = DefDocument<MiscRow>;
pub type StairsFile = DefDocument<StairsRow>;
pub type TeleportersFile = DefDocument<TeleportersRow>;
pub type RoofFile = DefDocument<RoofRow>;

/// A `suppinfo.txt` row: a word, the tile, then the flags (`top`, `bottom`, the eight `adj_*`, `directSupports`,
/// `cango W`, `cango N`, `cango NWC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppInfoRecord {
    /// The first word, read and not used by the client.
    pub word: String,
    pub tile: i32,
    /// As written; the client keeps each as `!= 0`.
    pub flags: [i32; SUPPINFO_FLAGS],
    /// The text after the numbers, not read by the client; empty for none.
    pub comment: String,
}

pub type SuppInfoFile = DefDocument<SuppInfoRecord>;

impl DefRecord for SuppInfoRecord {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        if line_number <= HEADER_LINES {
            return Ok(None);
        }
        let mut scanner = Scanner::new(line, line_number);
        scanner.skip_spaces();
        let word_start = scanner.at;
        while scanner.chars.get(scanner.at).is_some_and(|&c| !is_c_space(c)) {
            scanner.at += 1;
        }
        if scanner.at == word_start {
            return Ok(None);
        }
        // A character is a byte: the file is read as Latin-1.
        let word_bytes = scanner.at - word_start;
        if word_bytes >= SUPPINFO_WORD_BUFFER_BYTES {
            return Err(scanner.error(word_start, format!(
                "the first word has {word_bytes} bytes; the client reads it into {SUPPINFO_WORD_BUFFER_BYTES} with its NUL"
            )));
        }
        let word: String = scanner.chars[word_start..scanner.at].iter().collect();
        let mut values = Vec::new();
        while values.len() < 1 + SUPPINFO_FLAGS {
            match scanner.number()? {
                Some(value) => values.push(value),
                None => break,
            }
        }
        if values.is_empty() {
            return Ok(None);
        }
        if values.len() < 1 + SUPPINFO_FLAGS {
            scanner.skip_spaces();
            return Err(scanner.error(scanner.at, format!(
                "a row needs the tile and {SUPPINFO_FLAGS} flags (the client drops it), found {} numbers", values.len()
            )));
        }
        let flags = std::array::from_fn(|flag| values[1 + flag]);
        Ok(Some(Self { word, tile: values[0], flags, comment: scanner.comment() }))
    }

    fn format(&self) -> String {
        let mut values = vec![self.tile];
        values.extend(self.flags);
        format_row(Some(&self.word), &values, &self.comment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "int\tint\tstring\r\nA\tB\tComment\r\n";

    #[test]
    fn rows_read_as_the_client_reads_them_and_write_back() {
        let text = format!("{HEADER}1\t2\t3\tOne\r\n\t\t\t\r\nNames\tRow\r\n4\t5\r\n-6 +7 8\r\n");
        let mut file = DefDocument::<HouseRow<3, 2>>::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        let rows: Vec<_> = file.records().cloned().collect();
        assert_eq!(rows[0], HouseRow { values: vec![1, 2, 3], comment: "One".to_owned() });
        assert_eq!(rows[1].client_values(), [4, 5, 0]);
        assert_eq!(rows[2].values, [-6, 7, 8]);
        assert_eq!(rows.len(), 3);

        file.records_mut().next().unwrap().values[0] = 9;
        assert_eq!(file.to_bytes(), text.replacen("1\t2\t3\tOne", "9\t2\t3\tOne", 1).as_bytes());
        file.push(HouseRow { values: vec![1, 1], comment: String::new() });
        assert!(file.to_bytes().ends_with(b"-6 +7 8\r\n1\t1\r\n"));
    }

    #[test]
    fn rows_the_client_drops_or_misreads_are_errors() {
        let error = |row: &str| DefDocument::<HouseRow<3, 2>>::from_bytes(format!("{HEADER}{row}\r\n").as_bytes()).unwrap_err();
        let short = error("\t5\tComment");
        assert_eq!((short.line, short.column), (3, 4));
        assert!(error("1\t2x\t3").message.contains("runs into"));
        assert!(error("1\t99999999999").message.contains("32 bits"));
        let long = format!("{HEADER}1\t2\t{}\r\n", "c".repeat(MAX_LINE_BYTES));
        assert!(DefDocument::<HouseRow<3, 2>>::from_bytes(long.as_bytes()).is_err());
    }

    #[test]
    fn suppinfo_rows_need_the_word_tile_and_flags() {
        let flags = "\t1".repeat(SUPPINFO_FLAGS);
        let text = format!("{HEADER}\\\t10{flags}\tDark Wood\r\nDOOR TABLE BEGINS HERE\r\n\t\t\tSandstone Arches\r\n");
        let mut file = SuppInfoFile::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), text.as_bytes());
        let row = file.records().next().unwrap().clone();
        assert_eq!((row.word.as_str(), row.tile, row.flags, row.comment.as_str()), ("\\", 10, [1; SUPPINFO_FLAGS], "Dark Wood"));
        assert_eq!(file.records().count(), 1);
        file.records_mut().next().unwrap().flags[0] = 0;
        let edited = String::from_utf8(file.to_bytes()).unwrap();
        assert!(edited.contains(&format!("\\\t10\t0{}\tDark Wood\r\n", "\t1".repeat(SUPPINFO_FLAGS - 1))));

        let short = format!("{HEADER}\\\t10\t1\r\n");
        assert!(SuppInfoFile::from_bytes(short.as_bytes()).unwrap_err().message.contains("flags"));
        let long_word = format!("{HEADER}{}\t10{flags}\r\n", "w".repeat(SUPPINFO_WORD_BUFFER_BYTES));
        assert!(SuppInfoFile::from_bytes(long_word.as_bytes()).unwrap_err().message.contains("first word"));
    }
}
