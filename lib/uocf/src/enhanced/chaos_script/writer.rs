//! The lossless and canonical writers of Chaos scripts.

use super::{
    ChaosItem, ChaosScalar, ChaosScript, ChaosSequence, ChaosSource, ChaosStatement, ChaosTiming, ChaosValue,
    SourceText, BOOLEAN_FALSE_NAMES, BOOLEAN_TRUE_NAMES, DEFAULT_NEWLINE, FUNCTION_CEL_SETUP, FUNCTION_RANDOM,
    FUNCTION_RGBA, FUNCTION_XYZ, INDENT, KEYWORD_AT, KEYWORD_EVENT_SEQUENCE, KEYWORD_FADE, KEYWORD_FINAL,
    KEYWORD_INITIAL, KEYWORD_PARTICLE_SYSTEM, KEYWORD_SO,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteMode {
    /// Parsed text for unchanged elements, canonical text for the others.
    Lossless,
    /// Canonical text for everything.
    Canonical,
}

pub(super) fn write_script(script: &ChaosScript, mode: WriteMode) -> String {
    let newline = script.source.0.as_ref().map_or(DEFAULT_NEWLINE, |source| source.newline.as_str());
    let mut writer = Writer { out: String::new(), newline, mode };
    let header = script_header(script);
    match writer.source(&script.source) {
        Some(source) => {
            writer.push_header(source, &header);
            writer.out.push('{');
            writer.items(&script.items, INDENT);
            writer.out.push_str(&source.closing);
            writer.out.push('}');
            writer.out.push_str(&source.trailing);
        }
        None => {
            writer.generated_block(&header, &script.items, "");
            writer.out.push_str(newline);
        }
    }
    writer.out
}

/// The canonical header of the script: `particlesystem "<name>" <version>`.
pub(super) fn script_header(script: &ChaosScript) -> String {
    format!("{KEYWORD_PARTICLE_SYSTEM} \"{}\" {}", script.name, number(script.version))
}

/// The canonical header of a sequence: `eventsequence "<name>"`.
pub(super) fn sequence_header(sequence: &ChaosSequence) -> String {
    format!("{KEYWORD_EVENT_SEQUENCE} \"{}\"", sequence.name)
}

/// The canonical text of an item (a sequence: its header), without indentation.
pub(super) fn item_text(item: &ChaosItem) -> String {
    match item {
        ChaosItem::Statement(statement) => statement_text(statement),
        ChaosItem::Sequence(sequence) => sequence_header(sequence),
        ChaosItem::Raw(raw) => raw.text.clone(),
    }
}

/// Whether the element still has the content it was parsed with.
pub(super) fn is_unchanged(source: &SourceText, canonical: &str) -> bool {
    source.canonical == canonical
}

fn statement_text(statement: &ChaosStatement) -> String {
    let timing = match statement.timing {
        None => String::new(),
        Some(ChaosTiming::Initial) => format!("{KEYWORD_INITIAL} "),
        Some(ChaosTiming::At { fade, time }) => format!("{}{KEYWORD_AT} {} ", fade_prefix(fade), scalar(time)),
        Some(ChaosTiming::Final { fade }) => format!("{}{KEYWORD_FINAL} ", fade_prefix(fade)),
    };
    format!("{timing}{} = {}", statement.key, value(&statement.value))
}

fn fade_prefix(fade: bool) -> String {
    if fade { format!("{KEYWORD_FADE} {KEYWORD_SO} ") } else { String::new() }
}

fn value(chaos_value: &ChaosValue) -> String {
    match chaos_value {
        ChaosValue::Scalar(number) => scalar(*number),
        ChaosValue::Vector(components) => call(FUNCTION_XYZ, components.iter().map(|component| scalar(*component))),
        ChaosValue::Color(components) => call(FUNCTION_RGBA, components.iter().map(|component| scalar(*component))),
        ChaosValue::CelSetup(setup) => call(
            FUNCTION_CEL_SETUP,
            [setup.cel_count, setup.cel_u, setup.cel_v, setup.columns, setup.rows, setup.fps, setup.looping]
                .into_iter()
                .map(number),
        ),
        ChaosValue::Bool(flag) => {
            let names = if *flag { BOOLEAN_TRUE_NAMES } else { BOOLEAN_FALSE_NAMES };
            names[0].to_owned()
        }
        ChaosValue::BlendMode(mode) => mode.name().to_owned(),
        ChaosValue::ParticleType(particle_type) => particle_type.name().to_owned(),
        ChaosValue::FileName(name) => format!("\"{name}\""),
        ChaosValue::Ident(name) => name.clone(),
        ChaosValue::Call { name, args } => call(name, args.iter().map(value)),
    }
}

fn scalar(value: ChaosScalar) -> String {
    match value {
        ChaosScalar::Number(value) => number(value),
        ChaosScalar::Random { min, max } => call(FUNCTION_RANDOM, [min, max].into_iter().map(number)),
    }
}

fn call(name: &str, args: impl Iterator<Item = String>) -> String {
    format!("{name}({})", args.collect::<Vec<_>>().join(","))
}

/// The shortest decimal text that reads back as `value` (never an exponent).
fn number(value: f64) -> String {
    format!("{value}")
}

struct Writer<'a> {
    out: String,
    newline: &'a str,
    mode: WriteMode,
}

