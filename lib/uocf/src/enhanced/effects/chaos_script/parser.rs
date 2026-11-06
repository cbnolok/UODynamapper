//! Lexer and parser of Chaos scripts. Tokens carry byte offsets into the text; whatever lies between the tokens
//! of two elements (whitespace and comments) is kept as their layout, so the source text can be rebuilt.

use color_eyre::eyre;

use super::validation::{ChaosIssue, ChaosIssueKind};
use super::writer;
use super::{
    ChaosBlendMode, ChaosCelSetup, ChaosItem, ChaosKey, ChaosParticleType, ChaosRaw, ChaosScalar, ChaosScript,
    ChaosSequence, ChaosSource, ChaosSourceLocation, ChaosStatement, ChaosTiming, ChaosValue, SourceText,
    BOOLEAN_FALSE_NAMES, BOOLEAN_TRUE_NAMES, CHAOS_CEL_SETUP_ARGUMENTS, CHAOS_COLOR_COMPONENTS,
    CHAOS_VECTOR_COMPONENTS, COMMENT_PREFIX, DEFAULT_NEWLINE, FUNCTION_CEL_SETUP, FUNCTION_RANDOM, FUNCTION_RGBA,
    FUNCTION_XYZ, KEYWORD_AT, KEYWORD_EVENT_SEQUENCE, KEYWORD_FADE, KEYWORD_FINAL, KEYWORD_INITIAL,
    KEYWORD_PARTICLE_SYSTEM, KEYWORD_SO,
};

/// Deepest nesting of calls inside a value. Ours, not the client's: client values nest 2 deep
/// (`XYZ(random(...), ...)`); the limit only rejects absurd input before it exhausts the stack.
const MAX_VALUE_NESTING: usize = 8;
/// Arguments of `random(min, max)`.
const RANDOM_ARGUMENTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Ident,
    Number,
    /// A quoted string; the token includes the quotes.
    Str,
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Comma,
    Equals,
    /// Any other character.
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

/// Byte offsets of the line starts, for 1-based line and column numbers.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(index, _)| index + 1)).collect();
        Self { starts }
    }

    fn locate(&self, text: &str, offset: usize) -> ChaosSourceLocation {
        let line = self.starts.partition_point(|&start| start <= offset);
        let line_start = self.starts[line - 1];
        ChaosSourceLocation { line, column: text[line_start..offset].chars().count() + 1 }
    }
}

pub(super) fn parse_script(text: &str) -> eyre::Result<ChaosScript> {
    let lines = LineIndex::new(text);
    let tokens = lex(text, &lines)?;
    let mut parser = Parser { text, lines, tokens, position: 0, cursor: 0 };
    parser.script()
}

fn lex(text: &str, lines: &LineIndex) -> eyre::Result<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let byte = bytes[position];
        if byte.is_ascii_whitespace() {
            position += 1;
            continue;
        }
        if text[position..].starts_with(COMMENT_PREFIX) {
            position = text[position..].find('\n').map_or(text.len(), |newline| position + newline);
            continue;
        }
        let start = position;
        let kind = match byte {
            b'{' => TokenKind::LeftBrace,
            b'}' => TokenKind::RightBrace,
            b'(' => TokenKind::LeftParen,
            b')' => TokenKind::RightParen,
            b',' => TokenKind::Comma,
            b'=' => TokenKind::Equals,
            b'"' => {
                let length = text[start + 1..].find(['"', '\n']).filter(|&length| bytes[start + 1 + length] == b'"');
                let Some(length) = length else {
                    eyre::bail!("{}: unterminated string", lines.locate(text, start));
                };
                position = start + 1 + length;
                TokenKind::Str
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                position = scan(bytes, start, |byte| byte.is_ascii_alphanumeric() || byte == b'_') - 1;
                TokenKind::Ident
            }
            b'0'..=b'9' | b'.' | b'-' | b'+' => {
                let digits_start = if matches!(byte, b'-' | b'+') { start + 1 } else { start };
                let end = scan(bytes, digits_start, |byte| byte.is_ascii_digit() || byte == b'.');
                if bytes[digits_start..end].iter().any(u8::is_ascii_digit) {
                    position = end - 1;
                    TokenKind::Number
                } else {
                    TokenKind::Other
                }
            }
            _ => {
                position += text[start..].chars().next().map_or(1, char::len_utf8) - 1;
                TokenKind::Other
            }
        };
        position += 1;
        tokens.push(Token { kind, start, end: position });
    }
    Ok(tokens)
}

