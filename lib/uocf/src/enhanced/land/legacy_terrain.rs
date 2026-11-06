//! `LegacyTerrain.uop`: one XML definition per Classic land tile, stored as the entry
//! `data/definitions/legacyterrain/%08d.legacyterrain` of the land tile id:
//!
//! ```xml
//! <legacyTerrain>
//!     <terrainType id="969" name="sand" texMap="969">
//!         <typeFlags>
//!             <impassable />
//!         </typeFlags>
//!     </terrainType>
//! </legacyTerrain>
//! ```
//!
//! [`LegacyTerrainType`] is the typed definition. [`LegacyTerrainDocument`] pairs it with the formatting of its
//! document: every byte outside the typed values (prolog, whitespace and comments between elements, attribute
//! order, spacing and quotes, the spelling of each value, the self-closing style, the epilog). The writer rebuilds a
//! document from the two, so an unedited one comes back byte for byte and an edited one keeps its style: a changed
//! value is written fresh (escaped for its quote), a removed flag drops its line, an added flag copies the
//! indentation of the document's flags (or derives it from the element indentation when there were none).
//!
//! The parser accepts this schema only, as a well-formed UTF-8 document: an optional BOM, XML declaration,
//! processing instructions and comments before the root, whitespace and comments between elements, comments after
//! it. Anything else (an unknown element or attribute, a missing or repeated attribute, text content, a non-empty
//! flag element) is an error.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use color_eyre::eyre::{self, bail, ensure, eyre, WrapErr};

use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const LEGACY_TERRAIN_UOP_NAME: &str = "LegacyTerrain.uop";
/// Compression of an entry added to a package without sized entries; the client compresses every entry.
const DEFAULT_LEGACY_TERRAIN_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

const ROOT_ELEMENT: &str = "legacyTerrain";
const TERRAIN_TYPE_ELEMENT: &str = "terrainType";
const TYPE_FLAGS_ELEMENT: &str = "typeFlags";

/// Formatting of a new document, the most common one of the client files: CRLF line breaks, four-space
/// indentation, a space before `/>`, a line break after the root.
const CLIENT_LINE_BREAK: &str = "\r\n";
const CLIENT_INDENT_UNIT: &str = "    ";
const CLIENT_SELF_CLOSING_TAIL: &str = " ";
const CLIENT_ATTRIBUTE_SEPARATOR: &str = " ";
const CLIENT_ATTRIBUTE_EQUALS: &str = "=";
const CLIENT_QUOTE: char = '"';
const UTF8_BOM: &str = "\u{FEFF}";

/// The entry path of the definition of land tile `id`.
pub fn legacy_terrain_entry_path(id: u32) -> String {
    format!("data/definitions/legacyterrain/{id:08}.legacyterrain")
}

/// The entry hash of the definition of land tile `id`.
pub fn legacy_terrain_entry_hash(id: u32) -> u64 {
    hash_file_name_single(&legacy_terrain_entry_path(id))
}

/// A child element of `<typeFlags>`. Every flag but `NoDiagonal` agrees with a Classic `tiledata.mul` land flag on
/// every client entry (see ENHANCED_CLIENT_FILES.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LegacyTerrainFlag {
    Impassable,
    Surface,
    NoHouse,
    Wet,
    Wall,
    MonGen,
    NoDiagonal,
}

impl LegacyTerrainFlag {
    pub const ALL: [Self; 7] =
        [Self::Impassable, Self::Surface, Self::NoHouse, Self::Wet, Self::Wall, Self::MonGen, Self::NoDiagonal];

    /// The element name of the flag.
    pub fn element_name(self) -> &'static str {
        match self {
            Self::Impassable => "impassable",
            Self::Surface => "surface",
            Self::NoHouse => "nohouse",
            Self::Wet => "wet",
            Self::Wall => "wall",
            Self::MonGen => "mongen",
            Self::NoDiagonal => "no_diagonal",
        }
    }

    pub fn from_element_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|flag| flag.element_name() == name)
    }
}

/// The typed definition of one land tile: the `<terrainType>` attributes and its flags in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyTerrainType {
    /// Land tile id; also the entry's file number.
    pub id: u32,
    /// The `tiledata.mul` land tile name on every client entry.
    pub name: String,
    /// The `tiledata.mul` land texture id on every client entry.
    pub tex_map: u32,
    pub flags: Vec<LegacyTerrainFlag>,
}

impl LegacyTerrainType {
    pub fn has_flag(&self, flag: LegacyTerrainFlag) -> bool {
        self.flags.contains(&flag)
    }

    /// Adds `flag` after the others when `on` and absent; removes every occurrence when not `on`.
    pub fn set_flag(&mut self, flag: LegacyTerrainFlag, on: bool) {
        if !on {
            self.flags.retain(|existing| *existing != flag);
        } else if !self.has_flag(flag) {
            self.flags.push(flag);
        }
    }
}

/// A `<terrainType>` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerrainAttribute {
    Id,
    Name,
    TexMap,
}

impl TerrainAttribute {
    /// In the order of a new document.
    const ALL: [Self; 3] = [Self::Id, Self::Name, Self::TexMap];

    fn xml_name(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::TexMap => "texMap",
        }
    }

    fn from_xml_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|attribute| attribute.xml_name() == name)
    }
}

