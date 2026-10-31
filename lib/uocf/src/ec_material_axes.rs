//! Metadata axes for Enhanced Client texture references.
//!
//! These types describe the physical origin and logical grouping of an EC material
//! reference. They are orthogonal: a single texture ref can have any combination.
//!
//! # Design
//!
//! - `EcPhysicalPackage` — which UOP archive a path was originally sourced from.
//! - `EcLogicalFamily` — the folder-level grouping the path belongs to (e.g. WorldArt,
//!   TileArtLegacy, …). Usually correlates with the physical package but is derived
//!   purely from the path, so a terrain-archive entry that lives under
//!   `build/worldart/` still gets `LogicalFamily::WorldArt`.
//!
//! The role of a texture item is not a path property: the client binds item i to shader map i of
//! its block's shader (`udd_assets::tilemeta::EcMaterialStableRole::for_texture_position`).

/// Which UOP archive physically contains the texture path.
///
/// Inferred from path prefix heuristics. When the path does not match any
/// known prefix, `Unknown` is returned.
///
/// Physical package describes origin, not semantics. A `TerrainTextureUop` ref
/// may still be art-owned support; an `EffectTextureUop` ref may still be an
/// image-bearing texture rather than particle-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EcPhysicalPackage {
    /// `build/worldart/` — Texture.uop image pool.
    TextureUop,
    /// `build/tileartlegacy/` — LegacyTexture.uop image pool.
    LegacyTextureUop,
    /// `build/terraintexture/` — TerrainTexture.uop support pool.
    TerrainTextureUop,
    /// `build/effecttexture/` — EffectTexture.uop mixed pool.
    EffectTextureUop,
    /// `build/shadertexture/` or any path containing `shader`.
    ShaderResourceLike,
    /// System textures (e.g. white, black, default) known by convention.
    SystemTextures,
    /// None of the above matched.
    #[default]
    Unknown,
}

impl EcPhysicalPackage {
    /// Infer from a path string such as those found in the EC string dictionary.
    ///
    /// Matching is case-insensitive and normalises backslashes. Priority order
    /// matches specificity: more specific prefixes are checked first.
    pub fn infer_from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        let normalized = lower.replace('\\', "/");

        if normalized.contains("build/worldart/") {
            Self::TextureUop
        } else if normalized.contains("build/tileartlegacy/") || normalized.contains("build/legacy") {
            Self::LegacyTextureUop
        } else if normalized.contains("build/terraintexture/") {
            Self::TerrainTextureUop
        } else if normalized.contains("build/effecttexture/") {
            Self::EffectTextureUop
        } else if normalized.contains("build/shadertexture/") || normalized.contains("shader") {
            Self::ShaderResourceLike
        } else {
            Self::Unknown
        }
    }

    /// Human-readable label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextureUop => "TextureUop",
            Self::LegacyTextureUop => "LegacyTextureUop",
            Self::TerrainTextureUop => "TerrainTextureUop",
            Self::EffectTextureUop => "EffectTextureUop",
            Self::ShaderResourceLike => "ShaderResourceLike",
            Self::SystemTextures => "SystemTextures",
            Self::Unknown => "Unknown",
        }
    }
}

/// Logical folder family that a texture path belongs to.
///
/// Derived entirely from the path string; independent of which physical UOP
/// actually stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EcLogicalFamily {
    /// `build/worldart/` or `data/worldart/` paths.
    WorldArt,
    /// `build/tileartlegacy/` or `data/tileartlegacy/` paths.
    TileArtLegacy,
    /// `build/tileartenhanced/` or `data/tileartenhanced/` paths.
    TileArtEnhanced,
    /// `build/terraintexture/` or `data/textures/` paths.
    Textures,
    /// `build/effecttexture/` paths.
    Effects,
    /// System textures (white, black, default).
    SystemTextures,
    /// `build/shadertexture/` or `shader`-containing paths.
    ShaderResources,
    /// None of the above matched.
    #[default]
    Unknown,
}

impl EcLogicalFamily {
    /// Infer from a path string such as those found in the EC string dictionary.
    pub fn infer_from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        let normalized = lower.replace('\\', "/");

        if normalized.contains("build/worldart/") || normalized.contains("data/worldart/") {
            Self::WorldArt
        } else if normalized.contains("build/tileartlegacy/")
            || normalized.contains("data/tileartlegacy/")
            || normalized.contains("build/legacy")
        {
            Self::TileArtLegacy
        } else if normalized.contains("build/tileartenhanced/")
            || normalized.contains("data/tileartenhanced/")
            || normalized.contains("build/enhanced")
        {
            Self::TileArtEnhanced
        } else if normalized.contains("build/terraintexture/")
            || normalized.contains("data/textures/")
        {
            Self::Textures
        } else if normalized.contains("build/effecttexture/") {
            Self::Effects
        } else if normalized.contains("build/shadertexture/") || normalized.contains("shader") {
            Self::ShaderResources
        } else {
            Self::Unknown
        }
    }

    /// Human-readable label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorldArt => "WorldArt",
            Self::TileArtLegacy => "TileArtLegacy",
            Self::TileArtEnhanced => "TileArtEnhanced",
            Self::Textures => "Textures",
            Self::Effects => "Effects",
            Self::SystemTextures => "SystemTextures",
            Self::ShaderResources => "ShaderResources",
            Self::Unknown => "Unknown",
        }
    }
}
