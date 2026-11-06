//! The CSV tables of `GameData.uop` ([`GameDataTable`]), read as the client's CSV reader reads them and written
//! back byte for byte, with the typed readers of the tables whose meaning is known ([`LegacyContainer`],
//! [`WeaponAbility`], [`MobArt`], [`BuffDefinition`]).
//!
//! The shipped grammar, measured on the 78 EC and 63 KR tables (`docs/agent_notes/ec_uop_coverage.md`):
//!
//! - **Lines.** CRLF everywhere; some tables lack the final one. Lines are at most a few hundred characters (the
//!   client's line buffer holds [`MAX_LINE_CHARS`]).
//! - **Encoding.** UTF-8 without a BOM, except `itemprop.csv` (EC), Windows-1252 for one `A0` byte.
//! - **Fields.** Comma-separated, as Excel saves them: a `"` toggles quoting and is dropped (a quoted field may
//!   hold commas), no escapes, no trimming (some fields keep spaces). Every line holds an even number of `"`.
//! - **Rows.** Comment rows start with `#` (the craft and house tables open with a `#int,int,...,string` row
//!   naming the column types; the house tables carry an `#` preamble), blank lines sit between records in the
//!   clothing tables, and the column names are the row just before the first record (`abilities.csv` puts a
//!   `,,,,,Buff,...` row above them). Every record starts with an integer id.
//! - **Shape.** Records mostly have the header's field count; `elfskinhues.csv`, `playerstats.csv` (shorter rows)
//!   and `mobart.csv` (16 rows with two more, empty or spaces) do not. Columns hold integers, `TRUE`/`FALSE`,
//!   or text, one kind per column but where text is allowed.
//!
//! The reader is the client's `ReadLine`/`ReadStringField` of the audio tables (Ghidra-confirmed for UOSA.exe's
//! audio CSVs); that the GameData tables go through the same reader is probable, not traced. It is strict: a
//! line longer than the client reads, an unclosed quote, mixed line endings or a control character are refused,
//! with the line and column. A replacement of a shipped table must also keep its shape ([`keeps_shape`]).

use std::collections::{BTreeSet, HashSet};

use crate::enhanced::entry_text::{EntryText, LineEnding, TextEncoding};

crate::eyre_imports!();

/// The characters a line of the client's CSV reader holds.
pub const MAX_LINE_CHARS: usize = 0x7fe;
const SEPARATOR: char = ',';
const QUOTE: char = '"';
const COMMENT_MARK: char = '#';
const TAB: char = '\t';
const BOOLEAN_VALUES: [&str; 2] = ["TRUE", "FALSE"];

/// A row: its text as written and its fields as the client reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRow {
    raw: String,
    /// The fields as written (quotes kept), for rewriting one field.
    raw_fields: Vec<String>,
    fields: Vec<String>,
}

impl CsvRow {
    /// Splits `raw` (line `line`, 1-based) on the commas outside quotes.
    fn parse(raw: &str, line: usize) -> eyre::Result<Self> {
        let mut raw_fields = vec![String::new()];
        let mut fields = vec![String::new()];
        let mut quoted_since = None;
        for (column, c) in raw.chars().enumerate() {
            if c.is_control() && c != TAB {
                eyre::bail!("line {line}, column {}: control character {c:?}", column + 1);
            }
            match c {
                QUOTE => quoted_since = if quoted_since.is_some() { None } else { Some(column) },
                SEPARATOR if quoted_since.is_none() => {
                    raw_fields.push(String::new());
                    fields.push(String::new());
                    continue;
                }
                _ => fields.last_mut().expect("one field").push(c),
            }
            raw_fields.last_mut().expect("one field").push(c);
        }
        if let Some(column) = quoted_since {
            eyre::bail!("line {line}, column {}: the quote is never closed", column + 1);
        }
        Ok(Self { raw: raw.to_owned(), raw_fields, fields })
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The fields as the client reads them: quotes dropped, nothing trimmed.
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    pub fn field(&self, column: usize) -> Option<&str> {
        self.fields.get(column).map(String::as_str)
    }

    fn is_comment(&self) -> bool {
        self.fields[0].starts_with(COMMENT_MARK)
    }

    /// Sets the field `column` (adding empty fields up to it), quoted when it holds a comma.
    fn set(&mut self, column: usize, value: &str) -> eyre::Result<()> {
        if let Some(bad) = value.chars().find(|&c| c == QUOTE || (c.is_control() && c != TAB)) {
            eyre::bail!("a field cannot hold {bad:?} (the client drops quotes and ends the line at CR or LF)");
        }
        while self.fields.len() <= column {
            self.fields.push(String::new());
            self.raw_fields.push(String::new());
        }
        self.fields[column] = value.to_owned();
        self.raw_fields[column] = if value.contains(SEPARATOR) { format!("{QUOTE}{value}{QUOTE}") } else { value.to_owned() };
        self.raw = self.raw_fields.join(&SEPARATOR.to_string());
        Ok(())
    }
}

/// One line of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableLine {
    Blank,
    /// A row whose first field starts with `#`.
    Comment(CsvRow),
    Row(CsvRow),
}

