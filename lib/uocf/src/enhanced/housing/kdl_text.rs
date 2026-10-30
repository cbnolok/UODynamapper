//! KDL (v1) text form of [`HousingPieces`], for diffs and hand editing. It keeps everything `housing.bin` holds,
//! in file order, so `bytes -> KDL -> bytes` is the identity.
//!
//! ```kdl
//! tile-type 1 name="stairs" {
//!     piece category=0 mask=0x1 tooltip=1021826 {
//!         solid 1 0x0739 2 0x073C 9 0x0738
//!         multi 1 0x1DB0 2 0x1DB1
//!     }
//! }
//! tile-type 5 name="wall" {
//!     piece category=2 group=1 mask=0x40
//! }
//! ```
//!
//! - `tile-type <tile type>`: one per tile type entry, in file order. `name=` is informational; when present it
//!   must match the tile type.
//! - `piece`: one per piece, in file order. `category=` and `mask=` are required; `group=` and `tooltip=`
//!   default to 0 and are written only when nonzero.
//! - `solid`, `multi`, `window`: the slot lists as `<slot> <id>` pairs in list order, each list at most once per
//!   piece, in any order; an absent list is empty. The writer omits empty lists and the braces of empty nodes.
//! - Integers may be decimal or `0x` hex. The writer uses hex for ids and masks, decimal for the rest.
//! - Errors name the line and column (1-based) of the offending node or value.

use std::fmt::Write as _;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::ast::{Literal, Node, SpannedNode, Value};
use knuffel::span::LineSpan;

use super::{housing_tile_type_name, HousingPiece, HousingPieces, HousingSlot, HousingSlotList, HousingTileType};

const TILE_TYPE_NODE: &str = "tile-type";
const PIECE_NODE: &str = "piece";
const NAME_PROPERTY: &str = "name";
const CATEGORY_PROPERTY: &str = "category";
const GROUP_PROPERTY: &str = "group";
const MASK_PROPERTY: &str = "mask";
const TOOLTIP_PROPERTY: &str = "tooltip";
const TILE_TYPE_PROPERTIES: [&str; 1] = [NAME_PROPERTY];
const PIECE_PROPERTIES: [&str; 4] = [CATEGORY_PROPERTY, GROUP_PROPERTY, MASK_PROPERTY, TOOLTIP_PROPERTY];
/// Name given to the KDL parser for its diagnostics.
const KDL_SOURCE_NAME: &str = "housing.kdl";
const INDENT: &str = "    ";
/// Values per slot in a list node: slot, id.
const VALUES_PER_SLOT: usize = 2;

const KDL_HEADER: &str = "\
// House customization pieces (build/multicollection/housing.bin), uocf::enhanced::housing.
// tile-type <type> [name=informational]; piece category=<row> [group=<sub-row>] mask=<feature bits> [tooltip=<cliloc>];
// solid / multi / window: <slot> <id> pairs in list order. Omitted group, tooltip and lists are 0 / empty.
";

impl HousingPieces {
    /// The KDL text form (schema in the module docs), starting with a short comment header.
    pub fn to_kdl_string(&self) -> String {
        let mut text = String::from(KDL_HEADER);
        for entry in &self.tile_types {
            text.push_str(TILE_TYPE_NODE);
            write!(text, " {}", entry.tile_type).unwrap();
            if let Some(name) = housing_tile_type_name(entry.tile_type) {
                write!(text, " {NAME_PROPERTY}=\"{name}\"").unwrap();
            }
            if entry.pieces.is_empty() {
                text.push('\n');
                continue;
            }
            text.push_str(" {\n");
            for piece in &entry.pieces {
                write_piece(&mut text, piece);
            }
            text.push_str("}\n");
        }
        text
    }

