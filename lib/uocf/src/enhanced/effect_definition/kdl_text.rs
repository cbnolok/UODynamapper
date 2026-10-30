//! KDL (v1) text form of [`EffectDefinitionCollection`], for diffs and hand editing. It keeps everything the
//! package entries hold, in package order, so `bytes -> KDL -> bytes` is the identity.
//!
//! ```kdl
//! entry 0 {
//!     particle 110239 name="Placeholder01.ems"
//! }
//! entry 70003 {
//!     wait min=500 max=2000 {
//!         particle 1234 name="flash.nif" duration=0.5
//!         sound 43
//!     }
//! }
//! entry hash=0x0123456789ABCDEF {
//!     random {
//!         entry 60.0 { sound 1; }
//!         entry 40.0
//!     }
//! }
//! ```
//!
//! - `entry <id>` or `entry hash=<name hash>`: one per collection entry, in package order; its children are the
//!   definitions, played together.
//! - `particle <name index> [name=]`: the name index is the client string dictionary index. `name=` is
//!   informational: when a string dictionary is given it is written, and on reading it must match the index;
//!   with a dictionary the index may be left out and is then looked up by name. Properties with their default
//!   value are omitted: `offset-x=0.0 offset-y=0.0 camera-offset=0.0 scale=1.0 attach-mode=1 duration=0.0
//!   looping=false field-3c=0 hued=false`.
//! - `sound <sound id> [value-08=1.0]`.
//! - `round-robin { ... }`, `user-preference mask=<bits> { ... }`.
//! - `position-offset x0 y0 ... x7 y7`: the eight direction offsets.
//! - `wait min=<ms> max=<ms> [aligned=false] { ... }`.
//! - `lod { level <key> { ... } ... }`.
//! - `property-flag mode="flag-mask"|"property" [mask=0x0] [property=0] { default { ... } case <key> { ... } }`.
//! - `earthquake amplitude=<f64> duration=<f64> frequency=<f64>`.
//! - `random { entry <weight> { ... } ... }`.
//! - Floats are written with a decimal point (`1.0`); integers are accepted for them. NaN and infinities are the
//!   strings `"nan"`, `"inf"`, `"-inf"` (`"nan:0x<bits>"` for a NaN with other bits). Masks are written in hex.
//! - Errors name the line and column (1-based) of the offending node or value.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::ast::{Literal, Node, SpannedNode, Value};
use knuffel::span::LineSpan;

use super::collection::{EffectCollectionEntry, EffectDefinitionCollection, EffectEntryKey};
use super::{
    DirectionOffset, EarthquakeEffect, EffectDefinition, EffectMapEntry, LodEffect, ParticleEffect,
    PositionOffsetEffect, PropertyFlagEffect, PropertyFlagMode, RandomEffect, RandomEffectEntry, RoundRobinEffect,
    SoundEffect, UserPreferenceEffect, WaitEffect, MAX_EFFECT_NESTING_DEPTH, POSITION_OFFSET_DIRECTION_COUNT,
    STRING_DICTIONARY_INDEX_SHIFT,
};
use crate::enhanced::string_dictionary::UoStringDictionary;

const ENTRY_NODE: &str = "entry";
const HASH_PROPERTY: &str = "hash";
const PARTICLE_NODE: &str = "particle";
const SOUND_NODE: &str = "sound";
const ROUND_ROBIN_NODE: &str = "round-robin";
const POSITION_OFFSET_NODE: &str = "position-offset";
const WAIT_NODE: &str = "wait";
const LOD_NODE: &str = "lod";
const LEVEL_NODE: &str = "level";
const PROPERTY_FLAG_NODE: &str = "property-flag";
const DEFAULT_NODE: &str = "default";
const CASE_NODE: &str = "case";
const EARTHQUAKE_NODE: &str = "earthquake";
const RANDOM_NODE: &str = "random";
const RANDOM_ENTRY_NODE: &str = "entry";
const USER_PREFERENCE_NODE: &str = "user-preference";
const DEFINITION_NODES: [&str; 10] = [
    PARTICLE_NODE,
    SOUND_NODE,
    ROUND_ROBIN_NODE,
    POSITION_OFFSET_NODE,
    WAIT_NODE,
    LOD_NODE,
    PROPERTY_FLAG_NODE,
    EARTHQUAKE_NODE,
    RANDOM_NODE,
    USER_PREFERENCE_NODE,
];

const NAME_PROPERTY: &str = "name";
const OFFSET_X_PROPERTY: &str = "offset-x";
const OFFSET_Y_PROPERTY: &str = "offset-y";
const CAMERA_OFFSET_PROPERTY: &str = "camera-offset";
const SCALE_PROPERTY: &str = "scale";
const ATTACH_MODE_PROPERTY: &str = "attach-mode";
const DURATION_PROPERTY: &str = "duration";
const LOOPING_PROPERTY: &str = "looping";
const FIELD_3C_PROPERTY: &str = "field-3c";
const HUED_PROPERTY: &str = "hued";
const PARTICLE_PROPERTIES: [&str; 10] = [
    NAME_PROPERTY,
    OFFSET_X_PROPERTY,
    OFFSET_Y_PROPERTY,
    CAMERA_OFFSET_PROPERTY,
    SCALE_PROPERTY,
    ATTACH_MODE_PROPERTY,
    DURATION_PROPERTY,
    LOOPING_PROPERTY,
    FIELD_3C_PROPERTY,
    HUED_PROPERTY,
];
const VALUE_08_PROPERTY: &str = "value-08";
const MIN_PROPERTY: &str = "min";
const MAX_PROPERTY: &str = "max";
const ALIGNED_PROPERTY: &str = "aligned";
const MODE_PROPERTY: &str = "mode";
const MASK_PROPERTY: &str = "mask";
const PROPERTY_PROPERTY: &str = "property";
const AMPLITUDE_PROPERTY: &str = "amplitude";
const FREQUENCY_PROPERTY: &str = "frequency";
const MODE_FLAG_MASK: &str = "flag-mask";
const MODE_PROPERTY_VALUE: &str = "property";

