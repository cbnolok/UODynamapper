//! Chaos particle scripts (`.ems` files of `EffectTexture.uop`): a typed syntax tree, a parser and two writers.
//!
//! The Enhanced Client plays a particle effect whose name contains `.ems` through the Chaos
//! `ParticleSystemManager`, which compiles the text script (`Emitter::CompileScript`). This module covers the
//! syntax only; what the properties do is not established here. The grammar (keywords from the UOSA.exe Chaos
//! keyword and error strings, forms from the 61 client scripts):
//!
//! ```text
//! script    := "particlesystem" STRING NUMBER "{" item* "}"
//! item      := statement | "eventsequence" STRING "{" statement* "}"
//! statement := lhs "=" value
//! lhs       := PROPERTY                                   // plain property
//!            | "initial" PROPERTY                         // events: initial value,
//!            | ["fade" "so"] "final" PROPERTY             //   value at the end of the particle life,
//!            | ["fade" "so"] "at" scalar PROPERTY         //   value at a time (seconds)
//! value     := scalar | "XYZ(" scalar "," scalar "," scalar ")" | "RGBA(" scalar "," scalar "," scalar "," scalar ")"
//!            | "CelSetup(" NUMBER x 7 ")" | IDENT | STRING
//! scalar    := NUMBER | "random(" NUMBER "," NUMBER ")"
//! ```
//!
//! - Keywords, properties and identifiers are case-insensitive; `//` starts a comment to the end of the line.
//!   Client files use CRLF line ends.
//! - Statements and sequences may interleave at system level (client scripts put `minEmitTime` before each
//!   sequence). A statement fits on one line, except inside parentheses.
//! - Identifier values: booleans `TRUE`/`FALSE`/`YES`/`NO` (the client also accepts 0 and 1), the
//!   [`ChaosBlendMode`] `ALPHA_*` names and the [`ChaosParticleType`] names. Strings are file names (no escapes).
//! - Numbers are digits with an optional sign and decimal point. Two client scripts have the malformed
//!   number `0.0.5`; it is read as its longest valid prefix, as C `atof` would, and reported.
//!
//! Nothing is lost: a statement whose left-hand side is not a known form keeps its text as an unknown key, a
//! call that is not one of the typed forms stays a generic [`ChaosValue::Call`], and a line that is not a
//! statement at all becomes a [`ChaosRaw`] item. [`ChaosScript::validate`] reports these.
//!
//! Writers:
//! - [`ChaosScript::to_lossless_string`] reproduces the parsed text byte for byte (whitespace, comments, case,
//!   number spelling) for every element whose content did not change, and regenerates only the edited or new
//!   elements in canonical form, with the indentation of their neighbours.
//! - [`ChaosScript::to_canonical_string`] pretty-prints the tree; parsing it gives an equal tree.
//!
//! Equality ([`PartialEq`]) compares content only, not source layout.

mod parser;
mod validation;
mod writer;

pub use validation::{ChaosIssue, ChaosIssueKind, ChaosIssueSeverity, ChaosValidationContext};

use std::fmt;

use color_eyre::eyre::{self, WrapErr};

/// File name extension of Chaos scripts.
pub const CHAOS_SCRIPT_EXTENSION: &str = "ems";

const KEYWORD_PARTICLE_SYSTEM: &str = "particlesystem";
const KEYWORD_EVENT_SEQUENCE: &str = "eventsequence";
const KEYWORD_INITIAL: &str = "initial";
const KEYWORD_FINAL: &str = "final";
const KEYWORD_FADE: &str = "fade";
const KEYWORD_SO: &str = "so";
const KEYWORD_AT: &str = "at";
const FUNCTION_RANDOM: &str = "random";
const FUNCTION_XYZ: &str = "XYZ";
const FUNCTION_RGBA: &str = "RGBA";
const FUNCTION_CEL_SETUP: &str = "CelSetup";
const BOOLEAN_TRUE_NAMES: [&str; 2] = ["TRUE", "YES"];
const BOOLEAN_FALSE_NAMES: [&str; 2] = ["FALSE", "NO"];
/// Line comment start.
const COMMENT_PREFIX: &str = "//";
/// Line end of the client scripts, used by the writers when the parsed text gives none.
const DEFAULT_NEWLINE: &str = "\r\n";
/// Indentation of one nesting level in generated text.
const INDENT: &str = "\t";

