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

#[derive(Decode, Debug, Clone)]
pub struct LayerDef {
    #[knuffel(argument)]
    pub id: u32,
    #[knuffel(argument)]
    pub stretch: f32,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainDefEntry {
    #[knuffel(argument)]
    pub id: u32,
    #[knuffel(argument)]
    pub terrain_type: String,
    
    #[knuffel(property)]
    pub speed: Option<f32>,
    #[knuffel(property)]
    pub waveheight: Option<f32>,
    #[knuffel(property)]
    pub textureid: Option<u32>,

    #[knuffel(child)]
    pub t0: Option<LayerDef>,
    #[knuffel(child)]
    pub t1: Option<LayerDef>,
    #[knuffel(child)]
    pub m: Option<LayerDef>,
    #[knuffel(child)]
    pub s: Option<LayerDef>,
    #[knuffel(child)]
    pub n: Option<LayerDef>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainDefinitionKdl {
    #[knuffel(children(name = "d"))]
    pub entries: Vec<TerrainDefEntry>,
}

impl TerrainDefinitionKdl {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        let content = strip_c_style_block_comments(&content);
        let content = strip_cpp_style_line_comments(&content);
        let source_name = path.as_ref().to_str().unwrap_or(TERRAIN_DEFINITION_SOURCE_NAME);
        parse_terrain_definition_kdl(&content).map_err(|error| udd_kdl::strict::in_file(source_name, error))
    }

    pub fn to_map(&self) -> HashMap<u32, TerrainDefEntry> {
        self.entries.iter().map(|e| (e.id, e.clone())).collect()
    }
}

/// Name given to the KDL parser for its diagnostics.
const TERRAIN_DEFINITION_SOURCE_NAME: &str = "TerrainDefinition.kdl";
/// `d <terrain id> "<type flags>" [speed=] [waveheight=] [textureid=] { <role> <texture id> <stretch> }`.
const TERRAIN_DEFINITION_NODE: &str = "d";
const TERRAIN_DEFINITION_PROPERTIES: [&str; 3] = ["speed", "waveheight", "textureid"];
const TERRAIN_LAYER_ROLES: [&str; 5] = ["t0", "t1", "m", "s", "n"];
const TERRAIN_DEFINITION_SCHEMA: &[NodeSchema] = &[NodeSchema {
    name: TERRAIN_DEFINITION_NODE,
    properties: &TERRAIN_DEFINITION_PROPERTIES,
    children: &[
        NodeSchema::leaf(TERRAIN_LAYER_ROLES[0], &[]),
        NodeSchema::leaf(TERRAIN_LAYER_ROLES[1], &[]),
        NodeSchema::leaf(TERRAIN_LAYER_ROLES[2], &[]),
        NodeSchema::leaf(TERRAIN_LAYER_ROLES[3], &[]),
        NodeSchema::leaf(TERRAIN_LAYER_ROLES[4], &[]),
    ],
}];

fn parse_terrain_definition_kdl(content: &str) -> eyre::Result<TerrainDefinitionKdl> {
    use udd_kdl::ast::Node;
    use udd_kdl::span::LineSpan;
    use udd_kdl::strict::{arguments, error_at, property, value_f32, value_string, value_u32};

    fn layer_from_node(node: &Node<LineSpan>) -> eyre::Result<LayerDef> {
        let [id, stretch] = arguments::<2>(node)?;
        Ok(LayerDef { id: value_u32(id, "layer id")?, stretch: value_f32(stretch, "layer stretch")? })
    }

    let document = udd_kdl::parse_ast_line_span(TERRAIN_DEFINITION_SOURCE_NAME, content)
        .map_err(|error| udd_kdl::strict::describe_error(content, &error))?;
    udd_kdl::strict::check_document(&document, TERRAIN_DEFINITION_SCHEMA)?;

    let mut entries = Vec::new();
    for node in &document.nodes {
        let [id, terrain_type] = arguments::<2>(node)?;
        let mut entry = TerrainDefEntry {
            id: value_u32(id, "terrain id")?,
            terrain_type: value_string(terrain_type, "terrain type")?,
            speed: property(node, "speed").map(|value| value_f32(value, "speed")).transpose()?,
            waveheight: property(node, "waveheight").map(|value| value_f32(value, "waveheight")).transpose()?,
            textureid: property(node, "textureid").map(|value| value_u32(value, "textureid")).transpose()?,
            t0: None,
            t1: None,
            m: None,
            s: None,
            n: None,
        };

        for child in node.children() {
            let role: &str = &child.node_name;
            let slot = match role {
                "t0" => &mut entry.t0,
                "t1" => &mut entry.t1,
                "m" => &mut entry.m,
                "s" => &mut entry.s,
                "n" => &mut entry.n,
                _ => unreachable!("the schema check admits only layer roles"),
            };
            if slot.is_some() {
                return Err(error_at(child.node_name.span(), format!("second `{role}` layer in the same `d`")));
            }
            *slot = Some(layer_from_node(child)?);
        }

        entries.push(entry);
    }

    Ok(TerrainDefinitionKdl { entries })
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
    fn snow_terrain_transcode_overrides_parse() {
        let content = include_str!(
            "../../../dynamapper/assets/cc_ec_convtables/TerrainTranscodeSnowOverrides.kdl"
        );
        let parsed = EckrTerrainRouting::from_str("TerrainTranscodeSnowOverrides.kdl", content)
            .expect("parse snow terrain transcode overrides");
        let map = parsed.to_map();

        assert_eq!(parsed.entries.len(), 10);
        assert_eq!(map.get(&3), Some(&8));
        assert_eq!(map.get(&196), Some(&8));
        assert_eq!(map.get(&244), Some(&8));
        assert_eq!(map.get(&361), Some(&8));
        assert_eq!(map.get(&36), Some(&63));
        assert_eq!(map.get(&742), Some(&61));
        assert_eq!(map.get(&141), None);
    }

    #[test]
    fn eckr_terrain_routing_accepts_generated_property_rows() {
        let parsed = EckrTerrainRouting::from_str(
            "KrFacetTranscode.generated.kdl",
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
            "KrTerrainRouting.generated.kdl",
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
    fn kr_generated_routing_file_parses() {
        let content = include_str!(
            "../../../dynamapper/assets/cc_ec_convtables/KrTerrainRouting.generated.kdl"
        );
        let parsed = EckrTerrainRouting::from_str("KrTerrainRouting.generated.kdl", content)
            .expect("parse generated KR routing");
        let map = parsed.to_map();

        assert_eq!(parsed.entries.len(), 175);
        assert_eq!(map.len(), 1454);
        assert_eq!(map.get(&168), Some(&5));
        assert_eq!(map.get(&3), Some(&1));
    }

    #[test]
    fn terrain_definition_loader_accepts_header_block_comments() {
        let content = r#"
/*
    Header prose for humans.
*/
d 5 "Liquid,Smooth" speed=0 waveheight=0.3 {
    t0 01000017 30
}
"#;
        let stripped = strip_c_style_block_comments(content);
        let stripped = strip_cpp_style_line_comments(&stripped);
        let parsed = parse_terrain_definition_kdl(&stripped).expect("parse stripped KDL");

        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].id, 5);
        assert_eq!(parsed.entries[0].speed, Some(0.0));
        assert_eq!(parsed.entries[0].waveheight, Some(0.3));
    }

    fn routing_error(content: &str) -> String {
        EckrTerrainRouting::from_str("routing.kdl", content).expect_err("routing error").to_string()
    }

    #[test]
    fn bundled_routing_files_parse() {
        for (name, content) in [
            ("TerrainTranscode.kdl", include_str!("../../../dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl")),
            (
                "EcFacetTranscode.generated.kdl",
                include_str!("../../../dynamapper/assets/cc_ec_convtables/EcFacetTranscode.generated.kdl"),
            ),
            (
                "KrFacetTranscode.generated.kdl",
                include_str!("../../../dynamapper/assets/cc_ec_convtables/KrFacetTranscode.generated.kdl"),
            ),
        ] {
            let parsed = EckrTerrainRouting::from_str(name, content).unwrap_or_else(|error| panic!("{error}"));
            assert!(!parsed.entries.is_empty(), "{name}");
        }
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

    #[test]
    fn legacy_terrain_definition_file_loads() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dynamapper/assets/cc_ec_convtables/legacy/TerrainDefinition.kdl");
        let parsed = TerrainDefinitionKdl::load(path).unwrap_or_else(|error| panic!("{error}"));
        assert!(!parsed.entries.is_empty());
    }

    #[test]
    fn terrain_definition_rejects_unknown_layers_on_their_line() {
        let content = r#"
/*
    Header prose for humans.
*/
d 5 "Liquid,Smooth" {
    t0 01000017 30
    t2 01000018 30
}
"#;
        let stripped = strip_c_style_block_comments(content);
        let stripped = strip_cpp_style_line_comments(&stripped);
        assert_eq!(
            parse_terrain_definition_kdl(&stripped).expect_err("unknown layer").to_string(),
            "line 7, column 5: unknown node `t2` in `d`: expected one of `t0`, `t1`, `m`, `s`, `n`"
        );

        let duplicate = "d 5 \"Liquid\" {\n    t0 1 30\n    t0 2 30\n}\n";
        assert_eq!(
            parse_terrain_definition_kdl(duplicate).expect_err("duplicate layer").to_string(),
            "line 3, column 5: second `t0` layer in the same `d`"
        );
        assert_eq!(
            parse_terrain_definition_kdl("d 5 \"Liquid\" wave=1\n").expect_err("unknown property").to_string(),
            "line 1, column 14: unknown property `wave` of `d`: expected one of `speed`, `waveheight`, `textureid`"
        );
    }
}
