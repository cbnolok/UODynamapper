//! Checks of a Chaos script against what the client's compiler accepts.

use std::fmt;

use super::writer;
use super::{
    ChaosItem, ChaosKey, ChaosProperty, ChaosScalar, ChaosScript, ChaosSequence, ChaosSourceLocation,
    ChaosStatement, ChaosTiming, ChaosValue, ChaosValueKind, SourceText,
};

/// What the validation can look up.
#[derive(Default, Clone, Copy)]
pub struct ChaosValidationContext<'a> {
    /// Whether a file named by a `texture` or `meshName` value exists. Called with the name as written in the
    /// script, a file of `data/effects/` (see `effect_texture::effect_texture_path`). `None` skips the check.
    pub file_exists: Option<&'a dyn Fn(&str) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChaosIssueSeverity {
    /// The client accepts it, but it is likely a mistake.
    Warning,
    /// The client rejects the script, or the element cannot be written.
    Error,
}

/// One validation finding.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosIssue {
    /// Where the element is in the parsed text; `None` for an element added after parsing.
    pub location: Option<ChaosSourceLocation>,
    /// The sequence holding the element; `None` at system level.
    pub sequence: Option<String>,
    pub kind: ChaosIssueKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChaosIssueKind {
    /// A number with more than one decimal point; the client reads its longest valid prefix (`value`).
    MalformedNumber { text: String, value: f64 },
    /// The particle system or a sequence has no name ("Must name particle system").
    EmptyName,
    /// A property name the client does not know, or a left-hand side that is no known statement form.
    UnknownProperty { key: String },
    /// A line kept verbatim because it is not a statement.
    UnparsedLine { text: String },
    /// A value of the wrong kind for its property.
    UnexpectedValue { property: ChaosProperty, expected: ChaosValueKind },
    /// A time specifier on a property that is not an event ("Fading is not supported on this event." and the
    /// "Expecting <event>" messages).
    NotAnEvent { property: ChaosProperty },
    /// An event outside an eventsequence.
    EventOutsideSequence { property: ChaosProperty },
    /// A sequence property after an event ("Cannot specify any sequence properties after specifying events.").
    PropertyAfterEvent { property: ChaosProperty },
    /// A string with a quote or line break, which the script syntax cannot hold.
    InvalidString { text: String },
    /// A NaN or infinite number, which the script syntax cannot hold.
    NonFiniteNumber,
    /// A texture or mesh file that is not in the package.
    MissingFile { property: ChaosProperty, name: String },
}

impl ChaosIssue {
    pub fn severity(&self) -> ChaosIssueSeverity {
        match self.kind {
            ChaosIssueKind::MalformedNumber { .. }
            | ChaosIssueKind::UnknownProperty { .. }
            | ChaosIssueKind::EventOutsideSequence { .. } => ChaosIssueSeverity::Warning,
            ChaosIssueKind::EmptyName
            | ChaosIssueKind::UnparsedLine { .. }
            | ChaosIssueKind::UnexpectedValue { .. }
            | ChaosIssueKind::NotAnEvent { .. }
            | ChaosIssueKind::PropertyAfterEvent { .. }
            | ChaosIssueKind::InvalidString { .. }
            | ChaosIssueKind::NonFiniteNumber
            | ChaosIssueKind::MissingFile { .. } => ChaosIssueSeverity::Error,
        }
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
        match &self.kind {
            ChaosIssueKind::MalformedNumber { text, value } => {
                write!(f, "malformed number `{text}`, read as {value}")
            }
            ChaosIssueKind::EmptyName => f.write_str("empty name"),
            ChaosIssueKind::UnknownProperty { key } => write!(f, "unknown property `{key}`"),
            ChaosIssueKind::UnparsedLine { text } => write!(f, "not a statement: `{text}`"),
            ChaosIssueKind::UnexpectedValue { property, expected } => {
                write!(f, "{property} expects {}", expected.description())
            }
            ChaosIssueKind::NotAnEvent { property } => write!(f, "{property} is not an event: it takes no time"),
            ChaosIssueKind::EventOutsideSequence { property } => {
                write!(f, "event {property} outside an eventsequence")
            }
            ChaosIssueKind::PropertyAfterEvent { property } => {
                write!(f, "property {property} after an event (the client rejects it)")
            }
            ChaosIssueKind::InvalidString { text } => write!(f, "string {text:?} has a quote or a line break"),
            ChaosIssueKind::NonFiniteNumber => f.write_str("NaN or infinite number"),
            ChaosIssueKind::MissingFile { property, name } => write!(f, "{property} file \"{name}\" not found"),
        }
    }
}

impl ChaosScript {
    /// Validates the script. Issues found while parsing (malformed numbers) are reported for the elements that
    /// did not change since.
    pub fn validate(&self, context: &ChaosValidationContext<'_>) -> Vec<ChaosIssue> {
        let mut validator = Validator { context, issues: Vec::new() };
        let location = self.source.location();
        if let Some(source) = self.source.0.as_deref() {
            validator.parse_issues(source, &writer::script_header(self), None);
        }
        if self.name.is_empty() {
            validator.push(location, None, ChaosIssueKind::EmptyName);
        }
        validator.string(location, None, &self.name);
        if !self.version.is_finite() {
            validator.push(location, None, ChaosIssueKind::NonFiniteNumber);
        }
        for item in &self.items {
            match item {
                ChaosItem::Sequence(sequence) => validator.sequence(sequence),
                item => validator.item(item, None, &mut false),
            }
        }
        validator.issues
    }
}

