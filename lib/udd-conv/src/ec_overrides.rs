//! KDL parsers for EC material override files produced by Phase 9 generators.
//!
//! The three override files are optional: if absent, the corresponding list
//! is empty and the caller's behaviour is unchanged.
//!
//! # File formats
//!
//! `base_choice_overrides.kdl`:
//! ```kdl
//! art <tile_id> base-texture=<texture_id> reason="<note>"
//! ```
//!
//! `role_overrides.kdl`:
//! ```kdl
//! texture <texture_id> stable-role=<role> speculative-role=<role> reason="<note>"
//! ```
//!
//! `metadata_only_overrides.kdl`:
//! ```kdl
//! texture <texture_id> reason="<note>"
//! ```

use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;

// region: --- KDL node structs

#[derive(Decode, Debug, Clone)]
struct BaseChoiceOverridesKdl {
    #[knuffel(children(name = "art"))]
    entries: Vec<BaseChoiceArtNodeKdl>,
}

#[derive(Decode, Debug, Clone)]
struct BaseChoiceArtNodeKdl {
    #[knuffel(argument)]
    art_id: u32,
    #[knuffel(property(name = "base-texture"))]
    base_texture_id: u32,
    #[knuffel(property)]
    reason: String,
}

#[derive(Decode, Debug, Clone)]
struct RoleOverridesKdl {
    #[knuffel(children(name = "texture"))]
    entries: Vec<RoleTextureNodeKdl>,
}

#[derive(Decode, Debug, Clone)]
struct RoleTextureNodeKdl {
    #[knuffel(argument)]
    texture_id: u32,
    #[knuffel(property(name = "stable-role"))]
    stable_role: Option<String>,
    #[knuffel(property(name = "speculative-role"))]
    speculative_role: Option<String>,
    #[knuffel(property)]
    reason: String,
}

#[derive(Decode, Debug, Clone)]
struct MetadataOnlyOverridesKdl {
    #[knuffel(children(name = "texture"))]
    entries: Vec<MetadataOnlyTextureNodeKdl>,
}

#[derive(Decode, Debug, Clone)]
struct MetadataOnlyTextureNodeKdl {
    #[knuffel(argument)]
    texture_id: u32,
    #[knuffel(property)]
    reason: String,
}

// endregion: --- KDL node structs

// region: --- Public types

/// One entry from `base_choice_overrides.kdl`.
#[derive(Debug, Clone)]
pub struct BaseChoiceOverride {
    pub art_id: u32,
    pub base_texture_id: u32,
    pub reason: String,
}

/// One entry from `role_overrides.kdl`.
#[derive(Debug, Clone)]
pub struct RoleOverride {
    pub texture_id: u32,
    pub stable_role: Option<String>,
    pub speculative_role: Option<String>,
    pub reason: String,
}

/// One entry from `metadata_only_overrides.kdl`.
#[derive(Debug, Clone)]
pub struct MetadataOnlyOverride {
    pub texture_id: u32,
    pub reason: String,
}

/// Combined set of EC material overrides loaded from a directory.
#[derive(Debug, Clone, Default)]
pub struct EcOverrides {
    pub base_choices: Vec<BaseChoiceOverride>,
    pub roles: Vec<RoleOverride>,
    pub metadata_only: Vec<MetadataOnlyOverride>,
}

impl EcOverrides {
    /// Load override files from `dir`.
    ///
    /// Each of the three files is optional — if absent the corresponding list
    /// remains empty. Returns an error only if a file exists but cannot be parsed.
    pub fn load_from_dir(dir: &Path) -> eyre::Result<Self> {
        let base_choices = load_base_choice_overrides(dir)?;
        let roles = load_role_overrides(dir)?;
        let metadata_only = load_metadata_only_overrides(dir)?;
        Ok(Self {
            base_choices,
            roles,
            metadata_only,
        })
    }

    /// Returns `true` when all lists are empty.
    pub fn is_empty(&self) -> bool {
        self.base_choices.is_empty() && self.roles.is_empty() && self.metadata_only.is_empty()
    }
}

// endregion: --- Public types

// region: --- Loaders

fn load_base_choice_overrides(dir: &Path) -> eyre::Result<Vec<BaseChoiceOverride>> {
    let path = dir.join("base_choice_overrides.kdl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .wrap_err_with(|| format!("Failed to read {:?}", path))?;
    let name = path.to_str().unwrap_or("base_choice_overrides.kdl");
    let parsed: BaseChoiceOverridesKdl =
        knuffel::parse(name, &content).wrap_err("Failed to parse base_choice_overrides.kdl")?;
    Ok(parsed
        .entries
        .into_iter()
        .map(|node| BaseChoiceOverride {
            art_id: node.art_id,
            base_texture_id: node.base_texture_id,
            reason: node.reason,
        })
        .collect())
}

fn load_role_overrides(dir: &Path) -> eyre::Result<Vec<RoleOverride>> {
    let path = dir.join("role_overrides.kdl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .wrap_err_with(|| format!("Failed to read {:?}", path))?;
    let name = path.to_str().unwrap_or("role_overrides.kdl");
    let parsed: RoleOverridesKdl =
        knuffel::parse(name, &content).wrap_err("Failed to parse role_overrides.kdl")?;
    Ok(parsed
        .entries
        .into_iter()
        .map(|node| RoleOverride {
            texture_id: node.texture_id,
            stable_role: node.stable_role,
            speculative_role: node.speculative_role,
            reason: node.reason,
        })
        .collect())
}

fn load_metadata_only_overrides(dir: &Path) -> eyre::Result<Vec<MetadataOnlyOverride>> {
    let path = dir.join("metadata_only_overrides.kdl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .wrap_err_with(|| format!("Failed to read {:?}", path))?;
    let name = path.to_str().unwrap_or("metadata_only_overrides.kdl");
    let parsed: MetadataOnlyOverridesKdl =
        knuffel::parse(name, &content).wrap_err("Failed to parse metadata_only_overrides.kdl")?;
    Ok(parsed
        .entries
        .into_iter()
        .map(|node| MetadataOnlyOverride {
            texture_id: node.texture_id,
            reason: node.reason,
        })
        .collect())
}

// endregion: --- Loaders
