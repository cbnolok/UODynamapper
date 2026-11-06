//! Syntax checks of replaced text entries: the Interface Lua scripts in the client's Lua (`full_moon`) and the XML
//! documents as the client's TinyXML reads them (`roxmltree`). Errors name the line and column ([`SyntaxError`]).
//! The text is read as [`EntryText`] reads it (UTF-8 with or without a BOM, UTF-16 with a BOM).
//!
//! **Lua.** The EC and KR clients run Lua 5.1 with the `continue` statement and the bitwise operators `&`, `|`,
//! `^^` (XOR), `<<` and `>>` (the lexer's token table: `continue`, `<<`, `>>`, `^^`; the `BAND`, `BOR`, `BXOR`,
//! `BSHL`, `BSHR` opcodes), and the shipped scripts use `continue` and `&`. `full_moon` has no such dialect, so the
//! script is read as Lua 5.3 after a pass that keeps every byte offset: `continue` becomes a call statement of the
//! same length, `^^` becomes `~`, and the 5.2 and 5.3 syntax the client has not (`goto`, labels, `//`, `~`) is then
//! refused by token. A backtick outside a string becomes `$`, another character Lua does not take: `full_moon` 3.0
//! panics on a backtick unless built for Luau or CfxLua. Loosenings, all syntax the client refuses at run time
//! rather than a shipped need: the 5.2 string escapes (`\z`, `\x`) and hexadecimal floats are taken, as is
//! `continue` as a field name.
//!
//! **XML.** TinyXML takes documents that are not well-formed, and the shipped ones rely on it. Before `roxmltree`
//! reads a document, a pass does what TinyXML does, keeping every line and column but one kind (the inserted
//! spaces, whose columns are taken off the errors):
//! - attributes without whitespace between them (`a="1"b="2"`, EC `customizehousewindow.xml`, `managedtextures.xml`,
//!   `fontscht.xml`): a space is inserted;
//! - a `&` that starts no reference TinyXML knows (the five predefined entities and character references; KR
//!   `icons.xml`: `Whistle & Yell`) and a `<` in an attribute value are characters;
//! - a comment ends at the first `-->`, whatever it holds (KR `bulkorderfilter.xml` nests `<!--`);
//! - an XML declaration after the start (KR `choicetext.xml`, after a comment) is a node of its own.
//!
//! Each of these is read without TinyXML's error flag in the client's TinyXML 2.5.3 (UOSA.exe:
//! `TiXmlElement::Parse` parses the next attribute right after a value, `GetEntity` takes an unknown `&` as a
//! character, `TiXmlAttribute::Parse` reads a quoted value up to its quote, `TiXmlComment::Parse` stops at the first
//! `-->`, `Identify` makes `<?xml` a declaration anywhere). An element still open at the end of the document is an
//! error, with the client's wording: TinyXML sets error 9 ("Error reading end tag"), and the client's Interface
//! loader (`UOXml::LoadInterfaceDocument`) then logs "unable to load '%s' due to error at line %d, column %d" and
//! drops the whole document, so such a replacement would do nothing in game. The EC and KR `startmenu.xml` lack
//! their last `</Interface>` and are dropped so; no Interface document, mod or script names them
//! (`ghidra-classic-defs`).

use full_moon::node::Node;
use full_moon::tokenizer::{Symbol, TokenType};

use crate::enhanced::entry_text::{EntryText, TextEncoding};

crate::eyre_imports!();

/// A syntax error at a 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for SyntaxError {}

fn text(bytes: &[u8]) -> eyre::Result<String> {
    match EntryText::decode(bytes) {
        Some(text) if text.encoding != TextEncoding::Windows1252 => Ok(text.text),
        _ => eyre::bail!("not UTF-8 or UTF-16 text"),
    }
}

