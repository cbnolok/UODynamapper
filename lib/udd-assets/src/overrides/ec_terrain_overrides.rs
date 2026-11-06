use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;
use uot_kdl::strict::NodeSchema;

#[derive(Decode, Debug, Clone)]
pub struct EcTerrainOverrides {
    #[knuffel(children(name = "terrain"))]
    pub terrains: Vec<EcTerrainOverrideTerrainEntry>,
}

impl EcTerrainOverrides {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        Self::parse(
            path.as_ref().to_str().unwrap_or("EcTerrainOverrides.kdl"),
            &content,
        )
    }

    pub fn parse(name: &str, content: &str) -> eyre::Result<Self> {
        uot_kdl::strict::decode(name, content, EC_TERRAIN_OVERRIDES_SCHEMA)
    }

    pub fn to_map(&self) -> HashMap<u32, EcTerrainOverrideEntry> {
        let mut map = HashMap::new();
        for terrain in &self.terrains {
            let entry = map
                .entry(terrain.id)
                .or_insert_with(|| EcTerrainOverrideEntry::new(terrain.id));
            entry.layers.extend(terrain.layers.clone());
            entry.textures.extend(terrain.textures.clone());
            entry.ignore = terrain.ignore.clone().or_else(|| entry.ignore.clone());
        }
        map
    }
}

/// The nodes and properties the decoders below accept, for error messages that name them.
const EC_TERRAIN_OVERRIDES_SCHEMA: &[NodeSchema] = &[NodeSchema {
    name: "terrain",
    properties: &[],
    children: &[
        NodeSchema::leaf("layer", &["tex", "stretch", "code"]),
        NodeSchema::leaf("texture", &["role", "code"]),
        NodeSchema::leaf("ignore", &["code"]),
    ],
}];

#[derive(Decode, Debug, Clone)]
pub struct EcTerrainOverrideTerrainEntry {
    #[knuffel(argument)]
    pub id: u32,
    #[knuffel(children(name = "layer"))]
    pub layers: Vec<TerrainLayerOverride>,
    #[knuffel(children(name = "texture"))]
    pub textures: Vec<TerrainTextureOverride>,
    #[knuffel(child)]
    pub ignore: Option<TerrainIgnoreOverride>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainLayerOverride {
    #[knuffel(argument)]
    pub role: String,
    #[knuffel(property(name = "tex"))]
    pub texture: u32,
    #[knuffel(property)]
    pub stretch: Option<f32>,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainTextureOverride {
    #[knuffel(argument)]
    pub texture: u32,
    #[knuffel(property)]
    pub role: Option<String>,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainIgnoreOverride {
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EcTerrainOverrideEntry {
    pub id: u32,
    pub layers: Vec<TerrainLayerOverride>,
    pub textures: Vec<TerrainTextureOverride>,
    pub ignore: Option<TerrainIgnoreOverride>,
}

impl EcTerrainOverrideEntry {
    fn new(id: u32) -> Self {
        Self {
            id,
            layers: Vec::new(),
            textures: Vec::new(),
            ignore: None,
        }
    }

    pub fn active_action_count(&self) -> usize {
        self.layers.len()
            + self.textures.len()
            + usize::from(self.ignore.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_terrain_override_nodes() {
        let overrides = EcTerrainOverrides::parse(
            "test.kdl",
            r#"
terrain 52 {
    layer "t0" tex=2000510 stretch=1.0 code="reviewed_layer_texture"
}
terrain 75 {
    texture 1269 role="base" code="reviewed_textureid"
}
terrain 201 {
    ignore code="intentionally_hidden"
}
"#,
        )
        .expect("parse terrain overrides");
        let map = overrides.to_map();

        assert_eq!(map.len(), 3);
        assert_eq!(map[&52].layers[0].texture, 2000510);
        assert_eq!(map[&75].textures[0].role.as_deref(), Some("base"));
        assert_eq!(
            map[&201].ignore.as_ref().and_then(|ignore| ignore.code.as_deref()),
            Some("intentionally_hidden")
        );
    }

    fn parse_error(content: &str) -> String {
        EcTerrainOverrides::parse("test.kdl", content).expect_err("parse error").to_string()
    }

    #[test]
    fn bundled_terrain_overrides_parse() {
        EcTerrainOverrides::parse(
            "EcTerrainOverrides.kdl",
            include_str!("../../../../dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn terrain_overrides_reject_unknown_nodes_and_properties_with_their_position() {
        assert_eq!(
            parse_error("terrain 5 {\n    shader \"x\"\n}\n"),
            "test.kdl: line 2, column 5: unknown node `shader` in `terrain`: expected one of `layer`, `texture`, \
             `ignore`"
        );
        // The removed `policy` and `liquid` nodes (no client basis, no reader) are now unknown.
        assert_eq!(
            parse_error("terrain 5 {\n    liquid speed=0.1\n}\n"),
            "test.kdl: line 2, column 5: unknown node `liquid` in `terrain`: expected one of `layer`, `texture`, \
             `ignore`"
        );
        assert_eq!(
            parse_error("terrain 52 {\n    layer \"t0\" tex=1 height=0.3\n}\n"),
            "test.kdl: line 2, column 22: unknown property `height` of `layer`: expected one of `tex`, \
             `stretch`, `code`"
        );
        assert_eq!(
            parse_error("terrian 5\n"),
            "test.kdl: line 1, column 1: unknown node `terrian` at the top level: expected `terrain`"
        );
    }

    #[test]
    fn terrain_overrides_reject_missing_and_extra_values() {
        let missing = parse_error("terrain 52 {\n    layer \"t0\" stretch=1.0\n}\n");
        assert!(missing.starts_with("test.kdl: invalid KDL: line 2, column 5:"), "{missing}");
        assert!(missing.contains("tex"), "{missing}");

        let extra = parse_error("terrain 5 6\n");
        assert!(extra.starts_with("test.kdl: invalid KDL: line 1, column 11: unexpected argument"), "{extra}");
    }
}
