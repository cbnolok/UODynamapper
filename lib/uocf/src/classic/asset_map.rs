//! `data/assetmap.xml`, the map from a resource type and number to the loose file or UOP entry that holds it,
//! read as the 2024 Classic client reads it, strictly, and written back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicUiFiles::AssetMap_Load` and the handlers it names,
//! `scripts/dev-helpers/ec-research/ghidra-classic-ui-files`): the client loads `Data/assetmap.xml` with TinyXML
//! through `VirtualFile_Open` in its UOP-only mode, so the copy it reads is the `data/assetmap.xml` entry of
//! `MainMisc.uop` (the loose `Data/assetmap.xml` the 2024 and fp clients ship, with `../../Runtime/` paths, is
//! not read). The root must be `<assetmap>` ("Assetmap root node isn't <assetmap>"); each child element is
//! offered to three handlers, matched by name (case-sensitive), and any other is skipped ("Node X ignored - no
//! valid mapping found."):
//!
//! - `simplemap`: `resourcetype`, `resourcenumber` (or, without it, `id`, a `u64`), `filename`, `fileFormat`;
//! - `patternmap`: `resourcetype`, `fileFormat`, `directory`, `prefix`, `suffix`, `padding`, `patternMatch`
//!   (an `i32`, true when nonzero);
//! - `bitmaskmap`: `resourcetype`, `fileFormat`, `directory`, `suffix`, and `section` children with `prefix`,
//!   `padding`, `mask` (hexadecimal), `patternMatch` and `isDirectory` (true when nonzero).
//!
//! Numbers are read with `strtoul`/`strtol` base 10 (`mask` base 16) and an attribute whose value does not parse
//! whole counts as absent; `resourcetype` keeps its low byte; `fileFormat` is `raw` (0) or `packed` (1), anything
//! else reading as raw. Missing attributes are 0 or empty. The UOKR and EC executables hold the same names
//! (strings only). The shipped copies (CC and EC `MainMisc.uop` identical, the KR one older) use only these
//! elements, one comment, CR LF and ASCII, no declaration and no entity.
//!
//! Strict where the client would read something else than an editor shows: an unknown element or attribute (the
//! client skips it), a repeated attribute, a missing `resourcetype`, a `simplemap` without a resource number or
//! with both, a value the client would read as absent, saturate, truncate or read as raw, text outside the tags,
//! and the XML this subset leaves out (declarations, entities, CDATA, whitespace around `=`, non-ASCII bytes) are
//! errors with the line and column ([`DefSyntaxError`]). Whitespace, comments, quotes and attribute order are kept,
//! so the file writes back byte for byte.

crate::eyre_imports!();
use std::path::Path;

use super::def_files::DefSyntaxError;
use crate::utils::output_file::write_through_temp;

/// The entry path in `MainMisc.uop`.
pub const ASSET_MAP_UOP_PATH: &str = "data/assetmap.xml";
/// The path the client opens.
pub const ASSET_MAP_FILE: &str = "Data/assetmap.xml";
pub const ROOT_ELEMENT: &str = "assetmap";
pub const SIMPLE_MAP_ELEMENT: &str = "simplemap";
pub const PATTERN_MAP_ELEMENT: &str = "patternmap";
pub const BITMASK_MAP_ELEMENT: &str = "bitmaskmap";
pub const SECTION_ELEMENT: &str = "section";
pub const RAW_FORMAT: &str = "raw";
pub const PACKED_FORMAT: &str = "packed";
const RESOURCE_TYPE: &str = "resourcetype";
const RESOURCE_NUMBER: &str = "resourcenumber";
const ID: &str = "id";
const FILENAME: &str = "filename";
const FILE_FORMAT: &str = "fileFormat";
const DIRECTORY: &str = "directory";
const PREFIX: &str = "prefix";
const SUFFIX: &str = "suffix";
const PADDING: &str = "padding";
const PATTERN_MATCH: &str = "patternMatch";
const MASK: &str = "mask";
const IS_DIRECTORY: &str = "isDirectory";
const SIMPLE_MAP_ATTRIBUTES: [&str; 5] = [RESOURCE_TYPE, RESOURCE_NUMBER, ID, FILENAME, FILE_FORMAT];
const PATTERN_MAP_ATTRIBUTES: [&str; 7] = [RESOURCE_TYPE, FILE_FORMAT, DIRECTORY, PREFIX, SUFFIX, PADDING, PATTERN_MATCH];
const BITMASK_MAP_ATTRIBUTES: [&str; 4] = [RESOURCE_TYPE, FILE_FORMAT, DIRECTORY, SUFFIX];
const SECTION_ATTRIBUTES: [&str; 5] = [PREFIX, PADDING, MASK, PATTERN_MATCH, IS_DIRECTORY];
/// What starts a `mask` value (the client's `strtoul` base 16 also takes `0X`).
const HEX_PREFIXES: [&str; 2] = ["0x", "0X"];
const MAX_MASK_DIGITS: usize = 8;
const HEX_RADIX: u32 = 16;
const COMMENT_OPEN: &str = "<!--";
const COMMENT_CLOSE: &str = "-->";
const CLOSE_TAG_OPEN: &str = "</";
const EMPTY_TAG_CLOSE: &str = "/>";
const TAG_OPEN: u8 = b'<';
const TAG_CLOSE: u8 = b'>';
const EQUALS: u8 = b'=';
const DOUBLE_QUOTE: u8 = b'"';
const SINGLE_QUOTE: u8 = b'\'';
const AMPERSAND: u8 = b'&';
const NEGATIVE_SIGN: char = '-';
/// What a new attribute is preceded by.
const ATTRIBUTE_SPACE: &str = " ";
/// What a new `section` is indented by, past its `bitmaskmap`.
const SECTION_INDENT: &str = "   ";