struct Validator<'c, 'a> {
    context: &'c ChaosValidationContext<'a>,
    issues: Vec<ChaosIssue>,
}

impl Validator<'_, '_> {
    fn push(&mut self, location: Option<ChaosSourceLocation>, sequence: Option<&str>, kind: ChaosIssueKind) {
        self.issues.push(ChaosIssue { location, sequence: sequence.map(str::to_owned), kind });
    }

    fn parse_issues(&mut self, source: &SourceText, canonical: &str, sequence: Option<&str>) {
        if writer::is_unchanged(source, canonical) {
            self.issues.extend(source.issues.iter().map(|issue| ChaosIssue {
                sequence: sequence.map(str::to_owned),
                ..issue.clone()
            }));
        }
    }

    fn sequence(&mut self, sequence: &ChaosSequence) {
        let location = sequence.source.location();
        let name = Some(sequence.name.as_str());
        if sequence.name.is_empty() {
            self.push(location, name, ChaosIssueKind::EmptyName);
        }
        self.string(location, name, &sequence.name);
        let mut seen_event = false;
        for item in &sequence.items {
            self.item(item, name, &mut seen_event);
        }
    }

    /// A statement or raw line; `seen_event` tracks the events of a sequence.
    fn item(&mut self, item: &ChaosItem, sequence: Option<&str>, seen_event: &mut bool) {
        let location = item.source().location();
        if let Some(source) = item.source().0.as_deref() {
            self.parse_issues(source, &writer::item_text(item), sequence);
        }
        match item {
            ChaosItem::Statement(statement) => self.statement(statement, location, sequence, seen_event),
            ChaosItem::Raw(raw) => self.push(location, sequence, ChaosIssueKind::UnparsedLine { text: raw.text.clone() }),
            ChaosItem::Sequence(nested) => self.sequence(nested),
        }
    }

    fn statement(
        &mut self,
        statement: &ChaosStatement,
        location: Option<ChaosSourceLocation>,
        sequence: Option<&str>,
        seen_event: &mut bool,
    ) {
        if let Some(ChaosTiming::At { time, .. }) = statement.timing {
            self.scalar(location, sequence, time);
        }
        self.value(location, sequence, &statement.value);
        let property = match &statement.key {
            ChaosKey::Known(property) => *property,
            ChaosKey::Unknown(key) => {
                self.push(location, sequence, ChaosIssueKind::UnknownProperty { key: key.clone() });
                return;
            }
        };
        if let Some(expected) = property.value_kind()
            && !expected.accepts(&statement.value)
        {
            self.push(location, sequence, ChaosIssueKind::UnexpectedValue { property, expected });
        }
        match statement.timing {
            Some(_) if !property.is_event() => self.push(location, sequence, ChaosIssueKind::NotAnEvent { property }),
            Some(_) if sequence.is_none() => {
                self.push(location, sequence, ChaosIssueKind::EventOutsideSequence { property })
            }
            Some(_) => *seen_event = true,
            None if *seen_event => self.push(location, sequence, ChaosIssueKind::PropertyAfterEvent { property }),
            None => {}
        }
        if property.is_file_reference()
            && let ChaosValue::FileName(name) = &statement.value
            && let Some(file_exists) = self.context.file_exists
            && !file_exists(name)
        {
            self.push(location, sequence, ChaosIssueKind::MissingFile { property, name: name.clone() });
        }
    }

    fn value(&mut self, location: Option<ChaosSourceLocation>, sequence: Option<&str>, value: &ChaosValue) {
        let scalars: &[ChaosScalar] = match value {
            ChaosValue::Scalar(scalar) => std::slice::from_ref(scalar),
            ChaosValue::Vector(components) => components,
            ChaosValue::Color(components) => components,
            ChaosValue::CelSetup(setup) => {
                let numbers =
                    [setup.cel_count, setup.cel_u, setup.cel_v, setup.columns, setup.rows, setup.fps, setup.looping];
                if !numbers.iter().all(|number| number.is_finite()) {
                    self.push(location, sequence, ChaosIssueKind::NonFiniteNumber);
                }
                &[]
            }
            ChaosValue::FileName(text) => {
                self.string(location, sequence, text);
                &[]
            }
            ChaosValue::Call { args, .. } => {
                for arg in args {
                    self.value(location, sequence, arg);
                }
                &[]
            }
            ChaosValue::Bool(_) | ChaosValue::BlendMode(_) | ChaosValue::ParticleType(_) | ChaosValue::Ident(_) => &[],
        };
        for scalar in scalars {
            self.scalar(location, sequence, *scalar);
        }
    }

    fn scalar(&mut self, location: Option<ChaosSourceLocation>, sequence: Option<&str>, scalar: ChaosScalar) {
        let finite = match scalar {
            ChaosScalar::Number(number) => number.is_finite(),
            ChaosScalar::Random { min, max } => min.is_finite() && max.is_finite(),
        };
        if !finite {
            self.push(location, sequence, ChaosIssueKind::NonFiniteNumber);
        }
    }

    fn string(&mut self, location: Option<ChaosSourceLocation>, sequence: Option<&str>, text: &str) {
        if text.contains(['"', '\n', '\r']) {
            self.push(location, sequence, ChaosIssueKind::InvalidString { text: text.to_owned() });
        }
    }
}