/// A GameData CSV table as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataTable {
    encoding: TextEncoding,
    line_ending: LineEnding,
    final_line_ending: bool,
    lines: Vec<TableLine>,
}

impl GameDataTable {
    /// Reads a table; `label` names it in errors.
    pub fn parse(label: &str, bytes: &[u8]) -> eyre::Result<Self> {
        Self::parse_bytes(bytes).wrap_err_with(|| format!("{label}: not a GameData table"))
    }

    fn parse_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let text = EntryText::decode(bytes).ok_or_else(|| eyre!("not text"))?;
        if matches!(text.encoding, TextEncoding::Utf16Le | TextEncoding::Utf16Be) {
            eyre::bail!("{}: the client reads the tables as bytes", text.encoding.label());
        }
        if text.line_ending == LineEnding::Mixed {
            eyre::bail!("mixed line endings");
        }
        let body = text.text.strip_suffix('\n');
        let final_line_ending = body.is_some();
        let lines = body
            .unwrap_or(&text.text)
            .split('\n')
            .enumerate()
            .map(|(index, raw)| {
                let line = index + 1;
                let length = raw.chars().count();
                if length > MAX_LINE_CHARS {
                    eyre::bail!("line {line}: {length} characters; the client reads at most {MAX_LINE_CHARS}");
                }
                if raw.is_empty() {
                    return Ok(TableLine::Blank);
                }
                let row = CsvRow::parse(raw, line)?;
                Ok(if row.is_comment() { TableLine::Comment(row) } else { TableLine::Row(row) })
            })
            .collect::<eyre::Result<_>>()?;
        Ok(Self { encoding: text.encoding, line_ending: text.line_ending, final_line_ending, lines })
    }

    /// The table as written: an unedited table gives back the bytes it was read from.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut text = self
            .lines
            .iter()
            .map(|line| match line {
                TableLine::Blank => "",
                TableLine::Comment(row) | TableLine::Row(row) => row.raw(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        if self.final_line_ending {
            text.push('\n');
        }
        EntryText { encoding: self.encoding, line_ending: self.line_ending, text: String::new() }.encode(&text)
    }

    pub fn lines(&self) -> &[TableLine] {
        &self.lines
    }

    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// The rows that are not comments, with their line index.
    fn rows(&self) -> impl Iterator<Item = (usize, &CsvRow)> {
        self.lines.iter().enumerate().filter_map(|(index, line)| match line {
            TableLine::Row(row) => Some((index, row)),
            TableLine::Blank | TableLine::Comment(_) => None,
        })
    }

    /// The line index of the column names: the row before the first row that starts with an integer, else the
    /// first row.
    pub fn header_index(&self) -> Option<usize> {
        let rows: Vec<(usize, &CsvRow)> = self.rows().collect();
        match rows.iter().position(|(_, row)| is_integer(&row.fields[0])) {
            Some(0) | None => rows.first().map(|(index, _)| *index),
            Some(first_record) => Some(rows[first_record - 1].0),
        }
    }

    pub fn header(&self) -> Option<&[String]> {
        let index = self.header_index()?;
        match &self.lines[index] {
            TableLine::Row(row) => Some(row.fields()),
            TableLine::Blank | TableLine::Comment(_) => None,
        }
    }

    /// The column named `name`.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header()?.iter().position(|column| column == name)
    }

    /// The records: the rows after the column names, with their line index.
    pub fn records(&self) -> impl Iterator<Item = (usize, &CsvRow)> {
        let header = self.header_index();
        self.rows().filter(move |(index, _)| header.is_some_and(|header| *index > header))
    }

    /// Sets field `column` of the row at line index `line` (a record, the header or a comment).
    pub fn set_field(&mut self, line: usize, column: usize, value: &str) -> eyre::Result<()> {
        let header = self.header_index();
        let row = match self.lines.get_mut(line) {
            Some(TableLine::Row(row) | TableLine::Comment(row)) => row,
            Some(TableLine::Blank) => eyre::bail!("line {}: a blank line has no fields", line + 1),
            None => eyre::bail!("line {}: past the table's {} lines", line + 1, self.lines.len()),
        };
        let mut edited = row.clone();
        edited.set(column, value).wrap_err_with(|| format!("line {}, field {}", line + 1, column + 1))?;
        if edited.raw.chars().count() > MAX_LINE_CHARS {
            eyre::bail!("line {}: the edit makes it longer than the {MAX_LINE_CHARS} characters the client reads", line + 1);
        }
        if header == Some(line) || edited.is_comment() != row.is_comment() {
            eyre::bail!("line {}: an edit cannot turn a row into a comment or back, nor rename a column", line + 1);
        }
        *row = edited;
        Ok(())
    }

    /// Appends a record of `values`.
    pub fn push_record(&mut self, values: &[&str]) -> eyre::Result<()> {
        let first = values.first().ok_or_else(|| eyre!("a record needs an id"))?;
        if !is_integer(first) {
            eyre::bail!("a record starts with an integer id, not `{first}`");
        }
        let mut row = CsvRow { raw: String::new(), raw_fields: vec![String::new()], fields: vec![String::new()] };
        for (column, value) in values.iter().enumerate() {
            row.set(column, value).wrap_err_with(|| format!("field {}", column + 1))?;
        }
        if row.raw.chars().count() > MAX_LINE_CHARS {
            eyre::bail!("the record is longer than the {MAX_LINE_CHARS} characters the client reads");
        }
        self.lines.push(TableLine::Row(row));
        Ok(())
    }

    /// The integer of field `column` of `row` (line index `line`), or why not.
    pub fn integer(&self, line: usize, row: &CsvRow, column: usize) -> eyre::Result<i32> {
        let name = self.header().and_then(|header| header.get(column)).map_or_else(String::new, |name| format!(" ({name})"));
        let value = row.field(column).ok_or_else(|| eyre!("line {}: no field {}{name}", line + 1, column + 1))?;
        parse_integer(value).ok_or_else(|| eyre!("line {}, field {}{name}: `{value}` is not an integer", line + 1, column + 1))
    }
}