/// Particle values the writer omits.
const DEFAULT_OFFSET: f32 = 0.0;
const DEFAULT_SCALE: f32 = 1.0;
const DEFAULT_ATTACH_MODE: u8 = 1;
const DEFAULT_DURATION: f32 = 0.0;
const DEFAULT_FIELD_3C: u8 = 0;
/// Sound `value_08` the writer omits.
const DEFAULT_SOUND_VALUE_08: f32 = 1.0;

/// Strings of the non-finite floats.
const NAN_TEXT: &str = "nan";
const INFINITY_TEXT: &str = "inf";
const NEGATIVE_INFINITY_TEXT: &str = "-inf";
/// Prefix of a NaN with other bits than the canonical one: `nan:0x<bits>`.
const NAN_BITS_PREFIX: &str = "nan:0x";
const HEX_RADIX: u32 = 16;

/// Name given to the KDL parser for its diagnostics.
const KDL_SOURCE_NAME: &str = "effects.kdl";
const INDENT: &str = "    ";
/// Values of `position-offset`: x and y per direction.
const POSITION_OFFSET_VALUES: usize = POSITION_OFFSET_DIRECTION_COUNT * 2;

const KDL_HEADER: &str = "\
// Effect definitions (EffectDefinitionCollection.uop), uocf::enhanced::effect_definition.
// entry <id> | entry hash=<hash>; particle <name index> [name=informational] ...; sound <id>; round-robin;
// position-offset <16 values>; wait min= max=; lod { level <k> }; property-flag mode= { default; case <k> };
// earthquake; random { entry <weight> }; user-preference mask=. Omitted properties have their default value.
";

impl EffectDefinitionCollection {
    /// The KDL text form (schema in the module docs), starting with a short comment header. With `strings`,
    /// particles get their informational `name=`.
    pub fn to_kdl_string(&self, strings: Option<&UoStringDictionary>) -> String {
        let mut writer = Writer { text: String::from(KDL_HEADER), strings };
        for entry in &self.entries {
            writer.text.push_str(ENTRY_NODE);
            match entry.key {
                EffectEntryKey::Id(id) => write!(writer.text, " {id}").unwrap(),
                EffectEntryKey::Hash(hash) => write!(writer.text, " {HASH_PROPERTY}={hash:#018X}").unwrap(),
            }
            writer.block(&entry.definitions, 0);
        }
        writer.text
    }

    /// Parses the KDL text form (schema in the module docs). `strings` checks informational particle names and
    /// resolves particles given by name only.
    pub fn from_kdl_str(text: &str, strings: Option<&UoStringDictionary>) -> eyre::Result<Self> {
        let document =
            knuffel::parse_ast::<LineSpan>(KDL_SOURCE_NAME, text).map_err(|error| syntax_error(text, &error))?;
        let mut reader = Reader { strings, indices: None };
        let entries = document
            .nodes
            .iter()
            .map(|node| reader.collection_entry(node))
            .collect::<eyre::Result<_>>()?;
        Ok(Self { entries })
    }

    /// Reads a KDL file ([`Self::from_kdl_str`]).
    pub fn load_kdl(path: impl AsRef<Path>, strings: Option<&UoStringDictionary>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::from_kdl_str(&text, strings).wrap_err_with(|| format!("failed to parse {}", path.display()))
    }

    /// Writes a KDL file ([`Self::to_kdl_string`]).
    pub fn write_kdl(&self, path: impl AsRef<Path>, strings: Option<&UoStringDictionary>) -> eyre::Result<()> {
        let path = path.as_ref();
        std::fs::write(path, self.to_kdl_string(strings))
            .wrap_err_with(|| format!("failed to write {}", path.display()))
    }
}

/// A definition list (a collection entry or a tileart record's effects) as KDL definition nodes, without the
/// header.
pub fn effect_definitions_to_kdl(definitions: &[EffectDefinition], strings: Option<&UoStringDictionary>) -> String {
    let mut writer = Writer { text: String::new(), strings };
    for definition in definitions {
        writer.definition(definition, 0);
    }
    writer.text
}

/// Parses KDL definition nodes ([`effect_definitions_to_kdl`]).
pub fn effect_definitions_from_kdl(
    text: &str,
    strings: Option<&UoStringDictionary>,
) -> eyre::Result<Vec<EffectDefinition>> {
    let document = knuffel::parse_ast::<LineSpan>(KDL_SOURCE_NAME, text).map_err(|error| syntax_error(text, &error))?;
    let mut reader = Reader { strings, indices: None };
    document.nodes.iter().map(|node| reader.definition(node, 0)).collect()
}