/// The bytes of one attribute around its value: `{before}{name}{equals}{quote}{raw_value}{quote}`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeFormat {
    attribute: TerrainAttribute,
    before: String,
    equals: String,
    quote: char,
    /// The value as stored (escaped); written again while it still means the typed value.
    raw_value: String,
}

/// One flag element: `{before}<{name}{tail}/>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FlagFormat {
    flag: LegacyTerrainFlag,
    before: String,
    tail: String,
}

/// The `<typeFlags>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TypeFlagsFormat {
    /// `<typeFlags{tail}/>`.
    Empty { tail: String },
    /// `<typeFlags{tail}>`, the flags, `{before_end}</typeFlags{end_tail}>`.
    Open { tail: String, flags: Vec<FlagFormat>, before_end: String, end_tail: String },
}

/// Every byte of a document outside its typed values, in document order. A `tail` is the whitespace between the
/// last attribute (or the name) of a tag and its `>` or `/>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentFormat {
    /// BOM, XML declaration, processing instructions, comments and whitespace before the root.
    prolog: String,
    root_tail: String,
    before_terrain_type: String,
    attributes: Vec<AttributeFormat>,
    terrain_type_tail: String,
    before_type_flags: String,
    type_flags: TypeFlagsFormat,
    before_terrain_type_end: String,
    terrain_type_end_tail: String,
    before_root_end: String,
    root_end_tail: String,
    /// Whitespace and comments after the root.
    epilog: String,
}

impl DocumentFormat {
    /// The formatting of a new document (the most common one of the client files).
    fn client_default() -> Self {
        let line = |depth: usize| format!("{CLIENT_LINE_BREAK}{}", CLIENT_INDENT_UNIT.repeat(depth));
        Self {
            prolog: String::new(),
            root_tail: String::new(),
            before_terrain_type: line(1),
            attributes: TerrainAttribute::ALL
                .into_iter()
                .map(|attribute| AttributeFormat {
                    attribute,
                    before: CLIENT_ATTRIBUTE_SEPARATOR.to_owned(),
                    equals: CLIENT_ATTRIBUTE_EQUALS.to_owned(),
                    quote: CLIENT_QUOTE,
                    raw_value: String::new(),
                })
                .collect(),
            terrain_type_tail: String::new(),
            before_type_flags: line(2),
            type_flags: TypeFlagsFormat::Empty { tail: CLIENT_SELF_CLOSING_TAIL.to_owned() },
            before_terrain_type_end: line(1),
            terrain_type_end_tail: String::new(),
            before_root_end: line(0),
            root_end_tail: String::new(),
            epilog: CLIENT_LINE_BREAK.to_owned(),
        }
    }

    /// The indentation one level deeper than `<terrainType>` adds to it (four spaces when it cannot tell).
    fn indent_unit(&self) -> &str {
        let (outer, inner) = (line_indent(&self.before_terrain_type), line_indent(&self.before_type_flags));
        inner.strip_prefix(outer).filter(|unit| !unit.is_empty()).unwrap_or(CLIENT_INDENT_UNIT)
    }

    /// The line break and indentation of `<typeFlags>`, one `indent_unit` deeper when `deeper`; nothing when the
    /// elements share a line.
    fn type_flags_line(&self, deeper: bool) -> String {
        let gap = &self.before_type_flags;
        if !gap.contains('\n') {
            return String::new();
        }
        let line_break = if gap.contains(CLIENT_LINE_BREAK) { CLIENT_LINE_BREAK } else { "\n" };
        let unit = if deeper { self.indent_unit() } else { "" };
        format!("{line_break}{}{unit}", line_indent(gap))
    }
}

/// The text after the last line break of `gap` (all of it when it has none).
fn line_indent(gap: &str) -> &str {
    gap.rfind('\n').map_or(gap, |index| &gap[index + 1..])
}

/// A LegacyTerrain document: its typed definition and its formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTerrainDocument {
    pub terrain: LegacyTerrainType,
    format: DocumentFormat,
}

impl LegacyTerrainDocument {
    /// A document of `terrain` formatted like most client files.
    pub fn new(terrain: LegacyTerrainType) -> Self {
        Self { terrain, format: DocumentFormat::client_default() }
    }

    /// The same formatting with another definition: for a new entry styled like an existing one.
    pub fn with_terrain(&self, terrain: LegacyTerrainType) -> Self {
        Self { terrain, format: self.format.clone() }
    }

    /// Parses a UTF-8 document of the LegacyTerrain schema.
    pub fn parse(bytes: &[u8]) -> eyre::Result<Self> {
        let text = std::str::from_utf8(bytes).wrap_err("a LegacyTerrain document is not UTF-8")?;
        let mut scanner = Scanner { text, position: 0 };
        let (terrain, format) = scanner.document()?;
        Ok(Self { terrain, format })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_xml().into_bytes()
    }

