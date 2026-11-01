//! `LocalizedStrings.uop` string tables (Enhanced Client and Kingdom Reborn) and the Classic `Cliloc.<lang>` files,
//! which share one table format: read, edit, write back byte for byte, and KDL for hand editing.
//!
//! Table layout (all little-endian), the same in the UOP entries and the Classic files:
//! - header: `u32 header1`, `u16 header2`; every shipped table holds 2 and 1 (meaning inferred: a format version
//!   and a flag; the Classic reader of this crate and the community tools read them the same way);
//! - entries up to the end of the payload: `u32 id`, `u8 flag`, `u16 length`, `length` UTF-8 bytes. Every shipped
//!   table is sorted by id with no duplicate, every flag is 0, and every text is valid UTF-8.
//!
//! Entry names (verified: their hashes are the shipped entries'): `data/localizedstrings/%03d.cliloc`, with the
//! language ids of [`LOCALIZED_STRINGS_LANGUAGES`]. The language of each id is verified by content: the eight
//! Enhanced Client entries are byte-identical to the Classic client's `Cliloc.enu` ... `Cliloc.cht`. Kingdom Reborn
//! ships ids 1, 2 and 8.
//!
//! Writers never overwrite their source: [`write_tables`] writes a copy of the package with only the given tables
//! replaced or added (`uop_entries`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_kdl::ast::Node;
use udd_kdl::span::LineSpan;
use udd_kdl::strict::{
    arguments, check_document, describe_error, error_at, in_file, property, reject_children, value_string,
    value_u16, value_u32, NodeSchema,
};

use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const LOCALIZED_STRINGS_UOP_NAME: &str = "localizedstrings.uop";

/// Size of the table header: `u32 header1`, `u16 header2`.
const TABLE_HEADER_SIZE: usize = 6;
/// Size of an entry header: `u32 id`, `u8 flag`, `u16 length`.
const ENTRY_HEADER_SIZE: usize = 7;
/// Header values of every shipped table, used for new tables.
pub const DEFAULT_HEADER1: u32 = 2;
pub const DEFAULT_HEADER2: u16 = 1;
/// Compression of a new entry when the package has no sized entry (every shipped table is zlib-compressed).
const DEFAULT_ENTRY_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;
/// Byte 3 of a compressed Classic cliloc (the high byte of its length XOR key), as `classic::cliloc` detects it.
const COMPRESSED_CLILOC_MARKER_OFFSET: usize = 3;
const COMPRESSED_CLILOC_MARKER: u8 = 0x8E;
/// First bytes of a UOP package.
const UOP_MAGIC: &[u8; 4] = b"MYP\0";

/// A language of the string tables: its entry id and the Classic file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalizedStringsLanguage {
    pub id: u32,
    /// Extension of the Classic file with the same table (`Cliloc.enu`), also the KDL file stem.
    pub code: &'static str,
}

/// The languages of the shipped tables, by entry id.
pub const LOCALIZED_STRINGS_LANGUAGES: [LocalizedStringsLanguage; 8] = [
    LocalizedStringsLanguage { id: 1, code: "enu" },
    LocalizedStringsLanguage { id: 2, code: "jpn" },
    LocalizedStringsLanguage { id: 3, code: "kor" },
    LocalizedStringsLanguage { id: 4, code: "fra" },
    LocalizedStringsLanguage { id: 5, code: "deu" },
    LocalizedStringsLanguage { id: 6, code: "esp" },
    LocalizedStringsLanguage { id: 7, code: "chs" },
    LocalizedStringsLanguage { id: 8, code: "cht" },
];

/// `data/localizedstrings/%03d.cliloc`: the entry path of language `id`.
pub fn localized_strings_entry_path(language_id: u32) -> String {
    format!("data/localizedstrings/{language_id:03}.cliloc")
}

/// The UOP hash of language `id`'s table.
pub fn localized_strings_entry_hash(language_id: u32) -> u64 {
    hash_file_name_single(&localized_strings_entry_path(language_id))
}