struct Writer<'a> {
    text: String,
    strings: Option<&'a UoStringDictionary>,
}

impl Writer<'_> {
    /// ` { children }` and the line end, or only the line end without children.
    fn block(&mut self, definitions: &[EffectDefinition], depth: usize) {
        if definitions.is_empty() {
            self.text.push('\n');
            return;
        }
        self.text.push_str(" {\n");
        for definition in definitions {
            self.definition(definition, depth + 1);
        }
        self.indent(depth);
        self.text.push_str("}\n");
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.text.push_str(INDENT);
        }
    }

    fn definition(&mut self, definition: &EffectDefinition, depth: usize) {
        self.indent(depth);
        match definition {
            EffectDefinition::Particle(effect) => {
                write!(self.text, "{PARTICLE_NODE} {}", effect.name_index).unwrap();
                if let Some(name) = self.strings.and_then(|strings| effect.name(strings)) {
                    write!(self.text, " {NAME_PROPERTY}={}", quoted(name)).unwrap();
                }
                self.f32_property(OFFSET_X_PROPERTY, effect.offset_x, DEFAULT_OFFSET);
                self.f32_property(OFFSET_Y_PROPERTY, effect.offset_y, DEFAULT_OFFSET);
                self.f32_property(CAMERA_OFFSET_PROPERTY, effect.camera_offset, DEFAULT_OFFSET);
                self.f32_property(SCALE_PROPERTY, effect.scale, DEFAULT_SCALE);
                if effect.attach_mode != DEFAULT_ATTACH_MODE {
                    write!(self.text, " {ATTACH_MODE_PROPERTY}={}", effect.attach_mode).unwrap();
                }
                self.f32_property(DURATION_PROPERTY, effect.duration, DEFAULT_DURATION);
                self.bool_property(LOOPING_PROPERTY, effect.looping);
                if effect.field_3c != DEFAULT_FIELD_3C {
                    write!(self.text, " {FIELD_3C_PROPERTY}={}", effect.field_3c).unwrap();
                }
                self.bool_property(HUED_PROPERTY, effect.hued);
                self.text.push('\n');
            }
            EffectDefinition::Sound(effect) => {
                write!(self.text, "{SOUND_NODE} {}", effect.sound_id).unwrap();
                self.f32_property(VALUE_08_PROPERTY, effect.value_08, DEFAULT_SOUND_VALUE_08);
                self.text.push('\n');
            }
            EffectDefinition::RoundRobin(effect) => {
                self.text.push_str(ROUND_ROBIN_NODE);
                self.block(&effect.children, depth);
            }
            EffectDefinition::PositionOffset(effect) => {
                self.text.push_str(POSITION_OFFSET_NODE);
                for offset in &effect.offsets {
                    write!(self.text, " {} {}", offset.x, offset.y).unwrap();
                }
                self.text.push('\n');
            }
            EffectDefinition::Wait(effect) => {
                write!(self.text, "{WAIT_NODE} {MIN_PROPERTY}={} {MAX_PROPERTY}={}", effect.min_delay_ms, effect.max_delay_ms)
                    .unwrap();
                self.bool_property(ALIGNED_PROPERTY, effect.aligned);
                self.block(&effect.children, depth);
            }
            EffectDefinition::Lod(effect) => {
                self.text.push_str(LOD_NODE);
                self.map(LEVEL_NODE, &effect.levels, depth);
            }
            EffectDefinition::PropertyFlag(effect) => {
                let mode = match effect.mode {
                    PropertyFlagMode::FlagMask => MODE_FLAG_MASK,
                    PropertyFlagMode::Property => MODE_PROPERTY_VALUE,
                };
                write!(self.text, "{PROPERTY_FLAG_NODE} {MODE_PROPERTY}={}", quoted(mode)).unwrap();
                if effect.mask != 0 {
                    write!(self.text, " {MASK_PROPERTY}={:#X}", effect.mask).unwrap();
                }
                if effect.property != 0 {
                    write!(self.text, " {PROPERTY_PROPERTY}={}", effect.property).unwrap();
                }
                if effect.default_children.is_empty() && effect.cases.is_empty() {
                    self.text.push('\n');
                    return;
                }
                self.text.push_str(" {\n");
                if !effect.default_children.is_empty() {
                    self.indent(depth + 1);
                    self.text.push_str(DEFAULT_NODE);
                    self.block(&effect.default_children, depth + 1);
                }
                for case in &effect.cases {
                    self.indent(depth + 1);
                    write!(self.text, "{CASE_NODE} {}", case.key).unwrap();
                    self.block(&case.children, depth + 1);
                }
                self.indent(depth);
                self.text.push_str("}\n");
            }
            EffectDefinition::Earthquake(effect) => {
                write!(
                    self.text,
                    "{EARTHQUAKE_NODE} {AMPLITUDE_PROPERTY}={} {DURATION_PROPERTY}={} {FREQUENCY_PROPERTY}={}",
                    f64_text(effect.amplitude),
                    f64_text(effect.duration),
                    f64_text(effect.frequency)
                )
                .unwrap();
                self.text.push('\n');
            }
            EffectDefinition::Random(effect) => {
                self.text.push_str(RANDOM_NODE);
                if effect.entries.is_empty() {
                    self.text.push('\n');
                    return;
                }
                self.text.push_str(" {\n");
                for entry in &effect.entries {
                    self.indent(depth + 1);
                    write!(self.text, "{RANDOM_ENTRY_NODE} {}", f32_text(entry.weight)).unwrap();
                    self.block(&entry.children, depth + 1);
                }
                self.indent(depth);
                self.text.push_str("}\n");
            }
            EffectDefinition::UserPreference(effect) => {
                write!(self.text, "{USER_PREFERENCE_NODE} {MASK_PROPERTY}={:#X}", effect.mask).unwrap();
                self.block(&effect.children, depth);
            }
        }
    }

    fn map(&mut self, node: &str, entries: &[EffectMapEntry], depth: usize) {
        if entries.is_empty() {
            self.text.push('\n');
            return;
        }
        self.text.push_str(" {\n");
        for entry in entries {
            self.indent(depth + 1);
            write!(self.text, "{node} {}", entry.key).unwrap();
            self.block(&entry.children, depth + 1);
        }
        self.indent(depth);
        self.text.push_str("}\n");
    }

    /// ` name=value` unless `value` has the bits of `default`.
    fn f32_property(&mut self, name: &str, value: f32, default: f32) {
        if value.to_bits() != default.to_bits() {
            write!(self.text, " {name}={}", f32_text(value)).unwrap();
        }
    }

    /// ` name=true` when set.
    fn bool_property(&mut self, name: &str, value: bool) {
        if value {
            write!(self.text, " {name}=true").unwrap();
        }
    }
}