    /// The document text.
    pub fn to_xml(&self) -> String {
        let format = &self.format;
        let mut out = String::new();
        out.push_str(&format.prolog);
        push_tag(&mut out, ROOT_ELEMENT, &format.root_tail, ">");
        out.push_str(&format.before_terrain_type);
        out.push('<');
        out.push_str(TERRAIN_TYPE_ELEMENT);
        for attribute in &format.attributes {
            out.push_str(&attribute.before);
            out.push_str(attribute.attribute.xml_name());
            out.push_str(&attribute.equals);
            out.push(attribute.quote);
            out.push_str(&self.attribute_text(attribute));
            out.push(attribute.quote);
        }
        out.push_str(&format.terrain_type_tail);
        out.push('>');
        out.push_str(&format.before_type_flags);
        self.push_type_flags(&mut out);
        out.push_str(&format.before_terrain_type_end);
        push_end_tag(&mut out, TERRAIN_TYPE_ELEMENT, &format.terrain_type_end_tail);
        out.push_str(&format.before_root_end);
        push_end_tag(&mut out, ROOT_ELEMENT, &format.root_end_tail);
        out.push_str(&format.epilog);
        out
    }

    /// The stored spelling of `attribute` while it still means the typed value, else the typed value escaped.
    fn attribute_text(&self, attribute: &AttributeFormat) -> String {
        let decoded = decode_attribute_value(&attribute.raw_value).ok();
        let keeps = |value: u32| decoded.as_deref().and_then(|text| text.parse::<u32>().ok()) == Some(value);
        match attribute.attribute {
            TerrainAttribute::Id if keeps(self.terrain.id) => attribute.raw_value.clone(),
            TerrainAttribute::Id => self.terrain.id.to_string(),
            TerrainAttribute::TexMap if keeps(self.terrain.tex_map) => attribute.raw_value.clone(),
            TerrainAttribute::TexMap => self.terrain.tex_map.to_string(),
            TerrainAttribute::Name if decoded.as_deref() == Some(self.terrain.name.as_str()) => {
                attribute.raw_value.clone()
            }
            TerrainAttribute::Name => encode_attribute_value(&self.terrain.name, attribute.quote),
        }
    }

    fn push_type_flags(&self, out: &mut String) {
        let format = &self.format;
        let flags = &self.terrain.flags;
        match &format.type_flags {
            TypeFlagsFormat::Empty { tail } if flags.is_empty() => push_tag(out, TYPE_FLAGS_ELEMENT, tail, "/>"),
            // Every flag removed: an empty element, closed like the document's flags.
            TypeFlagsFormat::Open { flags: stored, .. } if flags.is_empty() && !stored.is_empty() => {
                push_tag(out, TYPE_FLAGS_ELEMENT, &stored[stored.len() - 1].tail, "/>")
            }
            TypeFlagsFormat::Open { tail, flags: stored, before_end, end_tail } => {
                let (before, flag_tail) = match stored.last() {
                    Some(last) => (last.before.clone(), last.tail.as_str()),
                    None => (format.type_flags_line(true), CLIENT_SELF_CLOSING_TAIL),
                };
                push_tag(out, TYPE_FLAGS_ELEMENT, tail, ">");
                push_flags(out, flags, stored, &before, flag_tail);
                out.push_str(before_end);
                push_end_tag(out, TYPE_FLAGS_ELEMENT, end_tail);
            }
            // The first flags of the document: one level deeper than `<typeFlags>`, closed like it.
            TypeFlagsFormat::Empty { tail } => {
                push_tag(out, TYPE_FLAGS_ELEMENT, "", ">");
                push_flags(out, flags, &[], &format.type_flags_line(true), tail);
                out.push_str(&format.type_flags_line(false));
                push_end_tag(out, TYPE_FLAGS_ELEMENT, "");
            }
        }
    }
}

/// `<{name}{tail}{close}`.
fn push_tag(out: &mut String, name: &str, tail: &str, close: &str) {
    out.push('<');
    out.push_str(name);
    out.push_str(tail);
    out.push_str(close);
}

/// `</{name}{tail}>`.
fn push_end_tag(out: &mut String, name: &str, tail: &str) {
    out.push_str("</");
    out.push_str(name);
    out.push_str(tail);
    out.push('>');
}

/// Writes `flags`, each with the formatting of the first unused stored flag of the same kind, else with `before`
/// and `tail`.
fn push_flags(out: &mut String, flags: &[LegacyTerrainFlag], stored: &[FlagFormat], before: &str, tail: &str) {
    let mut used = vec![false; stored.len()];
    for &flag in flags {
        let (flag_before, flag_tail) =
            match stored.iter().enumerate().position(|(index, format)| !used[index] && format.flag == flag) {
                Some(index) => {
                    used[index] = true;
                    (stored[index].before.as_str(), stored[index].tail.as_str())
                }
                None => (before, tail),
            };
        out.push_str(flag_before);
        push_tag(out, flag.element_name(), flag_tail, "/>");
    }
}

/// The value of an attribute: entity and character references resolved, literal tabs and line breaks normalized to
/// spaces (XML attribute-value normalization).
fn decode_attribute_value(raw: &str) -> eyre::Result<String> {
    let mut value = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(character) = rest.chars().next() {
        rest = &rest[character.len_utf8()..];
        match character {
            '&' => {
                let end = rest.find(';').ok_or_else(|| eyre!("unterminated reference in attribute value {raw:?}"))?;
                value.push(decode_reference(&rest[..end]).ok_or_else(|| {
                    eyre!("unknown reference &{}; in attribute value {raw:?}", &rest[..end])
                })?);
                rest = &rest[end + 1..];
            }
            '<' => bail!("'<' in attribute value {raw:?}"),
            '\r' => {
                value.push(' ');
                rest = rest.strip_prefix('\n').unwrap_or(rest);
            }
            '\t' | '\n' => value.push(' '),
            _ => value.push(character),
        }
    }
    Ok(value)
}

