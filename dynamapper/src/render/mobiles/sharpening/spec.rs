//! How the mobiles' frames are sharpened and their edges drawn, `runtime_specs/mobiles/Sharpening.kdl`: per animation
//! package (`classic`, `enhanced`), a `default`, size classes by the body's largest stored frame side and bodies by
//! their in-game id, each with any of the filter's properties. A property resolves from the body's entry, else its
//! size class, else the package's default, else off. The file is hand-written and read strictly (`uot_kdl::strict`):
//! every error names its line. It is read at startup and again when it is saved (sharpening.rs); a user copy under
//! the cache dir takes the bundled file's place (`settings::runtime_specs`).

use std::ops::RangeInclusive;

use color_eyre::eyre;
use uot_kdl::ast::{Node, SpannedNode};
use uot_kdl::span::LineSpan;
use uot_kdl::strict::{self, NodeSchema};

use super::super::clips::AnimSource;

/// The folder of the file under `runtime_specs`, and its name.
pub const SHARPENING_SPECS_FOLDER: &str = "mobiles";
pub const SHARPENING_FILE_NAME: &str = "Sharpening.kdl";

const CLASSIC_NODE: &str = "classic";
const ENHANCED_NODE: &str = "enhanced";
const DEFAULT_NODE: &str = "default";
const SIZE_NODE: &str = "size";
const BODY_NODE: &str = "body";
const MAX_PROPERTY: &str = "max";
const SHARPENING_PROPERTY: &str = "sharpening";
const FRINGE_SHARPENING_PROPERTY: &str = "fringe_sharpening";
const ALPHA_CONTRAST_PROPERTY: &str = "alpha_contrast";
const ALPHA_MIDPOINT_PROPERTY: &str = "alpha_midpoint";
const CONTOUR_PROPERTY: &str = "contour";
const CONTOUR_COLOR_PROPERTY: &str = "contour_color";
const OUTLINE_PROPERTY: &str = "outline";
const INNER_LINES_PROPERTY: &str = "inner_lines";
const INNER_CONTRAST_PROPERTY: &str = "inner_contrast";
const INNER_THRESHOLD_PROPERTY: &str = "inner_threshold";
/// The filter's properties, which every entry takes.
const FILTER_PROPERTIES: &[&str] = &[SHARPENING_PROPERTY, FRINGE_SHARPENING_PROPERTY, ALPHA_CONTRAST_PROPERTY,
    ALPHA_MIDPOINT_PROPERTY, CONTOUR_PROPERTY, CONTOUR_COLOR_PROPERTY, OUTLINE_PROPERTY, INNER_LINES_PROPERTY,
    INNER_CONTRAST_PROPERTY, INNER_THRESHOLD_PROPERTY];
const SIZE_PROPERTIES: &[&str] = &[MAX_PROPERTY, SHARPENING_PROPERTY, FRINGE_SHARPENING_PROPERTY,
    ALPHA_CONTRAST_PROPERTY, ALPHA_MIDPOINT_PROPERTY, CONTOUR_PROPERTY, CONTOUR_COLOR_PROPERTY, OUTLINE_PROPERTY,
    INNER_LINES_PROPERTY, INNER_CONTRAST_PROPERTY, INNER_THRESHOLD_PROPERTY];
const ENTRY_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(DEFAULT_NODE, FILTER_PROPERTIES),
    NodeSchema::leaf(SIZE_NODE, SIZE_PROPERTIES),
    NodeSchema::leaf(BODY_NODE, FILTER_PROPERTIES),
];
pub const SHARPENING_SCHEMA: &[NodeSchema] = &[
    NodeSchema { name: CLASSIC_NODE, properties: &[], children: ENTRY_SCHEMA },
    NodeSchema { name: ENHANCED_NODE, properties: &[], children: ENTRY_SCHEMA },
];