fn quoted(text: &str) -> String {
    format!("{text:?}")
}

/// A float as KDL: a decimal with a point, or a string for NaN and the infinities.
fn float_text(display: String, is_nan: bool, canonical_nan: bool, bits: impl std::fmt::UpperHex, infinite: Option<bool>) -> String {
    if is_nan {
        return if canonical_nan { quoted(NAN_TEXT) } else { quoted(&format!("{NAN_BITS_PREFIX}{bits:X}")) };
    }
    match infinite {
        Some(true) => quoted(INFINITY_TEXT),
        Some(false) => quoted(NEGATIVE_INFINITY_TEXT),
        None if display.contains('.') => display,
        None => format!("{display}.0"),
    }
}

fn f32_text(value: f32) -> String {
    let infinite = value.is_infinite().then_some(value.is_sign_positive());
    float_text(value.to_string(), value.is_nan(), value.to_bits() == f32::NAN.to_bits(), value.to_bits(), infinite)
}

fn f64_text(value: f64) -> String {
    let infinite = value.is_infinite().then_some(value.is_sign_positive());
    float_text(value.to_string(), value.is_nan(), value.to_bits() == f64::NAN.to_bits(), value.to_bits(), infinite)
}

struct Reader<'a> {
    strings: Option<&'a UoStringDictionary>,
    /// Particle name -> name index, built on the first particle given by name only.
    indices: Option<HashMap<&'a str, u32>>,
}

type KdlNode = SpannedNode<LineSpan>;

impl<'a> Reader<'a> {
    fn collection_entry(&mut self, node: &KdlNode) -> eyre::Result<EffectCollectionEntry> {
        expect_node_name(node, ENTRY_NODE)?;
        check_properties(node, &[HASH_PROPERTY])?;
        let key = match (node.arguments.as_slice(), property(node, HASH_PROPERTY)) {
            ([id], None) => EffectEntryKey::Id(value_int(id, "entry id")?),
            ([], Some(hash)) => EffectEntryKey::Hash(value_int(hash, HASH_PROPERTY)?),
            _ => {
                return Err(error_at(node.node_name.span(), format!("`{ENTRY_NODE}` takes an id or {HASH_PROPERTY}=")));
            }
        };
        let definitions = self.children(node, 0)?;
        Ok(EffectCollectionEntry { key, definitions })
    }

    fn children(&mut self, node: &KdlNode, depth: usize) -> eyre::Result<Vec<EffectDefinition>> {
        node.children().map(|child| self.definition(child, depth)).collect()
    }

