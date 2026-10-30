//! Compiles a uocf Chaos syntax tree into a [`ChaosSystemDesc`] with the client's rules
//! (`Emitter::CompileScript` 0x008a1160, `ParticleEventSequence::CompileBlock` 0x0089ff00,
//! `ChaosScript::ClassifyToken` 0x008b15c0, `ParticleEventSequence::CreateEvent` 0x0089f9b0).
//!
//! The uocf parser accepts more than the client and keeps what it does not understand, so this pass decides
//! what the client would do with each statement:
//! - keywords are matched as the client does: uppercased, by substring, first match wins (`sorted` is `sort`,
//!   `alphaColor` is the otherwise unreachable ALPHA event, `alpha` alone is no event);
//! - a property after the first event of a sequence, `emitRadius`, `collisionCheck`, StayAtEmitter / Local
//!   conflicts and the other compile errors are reported with the client's message;
//! - multipliers scale the events that follow them, ROTATION becomes radians, `final` takes the lifetime max
//!   parsed so far;
//! - quirks the client accepts are reported as warnings or notes (see [`ChaosIssueKind`]).
//!
//! The client stops at its first error; this pass reports every error it finds and returns no description
//! when there is one. Lexer-level differences need the source text: [`compile_chaos_source`] adds
//! [`super::lexer_check::check_client_lexer`].

use uocf::enhanced::chaos_script::{
    ChaosItem, ChaosKey, ChaosScalar, ChaosScript, ChaosSequence, ChaosSourceLocation, ChaosStatement, ChaosTiming,
    ChaosValue,
};

use super::desc::{
    ChaosCelSetup, ChaosEventDesc, ChaosEventKind, ChaosEventValue, ChaosParticleType, ChaosSequenceDesc,
    ChaosSequenceFlags, ChaosSystemDesc, IntRange, Range, VecRange,
};
use super::issues::{ChaosIssue, ChaosIssueKind, SourceLocation};
use super::lexer_check::check_client_lexer;
use super::{DEGREES_TO_RADIANS, INT_RANGE_MASK};
use crate::render::BlendFactor;

/// The result of a compile: the description when no error was found, and every finding.
#[derive(Debug, Clone)]
pub struct ChaosCompileReport {
    pub desc: Option<ChaosSystemDesc>,
    pub issues: Vec<ChaosIssue>,
}

impl ChaosCompileReport {
    pub fn errors(&self) -> impl Iterator<Item = &ChaosIssue> {
        self.issues.iter().filter(|issue| issue.is_error())
    }

    /// The description, or the errors.
    pub fn into_result(self) -> Result<ChaosSystemDesc, Vec<ChaosIssue>> {
        match self.desc {
            Some(desc) => Ok(desc),
            None => Err(self.issues.into_iter().filter(ChaosIssue::is_error).collect()),
        }
    }
}

/// How the client classifies a word (`ClassifyToken` type codes in the comments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenClass {
    /// 0x01: starts with a digit or `-`.
    Number,
    /// 0x26
    BoolProperty,
    /// 0x09
    ParticleSystem,
    /// 0x0a
    EventSequence,
    /// 0x0b
    Random,
    /// 0x0c
    Xyz,
    /// 0x0e
    Rgba,
    /// 0x21
    Fade,
    /// 0x0d
    CelSetup,
    /// 0x24
    Initial,
    /// 0x25
    Final,
    /// 0x0f
    Texture,
    /// 0x11
    Type,
    /// 0x10
    Inherited,
    /// 0x13
    EmitterName,
    /// 0x14
    MeshName,
    /// 0x15
    CelAnimate,
    /// 0x16
    Burst,
    /// 0x17
    EmissionBoost,
    /// 0x18
    CollisionCheck,
    /// 0x19
    TypeValue,
    /// 0x22, exact match only.
    So,
    /// 0x23, exact match only.
    At,
    /// 0x1a
    BlendValue,
    /// 0x29
    BlendProperty,
    /// 0x28
    VectorProperty,
    /// 0x27
    NumericProperty,
    /// 0x1b
    MinEmitTime,
    /// 0x1d
    LocationOnly,
    /// 0x1e
    ScalarEvent,
    /// 0x1f
    VectorEvent,
    /// 0x20
    ColorEvent,
    /// 0x2a
    Other,
}

