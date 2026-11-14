//! The item light scales, `runtime_specs/lights/LightScale.kdl`: a multiplier on the size of chosen items' local
//! lights in KR-like, Enhanced and UOD-Painterly, the light sprites (on top of their tileart scale) and the light.mul
//! lights alike. Classic draws the lights as the client does. The file is hand-written and read strictly
//! (`uot_kdl::strict`): every error names its line. It is read at startup; a user copy under the cache dir takes the
//! bundled file's place (`configs::runtime_specs`).

use std::ops::RangeInclusive;

use bevy::prelude::Resource;
use color_eyre::eyre;
use uot_kdl::ast::{Node, Value};
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};

use crate::configs::runtime_specs::SpecFiles;

/// The folder of the file under `runtime_specs`, and its name.
pub const LIGHT_SCALE_SPECS_FOLDER: &str = "lights";
pub const LIGHT_SCALE_FILE_NAME: &str = "LightScale.kdl";
/// The multipliers the file accepts.
pub const LIGHT_SCALE_RANGE: RangeInclusive<f32> = 0.25..=8.0;
/// The multiplier of an item the file does not list: its lights as they come.
pub const NEUTRAL_LIGHT_SCALE: f32 = 1.0;
/// What the log calls the file.
const LIGHT_SCALE_WHAT: &str = "the item light scales";

const ITEM_NODE: &str = "item";
const SCALE_PROPERTY: &str = "scale";
pub const LIGHT_SCALE_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf(ITEM_NODE, &[SCALE_PROPERTY])];

/// The items `first..=last` and their multiplier.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ItemLightScaleRule {
    first: u16,
    last: u16,
    scale: f32,
    /// The rule's 1-based line, for the error of an item listed twice.
    line: usize,
}

/// The multipliers of the items' light sizes.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct ItemLightScales {
    rules: Vec<ItemLightScaleRule>,
}

impl ItemLightScales {
    /// Reads the file at startup: the bundled file is required, a missing or invalid one stops the app; a user copy
    /// that does not read leaves it in force, with a warning.
    pub fn load() -> Self {
        let loaded = SpecFiles::of(LIGHT_SCALE_SPECS_FOLDER, LIGHT_SCALE_FILE_NAME)
            .load(Self::decode)
            .unwrap_or_else(|error| crate::configs::settings::exit_on_startup_config_error(&error));
        loaded.report(LIGHT_SCALE_WHAT);
        loaded.value
    }

    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        let mut rules: Vec<ItemLightScaleRule> = Vec::new();
        for node in &document.nodes {
            let rule = strict::node_schema(node, None, LIGHT_SCALE_SCHEMA)
                .and_then(|schema| decode_rule(node, schema))
                .map_err(|error| strict::in_file(source_name, error))?;
            if let Some(earlier) = rules.iter().find(|earlier| earlier.first <= rule.last && rule.first <= earlier.last) {
                let id = rule.first.max(earlier.first);
                let error = strict::error_at(node.node_name.span(),
                    format!("item {id} is listed already, on line {}", earlier.line));
                return Err(strict::in_file(source_name, error));
            }
            rules.push(rule);
        }
        Ok(Self { rules })
    }

    /// The multiplier of the item's lights; `NEUTRAL_LIGHT_SCALE` when the file does not list it.
    pub fn scale(&self, graphic: u16) -> f32 {
        self.rules
            .iter()
            .find(|rule| (rule.first..=rule.last).contains(&graphic))
            .map_or(NEUTRAL_LIGHT_SCALE, |rule| rule.scale)
    }

    /// The largest multiplier, at least the neutral one: how much farther a light may reach than its data says.
    pub fn largest(&self) -> f32 {
        self.rules.iter().map(|rule| rule.scale).fold(NEUTRAL_LIGHT_SCALE, f32::max)
    }
}