/// Number of values of `XYZ(...)`.
pub const CHAOS_VECTOR_COMPONENTS: usize = 3;
/// Number of values of `RGBA(...)`.
pub const CHAOS_COLOR_COMPONENTS: usize = 4;
/// Number of values of `CelSetup(...)`.
pub const CHAOS_CEL_SETUP_ARGUMENTS: usize = 7;

/// A parsed `.ems` script: `particlesystem "<name>" <version> { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosScript {
    /// The particle system name. The client looks a compiled system up by its uppercase name.
    pub name: String,
    /// The number after the name (client files: 0, 1, 2 or 4; meaning unknown).
    pub version: f64,
    /// System-level statements and the event sequences, in file order.
    pub items: Vec<ChaosItem>,
    source: ChaosSource,
}

/// An element of a block.
#[derive(Debug, Clone, PartialEq)]
pub enum ChaosItem {
    Statement(ChaosStatement),
    /// An `eventsequence` block; only valid at system level.
    Sequence(ChaosSequence),
    /// A line that is not a statement, kept verbatim.
    Raw(ChaosRaw),
}

/// `eventsequence "<name>" { ... }`: one particle emitter of the system.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosSequence {
    pub name: String,
    /// The statements of the sequence: properties first, then events (the client rejects a property after an
    /// event).
    pub items: Vec<ChaosItem>,
    source: ChaosSource,
}

/// `<lhs> = <value>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosStatement {
    /// `None` for a plain property, the time of an event otherwise.
    pub timing: Option<ChaosTiming>,
    pub key: ChaosKey,
    pub value: ChaosValue,
    source: ChaosSource,
}

/// A line kept verbatim because it is not a statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ChaosRaw {
    /// The line text without the line end and the trailing whitespace or comment.
    pub text: String,
    source: ChaosSource,
}

/// When an event statement applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChaosTiming {
    /// `initial <property>`: the value at particle birth.
    Initial,
    /// `[fade so] at <time> <property>`: the value at `time` seconds; `fade` interpolates towards it.
    At { fade: bool, time: ChaosScalar },
    /// `[fade so] final <property>`: the value at the end of the particle life; `fade` interpolates towards it.
    Final { fade: bool },
}

/// The property of a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaosKey {
    Known(ChaosProperty),
    /// A property name the client does not know, or a left-hand side that is no known form (its tokens with
    /// single spaces where the source had whitespace).
    Unknown(String),
}

/// A number or a `random(min, max)` range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChaosScalar {
    Number(f64),
    Random { min: f64, max: f64 },
}

/// A statement value.
#[derive(Debug, Clone, PartialEq)]
pub enum ChaosValue {
    Scalar(ChaosScalar),
    /// `XYZ(x, y, z)`.
    Vector([ChaosScalar; CHAOS_VECTOR_COMPONENTS]),
    /// `RGBA(r, g, b, a)`.
    Color([ChaosScalar; CHAOS_COLOR_COMPONENTS]),
    /// `CelSetup(...)`, the value of `celAnimate`.
    CelSetup(ChaosCelSetup),
    /// `TRUE`, `FALSE`, `YES` or `NO`.
    Bool(bool),
    BlendMode(ChaosBlendMode),
    ParticleType(ChaosParticleType),
    /// A quoted string: a file name under `data/effects/` (texture, mesh) or an emitter name.
    FileName(String),
    /// Any other identifier.
    Ident(String),
    /// Any other call, or a typed call with other arguments, as written.
    Call { name: String, args: Vec<ChaosValue> },
}

