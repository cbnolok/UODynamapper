use std::collections::HashMap;
use std::path::Path;
use knuffel::Decode;
use color_eyre::eyre::{self, Context};

#[derive(Decode, Debug, Clone)]
pub struct TranscodeEntry {
    #[knuffel(argument)]
    pub new_id: u32,
    #[knuffel(arguments)]
    pub old_ids: Vec<u32>,
}

#[derive(Decode, Debug, Clone)]
pub struct TerrainTranscode {
    #[knuffel(children(name = "t"))]
    pub entries: Vec<TranscodeEntry>,
}

impl TerrainTranscode {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        knuffel::parse(path.as_ref().to_str().unwrap_or("transcode.kdl"), &content)
            .wrap_err("Failed to parse TerrainTranscode KDL")
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
        knuffel::parse(path.as_ref().to_str().unwrap_or("definition.kdl"), &content)
            .wrap_err("Failed to parse TerrainDefinition KDL")
    }

    pub fn to_map(&self) -> HashMap<u32, TerrainDefEntry> {
        self.entries.iter().map(|e| (e.id, e.clone())).collect()
    }
}