/// The strengths the file takes: `sharpening` (1 is AMD CAS's strongest lobe), `fringe_sharpening`, `alpha_contrast`,
/// `contour`, `outline`, `inner_lines` and `inner_contrast`.
pub const FILTER_STRENGTH_RANGE: RangeInclusive<f32> = 0.0..=1.0;
/// The colour steps the interior edges may start at (art/mobile_inner_edges.wesl): above 0, where every texel of
/// a gradient would be an edge, up to the step from black to white.
pub const INNER_THRESHOLD_RANGE: RangeInclusive<f32> = 0.01..=1.0;
/// The alpha midpoints the contrast may turn about: away from 0 and 1, which would leave nothing or everything.
pub const ALPHA_MIDPOINT_RANGE: RangeInclusive<f32> = 0.05..=0.95;
/// The size classes' largest sides, in pixels: up to the largest frame a package stores (640).
pub const SIZE_CLASS_SIDE_RANGE: RangeInclusive<u32> = 1..=4096;
/// The digits of a `contour_color`, after its `#`: `rrggbb`.
const COLOR_HEX_DIGITS: usize = 6;
const COLOR_HEX_RADIX: u32 = 16;

/// The filter of a body: each strength 0..1, the contour's colour sRGB-encoded. [`Self::OFF`] leaves the frames as
/// they are stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameFilter {
    /// The CAS strength (art/mobile_sharpen.wesl), before the style's multiplier.
    pub sharpening: f32,
    /// The share of `sharpening` a partly transparent texel (the Enhanced frames' anti-aliased fringe) takes.
    pub fringe_sharpening: f32,
    /// How far the partial alpha is pulled to 0 or 1 about `alpha_midpoint`: 0 leaves it, 1 thresholds it.
    pub alpha_contrast: f32,
    pub alpha_midpoint: f32,
    /// How far the fringe and the outermost opaque ring are darkened towards `contour_color`.
    pub contour: f32,
    pub contour_color: [u8; 3],
    /// The opacity of a one-texel line of `contour_color` outside the silhouette.
    pub outline: f32,
    /// The strength of the interior lines: an art pixel wide at the foot of the dark side of a strong edge of the
    /// smoothed brightness inside the body, a darker shade of the colour under them.
    pub inner_lines: f32,
    /// The strength of the contrast across the same edges: their dark side darkened, their light side lightened.
    pub inner_contrast: f32,
    /// The step (0..1, black to white) at which an interior edge starts: of the smoothed brightness for the lines,
    /// of the colour over the 3x3 texels about the texel for the contrast.
    pub inner_threshold: f32,
}

impl FrameFilter {
    /// Nothing changed: no sharpening, the fringe as sharpened as the rest, no contrast, contour or outline.
    pub const OFF: Self = Self {
        sharpening: 0.0,
        fringe_sharpening: 1.0,
        alpha_contrast: 0.0,
        alpha_midpoint: 0.5,
        contour: 0.0,
        contour_color: DEFAULT_CONTOUR_COLOR,
        outline: 0.0,
        inner_lines: 0.0,
        inner_contrast: 0.0,
        inner_threshold: DEFAULT_INNER_THRESHOLD,
    };
}

/// The contour's colour when an entry names none: the near black of the Classic frames' outlines.
pub const DEFAULT_CONTOUR_COLOR: [u8; 3] = [0x20, 0x20, 0x20];
/// The interior edges' threshold when an entry names none.
pub const DEFAULT_INNER_THRESHOLD: f32 = 0.12;

/// The properties an entry gives; those it leaves out come from the entry below it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct FilterEntry {
    sharpening: Option<f32>,
    fringe_sharpening: Option<f32>,
    alpha_contrast: Option<f32>,
    alpha_midpoint: Option<f32>,
    contour: Option<f32>,
    contour_color: Option<[u8; 3]>,
    outline: Option<f32>,
    inner_lines: Option<f32>,
    inner_contrast: Option<f32>,
    inner_threshold: Option<f32>,
}

impl FilterEntry {
    /// `filter` with the properties this entry gives.
    fn over(self, filter: FrameFilter) -> FrameFilter {
        FrameFilter {
            sharpening: self.sharpening.unwrap_or(filter.sharpening),
            fringe_sharpening: self.fringe_sharpening.unwrap_or(filter.fringe_sharpening),
            alpha_contrast: self.alpha_contrast.unwrap_or(filter.alpha_contrast),
            alpha_midpoint: self.alpha_midpoint.unwrap_or(filter.alpha_midpoint),
            contour: self.contour.unwrap_or(filter.contour),
            contour_color: self.contour_color.unwrap_or(filter.contour_color),
            outline: self.outline.unwrap_or(filter.outline),
            inner_lines: self.inner_lines.unwrap_or(filter.inner_lines),
            inner_contrast: self.inner_contrast.unwrap_or(filter.inner_contrast),
            inner_threshold: self.inner_threshold.unwrap_or(filter.inner_threshold),
        }
    }
}