/// `CelSetup(cel_count, cel_u, cel_v, columns, rows, fps, looping)`: a texture atlas animation. The field names
/// follow the argument order; their meaning is not verified in the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaosCelSetup {
    pub cel_count: f64,
    pub cel_u: f64,
    pub cel_v: f64,
    pub columns: f64,
    pub rows: f64,
    pub fps: f64,
    pub looping: f64,
}

/// What kind of value a property takes (from the client data and the client's parse error messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChaosValueKind {
    Scalar,
    Vector,
    Color,
    /// A boolean identifier, or 0 / 1.
    Bool,
    BlendMode,
    ParticleType,
    FileName,
    CelSetup,
}

/// 1-based line and column (in characters) of an element in the parsed text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChaosSourceLocation {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ChaosSourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

macro_rules! keyword_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident => $text:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];

            /// The spelling the writers use.
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            /// The value of a case-insensitive name.
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|value| value.name().eq_ignore_ascii_case(name))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.name())
            }
        }
    };
}

keyword_enum! {
    /// The property names the client compiles (UOSA.exe Chaos keyword strings). Spellings follow the client
    /// files.
    ChaosProperty {
        Type => "type",
        Texture => "texture",
        MeshName => "meshName",
        EmitterName => "emitterName",
        SourceBlendMode => "sourceBlendMode",
        DestBlendMode => "destBlendMode",
        Length => "length",
        Rotation => "rotation",
        ChaosLocation => "chaosLocation",
        ChaosVelocity => "chaosVelocity",
        EmitRate => "emitRate",
        ScaleMultiplier => "scaleMultiplier",
        VelocityMultiplier => "velocityMultiplier",
        Lifetime => "lifetime",
        NumParticles => "numParticles",
        Loops => "loops",
        Gravity => "gravity",
        StayAtEmitter => "stayAtEmitter",
        Centered => "centered",
        Sort => "sort",
        Local => "local",
        MinEmitTime => "minEmitTime",
        LocationOnly => "locationOnly",
        Scale => "scale",
        EventTimer => "eventTimer",
        Color => "color",
        RedColor => "redColor",
        GreenColor => "greenColor",
        BlueColor => "blueColor",
        Alpha => "alpha",
        VelocityX => "velocityX",
        VelocityY => "velocityY",
        VelocityZ => "velocityZ",
        Velocity => "velocity",
        Thrust => "thrust",
        CelAnimate => "celAnimate",
        EmissionBoost => "emissionBoost",
        Burst => "burst",
        Inherited => "inherited",
        CollisionCheck => "collisionCheck",
        EmitRadius => "emitRadius",
    }
}

keyword_enum! {
    /// Alpha blending factors (`sourceBlendMode`, `destBlendMode`), the client's 11 `ALPHA_*` names.
    ChaosBlendMode {
        Zero => "ALPHA_ZERO",
        One => "ALPHA_ONE",
        SrcColor => "ALPHA_SRCCOLOR",
        InvSrcColor => "ALPHA_INVSRCCOLOR",
        SrcAlpha => "ALPHA_SRCALPHA",
        InvSrcAlpha => "ALPHA_INVSRCALPHA",
        DestAlpha => "ALPHA_DESTALPHA",
        InvDestAlpha => "ALPHA_INVDESTALPHA",
        DestColor => "ALPHA_DESTCOLOR",
        InvDestColor => "ALPHA_INVDESTCOLOR",
        SrcAlphaSat => "ALPHA_SRCALPHASAT",
    }
}

keyword_enum! {
    /// Particle types (`type`). Client files use point, axial and mesh.
    ChaosParticleType {
        Point => "POINT",
        Mesh => "MESH",
        Light => "LIGHT",
        Billboard => "BILLBOARD",
        Axial => "AXIAL",
        Emitter => "EMITTER",
        Test => "TEST",
    }
}

