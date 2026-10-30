//! Text-level check against the client's lexer (`ChaosScript::Tokenize` 0x008b1d50).
//!
//! The uocf parser tokenizes scripts its own way, so the syntax tree cannot show where the client's lexer reads
//! the text differently. [`check_client_lexer`] runs the client's four-state machine (whitespace, word, quoted
//! string, `//` comment) over the raw bytes and reports each such place:
//! - `//` right after a word drops the word (`3.0// x` loses `3.0`);
//! - a delimiter `( ) , { }` typed after whitespace starts a word and merges with the next word character
//!   (`XYZ (1, 2, 3)` gives the token `(1`); after a word or another delimiter it is a token of its own;
//! - a quote right after a word starts a string that includes the word;
//! - `=` is not a delimiter (`a=1` is one token);
//! - a token is a number only if it starts with a digit or `-` (`.5`, `+1`).
//!
//! The 61 client scripts hit none of these. The state machine is the one of the reference script
//! `scripts/dev-helpers/ec-research/ghidra-effects/verify_chaos.py`.

use super::issues::{ChaosIssue, ChaosIssueKind, SourceLocation};

/// Bytes that end a word and form tokens of their own (`=` is not one of them).
const DELIMITERS: &[u8] = b"(),\"{}";
const QUOTE: u8 = b'"';
const EQUALS: u8 = b'=';
const COMMENT_START: &[u8] = b"//";
/// Bytes from here up are skipped in the whitespace state.
const HIGH_BIT_BYTE: u8 = 0x80;
/// Signs and points that look like the start of a number but are not one for the client.
const NOT_A_NUMBER_PREFIXES: &[u8] = b".+";

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Space,
    Word,
    Quoted,
    Comment,
}

/// C `isspace` in the "C" locale.
fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The places where the client's lexer reads `bytes` differently from the uocf parser.
pub fn check_client_lexer(bytes: &[u8]) -> Vec<ChaosIssue> {
    let mut issues = Vec::new();
    let mut state = State::Space;
    let mut token = Vec::new();
    let mut token_location = SourceLocation::default();
    let mut location = SourceLocation { line: 1, column: 1 };
    let push = |issues: &mut Vec<ChaosIssue>, token: &mut Vec<u8>, at: SourceLocation| {
        if let Some(kind) = word_issue(token) {
            issues.push(ChaosIssue { location: Some(at), sequence: None, kind });
        }
        token.clear();
    };
    for (index, &byte) in bytes.iter().enumerate() {
        let starts_comment = bytes[index..].starts_with(COMMENT_START);
        match state {
            State::Space => {
                if is_c_space(byte) || byte >= HIGH_BIT_BYTE {
                } else if starts_comment {
                    state = State::Comment;
                } else {
                    token_location = location;
                    token.push(byte);
                    state = if byte == QUOTE { State::Quoted } else { State::Word };
                }
            }
            State::Word => {
                if is_c_space(byte) {
                    push(&mut issues, &mut token, token_location);
                    state = State::Space;
                } else if byte == QUOTE {
                    // The word is pushed, and the string starts with its text.
                    if !token.is_empty() {
                        issues.push(ChaosIssue {
                            location: Some(token_location),
                            sequence: None,
                            kind: ChaosIssueKind::QuoteAfterWord { word: latin1(&token) },
                        });
                    }
                    token.push(byte);
                    state = State::Quoted;
                } else if DELIMITERS.contains(&byte) {
                    // The word and the delimiter are pushed; the state stays "word" with an empty token.
                    push(&mut issues, &mut token, token_location);
                    token_location = location;
                } else if starts_comment {
                    if !token.is_empty() {
                        issues.push(ChaosIssue {
                            location: Some(token_location),
                            sequence: None,
                            kind: ChaosIssueKind::WordDroppedBeforeComment { word: latin1(&token) },
                        });
                    }
                    token.clear();
                    state = State::Comment;
                } else {
                    if token.is_empty() {
                        token_location = location;
                    }
                    token.push(byte);
                }
            }
            State::Quoted => {
                token.push(byte);
                if byte == QUOTE {
                    token.clear();
                    state = State::Space;
                }
            }
            State::Comment => {
                if byte == b'\n' {
                    state = State::Space;
                }
            }
        }
        if byte == b'\n' {
            location.line += 1;
            location.column = 1;
        } else {
            location.column += 1;
        }
    }
    if state == State::Word {
        push(&mut issues, &mut token, token_location);
    }
    issues
}

/// The issue of a pushed word token, if any.
fn word_issue(token: &[u8]) -> Option<ChaosIssueKind> {
    let (&first, rest) = token.split_first()?;
    if DELIMITERS.contains(&first) && !rest.is_empty() {
        return Some(ChaosIssueKind::DelimiterMergedWithWord { token: latin1(token) });
    }
    if token.contains(&EQUALS) && token.len() > 1 {
        return Some(ChaosIssueKind::EqualsInsideWord { token: latin1(token) });
    }
    if NOT_A_NUMBER_PREFIXES.contains(&first) && rest.first().is_some_and(u8::is_ascii_digit) {
        return Some(ChaosIssueKind::NotANumber { token: latin1(token) });
    }
    None
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<ChaosIssueKind> {
        check_client_lexer(text.as_bytes()).into_iter().map(|issue| issue.kind).collect()
    }

    #[test]
    fn clean_scripts_have_no_issues() {
        let text = "particlesystem \"a\" 1\r\n{\r\n\tlifetime = 1.0\t//random(4.75,5)\r\n\
                    \tchaosLocation = XYZ(random(-1,1),0,0)\r\n\ttexture = \"x.tga\"\r\n}\r\n";
        assert_eq!(kinds(text), []);
    }

    #[test]
    fn reports_the_lexer_quirks() {
        assert_eq!(kinds("lifetime = 3.0// x"), [ChaosIssueKind::WordDroppedBeforeComment { word: "3.0".into() }]);
        assert_eq!(kinds("v = XYZ (1, 2, 3)"), [ChaosIssueKind::DelimiterMergedWithWord { token: "(1".into() }]);
        assert_eq!(kinds("a=1"), [ChaosIssueKind::EqualsInsideWord { token: "a=1".into() }]);
        assert_eq!(kinds("scale = .5"), [ChaosIssueKind::NotANumber { token: ".5".into() }]);
        assert_eq!(kinds("texture =x\"a.tga\""), [
            ChaosIssueKind::QuoteAfterWord { word: "=x".into() }
        ]);
    }
}
