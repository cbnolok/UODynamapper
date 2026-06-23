//! Metadata axes for Enhanced Client texture references.
//!
//! These types describe the physical origin, logical grouping, and semantic
//! role of an EC material reference. They are orthogonal: a single texture ref
//! can be assigned any combination independently.
//!
//! # Design
//!
//! - `EcPhysicalPackage` — which UOP archive a path was originally sourced from.
//! - `EcLogicalFamily` — the folder-level grouping the path belongs to (e.g. WorldArt,
//!   TileArtLegacy, …). Usually correlates with the physical package but is derived
//!   purely from the path, so a terrain-archive entry that lives under
//!   `build/worldart/` still gets `LogicalFamily::WorldArt`.
//! - `EcStableRole` — the semantically stable function of the texture in the material
//!   (base colour, noise, normal map, …). Set by heuristic; confidence is tracked
//!   separately via `EcRoleConfidence`.
//! - `EcSpeculativeRole` — additional, speculative classification for textures whose
//!   stable role is still `UnknownSupport`. Can co-exist with any stable role.
//! - `EcRoleConfidence` — how reliable the role assignment is.

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

/// Semantically stable role of a texture reference within its material.
///
/// "Stable" means the role is unlikely to change as we learn more about the
/// EC format. Contrast with `EcSpeculativeRole` which tracks guesses that
/// may be revised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EcStableRole {
    /// Primary colour/albedo texture.
    Base,
    /// Secondary colour/albedo (e.g. an alternate base for blending).
    SecondaryBase,
    /// Alpha/transparency mask.
    AlphaMask,
    /// Generic mask (not specifically alpha — could be blend, dirt, etc.).
    GenericMask,
    /// Noise texture (tiling noise used for procedural variation).
    Noise,
    /// Detail/micro-detail texture.
    Detail,
    /// Overlay / emissive / glow layer.
    Overlay,
    /// Normal map or similar tangent-space direction data.
    NormalLike,
    /// Generic image-based support (catch-all for auxiliary images).
    ImageSupport,
    /// Only meaningful as metadata for effect-only entries; never sampled directly.
    EffectOnlyMetadata,
    /// Role not yet classified by any heuristic.
    UnknownSupport,
}

/// Speculative role for textures whose stable role is still `UnknownSupport`.
///
/// These are informed guesses based on path names and shader context. They
/// should be treated as hints only and may be overridden once the EC format
/// is better understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EcSpeculativeRole {
    /// No speculative assignment.
    None,
    /// Animated ripple layer used for liquid surfaces.
    LiquidRipple,
    /// Reflection support texture for liquid (environment probe source).
    LiquidReflectionSupport,
    /// Cubemap-like environment probe for liquid.
    LiquidEnvProbe,
    /// Foam or highlight layer on water edges.
    FoamHighlight,
    /// Flow map (used to distort UV coordinates over time).
    FlowMapLike,
    /// Refraction / distortion map.
    RefractionDistortionLike,
    /// Support texture specific to waterfall materials.
    WaterfallSupport,
    /// Support texture specific to lava / bubble materials.
    LavaBubbleSupport,
    /// Post-blend / compositing support (applied after the primary blend).
    PostBlendSupport,
}

/// Confidence level for a role assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EcRoleConfidence {
    /// Derived from authoritative data (e.g. an explicit format field).
    High,
    /// Derived from a reliable heuristic (e.g. shader name + path pattern).
    Medium,
    /// Derived from a weak heuristic (path keyword only).
    Low,
    /// No heuristic could produce an assignment.
    Unresolved,
}