/// An integer as the client's `atol` reads it whole: an optional `-` and decimal digits, within 32 bits.
fn parse_integer(value: &str) -> Option<i32> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then(|| value.parse().ok()).flatten()
}

fn is_integer(value: &str) -> bool {
    parse_integer(value).is_some()
}

/// What a column of a table holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ValueKind {
    Empty,
    Integer,
    Boolean,
    Text,
}

impl ValueKind {
    fn of(value: &str) -> Self {
        if value.is_empty() {
            Self::Empty
        } else if is_integer(value) {
            Self::Integer
        } else if BOOLEAN_VALUES.contains(&value) {
            Self::Boolean
        } else {
            Self::Text
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Integer => "an integer",
            Self::Boolean => "TRUE or FALSE",
            Self::Text => "text",
        }
    }
}

/// Whether `file` keeps the shape of the table `current` it replaces: the same column names, record field
/// counts the current records have, the kind of every column (an integer column takes integers, empty only
/// where the current one has an empty value; a `TRUE`/`FALSE` column takes those; a column with text takes
/// anything), and unique ids where the current ids are.
pub fn keeps_shape(current: &[u8], file: &[u8]) -> eyre::Result<()> {
    let current = GameDataTable::parse_bytes(current).wrap_err("the current entry")?;
    let file = GameDataTable::parse_bytes(file)?;
    let header = current.header().unwrap_or_default();
    if file.header().unwrap_or_default() != header {
        let line = file.header_index().map_or(0, |index| index + 1);
        eyre::bail!("line {line}: the column names must stay `{}`", header.join(","));
    }
    let counts: BTreeSet<usize> = current.records().map(|(_, row)| row.fields.len()).collect();
    let kinds: Vec<BTreeSet<ValueKind>> = (0..header.len())
        .map(|column| current.records().filter_map(|(_, row)| row.field(column)).map(ValueKind::of).collect())
        .collect();
    let unique_ids = {
        let mut seen = HashSet::new();
        current.records().all(|(_, row)| seen.insert(row.fields[0].clone()))
    };
    let mut seen = HashSet::new();
    for (index, row) in file.records() {
        let line = index + 1;
        if !counts.is_empty() && !counts.contains(&row.fields.len()) {
            let shipped: Vec<String> = counts.iter().map(usize::to_string).collect();
            eyre::bail!("line {line}: {} fields; the table's records have {}", row.fields.len(), shipped.join(" or "));
        }
        for (column, allowed) in kinds.iter().enumerate() {
            let Some(value) = row.field(column) else { continue };
            let kind = ValueKind::of(value);
            if allowed.is_empty() || allowed.contains(&ValueKind::Text) || allowed.contains(&kind) {
                continue;
            }
            let wanted: Vec<&str> = allowed.iter().map(|kind| kind.label()).collect();
            eyre::bail!("line {line}, field {} ({}): `{value}` is {}; the column holds {}", column + 1, header[column], kind.label(), wanted.join(" or "));
        }
        if unique_ids && !seen.insert(row.fields[0].clone()) {
            eyre::bail!("line {line}: the id {} is already used (the table's ids are unique)", row.fields[0]);
        }
    }
    Ok(())
}

