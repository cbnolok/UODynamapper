//! KDL (v1) text form of animation definitions, for diffs and hand editing. It keeps everything an entry holds, so
//! `bytes -> KDL -> bytes` is the identity.
//!
//! ```kdl
//! animation 7905 stored-id=7897 second=1 {
//!     set 0 interval=0.25 next=1 {
//!         frames 0x1EDD 0x1EDE 0x1EDF
//!         frame 0x1EE0 {
//!             sound 42
//!         }
//!     }
//!     set 1 interval=0.0625 {
//!         frames 0x1EE1
//!     }
//! }
//! ```
//!
//! - `animation <item id>`: one per entry (`build/animationdefinition/<item id>.bin`). `stored-id=` defaults to the
//!   item id, `start=` (the start set) to 0, `second=` (the second set) to the start set, `value=` to 0.0; the
//!   writer omits them at their default.
//! - `set <key> interval=<seconds> [next=<key>]`: one per frame set, in any order (the binary form sorts them by
//!   key); without `next=` the set has no successor.
//! - `frames <tile id> ...`: frames without effects, in order; `frame <tile id> { effects }`: one frame whose
//!   effects are the children, in the `EffectDefinitionCollection` KDL grammar
//!   (`effect_definition/kdl_text.rs`). The writer groups runs of frames without effects into `frames` lines.
//! - Tile ids are written in hex, the rest in decimal; integers may be either. Floats are written with a decimal
//!   point; integers are accepted for them. NaN and the infinities are the strings `"nan"`, `"inf"`, `"-inf"`.
//! - The reader is strict: unknown nodes and properties are errors naming the allowed ones; so are a repeated item
//!   id or set key, start, second or next keys naming no set, a next above `i32::MAX`, a NaN, infinite or negative
//!   interval, a NaN or negative `value`, and more than 255 effects on a frame (what the writer refuses). Errors
//!   name the line and column (1-based).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use uot_kdl::ast::{Node, SpannedNode, Value};
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{
    arguments, check_properties, describe_error, error_at, in_file, property, reject_children, required_property,
    unknown_node, value_u32,
};

use super::{AnimationDefinition, DefinitionFrame, FrameSet, FRAME_EFFECT_COUNT_MAX};
use crate::enhanced::effect_definition::{effect_definitions_from_nodes, f32_text, value_f32, write_effect_definition_nodes};
use crate::enhanced::string_dictionary::UoStringDictionary;

type KdlNode = SpannedNode<LineSpan>;

const ANIMATION_NODE: &str = "animation";
const SET_NODE: &str = "set";
const FRAMES_NODE: &str = "frames";
const FRAME_NODE: &str = "frame";
const STORED_ID_PROPERTY: &str = "stored-id";
const START_PROPERTY: &str = "start";
const SECOND_PROPERTY: &str = "second";
const VALUE_PROPERTY: &str = "value";
const INTERVAL_PROPERTY: &str = "interval";
const NEXT_PROPERTY: &str = "next";
const ANIMATION_PROPERTIES: [&str; 4] = [STORED_ID_PROPERTY, START_PROPERTY, SECOND_PROPERTY, VALUE_PROPERTY];
const SET_PROPERTIES: [&str; 2] = [INTERVAL_PROPERTY, NEXT_PROPERTY];
const SET_CHILDREN: [&str; 2] = [FRAMES_NODE, FRAME_NODE];
/// Start set and `value` the writer omits.
const DEFAULT_START_SET: u32 = 0;
const DEFAULT_VALUE: f32 = 0.0;
/// Most tile ids the writer puts on one `frames` line.
const FRAMES_PER_LINE: usize = 16;
/// Digits of the hex tile ids, with the `0x` prefix (`0x1E5D`).
const TILE_ID_WIDTH: usize = 6;
/// Name given to the KDL parser for its diagnostics.
const KDL_SOURCE_NAME: &str = "animationdefinition.kdl";
const INDENT: &str = "    ";
/// Depth of the effect nodes in a `frame`: animation, set, frame, effect.
const EFFECT_DEPTH: usize = 3;

const KDL_HEADER: &str = "\
// Animated items (AnimationDefinition.uop), uocf::enhanced::animation_definition.
// animation <item id> [stored-id=item id] [start=0] [second=start] [value=0.0];
// set <key> interval=<seconds> [next=<key>]; frames <tile id>...; frame <tile id> { effects (EffectDefinitionCollection grammar) }.
";