/// The character of the reference `&{reference};`.
fn decode_reference(reference: &str) -> Option<char> {
    const HEX_RADIX: u32 = 16;
    const DECIMAL_RADIX: u32 = 10;
    let code = match reference {
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "amp" => return Some('&'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        _ => match reference.strip_prefix("#x") {
            Some(hex) => u32::from_str_radix(hex, HEX_RADIX).ok()?,
            None => u32::from_str_radix(reference.strip_prefix('#')?, DECIMAL_RADIX).ok()?,
        },
    };
    char::from_u32(code)
}

/// `value` escaped for an attribute quoted with `quote`; tabs and line breaks become character references so they
/// survive attribute-value normalization.
fn encode_attribute_value(value: &str, quote: char) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => encoded.push_str("&amp;"),
            '<' => encoded.push_str("&lt;"),
            '>' => encoded.push_str("&gt;"),
            '"' if quote == '"' => encoded.push_str("&quot;"),
            '\'' if quote == '\'' => encoded.push_str("&apos;"),
            '\t' => encoded.push_str("&#x9;"),
            '\n' => encoded.push_str("&#xA;"),
            '\r' => encoded.push_str("&#xD;"),
            _ => encoded.push(character),
        }
    }
    encoded
}

fn is_xml_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n')
}

/// Where a gap between markup may hold processing instructions as well as whitespace and comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gap {
    Prolog,
    Content,
}

/// An attribute as written: `{before}{name}{equals}{quote}{raw_value}{quote}`.
struct RawAttribute<'a> {
    before: String,
    name: &'a str,
    equals: String,
    quote: char,
    raw_value: &'a str,
}