/// The end of the run of bytes from `start` that satisfy `accept`.
fn scan(bytes: &[u8], start: usize, accept: impl Fn(u8) -> bool) -> usize {
    bytes[start..].iter().position(|&byte| !accept(byte)).map_or(bytes.len(), |length| start + length)
}

/// A number as C `atof` reads it: the longest prefix `[+-]digits[.digits]`. The flag is false when the token
/// has more (a second decimal point).
fn number_value(token: &str) -> (f64, bool) {
    let integer_end = token.find('.').map_or(token.len(), |point| point + 1);
    let prefix_end = token[integer_end..].find('.').map_or(token.len(), |point| integer_end + point);
    let value = token[..prefix_end].parse().unwrap_or(0.0);
    (value, prefix_end == token.len())
}

struct Parser<'a> {
    text: &'a str,
    lines: LineIndex,
    tokens: Vec<Token>,
    position: usize,
    /// End of the text already assigned to parsed elements.
    cursor: usize,
}

/// A part of a statement's left-hand side.
#[derive(Debug, Clone, Copy)]
enum LhsPart<'a> {
    Word(&'a str),
    Scalar(ChaosScalar),
    Other,
}

impl<'a> Parser<'a> {
    fn script(&mut self) -> eyre::Result<ChaosScript> {
        let Some(first) = self.peek() else {
            eyre::bail!("the script is empty: expected `{KEYWORD_PARTICLE_SYSTEM}`");
        };
        if !self.is_keyword(first, KEYWORD_PARTICLE_SYSTEM) {
            return Err(self.error_at(first.start, format!("the first word must be `{KEYWORD_PARTICLE_SYSTEM}`")));
        }
        self.position += 1;
        let name = self.expect(TokenKind::Str, first.end, "particlesystem: expected the quoted system name")?;
        let version_token = self.expect(TokenKind::Number, name.end, "particlesystem: expected the version number")?;
        let (version, well_formed) = number_value(self.slice(version_token));
        let mut issues = Vec::new();
        if !well_formed {
            issues.push(self.malformed_number(version_token, version));
        }
        let open = self.expect(TokenKind::LeftBrace, version_token.end, "particlesystem: expected `{`")?;

        let mut script = ChaosScript::new(self.string_value(name), version);
        let canonical = writer::script_header(&script);
        self.cursor = open.end;
        let (items, closing, close) = self.block_items(false, open)?;
        script.items = items;
        if let Some(extra) = self.peek() {
            return Err(self.error_at(extra.start, "unexpected text after the particlesystem block"));
        }
        let newline = if self.text.contains(DEFAULT_NEWLINE) || !self.text.contains('\n') { DEFAULT_NEWLINE } else { "\n" };
        script.source = ChaosSource(Some(Box::new(SourceText {
            leading: self.text[..first.start].to_owned(),
            text: self.text[first.start..version_token.end].to_owned(),
            canonical,
            before_brace: self.text[version_token.end..open.start].to_owned(),
            closing,
            trailing: self.text[close.end..].to_owned(),
            newline: newline.to_owned(),
            location: self.lines.locate(self.text, first.start),
            issues,
        })));
        Ok(script)
    }