    fn definition(&mut self, node: &KdlNode, depth: usize) -> eyre::Result<EffectDefinition> {
        let name: &str = &node.node_name;
        reject_type_name(node)?;
        if depth > MAX_EFFECT_NESTING_DEPTH {
            return Err(error_at(node.node_name.span(), format!("nested deeper than {MAX_EFFECT_NESTING_DEPTH} levels")));
        }
        let child_depth = depth + 1;
        let definition = match name {
            PARTICLE_NODE => EffectDefinition::Particle(self.particle(node)?),
            SOUND_NODE => {
                check_properties(node, &[VALUE_08_PROPERTY])?;
                no_children(node)?;
                let [sound_id] = arguments::<1>(node)?;
                EffectDefinition::Sound(SoundEffect {
                    sound_id: value_int(sound_id, "sound id")?,
                    value_08: optional(node, VALUE_08_PROPERTY, value_f32, DEFAULT_SOUND_VALUE_08)?,
                })
            }
            ROUND_ROBIN_NODE => {
                check_properties(node, &[])?;
                arguments::<0>(node)?;
                EffectDefinition::RoundRobin(RoundRobinEffect { children: self.children(node, child_depth)? })
            }
            POSITION_OFFSET_NODE => {
                check_properties(node, &[])?;
                no_children(node)?;
                let values = arguments::<POSITION_OFFSET_VALUES>(node)?;
                let mut offsets = [DirectionOffset::default(); POSITION_OFFSET_DIRECTION_COUNT];
                for (offset, pair) in offsets.iter_mut().zip(values.chunks_exact(2)) {
                    offset.x = value_int(&pair[0], "x offset")?;
                    offset.y = value_int(&pair[1], "y offset")?;
                }
                EffectDefinition::PositionOffset(PositionOffsetEffect { offsets })
            }
            WAIT_NODE => {
                check_properties(node, &[MIN_PROPERTY, MAX_PROPERTY, ALIGNED_PROPERTY])?;
                arguments::<0>(node)?;
                EffectDefinition::Wait(WaitEffect {
                    min_delay_ms: value_int(required(node, MIN_PROPERTY)?, MIN_PROPERTY)?,
                    max_delay_ms: value_int(required(node, MAX_PROPERTY)?, MAX_PROPERTY)?,
                    aligned: optional(node, ALIGNED_PROPERTY, value_bool, false)?,
                    children: self.children(node, child_depth)?,
                })
            }
            LOD_NODE => {
                check_properties(node, &[])?;
                arguments::<0>(node)?;
                EffectDefinition::Lod(LodEffect { levels: self.map(node, LEVEL_NODE, child_depth)? })
            }
            PROPERTY_FLAG_NODE => EffectDefinition::PropertyFlag(self.property_flag(node, child_depth)?),
            EARTHQUAKE_NODE => {
                check_properties(node, &[AMPLITUDE_PROPERTY, DURATION_PROPERTY, FREQUENCY_PROPERTY])?;
                arguments::<0>(node)?;
                no_children(node)?;
                EffectDefinition::Earthquake(EarthquakeEffect {
                    amplitude: value_f64(required(node, AMPLITUDE_PROPERTY)?, AMPLITUDE_PROPERTY)?,
                    duration: value_f64(required(node, DURATION_PROPERTY)?, DURATION_PROPERTY)?,
                    frequency: value_f64(required(node, FREQUENCY_PROPERTY)?, FREQUENCY_PROPERTY)?,
                })
            }
            RANDOM_NODE => {
                check_properties(node, &[])?;
                arguments::<0>(node)?;
                let entries = node
                    .children()
                    .map(|child| {
                        expect_node_name(child, RANDOM_ENTRY_NODE)?;
                        check_properties(child, &[])?;
                        let [weight] = arguments::<1>(child)?;
                        Ok(RandomEffectEntry {
                            weight: value_f32(weight, "weight")?,
                            children: self.children(child, child_depth)?,
                        })
                    })
                    .collect::<eyre::Result<_>>()?;
                EffectDefinition::Random(RandomEffect { entries })
            }
            USER_PREFERENCE_NODE => {
                check_properties(node, &[MASK_PROPERTY])?;
                arguments::<0>(node)?;
                EffectDefinition::UserPreference(UserPreferenceEffect {
                    mask: value_int(required(node, MASK_PROPERTY)?, MASK_PROPERTY)?,
                    children: self.children(node, child_depth)?,
                })
            }
            _ => {
                let message = format!("unknown definition `{name}`: expected one of {}", DEFINITION_NODES.join(", "));
                return Err(error_at(node.node_name.span(), message));
            }
        };
        Ok(definition)
    }

    fn particle(&mut self, node: &KdlNode) -> eyre::Result<ParticleEffect> {
        check_properties(node, &PARTICLE_PROPERTIES)?;
        no_children(node)?;
        let name = property(node, NAME_PROPERTY)
            .map(|value| match &*value.literal {
                Literal::String(text) => Ok((value, &**text)),
                _ => Err(error_at(value.literal.span(), "name: expected a string")),
            })
            .transpose()?;
        let name_index = match (node.arguments.as_slice(), name) {
            ([index], _) => {
                let index: u32 = value_int(index, "name index")?;
                if let (Some(strings), Some((value, name))) = (self.strings, name) {
                    let expected = index.checked_sub(STRING_DICTIONARY_INDEX_SHIFT).and_then(|i| strings.get_string(i as usize));
                    if expected != Some(name) {
                        let expected = expected.map_or_else(|| "no string".to_owned(), quoted);
                        let message = format!("name {} does not match name index {index}: expected {expected}", quoted(name));
                        return Err(error_at(value.literal.span(), message));
                    }
                }
                index
            }
            ([], Some((value, name))) => self.name_index(name).ok_or_else(|| {
                let reason = if self.strings.is_some() { "not in the string dictionary" } else { "no string dictionary given" };
                error_at(value.literal.span(), format!("particle name {}: {reason}", quoted(name)))
            })?,
            _ => {
                return Err(error_at(
                    node.node_name.span(),
                    format!("`{PARTICLE_NODE}` takes a name index, or {NAME_PROPERTY}= with a string dictionary"),
                ));
            }
        };
        Ok(ParticleEffect {
            name_index,
            offset_x: optional(node, OFFSET_X_PROPERTY, value_f32, DEFAULT_OFFSET)?,
            offset_y: optional(node, OFFSET_Y_PROPERTY, value_f32, DEFAULT_OFFSET)?,
            camera_offset: optional(node, CAMERA_OFFSET_PROPERTY, value_f32, DEFAULT_OFFSET)?,
            scale: optional(node, SCALE_PROPERTY, value_f32, DEFAULT_SCALE)?,
            attach_mode: optional(node, ATTACH_MODE_PROPERTY, value_int, DEFAULT_ATTACH_MODE)?,
            duration: optional(node, DURATION_PROPERTY, value_f32, DEFAULT_DURATION)?,
            looping: optional(node, LOOPING_PROPERTY, value_bool, false)?,
            field_3c: optional(node, FIELD_3C_PROPERTY, value_int, DEFAULT_FIELD_3C)?,
            hued: optional(node, HUED_PROPERTY, value_bool, false)?,
        })
    }