/// The KDL text form of `entries` (schema in the module docs), starting with a short comment header. With
/// `strings`, particles get their informational `name=`.
pub fn animation_definitions_to_kdl<'a>(
    entries: impl IntoIterator<Item = (u32, &'a AnimationDefinition)>,
    strings: Option<&UoStringDictionary>,
) -> String {
    let mut text = String::from(KDL_HEADER);
    for (item_id, definition) in entries {
        write_animation(&mut text, item_id, definition, strings);
    }
    text
}

/// Parses the KDL text form (schema in the module docs) into `(item id, definition)` pairs in file order.
/// `strings` checks informational particle names and resolves particles given by name only.
pub fn animation_definitions_from_kdl(
    text: &str,
    strings: Option<&UoStringDictionary>,
) -> eyre::Result<Vec<(u32, AnimationDefinition)>> {
    let document =
        uot_kdl::parse_ast_line_span(KDL_SOURCE_NAME, text).map_err(|error| describe_error(text, &error))?;
    let mut seen = BTreeMap::new();
    let mut entries = Vec::with_capacity(document.nodes.len());
    for node in &document.nodes {
        if &**node.node_name != ANIMATION_NODE {
            return Err(unknown_node(node, None, &[ANIMATION_NODE]));
        }
        let (item_id, definition) = read_animation(node, strings)?;
        if let Some(line) = seen.insert(item_id, node.node_name.span().0.line + 1) {
            return Err(error_at(node.node_name.span(), format!("item {item_id} repeats (first on line {line})")));
        }
        entries.push((item_id, definition));
    }
    Ok(entries)
}

/// Reads a KDL file ([`animation_definitions_from_kdl`]); errors start with the file path.
pub fn load_animation_definitions_kdl(
    path: impl AsRef<Path>,
    strings: Option<&UoStringDictionary>,
) -> eyre::Result<Vec<(u32, AnimationDefinition)>> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    animation_definitions_from_kdl(&text, strings).map_err(|error| in_file(&path.display().to_string(), error))
}

fn write_animation(text: &mut String, item_id: u32, definition: &AnimationDefinition, strings: Option<&UoStringDictionary>) {
    write!(text, "{ANIMATION_NODE} {item_id}").unwrap();
    if definition.stored_id != item_id {
        write!(text, " {STORED_ID_PROPERTY}={}", definition.stored_id).unwrap();
    }
    if definition.start_set != DEFAULT_START_SET {
        write!(text, " {START_PROPERTY}={}", definition.start_set).unwrap();
    }
    if definition.second_set != definition.start_set {
        write!(text, " {SECOND_PROPERTY}={}", definition.second_set).unwrap();
    }
    if definition.value.to_bits() != DEFAULT_VALUE.to_bits() {
        write!(text, " {VALUE_PROPERTY}={}", f32_text(definition.value)).unwrap();
    }
    if definition.sets.is_empty() {
        text.push('\n');
        return;
    }
    text.push_str(" {\n");
    for (key, set) in &definition.sets {
        write!(text, "{INDENT}{SET_NODE} {key} {INTERVAL_PROPERTY}={}", f32_text(set.frame_interval)).unwrap();
        if let Some(next) = set.next {
            write!(text, " {NEXT_PROPERTY}={next}").unwrap();
        }
        if set.frames.is_empty() {
            text.push('\n');
            continue;
        }
        text.push_str(" {\n");
        write_frames(text, &set.frames, strings);
        writeln!(text, "{INDENT}}}").unwrap();
    }
    text.push_str("}\n");
}

fn write_frames(text: &mut String, frames: &[DefinitionFrame], strings: Option<&UoStringDictionary>) {
    let mut plain = Vec::new();
    let flush = |text: &mut String, plain: &mut Vec<u32>| {
        for line in plain.chunks(FRAMES_PER_LINE) {
            write!(text, "{INDENT}{INDENT}{FRAMES_NODE}").unwrap();
            for tile_id in line {
                write!(text, " {tile_id:#0TILE_ID_WIDTH$X}").unwrap();
            }
            text.push('\n');
        }
        plain.clear();
    };
    for frame in frames {
        if frame.effects.is_empty() {
            plain.push(frame.tile_id);
            continue;
        }
        flush(text, &mut plain);
        writeln!(text, "{INDENT}{INDENT}{FRAME_NODE} {:#0TILE_ID_WIDTH$X} {{", frame.tile_id).unwrap();
        write_effect_definition_nodes(text, &frame.effects, strings, EFFECT_DEPTH);
        writeln!(text, "{INDENT}{INDENT}}}").unwrap();
    }
    flush(text, &mut plain);
}