/// The client's statement, and what it is read as: a statement of the same length.
const CONTINUE: &str = "continue";
const CONTINUE_STAND_IN: &str = "_cont() ";
/// The client's XOR, and the Lua 5.3 operator it is read as, of the same length.
const CLIENT_XOR: &str = "^^";
const XOR_STAND_IN: &str = "~ ";
/// A character `full_moon` 3.0 panics on outside a string or comment unless built for Luau or CfxLua (its lexer
/// sends it to the string reader, which takes it as a quote only with those features), and what it is read as: a
/// character that is no Lua token either.
const BACKTICK: u8 = b'`';
const BACKTICK_STAND_IN: u8 = b'$';
const LONG_BRACKET_OPEN: u8 = b'[';
const LONG_BRACKET_CLOSE: u8 = b']';
const LONG_BRACKET_LEVEL: u8 = b'=';
const COMMENT: &[u8] = b"--";

/// The level of the long bracket (`[==[`) at `at`, if one opens there.
fn long_bracket(bytes: &[u8], at: usize, bracket: u8) -> Option<usize> {
    if bytes.get(at) != Some(&bracket) {
        return None;
    }
    let level = bytes[at + 1..].iter().take_while(|&&byte| byte == LONG_BRACKET_LEVEL).count();
    (bytes.get(at + 1 + level) == Some(&bracket)).then_some(level)
}

/// Where the long bracket of `level` opened before `from` closes (past its `]==]`), or the end.
fn long_bracket_end(bytes: &[u8], from: usize, level: usize) -> usize {
    (from..bytes.len()).find(|&at| long_bracket(bytes, at, LONG_BRACKET_CLOSE) == Some(level)).map_or(bytes.len(), |at| at + level + 2)
}

/// The client's Lua made Lua 5.3 for `full_moon`, every byte where it was.
fn lua_for_full_moon(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if bytes[at..].starts_with(COMMENT) {
            at += COMMENT.len();
            at = match long_bracket(bytes, at, LONG_BRACKET_OPEN) {
                Some(level) => long_bracket_end(bytes, at + level + 2, level),
                None => bytes[at..].iter().position(|&byte| byte == b'\n').map_or(bytes.len(), |end| at + end),
            };
        } else if let Some(level) = long_bracket(bytes, at, LONG_BRACKET_OPEN) {
            at = long_bracket_end(bytes, at + level + 2, level);
        } else if byte == b'"' || byte == b'\'' {
            at += 1;
            while at < bytes.len() && bytes[at] != byte && bytes[at] != b'\n' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            at += 1;
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            let end = at + bytes[at..].iter().take_while(|&&byte| byte.is_ascii_alphanumeric() || byte == b'_').count();
            if !byte.is_ascii_digit() && &source[at..end] == CONTINUE {
                out[at..end].copy_from_slice(CONTINUE_STAND_IN.as_bytes());
            }
            at = end;
        } else if bytes[at..].starts_with(CLIENT_XOR.as_bytes()) {
            out[at..at + CLIENT_XOR.len()].copy_from_slice(XOR_STAND_IN.as_bytes());
            at += CLIENT_XOR.len();
        } else {
            if byte == BACKTICK {
                out[at] = BACKTICK_STAND_IN;
            }
            at += 1;
        }
    }
    String::from_utf8(out).expect("only ASCII bytes were replaced")
}

/// The syntax errors of the client Lua script `source`, as `full_moon` finds them.
pub fn lua_errors(source: &str) -> Vec<SyntaxError> {
    let lexed = lua_for_full_moon(source);
    let result = full_moon::parse_fallible(&lexed, full_moon::LuaVersion::lua53());
    let at = |position: full_moon::tokenizer::Position, message: String| SyntaxError { line: position.line(), column: position.character(), message };
    let mut errors: Vec<SyntaxError> = result
        .errors()
        .iter()
        .map(|error| {
            let (start, _) = error.range();
            let mut message = error.error_message().into_owned();
            if source.as_bytes().get(start.bytes()) == Some(&BACKTICK) {
                message = message.replace(char::from(BACKTICK_STAND_IN), &char::from(BACKTICK).to_string());
            }
            at(start, message)
        })
        .collect();
    if errors.is_empty() {
        for token in result.ast().nodes().tokens() {
            let start = token.token().start_position();
            let refused = match token.token_type() {
                TokenType::Symbol { symbol: Symbol::Goto } => "`goto` is Lua 5.2; the client's Lua has no goto",
                TokenType::Symbol { symbol: Symbol::TwoColons } => "labels (`::`) are Lua 5.2; the client's Lua has none",
                TokenType::Symbol { symbol: Symbol::DoubleSlash } => "`//` is Lua 5.3; the client's Lua has no integer division",
                TokenType::Symbol { symbol: Symbol::Tilde } if !source[start.bytes()..].starts_with(CLIENT_XOR) => {
                    "`~` is Lua 5.3; the client's XOR is `^^`, and it has no bitwise not"
                }
                _ => continue,
            };
            errors.push(at(start, refused.to_owned()));
        }
    }
    errors
}

