//! The Classic client's localized UI text: the string tables of `Intloc00-11.*`, `Chat.*`, `Gesture.*`, `Gt_*.*`,
//! `Intro.*`, `OPTNUOTD.*`, `options.*`, `Professn.*`, `SKILL16/30.*`, `Skilname.*`, `Tilehelp.*` and `Tooltips.*`
//! ([`LocalizedTextFile`]), and the language list `Langcode.iff` ([`LanguageCodesFile`]). Both are IFF files
//! ([`crate::classic::iff`]), read strictly and written back byte for byte.
//!
//! A string table is `FORM DATA { FORM LANG { INFO, TEXT [, TIME] [, POSN, COLR] [, TKEY] } }`:
//!
//! - `INFO`: the language code (`enu`, `deu`, `kor`; `bg5` in UO Gold's `Gesture.cht`), NUL-terminated; the
//!   character width (`u32`: 1 for single-byte text, 2 for UTF-16LE text, the Asian languages'); a flags
//!   `u32` (1 in `Tilehelp.*`, 0 elsewhere); the table name (`chat`, `intloc00`), NUL-terminated; a `u32` that
//!   is 1 in every file; the string count (`u32`). The numbers are little-endian.
//! - `TEXT`: the strings, each NUL-terminated (a 2-byte NUL in UTF-16 text). Single-byte text is read as
//!   Latin-1, so every byte comes back.
//! - `TIME` (`Intro.*`): two `u32` per string, the times the intro shows and hides it.
//! - `POSN` and `COLR` (`SKILL16/30.*`): two `u32` per string (x, y) and three (red, green, blue).
//! - `TKEY` (`Tilehelp.*`): groups of a string index, a tile count and that many tile ids, all `u32`.
//!
//! `Langcode.iff` is `FORM DATA { FORM LANG { FORM WIN3 { CODE... } } }`; a `CODE` is a 3-letter code, a `u32`,
//! the language and the country, the texts NUL-terminated.

crate::eyre_imports!();

use std::path::Path;

use crate::classic::iff::{IffNode, tag_name};
use crate::utils::output_file::write_through_temp;

const DATA_KIND: [u8; 4] = *b"DATA";
const LANG_KIND: [u8; 4] = *b"LANG";
const WIN3_KIND: [u8; 4] = *b"WIN3";
const INFO_TAG: [u8; 4] = *b"INFO";
const TEXT_TAG: [u8; 4] = *b"TEXT";
const TIME_TAG: [u8; 4] = *b"TIME";
const POSN_TAG: [u8; 4] = *b"POSN";
const COLR_TAG: [u8; 4] = *b"COLR";
const TKEY_TAG: [u8; 4] = *b"TKEY";
const CODE_TAG: [u8; 4] = *b"CODE";
/// The order a table's chunks are written in (and read in every shipped file).
const TABLE_CHUNK_ORDER: [[u8; 4]; 6] = [INFO_TAG, TEXT_TAG, TIME_TAG, POSN_TAG, COLR_TAG, TKEY_TAG];
/// The `INFO` character width of single-byte text.
pub const SINGLE_BYTE_WIDTH: u32 = 1;
/// The `INFO` character width of UTF-16LE text.
pub const UTF16_WIDTH: u32 = 2;
const U32_BYTES: usize = size_of::<u32>();

/// The stems of the string table files, matched ignoring case.
pub const LOCALIZED_TABLE_STEMS: [&str; 11] =
    ["chat", "gesture", "intro", "optnuotd", "options", "professn", "skill16", "skill30", "skilname", "tilehelp", "tooltips"];
/// The stem prefixes of the numbered string table files (`Gt_0000`, `Intloc00`), followed by digits.
pub const LOCALIZED_TABLE_PREFIXES: [&str; 2] = ["gt_", "intloc"];
/// The length of a table file's extension, the language (`enu`, `kor`).
const LANGUAGE_EXTENSION_CHARS: usize = 3;

/// Whether `file_name` names a string table: a known stem, or a known prefix and digits, and a 3-letter
/// language extension.
pub fn is_localized_table_file(file_name: &str) -> bool {
    let Some((stem, extension)) = file_name.rsplit_once('.') else { return false };
    if extension.len() != LANGUAGE_EXTENSION_CHARS || !extension.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let stem = stem.to_ascii_lowercase();
    LOCALIZED_TABLE_STEMS.contains(&stem.as_str())
        || LOCALIZED_TABLE_PREFIXES.iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        })
}