/// A cursor over the document text.
struct Scanner<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Scanner<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn eat(&mut self, prefix: &str) -> bool {
        let found = self.rest().starts_with(prefix);
        if found {
            self.position += prefix.len();
        }
        found
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> &'a str {
        let rest = self.rest();
        let length = rest.find(|character| !keep(character)).unwrap_or(rest.len());
        self.position += length;
        &rest[..length]
    }

    /// Skips past `terminator`, which must follow.
    fn skip_past(&mut self, terminator: &str, what: &str) -> eyre::Result<()> {
        let end = self.rest().find(terminator).ok_or_else(|| eyre!("unterminated {what} at byte {}", self.position))?;
        self.position += end + terminator.len();
        Ok(())
    }

    /// Whitespace and comments (and processing instructions in the prolog), as written.
    fn gap(&mut self, gap: Gap) -> eyre::Result<String> {
        let start = self.position;
        loop {
            self.take_while(is_xml_whitespace);
            if self.eat("<!--") {
                self.skip_past("-->", "comment")?;
            } else if gap == Gap::Prolog && self.eat("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else {
                break;
            }
        }
        Ok(self.text[start..self.position].to_owned())
    }

    fn name(&mut self) -> &'a str {
        self.take_while(|character| {
            !is_xml_whitespace(character) && !matches!(character, '/' | '>' | '<' | '=' | '"' | '\'')
        })
    }

    /// `<{name}`, failing on another element or on text.
    fn open_tag(&mut self, expected: &str) -> eyre::Result<()> {
        let at = self.position;
        ensure!(self.eat("<"), "expected <{expected}> at byte {at}, found text");
        let name = self.name();
        ensure!(name == expected, "expected <{expected}> at byte {at}, found <{name}>");
        Ok(())
    }

    /// The attributes of a tag (whitespace before each, name, `=` with its spacing, quote, raw value), the tail
    /// whitespace and whether the tag is self-closing.
    fn tag_rest(&mut self) -> eyre::Result<(Vec<RawAttribute<'a>>, String, bool)> {
        let mut attributes = Vec::new();
        loop {
            let before = self.take_while(is_xml_whitespace).to_owned();
            if self.eat("/>") {
                return Ok((attributes, before, true));
            }
            if self.eat(">") {
                return Ok((attributes, before, false));
            }
            let at = self.position;
            ensure!(!before.is_empty(), "expected whitespace before the attribute at byte {at}");
            let name = self.name();
            ensure!(!name.is_empty(), "malformed tag at byte {at}");
            let equals_start = self.position;
            self.take_while(is_xml_whitespace);
            ensure!(self.eat("="), "attribute {name} has no value at byte {at}");
            self.take_while(is_xml_whitespace);
            let equals = self.text[equals_start..self.position].to_owned();
            let quote = self.rest().chars().next().filter(|quote| matches!(quote, '"' | '\''));
            let quote = quote.ok_or_else(|| eyre!("attribute {name} at byte {at} is not quoted"))?;
            self.position += quote.len_utf8();
            let value_start = self.position;
            self.skip_past(&quote.to_string(), "attribute value")?;
            let raw_value = &self.text[value_start..self.position - quote.len_utf8()];
            attributes.push(RawAttribute { before, name, equals, quote, raw_value });
        }
    }

    /// `</{name}{tail}>`; returns the tail.
    fn end_tag(&mut self, name: &str) -> eyre::Result<String> {
        let at = self.position;
        ensure!(self.eat("</") && self.name() == name, "expected </{name}> at byte {at}");
        let tail = self.take_while(is_xml_whitespace).to_owned();
        ensure!(self.eat(">"), "malformed </{name}> at byte {at}");
        Ok(tail)
    }

    /// A tag without attributes that must (`self_closing`) or must not close itself; returns its tail.
    fn plain_tag(&mut self, name: &str, self_closing: bool) -> eyre::Result<String> {
        let at = self.position;
        let (attributes, tail, closed) = self.tag_rest()?;
        ensure!(attributes.is_empty(), "<{name}> at byte {at} has attributes");
        ensure!(closed == self_closing, "<{name}> at byte {at} {} an empty element", if self_closing { "is not" } else { "is" });
        Ok(tail)
    }

    fn document(&mut self) -> eyre::Result<(LegacyTerrainType, DocumentFormat)> {
        self.eat(UTF8_BOM);
        self.gap(Gap::Prolog)?;
        let prolog = self.text[..self.position].to_owned();
        self.open_tag(ROOT_ELEMENT)?;
        let root_tail = self.plain_tag(ROOT_ELEMENT, false)?;
        let before_terrain_type = self.gap(Gap::Content)?;

        self.open_tag(TERRAIN_TYPE_ELEMENT)?;
        let at = self.position;
        let (raw_attributes, terrain_type_tail, closed) = self.tag_rest()?;
        ensure!(!closed, "<{TERRAIN_TYPE_ELEMENT}> at byte {at} has no <{TYPE_FLAGS_ELEMENT}>");
        let mut terrain = LegacyTerrainType::default();
        let mut attributes: Vec<AttributeFormat> = Vec::new();
        for RawAttribute { before, name, equals, quote, raw_value } in raw_attributes {
            let attribute = TerrainAttribute::from_xml_name(name)
                .ok_or_else(|| eyre!("unknown attribute {name} of <{TERRAIN_TYPE_ELEMENT}>"))?;
            ensure!(attributes.iter().all(|seen| seen.attribute != attribute), "repeated attribute {name}");
            let value = decode_attribute_value(raw_value)?;
            let number = || value.parse::<u32>().wrap_err_with(|| format!("{name}={value:?} is not a number"));
            match attribute {
                TerrainAttribute::Id => terrain.id = number()?,
                TerrainAttribute::TexMap => terrain.tex_map = number()?,
                TerrainAttribute::Name => terrain.name = value.clone(),
            }
            attributes.push(AttributeFormat { attribute, before, equals, quote, raw_value: raw_value.to_owned() });
        }
        for attribute in TerrainAttribute::ALL {
            ensure!(
                attributes.iter().any(|seen| seen.attribute == attribute),
                "<{TERRAIN_TYPE_ELEMENT}> has no {} attribute",
                attribute.xml_name()
            );
        }
        let before_type_flags = self.gap(Gap::Content)?;

        self.open_tag(TYPE_FLAGS_ELEMENT)?;
        let at = self.position;
        let (flags_attributes, flags_tail, flags_closed) = self.tag_rest()?;
        ensure!(flags_attributes.is_empty(), "<{TYPE_FLAGS_ELEMENT}> at byte {at} has attributes");
        let type_flags = if flags_closed {
            TypeFlagsFormat::Empty { tail: flags_tail }
        } else {
            let mut flags = Vec::new();
            let before_end = loop {
                let before = self.gap(Gap::Content)?;
                if self.rest().starts_with("</") {
                    break before;
                }
                let at = self.position;
                ensure!(self.eat("<"), "text inside <{TYPE_FLAGS_ELEMENT}> at byte {at}");
                let name = self.name();
                let flag = LegacyTerrainFlag::from_element_name(name)
                    .ok_or_else(|| eyre!("unknown flag <{name}> at byte {at}"))?;
                let tail = self.plain_tag(name, true)?;
                terrain.flags.push(flag);
                flags.push(FlagFormat { flag, before, tail });
            };
            let end_tail = self.end_tag(TYPE_FLAGS_ELEMENT)?;
            TypeFlagsFormat::Open { tail: flags_tail, flags, before_end, end_tail }
        };

        let before_terrain_type_end = self.gap(Gap::Content)?;
        let terrain_type_end_tail = self.end_tag(TERRAIN_TYPE_ELEMENT)?;
        let before_root_end = self.gap(Gap::Content)?;
        let root_end_tail = self.end_tag(ROOT_ELEMENT)?;
        let epilog = self.gap(Gap::Content)?;
        ensure!(self.rest().is_empty(), "content after </{ROOT_ELEMENT}> at byte {}", self.position);

        let format = DocumentFormat {
            prolog,
            root_tail,
            before_terrain_type,
            attributes,
            terrain_type_tail,
            before_type_flags,
            type_flags,
            before_terrain_type_end,
            terrain_type_end_tail,
            before_root_end,
            root_end_tail,
            epilog,
        };
        Ok((terrain, format))
    }
}