    /// The name index of a particle name: the first string dictionary index holding it.
    fn name_index(&mut self, name: &str) -> Option<u32> {
        let strings = self.strings?;
        let indices = self.indices.get_or_insert_with(|| {
            let mut indices = HashMap::new();
            for (index, text) in strings.iter() {
                indices.entry(text).or_insert(index as u32 + STRING_DICTIONARY_INDEX_SHIFT);
            }
            indices
        });
        indices.get(name).copied()
    }

    fn property_flag(&mut self, node: &KdlNode, depth: usize) -> eyre::Result<PropertyFlagEffect> {
        check_properties(node, &[MODE_PROPERTY, MASK_PROPERTY, PROPERTY_PROPERTY])?;
        arguments::<0>(node)?;
        let mode_value = required(node, MODE_PROPERTY)?;
        let mode = match &*mode_value.literal {
            Literal::String(text) if &**text == MODE_FLAG_MASK => PropertyFlagMode::FlagMask,
            Literal::String(text) if &**text == MODE_PROPERTY_VALUE => PropertyFlagMode::Property,
            _ => {
                let message = format!("{MODE_PROPERTY}: expected \"{MODE_FLAG_MASK}\" or \"{MODE_PROPERTY_VALUE}\"");
                return Err(error_at(mode_value.literal.span(), message));
            }
        };
        let mut effect = PropertyFlagEffect {
            mode,
            mask: optional(node, MASK_PROPERTY, value_int::<u64>, 0)?,
            property: optional(node, PROPERTY_PROPERTY, value_int::<u32>, 0)?,
            default_children: Vec::new(),
            cases: Vec::new(),
        };
        let mut seen_default = false;
        for child in node.children() {
            let name: &str = &child.node_name;
            reject_type_name(child)?;
            check_properties(child, &[])?;
            match name {
                DEFAULT_NODE => {
                    if seen_default {
                        return Err(error_at(child.node_name.span(), format!("second `{DEFAULT_NODE}`")));
                    }
                    seen_default = true;
                    arguments::<0>(child)?;
                    effect.default_children = self.children(child, depth)?;
                }
                CASE_NODE => effect.cases.push(self.map_entry(child, depth)?),
                _ => {
                    let message = format!("unknown node `{name}` in {PROPERTY_FLAG_NODE}: expected {DEFAULT_NODE} or {CASE_NODE}");
                    return Err(error_at(child.node_name.span(), message));
                }
            }
        }
        Ok(effect)
    }

    fn map(&mut self, node: &KdlNode, entry_node: &str, depth: usize) -> eyre::Result<Vec<EffectMapEntry>> {
        node.children()
            .map(|child| {
                expect_node_name(child, entry_node)?;
                check_properties(child, &[])?;
                self.map_entry(child, depth)
            })
            .collect()
    }

    fn map_entry(&mut self, node: &KdlNode, depth: usize) -> eyre::Result<EffectMapEntry> {
        let [key] = arguments::<1>(node)?;
        Ok(EffectMapEntry { key: value_int(key, "key")?, children: self.children(node, depth)? })
    }
}

fn expect_node_name(node: &KdlNode, expected: &str) -> eyre::Result<()> {
    let name: &str = &node.node_name;
    if name != expected {
        return Err(error_at(node.node_name.span(), format!("unknown node `{name}`: expected `{expected}`")));
    }
    reject_type_name(node)
}

fn reject_type_name(node: &Node<LineSpan>) -> eyre::Result<()> {
    match &node.type_name {
        Some(type_name) => Err(error_at(type_name.span(), "type annotations are not supported")),
        None => Ok(()),
    }
}

fn no_children(node: &KdlNode) -> eyre::Result<()> {
    match node.children().next() {
        Some(child) => Err(error_at(child.node_name.span(), format!("`{}` takes no children", &*node.node_name))),
        None => Ok(()),
    }
}

fn check_properties(node: &Node<LineSpan>, allowed: &[&str]) -> eyre::Result<()> {
    let node_name: &str = &node.node_name;
    for key in node.properties.keys() {
        let key_name: &str = key;
        if !allowed.contains(&key_name) {
            let message = if allowed.is_empty() {
                format!("`{node_name}` takes no properties, found `{key_name}`")
            } else {
                format!("unknown property `{key_name}` of `{node_name}`: expected one of {}", allowed.join(", "))
            };
            return Err(error_at(key.span(), message));
        }
    }
    Ok(())
}

fn arguments<const N: usize>(node: &Node<LineSpan>) -> eyre::Result<&[Value<LineSpan>; N]> {
    let node_name: &str = &node.node_name;
    node.arguments.as_slice().try_into().map_err(|_| {
        let span = node.arguments.get(N).map_or(node.node_name.span(), |extra| extra.literal.span());
        error_at(span, format!("`{node_name}` takes {N} arguments, found {}", node.arguments.len()))
    })
}