    /// The items of a block whose `{` was consumed, up to and including its `}`; returns the items, the text
    /// before `}` and the `}` token.
    fn block_items(&mut self, in_sequence: bool, open: Token) -> eyre::Result<(Vec<ChaosItem>, String, Token)> {
        let mut items = Vec::new();
        loop {
            let Some(token) = self.peek() else {
                let block = if in_sequence { KEYWORD_EVENT_SEQUENCE } else { KEYWORD_PARTICLE_SYSTEM };
                return Err(self.error_at(open.start, format!("this {block} block has no closing `}}`")));
            };
            if token.kind == TokenKind::RightBrace {
                let closing = self.text[self.cursor..token.start].to_owned();
                self.position += 1;
                self.cursor = token.end;
                return Ok((items, closing, token));
            }
            if self.is_keyword(token, KEYWORD_EVENT_SEQUENCE) {
                if in_sequence {
                    return Err(self.error_at(token.start, "an eventsequence cannot be inside another one"));
                }
                items.push(ChaosItem::Sequence(self.sequence(token)?));
            } else {
                items.push(self.statement_or_raw(token));
            }
        }
    }

    fn sequence(&mut self, keyword: Token) -> eyre::Result<ChaosSequence> {
        let leading = self.text[self.cursor..keyword.start].to_owned();
        self.position += 1;
        let name = self.expect(TokenKind::Str, keyword.end, "eventsequence: expected the quoted sequence name")?;
        let open = self.expect(TokenKind::LeftBrace, name.end, "eventsequence: expected `{`")?;
        self.cursor = open.end;
        let (items, closing, close) = self.block_items(true, open)?;
        let mut sequence = ChaosSequence::new(self.string_value(name));
        sequence.items = items;
        let trailing = self.take_trailing(close.end);
        sequence.source = ChaosSource(Some(Box::new(SourceText {
            leading,
            text: self.text[keyword.start..name.end].to_owned(),
            canonical: writer::sequence_header(&sequence),
            before_brace: self.text[name.end..open.start].to_owned(),
            closing,
            trailing,
            location: self.lines.locate(self.text, keyword.start),
            ..SourceText::default()
        })));
        Ok(sequence)
    }

    fn statement_or_raw(&mut self, first: Token) -> ChaosItem {
        let leading = self.text[self.cursor..first.start].to_owned();
        let start_position = self.position;
        let mut issues = Vec::new();
        let item = match self.statement(&mut issues) {
            Some(statement) => ChaosItem::Statement(statement),
            None => {
                self.position = start_position;
                issues.clear();
                ChaosItem::Raw(self.raw_line(first))
            }
        };
        let end = self.tokens[self.position - 1].end;
        let trailing = self.take_trailing(end);
        let source = SourceText {
            leading,
            text: self.text[first.start..end].to_owned(),
            canonical: writer::item_text(&item),
            trailing,
            location: self.lines.locate(self.text, first.start),
            issues,
            ..SourceText::default()
        };
        let mut item = item;
        match &mut item {
            ChaosItem::Statement(statement) => statement.source = ChaosSource(Some(Box::new(source))),
            ChaosItem::Raw(raw) => raw.source = ChaosSource(Some(Box::new(source))),
            ChaosItem::Sequence(_) => unreachable!("statement_or_raw makes no sequences"),
        }
        item
    }

    /// `lhs = value`, with the left-hand side, `=` and the start of the value on one line; `None` (position
    /// unspecified) when the tokens are no statement.
    fn statement(&mut self, issues: &mut Vec<ChaosIssue>) -> Option<ChaosStatement> {
        let lhs_start = self.position;
        loop {
            let token = self.peek()?;
            if matches!(token.kind, TokenKind::LeftBrace | TokenKind::RightBrace) {
                return None;
            }
            if self.position > lhs_start && self.starts_line(self.position) {
                return None;
            }
            self.position += 1;
            if token.kind == TokenKind::Equals {
                break;
            }
        }
        let lhs_end = self.position - 1;
        if lhs_end == lhs_start || self.peek().is_none() || self.starts_line(self.position) {
            return None;
        }
        let value = self.value(0, issues)?;
        let (timing, key) = self.lhs(lhs_start, lhs_end, issues);
        Some(ChaosStatement::new(timing, key, value))
    }