/// How a table's strings are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// One byte per character, read as Latin-1 (the Western languages' code page bytes, kept as they are).
    SingleByte,
    /// UTF-16LE (Japanese, Korean, Chinese).
    Utf16,
}

/// A `Tilehelp` group: the tiles whose help is string `string`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileHelpKey {
    pub string: u32,
    pub tiles: Vec<u32>,
}

/// One localized string table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedTextFile {
    pub language: String,
    pub encoding: TextEncoding,
    pub flags: u32,
    pub name: String,
    /// The `u32` after the name (1 in every shipped file).
    pub version: u32,
    pub strings: Vec<String>,
    /// `TIME`: per string, when the intro shows and hides it.
    pub times: Option<Vec<[u32; 2]>>,
    /// `POSN`: per string, its x and y.
    pub positions: Option<Vec<[u32; 2]>>,
    /// `COLR`: per string, its red, green and blue.
    pub colors: Option<Vec<[u32; 3]>>,
    /// `TKEY`: the tiles of each help string.
    pub tile_keys: Option<Vec<TileHelpKey>>,
}

/// A reader over a chunk's bytes.
struct Fields<'a> {
    what: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Fields<'a> {
    fn new(what: &'a str, bytes: &'a [u8]) -> Self {
        Self { what, bytes, position: 0 }
    }

    fn u32(&mut self) -> eyre::Result<u32> {
        let end = self.position + U32_BYTES;
        let field = self.bytes.get(self.position..end).ok_or_else(|| eyre!("{}: ends at {} inside a number", self.what, self.bytes.len()))?;
        self.position = end;
        Ok(u32::from_le_bytes(field.try_into().expect("four bytes")))
    }

    fn text(&mut self) -> eyre::Result<String> {
        let rest = &self.bytes[self.position..];
        let length = rest.iter().position(|&byte| byte == 0).ok_or_else(|| eyre!("{}: a text at {} has no NUL", self.what, self.position))?;
        self.position += length + 1;
        Ok(rest[..length].iter().map(|&byte| char::from(byte)).collect())
    }

    fn finish(&self) -> eyre::Result<()> {
        if self.position != self.bytes.len() {
            eyre::bail!("{}: {} bytes after its fields", self.what, self.bytes.len() - self.position);
        }
        Ok(())
    }
}

fn latin1_bytes(what: &str, text: &str) -> eyre::Result<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok().filter(|&byte| byte != 0))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| eyre!("{what}: `{text}` holds a NUL or a character past U+00FF"))
}

fn u32_rows<const N: usize>(what: &str, data: &[u8], count: usize) -> eyre::Result<Vec<[u32; N]>> {
    let expected = count * N * U32_BYTES;
    if data.len() != expected {
        eyre::bail!("{what}: {} bytes, {N} numbers for each of the {count} strings take {expected}", data.len());
    }
    Ok(data
        .chunks_exact(N * U32_BYTES)
        .map(|row| std::array::from_fn(|field| u32::from_le_bytes(row[field * U32_BYTES..(field + 1) * U32_BYTES].try_into().expect("four bytes"))))
        .collect())
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn count_u32(what: &str, count: usize) -> eyre::Result<u32> {
    u32::try_from(count).map_err(|_| eyre!("{what}: {count} does not fit 32 bits"))
}

/// The form `kind` holding exactly one form `inner`: its children.
fn single_form<'a>(node: &'a IffNode, kind: [u8; 4], inner: [u8; 4]) -> eyre::Result<&'a [IffNode]> {
    let IffNode::Form { kind: outer_kind, children } = node else {
        eyre::bail!("expected FORM {}, found the chunk {}", tag_name(&kind), node.tag_name());
    };
    if *outer_kind != kind {
        eyre::bail!("expected FORM {}, found FORM {}", tag_name(&kind), tag_name(outer_kind));
    }
    match children.as_slice() {
        [IffNode::Form { kind: inner_kind, children }] if *inner_kind == inner => Ok(children),
        _ => eyre::bail!("FORM {} must hold one FORM {} and nothing else", tag_name(&kind), tag_name(&inner)),
    }
}

fn read_bytes(path: &Path) -> eyre::Result<Vec<u8>> {
    std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))
}

fn save_bytes(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    write_through_temp(path, |temp| std::fs::write(temp, bytes).wrap_err_with(|| format!("write {}", temp.display())))
}