/// How a mapped file is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// Loose (`raw`, 0); also what an absent `fileFormat` reads as.
    Raw,
    /// Built for a UOP (`packed`, 1).
    Packed,
}

impl FileFormat {
    fn name(self) -> &'static str {
        match self {
            Self::Raw => RAW_FORMAT,
            Self::Packed => PACKED_FORMAT,
        }
    }
}

/// The resource a `simplemap` maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleResource {
    /// `resourcenumber`.
    Number(u32),
    /// `id`, read when there is no `resourcenumber`.
    Id(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleMap {
    pub resource_type: u8,
    pub file_format: FileFormat,
    pub resource: SimpleResource,
    pub filename: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternMap {
    pub resource_type: u8,
    pub file_format: FileFormat,
    pub directory: String,
    pub prefix: String,
    pub suffix: String,
    pub padding: u32,
    pub pattern_match: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmaskSection {
    pub prefix: String,
    pub padding: u32,
    pub mask: u32,
    pub pattern_match: bool,
    pub is_directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmaskMap {
    pub resource_type: u8,
    pub file_format: FileFormat,
    pub directory: String,
    pub suffix: String,
    pub sections: Vec<BitmaskSection>,
}

/// A child of `<assetmap>`, as the client reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetMapping {
    Simple(SimpleMap),
    Pattern(PatternMap),
    Bitmask(BitmaskMap),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attribute {
    /// The whitespace before the name.
    space_before: String,
    name: String,
    quote: u8,
    value: String,
    /// Where the value starts in the file read (0 for an edit).
    offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Whitespace(String),
    /// A comment's text, between `<!--` and `-->`.
    Comment(String),
    Element(Element),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Element {
    name: String,
    attributes: Vec<Attribute>,
    /// The whitespace before `/>` or `>`.
    space_before_end: String,
    /// `None` for `<name ... />`.
    content: Option<Vec<Item>>,
    /// Where the element starts in the file read (0 for an edit).
    offset: usize,
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|attribute| attribute.name == name)
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.content.iter().flatten().filter_map(|item| match item {
            Item::Element(element) => Some(element),
            _ => None,
        })
    }

    fn write(&self, out: &mut String) {
        out.push(char::from(TAG_OPEN));
        out.push_str(&self.name);
        for attribute in &self.attributes {
            out.push_str(&attribute.space_before);
            out.push_str(&attribute.name);
            out.push(char::from(EQUALS));
            out.push(char::from(attribute.quote));
            out.push_str(&attribute.value);
            out.push(char::from(attribute.quote));
        }
        out.push_str(&self.space_before_end);
        match &self.content {
            None => out.push_str(EMPTY_TAG_CLOSE),
            Some(items) => {
                out.push(char::from(TAG_CLOSE));
                write_items(items, out);
                out.push_str(CLOSE_TAG_OPEN);
                out.push_str(&self.name);
                out.push(char::from(TAG_CLOSE));
            }
        }
    }

    /// A new element with `attributes` (name, value) in that order.
    fn new(name: &str, attributes: &[(&str, String)], space_before_end: &str, content: Option<Vec<Item>>) -> Self {
        let attributes = attributes
            .iter()
            .map(|(name, value)| Attribute {
                space_before: ATTRIBUTE_SPACE.to_owned(),
                name: (*name).to_owned(),
                quote: DOUBLE_QUOTE,
                value: value.clone(),
                offset: 0,
            })
            .collect();
        Self { name: name.to_owned(), attributes, space_before_end: space_before_end.to_owned(), content, offset: 0 }
    }
}

fn write_items(items: &[Item], out: &mut String) {
    for item in items {
        match item {
            Item::Whitespace(text) => out.push_str(text),
            Item::Comment(text) => {
                out.push_str(COMMENT_OPEN);
                out.push_str(text);
                out.push_str(COMMENT_CLOSE);
            }
            Item::Element(element) => element.write(out),
        }
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
}

/// An error at a byte offset, made a [`DefSyntaxError`] once the line and column are known.
type OffsetError = (usize, String);

fn locate(text: &[u8], (offset, message): OffsetError) -> DefSyntaxError {
    let before = &text[..offset.min(text.len())];
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let column = before.iter().rev().take_while(|&&byte| byte != b'\n').count() + 1;
    DefSyntaxError { line, column, message }
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &[u8] {
        &self.text[self.at..]
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, OffsetError> {
        Err((self.at, message.into()))
    }

    fn take_while(&mut self, keep: impl Fn(u8) -> bool) -> String {
        let start = self.at;
        while self.at < self.text.len() && keep(self.text[self.at]) {
            self.at += 1;
        }
        String::from_utf8(self.text[start..self.at].to_vec()).expect("ASCII checked")
    }

    /// The items up to `</closing>` (consumed), or to the end of the file.
    fn items(&mut self, closing: Option<&str>) -> Result<Vec<Item>, OffsetError> {
        let mut items = Vec::new();
        loop {
            let Some(&byte) = self.rest().first() else {
                return match closing {
                    Some(name) => self.fail(format!("<{name}> is not closed")),
                    None => Ok(items),
                };
            };
            if is_whitespace(byte) {
                items.push(Item::Whitespace(self.take_while(is_whitespace)));
            } else if self.rest().starts_with(COMMENT_OPEN.as_bytes()) {
                let start = self.at + COMMENT_OPEN.len();
                let Some(length) = self.text[start..].windows(COMMENT_CLOSE.len()).position(|w| w == COMMENT_CLOSE.as_bytes()) else {
                    return self.fail("a comment that does not end");
                };
                items.push(Item::Comment(String::from_utf8(self.text[start..start + length].to_vec()).expect("ASCII")));
                self.at = start + length + COMMENT_CLOSE.len();
            } else if self.rest().starts_with(CLOSE_TAG_OPEN.as_bytes()) {
                let Some(name) = closing else {
                    return self.fail("a closing tag without its element");
                };
                let tag = format!("{CLOSE_TAG_OPEN}{name}{}", char::from(TAG_CLOSE));
                if !self.rest().starts_with(tag.as_bytes()) {
                    return self.fail(format!("expected `{tag}`"));
                }
                self.at += tag.len();
                return Ok(items);
            } else if byte == TAG_OPEN {
                match self.rest().get(1) {
                    Some(&next) if is_name_start(next) => items.push(Item::Element(self.element()?)),
                    Some(b'?') => return self.fail("an XML declaration or processing instruction: not in the shipped files, not supported"),
                    Some(b'!') => return self.fail("a DOCTYPE or CDATA section: not in the shipped files, not supported"),
                    _ => return self.fail("`<` not starting a tag"),
                }
            } else {
                return self.fail("text outside a tag: the client reads elements only");
            }
        }
    }

    fn element(&mut self) -> Result<Element, OffsetError> {
        let offset = self.at;
        self.at += 1;
        let name = self.take_while(is_name_byte);
        let mut attributes: Vec<Attribute> = Vec::new();
        loop {
            let space = self.take_while(is_whitespace);
            if self.rest().starts_with(EMPTY_TAG_CLOSE.as_bytes()) {
                self.at += EMPTY_TAG_CLOSE.len();
                return Ok(Element { name, attributes, space_before_end: space, content: None, offset });
            }
            if self.rest().first() == Some(&TAG_CLOSE) {
                self.at += 1;
                let content = self.items(Some(&name))?;
                return Ok(Element { name, attributes, space_before_end: space, content: Some(content), offset });
            }
            if space.is_empty() {
                return self.fail(format!("expected whitespace, `/>` or `>` in <{name}>"));
            }
            if !self.rest().first().copied().is_some_and(is_name_start) {
                return self.fail(format!("expected an attribute name in <{name}>"));
            }
            let attribute_at = self.at;
            let attribute = self.take_while(is_name_byte);
            if self.rest().first() != Some(&EQUALS) {
                return self.fail(format!("expected `=` right after `{attribute}` (whitespace around `=` is not supported)"));
            }
            self.at += 1;
            let quote = match self.rest().first() {
                Some(&quote @ (DOUBLE_QUOTE | SINGLE_QUOTE)) => quote,
                _ => return self.fail(format!("expected a quoted value right after `{attribute}=`")),
            };
            self.at += 1;
            let value_at = self.at;
            let value = self.take_while(|byte| byte != quote && byte != TAG_OPEN && byte != AMPERSAND);
            match self.rest().first() {
                Some(&byte) if byte == quote => self.at += 1,
                Some(&AMPERSAND) => return self.fail("an entity reference: not in the shipped files, not supported"),
                Some(_) => return self.fail("`<` inside an attribute value"),
                None => return self.fail("an attribute value that does not end"),
            }
            if attributes.iter().any(|existing| existing.name == attribute) {
                return Err((attribute_at, format!("`{attribute}` repeated: TinyXML refuses the document")));
            }
            attributes.push(Attribute { space_before: space, name: attribute, quote, value, offset: value_at });
        }
    }
}

/// The value of attribute `name`, if present.
fn value<'a>(element: &'a Element, name: &str) -> Option<(&'a str, usize)> {
    element.attribute(name).map(|attribute| (attribute.value.as_str(), attribute.offset))
}

fn plain_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// A `strtoul` base 10 attribute: 0 when absent.
fn unsigned(element: &Element, name: &str) -> Result<u32, OffsetError> {
    let Some((text, at)) = value(element, name) else {
        return Ok(0);
    };
    if !plain_digits(text) {
        return Err((at, format!("{name}=\"{text}\": not a plain decimal; the client would treat the attribute as absent")));
    }
    text.parse().map_err(|_| (at, format!("{name}=\"{text}\": past {}; the client's strtoul would saturate it", u32::MAX)))
}

/// A `strtol` base 10 attribute: 0 when absent.
fn signed(element: &Element, name: &str) -> Result<i32, OffsetError> {
    let Some((text, at)) = value(element, name) else {
        return Ok(0);
    };
    if !plain_digits(text.strip_prefix(NEGATIVE_SIGN).unwrap_or(text)) {
        return Err((at, format!("{name}=\"{text}\": not a plain decimal; the client would treat the attribute as absent")));
    }
    text.parse().map_err(|_| (at, format!("{name}=\"{text}\": outside i32; the client's strtol would saturate it")))
}

fn string(element: &Element, name: &str) -> String {
    value(element, name).map_or_else(String::new, |(text, _)| text.to_owned())
}

fn resource_type(element: &Element) -> Result<u8, OffsetError> {
    if value(element, RESOURCE_TYPE).is_none() {
        return Err((element.offset, format!("<{}> without {RESOURCE_TYPE}", element.name)));
    }
    let number = unsigned(element, RESOURCE_TYPE)?;
    u8::try_from(number).map_err(|_| {
        let at = value(element, RESOURCE_TYPE).map_or(element.offset, |(_, at)| at);
        (at, format!("{RESOURCE_TYPE}=\"{number}\": the client keeps only its low byte ({})", number & u32::from(u8::MAX)))
    })
}

fn file_format(element: &Element) -> Result<FileFormat, OffsetError> {
    match value(element, FILE_FORMAT) {
        None => Ok(FileFormat::Raw),
        Some((RAW_FORMAT, _)) => Ok(FileFormat::Raw),
        Some((PACKED_FORMAT, _)) => Ok(FileFormat::Packed),
        Some((other, at)) => Err((at, format!("{FILE_FORMAT}=\"{other}\": not `{RAW_FORMAT}` or `{PACKED_FORMAT}`; the client reads it as raw"))),
    }
}

fn mask(element: &Element) -> Result<u32, OffsetError> {
    let Some((text, at)) = value(element, MASK) else {
        return Ok(0);
    };
    let digits = HEX_PREFIXES.iter().find_map(|prefix| text.strip_prefix(prefix));
    match digits {
        Some(digits) if !digits.is_empty() && digits.len() <= MAX_MASK_DIGITS && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            Ok(u32::from_str_radix(digits, HEX_RADIX).expect("checked hex digits"))
        }
        _ => Err((at, format!("{MASK}=\"{text}\": not `0x` and up to {MAX_MASK_DIGITS} hex digits, as the shipped files write it"))),
    }
}

/// Unknown attributes and element children of `element`.
fn check_shape(element: &Element, attributes: &[&str], children: Option<&str>) -> Result<(), OffsetError> {
    if let Some(unknown) = element.attributes.iter().find(|attribute| !attributes.contains(&attribute.name.as_str())) {
        return Err((
            unknown.offset,
            format!("`{}` is not a <{}> attribute; the client ignores it", unknown.name, element.name),
        ));
    }
    if let Some(child) = element.elements().find(|child| Some(child.name.as_str()) != children) {
        let message = match children {
            Some(name) => format!("<{}> in <{}>: the client reads only <{name}> children", child.name, element.name),
            None => format!("<{}> in <{}>: the client reads no children", child.name, element.name),
        };
        return Err((child.offset, message));
    }
    Ok(())
}

fn mapping(element: &Element) -> Result<AssetMapping, OffsetError> {
    match element.name.as_str() {
        SIMPLE_MAP_ELEMENT => {
            check_shape(element, &SIMPLE_MAP_ATTRIBUTES, None)?;
            let resource = match (value(element, RESOURCE_NUMBER), value(element, ID)) {
                (Some(_), None) => SimpleResource::Number(unsigned(element, RESOURCE_NUMBER)?),
                (None, Some((text, at))) => {
                    let id = text.parse().ok().filter(|_| plain_digits(text));
                    SimpleResource::Id(id.ok_or_else(|| (at, format!("{ID}=\"{text}\": not a plain decimal u64")))?)
                }
                (Some(_), Some((_, at))) => return Err((at, format!("{ID} next to {RESOURCE_NUMBER}: the client ignores it"))),
                (None, None) => return Err((element.offset, format!("<{SIMPLE_MAP_ELEMENT}> without {RESOURCE_NUMBER} or {ID}"))),
            };
            Ok(AssetMapping::Simple(SimpleMap {
                resource_type: resource_type(element)?,
                file_format: file_format(element)?,
                resource,
                filename: string(element, FILENAME),
            }))
        }
        PATTERN_MAP_ELEMENT => {
            check_shape(element, &PATTERN_MAP_ATTRIBUTES, None)?;
            Ok(AssetMapping::Pattern(PatternMap {
                resource_type: resource_type(element)?,
                file_format: file_format(element)?,
                directory: string(element, DIRECTORY),
                prefix: string(element, PREFIX),
                suffix: string(element, SUFFIX),
                padding: unsigned(element, PADDING)?,
                pattern_match: signed(element, PATTERN_MATCH)? != 0,
            }))
        }
        BITMASK_MAP_ELEMENT => {
            check_shape(element, &BITMASK_MAP_ATTRIBUTES, Some(SECTION_ELEMENT))?;
            let sections = element
                .elements()
                .map(|section| {
                    check_shape(section, &SECTION_ATTRIBUTES, None)?;
                    Ok(BitmaskSection {
                        prefix: string(section, PREFIX),
                        padding: unsigned(section, PADDING)?,
                        mask: mask(section)?,
                        pattern_match: unsigned(section, PATTERN_MATCH)? != 0,
                        is_directory: unsigned(section, IS_DIRECTORY)? != 0,
                    })
                })
                .collect::<Result<_, OffsetError>>()?;
            Ok(AssetMapping::Bitmask(BitmaskMap {
                resource_type: resource_type(element)?,
                file_format: file_format(element)?,
                directory: string(element, DIRECTORY),
                suffix: string(element, SUFFIX),
                sections,
            }))
        }
        other => Err((element.offset, format!("Node {other} ignored - no valid mapping found. (the client's words)"))),
    }
}

/// `data/assetmap.xml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetMap {
    /// The document: whitespace and comments around one root element.
    items: Vec<Item>,
    mappings: Vec<AssetMapping>,
}

impl AssetMap {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        if let Some(at) = bytes.iter().position(|byte| !byte.is_ascii()) {
            return Err(locate(bytes, (at, "a non-ASCII byte: the shipped files are ASCII; not supported".to_owned())));
        }
        let mut parser = Parser { text: bytes, at: 0 };
        let items = parser.items(None).map_err(|error| locate(bytes, error))?;
        let mappings = Self::read(&items).map_err(|error| locate(bytes, error))?;
        Ok(Self { items, mappings })
    }

    fn read(items: &[Item]) -> Result<Vec<AssetMapping>, OffsetError> {
        let mut roots = items.iter().filter_map(|item| match item {
            Item::Element(element) => Some(element),
            _ => None,
        });
        let Some(root) = roots.next() else {
            return Err((0, "no root element".to_owned()));
        };
        if let Some(second) = roots.next() {
            return Err((second.offset, "a second root element".to_owned()));
        }
        if root.name != ROOT_ELEMENT {
            return Err((root.offset, format!("Assetmap root node isn't <{ROOT_ELEMENT}> (the client's words)")));
        }
        if let Some(attribute) = root.attributes.first() {
            return Err((attribute.offset, format!("`{}` on <{ROOT_ELEMENT}>: the client ignores it", attribute.name)));
        }
        root.elements().map(mapping).collect()
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        write_items(&self.items, &mut out);
        out.into_bytes()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes();
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    /// The children of `<assetmap>`, in order.
    pub fn mappings(&self) -> &[AssetMapping] {
        &self.mappings
    }

    /// Applies `edit` to a copy of the document and keeps it when it reads back.
    fn edit(&mut self, edit: impl FnOnce(&mut Vec<Item>) -> eyre::Result<()>) -> eyre::Result<()> {
        let mut items = self.items.clone();
        edit(&mut items)?;
        let mut out = String::new();
        write_items(&items, &mut out);
        *self = Self::from_bytes(out.as_bytes()).map_err(|error| eyre!("the edit does not read back: {error}"))?;
        Ok(())
    }

    /// Sets attribute `name` of mapping `index` (of its section `section` when given) to `value`, keeping its
    /// place and quote; a new attribute goes last, after a space.
    pub fn set_attribute(&mut self, index: usize, section: Option<usize>, name: &str, value: &str) -> eyre::Result<()> {
        self.edit(|items| {
            let element = target(items, index, section)?;
            match element.attributes.iter_mut().find(|attribute| attribute.name == name) {
                Some(attribute) => attribute.value = value.to_owned(),
                None => element.attributes.push(Attribute {
                    space_before: ATTRIBUTE_SPACE.to_owned(),
                    name: name.to_owned(),
                    quote: DOUBLE_QUOTE,
                    value: value.to_owned(),
                    offset: 0,
                }),
            }
            Ok(())
        })
    }

    /// Removes attribute `name` of mapping `index` (of its section `section` when given).
    pub fn remove_attribute(&mut self, index: usize, section: Option<usize>, name: &str) -> eyre::Result<()> {
        self.edit(|items| {
            let element = target(items, index, section)?;
            let before = element.attributes.len();
            element.attributes.retain(|attribute| attribute.name != name);
            if element.attributes.len() == before {
                eyre::bail!("no `{name}` attribute");
            }
            Ok(())
        })
    }

    /// Adds `mapping` after the last one, on a line indented as the last one is.
    pub fn push_mapping(&mut self, mapping: &AssetMapping) -> eyre::Result<()> {
        self.edit(|items| {
            let root = root_mut(items)?;
            let content = root.content.get_or_insert_with(Vec::new);
            let last = content.iter().rposition(|item| matches!(item, Item::Element(_)));
            let indent = last
                .and_then(|at| at.checked_sub(1))
                .and_then(|at| match &content[at] {
                    Item::Whitespace(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let element = render(mapping, &indent);
            let at = last.map_or(0, |at| at + 1);
            content.insert(at, Item::Element(element));
            content.insert(at, Item::Whitespace(indent));
            Ok(())
        })
    }

    /// Removes mapping `index` and the whitespace before it.
    pub fn remove_mapping(&mut self, index: usize) -> eyre::Result<()> {
        self.edit(|items| {
            let content = root_mut(items)?.content.as_mut().ok_or_else(|| eyre!("<{ROOT_ELEMENT}> is empty"))?;
            let at = element_position(content, index)?;
            content.remove(at);
            if at > 0 && matches!(content[at - 1], Item::Whitespace(_)) {
                content.remove(at - 1);
            }
            Ok(())
        })
    }
}

fn root_mut(items: &mut [Item]) -> eyre::Result<&mut Element> {
    items
        .iter_mut()
        .find_map(|item| match item {
            Item::Element(element) => Some(element),
            _ => None,
        })
        .ok_or_else(|| eyre!("no root element"))
}

/// Where the `index`th element of `items` is.
fn element_position(items: &[Item], index: usize) -> eyre::Result<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, Item::Element(_)))
        .nth(index)
        .map(|(at, _)| at)
        .ok_or_else(|| eyre!("no element {index}"))
}

/// Mapping `index`, or its section `section`.
fn target(items: &mut [Item], index: usize, section: Option<usize>) -> eyre::Result<&mut Element> {
    let content = root_mut(items)?.content.as_mut().ok_or_else(|| eyre!("<{ROOT_ELEMENT}> is empty"))?;
    let at = element_position(content, index).wrap_err_with(|| format!("mapping {index}"))?;
    let Item::Element(mapping) = &mut content[at] else { unreachable!("an element position") };
    let Some(section) = section else {
        return Ok(mapping);
    };
    let children = mapping.content.as_mut().ok_or_else(|| eyre!("mapping {index} has no sections"))?;
    let at = element_position(children, section).wrap_err_with(|| format!("mapping {index}, section {section}"))?;
    let Item::Element(element) = &mut children[at] else { unreachable!("an element position") };
    Ok(element)
}

/// `mapping` as an element, attributes in the shipped order, values the client reads as absent left out.
fn render(mapping: &AssetMapping, indent: &str) -> Element {
    let flag = |on: bool| on.then(|| "1".to_owned());
    let nonzero = |number: u32| (number != 0).then(|| number.to_string());
    let nonempty = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    let collect = |pairs: Vec<(&'static str, Option<String>)>| -> Vec<(&'static str, String)> {
        pairs.into_iter().filter_map(|(name, value)| value.map(|value| (name, value))).collect()
    };
    match mapping {
        AssetMapping::Simple(map) => {
            let resource = match map.resource {
                SimpleResource::Number(number) => (RESOURCE_NUMBER, number.to_string()),
                SimpleResource::Id(id) => (ID, id.to_string()),
            };
            let attributes = [
                (RESOURCE_TYPE, map.resource_type.to_string()),
                (FILE_FORMAT, map.file_format.name().to_owned()),
                resource,
                (FILENAME, map.filename.clone()),
            ];
            Element::new(SIMPLE_MAP_ELEMENT, &attributes, ATTRIBUTE_SPACE, None)
        }
        AssetMapping::Pattern(map) => {
            let attributes = collect(vec![
                (RESOURCE_TYPE, Some(map.resource_type.to_string())),
                (FILE_FORMAT, Some(map.file_format.name().to_owned())),
                (PADDING, nonzero(map.padding)),
                (DIRECTORY, Some(map.directory.clone())),
                (PREFIX, nonempty(&map.prefix)),
                (SUFFIX, Some(map.suffix.clone())),
                (PATTERN_MATCH, flag(map.pattern_match)),
            ]);
            Element::new(PATTERN_MAP_ELEMENT, &attributes, ATTRIBUTE_SPACE, None)
        }
        AssetMapping::Bitmask(map) => {
            let attributes = [
                (RESOURCE_TYPE, map.resource_type.to_string()),
                (FILE_FORMAT, map.file_format.name().to_owned()),
                (DIRECTORY, map.directory.clone()),
                (SUFFIX, map.suffix.clone()),
            ];
            let section_indent = format!("{indent}{SECTION_INDENT}");
            let mut content = Vec::new();
            for section in &map.sections {
                let attributes = collect(vec![
                    (IS_DIRECTORY, flag(section.is_directory)),
                    (PATTERN_MATCH, flag(section.pattern_match)),
                    (PREFIX, nonempty(&section.prefix)),
                    (MASK, Some(format!("{}{:x}", HEX_PREFIXES[0], section.mask))),
                    (PADDING, Some(section.padding.to_string())),
                ]);
                content.push(Item::Whitespace(section_indent.clone()));
                content.push(Item::Element(Element::new(SECTION_ELEMENT, &attributes, "", None)));
            }
            content.push(Item::Whitespace(indent.to_owned()));
            Element::new(BITMASK_MAP_ELEMENT, &attributes, "", Some(content))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<assetmap>\r\n   <patternmap resourcetype=\"1\" fileFormat=\"raw\" padding=\"8\" directory=\"Data/WorldArt/\" suffix=\"*\" patternMatch=\"1\" />\r\n   <patternmap resourcetype=\"2\" directory=\"Data/Maps/\" suffix=\".dat\"/>\r\n   <bitmaskmap resourcetype=\"9\" fileFormat=\"packed\" directory=\"Build/Sectors/\" suffix=\".bin\">\r\n      <section prefix=\"Facet_\" mask=\"0x3f\" padding=\"2\"/>\r\n      <section isDirectory=\"1\" mask=\"0xffffffc0\" padding=\"8\"/>\r\n   </bitmaskmap>\r\n   <simplemap resourcetype=\"20\" fileFormat=\"packed\" resourcenumber=\"0\" filename='Build/MultiCollection/housing.bin' />\r\n   <!-- Resource type 25 is only used by legacy -->\r\n </assetmap>\r\n";

    #[test]
    fn the_asset_map_reads_as_the_client_reads_it_and_writes_back() {
        let map = AssetMap::from_bytes(SAMPLE.as_bytes()).unwrap();
        assert_eq!(map.to_bytes(), SAMPLE.as_bytes());
        assert_eq!(map.mappings().len(), 4);
        assert_eq!(
            map.mappings()[1],
            AssetMapping::Pattern(PatternMap {
                resource_type: 2,
                file_format: FileFormat::Raw,
                directory: "Data/Maps/".to_owned(),
                prefix: String::new(),
                suffix: ".dat".to_owned(),
                padding: 0,
                pattern_match: false,
            })
        );
        let AssetMapping::Bitmask(bitmask) = &map.mappings()[2] else { panic!("a bitmaskmap") };
        assert_eq!(bitmask.sections[1], BitmaskSection { prefix: String::new(), padding: 8, mask: 0xffff_ffc0, pattern_match: false, is_directory: true });
        let AssetMapping::Simple(simple) = &map.mappings()[3] else { panic!("a simplemap") };
        assert_eq!((simple.resource, simple.filename.as_str()), (SimpleResource::Number(0), "Build/MultiCollection/housing.bin"));
    }

    #[test]
    fn edits_read_back() {
        let mut map = AssetMap::from_bytes(SAMPLE.as_bytes()).unwrap();
        map.set_attribute(1, None, PADDING, "6").unwrap();
        map.set_attribute(2, Some(0), MASK, "0x7f").unwrap();
        let AssetMapping::Pattern(pattern) = &map.mappings()[1] else { panic!("a patternmap") };
        assert_eq!(pattern.padding, 6);
        let added = AssetMapping::Bitmask(BitmaskMap {
            resource_type: 16,
            file_format: FileFormat::Packed,
            directory: "Build/AnimationFrame/".to_owned(),
            suffix: ".bin".to_owned(),
            sections: vec![
                BitmaskSection { prefix: String::new(), padding: 6, mask: 0xffff00, pattern_match: false, is_directory: true },
                BitmaskSection { prefix: String::new(), padding: 2, mask: 0xff, pattern_match: false, is_directory: false },
            ],
        });
        map.push_mapping(&added).unwrap();
        let text = String::from_utf8(map.to_bytes()).unwrap();
        assert!(text.contains("\r\n   <bitmaskmap resourcetype=\"16\" fileFormat=\"packed\" directory=\"Build/AnimationFrame/\" suffix=\".bin\">\r\n      <section isDirectory=\"1\" mask=\"0xffff00\" padding=\"6\"/>\r\n      <section mask=\"0xff\" padding=\"2\"/>\r\n   </bitmaskmap>\r\n   <!--"));
        let back = AssetMap::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(back.mappings()[4], added);
        map.remove_mapping(4).unwrap();
        map.remove_attribute(1, None, PADDING).unwrap();
        map.set_attribute(2, Some(0), MASK, "0x3f").unwrap();
        assert_eq!(map.to_bytes(), SAMPLE.as_bytes());
        assert!(map.set_attribute(0, None, "paddign", "1").is_err());
        assert!(map.set_attribute(0, None, RESOURCE_TYPE, "256").is_err());
        assert_eq!(map.to_bytes(), SAMPLE.as_bytes(), "a refused edit changes nothing");
    }

    #[test]
    fn what_the_client_reads_otherwise_is_an_error() {
        let wrap = |inner: &str| format!("<assetmap>\n{inner}\n</assetmap>\n");
        for (inner, needle) in [
            ("<patternmap resourcetype=\"1\" paddng=\"8\"/>", "not a <patternmap> attribute"),
            ("<patternmap padding=\"8\"/>", "without resourcetype"),
            ("<patternmap resourcetype=\"300\"/>", "low byte"),
            ("<patternmap resourcetype=\"1\" padding=\"8x\"/>", "absent"),
            ("<patternmap resourcetype=\"1\" padding=\"4294967296\"/>", "saturate"),
            ("<patternmap resourcetype=\"1\" fileFormat=\"Packed\"/>", "as raw"),
            ("<patternmap resourcetype=\"1\" suffix=\"a\" suffix=\"b\"/>", "repeated"),
            ("<patternmap resourcetype=\"1\" suffix=\"&amp;\"/>", "entity"),
            ("<patternmap resourcetype = \"1\"/>", "around `=`"),
            ("<simplemap resourcetype=\"1\" filename=\"a\"/>", "without resourcenumber or id"),
            ("<simplemap resourcetype=\"1\" resourcenumber=\"1\" id=\"2\"/>", "ignores it"),
            ("<bitmaskmap resourcetype=\"1\"><sektion/></bitmaskmap>", "only <section>"),
            ("<bitmaskmap resourcetype=\"1\"><section mask=\"3f\"/></bitmaskmap>", "hex digits"),
            ("<simplemaps resourcetype=\"1\"/>", "no valid mapping found"),
            ("text", "text outside a tag"),
        ] {
            let error = AssetMap::from_bytes(wrap(inner).as_bytes()).unwrap_err();
            assert!(error.message.contains(needle), "{inner}: {}", error.message);
            assert_eq!(error.line, 2, "{inner}");
        }
        assert!(AssetMap::from_bytes(b"<map/>").unwrap_err().message.contains("isn't <assetmap>"));
        assert!(AssetMap::from_bytes(b"<assetmap><patternmap resourcetype=\"1\">").unwrap_err().message.contains("not closed"));
        assert!(AssetMap::from_bytes(b"<assetmap><patternmap resourcetype=\"1\"></assetmap>").unwrap_err().message.contains("expected `</patternmap>`"));
        assert!(AssetMap::from_bytes(b"<?xml version=\"1.0\"?><assetmap/>").is_err());
        assert!(AssetMap::from_bytes("<assetmap>\u{e9}</assetmap>".as_bytes()).is_err());
    }
}