    /// Interprets the left-hand side tokens `start..end`.
    fn lhs(&self, start: usize, end: usize, issues: &mut Vec<ChaosIssue>) -> (Option<ChaosTiming>, ChaosKey) {
        let mut parts = Vec::new();
        let mut scalar_issues = Vec::new();
        let mut index = start;
        while index < end {
            let token = self.tokens[index];
            if let Some((scalar, next)) = self.lhs_scalar(index, end, &mut scalar_issues) {
                parts.push(LhsPart::Scalar(scalar));
                index = next;
                continue;
            }
            parts.push(match token.kind {
                TokenKind::Ident => LhsPart::Word(self.slice(token)),
                _ => LhsPart::Other,
            });
            index += 1;
        }

        let keyword = |part: &LhsPart<'_>, expected: &str| matches!(part, LhsPart::Word(word) if word.eq_ignore_ascii_case(expected));
        let timing = match parts.as_slice() {
            [LhsPart::Word(_)] => Some((None, parts.len() - 1)),
            [initial, LhsPart::Word(_)] if keyword(initial, KEYWORD_INITIAL) => Some((Some(ChaosTiming::Initial), 1)),
            [final_, LhsPart::Word(_)] if keyword(final_, KEYWORD_FINAL) => {
                Some((Some(ChaosTiming::Final { fade: false }), 1))
            }
            [at, LhsPart::Scalar(time), LhsPart::Word(_)] if keyword(at, KEYWORD_AT) => {
                Some((Some(ChaosTiming::At { fade: false, time: *time }), 2))
            }
            [fade, so, final_, LhsPart::Word(_)]
                if keyword(fade, KEYWORD_FADE) && keyword(so, KEYWORD_SO) && keyword(final_, KEYWORD_FINAL) =>
            {
                Some((Some(ChaosTiming::Final { fade: true }), 3))
            }
            [fade, so, at, LhsPart::Scalar(time), LhsPart::Word(_)]
                if keyword(fade, KEYWORD_FADE) && keyword(so, KEYWORD_SO) && keyword(at, KEYWORD_AT) =>
            {
                Some((Some(ChaosTiming::At { fade: true, time: *time }), 4))
            }
            _ => None,
        };
        match timing {
            Some((timing, property_index)) => {
                let LhsPart::Word(name) = parts[property_index] else { unreachable!("the patterns end with a word") };
                issues.append(&mut scalar_issues);
                (timing, ChaosKey::from_name(name))
            }
            None => (None, ChaosKey::Unknown(self.joined_tokens(start, end))),
        }
    }

    /// A number or `random(number, number)` at token `index` of a left-hand side ending before `end`, with the
    /// index after it.
    fn lhs_scalar(&self, index: usize, end: usize, issues: &mut Vec<ChaosIssue>) -> Option<(ChaosScalar, usize)> {
        let token = self.tokens[index];
        match token.kind {
            TokenKind::Number => Some((ChaosScalar::Number(self.number(token, issues)), index + 1)),
            TokenKind::Ident if self.slice(token).eq_ignore_ascii_case(FUNCTION_RANDOM) => {
                const RANDOM_TOKENS: usize = 6;
                let tokens = self.tokens.get(index..index + RANDOM_TOKENS).filter(|_| index + RANDOM_TOKENS <= end)?;
                let kinds = tokens.iter().map(|token| token.kind).collect::<Vec<_>>();
                let expected = [
                    TokenKind::Ident,
                    TokenKind::LeftParen,
                    TokenKind::Number,
                    TokenKind::Comma,
                    TokenKind::Number,
                    TokenKind::RightParen,
                ];
                if kinds != expected {
                    return None;
                }
                let min = self.number(tokens[2], issues);
                let max = self.number(tokens[4], issues);
                Some((ChaosScalar::Random { min, max }, index + RANDOM_TOKENS))
            }
            _ => None,
        }
    }