const PREDEFINED_ENTITIES: [&str; 5] = ["amp;", "lt;", "gt;", "quot;", "apos;"];
const CHARACTER_REFERENCE: char = '#';
const HEX_REFERENCE: char = 'x';
const REFERENCE_END: char = ';';
/// What a `&` or `<` TinyXML takes as a character is read as.
const CHARACTER_STAND_IN: char = '_';
const COMMENT_OPEN: &str = "<!--";
const COMMENT_CLOSE: &str = "-->";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";
const DECLARATION_OPEN: &str = "<?xml";
const PROCESSING_CLOSE: &str = "?>";

/// Whether the `&` at the start of `rest` starts a reference TinyXML decodes.
fn starts_reference(rest: &str) -> bool {
    let after = &rest[1..];
    if PREDEFINED_ENTITIES.iter().any(|entity| after.starts_with(entity)) {
        return true;
    }
    let Some(number) = after.strip_prefix(CHARACTER_REFERENCE) else { return false };
    let (digits, hex) = match number.strip_prefix(HEX_REFERENCE) {
        Some(digits) => (digits, true),
        None => (number, false),
    };
    let count = digits.chars().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
    count > 0 && digits[count..].starts_with(REFERENCE_END)
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_numeric() || c == '-' || c == '.'
}

/// An XML document made well-formed as TinyXML reads it, with where spaces were inserted (line and column of the
/// output, 1-based).
struct TinyXmlPass {
    out: String,
    inserted: Vec<(usize, usize)>,
    /// The end of the document and the innermost element still open there, if one is.
    unclosed: Option<(usize, usize, String)>,
    line: usize,
    column: usize,
}

impl TinyXmlPass {
    fn push(&mut self, c: char) {
        self.out.push(c);
        if c == '\n' {
            (self.line, self.column) = (self.line + 1, 1);
        } else {
            self.column += 1;
        }
    }

    fn push_str(&mut self, text: &str) {
        text.chars().for_each(|c| self.push(c));
    }

    fn insert_space(&mut self) {
        self.inserted.push((self.line, self.column));
        self.push(' ');
    }

    /// Copies an attribute value or text, the `&` TinyXML takes as characters made characters.
    fn push_text(&mut self, text: &str, in_attribute: bool) {
        for (at, c) in text.char_indices() {
            let character = (c == '&' && !starts_reference(&text[at..])) || (in_attribute && c == '<');
            self.push(if character { CHARACTER_STAND_IN } else { c });
        }
    }

    /// Copies the attributes of a tag from the start of `rest` up to its `>` (or `?>`), inserting the missing
    /// spaces between them; returns the bytes read. Anything else in the tag is copied for `roxmltree` to judge.
    fn push_attributes(&mut self, rest: &str) -> usize {
        let mut at = 0;
        loop {
            let spaces = rest[at..].chars().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum::<usize>();
            self.push_str(&rest[at..at + spaces]);
            at += spaces;
            let name = rest[at..].chars().take_while(|&c| is_name_char(c)).map(char::len_utf8).sum::<usize>();
            if name == 0 || !rest[at..].starts_with(is_name_start) {
                return at;
            }
            self.push_str(&rest[at..at + name]);
            at += name;
            let before_equals = rest[at..].chars().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum::<usize>();
            if !rest[at + before_equals..].starts_with('=') {
                return at;
            }
            self.push_str(&rest[at..at + before_equals + 1]);
            at += before_equals + 1;
            let before_value = rest[at..].chars().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum::<usize>();
            self.push_str(&rest[at..at + before_value]);
            at += before_value;
            let Some(quote) = rest[at..].chars().next().filter(|&c| c == '"' || c == '\'') else { return at };
            let Some(length) = rest[at + 1..].find(quote) else { return at };
            self.push(quote);
            self.push_text(&rest[at + 1..at + 1 + length], true);
            self.push(quote);
            at += length + 2;
            if rest[at..].starts_with(is_name_start) {
                self.insert_space();
            }
        }
    }