impl LocalizedTextFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let root = IffNode::from_bytes(bytes)?;
        let chunks = single_form(&root, DATA_KIND, LANG_KIND)?;
        let mut found: Vec<([u8; 4], &[u8])> = Vec::new();
        for chunk in chunks {
            let IffNode::Chunk { tag, data } = chunk else {
                eyre::bail!("FORM LANG holds {}, a table holds chunks only", chunk.tag_name());
            };
            let Some(rank) = TABLE_CHUNK_ORDER.iter().position(|known| known == tag) else {
                eyre::bail!("FORM LANG holds the unknown chunk {}", tag_name(tag));
            };
            if let Some((last, _)) = found.last()
                && TABLE_CHUNK_ORDER.iter().position(|known| known == last).is_some_and(|last_rank| last_rank >= rank)
            {
                eyre::bail!("the {} chunk comes after {}: expected the order INFO, TEXT, TIME, POSN, COLR, TKEY, each once", tag_name(tag), tag_name(last));
            }
            found.push((*tag, data));
        }
        let chunk = |tag: [u8; 4]| found.iter().find(|(found_tag, _)| *found_tag == tag).map(|(_, data)| *data);
        let info = chunk(INFO_TAG).ok_or_else(|| eyre!("no INFO chunk"))?;
        let text = chunk(TEXT_TAG).ok_or_else(|| eyre!("no TEXT chunk"))?;

        let mut fields = Fields::new("INFO", info);
        let language = fields.text()?;
        let encoding = match fields.u32()? {
            SINGLE_BYTE_WIDTH => TextEncoding::SingleByte,
            UTF16_WIDTH => TextEncoding::Utf16,
            other => eyre::bail!("INFO: character width {other}, expected {SINGLE_BYTE_WIDTH} or {UTF16_WIDTH}"),
        };
        let flags = fields.u32()?;
        let name = fields.text()?;
        let version = fields.u32()?;
        let count = fields.u32()? as usize;
        fields.finish()?;

        let strings = read_strings(text, encoding)?;
        if strings.len() != count {
            eyre::bail!("TEXT holds {} strings, INFO counts {count}", strings.len());
        }
        let times = chunk(TIME_TAG).map(|data| u32_rows("TIME", data, count)).transpose()?;
        let positions = chunk(POSN_TAG).map(|data| u32_rows("POSN", data, count)).transpose()?;
        let colors = chunk(COLR_TAG).map(|data| u32_rows("COLR", data, count)).transpose()?;
        let tile_keys = chunk(TKEY_TAG).map(|data| read_tile_keys(data, count)).transpose()?;
        Ok(Self { language, encoding, flags, name, version, strings, times, positions, colors, tile_keys })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read_bytes(path)?).wrap_err_with(|| format!("{}", path.display()))
    }

    /// The file; the per-string chunks must hold one row per string.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let count = self.strings.len();
        let mut info = latin1_bytes("INFO language", &self.language)?;
        info.push(0);
        put_u32(&mut info, match self.encoding {
            TextEncoding::SingleByte => SINGLE_BYTE_WIDTH,
            TextEncoding::Utf16 => UTF16_WIDTH,
        });
        put_u32(&mut info, self.flags);
        info.extend(latin1_bytes("INFO name", &self.name)?);
        info.push(0);
        put_u32(&mut info, self.version);
        put_u32(&mut info, count_u32("the string count", count)?);

        let mut chunks = vec![(INFO_TAG, info), (TEXT_TAG, write_strings(&self.strings, self.encoding)?)];
        let rows = |what: &str, rows: Option<usize>| match rows {
            Some(rows) if rows != count => Err(eyre!("{what}: {rows} rows for {count} strings")),
            _ => Ok(()),
        };
        rows("TIME", self.times.as_ref().map(Vec::len))?;
        rows("POSN", self.positions.as_ref().map(Vec::len))?;
        rows("COLR", self.colors.as_ref().map(Vec::len))?;
        if let Some(times) = &self.times {
            chunks.push((TIME_TAG, times.iter().flatten().flat_map(|value| value.to_le_bytes()).collect()));
        }
        if let Some(positions) = &self.positions {
            chunks.push((POSN_TAG, positions.iter().flatten().flat_map(|value| value.to_le_bytes()).collect()));
        }
        if let Some(colors) = &self.colors {
            chunks.push((COLR_TAG, colors.iter().flatten().flat_map(|value| value.to_le_bytes()).collect()));
        }
        if let Some(keys) = &self.tile_keys {
            let mut data = Vec::new();
            for key in keys {
                if key.string as usize >= count {
                    eyre::bail!("TKEY: string {} of {count}", key.string);
                }
                put_u32(&mut data, key.string);
                put_u32(&mut data, count_u32("TKEY tile count", key.tiles.len())?);
                key.tiles.iter().for_each(|&tile| put_u32(&mut data, tile));
            }
            chunks.push((TKEY_TAG, data));
        }
        let children = chunks.into_iter().map(|(tag, data)| IffNode::Chunk { tag, data }).collect();
        IffNode::Form { kind: DATA_KIND, children: vec![IffNode::Form { kind: LANG_KIND, children }] }.to_bytes()
    }

    /// Writes [`Self::to_bytes`] through a temporary file in the folder of `path`.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save_bytes(path, &self.to_bytes()?)
    }
}