// region: --- Typed tables

/// The typed readers' column lookup: `names` must be the table's column names, in order.
fn typed_records<'a>(table: &'a GameDataTable, names: &[&str]) -> eyre::Result<Vec<(usize, &'a CsvRow)>> {
    let header = table.header().unwrap_or_default();
    if header.len() < names.len() || header.iter().zip(names).any(|(found, wanted)| found != wanted) {
        eyre::bail!("the columns are `{}`, not `{}`", header.join(","), names.join(","));
    }
    Ok(table.records().collect())
}

fn text(row: &CsvRow, column: usize) -> String {
    row.field(column).unwrap_or_default().to_owned()
}

/// `legacycontainers.csv` (EC; UOSA.exe opens it as `data/gamedata/legacycontainers.csv`): the item area of a
/// Classic container gump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyContainer {
    pub gump_id: i32,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub name: String,
}

impl LegacyContainer {
    pub const ENTRY: &str = "data/gamedata/legacycontainers.csv";
    pub const COLUMNS: [&str; 6] = ["gumpId", "left", "top", "right", "bottom", "name"];

    pub fn read(table: &GameDataTable) -> eyre::Result<Vec<Self>> {
        typed_records(table, &Self::COLUMNS)?
            .into_iter()
            .map(|(line, row)| {
                let int = |column| table.integer(line, row, column);
                Ok(Self { gump_id: int(0)?, left: int(1)?, top: int(2)?, right: int(3)?, bottom: int(4)?, name: text(row, 5) })
            })
            .collect()
    }
}

/// `weaponability.csv` (EC and KR): a weapon's two special moves, by name and id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponAbility {
    pub object_id: i32,
    pub primary_ability: String,
    pub secondary_ability: String,
    pub primary_id: i32,
    pub secondary_id: i32,
}