    fn run(source: &str) -> Self {
        let mut pass =
            Self { out: String::with_capacity(source.len()), inserted: Vec::new(), unclosed: None, line: 1, column: 1 };
        let mut open: Vec<&str> = Vec::new();
        let mut at = 0;
        while at < source.len() {
            let rest = &source[at..];
            let Some(tag) = rest.find('<') else {
                pass.push_text(rest, false);
                break;
            };
            pass.push_text(&rest[..tag], false);
            at += tag;
            let rest = &source[at..];
            // A construct without its end is copied for `roxmltree` to refuse.
            let copy_to = |close: &str, from: usize| rest[from..].find(close).map_or(rest.len(), |end| from + end + close.len());
            if rest.starts_with(COMMENT_OPEN) {
                let end = copy_to(COMMENT_CLOSE, COMMENT_OPEN.len());
                let body_end = end.saturating_sub(COMMENT_CLOSE.len()).max(COMMENT_OPEN.len());
                pass.push_str(COMMENT_OPEN);
                // Its text is no syntax: blanked, so `--` and `-` before the end are no error.
                rest[COMMENT_OPEN.len()..body_end].chars().for_each(|c| pass.push(if c == '\n' || c == '\r' { c } else { ' ' }));
                pass.push_str(&rest[body_end..end]);
                at += end;
            } else if rest.starts_with(CDATA_OPEN) {
                let end = copy_to(CDATA_CLOSE, CDATA_OPEN.len());
                pass.push_str(&rest[..end]);
                at += end;
            } else if rest.starts_with(DECLARATION_OPEN) && rest[DECLARATION_OPEN.len()..].starts_with(char::is_whitespace) {
                let end = copy_to(PROCESSING_CLOSE, DECLARATION_OPEN.len());
                if at == 0 {
                    pass.push_str(DECLARATION_OPEN);
                    let read = pass.push_attributes(&rest[DECLARATION_OPEN.len()..end]);
                    pass.push_str(&rest[DECLARATION_OPEN.len() + read..end]);
                } else {
                    rest[..end].chars().for_each(|c| pass.push(if c == '\n' || c == '\r' { c } else { ' ' }));
                }
                at += end;
            } else if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = copy_to(">", 1);
                pass.push_str(&rest[..end]);
                at += end;
            } else if let Some(name) = rest.strip_prefix("</") {
                let length = name.chars().take_while(|&c| is_name_char(c)).map(char::len_utf8).sum::<usize>();
                if open.last() == Some(&&name[..length]) {
                    open.pop();
                }
                let end = copy_to(">", 2);
                pass.push_str(&rest[..end]);
                at += end;
            } else {
                let length = rest[1..].chars().take_while(|&c| is_name_char(c)).map(char::len_utf8).sum::<usize>();
                pass.push('<');
                pass.push_str(&rest[1..1 + length]);
                let read = 1 + length + pass.push_attributes(&rest[1 + length..]);
                let end = copy_to(">", read).max(read);
                pass.push_str(&rest[read..end]);
                if length > 0 && !rest[..end].ends_with("/>") {
                    open.push(&rest[1..1 + length]);
                }
                at += end;
            }
        }
        if let Some(innermost) = open.last() {
            pass.unclosed = Some((pass.line, pass.column, innermost.to_string()));
        }
        // Closed, so that `roxmltree` judges the rest of the document; an earlier error is reported first.
        for name in open.iter().rev() {
            pass.out.push_str(&format!("</{name}>"));
        }
        pass
    }

    /// `column` of the output on `line` as a column of the source: the inserted spaces before it taken off.
    fn source_column(&self, line: usize, column: usize) -> usize {
        column - self.inserted.iter().filter(|&&(at_line, at_column)| at_line == line && at_column < column).count()
    }
}