/// The bodies whose largest stored frame side is `max_side` pixels or less, and above the next smaller class's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SizeClass {
    max_side: u32,
    entry: FilterEntry,
    line: usize,
}

/// Bodies `first..=last`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BodyEntry {
    first: u32,
    last: u32,
    entry: FilterEntry,
    line: usize,
}

/// One package's section.
#[derive(Debug, Clone, Default, PartialEq)]
struct PackageFilters {
    default: FilterEntry,
    /// By `max_side`, smallest first.
    sizes: Vec<SizeClass>,
    bodies: Vec<BodyEntry>,
}

impl PackageFilters {
    fn resolve(&self, body: u32, largest_side: Option<u32>) -> FrameFilter {
        let mut filter = self.default.over(FrameFilter::OFF);
        if let Some(side) = largest_side
            && let Some(class) = self.sizes.iter().find(|class| side <= class.max_side) {
            filter = class.entry.over(filter);
        }
        if let Some(listed) = self.bodies.iter().find(|listed| (listed.first..=listed.last).contains(&body)) {
            filter = listed.entry.over(filter);
        }
        filter
    }
}

/// The file: a section per package; a package without one draws its frames as stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MobileSharpeningSpec {
    classic: PackageFilters,
    enhanced: PackageFilters,
}

impl MobileSharpeningSpec {
    pub fn decode(source_name: &str, text: &str) -> eyre::Result<Self> {
        let document = uot_kdl::parse_ast_line_span(source_name, text)
            .map_err(|error| strict::in_file(source_name, strict::describe_error(text, &error)))?;
        let mut spec = Self::default();
        let mut section_lines: [Option<usize>; 2] = [None, None];
        for node in &document.nodes {
            let schema = strict::node_schema(node, None, SHARPENING_SCHEMA)
                .map_err(|error| strict::in_file(source_name, error))?;
            let line = node.node_name.span().0.line + 1;
            let (slot, package) = match schema.name {
                CLASSIC_NODE => (0, &mut spec.classic),
                _ => (1, &mut spec.enhanced),
            };
            if let Some(earlier) = section_lines[slot] {
                let error = strict::error_at(node.node_name.span(),
                    format!("`{}` is given already, on line {earlier}", schema.name));
                return Err(strict::in_file(source_name, error));
            }
            section_lines[slot] = Some(line);
            *package = decode_package(node).map_err(|error| strict::in_file(source_name, error))?;
        }
        Ok(spec)
    }

    /// The filter of server body `body` of `source`, whose largest stored frame side is `largest_side` pixels (none
    /// when the package has no frames of it): its body entry's properties, else its size class's, else the package
    /// default's, else off.
    pub fn resolve(&self, source: AnimSource, body: u32, largest_side: Option<u32>) -> FrameFilter {
        match source {
            AnimSource::Classic => self.classic.resolve(body, largest_side),
            AnimSource::Enhanced => self.enhanced.resolve(body, largest_side),
        }
    }
}