    /// Parses the KDL text form (schema in the module docs).
    pub fn from_kdl_str(text: &str) -> eyre::Result<Self> {
        let document =
            knuffel::parse_ast::<LineSpan>(KDL_SOURCE_NAME, text).map_err(|error| syntax_error(text, &error))?;
        let tile_types = document
            .nodes
            .iter()
            .map(|node| {
                expect_node_name(node, TILE_TYPE_NODE)?;
                parse_tile_type(node)
            })
            .collect::<eyre::Result<_>>()?;
        Ok(Self { tile_types })
    }

    /// Reads a KDL file ([`Self::from_kdl_str`]).
    pub fn load_kdl(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::from_kdl_str(&text).wrap_err_with(|| format!("failed to parse {}", path.display()))
    }

    /// Writes a KDL file ([`Self::to_kdl_string`]).
    pub fn write_kdl(&self, path: impl AsRef<Path>) -> eyre::Result<()> {
        let path = path.as_ref();
        std::fs::write(path, self.to_kdl_string()).wrap_err_with(|| format!("failed to write {}", path.display()))
    }
}

fn write_piece(text: &mut String, piece: &HousingPiece) {
    write!(text, "{INDENT}{PIECE_NODE} {CATEGORY_PROPERTY}={}", piece.category).unwrap();
    if piece.group != 0 {
        write!(text, " {GROUP_PROPERTY}={}", piece.group).unwrap();
    }
    write!(text, " {MASK_PROPERTY}={:#X}", piece.feature_mask).unwrap();
    if piece.tooltip_cliloc != 0 {
        write!(text, " {TOOLTIP_PROPERTY}={}", piece.tooltip_cliloc).unwrap();
    }
    let lists: Vec<_> = HousingSlotList::ALL.into_iter().filter(|&list| !piece.slots(list).is_empty()).collect();
    if lists.is_empty() {
        text.push('\n');
        return;
    }
    text.push_str(" {\n");
    for list in lists {
        write!(text, "{INDENT}{INDENT}{}", list.name()).unwrap();
        for slot in piece.slots(list) {
            write!(text, " {} {:#06X}", slot.slot, slot.id).unwrap();
        }
        text.push('\n');
    }
    writeln!(text, "{INDENT}}}").unwrap();
}

fn parse_tile_type(node: &SpannedNode<LineSpan>) -> eyre::Result<HousingTileType> {
    check_properties(node, &TILE_TYPE_PROPERTIES)?;
    let [tile_type] = arguments::<1>(node)?;
    let tile_type = value_u32(tile_type, "tile type")?;
    if let Some(name) = property(node, NAME_PROPERTY) {
        let Literal::String(name_text) = &*name.literal else {
            return Err(error_at(name.literal.span(), "name: expected a string"));
        };
        let expected = housing_tile_type_name(tile_type);
        if expected != Some(&**name_text) {
            let expected =
                expected.map_or_else(|| "no name (unknown tile type)".to_owned(), |name| format!("\"{name}\""));
            return Err(error_at(
                name.literal.span(),
                format!("name \"{name_text}\" does not match tile type {tile_type}: expected {expected}"),
            ));
        }
    }
    let pieces = node
        .children()
        .map(|child| {
            expect_node_name(child, PIECE_NODE)?;
            parse_piece(child)
        })
        .collect::<eyre::Result<_>>()?;
    Ok(HousingTileType { tile_type, pieces })
}

fn parse_piece(node: &SpannedNode<LineSpan>) -> eyre::Result<HousingPiece> {
    check_properties(node, &PIECE_PROPERTIES)?;
    arguments::<0>(node)?;
    let required = |name: &str| {
        property(node, name).ok_or_else(|| error_at(node.node_name.span(), format!("{PIECE_NODE}: missing {name}=")))
    };
    let optional = |name: &str| property(node, name).map_or(Ok(0), |value| value_u32(value, name));
    let mut piece = HousingPiece {
        category: value_u32(required(CATEGORY_PROPERTY)?, CATEGORY_PROPERTY)?,
        group: optional(GROUP_PROPERTY)?,
        feature_mask: value_u32(required(MASK_PROPERTY)?, MASK_PROPERTY)?,
        tooltip_cliloc: optional(TOOLTIP_PROPERTY)?,
        solid: Vec::new(),
        multis: Vec::new(),
        windows: Vec::new(),
    };

    let mut seen_lists = Vec::new();
    for child in node.children() {
        let name: &str = &child.node_name;
        let list = HousingSlotList::from_name(name).ok_or_else(|| {
            let message = format!("unknown node `{name}` in {PIECE_NODE}: expected solid, multi or window");
            error_at(child.node_name.span(), message)
        })?;
        if seen_lists.contains(&list) {
            return Err(error_at(child.node_name.span(), format!("second `{name}` list in the same {PIECE_NODE}")));
        }
        seen_lists.push(list);
        *piece.slots_mut(list) = parse_slots(child)?;
    }
    Ok(piece)
}

