use std::collections::HashMap;
use std::path::Path;
use knuffel::Decode;
use color_eyre::eyre::{self, Context};
use udd_kdl::strict::NodeSchema;

#[derive(Decode, Debug, Clone)]
pub struct EckrTerrainRouteEntry {
    #[knuffel(argument)]
    pub new_id: u32,
    #[knuffel(arguments)]
    pub old_ids: Vec<u32>,
}

#[derive(Decode, Debug, Clone)]
pub struct EckrTerrainRouting {
    #[knuffel(children(name = "t"))]
    pub entries: Vec<EckrTerrainRouteEntry>,
}

impl EckrTerrainRouting {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        Self::from_str(path.as_ref().to_str().unwrap_or("transcode.kdl"), &content)
    }

    pub fn from_str(source_name: &str, content: &str) -> eyre::Result<Self> {
        let content = strip_c_style_block_comments(content);
        let content = strip_cpp_style_line_comments(&content);
        parse_eckr_terrain_routing_kdl(source_name, &content)
            .map_err(|error| udd_kdl::strict::in_file(source_name, error))
    }

    pub fn to_map(&self) -> HashMap<u32, u32> {
        let mut map = HashMap::new();
        for entry in &self.entries {
            for &old_id in &entry.old_ids {
                map.insert(old_id, entry.new_id);
            }
        }
        map
    }
}

/// A routing row: `t <target id> <source id>...`, or `t cc=<source id> kr|ec|target=<target id> [count=<tiles>]`.
const ROUTE_ROW_NODE: &str = "t";
const ROUTE_ROW_SOURCE_PROPERTY: &str = "cc";
/// Target id properties of a property row, one per row; the generated files name the target client.
const ROUTE_ROW_TARGET_PROPERTIES: [&str; 3] = ["kr", "ec", "target"];
/// Number of facet tiles behind a generated row: evidence, not read.
const ROUTE_ROW_COUNT_PROPERTY: &str = "count";
const ROUTE_ROW_PROPERTIES: [&str; 5] = [
    ROUTE_ROW_SOURCE_PROPERTY,
    ROUTE_ROW_TARGET_PROPERTIES[0],
    ROUTE_ROW_TARGET_PROPERTIES[1],
    ROUTE_ROW_TARGET_PROPERTIES[2],
    ROUTE_ROW_COUNT_PROPERTY,
];

/// Groups generated property rows.
const FACET_TRANSCODES_NODE: &str = "facet_transcodes";

/// The routing files' nodes. `route` and `source` are generator headers, `facet_transcodes` groups generated rows,
/// `terrain_definition` is source evidence in generated facet files; only the `t` rows are read.
const ROUTING_SCHEMA: &[NodeSchema] = &[
    NodeSchema::leaf(ROUTE_ROW_NODE, &ROUTE_ROW_PROPERTIES),
    NodeSchema::leaf("route", &["client", "source"]),
    NodeSchema::leaf(
        "source",
        &["client", "map_index", "facet", "width", "height", "cc_map", "terrain_definition", "kr_dictionary_csv"],
    ),
    NodeSchema {
        name: FACET_TRANSCODES_NODE,
        properties: &[],
        children: &[NodeSchema::leaf(ROUTE_ROW_NODE, &ROUTE_ROW_PROPERTIES)],
    },
    NodeSchema {
        name: "terrain_definition",
        properties: &[],
        children: &[NodeSchema {
            name: "material",
            properties: &["id", "name", "primary_texture_id"],
            children: &[
                NodeSchema::leaf("alias", &["id", "count_index", "flags"]),
                NodeSchema::leaf("shader", &["name"]),
                NodeSchema::leaf("layer", &["texture_id", "path", "repetition", "type"]),
            ],
        }],
    },
];

fn parse_eckr_terrain_routing_kdl(source_name: &str, content: &str) -> eyre::Result<EckrTerrainRouting> {
    use udd_kdl::ast::SpannedNode;
    use udd_kdl::span::LineSpan;
    use udd_kdl::strict::{error_at, property, value_u32};

    fn route_row(node: &SpannedNode<LineSpan>) -> eyre::Result<EckrTerrainRouteEntry> {
        if let Some(source) = property(node, ROUTE_ROW_SOURCE_PROPERTY) {
            udd_kdl::strict::arguments::<0>(node)?;
            let mut targets = ROUTE_ROW_TARGET_PROPERTIES
                .iter()
                .filter_map(|&name| property(node, name).map(|value| (name, value)));
            let (target_name, target) = targets.next().ok_or_else(|| {
                let names = ROUTE_ROW_TARGET_PROPERTIES.map(|name| format!("{name}=")).join(", ");
                error_at(node.node_name.span(), format!("`{ROUTE_ROW_NODE}`: missing the target id, one of {names}"))
            })?;
            if let Some((second_name, second)) = targets.next() {
                return Err(error_at(
                    second.literal.span(),
                    format!("`{ROUTE_ROW_NODE}`: {target_name}= and {second_name}= both set; give one target id"),
                ));
            }
            if let Some(count) = property(node, ROUTE_ROW_COUNT_PROPERTY) {
                value_u32(count, ROUTE_ROW_COUNT_PROPERTY)?;
            }
            return Ok(EckrTerrainRouteEntry {
                new_id: value_u32(target, target_name)?,
                old_ids: vec![value_u32(source, ROUTE_ROW_SOURCE_PROPERTY)?],
            });
        }

        udd_kdl::strict::check_properties(node, &[])?;
        let Some((new_id, old_ids)) = node.arguments.split_first() else {
            return Err(error_at(node.node_name.span(), format!("`{ROUTE_ROW_NODE}`: missing target id")));
        };
        let new_id = value_u32(new_id, "target id")?;
        if old_ids.is_empty() {
            return Err(error_at(node.node_name.span(), format!("`{ROUTE_ROW_NODE}` {new_id}: missing source ids")));
        }
        let old_ids = old_ids.iter().map(|value| value_u32(value, "source id")).collect::<eyre::Result<Vec<_>>>()?;
        Ok(EckrTerrainRouteEntry { new_id, old_ids })
    }

    let document = udd_kdl::parse_ast_line_span(source_name, content)
        .map_err(|error| udd_kdl::strict::describe_error(content, &error))?;
    udd_kdl::strict::check_document(&document, ROUTING_SCHEMA)?;
    let mut entries = Vec::new();
    for node in &document.nodes {
        match &**node.node_name {
            ROUTE_ROW_NODE => entries.push(route_row(node)?),
            FACET_TRANSCODES_NODE => {
                for row in node.children() {
                    entries.push(route_row(row)?);
                }
            }
            _ => {}
        }
    }
    Ok(EckrTerrainRouting { entries })
}