fn property<'a>(node: &'a Node<LineSpan>, name: &str) -> Option<&'a Value<LineSpan>> {
    node.properties.iter().find(|(key, _)| key.as_ref() == name).map(|(_, value)| value)
}

fn required<'a>(node: &'a KdlNode, name: &str) -> eyre::Result<&'a Value<LineSpan>> {
    property(node, name)
        .ok_or_else(|| error_at(node.node_name.span(), format!("`{}`: missing {name}=", &*node.node_name)))
}

fn optional<T>(
    node: &KdlNode,
    name: &str,
    read: fn(&Value<LineSpan>, &str) -> eyre::Result<T>,
    default: T,
) -> eyre::Result<T> {
    property(node, name).map_or(Ok(default), |value| read(value, name))
}

/// The literal of a value without type annotation.
fn literal<'a>(value: &'a Value<LineSpan>) -> eyre::Result<&'a Literal> {
    match &value.type_name {
        Some(type_name) => Err(error_at(type_name.span(), "type annotations are not supported")),
        None => Ok(&value.literal),
    }
}

fn value_int<T>(value: &Value<LineSpan>, field: &str) -> eyre::Result<T>
where
    T: for<'i> TryFrom<&'i knuffel::ast::Integer>,
    for<'i> <T as TryFrom<&'i knuffel::ast::Integer>>::Error: std::fmt::Display,
{
    match literal(value)? {
        Literal::Int(integer) => T::try_from(integer).map_err(|error| {
            error_at(value.literal.span(), format!("{field}: out of range for {} ({error})", std::any::type_name::<T>()))
        }),
        _ => Err(error_at(value.literal.span(), format!("{field}: expected an integer"))),
    }
}

fn value_bool(value: &Value<LineSpan>, field: &str) -> eyre::Result<bool> {
    match literal(value)? {
        Literal::Bool(flag) => Ok(*flag),
        _ => Err(error_at(value.literal.span(), format!("{field}: expected true or false"))),
    }
}

fn value_f32(value: &Value<LineSpan>, field: &str) -> eyre::Result<f32> {
    value_float(value, field, f32::from_bits, |decimal| f32::try_from(decimal), |integer| integer as f32, f32::NAN, f32::INFINITY)
}

fn value_f64(value: &Value<LineSpan>, field: &str) -> eyre::Result<f64> {
    value_float(value, field, f64::from_bits, |decimal| f64::try_from(decimal), |integer| integer as f64, f64::NAN, f64::INFINITY)
}

/// A float: a decimal, an integer, or one of the non-finite strings.
fn value_float<F, B>(
    value: &Value<LineSpan>,
    field: &str,
    from_bits: fn(B) -> F,
    from_decimal: fn(&knuffel::ast::Decimal) -> Result<F, std::num::ParseFloatError>,
    from_integer: fn(i64) -> F,
    nan: F,
    infinity: F,
) -> eyre::Result<F>
where
    F: std::ops::Neg<Output = F>,
    B: TryFrom<u64>,
{
    let span = value.literal.span();
    match literal(value)? {
        Literal::Decimal(decimal) => from_decimal(decimal).map_err(|error| error_at(span, format!("{field}: {error}"))),
        Literal::Int(integer) => i64::try_from(integer)
            .map(from_integer)
            .map_err(|error| error_at(span, format!("{field}: {error}"))),
        Literal::String(text) => match &**text {
            NAN_TEXT => Ok(nan),
            INFINITY_TEXT => Ok(infinity),
            NEGATIVE_INFINITY_TEXT => Ok(-infinity),
            text => text
                .strip_prefix(NAN_BITS_PREFIX)
                .and_then(|bits| u64::from_str_radix(bits, HEX_RADIX).ok())
                .and_then(|bits| B::try_from(bits).ok())
                .map(from_bits)
                .ok_or_else(|| error_at(span, format!("{field}: expected a number, \"{NAN_TEXT}\" or \"{INFINITY_TEXT}\""))),
        },
        _ => Err(error_at(span, format!("{field}: expected a number"))),
    }
}

fn error_at(span: &LineSpan, message: impl std::fmt::Display) -> eyre::Report {
    eyre::eyre!("line {}, column {}: {message}", span.0.line + 1, span.0.column + 1)
}

/// The KDL syntax errors with their 1-based line and column.
fn syntax_error(text: &str, error: &knuffel::Error) -> eyre::Report {
    use miette::Diagnostic;

    let mut messages = Vec::new();
    for diagnostic in error.related().into_iter().flatten() {
        let labels: Vec<_> = diagnostic.labels().into_iter().flatten().collect();
        match labels.first() {
            Some(label) => {
                let (line, column) = line_column(text, label.offset());
                let detail = label.label().map(|label| format!(" ({label})")).unwrap_or_default();
                messages.push(format!("line {line}, column {column}: {diagnostic}{detail}"));
            }
            None => messages.push(diagnostic.to_string()),
        }
    }
    if messages.is_empty() {
        messages.push(error.to_string());
    }
    eyre::eyre!("invalid KDL: {}", messages.join("; "))
}

