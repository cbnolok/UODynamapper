//! Findings of the Chaos compiler and of the client lexer check.

use std::fmt;

use super::desc::{ChaosEventKind, ChaosParticleType};

/// 1-based line and column of an element in the script text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChaosIssueSeverity {
    /// A client rule that shapes the result but is common in the client data.
    Note,
    /// The client accepts it, but it is likely a mistake or has no effect.
    Warning,
    /// The client rejects the script (or hangs on it): it plays nothing.
    Error,
}

/// One finding.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosIssue {
    pub location: Option<SourceLocation>,
    /// The sequence holding the element; `None` at system level.
    pub sequence: Option<String>,
    pub kind: ChaosIssueKind,
}

/// What was found. Error messages quote the client's where it has one.
#[derive(Debug, Clone, PartialEq)]
pub enum ChaosIssueKind {
    // --- Errors: the client's compiler throws.
    /// The uocf parser could not read the script at all.
    SyntaxError { message: String },
    /// "Unexpected "%s" in Particle System Block!"
    UnexpectedInSystemBlock { token: String },
    /// "Unexpected "%s" in Sequence Block!"
    UnexpectedInSequenceBlock { token: String },
    /// "Cannot specify any sequence properties after specifying events."
    PropertyAfterEvent { token: String },
    /// "Expecting either Random(...) or a number value."
    ExpectedNumber { token: String },
    /// A vector property or event needs `XYZ(x, y, z)`.
    ExpectedVector { token: String },
    /// A colour event needs `RGBA(r, g, b, a)`.
    ExpectedColor { token: String },
    /// "Expecting CELSETUP(...)!"
    ExpectedCelSetup,
    /// "Expected filename after texture sequence property." (and the mesh and emitter forms).
    ExpectedFileName { token: String },
    /// A bool value without 1/T/Y or 0/F/N.
    InvalidBool { token: String },
    /// A `type` value that names no particle type.
    UnknownParticleType { token: String },
    /// A blend value that is not one of the `ALPHA_*` names.
    UnknownBlendMode { token: String },
    /// "Unknown sequence vector property!" (`emitRadius` is classified but has no handler).
    UnknownVectorProperty { token: String },
    /// "Expecting particle property after time specifier!" (`alpha`, `chaosLocation`, ...).
    ExpectedEventProperty { token: String },
    /// `fade so initial`: an initial value cannot fade.
    FadeOnInitial,
    /// "Cannot have StayAtEmitter if Local is true!"
    StayAtEmitterWithLocal,
    /// "Cannot have Local if StayAtEmitter is true!"
    LocalWithStayAtEmitter,
    /// "Cannot have ChaosLocation event when StayAtEmitter is true!" (chaosLocation and chaosVelocity).
    SpawnVectorWithStayAtEmitter { token: String },
    /// "Cannot have <event> event when StayAtEmitter is true!"
    MotionEventWithStayAtEmitter { event: ChaosEventKind },
    /// "Cannot have inherited velocity if Local is true!"
    InheritedWithLocal,
    /// `collisionCheck`: the client compiles it, then loops forever in the sequence update.
    CollisionCheckHangs,
    // --- Errors: the client's lexer reads the text differently from the syntax tree.
    /// `//` right after a word: the client drops the word.
    WordDroppedBeforeComment { word: String },
    /// A delimiter typed after whitespace merges with the next word (`XYZ (1` gives the token `(1`).
    DelimiterMergedWithWord { token: String },
    /// A quote right after a word: the quoted string starts with the word.
    QuoteAfterWord { word: String },
    /// `=` is not a delimiter: `a=1` is one token.
    EqualsInsideWord { token: String },
    /// A number must start with a digit or `-` (`.5` and `+1` are words).
    NotANumber { token: String },
    // --- Warnings.
    /// `random(min, max)` with `min > max`: the client draws from 0..1 (0 for integer ranges).
    ReversedRandom { min: f32, max: f32 },
    /// A keyword the client matched as a substring of a longer word (`sorted` is `sort`).
    SubstringKeyword { written: String, keyword: &'static str },
    /// Parsed but never read at run time (`loops`, `centered`, `sort`, `locationOnly`).
    NoEffect { token: String },
    /// The particle type has no renderer (BILLBOARD, LIGHT, TEST, none) or is not simulated (EMITTER).
    NotRendered { particle_type: ChaosParticleType },
    /// `final` before any `lifetime` line: the event time is 0, at spawn.
    FinalBeforeLifetime,
    /// A negative emit rate: the interval clamps to the minimum, so the sequence emits 60 times per second.
    NegativeEmitRate { rate: f32 },
    /// An EVENTTIMER event sets the timer to a value its own time can take: it can rewind the event list onto
    /// itself, which hangs the client.
    EventTimerLoop,
    // --- Notes.
    /// `emitRate = random(a, b)`: only `b` is used.
    EmitRateRange { min: f32, max: f32 },
    /// `burst = random(a, b)`: `b` is exclusive.
    BurstRange { min: u32, max: u32 },
    /// `at random(a, b)`: drawn once for the compiled script, shared by every particle.
    SharedRandomEventTime { min: f32, max: f32 },
    /// A fade with no later fade of its kind before the lifetime max: its ramp continues.
    FadeStepPersists { event: ChaosEventKind },
}

impl ChaosIssue {
    pub fn severity(&self) -> ChaosIssueSeverity {
        use ChaosIssueKind as K;
        match self.kind {
            K::ReversedRandom { .. }
            | K::SubstringKeyword { .. }
            | K::NoEffect { .. }
            | K::NotRendered { .. }
            | K::FinalBeforeLifetime
            | K::NegativeEmitRate { .. }
            | K::EventTimerLoop => ChaosIssueSeverity::Warning,
            K::EmitRateRange { .. }
            | K::BurstRange { .. }
            | K::SharedRandomEventTime { .. }
            | K::FadeStepPersists { .. } => ChaosIssueSeverity::Note,
            _ => ChaosIssueSeverity::Error,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity() == ChaosIssueSeverity::Error
    }
}

impl fmt::Display for ChaosIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = self.location {
            write!(f, "{location}: ")?;
        }
        if let Some(sequence) = &self.sequence {
            write!(f, "eventsequence \"{sequence}\": ")?;
        }
        write!(f, "{}", self.kind)
    }
}

