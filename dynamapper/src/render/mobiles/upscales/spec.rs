//! Which bodies the Balanced upscale mode upscales, `runtime_specs/mobiles/Upscale.kdl`: the size rule's largest
//! canvas, and bodies always or never upscaled whatever their size, by their in-game body id, in either package or
//! one. The `never` entries hold in the Always mode too. The file is hand-written and read strictly (`uot_kdl::strict`): every error names its line. It is read at
//! startup; a user copy under the cache dir takes the bundled file's place (`settings::runtime_specs`).

use std::ops::RangeInclusive;

use color_eyre::eyre;
use uot_kdl::ast::Node;
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};

use crate::settings::runtime_specs::SpecFiles;
use super::super::clips::AnimSource;
use super::{UPSCALE_CANVAS_MULTIPLE, UPSCALE_MAX_CANVAS_SIDE};

/// The folder of the file under `runtime_specs`, and its name.
pub const UPSCALE_SPECS_FOLDER: &str = "mobiles";
pub const UPSCALE_FILE_NAME: &str = "Upscale.kdl";
/// The sides of the largest canvas the size rule may name: up to the workspace a canvas upscales whole on.
pub const UPSCALE_SPEC_CANVAS_RANGE: RangeInclusive<u32> = UPSCALE_CANVAS_MULTIPLE..=UPSCALE_MAX_CANVAS_SIDE;
/// What the log calls the file.
const UPSCALE_WHAT: &str = "the mobile upscale bodies";

const DEFAULT_NODE: &str = "default";
const BODY_NODE: &str = "body";
const MAX_CANVAS_PROPERTY: &str = "max_canvas";
const PACKAGE_PROPERTY: &str = "package";
const UPSCALE_PROPERTY: &str = "upscale";
pub const UPSCALE_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(DEFAULT_NODE, &[MAX_CANVAS_PROPERTY]),
    NodeSchema::leaf(BODY_NODE, &[PACKAGE_PROPERTY, UPSCALE_PROPERTY]),
];
/// The packages' names in the file.
const CLASSIC_PACKAGE: &str = "cc";
const ENHANCED_PACKAGE: &str = "ec";
const ALWAYS: &str = "always";
const NEVER: &str = "never";

/// What a body entry asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyUpscale {
    Always,
    Never,
}

/// Bodies `first..=last` of `package` (of both without one), and what they ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BodyRule {
    first: u32,
    last: u32,
    package: Option<AnimSource>,
    upscale: BodyUpscale,
    /// The rule's 1-based line, for the error of a body listed twice.
    line: usize,
}

/// The file: the size rule's largest canvas side, and the bodies listed.
#[derive(Debug, Clone, PartialEq)]
pub struct MobileUpscaleSpec {
    max_canvas: u32,
    rules: Vec<BodyRule>,
}

impl Default for MobileUpscaleSpec {
    /// Without a file: the size rule at the largest canvas, no body listed.
    fn default() -> Self {
        Self { max_canvas: UPSCALE_MAX_CANVAS_SIDE, rules: Vec::new() }
    }
}