/// The language whose table is the entry `hash`.
pub fn language_of_hash(hash: u64) -> Option<LocalizedStringsLanguage> {
    LOCALIZED_STRINGS_LANGUAGES.into_iter().find(|language| localized_strings_entry_hash(language.id) == hash)
}

/// The language with Classic extension (and KDL file stem) `code`, ignoring ASCII case.
pub fn language_of_code(code: &str) -> Option<LocalizedStringsLanguage> {
    LOCALIZED_STRINGS_LANGUAGES.into_iter().find(|language| language.code.eq_ignore_ascii_case(code))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedStringEntry {
    pub id: u32,
    /// The `u8` after the id; 0 in every shipped table.
    pub unk: u8,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedStringTable {
    pub header1: u32,
    pub header2: u16,
    pub entries: Vec<LocalizedStringEntry>,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringsFile {
    pub filename_hash: u64,
    pub byte_len: u32,
    pub strings: LocalizedStringTable,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringsPackage {
    pub files: Vec<LocalizedStringsFile>,
}

impl LocalizedStringsPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut files = Vec::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            let bytes = file.unpack().wrap_err_with(|| {
                format!("failed to unpack localized strings file {:016X}", file.filename_hash())
            })?;
            let strings = LocalizedStringTable::from_payload_bytes(&bytes).wrap_err_with(|| {
                format!("failed to parse localized strings file {:016X}", file.filename_hash())
            })?;
            files.push(LocalizedStringsFile {
                filename_hash: file.filename_hash(),
                byte_len: file.decompressed_size(),
                strings,
            });
        }
        files.sort_by_key(|file| file.filename_hash);
        Ok(Self { files })
    }

    pub fn len(&self) -> usize {
        self.files.iter().map(|file| file.strings.len()).sum()
    }
}

/// What [`write_tables`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TablesWrite {
    pub replaced: usize,
    pub added: usize,
    /// Tables that already held the payload.
    pub unchanged: usize,
}

/// Writes a copy of the `source` package to `output` with each `(hash, table)` stored as that entry: replaced
/// (keeping its compression) or added (with the compression of the package's entries). Untouched entries keep
/// their bytes and headers. `output` must not be `source`.
pub fn write_tables(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    tables: &[(u64, &LocalizedStringTable)],
) -> eyre::Result<TablesWrite> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let compression = package
        .iter_files()
        .find(|file| file.has_size())
        .map_or(DEFAULT_ENTRY_COMPRESSION, |file| file.compression());
    let mut write = TablesWrite::default();
    for &(hash, table) in tables {
        let payload = table.to_payload_bytes().wrap_err_with(|| format!("failed to encode table {hash:016X}"))?;
        let current = package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack {hash:016X}"))?;
        if current.as_deref() == Some(payload.as_slice()) {
            write.unchanged += 1;
            continue;
        }
        match uop_entries::put_entry(&mut package, hash, &payload, compression)? {
            EntryPut::Replaced => write.replaced += 1,
            EntryPut::Added => write.added += 1,
        }
    }
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(write)
}

/// Reads a Classic `Cliloc.<lang>` file as a table. Fails on the compressed files of later Classic clients
/// (`classic::cliloc` reads those), which this writer does not produce.
pub fn read_classic_cliloc(path: impl AsRef<Path>) -> eyre::Result<LocalizedStringTable> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    if bytes.get(COMPRESSED_CLILOC_MARKER_OFFSET) == Some(&COMPRESSED_CLILOC_MARKER) {
        eyre::bail!("{} is a compressed Classic cliloc: only uncompressed ones are edited", path.display());
    }
    LocalizedStringTable::from_payload_bytes(&bytes).wrap_err_with(|| format!("failed to parse {}", path.display()))
}

/// Writes `table` as an uncompressed Classic cliloc file to `output`, which must not be `source`; returns whether
/// the bytes differ from `source`'s.
pub fn write_classic_cliloc(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    table: &LocalizedStringTable,
) -> eyre::Result<bool> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let bytes = table.to_payload_bytes()?;
    let changed = std::fs::read(source).wrap_err_with(|| format!("failed to read {}", source.display()))? != bytes;
    std::fs::write(output, &bytes).wrap_err_with(|| format!("failed to write {}", output.display()))?;
    Ok(changed)
}