impl ChaosProperty {
    /// Whether the property can follow a time specifier (`initial`, `at`, `final`): the client's event list
    /// ("Expecting <property>!" messages, plus chaosLocation).
    pub fn is_event(self) -> bool {
        matches!(
            self,
            Self::Scale
                | Self::Color
                | Self::RedColor
                | Self::GreenColor
                | Self::BlueColor
                | Self::Alpha
                | Self::Velocity
                | Self::VelocityX
                | Self::VelocityY
                | Self::VelocityZ
                | Self::Thrust
                | Self::Rotation
                | Self::Length
                | Self::EventTimer
                | Self::ChaosLocation
        )
    }

    /// The value kind of the property; `None` when unknown (inherited, collisionCheck). Kinds of properties no
    /// client file uses are inferred from their names and the client's keyword groups.
    pub fn value_kind(self) -> Option<ChaosValueKind> {
        Some(match self {
            Self::Type => ChaosValueKind::ParticleType,
            Self::Texture | Self::MeshName | Self::EmitterName => ChaosValueKind::FileName,
            Self::SourceBlendMode | Self::DestBlendMode => ChaosValueKind::BlendMode,
            Self::Rotation | Self::ChaosLocation | Self::ChaosVelocity | Self::Gravity | Self::Velocity => {
                ChaosValueKind::Vector
            }
            Self::Color => ChaosValueKind::Color,
            Self::StayAtEmitter | Self::Centered | Self::Sort | Self::Local | Self::LocationOnly => {
                ChaosValueKind::Bool
            }
            Self::CelAnimate => ChaosValueKind::CelSetup,
            Self::Length
            | Self::EmitRate
            | Self::ScaleMultiplier
            | Self::VelocityMultiplier
            | Self::Lifetime
            | Self::NumParticles
            | Self::Loops
            | Self::MinEmitTime
            | Self::Scale
            | Self::EventTimer
            | Self::RedColor
            | Self::GreenColor
            | Self::BlueColor
            | Self::Alpha
            | Self::VelocityX
            | Self::VelocityY
            | Self::VelocityZ
            | Self::Thrust
            | Self::EmissionBoost
            | Self::Burst
            | Self::EmitRadius => ChaosValueKind::Scalar,
            Self::Inherited | Self::CollisionCheck => return None,
        })
    }

    /// Whether the property names a file of `data/effects/` (texture, meshName).
    pub fn is_file_reference(self) -> bool {
        matches!(self, Self::Texture | Self::MeshName)
    }
}

impl ChaosKey {
    /// The known property of a case-insensitive name, or an unknown key.
    pub fn from_name(name: &str) -> Self {
        ChaosProperty::from_name(name).map_or_else(|| Self::Unknown(name.to_owned()), Self::Known)
    }

    pub fn property(&self) -> Option<ChaosProperty> {
        match self {
            Self::Known(property) => Some(*property),
            Self::Unknown(_) => None,
        }
    }
}

impl fmt::Display for ChaosKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(property) => f.write_str(property.name()),
            Self::Unknown(text) => f.write_str(text),
        }
    }
}

impl ChaosValueKind {
    /// Whether `value` has this kind.
    pub fn accepts(self, value: &ChaosValue) -> bool {
        const BOOLEAN_NUMBERS: [f64; 2] = [0.0, 1.0];
        match (self, value) {
            (Self::Scalar, ChaosValue::Scalar(_))
            | (Self::Vector, ChaosValue::Vector(_))
            | (Self::Color, ChaosValue::Color(_))
            | (Self::Bool, ChaosValue::Bool(_))
            | (Self::BlendMode, ChaosValue::BlendMode(_))
            | (Self::ParticleType, ChaosValue::ParticleType(_))
            | (Self::FileName, ChaosValue::FileName(_))
            | (Self::CelSetup, ChaosValue::CelSetup(_)) => true,
            (Self::Bool, ChaosValue::Scalar(ChaosScalar::Number(number))) => BOOLEAN_NUMBERS.contains(number),
            _ => false,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Scalar => "a number or random(min,max)",
            Self::Vector => "XYZ(x,y,z)",
            Self::Color => "RGBA(r,g,b,a)",
            Self::Bool => "TRUE, FALSE, YES, NO, 0 or 1",
            Self::BlendMode => "an ALPHA_* blend mode",
            Self::ParticleType => "a particle type (POINT, MESH, LIGHT, BILLBOARD, AXIAL, EMITTER, TEST)",
            Self::FileName => "a quoted file name",
            Self::CelSetup => "CelSetup(...) with 7 numbers",
        }
    }
}