impl MobileUpscaleSpec {
    /// Reads the file at startup: the bundled file is required, a missing or invalid one stops the app; a user copy
    /// that does not read leaves it in force, with a warning.
    pub fn load() -> Self {
        let loaded = SpecFiles::of(UPSCALE_SPECS_FOLDER, UPSCALE_FILE_NAME)
            .load(Self::decode)
            .unwrap_or_else(|error| crate::settings::exit_on_startup_config_error(&error));
        loaded.report(UPSCALE_WHAT);
        loaded.value
    }

    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        let mut spec = Self::default();
        let mut default_line = None;
        for node in &document.nodes {
            let schema = strict::node_schema(node, None, UPSCALE_SCHEMA)
                .map_err(|error| strict::in_file(source_name, error))?;
            let line = node.node_name.span().0.line + 1;
            if schema.name == DEFAULT_NODE {
                if let Some(earlier) = default_line {
                    let error = strict::error_at(node.node_name.span(),
                        format!("`{DEFAULT_NODE}` is given already, on line {earlier}"));
                    return Err(strict::in_file(source_name, error));
                }
                default_line = Some(line);
                spec.max_canvas = decode_default(node).map_err(|error| strict::in_file(source_name, error))?;
                continue;
            }
            let rule = decode_rule(node, line).map_err(|error| strict::in_file(source_name, error))?;
            if let Some(earlier) = spec.rules.iter().find(|earlier| earlier.package == rule.package
                && earlier.first <= rule.last && rule.first <= earlier.last) {
                let body = rule.first.max(earlier.first);
                let error = strict::error_at(node.node_name.span(),
                    format!("body {body} is listed already{}, on line {}", package_words(rule.package), earlier.line));
                return Err(strict::in_file(source_name, error));
            }
            spec.rules.push(rule);
        }
        Ok(spec)
    }

    /// The size rule's largest canvas, in pixels: `max_canvas` squared.
    pub fn max_canvas_pixels(&self) -> u32 {
        self.max_canvas * self.max_canvas
    }

    /// What the file asks for body `body` of `source`: its entry for that package, else its entry for both, else
    /// none (the size rule).
    pub fn body(&self, source: AnimSource, body: u32) -> Option<BodyUpscale> {
        let listed = |package: Option<AnimSource>| self.rules.iter()
            .find(|rule| rule.package == package && (rule.first..=rule.last).contains(&body))
            .map(|rule| rule.upscale);
        listed(Some(source)).or_else(|| listed(None))
    }
}

fn package_words(package: Option<AnimSource>) -> String {
    match package {
        Some(AnimSource::Classic) => format!(" for {PACKAGE_PROPERTY}=\"{CLASSIC_PACKAGE}\""),
        Some(AnimSource::Enhanced) => format!(" for {PACKAGE_PROPERTY}=\"{ENHANCED_PACKAGE}\""),
        None => " for both packages".to_string(),
    }
}

fn decode_default(node: &Node<LineSpan>) -> eyre::Result<u32> {
    strict::check_properties(node, &[MAX_CANVAS_PROPERTY])?;
    strict::reject_children(node)?;
    if let Some(argument) = node.arguments.first() {
        return Err(strict::error_at(argument.literal.span(), format!("`{DEFAULT_NODE}` takes no arguments")));
    }
    let value = strict::required_property(node, MAX_CANVAS_PROPERTY)?;
    let side = strict::value_u32(value, MAX_CANVAS_PROPERTY)?;
    if !UPSCALE_SPEC_CANVAS_RANGE.contains(&side) {
        return Err(strict::error_at(value.literal.span(), format!("{MAX_CANVAS_PROPERTY}: {side} is outside {}..={}",
            UPSCALE_SPEC_CANVAS_RANGE.start(), UPSCALE_SPEC_CANVAS_RANGE.end())));
    }
    Ok(side)
}

fn decode_rule(node: &Node<LineSpan>, line: usize) -> eyre::Result<BodyRule> {
    strict::check_properties(node, &[PACKAGE_PROPERTY, UPSCALE_PROPERTY])?;
    strict::reject_children(node)?;
    let (first, last) = id_range(node)?;
    let package = match strict::property(node, PACKAGE_PROPERTY) {
        None => None,
        Some(value) => Some(match strict::value_string(value, PACKAGE_PROPERTY)?.as_str() {
            CLASSIC_PACKAGE => AnimSource::Classic,
            ENHANCED_PACKAGE => AnimSource::Enhanced,
            other => return Err(strict::error_at(value.literal.span(), format!(
                "{PACKAGE_PROPERTY}: \"{other}\" is not \"{CLASSIC_PACKAGE}\" or \"{ENHANCED_PACKAGE}\""))),
        }),
    };
    let value = strict::required_property(node, UPSCALE_PROPERTY)?;
    let upscale = match strict::value_string(value, UPSCALE_PROPERTY)?.as_str() {
        ALWAYS => BodyUpscale::Always,
        NEVER => BodyUpscale::Never,
        other => return Err(strict::error_at(value.literal.span(),
            format!("{UPSCALE_PROPERTY}: \"{other}\" is not \"{ALWAYS}\" or \"{NEVER}\""))),
    };
    Ok(BodyRule { first, last, package, upscale, line })
}