impl<'a> Writer<'a> {
    /// The parsed text of an element, when the mode uses it.
    fn source<'s>(&self, source: &'s ChaosSource) -> Option<&'s SourceText> {
        match self.mode {
            WriteMode::Lossless => source.0.as_deref(),
            WriteMode::Canonical => None,
        }
    }

    /// The leading text and the header (as parsed when unchanged) of a parsed element.
    fn push_header(&mut self, source: &SourceText, canonical: &str) {
        self.out.push_str(&source.leading);
        self.out.push_str(if is_unchanged(source, canonical) { &source.text } else { canonical });
        self.out.push_str(&source.before_brace);
    }

    /// The items of a parsed block; new items get the indentation of their neighbours, or `indent`.
    fn items(&mut self, items: &[ChaosItem], indent: &str) {
        for (index, item) in items.iter().enumerate() {
            match self.source(item.source()) {
                Some(source) => self.parsed_item(item, source, indent),
                None => {
                    let indent = neighbour_indent(items, index).unwrap_or_else(|| indent.to_owned());
                    self.out.push_str(self.newline);
                    self.generated_item(item, &indent);
                }
            }
        }
    }

    fn parsed_item(&mut self, item: &ChaosItem, source: &SourceText, indent: &str) {
        let canonical = item_text(item);
        match item {
            ChaosItem::Sequence(sequence) => {
                self.push_header(source, &canonical);
                self.out.push('{');
                let inner_indent = format!("{}{INDENT}", line_indent(&source.leading).unwrap_or(indent));
                self.items(&sequence.items, &inner_indent);
                self.out.push_str(&source.closing);
                self.out.push('}');
            }
            ChaosItem::Statement(_) | ChaosItem::Raw(_) => {
                self.out.push_str(&source.leading);
                self.out.push_str(if is_unchanged(source, &canonical) { &source.text } else { &canonical });
            }
        }
        self.out.push_str(&source.trailing);
    }

    /// A new item at `indent`, after its line end.
    fn generated_item(&mut self, item: &ChaosItem, indent: &str) {
        self.out.push_str(indent);
        match item {
            ChaosItem::Sequence(sequence) => self.generated_block(&sequence_header(sequence), &sequence.items, indent),
            ChaosItem::Statement(_) | ChaosItem::Raw(_) => self.out.push_str(&item_text(item)),
        }
    }

    /// `header`, then `{`, the items one level deeper and `}` on their own lines.
    fn generated_block(&mut self, header: &str, items: &[ChaosItem], indent: &str) {
        let inner_indent = format!("{indent}{INDENT}");
        self.out.push_str(header);
        self.out.push_str(self.newline);
        self.out.push_str(indent);
        self.out.push('{');
        for item in items {
            self.out.push_str(self.newline);
            self.generated_item(item, &inner_indent);
        }
        self.out.push_str(self.newline);
        self.out.push_str(indent);
        self.out.push('}');
    }
}

/// The indentation of the nearest parsed item before `index`, or else after it.
fn neighbour_indent(items: &[ChaosItem], index: usize) -> Option<String> {
    items[..index].iter().rev().chain(&items[index + 1..]).find_map(item_indent).map(str::to_owned)
}

/// The indentation of a parsed item.
fn item_indent(item: &ChaosItem) -> Option<&str> {
    item.source().0.as_deref().and_then(|source| line_indent(&source.leading))
}

/// The whitespace after the last line break of `leading`, when it is whitespace only.
fn line_indent(leading: &str) -> Option<&str> {
    let (_, last_line) = leading.rsplit_once('\n')?;
    last_line.chars().all(|character| character == ' ' || character == '\t').then_some(last_line)
}