impl ChaosScript {
    /// An empty script, without source text.
    pub fn new(name: impl Into<String>, version: f64) -> Self {
        Self { name: name.into(), version, items: Vec::new(), source: ChaosSource::default() }
    }

    /// Parses a script. Errors name the line and column (1-based) of the offending token.
    pub fn parse(text: &str) -> eyre::Result<Self> {
        parser::parse_script(text)
    }

    /// Parses script bytes as Latin-1 (every byte one character), so any byte survives the lossless writer.
    /// Client scripts are ASCII.
    pub fn parse_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        Self::parse(&latin1_decode(bytes))
    }

    /// Reads a script file ([`Self::parse_bytes`]).
    pub fn load(path: impl AsRef<std::path::Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::parse_bytes(&bytes).wrap_err_with(|| format!("failed to parse {}", path.display()))
    }

    /// The text with every unchanged element as parsed and the changed or new ones in canonical form.
    pub fn to_lossless_string(&self) -> String {
        writer::write_script(self, writer::WriteMode::Lossless)
    }

    /// [`Self::to_lossless_string`] as Latin-1 bytes, the inverse of [`Self::parse_bytes`]. Fails when an edit
    /// introduced a character outside Latin-1.
    pub fn to_lossless_bytes(&self) -> eyre::Result<Vec<u8>> {
        latin1_encode(&self.to_lossless_string())
    }

    /// The canonical text: one statement per line, tab indentation, the parsed line end (CRLF by default).
    pub fn to_canonical_string(&self) -> String {
        writer::write_script(self, writer::WriteMode::Canonical)
    }

    /// [`Self::to_canonical_string`] as Latin-1 bytes.
    pub fn to_canonical_bytes(&self) -> eyre::Result<Vec<u8>> {
        latin1_encode(&self.to_canonical_string())
    }

    /// The event sequences, in file order.
    pub fn sequences(&self) -> impl Iterator<Item = &ChaosSequence> {
        self.items.iter().filter_map(|item| match item {
            ChaosItem::Sequence(sequence) => Some(sequence),
            _ => None,
        })
    }

    /// The first sequence with a case-insensitive name.
    pub fn sequence_mut(&mut self, name: &str) -> Option<&mut ChaosSequence> {
        self.items.iter_mut().find_map(|item| match item {
            ChaosItem::Sequence(sequence) if sequence.name.eq_ignore_ascii_case(name) => Some(sequence),
            _ => None,
        })
    }

    /// Every statement with the sequence holding it (`None` at system level), in file order.
    pub fn statements(&self) -> impl Iterator<Item = (Option<&ChaosSequence>, &ChaosStatement)> {
        self.items.iter().flat_map(|item| -> Box<dyn Iterator<Item = _> + '_> {
            match item {
                ChaosItem::Statement(statement) => Box::new(std::iter::once((None, statement))),
                ChaosItem::Sequence(sequence) => {
                    Box::new(sequence.statements().map(move |statement| (Some(sequence), statement)))
                }
                ChaosItem::Raw(_) => Box::new(std::iter::empty()),
            }
        })
    }

    /// The files of `data/effects/` the script names (texture and meshName values), in file order, repeated
    /// names included.
    pub fn referenced_files(&self) -> Vec<&str> {
        self.statements()
            .filter(|(_, statement)| statement.key.property().is_some_and(ChaosProperty::is_file_reference))
            .filter_map(|(_, statement)| match &statement.value {
                ChaosValue::FileName(name) => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Where the script header was in the parsed text.
    pub fn source_location(&self) -> Option<ChaosSourceLocation> {
        self.source.location()
    }
}

impl ChaosSequence {
    /// An empty sequence, without source text.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), items: Vec::new(), source: ChaosSource::default() }
    }

    /// The statements, in file order.
    pub fn statements(&self) -> impl Iterator<Item = &ChaosStatement> {
        self.items.iter().filter_map(|item| match item {
            ChaosItem::Statement(statement) => Some(statement),
            _ => None,
        })
    }

    /// The last plain statement (no timing) of a property: the one the client keeps if repeated.
    pub fn property(&self, property: ChaosProperty) -> Option<&ChaosStatement> {
        self.statements()
            .filter(|statement| statement.timing.is_none() && statement.key == ChaosKey::Known(property))
            .last()
    }

    pub fn source_location(&self) -> Option<ChaosSourceLocation> {
        self.source.location()
    }
}

impl ChaosStatement {
    /// A new statement, without source text.
    pub fn new(timing: Option<ChaosTiming>, key: ChaosKey, value: ChaosValue) -> Self {
        Self { timing, key, value, source: ChaosSource::default() }
    }

    /// A new plain property statement.
    pub fn property(property: ChaosProperty, value: ChaosValue) -> Self {
        Self::new(None, ChaosKey::Known(property), value)
    }

    pub fn source_location(&self) -> Option<ChaosSourceLocation> {
        self.source.location()
    }
}

impl ChaosRaw {
    /// A new raw line, without source text.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), source: ChaosSource::default() }
    }

    pub fn source_location(&self) -> Option<ChaosSourceLocation> {
        self.source.location()
    }
}