fn decode_package(section: &Node<LineSpan>) -> eyre::Result<PackageFilters> {
    strict::check_properties(section, &[])?;
    if let Some(argument) = section.arguments.first() {
        let name: &str = &section.node_name;
        return Err(strict::error_at(argument.literal.span(), format!("`{name}` takes no arguments")));
    }
    let section_name: &str = &section.node_name;
    let mut package = PackageFilters::default();
    let mut default_line = None;
    for node in section.children() {
        let schema = strict::node_schema(node, Some(section_name), ENTRY_SCHEMA)?;
        strict::check_properties(node, schema.properties)?;
        strict::reject_children(node)?;
        let line = node.node_name.span().0.line + 1;
        let entry = decode_entry(node)?;
        match schema.name {
            DEFAULT_NODE => {
                if let Some(earlier) = default_line {
                    return Err(strict::error_at(node.node_name.span(),
                        format!("`{DEFAULT_NODE}` is given already, on line {earlier}")));
                }
                no_arguments(node)?;
                default_line = Some(line);
                package.default = entry;
            }
            SIZE_NODE => {
                no_arguments(node)?;
                let value = strict::required_property(node, MAX_PROPERTY)?;
                let max_side = strict::value_u32(value, MAX_PROPERTY)?;
                if !SIZE_CLASS_SIDE_RANGE.contains(&max_side) {
                    return Err(strict::error_at(value.literal.span(), format!("{MAX_PROPERTY}: {max_side} is \
                        outside {}..={}", SIZE_CLASS_SIDE_RANGE.start(), SIZE_CLASS_SIDE_RANGE.end())));
                }
                if let Some(earlier) = package.sizes.iter().find(|class| class.max_side == max_side) {
                    return Err(strict::error_at(value.literal.span(),
                        format!("{MAX_PROPERTY}={max_side} is given already, on line {}", earlier.line)));
                }
                package.sizes.push(SizeClass { max_side, entry, line });
            }
            _ => {
                let (first, last) = id_range(node)?;
                if let Some(earlier) = package.bodies.iter()
                    .find(|earlier| earlier.first <= last && first <= earlier.last) {
                    return Err(strict::error_at(node.node_name.span(), format!("body {} is listed already, on \
                        line {}", first.max(earlier.first), earlier.line)));
                }
                package.bodies.push(BodyEntry { first, last, entry, line });
            }
        }
    }
    package.sizes.sort_by_key(|class| class.max_side);
    Ok(package)
}

fn no_arguments(node: &SpannedNode<LineSpan>) -> eyre::Result<()> {
    match node.arguments.first() {
        Some(argument) => {
            let name: &str = &node.node_name;
            Err(strict::error_at(argument.literal.span(), format!("`{name}` takes no arguments")))
        }
        None => Ok(()),
    }
}

/// The filter's properties `node` gives, each in range.
fn decode_entry(node: &Node<LineSpan>) -> eyre::Result<FilterEntry> {
    let strength = |name: &str| -> eyre::Result<Option<f32>> {
        bounded(node, name, &FILTER_STRENGTH_RANGE)
    };
    let contour_color = match strict::property(node, CONTOUR_COLOR_PROPERTY) {
        None => None,
        Some(value) => Some(parse_color(&strict::value_string(value, CONTOUR_COLOR_PROPERTY)?)
            .ok_or_else(|| strict::error_at(value.literal.span(),
                format!("{CONTOUR_COLOR_PROPERTY}: expected \"#rrggbb\", six hexadecimal digits")))?),
    };
    Ok(FilterEntry {
        sharpening: strength(SHARPENING_PROPERTY)?,
        fringe_sharpening: strength(FRINGE_SHARPENING_PROPERTY)?,
        alpha_contrast: strength(ALPHA_CONTRAST_PROPERTY)?,
        alpha_midpoint: bounded(node, ALPHA_MIDPOINT_PROPERTY, &ALPHA_MIDPOINT_RANGE)?,
        contour: strength(CONTOUR_PROPERTY)?,
        contour_color,
        outline: strength(OUTLINE_PROPERTY)?,
        inner_lines: strength(INNER_LINES_PROPERTY)?,
        inner_contrast: strength(INNER_CONTRAST_PROPERTY)?,
        inner_threshold: bounded(node, INNER_THRESHOLD_PROPERTY, &INNER_THRESHOLD_RANGE)?,
    })
}

/// Property `name` of `node` if given, within `range`.
fn bounded(node: &Node<LineSpan>, name: &str, range: &RangeInclusive<f32>) -> eyre::Result<Option<f32>> {
    let Some(value) = strict::property(node, name) else { return Ok(None) };
    let number = strict::value_f32(value, name)?;
    if !range.contains(&number) {
        return Err(strict::error_at(value.literal.span(),
            format!("{name}: {number} is outside {}..={}", range.start(), range.end())));
    }
    Ok(Some(number))
}