fn decode_rule(node: &Node<LineSpan>, schema: &NodeSchema) -> eyre::Result<ItemLightScaleRule> {
    strict::check_properties(node, schema.properties)?;
    strict::reject_children(node)?;
    let (first, last) = id_range(node)?;
    let value = strict::required_property(node, SCALE_PROPERTY)?;
    let scale = strict::value_f32(value, SCALE_PROPERTY)?;
    if !LIGHT_SCALE_RANGE.contains(&scale) {
        return Err(strict::error_at(value.literal.span(), format!("scale: {scale} is outside {}..={}",
            LIGHT_SCALE_RANGE.start(), LIGHT_SCALE_RANGE.end())));
    }
    Ok(ItemLightScaleRule { first, last, scale, line: node.node_name.span().0.line + 1 })
}

/// The node's item id, or the first and last id of its range.
fn id_range(node: &Node<LineSpan>) -> eyre::Result<(u16, u16)> {
    match node.arguments.as_slice() {
        [id] => {
            let id = item_id(id, "the id")?;
            Ok((id, id))
        }
        [first, last] => {
            let (first_id, last_id) = (item_id(first, "the first id")?, item_id(last, "the last id")?);
            if last_id < first_id {
                return Err(strict::error_at(last.literal.span(),
                    format!("the range ends at {last_id}, before its first id {first_id}")));
            }
            Ok((first_id, last_id))
        }
        arguments => {
            let span = arguments.get(2).map_or(node.node_name.span(), |extra| extra.literal.span());
            Err(strict::error_at(span, format!(
                "`{ITEM_NODE}` takes an item id, or the first and last id of a range; found {} arguments",
                arguments.len())))
        }
    }
}

fn item_id(value: &Value<LineSpan>, field: &str) -> eyre::Result<u16> {
    let id = strict::value_u32(value, field)?;
    u16::try_from(id).map_err(|_| strict::error_at(value.literal.span(),
        format!("{field}: {id} is past the last item id, {}", u16::MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "LightScale.kdl";

    fn decode(text: &str) -> ItemLightScales {
        ItemLightScales::decode(FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// The error starts with the file and the line, and says what is wrong.
    fn assert_error(text: &str, line: usize, needle: &str) {
        let error = format!("{:#}", ItemLightScales::decode(FILE, text).expect_err("the file should not read"));
        assert!(error.starts_with(&format!("{FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn items_and_ranges_take_their_scale_and_the_rest_stay_neutral() {
        let scales = decode("item 4846 4892 scale=1.5 // lava\nitem 12935 scale=2\n");
        assert_eq!(scales.scale(4846), 1.5);
        assert_eq!(scales.scale(4870), 1.5);
        assert_eq!(scales.scale(4892), 1.5);
        assert_eq!(scales.scale(4893), NEUTRAL_LIGHT_SCALE);
        assert_eq!(scales.scale(12935), 2.0);
        assert_eq!(scales.largest(), 2.0);
        assert_eq!(decode("").largest(), NEUTRAL_LIGHT_SCALE);
    }

    #[test]
    fn the_bundled_file_reads_and_scales_the_lava() {
        const LAVA: u16 = 4846;
        let text = include_str!("../../assets/runtime_specs/lights/LightScale.kdl");
        let scales = decode(text);
        assert!(scales.scale(LAVA) > NEUTRAL_LIGHT_SCALE);
    }

    #[test]
    fn errors_name_the_line() {
        assert_error("item 1 scale=1\nlight 2 scale=1\n", 2, "unknown node `light` at the top level: expected `item`");
        assert_error("item 1 size=2\n", 1, "unknown property `size` of `item`");
        assert_error("item 1\n", 1, "`item`: missing scale=");
        assert_error("item 1 scale=\"big\"\n", 1, "scale: expected a number");
        assert_error("item 1 scale=0\n", 1, "scale: 0 is outside 0.25..=8");
        assert_error("item 1 scale=9\n", 1, "scale: 9 is outside 0.25..=8");
        assert_error("item 70000 scale=2\n", 1, "the id: 70000 is past the last item id");
        assert_error("item 9 3 scale=2\n", 1, "the range ends at 3, before its first id 9");
        assert_error("item scale=2\n", 1, "found 0 arguments");
        assert_error("item 1 2 3 scale=2\n", 1, "found 3 arguments");
        assert_error("item 1 scale=2 { item 2 scale=2; }\n", 1, "`item` takes no children");
        assert_error("item 10 20 scale=2\nitem 5 12 scale=3\n", 2, "item 10 is listed already, on line 1");
    }
}