impl ChaosItem {
    fn source(&self) -> &ChaosSource {
        match self {
            Self::Statement(statement) => &statement.source,
            Self::Sequence(sequence) => &sequence.source,
            Self::Raw(raw) => &raw.source,
        }
    }

    pub fn source_location(&self) -> Option<ChaosSourceLocation> {
        self.source().location()
    }
}

/// How an element was written in the parsed text. Equality ignores it, so trees compare by content.
#[derive(Debug, Clone, Default)]
struct ChaosSource(Option<Box<SourceText>>);

impl PartialEq for ChaosSource {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl ChaosSource {
    fn location(&self) -> Option<ChaosSourceLocation> {
        self.0.as_ref().map(|source| source.location)
    }
}

/// The parsed text of an element. The text of a block is `leading`, `text` (its header), `before_brace`, `{`,
/// the items, `closing`, `}`, `trailing`; that of a statement or raw line is `leading`, `text`, `trailing`.
#[derive(Debug, Clone, Default)]
struct SourceText {
    /// Whitespace and comments before the element, from the end of the previous one.
    leading: String,
    /// The element text (a block's header: keyword, name and, for the system, version).
    text: String,
    /// The canonical form of `text` when parsed: an element whose canonical form still equals it is unchanged.
    canonical: String,
    /// Blocks: whitespace and comments between the header and `{`.
    before_brace: String,
    /// Blocks: whitespace and comments between the last item and `}`.
    closing: String,
    /// Spaces and a comment after the element on its line (the script: everything after its `}`).
    trailing: String,
    /// The script: the line end of the parsed text.
    newline: String,
    location: ChaosSourceLocation,
    /// Issues found while parsing the element (malformed numbers), reported while it is unchanged.
    issues: Vec<ChaosIssue>,
}

fn latin1_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

fn latin1_encode(text: &str) -> eyre::Result<Vec<u8>> {
    text.chars()
        .map(|character| {
            u8::try_from(u32::from(character))
                .map_err(|_| eyre::eyre!("character {character:?} is outside Latin-1 and cannot be written"))
        })
        .collect()
}

#[cfg(test)]
mod tests;