fn read_strings(text: &[u8], encoding: TextEncoding) -> eyre::Result<Vec<String>> {
    match encoding {
        TextEncoding::SingleByte => {
            let Some(body) = text.strip_suffix(&[0]) else {
                return if text.is_empty() { Ok(Vec::new()) } else { Err(eyre!("TEXT does not end in a NUL")) };
            };
            Ok(body.split(|&byte| byte == 0).map(|string| string.iter().map(|&byte| char::from(byte)).collect()).collect())
        }
        TextEncoding::Utf16 => {
            if text.len() % 2 == 1 {
                eyre::bail!("TEXT: {} bytes of UTF-16 text", text.len());
            }
            let units: Vec<u16> = text.chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0], unit[1]])).collect();
            let Some(body) = units.strip_suffix(&[0]) else {
                return if units.is_empty() { Ok(Vec::new()) } else { Err(eyre!("TEXT does not end in a NUL")) };
            };
            body.split(|&unit| unit == 0)
                .enumerate()
                .map(|(index, string)| String::from_utf16(string).map_err(|_| eyre!("TEXT string {index}: not UTF-16")))
                .collect()
        }
    }
}

fn write_strings(strings: &[String], encoding: TextEncoding) -> eyre::Result<Vec<u8>> {
    let mut text = Vec::new();
    for (index, string) in strings.iter().enumerate() {
        match encoding {
            TextEncoding::SingleByte => {
                text.extend(latin1_bytes(&format!("string {index}"), string)?);
                text.push(0);
            }
            TextEncoding::Utf16 => {
                if string.contains('\0') {
                    eyre::bail!("string {index} holds a NUL");
                }
                string.encode_utf16().chain(std::iter::once(0)).for_each(|unit| text.extend_from_slice(&unit.to_le_bytes()));
            }
        }
    }
    Ok(text)
}

fn read_tile_keys(data: &[u8], count: usize) -> eyre::Result<Vec<TileHelpKey>> {
    let mut fields = Fields::new("TKEY", data);
    let mut keys = Vec::new();
    while fields.position < data.len() {
        let string = fields.u32()?;
        if string as usize >= count {
            eyre::bail!("TKEY: group {} names string {string} of {count}", keys.len());
        }
        let tiles = fields.u32()?;
        let tiles = (0..tiles).map(|_| fields.u32()).collect::<eyre::Result<_>>()?;
        keys.push(TileHelpKey { string, tiles });
    }
    Ok(keys)
}

/// One `CODE` of `Langcode.iff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCode {
    /// The 3-letter code (`ENU`, `DEU`).
    pub code: String,
    /// The `u32` after it: a character code, `e` for the English codes, `s` the Spanish ones, `?` for most.
    pub group: u32,
    pub language: String,
    pub country: String,
}

/// `Langcode.iff`: the language codes, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCodesFile {
    pub codes: Vec<LanguageCode>,
}