fn parse_slots(node: &SpannedNode<LineSpan>) -> eyre::Result<Vec<HousingSlot>> {
    let name: &str = &node.node_name;
    check_properties(node, &[])?;
    reject_type_name(node)?;
    if let Some(child) = node.children().next() {
        return Err(error_at(child.node_name.span(), format!("`{name}` takes no children")));
    }
    if node.arguments.len() % VALUES_PER_SLOT != 0 {
        return Err(error_at(
            node.node_name.span(),
            format!("`{name}`: {} values, expected <slot> <id> pairs", node.arguments.len()),
        ));
    }
    node.arguments
        .chunks_exact(VALUES_PER_SLOT)
        .map(|pair| Ok(HousingSlot { slot: value_u32(&pair[0], "slot")?, id: value_u32(&pair[1], "id")? }))
        .collect()
}

fn expect_node_name(node: &SpannedNode<LineSpan>, expected: &str) -> eyre::Result<()> {
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

fn value_u32(value: &Value<LineSpan>, field: &str) -> eyre::Result<u32> {
    let span = value.literal.span();
    if let Some(type_name) = &value.type_name {
        return Err(error_at(type_name.span(), "type annotations are not supported"));
    }
    match &*value.literal {
        Literal::Int(integer) => {
            u32::try_from(integer).map_err(|error| error_at(span, format!("{field}: not a u32 ({error})")))
        }
        _ => Err(error_at(span, format!("{field}: expected an integer"))),
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
    use crate::enhanced::housing::{
        HOUSING_FEATURE_NO_EXPANSION, HOUSING_FEATURE_TIME_OF_LEGENDS, HOUSING_TILE_TYPE_STAIRS, HOUSING_TILE_TYPE_WALL,
    };

    const UNKNOWN_TILE_TYPE: u32 = 9;
    const STAIRS_TOOLTIP: u32 = 1021826;

    fn sample() -> HousingPieces {
        HousingPieces {
            tile_types: vec![
                HousingTileType {
                    tile_type: HOUSING_TILE_TYPE_STAIRS,
                    pieces: vec![HousingPiece {
                        category: 0,
                        group: 0,
                        feature_mask: HOUSING_FEATURE_NO_EXPANSION,
                        tooltip_cliloc: STAIRS_TOOLTIP,
                        solid: vec![HousingSlot { slot: 1, id: 0x0739 }, HousingSlot { slot: 9, id: 0x0738 }],
                        multis: vec![HousingSlot { slot: 1, id: 7600 }],
                        windows: Vec::new(),
                    }],
                },
                HousingTileType {
                    tile_type: HOUSING_TILE_TYPE_WALL,
                    pieces: vec![
                        HousingPiece {
                            category: 2,
                            group: 1,
                            feature_mask: HOUSING_FEATURE_TIME_OF_LEGENDS,
                            tooltip_cliloc: 0,
                            solid: Vec::new(),
                            multis: Vec::new(),
                            // Repeated slot and unsorted order survive the round trip.
                            windows: vec![HousingSlot { slot: 6, id: 0x11 }, HousingSlot { slot: 6, id: 0x10 }],
                        },
                        HousingPiece {
                            category: 3,
                            group: 0,
                            feature_mask: 0,
                            tooltip_cliloc: 0,
                            solid: Vec::new(),
                            multis: Vec::new(),
                            windows: Vec::new(),
                        },
                    ],
                },
                HousingTileType { tile_type: UNKNOWN_TILE_TYPE, pieces: Vec::new() },
            ],
        }
    }

    fn parse_error(text: &str) -> String {
        format!("{:#}", HousingPieces::from_kdl_str(text).unwrap_err())
    }

    #[test]
    fn writes_the_compact_form() {
        let text = sample().to_kdl_string();
        let body = text.strip_prefix(KDL_HEADER).unwrap();

        assert_eq!(
            body,
            "tile-type 1 name=\"stairs\" {\n\
             \x20   piece category=0 mask=0x1 tooltip=1021826 {\n\
             \x20       solid 1 0x0739 9 0x0738\n\
             \x20       multi 1 0x1DB0\n\
             \x20   }\n\
             }\n\
             tile-type 5 name=\"wall\" {\n\
             \x20   piece category=2 group=1 mask=0x400000 {\n\
             \x20       window 6 0x0011 6 0x0010\n\
             \x20   }\n\
             \x20   piece category=3 mask=0x0\n\
             }\n\
             tile-type 9\n"
        );
    }

    #[test]
    fn round_trips_through_kdl() {
        let pieces = sample();

        let parsed = HousingPieces::from_kdl_str(&pieces.to_kdl_string()).unwrap();

        assert_eq!(parsed, pieces);
        assert_eq!(parsed.to_payload_bytes(), pieces.to_payload_bytes());
    }

    #[test]
    fn accepts_comments_decimal_ids_and_any_list_order() {
        let text = "// comment\n\
                    tile-type 1 { /* inline */ piece mask=1 category=0 { multi 1 7600; solid 1 1849; }; }\n";

        let parsed = HousingPieces::from_kdl_str(text).unwrap();

        let piece = &parsed.tile_types[0].pieces[0];
        assert_eq!(piece.solid, [HousingSlot { slot: 1, id: 1849 }]);
        assert_eq!(piece.multis, [HousingSlot { slot: 1, id: 7600 }]);
        assert_eq!((piece.group, piece.tooltip_cliloc), (0, 0));
    }

    #[test]
    fn errors_name_line_and_column() {
        let cases = [
            ("tile-type 1 name=\"floor\"", "line 1, column 18: name \"floor\" does not match tile type 1"),
            ("tile-type 1 {\n    piece category=0\n}", "line 2, column 5: piece: missing mask="),
            ("tile-type 1 {\n    piece category=0 mask=1 {\n        solid 1\n    }\n}", "line 3, column 9: `solid`: 1 values"),
            ("tile-type 1 {\n    piece category=0 mask=1 grp=2\n}", "line 2, column 29: unknown property `grp`"),
            ("tile-type 1 {\n    piece category=0 mask=1 {\n        solid 1 2\n        solid 3 4\n    }\n}", "line 4, column 9: second `solid` list"),
            ("tile-type 1 {\n    piece category=-1 mask=1\n}", "line 2, column 20: category: not a u32"),
            ("tile-type 1 {\n    piece category=\"x\" mask=1\n}", "line 2, column 20: category: expected an integer"),
            ("tile-types 1", "line 1, column 1: unknown node `tile-types`"),
            ("tile-type 1 2", "line 1, column 13: `tile-type` takes 1 arguments, found 2"),
            ("tile-type 1 {\n    piece category=0 mask=1 {\n        door 1 2\n    }\n}", "line 3, column 9: unknown node `door`"),
        ];
        for (text, expected) in cases {
            let error = parse_error(text);
            assert!(error.contains(expected), "{text:?}: {error}");
        }
    }

    #[test]
    fn syntax_errors_name_line_and_column() {
        let error = parse_error("tile-type 1 {\n    piece category=0 mask=1 {\n");

        assert!(error.starts_with("invalid KDL: line "), "{error}");
    }
}