/// The node's body id, or the first and last id of its range.
fn id_range(node: &Node<LineSpan>) -> eyre::Result<(u32, u32)> {
    match node.arguments.as_slice() {
        [id] => {
            let id = strict::value_u32(id, "the id")?;
            Ok((id, id))
        }
        [first, last] => {
            let (first_id, last_id) = (strict::value_u32(first, "the first id")?, strict::value_u32(last, "the last id")?);
            if last_id < first_id {
                return Err(strict::error_at(last.literal.span(),
                    format!("the range ends at {last_id}, before its first id {first_id}")));
            }
            Ok((first_id, last_id))
        }
        arguments => {
            let span = arguments.get(2).map_or(node.node_name.span(), |extra| extra.literal.span());
            Err(strict::error_at(span, format!(
                "`{BODY_NODE}` takes a body id, or the first and last id of a range; found {} arguments",
                arguments.len())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "Upscale.kdl";

    fn decode(text: &str) -> MobileUpscaleSpec {
        MobileUpscaleSpec::decode(FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// The error starts with the file and the line, and says what is wrong.
    fn assert_error(text: &str, line: usize, needle: &str) {
        let error = format!("{:#}", MobileUpscaleSpec::decode(FILE, text).expect_err("the file should not read"));
        assert!(error.starts_with(&format!("{FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn a_package_entry_comes_before_one_for_both_and_the_rest_follow_the_size_rule() {
        let spec = decode("default max_canvas=200\nbody 12 package=\"cc\" upscale=\"always\"\n\
            body 12 upscale=\"never\"\nbody 400 410 upscale=\"always\" // humans\n");
        assert_eq!(spec.max_canvas_pixels(), 200 * 200);
        assert_eq!(spec.body(AnimSource::Classic, 12), Some(BodyUpscale::Always));
        assert_eq!(spec.body(AnimSource::Enhanced, 12), Some(BodyUpscale::Never));
        assert_eq!(spec.body(AnimSource::Enhanced, 405), Some(BodyUpscale::Always));
        assert_eq!(spec.body(AnimSource::Classic, 411), None);
        // Without a `default`, the largest canvas.
        assert_eq!(decode("").max_canvas_pixels(), UPSCALE_MAX_CANVAS_SIDE * UPSCALE_MAX_CANVAS_SIDE);
    }

    #[test]
    fn the_bundled_file_reads_with_the_largest_canvas() {
        let text = include_str!("../../../../assets/runtime_specs/mobiles/Upscale.kdl");
        assert_eq!(decode(text).max_canvas_pixels(), UPSCALE_MAX_CANVAS_SIDE * UPSCALE_MAX_CANVAS_SIDE);
    }

    #[test]
    fn errors_name_the_line() {
        assert_error("default max_canvas=200\nmobile 2 upscale=\"never\"\n", 2,
            "unknown node `mobile` at the top level: expected `default` or `body`");
        assert_error("default max_canvas=200\ndefault max_canvas=100\n", 2, "`default` is given already, on line 1");
        assert_error("default\n", 1, "`default`: missing max_canvas=");
        assert_error("default 3 max_canvas=200\n", 1, "`default` takes no arguments");
        assert_error("default max_canvas=261\n", 1, "max_canvas: 261 is outside 8..=260");
        assert_error("default max_canvas=4\n", 1, "max_canvas: 4 is outside 8..=260");
        assert_error("body 1 size=2\n", 1, "unknown property `size` of `body`");
        assert_error("body 1\n", 1, "`body`: missing upscale=");
        assert_error("body 1 upscale=\"maybe\"\n", 1, "upscale: \"maybe\" is not \"always\" or \"never\"");
        assert_error("body 1 package=\"kr\" upscale=\"never\"\n", 1, "package: \"kr\" is not \"cc\" or \"ec\"");
        assert_error("body 9 3 upscale=\"never\"\n", 1, "the range ends at 3, before its first id 9");
        assert_error("body upscale=\"never\"\n", 1, "found 0 arguments");
        assert_error("body 1 upscale=\"never\" { body 2; }\n", 1, "`body` takes no children");
        assert_error("body 10 20 package=\"ec\" upscale=\"never\"\nbody 5 12 package=\"ec\" upscale=\"always\"\n", 2,
            "body 10 is listed already for package=\"ec\", on line 1");
        assert_error("body 7 upscale=\"never\"\nbody 7 upscale=\"never\"\n", 2,
            "body 7 is listed already for both packages, on line 1");
    }
}