fn read_animation(node: &KdlNode, strings: Option<&UoStringDictionary>) -> eyre::Result<(u32, AnimationDefinition)> {
    reject_type_name(node)?;
    check_properties(node, &ANIMATION_PROPERTIES)?;
    let [item_id] = arguments::<1>(node)?;
    let item_id = value_u32(item_id, "item id")?;
    let stored_id = optional_u32(node, STORED_ID_PROPERTY)?.unwrap_or(item_id);
    let start_set = optional_u32(node, START_PROPERTY)?.unwrap_or(DEFAULT_START_SET);
    let second_set = optional_u32(node, SECOND_PROPERTY)?.unwrap_or(start_set);
    let value = match property(node, VALUE_PROPERTY) {
        Some(value) => read_value(value)?,
        None => DEFAULT_VALUE,
    };

    let mut sets = BTreeMap::new();
    let mut nexts = Vec::new();
    for child in node.children() {
        if &**child.node_name != SET_NODE {
            return Err(unknown_node(child, Some(ANIMATION_NODE), &[SET_NODE]));
        }
        let (key, set) = read_set(child, strings)?;
        if let Some(next) = set.next {
            nexts.push((key, next, property(child, NEXT_PROPERTY).expect("a set with next has next=")));
        }
        if sets.insert(key, set).is_some() {
            return Err(error_at(child.arguments[0].literal.span(), format!("frame set {key} repeats")));
        }
    }
    let keys: BTreeSet<u32> = sets.keys().copied().collect();
    let missing = |key: u32, what: &str, span: &LineSpan| {
        let message = format!("{what} {key} is not a frame set (sets: {})", list(&keys));
        if keys.contains(&key) { Ok(()) } else { Err(error_at(span, message)) }
    };
    let property_span = |name: &str| property(node, name).map_or(node.node_name.span(), |value| value.literal.span());
    missing(start_set, "start set", property_span(START_PROPERTY))?;
    missing(second_set, "second set", property_span(SECOND_PROPERTY))?;
    for (key, next, value) in nexts {
        missing(next, &format!("frame set {key}: next"), value.literal.span())?;
    }
    Ok((item_id, AnimationDefinition { stored_id, start_set, second_set, value, sets }))
}

fn read_set(node: &KdlNode, strings: Option<&UoStringDictionary>) -> eyre::Result<(u32, FrameSet)> {
    reject_type_name(node)?;
    check_properties(node, &SET_PROPERTIES)?;
    let [key] = arguments::<1>(node)?;
    let key = value_u32(key, "frame set key")?;
    let interval = required_property(node, INTERVAL_PROPERTY)?;
    let frame_interval = value_f32(interval, INTERVAL_PROPERTY)?;
    if !frame_interval.is_finite() || frame_interval < 0.0 {
        let message = format!("{INTERVAL_PROPERTY}: {frame_interval} is not a finite non-negative number of seconds");
        return Err(error_at(interval.literal.span(), message));
    }
    let next = match property(node, NEXT_PROPERTY) {
        None => None,
        Some(value) => {
            let next = value_u32(value, NEXT_PROPERTY)?;
            if i32::try_from(next).is_err() {
                return Err(error_at(value.literal.span(), format!("{NEXT_PROPERTY}: {next} exceeds i32 (the file field is signed)")));
            }
            Some(next)
        }
    };
    let mut frames = Vec::new();
    for child in node.children() {
        reject_type_name(child)?;
        match &**child.node_name {
            FRAMES_NODE => {
                check_properties(child, &[])?;
                reject_children(child)?;
                if child.arguments.is_empty() {
                    return Err(error_at(child.node_name.span(), format!("`{FRAMES_NODE}` takes at least one tile id")));
                }
                for tile_id in &child.arguments {
                    frames.push(DefinitionFrame { tile_id: value_u32(tile_id, "tile id")?, effects: Vec::new() });
                }
            }
            FRAME_NODE => {
                check_properties(child, &[])?;
                let [tile_id] = arguments::<1>(child)?;
                let tile_id = value_u32(tile_id, "tile id")?;
                let effects = effect_definitions_from_nodes(child.children(), strings)?;
                if effects.len() > FRAME_EFFECT_COUNT_MAX {
                    let message = format!("{} effects, at most {FRAME_EFFECT_COUNT_MAX}", effects.len());
                    return Err(error_at(child.node_name.span(), message));
                }
                frames.push(DefinitionFrame { tile_id, effects });
            }
            _ => return Err(unknown_node(child, Some(SET_NODE), &SET_CHILDREN)),
        }
    }
    Ok((key, FrameSet { next, frame_interval, frames }))
}