    /// One value. Line breaks are allowed only inside parentheses.
    fn value(&mut self, depth: usize, issues: &mut Vec<ChaosIssue>) -> Option<ChaosValue> {
        if depth > MAX_VALUE_NESTING {
            return None;
        }
        let token = self.peek()?;
        self.position += 1;
        match token.kind {
            TokenKind::Number => Some(ChaosValue::Scalar(ChaosScalar::Number(self.number(token, issues)))),
            TokenKind::Str => Some(ChaosValue::FileName(self.string_value(token))),
            TokenKind::Ident => {
                let name = self.slice(token);
                if self.peek().is_some_and(|next| next.kind == TokenKind::LeftParen) {
                    self.position += 1;
                    let args = self.arguments(depth, issues)?;
                    Some(typed_call(name, args))
                } else {
                    Some(typed_ident(name))
                }
            }
            _ => None,
        }
    }

    /// Comma-separated values after `(`, up to and including `)`.
    fn arguments(&mut self, depth: usize, issues: &mut Vec<ChaosIssue>) -> Option<Vec<ChaosValue>> {
        let mut args = Vec::new();
        if self.peek()?.kind == TokenKind::RightParen {
            self.position += 1;
            return Some(args);
        }
        loop {
            args.push(self.value(depth + 1, issues)?);
            let separator = self.peek()?;
            self.position += 1;
            match separator.kind {
                TokenKind::Comma => {}
                TokenKind::RightParen => return Some(args),
                _ => return None,
            }
        }
    }

    /// The tokens of the line starting at `first`, up to a `}`, as a raw item.
    fn raw_line(&mut self, first: Token) -> ChaosRaw {
        self.position += 1;
        while let Some(token) = self.peek() {
            if token.kind == TokenKind::RightBrace || self.starts_line(self.position) {
                break;
            }
            self.position += 1;
        }
        let end = self.tokens[self.position - 1].end;
        ChaosRaw::new(&self.text[first.start..end])
    }

    /// The spaces and comment after `end` on its line (not the line end), consumed.
    fn take_trailing(&mut self, end: usize) -> String {
        let bytes = self.text.as_bytes();
        let mut trailing_end = scan(bytes, end, |byte| byte == b' ' || byte == b'\t');
        if self.text[trailing_end..].starts_with(COMMENT_PREFIX) {
            trailing_end = self.text[trailing_end..].find('\n').map_or(self.text.len(), |newline| trailing_end + newline);
            if bytes[..trailing_end].ends_with(b"\r") {
                trailing_end -= 1;
            }
        }
        self.cursor = trailing_end;
        self.text[end..trailing_end].to_owned()
    }

    fn number(&self, token: Token, issues: &mut Vec<ChaosIssue>) -> f64 {
        let (value, well_formed) = number_value(self.slice(token));
        if !well_formed {
            issues.push(self.malformed_number(token, value));
        }
        value
    }

    fn malformed_number(&self, token: Token, value: f64) -> ChaosIssue {
        ChaosIssue {
            location: Some(self.lines.locate(self.text, token.start)),
            sequence: None,
            kind: ChaosIssueKind::MalformedNumber { text: self.slice(token).to_owned(), value },
        }
    }

    /// The tokens `start..end` with one space where the text between two of them is not empty.
    fn joined_tokens(&self, start: usize, end: usize) -> String {
        let mut joined = String::new();
        for index in start..end {
            let token = self.tokens[index];
            if index > start && self.tokens[index - 1].end != token.start {
                joined.push(' ');
            }
            joined.push_str(self.slice(token));
        }
        joined
    }