/// `ClassifyToken`'s substring tests, in order (first match wins). `So` and `At` are tested separately, for the
/// whole word, at their place in this order (after the type values).
const CLASSIFY_BEFORE_SO_AT: &[(&[&str], TokenClass)] = &[
    (&["STAYATEMITTER", "CENTERED", "SORT", "LOCAL"], TokenClass::BoolProperty),
    (&["PARTICLESYSTEM"], TokenClass::ParticleSystem),
    (&["EVENTSEQUENCE"], TokenClass::EventSequence),
    (&["RANDOM"], TokenClass::Random),
    (&["XYZ"], TokenClass::Xyz),
    (&["RGBA"], TokenClass::Rgba),
    (&["FADE"], TokenClass::Fade),
    (&["CELSETUP"], TokenClass::CelSetup),
    (&["INITIAL"], TokenClass::Initial),
    (&["FINAL"], TokenClass::Final),
    (&["TEXTURE"], TokenClass::Texture),
    (&["TYPE"], TokenClass::Type),
    (&["INHERITED"], TokenClass::Inherited),
    (&["EMITTERNAME"], TokenClass::EmitterName),
    (&["MESHNAME"], TokenClass::MeshName),
    (&["CELANIMATE"], TokenClass::CelAnimate),
    (&["BURST"], TokenClass::Burst),
    (&["EMISSIONBOOST"], TokenClass::EmissionBoost),
    (&["COLLISIONCHECK"], TokenClass::CollisionCheck),
    (&["POINT", "MESH", "BILLBOARD", "EMITTER", "LIGHT", "TEST", "AXIAL"], TokenClass::TypeValue),
];
const KEYWORD_SO: &str = "SO";
const KEYWORD_AT: &str = "AT";
const CLASSIFY_AFTER_SO_AT: &[(&[&str], TokenClass)] = &[
    (&["ALPHA_"], TokenClass::BlendValue),
    (&["SOURCEBLENDMODE", "DESTBLENDMODE"], TokenClass::BlendProperty),
    (&["GRAVITY", "EMITRADIUS", "CHAOSLOCATION", "CHAOSVELOCITY"], TokenClass::VectorProperty),
    (
        &["LIFETIME", "EMITRATE", "NUMPARTICLES", "SCALEMULTIPLIER", "VELOCITYMULTIPLIER", "LOOPS"],
        TokenClass::NumericProperty,
    ),
    (&["MINEMITTIME"], TokenClass::MinEmitTime),
    (&["LOCATIONONLY"], TokenClass::LocationOnly),
    (&["SCALE", "EVENTTIMER", "THRUST", "LENGTH"], TokenClass::ScalarEvent),
    (&["VELOCITY", "ROTATION"], TokenClass::VectorEvent),
    (&["COLOR"], TokenClass::ColorEvent),
];