impl WeaponAbility {
    pub const ENTRY: &str = "data/gamedata/weaponability.csv";
    pub const COLUMNS: [&str; 5] = ["objectId", "primaryAbility", "secondaryAbility", "primaryId", "secondaryId"];

    pub fn read(table: &GameDataTable) -> eyre::Result<Vec<Self>> {
        typed_records(table, &Self::COLUMNS)?
            .into_iter()
            .map(|(line, row)| {
                let int = |column| table.integer(line, row, column);
                Ok(Self { object_id: int(0)?, primary_ability: text(row, 1), secondary_ability: text(row, 2), primary_id: int(3)?, secondary_id: int(4)? })
            })
            .collect()
    }
}

/// `mobart.csv` (EC and KR): the portrait texture of a body type. 16 rows carry two more fields, empty or
/// spaces, kept as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobArt {
    pub body_type: i32,
    pub file_name: String,
    pub description: String,
}

impl MobArt {
    pub const ENTRY: &str = "data/gamedata/mobart.csv";
    pub const COLUMNS: [&str; 3] = ["BodyType", "Filename", "Description"];

    pub fn read(table: &GameDataTable) -> eyre::Result<Vec<Self>> {
        typed_records(table, &Self::COLUMNS)?
            .into_iter()
            .map(|(line, row)| Ok(Self { body_type: table.integer(line, row, 0)?, file_name: text(row, 1), description: text(row, 2) }))
            .collect()
    }
}

/// `buffdata.csv` (EC and KR): a buff icon and the server's id for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuffDefinition {
    pub id: i32,
    pub define: String,
    pub icon_id: i32,
    pub server_id: i32,
}

impl BuffDefinition {
    pub const ENTRY: &str = "data/gamedata/buffdata.csv";
    pub const COLUMNS: [&str; 4] = ["ID", "Define", "IconId", "ServerId"];

    pub fn read(table: &GameDataTable) -> eyre::Result<Vec<Self>> {
        typed_records(table, &Self::COLUMNS)?
            .into_iter()
            .map(|(line, row)| {
                let int = |column| table.integer(line, row, column);
                Ok(Self { id: int(0)?, define: text(row, 1), icon_id: int(2)?, server_id: int(3)? })
            })
            .collect()
    }
}

// endregion: --- Typed tables

#[cfg(test)]
mod tests {
    use super::*;

    const HOUSE_TABLE: &[u8] = b"# CSV file for the house gump,,\r\n#,,\r\nNameTid,Name,Cost\r\n#,,\r\n1011303,\"Stone, plaster\",36750\r\n\r\n1011304,Field stone house ,36750\r\n";

    #[test]
    fn reads_like_the_client_and_writes_back() {
        let table = GameDataTable::parse("house", HOUSE_TABLE).unwrap();
        assert_eq!(table.to_bytes().unwrap(), HOUSE_TABLE);
        assert_eq!(table.header().unwrap(), ["NameTid", "Name", "Cost"]);
        let records: Vec<&[String]> = table.records().map(|(_, row)| row.fields()).collect();
        assert_eq!(records, [&["1011303", "Stone, plaster", "36750"][..], &["1011304", "Field stone house ", "36750"]]);
        assert_eq!(table.lines()[5], TableLine::Blank);

        let unterminated = GameDataTable::parse("t", b"ID,Name\r\n1,\"open\r\n").unwrap_err();
        assert!(format!("{unterminated:#}").contains("line 2, column 3: the quote is never closed"), "{unterminated:#}");
        assert!(GameDataTable::parse("t", b"ID\r\n1\n2\r\n").is_err(), "mixed endings");
        let long = format!("ID\r\n1,{}\r\n", "x".repeat(MAX_LINE_CHARS));
        assert!(format!("{:#}", GameDataTable::parse("t", long.as_bytes()).unwrap_err()).contains("line 2"));
        let no_final = GameDataTable::parse("t", b"ID,TipTID\r\n1,1111882").unwrap();
        assert_eq!(no_final.to_bytes().unwrap(), b"ID,TipTID\r\n1,1111882");
        let latin = b"ID,Name\r\n110,Prized\xa0\r\n";
        assert_eq!(GameDataTable::parse("t", latin).unwrap().to_bytes().unwrap(), latin);
    }