    /// The next token, which must be of `kind`.
    fn expect(&mut self, kind: TokenKind, after: usize, message: &str) -> eyre::Result<Token> {
        match self.peek() {
            Some(token) if token.kind == kind => {
                self.position += 1;
                Ok(token)
            }
            Some(token) => Err(self.error_at(token.start, message)),
            None => Err(self.error_at(after, message)),
        }
    }

    /// Whether a line break separates token `index` from the previous token.
    fn starts_line(&self, index: usize) -> bool {
        index > 0 && self.text[self.tokens[index - 1].end..self.tokens[index].start].contains('\n')
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.position).copied()
    }

    fn is_keyword(&self, token: Token, keyword: &str) -> bool {
        token.kind == TokenKind::Ident && self.slice(token).eq_ignore_ascii_case(keyword)
    }

    fn slice(&self, token: Token) -> &'a str {
        &self.text[token.start..token.end]
    }

    /// The text between the quotes of a string token.
    fn string_value(&self, token: Token) -> String {
        self.text[token.start + 1..token.end - 1].to_owned()
    }

    fn error_at(&self, offset: usize, message: impl std::fmt::Display) -> eyre::Report {
        eyre::eyre!("{}: {message}", self.lines.locate(self.text, offset))
    }
}

/// The typed form of a call when its name and arguments match one, a generic call otherwise.
fn typed_call(name: &str, args: Vec<ChaosValue>) -> ChaosValue {
    let numbers = || {
        args.iter()
            .map(|arg| match arg {
                ChaosValue::Scalar(ChaosScalar::Number(number)) => Some(*number),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
    };
    let scalars = || {
        args.iter()
            .map(|arg| match arg {
                ChaosValue::Scalar(scalar) => Some(*scalar),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
    };
    let typed = if name.eq_ignore_ascii_case(FUNCTION_RANDOM) {
        numbers()
            .and_then(|numbers| <[f64; RANDOM_ARGUMENTS]>::try_from(numbers).ok())
            .map(|[min, max]| ChaosValue::Scalar(ChaosScalar::Random { min, max }))
    } else if name.eq_ignore_ascii_case(FUNCTION_XYZ) {
        scalars()
            .and_then(|scalars| <[ChaosScalar; CHAOS_VECTOR_COMPONENTS]>::try_from(scalars).ok())
            .map(ChaosValue::Vector)
    } else if name.eq_ignore_ascii_case(FUNCTION_RGBA) {
        scalars()
            .and_then(|scalars| <[ChaosScalar; CHAOS_COLOR_COMPONENTS]>::try_from(scalars).ok())
            .map(ChaosValue::Color)
    } else if name.eq_ignore_ascii_case(FUNCTION_CEL_SETUP) {
        numbers().and_then(|numbers| <[f64; CHAOS_CEL_SETUP_ARGUMENTS]>::try_from(numbers).ok()).map(
            |[cel_count, cel_u, cel_v, columns, rows, fps, looping]| {
                ChaosValue::CelSetup(ChaosCelSetup { cel_count, cel_u, cel_v, columns, rows, fps, looping })
            },
        )
    } else {
        None
    };
    typed.unwrap_or_else(|| ChaosValue::Call { name: name.to_owned(), args })
}

fn typed_ident(name: &str) -> ChaosValue {
    let any_of = |names: &[&str]| names.iter().any(|candidate| candidate.eq_ignore_ascii_case(name));
    if any_of(&BOOLEAN_TRUE_NAMES) {
        ChaosValue::Bool(true)
    } else if any_of(&BOOLEAN_FALSE_NAMES) {
        ChaosValue::Bool(false)
    } else if let Some(mode) = ChaosBlendMode::from_name(name) {
        ChaosValue::BlendMode(mode)
    } else if let Some(particle_type) = ChaosParticleType::from_name(name) {
        ChaosValue::ParticleType(particle_type)
    } else {
        ChaosValue::Ident(name.to_owned())
    }
}