/// The definition of land tile `id` in `package`, if it has one.
pub fn read_legacy_terrain(package: &UopPackage, id: u32) -> eyre::Result<Option<LegacyTerrainDocument>> {
    let hash = legacy_terrain_entry_hash(id);
    let Some(payload) =
        package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack LegacyTerrain entry {id}"))?
    else {
        return Ok(None);
    };
    LegacyTerrainDocument::parse(&payload).wrap_err_with(|| format!("LegacyTerrain entry {id}")).map(Some)
}

/// Every definition of `package` by land tile id. Fails on an entry that does not parse or that is not the entry of
/// its own id.
pub fn read_legacy_terrain_documents(package: &UopPackage) -> eyre::Result<BTreeMap<u32, LegacyTerrainDocument>> {
    let mut documents = BTreeMap::new();
    for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
        let hash = file.filename_hash();
        let payload = file.unpack().wrap_err_with(|| format!("failed to unpack LegacyTerrain entry {hash:016X}"))?;
        let document =
            LegacyTerrainDocument::parse(&payload).wrap_err_with(|| format!("LegacyTerrain entry {hash:016X}"))?;
        let id = document.terrain.id;
        ensure!(
            legacy_terrain_entry_hash(id) == hash,
            "LegacyTerrain entry {hash:016X} holds land tile {id}, whose entry is {}",
            legacy_terrain_entry_path(id)
        );
        documents.insert(id, document);
    }
    Ok(documents)
}

/// Reads every definition of the package at `path` ([`read_legacy_terrain_documents`]).
pub fn load_legacy_terrain_documents(path: impl AsRef<Path>) -> eyre::Result<BTreeMap<u32, LegacyTerrainDocument>> {
    let path = path.as_ref();
    let package = UopPackage::load(path).wrap_err_with(|| format!("failed to load {}", path.display()))?;
    read_legacy_terrain_documents(&package)
}

/// What [`apply_legacy_terrain_edits`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyTerrainChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

/// Stores each of `documents` as the entry of its land tile id and removes the entries of the `removed` ids; every
/// other entry is left untouched (same compressed bytes and data block header). A document whose entry already
/// holds its bytes is left untouched; a changed one is replaced keeping its compression flag; a new one is added
/// with the compression of the package's entries. Both get a fresh timestamp data block header (the package's edit
/// time). Fails on an id listed twice or both stored and removed; a removed id without an entry is not counted.
pub fn apply_legacy_terrain_edits(
    package: &mut UopPackage,
    documents: &[LegacyTerrainDocument],
    removed: &[u32],
) -> eyre::Result<LegacyTerrainChanges> {
    let mut stored = BTreeSet::new();
    for document in documents {
        let id = document.terrain.id;
        ensure!(stored.insert(id), "land tile {id} is listed twice");
        ensure!(!removed.contains(&id), "land tile {id} is both stored and removed");
    }
    let compression = package
        .iter_files()
        .find(|file| file.has_size())
        .map_or(DEFAULT_LEGACY_TERRAIN_COMPRESSION, |file| file.compression());
    let mut changes = LegacyTerrainChanges::default();
    for document in documents {
        let hash = legacy_terrain_entry_hash(document.terrain.id);
        let payload = document.to_bytes();
        let current = package
            .unpack_file_by_hash(hash)
            .wrap_err_with(|| format!("failed to unpack LegacyTerrain entry {}", document.terrain.id))?;
        if current.as_deref() == Some(payload.as_slice()) {
            continue;
        }
        match uop_entries::put_entry(package, hash, &payload, compression)? {
            EntryPut::Replaced => changes.replaced += 1,
            EntryPut::Added => changes.added += 1,
        }
    }
    for id in removed.iter().copied().collect::<BTreeSet<_>>() {
        if uop_entries::remove_entry(package, legacy_terrain_entry_hash(id)) {
            changes.removed += 1;
        }
    }
    Ok(changes)
}