impl LanguageCodesFile {
    pub const FILE_NAME: &str = "Langcode.iff";

    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let root = IffNode::from_bytes(bytes)?;
        let lang = single_form(&root, DATA_KIND, LANG_KIND)?;
        let codes = match lang {
            [IffNode::Form { kind, children }] if *kind == WIN3_KIND => children,
            _ => eyre::bail!("FORM LANG must hold one FORM WIN3 and nothing else"),
        };
        let codes = codes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let IffNode::Chunk { tag: CODE_TAG, data } = node else {
                    eyre::bail!("FORM WIN3 node {index} is {}, expected CODE", node.tag_name());
                };
                let what = format!("CODE {index}");
                let mut fields = Fields::new(&what, data);
                let code = LanguageCode { code: fields.text()?, group: fields.u32()?, language: fields.text()?, country: fields.text()? };
                fields.finish()?;
                Ok(code)
            })
            .collect::<eyre::Result<_>>()?;
        Ok(Self { codes })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        Self::from_bytes(&read_bytes(path)?).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut children = Vec::with_capacity(self.codes.len());
        for (index, code) in self.codes.iter().enumerate() {
            let what = format!("CODE {index}");
            let mut data = latin1_bytes(&what, &code.code)?;
            data.push(0);
            put_u32(&mut data, code.group);
            for text in [&code.language, &code.country] {
                data.extend(latin1_bytes(&what, text)?);
                data.push(0);
            }
            children.push(IffNode::Chunk { tag: CODE_TAG, data });
        }
        let win3 = IffNode::Form { kind: WIN3_KIND, children };
        IffNode::Form { kind: DATA_KIND, children: vec![IffNode::Form { kind: LANG_KIND, children: vec![win3] }] }.to_bytes()
    }

    /// Writes [`Self::to_bytes`] through a temporary file in the folder of `path`.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save_bytes(path, &self.to_bytes()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(encoding: TextEncoding) -> LocalizedTextFile {
        LocalizedTextFile {
            language: "enu".to_owned(),
            encoding,
            flags: 1,
            name: "tilehelp".to_owned(),
            version: 1,
            strings: vec!["A keyring".to_owned(), String::new(), "caf\u{e9}".to_owned()],
            times: Some(vec![[1, 2], [3, 4], [5, 6]]),
            positions: None,
            colors: Some(vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]]),
            tile_keys: Some(vec![TileHelpKey { string: 0, tiles: vec![4113, 5993] }, TileHelpKey { string: 2, tiles: vec![] }]),
        }
    }

    #[test]
    fn tables_round_trip_in_both_encodings() {
        for encoding in [TextEncoding::SingleByte, TextEncoding::Utf16] {
            let file = table(encoding);
            let bytes = file.to_bytes().unwrap();
            assert_eq!(LocalizedTextFile::from_bytes(&bytes).unwrap(), file);
        }
        let mut wide = table(TextEncoding::Utf16);
        wide.strings[1] = "\u{4f1a}\u{8b70}".to_owned();
        assert_eq!(LocalizedTextFile::from_bytes(&wide.to_bytes().unwrap()).unwrap(), wide);
        let mut narrow = table(TextEncoding::SingleByte);
        narrow.strings[1] = "\u{4f1a}".to_owned();
        assert!(narrow.to_bytes().is_err(), "single-byte text holds U+00FF at most");
    }

    #[test]
    fn tables_are_read_strictly() {
        let mut short = table(TextEncoding::SingleByte);
        short.times.as_mut().unwrap().pop();
        assert!(short.to_bytes().unwrap_err().to_string().contains("TIME: 2 rows for 3 strings"));
        let mut bad_key = table(TextEncoding::SingleByte);
        bad_key.tile_keys.as_mut().unwrap()[0].string = 3;
        assert!(bad_key.to_bytes().is_err());

        let bytes = table(TextEncoding::SingleByte).to_bytes().unwrap();
        let root = IffNode::from_bytes(&bytes).unwrap();
        let mut chunks = root.children()[0].children().to_vec();
        let rebuild = |chunks: &[IffNode]| {
            let lang = IffNode::Form { kind: LANG_KIND, children: chunks.to_vec() };
            IffNode::Form { kind: DATA_KIND, children: vec![lang] }.to_bytes().unwrap()
        };
        chunks.swap(1, 2);
        let error = LocalizedTextFile::from_bytes(&rebuild(&chunks)).unwrap_err().to_string();
        assert!(error.contains("the TEXT chunk comes after TIME"), "{error}");
        chunks.swap(1, 2);
        let IffNode::Chunk { data, .. } = &mut chunks[1] else { unreachable!() };
        data.pop();
        let error = LocalizedTextFile::from_bytes(&rebuild(&chunks)).unwrap_err().to_string();
        assert!(error.contains("does not end in a NUL"), "{error}");
    }

    #[test]
    fn language_codes_round_trip() {
        let file = LanguageCodesFile {
            codes: vec![
                LanguageCode { code: "ARA".to_owned(), group: u32::from(b'?'), language: "Arabic".to_owned(), country: "Saudi Arabia".to_owned() },
                LanguageCode { code: "ENU".to_owned(), group: u32::from(b'e'), language: "English".to_owned(), country: String::new() },
            ],
        };
        assert_eq!(LanguageCodesFile::from_bytes(&file.to_bytes().unwrap()).unwrap(), file);
    }
}