/// The first syntax error of the XML document `source`, read as the client's TinyXML reads it.
pub fn xml_error(source: &str) -> Option<SyntaxError> {
    let pass = TinyXmlPass::run(source);
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..roxmltree::ParsingOptions::default() };
    let error = roxmltree::Document::parse_with_options(&pass.out, options).err().map(|error| {
        let (line, column) = (error.pos().row as usize, error.pos().col as usize);
        let source_column = pass.source_column(line, column);
        // The message names the position in the passed text.
        let message = error.to_string().replace(&format!("{line}:{column}"), &format!("{line}:{source_column}"));
        SyntaxError { line, column: source_column, message }
    });
    error.or_else(|| {
        pass.unclosed.as_ref().map(|(line, column, element)| {
            let column = pass.source_column(*line, *column);
            SyntaxError {
                line: *line,
                column,
                message: format!(
                    "the client drops this document: unable to load due to error at line {line}, column {column} \
                     (`<{element}>` is not closed)"
                ),
            }
        })
    })
}

/// Whether `bytes` are a script of the client's Lua.
pub fn check_lua(bytes: &[u8]) -> eyre::Result<()> {
    match lua_errors(&text(bytes)?).into_iter().next() {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

/// Whether `bytes` are an XML document the client's TinyXML reads.
pub fn check_xml(bytes: &[u8]) -> eyre::Result<()> {
    match xml_error(&text(bytes)?) {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_error(source: &str) -> Option<String> {
        lua_errors(source).first().map(ToString::to_string)
    }

    #[test]
    fn reads_the_clients_lua() {
        let script = "for i = 1, 3 do\n\tif i & 1 == 0 then\n\t\tcontinue\n\tend\n\tx = (i << 2) | (i >> 1) ^^ 3\nend\ns = \"`\" -- `\n";
        assert_eq!(lua_error(script), None);
        assert_eq!(lua_error("x = 7 // 2\n").as_deref(), Some("line 1, column 7: `//` is Lua 5.3; the client's Lua has no integer division"));
        assert!(lua_error("x = ~1\n").unwrap().contains("bitwise not"));
        assert!(lua_error("goto done\n::done::\n").unwrap().contains("no goto"));
        let errors = lua_errors("x = `a`\n");
        assert!(errors.iter().any(|error| error.column == 5 && error.message.starts_with("unexpected character `")), "{errors:?}");
        let error = lua_errors("if x then\n  y = \nend\n").remove(0);
        assert_eq!(error.line, 2, "{error}");
    }

    #[test]
    fn reads_xml_as_tinyxml_does() {
        let shipped = "<!-- a -- b --><?xml version=\"1.0\"encoding=\"UTF-8\"?>\n<Root a=\"1\"b=\"x & y < z\">\n\t<!--<!-- nested -->\n\t<Item/>\n</Root>\n";
        assert_eq!(xml_error(shipped), None);
        let error = xml_error("<Root a=\"1\"b=\"2\">\n\t<Item/>\n").unwrap();
        assert_eq!((error.line, error.column), (3, 1), "{error}");
        assert!(error.message.starts_with("the client drops this document: unable to load due to error at line 3, column 1"), "{error}");
        assert!(error.message.contains("`<Root>`"), "{error}");
        let error = xml_error("<Root a=\"1\"b=\"2\">\n</Wrong>\n").unwrap();
        assert_eq!((error.line, error.column), (2, 1), "{error}");
        let error = xml_error("<a x=\"1\"y=\"2\" x=\"3\"/>").unwrap();
        assert!(error.message.contains("already defined"), "{error}");
        assert_eq!(error.column, 15, "the inserted space is not counted: {error}");
        assert!(check_xml(b"<a>&#x41;&amp;</a>").is_ok());
    }
}