impl fmt::Display for ChaosIssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SyntaxError { message } => write!(f, "syntax error: {message}"),
            Self::UnexpectedInSystemBlock { token } => write!(f, "Unexpected \"{token}\" in Particle System Block!"),
            Self::UnexpectedInSequenceBlock { token } => write!(f, "Unexpected \"{token}\" in Sequence Block!"),
            Self::PropertyAfterEvent { token } => {
                write!(f, "Cannot specify any sequence properties after specifying events. ({token})")
            }
            Self::ExpectedNumber { token } => write!(f, "Expecting either Random(...) or a number value. ({token})"),
            Self::ExpectedVector { token } => write!(f, "{token}: expecting XYZ(x, y, z)"),
            Self::ExpectedColor { token } => write!(f, "{token}: expecting RGBA(r, g, b, a)"),
            Self::ExpectedCelSetup => f.write_str("Expecting CELSETUP(...)!"),
            Self::ExpectedFileName { token } => write!(f, "{token}: expecting a quoted file name"),
            Self::InvalidBool { token } => write!(f, "\"{token}\" is not a bool (needs 1, T or Y, or 0, F or N)"),
            Self::UnknownParticleType { token } => write!(f, "unknown particle type \"{token}\""),
            Self::UnknownBlendMode { token } => write!(f, "unknown blend mode \"{token}\""),
            Self::UnknownVectorProperty { token } => write!(f, "Unknown sequence vector property! ({token})"),
            Self::ExpectedEventProperty { token } => {
                write!(f, "Expecting particle property after time specifier! ({token})")
            }
            Self::FadeOnInitial => f.write_str("an initial value cannot fade (fade so initial)"),
            Self::StayAtEmitterWithLocal => f.write_str("Cannot have StayAtEmitter if Local is true!"),
            Self::LocalWithStayAtEmitter => f.write_str("Cannot have Local if StayAtEmitter is true!"),
            Self::SpawnVectorWithStayAtEmitter { token } => {
                write!(f, "Cannot have ChaosLocation event when StayAtEmitter is true! ({token})")
            }
            Self::MotionEventWithStayAtEmitter { event } => {
                write!(f, "Cannot have {event:?} event when StayAtEmitter is true!")
            }
            Self::InheritedWithLocal => f.write_str("Cannot have inherited velocity if Local is true!"),
            Self::CollisionCheckHangs => f.write_str("collisionCheck hangs the client at run time"),
            Self::WordDroppedBeforeComment { word } => write!(f, "the client drops \"{word}\" (`//` right after it)"),
            Self::DelimiterMergedWithWord { token } => {
                write!(f, "the client reads the token \"{token}\" (a delimiter after whitespace joins the next word)")
            }
            Self::QuoteAfterWord { word } => write!(f, "the client starts a quoted string with \"{word}\""),
            Self::EqualsInsideWord { token } => write!(f, "\"{token}\": `=` needs whitespace around it"),
            Self::NotANumber { token } => write!(f, "\"{token}\" is not a number for the client (start with a digit or -)"),
            Self::ReversedRandom { min, max } => write!(f, "random({min}, {max}) has min > max: the client draws 0..1"),
            Self::SubstringKeyword { written, keyword } => write!(f, "\"{written}\" is read as {keyword}"),
            Self::NoEffect { token } => write!(f, "{token} has no effect in the client"),
            Self::NotRendered { particle_type } => write!(f, "particle type {particle_type:?} draws nothing"),
            Self::FinalBeforeLifetime => f.write_str("final before lifetime: the event happens at spawn"),
            Self::NegativeEmitRate { rate } => write!(f, "negative emit rate {rate}: emits 60 times per second"),
            Self::EventTimerLoop => f.write_str("eventTimer can rewind onto itself: the client would hang"),
            Self::EmitRateRange { min, max } => write!(f, "emitRate random({min}, {max}): only {max} is used"),
            Self::BurstRange { min, max } => write!(f, "burst random({min}, {max}): {max} is exclusive"),
            Self::SharedRandomEventTime { min, max } => {
                write!(f, "at random({min}, {max}) is drawn once and shared by every particle")
            }
            Self::FadeStepPersists { event } => {
                write!(f, "the {event:?} fade has no later fade before the lifetime max: it keeps ramping")
            }
        }
    }
}