fn read_value(value: &Value<LineSpan>) -> eyre::Result<f32> {
    let number = value_f32(value, VALUE_PROPERTY)?;
    if number >= 0.0 {
        Ok(number)
    } else {
        Err(error_at(value.literal.span(), format!("{VALUE_PROPERTY}: {number} is negative or NaN: the client rejects it")))
    }
}

fn optional_u32(node: &KdlNode, name: &str) -> eyre::Result<Option<u32>> {
    property(node, name).map(|value| value_u32(value, name)).transpose()
}

fn reject_type_name(node: &Node<LineSpan>) -> eyre::Result<()> {
    match &node.type_name {
        Some(type_name) => Err(error_at(type_name.span(), "type annotations are not supported")),
        None => Ok(()),
    }
}

fn list(keys: &BTreeSet<u32>) -> String {
    if keys.is_empty() {
        return String::from("none");
    }
    keys.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::{EffectDefinition, SoundEffect, WaitEffect};

    const ITEM_ID: u32 = 7905;
    const STORED_ID: u32 = 7897;
    const SOUND_ID: u32 = 42;

    fn sample() -> AnimationDefinition {
        let sound = EffectDefinition::Sound(SoundEffect { sound_id: SOUND_ID, value_08: 1.0 });
        let wait = EffectDefinition::Wait(WaitEffect { children: vec![sound.clone()], min_delay_ms: 5, max_delay_ms: 9, aligned: true });
        let plain = |tile_id| DefinitionFrame { tile_id, effects: Vec::new() };
        let frames = vec![plain(0x1EDD), plain(0x1EDE), DefinitionFrame { tile_id: 0x1EE0, effects: vec![sound, wait] }, plain(0x1EE1)];
        AnimationDefinition {
            stored_id: STORED_ID,
            start_set: 0,
            second_set: 1,
            value: 2.5,
            sets: BTreeMap::from([
                (0, FrameSet { next: Some(1), frame_interval: 0.25, frames }),
                (1, FrameSet { next: None, frame_interval: 0.0625, frames: vec![plain(0x1EE2)] }),
                (2, FrameSet { next: Some(2), frame_interval: 0.1, frames: Vec::new() }),
            ]),
        }
    }

    #[test]
    fn writes_the_compact_form() {
        let text = animation_definitions_to_kdl([(ITEM_ID, &sample())], None);
        let body = text.strip_prefix(KDL_HEADER).unwrap();
        assert_eq!(
            body,
            "animation 7905 stored-id=7897 second=1 value=2.5 {
    set 0 interval=0.25 next=1 {
        frames 0x1EDD 0x1EDE
        frame 0x1EE0 {
            sound 42
            wait min=5 max=9 aligned=true {
                sound 42
            }
        }
        frames 0x1EE1
    }
    set 1 interval=0.0625 {
        frames 0x1EE2
    }
    set 2 interval=0.1 next=2
}
"
        );
        let defaults = AnimationDefinition {
            stored_id: ITEM_ID,
            start_set: 0,
            second_set: 0,
            value: 0.0,
            sets: BTreeMap::from([(0, FrameSet { next: None, frame_interval: 0.5, frames: Vec::new() })]),
        };
        let text = animation_definitions_to_kdl([(ITEM_ID, &defaults)], None);
        assert_eq!(text.strip_prefix(KDL_HEADER).unwrap(), "animation 7905 {\n    set 0 interval=0.5\n}\n");
    }

    #[test]
    fn round_trips_through_kdl() {
        let mut long = sample();
        long.sets.get_mut(&1).unwrap().frames =
            (0..FRAMES_PER_LINE as u32 * 2 + 1).map(|tile_id| DefinitionFrame { tile_id, effects: Vec::new() }).collect();
        let mut odd = sample();
        odd.value = -0.0;
        odd.start_set = 2;
        odd.second_set = 2;
        odd.sets.get_mut(&0).unwrap().frame_interval = f32::from_bits(1);
        let entries = [(ITEM_ID, sample()), (1, long), (2, odd)];
        let text = animation_definitions_to_kdl(entries.iter().map(|(id, definition)| (*id, definition)), None);
        let parsed = animation_definitions_from_kdl(&text, None).unwrap();
        assert_eq!(parsed, entries);
        for (id, definition) in &parsed {
            let expected = entries.iter().find(|(entry_id, _)| entry_id == id).unwrap();
            assert_eq!(definition.to_bytes().unwrap(), expected.1.to_bytes().unwrap(), "item {id}");
            assert_eq!(definition.value.to_bits(), expected.1.value.to_bits(), "item {id}");
        }
    }

    #[test]
    fn errors_name_line_and_column() {
        let cases = [
            ("anim 1\n", "line 1, column 1: unknown node `anim` at the top level: expected `animation`"),
            ("animation 1 size=2 {\n}\n", "line 1, column 13: unknown property `size` of `animation`: expected one of `stored-id`, `start`, `second`, `value`"),
            ("animation 1 {\n    sets 0 interval=1\n}\n", "line 2, column 5: unknown node `sets` in `animation`: expected `set`"),
            ("animation 1 {\n    set 0 interval=1 {\n        tile 1\n    }\n}\n", "line 3, column 9: unknown node `tile` in `set`: expected `frames` or `frame`"),
            ("animation 1 {\n    set 0 interval=1 nxt=0\n}\n", "line 2, column 22: unknown property `nxt` of `set`: expected `interval` or `next`"),
            ("animation 1 {\n    set 0\n}\n", "line 2, column 5: `set`: missing interval="),
            ("animation 1 {\n    set 0 interval=1 next=4\n}\n", "line 2, column 27: frame set 0: next 4 is not a frame set (sets: 0)"),
            ("animation 1 start=3 {\n    set 0 interval=1\n}\n", "line 1, column 19: start set 3 is not a frame set (sets: 0)"),
            ("animation 1\n", "line 1, column 1: start set 0 is not a frame set (sets: none)"),
            ("animation 1 second=2 {\n    set 0 interval=1\n}\n", "line 1, column 20: second set 2 is not a frame set"),
            ("animation 1 {\n    set 0 interval=1\n    set 0 interval=2\n}\n", "line 3, column 9: frame set 0 repeats"),
            ("animation 1 {\n    set 0 interval=-1.0\n}\n", "line 2, column 20: interval: -1 is not a finite non-negative"),
            ("animation 1 {\n    set 0 interval=\"nan\"\n}\n", "line 2, column 20: interval: NaN is not a finite"),
            ("animation 1 value=-2.0 {\n    set 0 interval=1\n}\n", "line 1, column 19: value: -2 is negative or NaN"),
            ("animation 1 {\n    set 0 interval=1 next=2147483648\n}\n", "line 2, column 27: next: 2147483648 exceeds i32"),
            ("animation 1 {\n    set 0 interval=1 {\n        frames\n    }\n}\n", "line 3, column 9: `frames` takes at least one tile id"),
            ("animation 1 {\n    set 0 interval=1 {\n        frames 1 { sound 1; }\n    }\n}\n", "line 3, column 20: `frames` takes no children, found `sound`"),
            ("animation 1 {\n    set 0 interval=1 {\n        frame 1 2\n    }\n}\n", "line 3, column 17: `frame` takes 1 arguments, found 2"),
            ("animation 1 {\n    set 0 interval=1 {\n        frame 1 {\n            noise 1\n        }\n    }\n}\n", "line 4, column 13: unknown"),
            ("animation 1 {\n    set 0 interval=1 {\n        frame \"a\"\n    }\n}\n", "line 3, column 15: tile id: expected an integer"),
            ("animation 1 {\n    set 0 interval=1\n}\nanimation 1 {\n    set 0 interval=1\n}\n", "line 4, column 1: item 1 repeats (first on line 1)"),
        ];
        for (text, expected) in cases {
            let error = animation_definitions_from_kdl(text, None).unwrap_err().to_string();
            assert!(error.starts_with(expected), "{text:?}: {error}");
        }
    }

    #[test]
    fn syntax_errors_name_line_and_column() {
        let error = animation_definitions_from_kdl("animation 1 {\n    set 0 interval=1\n", None).unwrap_err().to_string();
        assert!(error.starts_with("invalid KDL: line "), "{error}");
    }

    #[test]
    fn load_names_the_file() {
        let dir = std::env::temp_dir().join(format!("uocf_animdef_kdl_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.kdl");
        std::fs::write(&path, "anim 1\n").unwrap();
        let error = load_animation_definitions_kdl(&path, None).unwrap_err().to_string();
        assert!(error.starts_with(&format!("{}: line 1, column 1:", path.display())), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
