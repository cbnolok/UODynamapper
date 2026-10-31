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
use udd_kdl::strict::NodeSchema;

// region: --- KDL node structs

/// The nodes and properties of each file, for error messages that name them.
const BASE_CHOICE_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf("art", &["base-texture", "reason"])];
const ROLE_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf("texture", &["stable-role", "speculative-role", "reason"])];
const METADATA_ONLY_SCHEMA: &[NodeSchema] = &[NodeSchema::leaf("texture", &["reason"])];

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
    let parsed: BaseChoiceOverridesKdl = udd_kdl::strict::decode(name, &content, BASE_CHOICE_SCHEMA)?;
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
    let parsed: RoleOverridesKdl = udd_kdl::strict::decode(name, &content, ROLE_SCHEMA)?;
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
    let parsed: MetadataOnlyOverridesKdl = udd_kdl::strict::decode(name, &content, METADATA_ONLY_SCHEMA)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dynamapper/assets/ec_overrides");

    #[test]
    fn bundled_override_files_load() {
        // The bundled files are commented templates; they must still parse.
        EcOverrides::load_from_dir(Path::new(BUNDLED_DIR)).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn override_files_reject_unknown_nodes_properties_and_missing_values() {
        let error = |content: &str| {
            udd_kdl::strict::decode::<RoleOverridesKdl>("role_overrides.kdl", content, ROLE_SCHEMA)
                .expect_err("parse error")
                .to_string()
        };
        assert_eq!(
            error("texture 1 reason=\"x\"\nart 2 reason=\"y\"\n"),
            "role_overrides.kdl: line 2, column 1: unknown node `art` at the top level: expected `texture`"
        );
        assert_eq!(
            error("texture 1 role=\"base\" reason=\"x\"\n"),
            "role_overrides.kdl: line 1, column 11: unknown property `role` of `texture`: expected one of \
             `stable-role`, `speculative-role`, `reason`"
        );
        let missing = error("texture 1 stable-role=\"base\"\n");
        assert!(missing.starts_with("role_overrides.kdl: invalid KDL: line 1, column 1:"), "{missing}");
        assert!(missing.contains("reason"), "{missing}");
    }
}