/// 1-based line and column (in characters) of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    (before.matches('\n').count() + 1, before[line_start..].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::encode::{encode_effect_definitions, every_definition_type};

    const ENTRY_HASH: u64 = 0x0123_4567_89AB_CDEF;

    fn sample() -> EffectDefinitionCollection {
        EffectDefinitionCollection {
            entries: vec![
                EffectCollectionEntry { key: EffectEntryKey::Id(0), definitions: every_definition_type() },
                EffectCollectionEntry { key: EffectEntryKey::Hash(ENTRY_HASH), definitions: vec![] },
            ],
        }
    }

    fn payloads(collection: &EffectDefinitionCollection) -> Vec<(EffectEntryKey, Vec<u8>)> {
        collection.entries.iter().map(|entry| (entry.key, encode_effect_definitions(&entry.definitions))).collect()
    }

    fn parse_error(text: &str) -> String {
        format!("{:#}", EffectDefinitionCollection::from_kdl_str(text, None).unwrap_err())
    }

    #[test]
    fn round_trips_through_kdl() {
        let collection = sample();
        let text = collection.to_kdl_string(None);
        let parsed = EffectDefinitionCollection::from_kdl_str(&text, None).unwrap();
        assert_eq!(payloads(&parsed), payloads(&collection), "{text}");
        assert_eq!(parsed.to_kdl_string(None), text);
    }

    #[test]
    fn writes_the_compact_form() {
        let text = sample().to_kdl_string(None);
        let body = text.strip_prefix(KDL_HEADER).unwrap();
        let expected = "\
entry 0 {
    particle 110239 offset-x=-8.5 offset-y=-0.0 camera-offset=0.0000001 scale=0.25 attach-mode=2 duration=3.0 looping=true hued=true
    particle 110239 offset-x=-8.5 camera-offset=0.0000001
    sound 43 value-08=\"nan\"
    round-robin {
        sound 1
        sound 2
    }
    position-offset 0 0 -128 127 0 0 0 0 0 0 0 0 0 0 0 0
    wait min=500 max=2000 aligned=true {
        sound 3
    }
    lod {
        level 1 {
            sound 4
        }
        level 3
    }
    property-flag mode=\"property\" mask=0x8000000000000040 property=7 {
        default {
            sound 5
        }
        case 33 {
            sound 6
        }
    }
    property-flag mode=\"flag-mask\"
    earthquake amplitude=0.1 duration=\"inf\" frequency=12.0
    random {
        entry 33.3 {
            sound 7
        }
        entry 66.7
    }
    random
    user-preference mask=0x1 {
        sound 8
    }
}
entry hash=0x0123456789ABCDEF
";
        assert_eq!(body, expected);
    }

    #[test]
    fn accepts_integers_for_floats_and_any_property_order() {
        let text = "entry 5 { particle 9 duration=2 scale=1; random { entry 100 { sound 1; }; }; }";
        let parsed = EffectDefinitionCollection::from_kdl_str(text, None).unwrap();
        let EffectDefinition::Particle(particle) = &parsed.entries[0].definitions[0] else { panic!("not a particle") };
        assert_eq!((particle.duration, particle.scale), (2.0, 1.0));
    }

    #[test]
    fn errors_name_line_and_column() {
        let cases = [
            ("entry 1 {\n    bogus 1\n}", "line 2, column 5", "unknown definition `bogus`"),
            ("entry 1 {\n    sound 1 volume=2\n}", "line 2, column 13", "unknown property `volume`"),
            ("entry 1 {\n    wait max=1\n}", "line 2, column 5", "missing min="),
            ("entry 1 {\n    particle 1 attach-mode=300\n}", "line 2, column 28", "out of range"),
            ("entry 1 {\n    particle name=\"x.nif\"\n}", "line 2, column 19", "no string dictionary given"),
            ("entry 1 {\n    property-flag mode=\"other\"\n}", "line 2, column 24", "expected \"flag-mask\""),
            ("entry {", "line 1", "invalid KDL"),
            ("entry 1 hash=2", "line 1, column 1", "takes an id or hash="),
        ];
        for (text, location, message) in cases {
            let error = parse_error(text);
            assert!(error.contains(location) && error.contains(message), "{text:?}: {error}");
        }
    }

    fn dictionary(strings: &[&str]) -> UoStringDictionary {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for value in strings {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }
        UoStringDictionary::from_payload_bytes(&payload, "test dictionary").unwrap()
    }

    #[test]
    fn particle_names_use_the_string_dictionary() {
        let strings = dictionary(&["first.nif", "second.ems"]);
        let text = "entry 1 {\n    particle 2 name=\"second.ems\"\n}\n";
        let parsed = EffectDefinitionCollection::from_kdl_str(text, Some(&strings)).unwrap();
        assert_eq!(parsed.to_kdl_string(Some(&strings)).strip_prefix(KDL_HEADER), Some(text));

        let by_name = EffectDefinitionCollection::from_kdl_str("entry 1 { particle name=\"second.ems\"; }", Some(&strings));
        assert_eq!(by_name.unwrap(), parsed);

        let mismatch = EffectDefinitionCollection::from_kdl_str("entry 1 { particle 1 name=\"second.ems\"; }", Some(&strings));
        assert!(format!("{:#}", mismatch.unwrap_err()).contains("expected \"first.nif\""));
    }

    #[test]
    fn definition_lists_round_trip() {
        let definitions = every_definition_type();
        let text = effect_definitions_to_kdl(&definitions, None);
        let parsed = effect_definitions_from_kdl(&text, None).unwrap();
        assert_eq!(encode_effect_definitions(&parsed), encode_effect_definitions(&definitions));
    }
}