/// Removes `/* */` comments, keeping their line breaks so error lines still match the file.
fn strip_c_style_block_comments(content: &str) -> String {
    let mut stripped = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("*/") else {
            return stripped;
        };
        stripped.extend(after_start[..end].matches('\n'));
        rest = &after_start[end + 2..];
    }
    stripped.push_str(rest);
    stripped
}

fn strip_cpp_style_line_comments(content: &str) -> String {
    let mut stripped = String::with_capacity(content.len());
    for line in content.lines() {
        stripped.push_str(line.split_once("//").map(|(before, _)| before).unwrap_or(line));
        stripped.push('\n');
    }
    stripped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eckr_terrain_routing_accepts_generated_property_rows() {
        let parsed = EckrTerrainRouting::from_str(
            "routing.kdl",
            r#"
source client="kr"
facet_transcodes {
    t cc=168 kr=5 count=1
    t cc=169 kr=5 count=1
    t cc=170 kr=6 count=1
}
"#,
        )
        .expect("parse generated routing rows");
        let map = parsed.to_map();

        assert_eq!(parsed.entries.len(), 3);
        assert_eq!(map.get(&168), Some(&5));
        assert_eq!(map.get(&169), Some(&5));
        assert_eq!(map.get(&170), Some(&6));
    }

    #[test]
    fn eckr_terrain_routing_accepts_grouped_routing_rows() {
        let parsed = EckrTerrainRouting::from_str(
            "routing.kdl",
            r#"
route client="kr" source="manawydan-tile-dictionary"
t 5 168 169
t 6 170
"#,
        )
        .expect("parse grouped routing rows");
        let map = parsed.to_map();

        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(map.get(&168), Some(&5));
        assert_eq!(map.get(&169), Some(&5));
        assert_eq!(map.get(&170), Some(&6));
    }

    #[test]
    fn routing_loader_accepts_header_block_comments() {
        let parsed = EckrTerrainRouting::from_str(
            "routing.kdl",
            r#"
/*
    Header prose for humans.
*/
t 5 168 // trailing note
"#,
        )
        .expect("parse routing with comments");

        assert_eq!(parsed.to_map().get(&168), Some(&5));
    }

    fn routing_error(content: &str) -> String {
        EckrTerrainRouting::from_str("routing.kdl", content).expect_err("routing error").to_string()
    }

    #[test]
    fn bundled_routing_file_parses() {
        const NAME: &str = "TerrainTranscode.kdl";
        let content = include_str!("../../../dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl");
        let parsed = EckrTerrainRouting::from_str(NAME, content).unwrap_or_else(|error| panic!("{error}"));
        assert!(!parsed.entries.is_empty(), "{NAME}");
    }

    #[test]
    fn routing_rejects_unknown_nodes_and_properties_with_their_position() {
        assert_eq!(
            routing_error("t 1 2\nrow 3 4\n"),
            "routing.kdl: line 2, column 1: unknown node `row` at the top level: expected one of `t`, `route`, \
             `source`, `facet_transcodes`, `terrain_definition`"
        );
        assert_eq!(
            routing_error("facet_transcodes {\n    t cc=1 kr=2 tiles=3\n}\n"),
            "routing.kdl: line 2, column 17: unknown property `tiles` of `t`: expected one of `cc`, `kr`, `ec`, \
             `target`, `count`"
        );
        assert_eq!(
            routing_error("facet_transcodes {\n    row cc=1 kr=2\n}\n"),
            "routing.kdl: line 2, column 5: unknown node `row` in `facet_transcodes`: expected `t`"
        );
        assert_eq!(
            routing_error("t 5 168 {\n    t 6 170\n}\n"),
            "routing.kdl: line 2, column 5: `t` takes no children, found `t`"
        );
    }

    #[test]
    fn routing_rejects_incomplete_or_mixed_rows() {
        assert_eq!(routing_error("t 5\n"), "routing.kdl: line 1, column 1: `t` 5: missing source ids");
        assert_eq!(
            routing_error("t cc=168 count=1\n"),
            "routing.kdl: line 1, column 1: `t`: missing the target id, one of kr=, ec=, target="
        );
        assert_eq!(
            routing_error("t cc=168 kr=5 ec=6\n"),
            "routing.kdl: line 1, column 18: `t`: kr= and ec= both set; give one target id"
        );
        assert_eq!(
            routing_error("t 5 168 kr=5\n"),
            "routing.kdl: line 1, column 9: `t` takes no properties, found `kr`"
        );
        assert_eq!(
            routing_error("t 7 cc=168 kr=5\n"),
            "routing.kdl: line 1, column 3: `t` takes 0 arguments, found 1"
        );
    }
}