    #[test]
    fn finds_the_column_names_above_the_first_record() {
        let table = GameDataTable::parse("abilities", b",,Buff\r\nID,name,Icon\r\n1,Clumsy,1\r\n").unwrap();
        assert_eq!(table.header().unwrap(), ["ID", "name", "Icon"]);
        assert_eq!(table.column("Icon"), Some(2));
        assert_eq!(table.records().count(), 1);
    }

    #[test]
    fn edits_fields_and_records() {
        let mut table = GameDataTable::parse("house", HOUSE_TABLE).unwrap();
        let (line, _) = table.records().next().unwrap();
        table.set_field(line, 1, "Brick house").unwrap();
        table.set_field(line, 2, "1,000").unwrap();
        table.push_record(&["1011305", "Small tower", "73500"]).unwrap();
        assert!(table.set_field(line, 1, "a \"quoted\" name").is_err());
        assert!(table.set_field(table.header_index().unwrap(), 0, "Renamed").is_err());
        assert!(table.push_record(&["tower"]).is_err());
        let bytes = table.to_bytes().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("\r\n1011303,Brick house,\"1,000\"\r\n"), "{text}");
        assert!(text.ends_with("1011304,Field stone house ,36750\r\n1011305,Small tower,73500\r\n"), "{text}");
    }

    #[test]
    fn a_replacement_keeps_the_shape() {
        let keeps = |file: &str| keeps_shape(HOUSE_TABLE, file.as_bytes()).map_err(|error| format!("{error:#}"));
        keeps("NameTid,Name,Cost\r\n1,Hut,5\r\n2,\"Tent, big\",6\r\n").unwrap();
        assert!(keeps("NameTid,Title,Cost\r\n1,Hut,5\r\n").unwrap_err().contains("column names"));
        assert!(keeps("NameTid,Name,Cost\r\n1,Hut,cheap\r\n").unwrap_err().contains("field 3 (Cost): `cheap` is text"));
        assert!(keeps("NameTid,Name,Cost\r\n1,Hut,\r\n").unwrap_err().contains("is empty"));
        assert!(keeps("NameTid,Name,Cost\r\n1,Hut,5,extra\r\n").unwrap_err().contains("4 fields"));
        assert!(keeps("NameTid,Name,Cost\r\n1,Hut,5\r\n1,Shed,4\r\n").unwrap_err().contains("already used"));
    }

    #[test]
    fn reads_the_typed_tables() {
        let containers = GameDataTable::parse("c", b"gumpId,left,top,right,bottom,name\r\n60,44,65,186,159,Backpack\r\n").unwrap();
        assert_eq!(LegacyContainer::read(&containers).unwrap()[0], LegacyContainer { gump_id: 60, left: 44, top: 65, right: 186, bottom: 159, name: "Backpack".into() });
        let bad = GameDataTable::parse("c", b"gumpId,left,top,right,bottom,name\r\n60,x,65,186,159,Backpack\r\n").unwrap();
        assert!(format!("{:#}", LegacyContainer::read(&bad).unwrap_err()).contains("line 2, field 2 (left)"));
        assert!(WeaponAbility::read(&containers).unwrap_err().to_string().contains("not `objectId"));
        let art = GameDataTable::parse("m", b"BodyType,Filename,Description\r\n776,Horde_Demon.dds,horde demon,,  \r\n").unwrap();
        assert_eq!(MobArt::read(&art).unwrap()[0].description, "horde demon");
        let buffs = GameDataTable::parse("b", b"ID,Define,IconId,ServerId\r\n1,BUFF_X, 1037,1001\r\n").unwrap();
        assert!(BuffDefinition::read(&buffs).is_err(), "a spaced integer is not one");
    }
}
