use std::collections::HashMap;
use std::path::Path;
use knuffel::Decode;
use color_eyre::eyre::{self, Context};

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
            .wrap_err("Failed to parse EC/KR terrain routing KDL")
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

pub type TranscodeEntry = EckrTerrainRouteEntry;
pub type TerrainTranscode = EckrTerrainRouting;

fn parse_eckr_terrain_routing_kdl(source_name: &str, content: &str) -> eyre::Result<EckrTerrainRouting> {
    use knuffel::ast::{Literal, Node, Value};
    use knuffel::span::{Span, Spanned};

    fn value_u32(value: &Value<Span>, field: &str) -> eyre::Result<u32> {
        match &*value.literal {
            Literal::Int(integer) => {
                u32::try_from(integer).wrap_err_with(|| format!("invalid u32 {field}"))
            }
            _ => eyre::bail!("{field}: expected integer"),
        }
    }

    fn property_u32(node: &Node<Span>, name: &str) -> eyre::Result<Option<u32>> {
        node.properties
            .iter()
            .find(|(key, _)| key.as_ref() == name)
            .map(|(_, value)| value_u32(value, name))
            .transpose()
    }

    fn collect_t_nodes<'a, I>(nodes: I, entries: &mut Vec<EckrTerrainRouteEntry>) -> eyre::Result<()>
    where
        I: IntoIterator<Item = &'a Spanned<Node<Span>, Span>>,
    {
        for node in nodes {
            if node.node_name.as_ref() == "t" {
                if let Some(cc_id) = property_u32(node, "cc")? {
                    let target_id = property_u32(node, "kr")?
                        .or(property_u32(node, "ec")?)
                        .or(property_u32(node, "target")?)
                        .ok_or_else(|| eyre::eyre!("t cc={cc_id}: missing kr/ec/target property"))?;
                    entries.push(EckrTerrainRouteEntry {
                        new_id: target_id,
                        old_ids: vec![cc_id],
                    });
                    continue;
                }

                let mut args = node.arguments.iter();
                let new_id = args
                    .next()
                    .ok_or_else(|| eyre::eyre!("t: missing target id"))?;
                let old_ids = args
                    .map(|value| value_u32(value, "source id"))
                    .collect::<eyre::Result<Vec<_>>>()?;
                if old_ids.is_empty() {
                    eyre::bail!("t {}: missing source ids", value_u32(new_id, "target id")?);
                }
                entries.push(EckrTerrainRouteEntry {
                    new_id: value_u32(new_id, "target id")?,
                    old_ids,
                });
            }

            collect_t_nodes(node.children(), entries)?;
        }
        Ok(())
    }

    let document = knuffel::parse_ast::<Span>(source_name, content)
        .wrap_err_with(|| format!("parse {source_name} AST"))?;
    let mut entries = Vec::new();
    collect_t_nodes(document.nodes.iter(), &mut entries)?;
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
        parse_terrain_definition_kdl(&content).wrap_err("Failed to parse TerrainDefinition KDL")
    }

    pub fn to_map(&self) -> HashMap<u32, TerrainDefEntry> {
        self.entries.iter().map(|e| (e.id, e.clone())).collect()
    }
}

fn parse_terrain_definition_kdl(content: &str) -> eyre::Result<TerrainDefinitionKdl> {
    use knuffel::ast::{Literal, Node, Value};
    use knuffel::span::Span;

    fn argument<'a>(
        node: &'a Node<Span>,
        index: usize,
        node_name: &str,
    ) -> eyre::Result<&'a Value<Span>> {
        node.arguments
            .get(index)
            .ok_or_else(|| eyre::eyre!("{node_name}: missing argument {index}"))
    }

    fn property<'a>(node: &'a Node<Span>, name: &str) -> Option<&'a Value<Span>> {
        node.properties
            .iter()
            .find(|(key, _)| key.as_ref() == name)
            .map(|(_, value)| value)
    }

    fn value_u32(value: &Value<Span>, field: &str) -> eyre::Result<u32> {
        match &*value.literal {
            Literal::Int(integer) => {
                u32::try_from(integer).wrap_err_with(|| format!("invalid u32 {field}"))
            }
            _ => eyre::bail!("{field}: expected integer"),
        }
    }

    fn value_f32(value: &Value<Span>, field: &str) -> eyre::Result<f32> {
        match &*value.literal {
            Literal::Int(integer) => {
                let value = u32::try_from(integer)
                    .wrap_err_with(|| format!("invalid integer f32 {field}"))?;
                Ok(value as f32)
            }
            Literal::Decimal(decimal) => {
                f32::try_from(decimal).wrap_err_with(|| format!("invalid f32 {field}"))
            }
            _ => eyre::bail!("{field}: expected number"),
        }
    }

    fn value_string(value: &Value<Span>, field: &str) -> eyre::Result<String> {
        match &*value.literal {
            Literal::String(value) => Ok(value.to_string()),
            _ => eyre::bail!("{field}: expected string"),
        }
    }

    fn layer_from_node(node: &Node<Span>) -> eyre::Result<LayerDef> {
        let role = &**node.node_name;
        Ok(LayerDef {
            id: value_u32(argument(node, 0, role)?, "layer id")?,
            stretch: value_f32(argument(node, 1, role)?, "layer stretch")?,
        })
    }

    let mut entries = Vec::new();
    let document = knuffel::parse_ast::<Span>("TerrainDefinition.kdl", content)
        .wrap_err("parse TerrainDefinition.kdl AST")?;

    for node in &document.nodes {
        if &**node.node_name != "d" {
            continue;
        }
        let mut entry = TerrainDefEntry {
            id: value_u32(argument(node, 0, "d")?, "terrain id")?,
            terrain_type: value_string(argument(node, 1, "d")?, "terrain type")?,
            speed: property(node, "speed")
                .map(|value| value_f32(value, "speed"))
                .transpose()?,
            waveheight: property(node, "waveheight")
                .map(|value| value_f32(value, "waveheight"))
                .transpose()?,
            textureid: property(node, "textureid")
                .map(|value| value_u32(value, "textureid"))
                .transpose()?,
            t0: None,
            t1: None,
            m: None,
            s: None,
            n: None,
        };

        for child in node.children() {
            match child.node_name.as_ref() {
                "t0" => entry.t0 = Some(layer_from_node(child)?),
                "t1" => entry.t1 = Some(layer_from_node(child)?),
                "m" => entry.m = Some(layer_from_node(child)?),
                "s" => entry.s = Some(layer_from_node(child)?),
                "n" => entry.n = Some(layer_from_node(child)?),
                _ => {}
            }
        }

        entries.push(entry);
    }

    Ok(TerrainDefinitionKdl { entries })
}

fn strip_c_style_block_comments(content: &str) -> String {
    let mut stripped = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("*/") {
            rest = &after_start[end + 2..];
        } else {
            return stripped;
        }
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
}