/// The class of an uppercase word, and the keyword that matched.
fn classify(upper: &str) -> (TokenClass, Option<&'static str>) {
    if upper.starts_with(|first: char| first.is_ascii_digit() || first == '-') {
        return (TokenClass::Number, None);
    }
    let find = |table: &[(&[&'static str], TokenClass)]| {
        table.iter().find_map(|(keywords, class)| {
            keywords.iter().find(|keyword| upper.contains(*keyword)).map(|keyword| (*class, Some(*keyword)))
        })
    };
    if let Some(found) = find(CLASSIFY_BEFORE_SO_AT) {
        return found;
    }
    if upper == KEYWORD_SO {
        return (TokenClass::So, Some(KEYWORD_SO));
    }
    if upper == KEYWORD_AT {
        return (TokenClass::At, Some(KEYWORD_AT));
    }
    find(CLASSIFY_AFTER_SO_AT).unwrap_or((TokenClass::Other, None))
}

/// The bool properties, in the order the sequence block tells them apart.
const BOOL_PROPERTIES: [(&str, u32); 4] = [
    ("STAYATEMITTER", ChaosSequenceFlags::STAY_AT_EMITTER),
    ("CENTERED", ChaosSequenceFlags::CENTERED),
    ("SORT", ChaosSequenceFlags::SORT),
    ("LOCAL", ChaosSequenceFlags::LOCAL),
];
const BOOL_TRUE_CHARACTERS: &[char] = &['1', 'T', 'Y'];
const BOOL_FALSE_CHARACTERS: &[char] = &['0', 'F', 'N'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericProperty {
    Lifetime,
    EmitRate,
    NumParticles,
    ScaleMultiplier,
    VelocityMultiplier,
    Loops,
}

const NUMERIC_PROPERTIES: [(&str, NumericProperty); 6] = [
    ("LIFETIME", NumericProperty::Lifetime),
    ("EMITRATE", NumericProperty::EmitRate),
    ("NUMPARTICLES", NumericProperty::NumParticles),
    ("SCALEMULTIPLIER", NumericProperty::ScaleMultiplier),
    ("VELOCITYMULTIPLIER", NumericProperty::VelocityMultiplier),
    ("LOOPS", NumericProperty::Loops),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VectorProperty {
    Gravity,
    ChaosLocation,
    ChaosVelocity,
}

/// The vector properties with a handler (`EMITRADIUS` is classified but has none).
const VECTOR_PROPERTIES: [(&str, VectorProperty); 3] = [
    ("GRAVITY", VectorProperty::Gravity),
    ("CHAOSLOCATION", VectorProperty::ChaosLocation),
    ("CHAOSVELOCITY", VectorProperty::ChaosVelocity),
];

const SOURCE_BLEND_KEYWORD: &str = "SOURCEBLENDMODE";

/// `ParseParticleType` substrings, in order.
const PARTICLE_TYPES: [(&str, ChaosParticleType); 7] = [
    ("POINT", ChaosParticleType::Point),
    ("MESH", ChaosParticleType::Mesh),
    ("BILLBOARD", ChaosParticleType::Billboard),
    ("EMITTER", ChaosParticleType::Emitter),
    ("AXIAL", ChaosParticleType::Axial),
    ("LIGHT", ChaosParticleType::Light),
    ("TEST", ChaosParticleType::Test),
];

/// `CreateEvent` substrings, in order.
const EVENT_KEYWORDS: [(&str, ChaosEventKind); 14] = [
    ("SCALE", ChaosEventKind::Scale),
    ("EVENTTIMER", ChaosEventKind::EventTimer),
    ("REDCOLOR", ChaosEventKind::RedColor),
    ("GREENCOLOR", ChaosEventKind::GreenColor),
    ("BLUECOLOR", ChaosEventKind::BlueColor),
    ("ALPHA", ChaosEventKind::Alpha),
    ("COLOR", ChaosEventKind::Color),
    ("LENGTH", ChaosEventKind::Length),
    ("VELOCITYX", ChaosEventKind::VelocityX),
    ("VELOCITYY", ChaosEventKind::VelocityY),
    ("VELOCITYZ", ChaosEventKind::VelocityZ),
    ("VELOCITY", ChaosEventKind::Velocity),
    ("THRUST", ChaosEventKind::Thrust),
    ("ROTATION", ChaosEventKind::Rotation),
];

/// Event kinds whose persisting fades are reported (as `verify_chaos.py` counts them).
const PERSISTING_FADE_KINDS: [ChaosEventKind; 3] =
    [ChaosEventKind::Scale, ChaosEventKind::Color, ChaosEventKind::Velocity];

/// Path separators `_splitpath` removes in front of the file name.
const PATH_SEPARATORS: [char; 3] = ['/', '\\', ':'];

const VECTOR_COMPONENTS: usize = 3;
const COLOR_COMPONENTS: usize = 4;
const CEL_SETUP_ARGUMENTS: usize = 7;
const RANDOM_ARGUMENTS: usize = 2;

/// When an event happens.
#[derive(Debug, Clone, Copy)]
enum EventTime {
    Initial,
    Final,
    At(Range),
}

/// The left-hand side of a statement, as the client reads it.
#[derive(Debug, Clone)]
enum Lhs {
    Property { word: String },
    Event { fade: bool, time: EventTime, word: String },
}

/// Parses `bytes` with uocf, checks them against the client's lexer and compiles them.
pub fn compile_chaos_source(bytes: &[u8]) -> ChaosCompileReport {
    let mut issues = check_client_lexer(bytes);
    match ChaosScript::parse_bytes(bytes) {
        Ok(script) => {
            let mut report = compile_chaos_script(&script);
            let has_lexer_error = issues.iter().any(ChaosIssue::is_error);
            issues.append(&mut report.issues);
            ChaosCompileReport { desc: report.desc.filter(|_| !has_lexer_error), issues }
        }
        Err(error) => {
            issues.push(ChaosIssue {
                location: None,
                sequence: None,
                kind: ChaosIssueKind::SyntaxError { message: format!("{error:#}") },
            });
            ChaosCompileReport { desc: None, issues }
        }
    }
}

/// Compiles a parsed script with the client's rules.
pub fn compile_chaos_script(script: &ChaosScript) -> ChaosCompileReport {
    let mut compiler = Compiler { issues: Vec::new(), location: None, sequence: None };
    let mut desc = ChaosSystemDesc::new(script.name.clone());
    desc.script_version = script.version as f32;
    for item in &script.items {
        compiler.location = item.source_location().map(convert_location);
        compiler.sequence = None;
        match item {
            ChaosItem::Statement(statement) => {
                if let Err(kind) = compiler.system_statement(&mut desc, statement) {
                    compiler.push(kind);
                }
            }
            ChaosItem::Sequence(sequence) => desc.sequences.push(compiler.sequence(sequence)),
            ChaosItem::Raw(raw) => compiler.push(ChaosIssueKind::UnexpectedInSystemBlock { token: first_word(&raw.text) }),
        }
    }
    let has_error = compiler.issues.iter().any(ChaosIssue::is_error);
    ChaosCompileReport { desc: (!has_error).then_some(desc), issues: compiler.issues }
}

struct Compiler {
    issues: Vec<ChaosIssue>,
    location: Option<SourceLocation>,
    sequence: Option<String>,
}

/// A sequence being compiled.
struct SequenceBuilder {
    desc: ChaosSequenceDesc,
    lifetime_set: bool,
    seen_event: bool,
}

impl Compiler {
    fn push(&mut self, kind: ChaosIssueKind) {
        self.issues.push(ChaosIssue { location: self.location, sequence: self.sequence.clone(), kind });
    }

    /// Warns when `keyword` matched only part of `written`.
    fn keyword_match(&mut self, written: &str, keyword: &'static str) {
        if !written.eq_ignore_ascii_case(keyword) {
            self.push(ChaosIssueKind::SubstringKeyword { written: written.to_owned(), keyword });
        }
    }

    fn system_statement(&mut self, desc: &mut ChaosSystemDesc, statement: &ChaosStatement) -> Result<(), ChaosIssueKind> {
        let word = match self.lhs(statement)? {
            Lhs::Property { word } => word,
            Lhs::Event { .. } => {
                return Err(ChaosIssueKind::UnexpectedInSystemBlock { token: timing_word(statement) });
            }
        };
        let upper = word.to_ascii_uppercase();
        match classify(&upper) {
            (TokenClass::MinEmitTime, Some(keyword)) => {
                self.keyword_match(&word, keyword);
                desc.min_emit_time = self.range(&statement.value)?;
            }
            (TokenClass::LocationOnly, Some(keyword)) => {
                self.keyword_match(&word, keyword);
                bool_value(&statement.value)?;
                self.push(ChaosIssueKind::NoEffect { token: upper });
            }
            _ => return Err(ChaosIssueKind::UnexpectedInSystemBlock { token: upper }),
        }
        Ok(())
    }

    fn sequence(&mut self, sequence: &ChaosSequence) -> ChaosSequenceDesc {
        self.sequence = Some(sequence.name.clone());
        let mut builder =
            SequenceBuilder { desc: ChaosSequenceDesc::new(sequence.name.clone()), lifetime_set: false, seen_event: false };
        for item in &sequence.items {
            self.location = item.source_location().map(convert_location);
            let result = match item {
                ChaosItem::Statement(statement) => self.sequence_statement(&mut builder, statement),
                ChaosItem::Raw(raw) => Err(ChaosIssueKind::UnexpectedInSequenceBlock { token: first_word(&raw.text) }),
                ChaosItem::Sequence(_) => {
                    Err(ChaosIssueKind::UnexpectedInSequenceBlock { token: "EVENTSEQUENCE".to_owned() })
                }
            };
            if let Err(kind) = result {
                self.push(kind);
            }
        }
        self.location = sequence.source_location().map(convert_location);
        self.finish_sequence(&builder);
        builder.desc
    }

    fn sequence_statement(&mut self, builder: &mut SequenceBuilder, statement: &ChaosStatement) -> Result<(), ChaosIssueKind> {
        match self.lhs(statement)? {
            Lhs::Event { fade, time, word } => {
                builder.seen_event = true;
                self.event(builder, fade, time, &word, &statement.value)
            }
            Lhs::Property { word } => {
                if builder.seen_event {
                    return Err(ChaosIssueKind::PropertyAfterEvent { token: word.to_ascii_uppercase() });
                }
                self.property(builder, &word, &statement.value)
            }
        }
    }

    /// The left-hand side as the client reads it. uocf keeps a left-hand side it cannot read as an unknown key
    /// (its words separated by spaces); the client may still read it (`initially scale` is `initial scale`).
    fn lhs(&mut self, statement: &ChaosStatement) -> Result<Lhs, ChaosIssueKind> {
        let key = match &statement.key {
            ChaosKey::Known(property) => property.name().to_owned(),
            ChaosKey::Unknown(text) => text.clone(),
        };
        let words: Vec<&str> = key.split_whitespace().collect();
        let Some(timing) = statement.timing else {
            return match words.as_slice() {
                [word] => Ok(Lhs::Property { word: (*word).to_owned() }),
                _ => self.lhs_words(&words),
            };
        };
        let [word] = words.as_slice() else {
            return Err(ChaosIssueKind::ExpectedEventProperty { token: key.to_ascii_uppercase() });
        };
        let (fade, time) = match timing {
            ChaosTiming::Initial => (false, EventTime::Initial),
            ChaosTiming::Final { fade } => (fade, EventTime::Final),
            ChaosTiming::At { fade, time } => (fade, EventTime::At(self.scalar(time))),
        };
        Ok(Lhs::Event { fade, time, word: (*word).to_owned() })
    }

    /// `[FADE SO] (INITIAL | FINAL | AT number) keyword` from separate words.
    fn lhs_words(&mut self, words: &[&str]) -> Result<Lhs, ChaosIssueKind> {
        let unexpected = |word: &str| ChaosIssueKind::UnexpectedInSequenceBlock { token: word.to_ascii_uppercase() };
        let Some(first) = words.first() else {
            return Err(unexpected(""));
        };
        let class = |word: &str| classify(&word.to_ascii_uppercase()).0;
        let (fade, rest) = match class(first) {
            TokenClass::Fade => match words.get(1) {
                Some(so) if class(so) == TokenClass::So => (true, &words[2..]),
                Some(other) => return Err(unexpected(other)),
                None => return Err(unexpected(first)),
            },
            _ => (false, words),
        };
        let (time, rest) = match rest.first().map(|word| class(word)) {
            Some(TokenClass::Initial) => (EventTime::Initial, &rest[1..]),
            Some(TokenClass::Final) => (EventTime::Final, &rest[1..]),
            Some(TokenClass::At) => match rest.get(1) {
                Some(number) if class(number) == TokenClass::Number => {
                    (EventTime::At(Range::constant(c_atof(number) as f32)), &rest[2..])
                }
                Some(other) => return Err(ChaosIssueKind::ExpectedNumber { token: other.to_ascii_uppercase() }),
                None => return Err(unexpected(rest[0])),
            },
            _ => return Err(unexpected(rest.first().unwrap_or(first))),
        };
        match rest {
            [word] => Ok(Lhs::Event { fade, time, word: (*word).to_owned() }),
            [] => Err(ChaosIssueKind::ExpectedEventProperty { token: String::new() }),
            [_, extra, ..] => Err(unexpected(extra)),
        }
    }

    fn property(&mut self, builder: &mut SequenceBuilder, word: &str, value: &ChaosValue) -> Result<(), ChaosIssueKind> {
        let upper = word.to_ascii_uppercase();
        let (class, keyword) = classify(&upper);
        let sequence = &mut builder.desc;
        match class {
            TokenClass::BoolProperty => {
                let (name, flag) = BOOL_PROPERTIES
                    .into_iter()
                    .find(|(name, _)| upper.contains(name))
                    .expect("the classifier matched one of the bool properties");
                self.keyword_match(word, name);
                if bool_value(value)? {
                    if flag == ChaosSequenceFlags::STAY_AT_EMITTER && sequence.flags.contains(ChaosSequenceFlags::LOCAL) {
                        return Err(ChaosIssueKind::StayAtEmitterWithLocal);
                    }
                    if flag == ChaosSequenceFlags::LOCAL && sequence.flags.contains(ChaosSequenceFlags::STAY_AT_EMITTER) {
                        return Err(ChaosIssueKind::LocalWithStayAtEmitter);
                    }
                    sequence.flags.insert(flag);
                }
                if matches!(flag, ChaosSequenceFlags::CENTERED | ChaosSequenceFlags::SORT) {
                    self.push(ChaosIssueKind::NoEffect { token: name.to_owned() });
                }
            }
            TokenClass::Texture | TokenClass::MeshName | TokenClass::EmitterName => {
                self.keyword_match(word, keyword.unwrap_or_default());
                sequence.resource = file_name(value, &upper)?;
            }
            TokenClass::Type => {
                self.keyword_match(word, keyword.unwrap_or_default());
                sequence.particle_type = particle_type(value)?;
            }
            TokenClass::BlendProperty => {
                self.keyword_match(word, keyword.unwrap_or_default());
                let factor = blend_factor(value)?;
                if upper.contains(SOURCE_BLEND_KEYWORD) {
                    sequence.src_blend = factor;
                } else {
                    sequence.dst_blend = factor;
                }
            }
            TokenClass::NumericProperty => {
                let (name, property) = NUMERIC_PROPERTIES
                    .into_iter()
                    .find(|(name, _)| upper.contains(name))
                    .expect("the classifier matched one of the numeric properties");
                self.keyword_match(word, name);
                let range = self.range(value)?;
                let sequence = &mut builder.desc;
                match property {
                    NumericProperty::Lifetime => {
                        sequence.lifetime = range;
                        builder.lifetime_set = true;
                    }
                    NumericProperty::EmitRate => {
                        sequence.emit_rate = range;
                        if range.min != range.max {
                            self.push(ChaosIssueKind::EmitRateRange { min: range.min, max: range.max });
                        }
                        if range.max < 0.0 {
                            self.push(ChaosIssueKind::NegativeEmitRate { rate: range.max });
                        }
                    }
                    NumericProperty::NumParticles => sequence.num_particles = range.max as i32 as u16,
                    NumericProperty::ScaleMultiplier => {
                        sequence.scale_multiplier = range.max;
                        sequence.flags.insert(ChaosSequenceFlags::SCALE_MULTIPLIER);
                    }
                    NumericProperty::VelocityMultiplier => {
                        sequence.velocity_multiplier = range.max;
                        sequence.flags.insert(ChaosSequenceFlags::VELOCITY_MULTIPLIER);
                    }
                    NumericProperty::Loops => {
                        sequence.loops = range.max as i32;
                        self.push(ChaosIssueKind::NoEffect { token: name.to_owned() });
                    }
                }
            }
            TokenClass::VectorProperty => {
                let Some((name, property)) = VECTOR_PROPERTIES.into_iter().find(|(name, _)| upper.contains(name)) else {
                    return Err(ChaosIssueKind::UnknownVectorProperty { token: upper });
                };
                self.keyword_match(word, name);
                if property != VectorProperty::Gravity && sequence.flags.contains(ChaosSequenceFlags::STAY_AT_EMITTER) {
                    return Err(ChaosIssueKind::SpawnVectorWithStayAtEmitter { token: upper });
                }
                let vector = self.vector(value, &upper)?;
                let sequence = &mut builder.desc;
                match property {
                    VectorProperty::Gravity => sequence.gravity = vector,
                    VectorProperty::ChaosLocation => sequence.chaos_location = vector,
                    VectorProperty::ChaosVelocity => sequence.chaos_velocity = vector,
                }
            }
            TokenClass::Inherited => {
                self.keyword_match(word, keyword.unwrap_or_default());
                if sequence.flags.contains(ChaosSequenceFlags::LOCAL) {
                    return Err(ChaosIssueKind::InheritedWithLocal);
                }
                builder.desc.inherited = self.vector(value, &upper)?;
            }
            TokenClass::Burst => {
                self.keyword_match(word, keyword.unwrap_or_default());
                let burst = int_range(value)?;
                if burst.min != burst.max {
                    self.push(ChaosIssueKind::BurstRange { min: burst.min, max: burst.max });
                }
                builder.desc.burst = burst;
            }
            TokenClass::EmissionBoost => {
                self.keyword_match(word, keyword.unwrap_or_default());
                builder.desc.emission_boost = self.range(value)?;
                builder.desc.flags.insert(ChaosSequenceFlags::EMISSION_BOOST);
            }
            TokenClass::CollisionCheck => return Err(ChaosIssueKind::CollisionCheckHangs),
            TokenClass::CelAnimate => {
                self.keyword_match(word, keyword.unwrap_or_default());
                let cel = self.cel_setup(value)?;
                let sequence = &mut builder.desc;
                sequence.flags.insert(ChaosSequenceFlags::CEL_ANIMATE);
                if cel.looping {
                    sequence.flags.insert(ChaosSequenceFlags::CEL_LOOP);
                }
                sequence.cel = cel;
            }
            _ => return Err(ChaosIssueKind::UnexpectedInSequenceBlock { token: upper }),
        }
        Ok(())
    }

    fn event(
        &mut self,
        builder: &mut SequenceBuilder,
        fade: bool,
        time: EventTime,
        word: &str,
        value: &ChaosValue,
    ) -> Result<(), ChaosIssueKind> {
        let upper = word.to_ascii_uppercase();
        let time = match time {
            EventTime::Initial if fade => return Err(ChaosIssueKind::FadeOnInitial),
            EventTime::Initial => Range::ZERO,
            EventTime::Final => {
                if !builder.lifetime_set {
                    self.push(ChaosIssueKind::FinalBeforeLifetime);
                }
                Range::constant(builder.desc.lifetime.max)
            }
            EventTime::At(range) => {
                if range.min != range.max {
                    self.push(ChaosIssueKind::SharedRandomEventTime { min: range.min, max: range.max });
                }
                range
            }
        };
        let (class, _) = classify(&upper);
        if !matches!(class, TokenClass::ScalarEvent | TokenClass::VectorEvent | TokenClass::ColorEvent) {
            return Err(ChaosIssueKind::ExpectedEventProperty { token: upper });
        }
        let Some((name, kind)) = EVENT_KEYWORDS.into_iter().find(|(name, _)| upper.contains(name)) else {
            return Err(ChaosIssueKind::ExpectedEventProperty { token: upper });
        };
        self.keyword_match(word, name);
        let sequence = &mut builder.desc;
        if kind.moves_particle() && sequence.flags.contains(ChaosSequenceFlags::STAY_AT_EMITTER) {
            return Err(ChaosIssueKind::MotionEventWithStayAtEmitter { event: kind });
        }
        let components: Vec<Range> = match kind.components() {
            VECTOR_COMPONENTS => {
                let vector = self.vector(value, &upper)?;
                (0..VECTOR_COMPONENTS).map(|index| Range::new(vector.min[index], vector.max[index])).collect()
            }
            COLOR_COMPONENTS => self.color(value, &upper)?.to_vec(),
            _ => vec![self.range(value)?],
        };
        let factor = match kind {
            ChaosEventKind::Scale => builder.desc.scale_multiplier,
            ChaosEventKind::Velocity => builder.desc.velocity_multiplier,
            ChaosEventKind::Rotation => DEGREES_TO_RADIANS,
            _ => 1.0,
        };
        let mut event_value = ChaosEventValue::default();
        for (index, range) in components.iter().enumerate() {
            event_value.min[index] = range.min * factor;
            event_value.max[index] = range.max * factor;
        }
        let sequence = &mut builder.desc;
        match kind {
            ChaosEventKind::Thrust => sequence.flags.insert(ChaosSequenceFlags::THRUST),
            ChaosEventKind::Rotation => sequence.flags.insert(ChaosSequenceFlags::ROTATION),
            ChaosEventKind::EventTimer if event_value.min[0] <= time.max && event_value.max[0] >= time.min => {
                self.push(ChaosIssueKind::EventTimerLoop);
            }
            _ => {}
        }
        builder.desc.events.push(ChaosEventDesc { kind, fade, time, value: event_value });
        Ok(())
    }

    /// Sequence-level findings once all its lines are read.
    fn finish_sequence(&mut self, builder: &SequenceBuilder) {
        let sequence = &builder.desc;
        if !sequence.particle_type.is_rendered() || sequence.particle_type == ChaosParticleType::Emitter {
            self.push(ChaosIssueKind::NotRendered { particle_type: sequence.particle_type });
        }
        let mut events: Vec<&ChaosEventDesc> = sequence.events.iter().collect();
        events.sort_by(|a, b| a.time.min.partial_cmp(&b.time.min).unwrap_or(std::cmp::Ordering::Equal));
        for (index, event) in events.iter().enumerate() {
            let has_later_fade = events[index + 1..].iter().any(|later| later.kind == event.kind && later.fade);
            if event.fade
                && !has_later_fade
                && PERSISTING_FADE_KINDS.contains(&event.kind)
                && event.time.min > 0.0
                && event.time.min < sequence.lifetime.max
            {
                self.push(ChaosIssueKind::FadeStepPersists { event: event.kind });
            }
        }
    }

    // --- Values.

    fn scalar(&mut self, scalar: ChaosScalar) -> Range {
        match scalar {
            ChaosScalar::Number(number) => Range::constant(number as f32),
            ChaosScalar::Random { min, max } => {
                let range = Range::new(min as f32, max as f32);
                if range.min > range.max {
                    self.push(ChaosIssueKind::ReversedRandom { min: range.min, max: range.max });
                }
                range
            }
        }
    }

    /// `number | RANDOM(number, number)` (`ParseRange` 0x008b0840).
    fn range(&mut self, value: &ChaosValue) -> Result<Range, ChaosIssueKind> {
        match value {
            ChaosValue::Scalar(scalar) => Ok(self.scalar(*scalar)),
            ChaosValue::Call { name, args } if classify(&name.to_ascii_uppercase()).0 == TokenClass::Random => {
                match args.as_slice() {
                    [ChaosValue::Scalar(ChaosScalar::Number(min)), ChaosValue::Scalar(ChaosScalar::Number(max))] => {
                        self.keyword_match(name, "RANDOM");
                        Ok(self.scalar(ChaosScalar::Random { min: *min, max: *max }))
                    }
                    _ => Err(ChaosIssueKind::ExpectedNumber { token: value_text(value) }),
                }
            }
            _ => Err(ChaosIssueKind::ExpectedNumber { token: value_text(value) }),
        }
    }

    /// The ranges of the arguments of `XYZ(...)`, `RGBA(...)` or `CELSETUP(...)`.
    fn call_ranges<const N: usize>(
        &mut self,
        value: &ChaosValue,
        typed: Option<[ChaosScalar; N]>,
        function: TokenClass,
        keyword: &'static str,
    ) -> Option<[Range; N]> {
        if let Some(scalars) = typed {
            return Some(scalars.map(|scalar| self.scalar(scalar)));
        }
        let ChaosValue::Call { name, args } = value else {
            return None;
        };
        if classify(&name.to_ascii_uppercase()).0 != function || args.len() != N {
            return None;
        }
        self.keyword_match(name, keyword);
        let mut ranges = [Range::ZERO; N];
        for (range, arg) in ranges.iter_mut().zip(args) {
            *range = self.range(arg).ok()?;
        }
        Some(ranges)
    }

    /// `XYZ(x, y, z)` (`ParseVector` 0x008b0ea0).
    fn vector(&mut self, value: &ChaosValue, token: &str) -> Result<VecRange, ChaosIssueKind> {
        let typed = match value {
            ChaosValue::Vector(scalars) => Some(*scalars),
            _ => None,
        };
        let ranges = self
            .call_ranges::<VECTOR_COMPONENTS>(value, typed, TokenClass::Xyz, "XYZ")
            .ok_or_else(|| ChaosIssueKind::ExpectedVector { token: token.to_owned() })?;
        Ok(VecRange { min: ranges.map(|range| range.min), max: ranges.map(|range| range.max) })
    }

    /// `RGBA(r, g, b, a)` (`ParseColor` 0x008b1050).
    fn color(&mut self, value: &ChaosValue, token: &str) -> Result<[Range; COLOR_COMPONENTS], ChaosIssueKind> {
        let typed = match value {
            ChaosValue::Color(scalars) => Some(*scalars),
            _ => None,
        };
        self.call_ranges::<COLOR_COMPONENTS>(value, typed, TokenClass::Rgba, "RGBA")
            .ok_or_else(|| ChaosIssueKind::ExpectedColor { token: token.to_owned() })
    }

    /// `CELSETUP(frames, uStep, vStep, columns, rows, fps, loop)`; each argument may be a range, of which the
    /// client keeps the min (`ParseCelSetup` 0x008b0b20).
    fn cel_setup(&mut self, value: &ChaosValue) -> Result<ChaosCelSetup, ChaosIssueKind> {
        let typed = match value {
            ChaosValue::CelSetup(cel) => {
                Some([cel.cel_count, cel.cel_u, cel.cel_v, cel.columns, cel.rows, cel.fps, cel.looping].map(ChaosScalar::Number))
            }
            _ => None,
        };
        let [frames, u_step, v_step, columns, rows, fps, looping] = self
            .call_ranges::<CEL_SETUP_ARGUMENTS>(value, typed, TokenClass::CelSetup, "CELSETUP")
            .ok_or(ChaosIssueKind::ExpectedCelSetup)?
            .map(|range| range.min);
        Ok(ChaosCelSetup {
            frames: frames as i32 as u16,
            u_step,
            v_step,
            columns: columns as i32 as u8,
            rows: rows as i32 as u8,
            fps,
            looping: looping > 0.0,
        })
    }
}

/// `number | RANDOM(number, number)` read with `atol & 0xffff` (`ParseIntRange` 0x008b09b0).
fn int_range(value: &ChaosValue) -> Result<IntRange, ChaosIssueKind> {
    let atol = |number: f64| (number.trunc() as i64 as u32) & INT_RANGE_MASK;
    match value {
        ChaosValue::Scalar(ChaosScalar::Number(number)) => Ok(IntRange { min: atol(*number), max: atol(*number) }),
        ChaosValue::Scalar(ChaosScalar::Random { min, max }) => Ok(IntRange { min: atol(*min), max: atol(*max) }),
        ChaosValue::Call { name, args }
            if classify(&name.to_ascii_uppercase()).0 == TokenClass::Random && args.len() == RANDOM_ARGUMENTS =>
        {
            match args.as_slice() {
                [ChaosValue::Scalar(ChaosScalar::Number(min)), ChaosValue::Scalar(ChaosScalar::Number(max))] => {
                    Ok(IntRange { min: atol(*min), max: atol(*max) })
                }
                _ => Err(ChaosIssueKind::ExpectedNumber { token: value_text(value) }),
            }
        }
        _ => Err(ChaosIssueKind::ExpectedNumber { token: value_text(value) }),
    }
}

/// `ParseBool` (0x008b1260): a token with `1`, `T` or `Y` is true; otherwise one with `0`, `F` or `N` is false.
fn bool_value(value: &ChaosValue) -> Result<bool, ChaosIssueKind> {
    let text = value_text(value);
    if text.contains(BOOL_TRUE_CHARACTERS) {
        Ok(true)
    } else if text.contains(BOOL_FALSE_CHARACTERS) {
        Ok(false)
    } else {
        Err(ChaosIssueKind::InvalidBool { token: text })
    }
}

/// A quoted file name, reduced to its name and extension (`_splitpath`).
fn file_name(value: &ChaosValue, token: &str) -> Result<String, ChaosIssueKind> {
    match value {
        ChaosValue::FileName(name) => Ok(name.rsplit(PATH_SEPARATORS).next().unwrap_or(name).to_owned()),
        _ => Err(ChaosIssueKind::ExpectedFileName { token: token.to_owned() }),
    }
}

fn particle_type(value: &ChaosValue) -> Result<ChaosParticleType, ChaosIssueKind> {
    let text = value_text(value);
    PARTICLE_TYPES
        .into_iter()
        .find(|(name, _)| text.contains(name))
        .map(|(_, particle_type)| particle_type)
        .ok_or(ChaosIssueKind::UnknownParticleType { token: text })
}

/// `ParseBlendMode` (0x008b1430): exact `ALPHA_*` names.
fn blend_factor(value: &ChaosValue) -> Result<BlendFactor, ChaosIssueKind> {
    let text = value_text(value);
    BlendFactor::ALL
        .into_iter()
        .find(|factor| factor.script_name() == text)
        .ok_or(ChaosIssueKind::UnknownBlendMode { token: text })
}

/// The uppercase token the client would see first for `value`.
fn value_text(value: &ChaosValue) -> String {
    match value {
        ChaosValue::Scalar(ChaosScalar::Number(number)) => format!("{number}"),
        ChaosValue::Scalar(ChaosScalar::Random { .. }) => "RANDOM".to_owned(),
        ChaosValue::Vector(_) => "XYZ".to_owned(),
        ChaosValue::Color(_) => "RGBA".to_owned(),
        ChaosValue::CelSetup(_) => "CELSETUP".to_owned(),
        ChaosValue::Bool(true) => "TRUE".to_owned(),
        ChaosValue::Bool(false) => "FALSE".to_owned(),
        ChaosValue::BlendMode(mode) => mode.name().to_ascii_uppercase(),
        ChaosValue::ParticleType(particle_type) => particle_type.name().to_ascii_uppercase(),
        ChaosValue::FileName(name) => format!("\"{name}\""),
        ChaosValue::Ident(name) | ChaosValue::Call { name, .. } => name.to_ascii_uppercase(),
    }
}

/// The first word of a statement's time specifier, uppercase.
fn timing_word(statement: &ChaosStatement) -> String {
    match statement.timing {
        Some(ChaosTiming::Initial) => "INITIAL",
        Some(ChaosTiming::Final { fade: false }) => "FINAL",
        Some(ChaosTiming::At { fade: false, .. }) => "AT",
        Some(_) => "FADE",
        None => "",
    }
    .to_owned()
}

fn first_word(text: &str) -> String {
    text.split_whitespace().next().unwrap_or_default().to_ascii_uppercase()
}

fn convert_location(location: ChaosSourceLocation) -> SourceLocation {
    SourceLocation { line: location.line, column: location.column }
}

/// C `atof`: the longest prefix `[+-]digits[.digits][(e|E)[+-]digits]`, 0 without one.
fn c_atof(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let digits = |from: usize| bytes[from..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let integer = digits(end);
    end += integer;
    let mut fraction = 0;
    if bytes.get(end) == Some(&b'.') {
        fraction = digits(end + 1);
        end += 1 + fraction;
    }
    if integer + fraction == 0 {
        return 0.0;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits(end + 1 + sign);
        if exponent > 0 {
            end += 1 + sign + exponent;
        }
    }
    text[..end].parse().unwrap_or(0.0)
}