/// `#rrggbb` as its three bytes.
fn parse_color(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#').filter(|digits| digits.len() == COLOR_HEX_DIGITS && digits.is_ascii())?;
    let channel = |index: usize| u8::from_str_radix(&digits[index * 2..index * 2 + 2], COLOR_HEX_RADIX).ok();
    Some([channel(0)?, channel(1)?, channel(2)?])
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

    const FILE: &str = "Sharpening.kdl";

    fn decode(text: &str) -> MobileSharpeningSpec {
        MobileSharpeningSpec::decode(FILE, text).unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// The error starts with the file and the line, and says what is wrong.
    fn assert_error(text: &str, line: usize, needle: &str) {
        let error = format!("{:#}", MobileSharpeningSpec::decode(FILE, text).expect_err("the file should not read"));
        assert!(error.starts_with(&format!("{FILE}: line {line}, column ")), "{error}");
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn a_property_comes_from_the_body_else_its_size_class_else_the_default() {
        let spec = decode("classic {\n  default sharpening=0.3 contour=0.2\n  size max=192 sharpening=0.6\n\
            size max=96 sharpening=0.7 alpha_contrast=0.5\n  body 17 sharpening=1.0\n  body 400 405 contour=0.0\n}\n\
            enhanced { default sharpening=0.2; }\n");
        let sharpening = |source, body, side| spec.resolve(source, body, side).sharpening;
        assert_eq!(sharpening(AnimSource::Classic, 1, Some(50)), 0.7);
        assert_eq!(sharpening(AnimSource::Classic, 1, Some(96)), 0.7);
        assert_eq!(sharpening(AnimSource::Classic, 1, Some(97)), 0.6);
        assert_eq!(sharpening(AnimSource::Classic, 1, Some(193)), 0.3, "above every class: the default");
        assert_eq!(sharpening(AnimSource::Classic, 1, None), 0.3, "no frames: the default");
        assert_eq!(sharpening(AnimSource::Classic, 17, Some(50)), 1.0);
        assert_eq!(sharpening(AnimSource::Enhanced, 17, Some(50)), 0.2, "each package its own section");
        // The body's entry gives the contour alone: its sharpening and contrast come from its size class.
        let human = spec.resolve(AnimSource::Classic, 401, Some(77));
        assert_eq!((human.sharpening, human.alpha_contrast, human.contour), (0.7, 0.5, 0.0));
        let other = spec.resolve(AnimSource::Classic, 2, Some(77));
        assert_eq!(other.contour, 0.2);
        assert_eq!((other.fringe_sharpening, other.outline, other.contour_color), (1.0, 0.0, DEFAULT_CONTOUR_COLOR));
        // Without a file or a section, the frames are as stored.
        assert_eq!(decode("").resolve(AnimSource::Enhanced, 9, Some(388)), FrameFilter::OFF);
    }

    #[test]
    fn the_contour_color_is_hexadecimal() {
        let spec = decode("enhanced { default contour=0.5 contour_color=\"#1a2B3c\"; }");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 1, None).contour_color, [0x1a, 0x2b, 0x3c]);
    }

    #[test]
    fn the_inner_edges_resolve_as_every_property() {
        let spec = decode("enhanced {\n  default inner_lines=0.6 inner_threshold=0.2\n\
            size max=96 inner_contrast=0.5\n  body 9 inner_lines=0.0 inner_contrast=1.0 inner_threshold=0.08\n}\n");
        let inner = |body, side| {
            let filter = spec.resolve(AnimSource::Enhanced, body, side);
            (filter.inner_lines, filter.inner_contrast, filter.inner_threshold)
        };
        assert_eq!(inner(1, Some(200)), (0.6, 0.0, 0.2), "the default");
        assert_eq!(inner(1, Some(80)), (0.6, 0.5, 0.2), "the size class adds the contrast");
        assert_eq!(inner(9, Some(80)), (0.0, 1.0, 0.08), "the body's entry wins");
        let classic = spec.resolve(AnimSource::Classic, 1, Some(80));
        assert_eq!((classic.inner_lines, classic.inner_contrast, classic.inner_threshold),
            (0.0, 0.0, DEFAULT_INNER_THRESHOLD), "off, at the default threshold, without a section");
        assert_error("enhanced {\n  default inner_lines=1.2\n}\n", 2, "inner_lines: 1.2 is outside 0..=1");
        assert_error("enhanced {\n  default inner_contrast=-0.1\n}\n", 2, "inner_contrast: -0.1 is outside 0..=1");
        assert_error("enhanced {\n  body 9 inner_threshold=0\n}\n", 2, "inner_threshold: 0 is outside 0.01..=1");
        assert_error("enhanced {\n  size max=96 inner_line=1\n}\n", 2, "unknown property `inner_line` of `size`");
    }

    #[test]
    fn the_bundled_file_reads() {
        let text = include_str!("../../../../assets/runtime_specs/mobiles/Sharpening.kdl");
        let spec = decode(text);
        assert_eq!(spec.resolve(AnimSource::Classic, 17, Some(80)).sharpening, 1.0, "the Classic orc");
        assert_eq!(spec.resolve(AnimSource::Classic, 400, Some(77)).sharpening, 0.7);
        assert_eq!(spec.resolve(AnimSource::Classic, 9, Some(388)).sharpening, 0.7, "a large Classic body");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 9, Some(388)).sharpening, 0.4);
        assert_eq!(spec.resolve(AnimSource::Enhanced, 17, Some(80)).sharpening, 1.0, "a small Enhanced monster");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 401, Some(150)).sharpening, 0.4, "an Enhanced human");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 401, Some(150)).outline, 1.0, "the Enhanced outline");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 17, Some(80)).outline, 1.0, "a small Enhanced monster's");
        assert_eq!(spec.resolve(AnimSource::Enhanced, 17, Some(80)).contour, 0.0, "no Enhanced contour");
        assert_eq!(spec.resolve(AnimSource::Classic, 400, Some(77)).outline, 0.0, "no Classic outline");
        // The interior lines and contrast are off everywhere until chosen.
        for (source, body, side) in [(AnimSource::Enhanced, 9, Some(388)), (AnimSource::Enhanced, 17, Some(80)),
            (AnimSource::Enhanced, 401, Some(150)), (AnimSource::Classic, 9, Some(388))] {
            let filter = spec.resolve(source, body, side);
            assert_eq!((filter.inner_lines, filter.inner_contrast), (0.0, 0.0), "{source:?} body {body}");
        }
    }

    #[test]
    fn errors_name_the_line() {
        assert_error("mobiles { }\n", 1, "unknown node `mobiles` at the top level: expected `classic` or `enhanced`");
        assert_error("classic { }\nclassic { }\n", 2, "`classic` is given already, on line 1");
        assert_error("classic 1 { }\n", 1, "`classic` takes no arguments");
        assert_error("classic {\n  dog sharpening=1\n}\n", 2, "unknown node `dog` in `classic`");
        assert_error("classic {\n  default sharpness=1\n}\n", 2, "unknown property `sharpness` of `default`");
        assert_error("classic {\n  default sharpening=1.5\n}\n", 2, "sharpening: 1.5 is outside 0..=1");
        assert_error("classic {\n  default alpha_midpoint=1\n}\n", 2, "alpha_midpoint: 1 is outside 0.05..=0.95");
        assert_error("classic {\n  default\n  default\n}\n", 3, "`default` is given already, on line 2");
        assert_error("classic {\n  default 3\n}\n", 2, "`default` takes no arguments");
        assert_error("classic {\n  size sharpening=1\n}\n", 2, "`size`: missing max=");
        assert_error("classic {\n  size max=96\n  size max=96\n}\n", 3, "max=96 is given already, on line 2");
        assert_error("classic {\n  size max=0\n}\n", 2, "max: 0 is outside 1..=4096");
        assert_error("classic {\n  body 9 3\n}\n", 2, "the range ends at 3, before its first id 9");
        assert_error("classic {\n  body sharpening=1\n}\n", 2, "found 0 arguments");
        assert_error("classic {\n  body 10 20\n  body 5 12\n}\n", 3, "body 10 is listed already, on line 2");
        assert_error("classic {\n  body 1 { body 2; }\n}\n", 2, "`body` takes no children");
        assert_error("classic {\n  body 1 contour_color=\"#12345\"\n}\n", 2, "contour_color: expected \"#rrggbb\"");
        assert_error("classic {\n  body 1 contour_color=\"123456\"\n}\n", 2, "contour_color: expected \"#rrggbb\"");
    }
}