/// Writes a copy of the `source` `LegacyTerrain.uop` to `output` with [`apply_legacy_terrain_edits`] applied
/// (stamped with the current time); the package keeps its version, block size and layout. `output` must not be
/// `source`.
pub fn write_legacy_terrain_package(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    documents: &[LegacyTerrainDocument],
    removed: &[u32],
) -> eyre::Result<LegacyTerrainChanges> {
    let (source, output) = (source.as_ref(), output.as_ref());
    uop_entries::refuse_to_overwrite(source, output)?;
    let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
    let changes = apply_legacy_terrain_edits(&mut package, documents, removed)?;
    uop_entries::save_package_as(&mut package, source, output)?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The most common client shape, with one flag.
    const CLIENT_WITH_FLAG: &str = "<legacyTerrain>\r\n    <terrainType id=\"15093\" name=\"leaves\" texMap=\"15093\">\r\n        <typeFlags>\r\n            <impassable />\r\n        </typeFlags>\r\n    </terrainType>\r\n</legacyTerrain>\r\n";
    /// The most common client shape, without flags.
    const CLIENT_WITHOUT_FLAGS: &str = "<legacyTerrain>\r\n    <terrainType id=\"1417\" name=\"jungle\" texMap=\"1417\">\r\n        <typeFlags />\r\n    </terrainType>\r\n</legacyTerrain>\r\n";
    /// A tab-indented client shape without the space before `/>`.
    const CLIENT_TABS: &str = "<legacyTerrain>\r\n\t<terrainType id=\"3\" name=\"water\" texMap=\"2\">\r\n\t\t<typeFlags>\r\n\t\t\t<wet/>\r\n\t\t\t<impassable/>\r\n\t\t</typeFlags>\r\n\t</terrainType>\r\n</legacyTerrain>\r\n";
    /// Irregular client whitespace: mixed indentation, trailing spaces, a blank line at the end.
    const CLIENT_IRREGULAR: &str = "<legacyTerrain>\r\n    <terrainType id=\"7\" name=\"rock\" texMap=\"7\">       \r\n        <typeFlags>\r\n \t    <impassable /> \r\n            <wall />\r\n        </typeFlags>\r\n    </terrainType>\r\n</legacyTerrain>\r\n\r\n";
    /// What the client files never hold: BOM, declaration, comments, other attribute order, quotes and spacing,
    /// references, tag tails, LF line breaks, no final line break.
    const UNUSUAL: &str = "\u{FEFF}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- terrain -->\n<legacyTerrain >\n  <terrainType texMap = '0012' name='a &amp; b&#x21;' id=\"5\" >\n    <!-- none -->\n    <typeFlags></typeFlags >\n  </terrainType>\n</legacyTerrain> <!-- end -->";

    fn parse(text: &str) -> LegacyTerrainDocument {
        LegacyTerrainDocument::parse(text.as_bytes()).unwrap()
    }

    fn terrain(id: u32, name: &str, tex_map: u32, flags: &[LegacyTerrainFlag]) -> LegacyTerrainType {
        LegacyTerrainType { id, name: name.to_owned(), tex_map, flags: flags.to_vec() }
    }

    #[test]
    fn documents_parse_to_the_typed_definition_and_write_back_byte_for_byte() {
        use LegacyTerrainFlag::*;
        let cases = [
            (CLIENT_WITH_FLAG, terrain(15093, "leaves", 15093, &[Impassable])),
            (CLIENT_WITHOUT_FLAGS, terrain(1417, "jungle", 1417, &[])),
            (CLIENT_TABS, terrain(3, "water", 2, &[Wet, Impassable])),
            (CLIENT_IRREGULAR, terrain(7, "rock", 7, &[Impassable, Wall])),
            (UNUSUAL, terrain(5, "a & b!", 12, &[])),
        ];
        for (text, expected) in cases {
            let document = parse(text);
            assert_eq!(document.terrain, expected);
            assert_eq!(document.to_xml(), text);
        }
    }

    #[test]
    fn a_new_document_has_the_client_formatting() {
        let document = LegacyTerrainDocument::new(terrain(15093, "leaves", 15093, &[LegacyTerrainFlag::Impassable]));
        assert_eq!(document.to_xml(), CLIENT_WITH_FLAG);
        let document = LegacyTerrainDocument::new(terrain(1417, "jungle", 1417, &[]));
        assert_eq!(document.to_xml(), CLIENT_WITHOUT_FLAGS);
        assert_eq!(parse(CLIENT_TABS).with_terrain(document.terrain.clone()).to_xml(), CLIENT_TABS.replace("id=\"3\" name=\"water\" texMap=\"2\"", "id=\"1417\" name=\"jungle\" texMap=\"1417\"").replace("<typeFlags>\r\n\t\t\t<wet/>\r\n\t\t\t<impassable/>\r\n\t\t</typeFlags>", "<typeFlags/>"));
    }

    #[test]
    fn edited_values_are_written_fresh_and_escaped_for_their_quote() {
        let mut document = parse(UNUSUAL);
        document.terrain.name = "it's <new> & \"quoted\"".to_owned();
        document.terrain.tex_map = 12;
        document.terrain.id = 6;
        let text = document.to_xml();
        assert!(text.contains("texMap = '0012' name='it&apos;s &lt;new&gt; &amp; \"quoted\"' id=\"6\" >"), "{text}");
        assert_eq!(parse(&text).terrain, document.terrain);
    }

    #[test]
    fn added_and_removed_flags_follow_the_document_style() {
        use LegacyTerrainFlag::*;
        let mut document = parse(CLIENT_WITHOUT_FLAGS);
        document.terrain.set_flag(Impassable, true);
        assert_eq!(document.to_xml(), CLIENT_WITH_FLAG.replace("15093", "1417").replace("leaves", "jungle"));

        let mut document = parse(CLIENT_WITH_FLAG);
        document.terrain.set_flag(Impassable, false);
        assert_eq!(document.to_xml(), CLIENT_WITHOUT_FLAGS.replace("1417", "15093").replace("jungle", "leaves"));

        let mut document = parse(CLIENT_TABS);
        document.terrain.set_flag(Wet, false);
        document.terrain.set_flag(Surface, true);
        document.terrain.set_flag(Impassable, true);
        let expected = CLIENT_TABS.replace("\t\t\t<wet/>\r\n", "").replace("<impassable/>", "<impassable/>\r\n\t\t\t<surface/>");
        assert_eq!(document.to_xml(), expected);
        document.terrain.flags.clear();
        assert!(document.to_xml().contains("\t\t<typeFlags/>\r\n\t</terrainType>"));

        // Each flag keeps the whitespace before it, the end of the previous line included.
        let mut document = parse(CLIENT_IRREGULAR);
        document.terrain.flags.reverse();
        let expected = CLIENT_IRREGULAR.replace(
            "<typeFlags>\r\n \t    <impassable /> \r\n            <wall />",
            "<typeFlags> \r\n            <wall />\r\n \t    <impassable />",
        );
        assert_eq!(document.to_xml(), expected);

        let mut document = parse(UNUSUAL);
        document.terrain.set_flag(NoDiagonal, true);
        let text = document.to_xml();
        assert!(text.contains("<!-- none -->\n    <typeFlags>\n      <no_diagonal /></typeFlags >"), "{text}");
        assert_eq!(parse(&text).terrain, document.terrain);
    }

    #[test]
    fn edited_documents_parse_back_to_their_definition() {
        for text in [CLIENT_WITH_FLAG, CLIENT_WITHOUT_FLAGS, CLIENT_TABS, CLIENT_IRREGULAR, UNUSUAL] {
            for flag in LegacyTerrainFlag::ALL {
                let mut document = parse(text);
                let on = !document.terrain.has_flag(flag);
                document.terrain.set_flag(flag, on);
                document.terrain.name.push_str(" & more");
                assert_eq!(parse(&document.to_xml()).terrain, document.terrain, "{text:?} {flag:?}");
            }
        }
    }

    #[test]
    fn documents_outside_the_schema_are_rejected() {
        let invalid = [
            CLIENT_WITH_FLAG.replace("impassable", "flying"),
            CLIENT_WITH_FLAG.replace("texMap", "texture"),
            CLIENT_WITH_FLAG.replace(" texMap=\"15093\"", ""),
            CLIENT_WITH_FLAG.replace(" texMap=\"15093\"", " id=\"1\" texMap=\"15093\""),
            CLIENT_WITH_FLAG.replace("id=\"15093\"", "id=\"-1\""),
            CLIENT_WITH_FLAG.replace("leaves", "a & b"),
            CLIENT_WITH_FLAG.replace("leaves", "&unknown;"),
            CLIENT_WITH_FLAG.replace("<impassable />", "<impassable></impassable>"),
            CLIENT_WITH_FLAG.replace("<impassable />", "impassable"),
            CLIENT_WITH_FLAG.replace("<typeFlags>", "<typeFlags kind=\"x\">"),
            CLIENT_WITHOUT_FLAGS.replace("<typeFlags />", ""),
            CLIENT_WITHOUT_FLAGS.replace("</legacyTerrain>", "</legacyTerrain><legacyTerrain/>"),
            CLIENT_WITHOUT_FLAGS.replace("name=\"jungle\"", "name=jungle"),
            CLIENT_WITHOUT_FLAGS.replace("</terrainType>", ""),
        ];
        for text in &invalid {
            assert!(LegacyTerrainDocument::parse(text.as_bytes()).is_err(), "{text:?}");
        }
        assert!(LegacyTerrainDocument::parse(b"<legacyTerrain>\xff</legacyTerrain>").is_err());
    }

    #[test]
    fn edits_replace_add_and_remove_entries() {
        use LegacyTerrainFlag::*;
        const EDIT_FILETIME: u64 = 0x01db_2a3b_4c5d_6e7f;
        let mut package = UopPackage::new_default();
        package.set_edit_filetime(Some(EDIT_FILETIME));
        for text in [CLIENT_WITH_FLAG, CLIENT_WITHOUT_FLAGS, CLIENT_TABS] {
            let id = parse(text).terrain.id;
            package.add_file_from_memory(text.as_bytes(), &legacy_terrain_entry_path(id), CompressionFlag::Zlib).unwrap();
        }
        let documents = read_legacy_terrain_documents(&package).unwrap();
        assert_eq!(documents.keys().copied().collect::<Vec<_>>(), [3, 1417, 15093]);

        let mut edited = documents[&1417].clone();
        edited.terrain.set_flag(Wet, true);
        let added = documents[&3].with_terrain(terrain(4, "deep water", 2, &[Wet, Impassable]));
        let unchanged = documents[&15093].clone();
        let changes =
            apply_legacy_terrain_edits(&mut package, &[edited.clone(), added.clone(), unchanged], &[3, 99]).unwrap();
        assert_eq!(changes, LegacyTerrainChanges { replaced: 1, added: 1, removed: 1 });
        assert_eq!(read_legacy_terrain(&package, 1417).unwrap().unwrap().to_xml(), edited.to_xml());
        assert_eq!(read_legacy_terrain(&package, 4).unwrap().unwrap().to_xml(), CLIENT_TABS.replace("id=\"3\" name=\"water\"", "id=\"4\" name=\"deep water\""));
        assert_eq!(read_legacy_terrain(&package, 3).unwrap(), None);
        let added_file = package.get_file_by_hash(legacy_terrain_entry_hash(4)).unwrap();
        assert_eq!(added_file.compression(), CompressionFlag::Zlib, "a new entry takes the package's compression");

        assert!(apply_legacy_terrain_edits(&mut package, &[edited.clone(), edited.clone()], &[]).is_err());
        assert!(apply_legacy_terrain_edits(&mut package, &[edited], &[1417]).is_err());
    }
}