/// Whether `bytes` start like a UOP package (rather than a Classic cliloc file).
pub fn is_uop_package(bytes: &[u8]) -> bool {
    bytes.starts_with(UOP_MAGIC)
}

impl LocalizedStringTable {
    /// An empty table with the shipped header values.
    pub fn new() -> Self {
        Self { header1: DEFAULT_HEADER1, header2: DEFAULT_HEADER2, entries: Vec::new() }
    }

    /// Parses a table payload (a `LocalizedStrings.uop` entry or an uncompressed Classic cliloc file). Fails on a
    /// truncated entry and on text that is not UTF-8, so that [`Self::to_payload_bytes`] gives the bytes back.
    pub fn from_payload_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);
        if bytes.len() < TABLE_HEADER_SIZE {
            eyre::bail!("truncated localized strings header: {} bytes", bytes.len());
        }
        let header1 = reader.read_u32::<LittleEndian>()?;
        let header2 = reader.read_u16::<LittleEndian>()?;
        let mut entries = Vec::new();

        while (reader.position() as usize) < bytes.len() {
            let remaining = bytes.len() - reader.position() as usize;
            if remaining < ENTRY_HEADER_SIZE {
                eyre::bail!("truncated localized string entry header");
            }

            let id = reader.read_u32::<LittleEndian>()?;
            let unk = reader.read_u8()?;
            let len = reader.read_u16::<LittleEndian>()? as usize;
            if bytes.len() - (reader.position() as usize) < len {
                eyre::bail!("truncated localized string for {id}");
            }

            let start = reader.position() as usize;
            let text = std::str::from_utf8(&bytes[start..start + len])
                .map_err(|error| eyre::eyre!("localized string {id} is not UTF-8: {error}"))?
                .to_owned();
            reader.set_position((start + len) as u64);
            entries.push(LocalizedStringEntry { id, unk, text });
        }

        Ok(Self { header1, header2, entries })
    }

    /// The payload: the header, then every entry in order. Gives back the bytes [`Self::from_payload_bytes`]
    /// parsed. Fails on a text longer than `u16::MAX` bytes and on a repeated id.
    pub fn to_payload_bytes(&self) -> eyre::Result<Vec<u8>> {
        self.check_unique_ids()?;
        let size = TABLE_HEADER_SIZE + self.entries.iter().map(|entry| ENTRY_HEADER_SIZE + entry.text.len()).sum::<usize>();
        let mut bytes = Vec::with_capacity(size);
        bytes.write_u32::<LittleEndian>(self.header1)?;
        bytes.write_u16::<LittleEndian>(self.header2)?;
        for entry in &self.entries {
            let length = u16::try_from(entry.text.len()).map_err(|_| {
                eyre::eyre!("string {} is {} bytes long; a table stores at most {}", entry.id, entry.text.len(), u16::MAX)
            })?;
            bytes.write_u32::<LittleEndian>(entry.id)?;
            bytes.write_u8(entry.unk)?;
            bytes.write_u16::<LittleEndian>(length)?;
            bytes.extend_from_slice(entry.text.as_bytes());
        }
        Ok(bytes)
    }

    fn check_unique_ids(&self) -> eyre::Result<()> {
        let mut seen = std::collections::HashSet::with_capacity(self.entries.len());
        match self.entries.iter().find(|entry| !seen.insert(entry.id)) {
            Some(entry) => eyre::bail!("string {} appears more than once", entry.id),
            None => Ok(()),
        }
    }

    pub fn get(&self, id: u32) -> Option<&str> {
        self.position(id).map(|index| self.entries[index].text.as_str())
    }

    /// Sets the text of `id`, keeping its flag; a new id is inserted before the first larger id (shipped tables
    /// are sorted). Returns the previous text.
    pub fn set(&mut self, id: u32, text: impl Into<String>) -> Option<String> {
        let text = text.into();
        if let Some(index) = self.position(id) {
            return Some(std::mem::replace(&mut self.entries[index].text, text));
        }
        let at = self.entries.iter().position(|entry| entry.id > id).unwrap_or(self.entries.len());
        self.entries.insert(at, LocalizedStringEntry { id, unk: 0, text });
        None
    }

    /// Removes `id`; returns its entry.
    pub fn remove(&mut self, id: u32) -> Option<LocalizedStringEntry> {
        self.position(id).map(|index| self.entries.remove(index))
    }

    fn position(&self, id: u32) -> Option<usize> {
        // Shipped tables are sorted: try a binary search, then fall back to a scan for edited orders.
        match self.entries.binary_search_by_key(&id, |entry| entry.id) {
            Ok(index) => Some(index),
            Err(_) => self.entries.iter().position(|entry| entry.id == id),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The table as KDL: a `localized-strings header1=.. header2=..` node, then one `string <id> "<text>"` node
    /// per entry in table order, with `flag=<n>` when the flag is not 0.
    pub fn to_kdl_string(&self) -> String {
        let mut text = String::with_capacity(self.entries.iter().map(|entry| entry.text.len() + KDL_LINE_OVERHEAD).sum());
        text.push_str(KDL_HEADER_COMMENT);
        writeln!(text, "{ROOT_NODE} {HEADER1_PROPERTY}={} {HEADER2_PROPERTY}={}", self.header1, self.header2).unwrap();
        for entry in &self.entries {
            write!(text, "{STRING_NODE} {} ", entry.id).unwrap();
            push_kdl_string(&mut text, &entry.text);
            if entry.unk != 0 {
                write!(text, " {FLAG_PROPERTY}={}", entry.unk).unwrap();
            }
            text.push('\n');
        }
        text
    }

    /// Parses [`Self::to_kdl_string`]'s format (strict: unknown nodes and properties, a missing or repeated
    /// `localized-strings` node and repeated ids are errors). `file_name` starts every error message.
    pub fn from_kdl_str(file_name: &str, text: &str) -> eyre::Result<Self> {
        Self::read_kdl(text).map_err(|error| in_file(file_name, error))
    }

    fn read_kdl(text: &str) -> eyre::Result<Self> {
        let document = udd_kdl::parse_ast_line_span(KDL_SOURCE_NAME, text).map_err(|error| describe_error(text, &error))?;
        check_document(&document, KDL_SCHEMA)?;
        let mut header: Option<(u32, u16)> = None;
        let mut entries = Vec::new();
        let mut ids = BTreeMap::new();
        for node in &document.nodes {
            reject_children(node)?;
            match &**node.node_name {
                ROOT_NODE => {
                    if header.is_some() {
                        return Err(error_at(node.node_name.span(), format!("`{ROOT_NODE}` repeats")));
                    }
                    arguments::<0>(node)?;
                    let header1 = required(node, HEADER1_PROPERTY).and_then(|value| value_u32(value, HEADER1_PROPERTY))?;
                    let header2 = required(node, HEADER2_PROPERTY).and_then(|value| value_u16(value, HEADER2_PROPERTY))?;
                    header = Some((header1, header2));
                }
                _ => {
                    if header.is_none() {
                        return Err(error_at(node.node_name.span(), format!("`{ROOT_NODE}` must come first")));
                    }
                    let [id, text] = arguments::<2>(node)?;
                    let line = node.node_name.span().0.line;
                    let id_value = value_u32(id, "id")?;
                    if let Some(first) = ids.insert(id_value, line) {
                        return Err(error_at(
                            id.literal.span(),
                            format!("string {id_value} repeats (first on line {})", first + 1),
                        ));
                    }
                    let flag = property(node, FLAG_PROPERTY)
                        .map(|value| {
                            let flag = value_u16(value, FLAG_PROPERTY)?;
                            u8::try_from(flag).map_err(|_| error_at(value.literal.span(), format!("{FLAG_PROPERTY}: not a u8")))
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let text_value = value_string(text, "text")?;
                    if text_value.len() > usize::from(u16::MAX) {
                        return Err(error_at(
                            text.literal.span(),
                            format!("the text is {} bytes long; a table stores at most {}", text_value.len(), u16::MAX),
                        ));
                    }
                    entries.push(LocalizedStringEntry { id: id_value, unk: flag, text: text_value });
                }
            }
        }
        let (header1, header2) = header.ok_or_else(|| eyre::eyre!("missing the `{ROOT_NODE}` node"))?;
        Ok(Self { header1, header2, entries })
    }
}

impl Default for LocalizedStringTable {
    fn default() -> Self {
        Self::new()
    }
}

fn required<'a>(node: &'a Node<LineSpan>, name: &str) -> eyre::Result<&'a udd_kdl::ast::Value<LineSpan>> {
    property(node, name).ok_or_else(|| error_at(node.node_name.span(), format!("`{}`: missing {name}=", &**node.node_name)))
}

/// Writes `value` as a KDL string: `\\`, `"`, `\n`, `\r`, `\t` escaped by name, other control characters and the
/// invisible separators (`U+FEFF`, `U+2028`, `U+2029`) as `\u{..}`, everything else as is.
fn push_kdl_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || KDL_ESCAPED_INVISIBLES.contains(&c) => {
                write!(out, "\\u{{{:X}}}", u32::from(c)).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

const KDL_SOURCE_NAME: &str = "localized strings KDL";
const KDL_HEADER_COMMENT: &str = "// A LocalizedStrings.uop table or Classic cliloc file (uocf-cli strings export).\n\
// `string <id> \"<text>\"` per entry, in file order; `flag=<n>` when the entry's u8 flag is not 0.\n";
/// Bytes of a KDL line besides the text: `string `, the id, the quotes and the newline.
const KDL_LINE_OVERHEAD: usize = 20;
const ROOT_NODE: &str = "localized-strings";
const STRING_NODE: &str = "string";
const HEADER1_PROPERTY: &str = "header1";
const HEADER2_PROPERTY: &str = "header2";
const FLAG_PROPERTY: &str = "flag";
const KDL_ESCAPED_INVISIBLES: [char; 3] = ['\u{FEFF}', '\u{2028}', '\u{2029}'];
const KDL_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(ROOT_NODE, &[HEADER1_PROPERTY, HEADER2_PROPERTY]),
    NodeSchema::leaf(STRING_NODE, &[FLAG_PROPERTY]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    fn payload(entries: &[(u32, u8, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_u32::<LittleEndian>(DEFAULT_HEADER1).unwrap();
        bytes.write_u16::<LittleEndian>(DEFAULT_HEADER2).unwrap();
        for &(id, flag, text) in entries {
            bytes.write_u32::<LittleEndian>(id).unwrap();
            bytes.write_u8(flag).unwrap();
            bytes.write_u16::<LittleEndian>(text.len() as u16).unwrap();
            bytes.extend_from_slice(text);
        }
        bytes
    }

    #[test]
    fn parses_localized_payload_with_reference_layout() {
        let mut bytes = Vec::new();
        bytes.write_u32::<LittleEndian>(0x33445566).unwrap();
        bytes.write_u16::<LittleEndian>(0x1122).unwrap();
        bytes.write_u32::<LittleEndian>(200).unwrap();
        bytes.write_u8(0xAB).unwrap();
        bytes.write_u16::<LittleEndian>(4).unwrap();
        bytes.extend_from_slice(b"text");

        let strings = LocalizedStringTable::from_payload_bytes(&bytes).unwrap();

        assert_eq!(strings.header1, 0x33445566);
        assert_eq!(strings.header2, 0x1122);
        assert_eq!(strings.len(), 1);
        assert_eq!(strings.entries[0].id, 200);
        assert_eq!(strings.entries[0].unk, 0xAB);
        assert_eq!(strings.get(200), Some("text"));
        assert_eq!(strings.to_payload_bytes().unwrap(), bytes);
    }

    #[test]
    fn rejects_truncated_localized_entry() {
        let mut bytes = payload(&[]);
        bytes.write_u32::<LittleEndian>(200).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u16::<LittleEndian>(5).unwrap();
        bytes.extend_from_slice(b"abc");

        let error = LocalizedStringTable::from_payload_bytes(&bytes).unwrap_err();
        assert!(error.to_string().contains("truncated localized string for 200"));
    }

    #[test]
    fn rejects_text_that_is_not_utf8() {
        let error = LocalizedStringTable::from_payload_bytes(&payload(&[(7, 0, &[0xFF, 0xFE])])).unwrap_err();
        assert!(error.to_string().contains("localized string 7 is not UTF-8"), "{error}");
    }

    #[test]
    fn edits_keep_the_id_order_and_the_flag() {
        let bytes = payload(&[(10, 3, b"ten"), (30, 0, b"thirty")]);
        let mut table = LocalizedStringTable::from_payload_bytes(&bytes).unwrap();
        assert_eq!(table.set(10, "TEN"), Some("ten".to_string()));
        assert_eq!(table.set(20, "twenty"), None);
        assert_eq!(table.remove(30).map(|entry| entry.text), Some("thirty".to_string()));
        assert_eq!(table.to_payload_bytes().unwrap(), payload(&[(10, 3, b"TEN"), (20, 0, b"twenty")]));
    }

    #[test]
    fn writer_rejects_repeated_ids_and_long_texts() {
        let mut table = LocalizedStringTable::new();
        table.entries.push(LocalizedStringEntry { id: 1, unk: 0, text: String::new() });
        table.entries.push(LocalizedStringEntry { id: 1, unk: 0, text: String::new() });
        assert!(table.to_payload_bytes().unwrap_err().to_string().contains("string 1 appears more than once"));
        let mut table = LocalizedStringTable::new();
        table.set(2, "x".repeat(usize::from(u16::MAX) + 1));
        assert!(table.to_payload_bytes().unwrap_err().to_string().contains("a table stores at most"));
    }

    #[test]
    fn kdl_round_trips_escapes_flags_and_empty_texts() {
        let text = "a \"quoted\" \\ back\nslash\r\t\u{0}\u{7F}\u{FEFF}\u{2028} ~1 ü 日本 \u{1F600}";
        let bytes = payload(&[(1, 0, b""), (500_000, 9, text.as_bytes()), (3, 0, b"unsorted")]);
        let table = LocalizedStringTable::from_payload_bytes(&bytes).unwrap();
        let kdl = table.to_kdl_string();
        let back = LocalizedStringTable::from_kdl_str("t.kdl", &kdl).unwrap();
        assert_eq!(back, table);
        assert_eq!(back.to_payload_bytes().unwrap(), bytes);
    }

    #[test]
    fn kdl_reader_is_strict() {
        let cases = [
            ("string 1 \"a\"\n", "must come first"),
            ("localized-strings header1=2 header2=1\nstring 1 \"a\"\nstring 1 \"b\"\n", "string 1 repeats (first on line 2)"),
            ("localized-strings header1=2 header2=1\nstring 1 \"a\" color=1\n", "unknown property `color`"),
            ("localized-strings header1=2 header2=1\ntext 1 \"a\"\n", "unknown node `text`"),
            ("localized-strings header1=2\n", "missing header2="),
            ("localized-strings header1=2 header2=1\nstring 1 \"a\" flag=256\n", "flag: not a u8"),
            ("localized-strings header1=2 header2=1\nstring 1\n", "takes 2 arguments"),
            ("localized-strings header1=2 header2=1\nlocalized-strings header1=2 header2=1\n", "repeats"),
            ("", "missing the `localized-strings` node"),
        ];
        for (text, expected) in cases {
            let error = LocalizedStringTable::from_kdl_str("t.kdl", text).unwrap_err().to_string();
            assert!(error.contains(expected), "{text:?}: {error}");
            assert!(error.starts_with("t.kdl: "), "{error}");
        }
    }

    #[test]
    fn entry_paths_name_the_languages() {
        assert_eq!(localized_strings_entry_path(1), "data/localizedstrings/001.cliloc");
        assert_eq!(localized_strings_entry_hash(1), 0xC7BD2BD170EC71C0);
        assert_eq!(localized_strings_entry_hash(8), 0x73FDE3076135612B);
        assert_eq!(language_of_hash(0x21A4DC198E393A7A).map(|language| language.code), Some("jpn"));
        assert_eq!(language_of_code("FRA").map(|language| language.id), Some(4));
    }
}
