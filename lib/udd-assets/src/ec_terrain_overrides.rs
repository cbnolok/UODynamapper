use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;

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
        knuffel::parse(name, content).wrap_err("Failed to parse EcTerrainOverrides KDL")
    }

    pub fn to_map(&self) -> HashMap<u32, EcTerrainOverrideEntry> {
        let mut map = HashMap::new();
        for terrain in &self.terrains {
            let entry = map
                .entry(terrain.id)
                .or_insert_with(|| EcTerrainOverrideEntry::new(terrain.id));
            entry.policies.extend(terrain.policies.clone());
            entry.liquid = terrain.liquid.clone().or_else(|| entry.liquid.clone());
            entry.layers.extend(terrain.layers.clone());
            entry.textures.extend(terrain.textures.clone());
            entry.ignore = terrain.ignore.clone().or_else(|| entry.ignore.clone());
        }
        map
    }
}

#[derive(Decode, Debug, Clone)]
pub struct EcTerrainOverrideTerrainEntry {
    #[knuffel(argument)]
    pub id: u32,
    #[knuffel(children(name = "policy"))]
    pub policies: Vec<TerrainPolicyOverride>,
    #[knuffel(child)]
    pub liquid: Option<TerrainLiquidOverride>,
    #[knuffel(children(name = "layer"))]
    pub layers: Vec<TerrainLayerOverride>,
    #[knuffel(children(name = "texture"))]
    pub textures: Vec<TerrainTextureOverride>,
    #[knuffel(child)]
    pub ignore: Option<TerrainIgnoreOverride>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainPolicyOverride {
    #[knuffel(argument)]
    pub policy: String,
    #[knuffel(property)]
    pub code: Option<String>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainLiquidOverride {
    #[knuffel(property)]
    pub speed: Option<f32>,
    #[knuffel(property)]
    pub waveheight: Option<f32>,
    #[knuffel(property)]
    pub code: Option<String>,
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
    pub policies: Vec<TerrainPolicyOverride>,
    pub liquid: Option<TerrainLiquidOverride>,
    pub layers: Vec<TerrainLayerOverride>,
    pub textures: Vec<TerrainTextureOverride>,
    pub ignore: Option<TerrainIgnoreOverride>,
}

impl EcTerrainOverrideEntry {
    fn new(id: u32) -> Self {
        Self {
            id,
            policies: Vec::new(),
            liquid: None,
            layers: Vec::new(),
            textures: Vec::new(),
            ignore: None,
        }
    }

    pub fn active_action_count(&self) -> usize {
        self.policies.len()
            + usize::from(self.liquid.is_some())
            + self.layers.len()
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
terrain 5 {
    policy "smooth" code="reviewed_runtime_policy"
    liquid speed=0.01 waveheight=0.3 code="reviewed_liquid_motion"
}
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

        assert_eq!(map.len(), 4);
        assert_eq!(map[&5].policies[0].policy, "smooth");
        assert_eq!(map[&5].liquid.as_ref().and_then(|liquid| liquid.speed), Some(0.01));
        assert_eq!(map[&52].layers[0].texture, 2000510);
        assert_eq!(map[&75].textures[0].role.as_deref(), Some("base"));
        assert_eq!(
            map[&201].ignore.as_ref().and_then(|ignore| ignore.code.as_deref()),
            Some("intentionally_hidden")
        );
    }
}
