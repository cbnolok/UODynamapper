use crate::common::{read_path_entry_cow, read_pod_vec};
use bytemuck::{Pod, Zeroable};
use color_eyre::eyre::{self, WrapErr};
use std::path::Path;
use udd_container::UddpReader;

pub const TILEMETA_LAND_ENTRY_PATH: &str = "metadata/land.bin";
pub const TILEMETA_ITEM_ENTRY_PATH: &str = "metadata/items.bin";
pub const TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH: &str = "metadata/item_texture_refs_index.bin";
pub const TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH: &str = "metadata/item_texture_refs.bin";

pub const TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY: u8 = 1 << 0;
pub const TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED: u8 = 1 << 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialPhysicalPackage {
    #[default]
    Unknown = 0,
    Texture = 1,
    LegacyTexture = 2,
    TerrainTexture = 3,
    EffectTexture = 4,
    SystemTextures = 5,
    ShaderResources = 6,
}

impl EcMaterialPhysicalPackage {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Texture,
            2 => Self::LegacyTexture,
            3 => Self::TerrainTexture,
            4 => Self::EffectTexture,
            5 => Self::SystemTextures,
            6 => Self::ShaderResources,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialLogicalFamily {
    #[default]
    Unknown = 0,
    WorldArt = 1,
    TileArtLegacy = 2,
    TileArtEnhanced = 3,
    Textures = 4,
    Effects = 5,
    SystemTextures = 6,
    ShaderResources = 7,
}

impl EcMaterialLogicalFamily {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::WorldArt,
            2 => Self::TileArtLegacy,
            3 => Self::TileArtEnhanced,
            4 => Self::Textures,
            5 => Self::Effects,
            6 => Self::SystemTextures,
            7 => Self::ShaderResources,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialStableRole {
    #[default]
    UnknownSupport = 0,
    Base = 1,
    SecondaryBase = 2,
    AlphaMask = 3,
    GenericMask = 4,
    Noise = 5,
    Detail = 6,
    Overlay = 7,
    NormalLike = 8,
    ImageSupport = 9,
    EffectOnlyMetadata = 10,
}

impl EcMaterialStableRole {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Base,
            2 => Self::SecondaryBase,
            3 => Self::AlphaMask,
            4 => Self::GenericMask,
            5 => Self::Noise,
            6 => Self::Detail,
            7 => Self::Overlay,
            8 => Self::NormalLike,
            9 => Self::ImageSupport,
            10 => Self::EffectOnlyMetadata,
            _ => Self::UnknownSupport,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EcMaterialSpeculativeRole {
    #[default]
    UnknownSupport = 0,
    LiquidRipple = 1,
    LiquidReflectionSupport = 2,
    LiquidEnvProbe = 3,
    FoamHighlight = 4,
    FlowMapLike = 5,
    RefractionDistortionLike = 6,
    WaterfallSupport = 7,
    LavaBubbleSupport = 8,
    PostBlendSupport = 9,
}

impl EcMaterialSpeculativeRole {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::LiquidRipple,
            2 => Self::LiquidReflectionSupport,
            3 => Self::LiquidEnvProbe,
            4 => Self::FoamHighlight,
            5 => Self::FlowMapLike,
            6 => Self::RefractionDistortionLike,
            7 => Self::WaterfallSupport,
            8 => Self::LavaBubbleSupport,
            9 => Self::PostBlendSupport,
            _ => Self::UnknownSupport,
        }
    }
}

/// Implementation readiness of a preserved EC texture reference.
///
/// Computed from `stable_role` and `speculative_role`; never stored as a separate
/// binary field.  Classifies every preserved ref by which future rendering feature
/// family it is meant to serve so later agents can consume or ignore refs without
/// repeating role archaeology.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EcMaterialImplementationReadiness {
    /// Role derivation was inconclusive.
    #[default]
    Unknown,
    /// Already selected or eligible as the visible base.  Stable role: `Base`.
    BaseSafe,
    /// Candidate for two-base + alpha-mask authored blend (Phase 10D).
    /// Stable roles: `SecondaryBase`, `AlphaMask`, `Detail`, `Overlay`.
    BlendReady,
    /// Candidate for liquid-base + ripple/normal-like perturbation (Phase 10E/10F).
    /// Speculative roles: `LiquidRipple`, `LiquidReflectionSupport`, `LiquidEnvProbe`
    /// on any stable role, or `NormalLike` with a liquid speculative role.
    LiquidReady,
    /// Normal-map-like candidate not yet consumable by any implemented pass.
    /// Stable role: `NormalLike` without a liquid speculative role.
    NormalLikeExperimental,
    /// Requires a dedicated extra render pass not yet implemented.
    /// Speculative roles: `FoamHighlight`, `WaterfallSupport`, `LavaBubbleSupport`,
    /// `RefractionDistortionLike`, `FlowMapLike`, `PostBlendSupport`.
    /// Also: `EffectOnlyMetadata` stable role.
    ExtraPassOnly,
}

impl EcMaterialImplementationReadiness {
    /// Derive readiness from an already-resolved stable and speculative role pair.
    pub fn from_roles(stable: EcMaterialStableRole, speculative: EcMaterialSpeculativeRole) -> Self {
        use EcMaterialImplementationReadiness::*;
        use EcMaterialSpeculativeRole as Spec;
        use EcMaterialStableRole as Stable;

        match stable {
            Stable::Base => BaseSafe,
            Stable::SecondaryBase | Stable::AlphaMask | Stable::Detail | Stable::Overlay => {
                BlendReady
            }
            Stable::NormalLike => match speculative {
                Spec::LiquidRipple | Spec::LiquidReflectionSupport | Spec::LiquidEnvProbe => {
                    LiquidReady
                }
                _ => NormalLikeExperimental,
            },
            Stable::ImageSupport
            | Stable::GenericMask
            | Stable::Noise
            | Stable::UnknownSupport => match speculative {
                Spec::LiquidRipple | Spec::LiquidReflectionSupport | Spec::LiquidEnvProbe => {
                    LiquidReady
                }
                Spec::FoamHighlight
                | Spec::WaterfallSupport
                | Spec::LavaBubbleSupport
                | Spec::RefractionDistortionLike
                | Spec::FlowMapLike
                | Spec::PostBlendSupport => ExtraPassOnly,
                Spec::UnknownSupport => Unknown,
            },
            Stable::EffectOnlyMetadata => ExtraPassOnly,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::BaseSafe => "base_safe",
            Self::BlendReady => "blend_ready",
            Self::LiquidReady => "liquid_ready",
            Self::NormalLikeExperimental => "normal_like_experimental",
            Self::ExtraPassOnly => "extra_pass_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileMetaMainEcTextureReason {
    RoleBaseExactTileId,
    RoleBasePrimarySelected,
    RoleBaseFirst,
    RoleSecondaryBasePrimarySelected,
    RoleSecondaryBaseFirst,
    ExactWorldArtTileId,
    PrimarySelectedWorldArt,
    FirstWorldArt,
    LegacyEcTextureIdFallback,
}

impl TileMetaMainEcTextureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoleBaseExactTileId => "role_base_exact_tile_id",
            Self::RoleBasePrimarySelected => "role_base_primary_selected",
            Self::RoleBaseFirst => "role_base_first",
            Self::RoleSecondaryBasePrimarySelected => "role_secondary_base_primary_selected",
            Self::RoleSecondaryBaseFirst => "role_secondary_base_first",
            Self::ExactWorldArtTileId => "exact_worldart_tile_id",
            Self::PrimarySelectedWorldArt => "primary_selected_worldart",
            Self::FirstWorldArt => "first_worldart",
            Self::LegacyEcTextureIdFallback => "legacy_ec_texture_id_fallback",
        }
    }
}

/// Reason produced by the stable-role-aware base texture chooser.
///
/// Priority order (highest first):
/// 1. `RoleBase`              — `stable_role == Base` && primary-selected
/// 2. `RoleBaseFirst`         — `stable_role == Base`, any (first found)
/// 3. `RoleSecondaryBase`     — `stable_role == SecondaryBase`, primary-selected
/// 4. `FallbackWorldArtPrimary` — no Base/SecondaryBase; WorldArt + primary-selected
/// 5. `FallbackWorldArtFirst`   — no Base/SecondaryBase; WorldArt, first non-aux
/// 6. `FallbackAnyPrimary`      — primary-selected, any non-aux ref
/// 7. `FallbackAnyFirst`        — first non-aux ref
/// 8. `FallbackLegacyEcTextureId` — used `item.ec_texture_id`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainEcTextureRoleReason {
    RoleBase,
    RoleBaseFirst,
    RoleSecondaryBase,
    FallbackWorldArtPrimary,
    FallbackWorldArtFirst,
    FallbackAnyPrimary,
    FallbackAnyFirst,
    FallbackLegacyEcTextureId,
}

impl MainEcTextureRoleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoleBase => "role_base",
            Self::RoleBaseFirst => "role_base_first",
            Self::RoleSecondaryBase => "role_secondary_base",
            Self::FallbackWorldArtPrimary => "fallback_worldart_primary",
            Self::FallbackWorldArtFirst => "fallback_worldart_first",
            Self::FallbackAnyPrimary => "fallback_any_primary",
            Self::FallbackAnyFirst => "fallback_any_first",
            Self::FallbackLegacyEcTextureId => "fallback_legacy_ec_texture_id",
        }
    }

    pub fn is_low_confidence(self) -> bool {
        matches!(
            self,
            Self::FallbackWorldArtFirst
                | Self::FallbackAnyPrimary
                | Self::FallbackAnyFirst
                | Self::FallbackLegacyEcTextureId
        )
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaLandTile {
    pub tile_id: u32,
    pub texture_id: u16,
    pub tile_type: u8,
    pub _pad1: u8,
    pub flags: u64,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],
}

impl TileMetaLandTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaItemTile {
    pub tile_id: u32,
    pub weight: u8,
    pub quality: u8,
    pub quantity: u8,
    pub hue_extra: u8,
    pub flags: u64,
    pub anim_id: u16,
    pub stacking_offset: u8,
    pub value: u8,
    pub height: i8,
    pub _pad1: u8,
    pub _pad2: u16,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],

    pub ec_texture_id: u32,
    pub ec_start_x: i16,
    pub ec_start_y: i16,

    pub cc_texture_id: u32,
    pub cc_start_x: i16,
    pub cc_start_y: i16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TileMetaItemVisualKind {
    #[default]
    RegularArt = 0,
    SurfaceLike = 1,
}

impl TileMetaItemTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }

    pub fn visual_kind(&self) -> TileMetaItemVisualKind {
        match self._pad1 {
            1 => TileMetaItemVisualKind::SurfaceLike,
            _ => TileMetaItemVisualKind::RegularArt,
        }
    }

    pub fn is_surface_like(&self) -> bool {
        self.visual_kind() == TileMetaItemVisualKind::SurfaceLike
    }

    pub fn set_visual_kind(&mut self, kind: TileMetaItemVisualKind) {
        self._pad1 = kind as u8;
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRefSpan {
    pub start: u32,
    pub len: u16,
    pub _pad: u16,
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRef {
    pub texture_id: u32,
    pub texture_type: u8,
    pub block_index: u8,
    pub item_index: u8,
    pub flags: u8,
    pub texture_stretch: f32,
    pub unk4: u8,
    pub physical_package: u8,
    pub stable_role: u8,
    pub speculative_role: u8,
    pub unk6: u32,
    pub unk7: u32,
}

impl TileMetaItemTextureRef {
    pub fn is_auxiliary(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY != 0
    }

    pub fn is_primary_selected(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED != 0
    }

    pub fn is_world_art(&self) -> bool {
        self.texture_type == 1
    }

    pub fn logical_family(&self) -> EcMaterialLogicalFamily {
        EcMaterialLogicalFamily::from_u8(self.texture_type)
    }

    pub fn physical_package(&self) -> EcMaterialPhysicalPackage {
        EcMaterialPhysicalPackage::from_u8(self.physical_package)
    }

    pub fn stable_role(&self) -> EcMaterialStableRole {
        EcMaterialStableRole::from_u8(self.stable_role)
    }

    pub fn speculative_role(&self) -> EcMaterialSpeculativeRole {
        EcMaterialSpeculativeRole::from_u8(self.speculative_role)
    }

    /// Compute the implementation readiness class from the stored role fields.
    ///
    /// Readiness is not stored as a separate binary field; it is always derived fresh
    /// from `stable_role` and `speculative_role` so it reflects the current derivation
    /// logic without requiring a format version bump.
    pub fn implementation_readiness(&self) -> EcMaterialImplementationReadiness {
        EcMaterialImplementationReadiness::from_roles(self.stable_role(), self.speculative_role())
    }
}

pub struct TileMetaPackage {
    #[allow(dead_code)]
    package: UddpReader,
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    item_texture_ref_spans: Vec<TileMetaItemTextureRefSpan>,
    item_texture_refs: Vec<TileMetaItemTextureRef>,
}

impl TileMetaPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let land_bytes = read_path_entry_cow(&package, TILEMETA_LAND_ENTRY_PATH)?;
        let item_bytes = read_path_entry_cow(&package, TILEMETA_ITEM_ENTRY_PATH)?;
        let item_texture_ref_spans =
            read_optional_pod_vec(&package, TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH)?;
        let item_texture_refs =
            read_optional_pod_vec(&package, TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH)?;
        let land_tiles = read_pod_vec(land_bytes.as_ref(), TILEMETA_LAND_ENTRY_PATH)?;
        let item_tiles = read_pod_vec(item_bytes.as_ref(), TILEMETA_ITEM_ENTRY_PATH)?;

        Ok(Self {
            package,
            land_tiles,
            item_tiles,
            item_texture_ref_spans,
            item_texture_refs,
        })
    }

    pub fn land_tiles(&self) -> &[TileMetaLandTile] {
        &self.land_tiles
    }

    pub fn item_tiles(&self) -> &[TileMetaItemTile] {
        &self.item_tiles
    }

    pub fn land_tile(&self, tile_id: u32) -> Option<&TileMetaLandTile> {
        self.land_tiles.get(tile_id as usize)
    }

    pub fn item_tile(&self, tile_id: u32) -> Option<&TileMetaItemTile> {
        self.item_tiles.get(tile_id as usize)
    }

    pub fn item_texture_refs(&self, tile_id: u32) -> &[TileMetaItemTextureRef] {
        let Some(span) = self.item_texture_ref_spans.get(tile_id as usize) else {
            return &self.item_texture_refs[0..0];
        };

        let start = span.start as usize;
        let end = start.saturating_add(span.len as usize);
        if start > self.item_texture_refs.len() || end > self.item_texture_refs.len() {
            return &self.item_texture_refs[0..0];
        }

        &self.item_texture_refs[start..end]
    }

    pub fn main_ec_texture_ref(&self, tile_id: u32) -> Option<&TileMetaItemTextureRef> {
        self.main_ec_texture_ref_with_reason(tile_id)
            .map(|(texture_ref, _)| texture_ref)
    }

    pub fn main_ec_texture_ref_with_reason(
        &self,
        tile_id: u32,
    ) -> Option<(&TileMetaItemTextureRef, TileMetaMainEcTextureReason)> {
        let allow_exact_tile_id = self
            .item_tile(tile_id)
            .is_none_or(|item| !item.is_surface_like());
        choose_main_ec_texture_ref(self.item_texture_refs(tile_id), tile_id, allow_exact_tile_id)
    }

    pub fn main_ec_texture_id(&self, tile_id: u32) -> Option<u32> {
        self.main_ec_texture_id_with_reason(tile_id)
            .map(|(texture_id, _)| texture_id)
    }

    pub fn main_ec_texture_id_with_reason(
        &self,
        tile_id: u32,
    ) -> Option<(u32, TileMetaMainEcTextureReason)> {
        self.main_ec_texture_ref_with_reason(tile_id)
            .map(|(texture_ref, reason)| (texture_ref.texture_id, reason))
            .or_else(|| {
                self.item_tile(tile_id)
                    .and_then(|item| (item.ec_texture_id != 0).then_some(item.ec_texture_id))
                    .map(|texture_id| {
                        (
                            texture_id,
                            TileMetaMainEcTextureReason::LegacyEcTextureIdFallback,
                        )
                    })
            })
    }

    /// Stable-role-aware base texture selector.
    ///
    /// Returns `None` only when there are no candidates at all.
    pub fn main_ec_texture_id_role_aware(
        &self,
        tile_id: u32,
    ) -> Option<(u32, EcMaterialStableRole, MainEcTextureRoleReason)> {
        choose_main_ec_texture_id_role_aware(
            self.item_texture_refs(tile_id),
            self.item_tile(tile_id),
        )
    }

    /// Returns `true` when `main_ec_texture_id_role_aware` would select a low-confidence fallback.
    pub fn main_ec_texture_selection_is_low_confidence(&self, tile_id: u32) -> bool {
        self.main_ec_texture_id_role_aware(tile_id)
            .is_some_and(|(_, _, reason)| reason.is_low_confidence())
    }

    pub fn main_ec_texture_id_with_branch(&self, tile_id: u32) -> Option<(u32, &'static str)> {
        self.main_ec_texture_id_role_aware(tile_id)
            .map(|(texture_id, _, reason)| (texture_id, reason.as_str()))
    }
}

fn choose_main_ec_texture_ref(
    texture_refs: &[TileMetaItemTextureRef],
    tile_id: u32,
    allow_exact_tile_id: bool,
) -> Option<(&TileMetaItemTextureRef, TileMetaMainEcTextureReason)> {
    texture_refs
        .iter()
        .find(|texture_ref| {
            allow_exact_tile_id
                && !texture_ref.is_auxiliary()
                && texture_ref.stable_role() == EcMaterialStableRole::Base
                && texture_ref.texture_id == tile_id
        })
        .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::RoleBaseExactTileId))
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::Base
                        && texture_ref.is_primary_selected()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleBasePrimarySelected,
                    )
                })
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::Base
                })
                .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::RoleBaseFirst))
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::SecondaryBase
                        && texture_ref.is_primary_selected()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleSecondaryBasePrimarySelected,
                    )
                })
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && texture_ref.stable_role() == EcMaterialStableRole::SecondaryBase
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::RoleSecondaryBaseFirst,
                    )
                })
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    allow_exact_tile_id
                        && texture_ref.stable_role() == EcMaterialStableRole::UnknownSupport
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_world_art()
                        && texture_ref.texture_id == tile_id
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::ExactWorldArtTileId,
                    )
                })
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    texture_ref.stable_role() == EcMaterialStableRole::UnknownSupport
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_primary_selected()
                        && texture_ref.is_world_art()
                })
                .map(|texture_ref| {
                    (
                        texture_ref,
                        TileMetaMainEcTextureReason::PrimarySelectedWorldArt,
                    )
                })
        })
        .or_else(|| {
            texture_refs
                .iter()
                .find(|texture_ref| {
                    texture_ref.stable_role() == EcMaterialStableRole::UnknownSupport
                        && valid_ec_art_texture_identity(
                            texture_ref,
                            tile_id,
                            allow_exact_tile_id,
                        )
                        && !texture_ref.is_auxiliary()
                        && texture_ref.is_world_art()
                })
                .map(|texture_ref| (texture_ref, TileMetaMainEcTextureReason::FirstWorldArt))
        })
}

fn valid_ec_art_texture_identity(
    texture_ref: &TileMetaItemTextureRef,
    tile_id: u32,
    allow_exact_tile_id: bool,
) -> bool {
    allow_exact_tile_id || texture_ref.texture_id != tile_id
}

fn choose_main_ec_texture_id_role_aware(
    texture_refs: &[TileMetaItemTextureRef],
    item_tile: Option<&TileMetaItemTile>,
) -> Option<(u32, EcMaterialStableRole, MainEcTextureRoleReason)> {
    // 1. Base + primary-selected
    if let Some(r) = texture_refs.iter().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::Base
            && r.is_primary_selected()
    }) {
        return Some((r.texture_id, EcMaterialStableRole::Base, MainEcTextureRoleReason::RoleBase));
    }

    // 2. Base, any (first found)
    if let Some(r) = texture_refs.iter().find(|r| {
        !r.is_auxiliary() && r.stable_role() == EcMaterialStableRole::Base
    }) {
        return Some((r.texture_id, EcMaterialStableRole::Base, MainEcTextureRoleReason::RoleBaseFirst));
    }

    // 3. SecondaryBase + primary-selected
    if let Some(r) = texture_refs.iter().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::SecondaryBase
            && r.is_primary_selected()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::SecondaryBase,
            MainEcTextureRoleReason::RoleSecondaryBase,
        ));
    }

    // 4. WorldArt + primary-selected (UnknownSupport only)
    if let Some(r) = texture_refs.iter().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::UnknownSupport
            && r.is_world_art()
            && r.is_primary_selected()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::UnknownSupport,
            MainEcTextureRoleReason::FallbackWorldArtPrimary,
        ));
    }

    // 5. WorldArt, first non-aux (UnknownSupport only)
    if let Some(r) = texture_refs.iter().find(|r| {
        !r.is_auxiliary()
            && r.stable_role() == EcMaterialStableRole::UnknownSupport
            && r.is_world_art()
    }) {
        return Some((
            r.texture_id,
            EcMaterialStableRole::UnknownSupport,
            MainEcTextureRoleReason::FallbackWorldArtFirst,
        ));
    }

    // 6. Any primary-selected non-aux
    if let Some(r) = texture_refs
        .iter()
        .find(|r| !r.is_auxiliary() && r.is_primary_selected())
    {
        return Some((
            r.texture_id,
            r.stable_role(),
            MainEcTextureRoleReason::FallbackAnyPrimary,
        ));
    }

    // 7. First non-aux
    if let Some(r) = texture_refs.iter().find(|r| !r.is_auxiliary()) {
        return Some((
            r.texture_id,
            r.stable_role(),
            MainEcTextureRoleReason::FallbackAnyFirst,
        ));
    }

    // 8. Legacy item.ec_texture_id
    if let Some(item) = item_tile {
        if item.ec_texture_id != 0 {
            return Some((
                item.ec_texture_id,
                EcMaterialStableRole::UnknownSupport,
                MainEcTextureRoleReason::FallbackLegacyEcTextureId,
            ));
        }
    }

    None
}

fn read_optional_pod_vec<T: Pod>(package: &UddpReader, path: &str) -> eyre::Result<Vec<T>> {
    match read_path_entry_cow(package, path) {
        Ok(bytes) => read_pod_vec(bytes.as_ref(), path),
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_texture_ref_material_axes_decode_from_dense_bytes() {
        let texture_ref = TileMetaItemTextureRef {
            texture_id: 42,
            texture_type: EcMaterialLogicalFamily::Textures as u8,
            physical_package: EcMaterialPhysicalPackage::TerrainTexture as u8,
            stable_role: EcMaterialStableRole::ImageSupport as u8,
            speculative_role: EcMaterialSpeculativeRole::LiquidRipple as u8,
            ..TileMetaItemTextureRef::zeroed()
        };

        assert_eq!(
            texture_ref.logical_family(),
            EcMaterialLogicalFamily::Textures
        );
        assert_eq!(
            texture_ref.physical_package(),
            EcMaterialPhysicalPackage::TerrainTexture
        );
        assert_eq!(
            texture_ref.stable_role(),
            EcMaterialStableRole::ImageSupport
        );
        assert_eq!(
            texture_ref.speculative_role(),
            EcMaterialSpeculativeRole::LiquidRipple
        );
    }

    #[test]
    fn zeroed_item_texture_ref_keeps_old_sidecar_axes_unknown() {
        let texture_ref = TileMetaItemTextureRef::zeroed();

        assert_eq!(
            texture_ref.logical_family(),
            EcMaterialLogicalFamily::Unknown
        );
        assert_eq!(
            texture_ref.physical_package(),
            EcMaterialPhysicalPackage::Unknown
        );
        assert_eq!(
            texture_ref.stable_role(),
            EcMaterialStableRole::UnknownSupport
        );
        assert_eq!(
            texture_ref.speculative_role(),
            EcMaterialSpeculativeRole::UnknownSupport
        );
        assert_eq!(std::mem::size_of::<TileMetaItemTextureRef>(), 24);
    }

    #[test]
    fn main_ec_texture_reason_freezes_exact_worldart_branch() {
        let texture_refs = [
            texture_ref(42, EcMaterialLogicalFamily::Textures, 0),
            texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, true).expect("chosen texture");

        assert_eq!(chosen.texture_id, 100);
        assert_eq!(reason, TileMetaMainEcTextureReason::ExactWorldArtTileId);
        assert_eq!(reason.as_str(), "exact_worldart_tile_id");
    }

    #[test]
    fn main_ec_texture_ignores_unknown_primary_non_worldart() {
        let texture_refs = [
            texture_ref(10, EcMaterialLogicalFamily::Textures, TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY),
            texture_ref(
                42,
                EcMaterialLogicalFamily::Textures,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, true).is_none());
    }

    #[test]
    fn main_ec_texture_prefers_base_role_over_exact_unknown_worldart() {
        let texture_refs = [
            texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::Textures,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, true).expect("chosen texture");

        assert_eq!(chosen.texture_id, 42);
        assert_eq!(reason, TileMetaMainEcTextureReason::RoleBasePrimarySelected);
    }

    #[test]
    fn main_ec_texture_disables_exact_id_for_surface_like_selection() {
        let texture_refs = [
            texture_ref_with_role(
                100,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                0,
            ),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (chosen, reason) =
            choose_main_ec_texture_ref(&texture_refs, 100, false).expect("chosen texture");

        assert_eq!(chosen.texture_id, 42);
        assert_eq!(reason, TileMetaMainEcTextureReason::RoleBasePrimarySelected);
    }

    #[test]
    fn main_ec_texture_disables_unknown_exact_id_for_surface_like_selection() {
        let texture_refs = [texture_ref(100, EcMaterialLogicalFamily::WorldArt, 0)];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, false).is_none());
    }

    #[test]
    fn main_ec_texture_rejects_explicit_support_roles() {
        let texture_refs = [
            texture_ref_with_role(
                100,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::AlphaMask,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                101,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::NormalLike,
                0,
            ),
        ];

        assert!(choose_main_ec_texture_ref(&texture_refs, 100, true).is_none());
    }

    fn texture_ref(
        texture_id: u32,
        family: EcMaterialLogicalFamily,
        flags: u8,
    ) -> TileMetaItemTextureRef {
        texture_ref_with_role(texture_id, family, EcMaterialStableRole::UnknownSupport, flags)
    }

    fn texture_ref_with_role(
        texture_id: u32,
        family: EcMaterialLogicalFamily,
        role: EcMaterialStableRole,
        flags: u8,
    ) -> TileMetaItemTextureRef {
        TileMetaItemTextureRef {
            texture_id,
            texture_type: family as u8,
            stable_role: role as u8,
            flags,
            ..TileMetaItemTextureRef::zeroed()
        }
    }

    // --- Role-aware selection tests ---

    #[test]
    fn selects_primary_base_first() {
        let texture_refs = [
            texture_ref_with_role(
                99,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::Base,
                0,
            ),
            texture_ref_with_role(
                42,
                EcMaterialLogicalFamily::Textures,
                EcMaterialStableRole::Base,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                77,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::UnknownSupport,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let (id, role, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(id, 42);
        assert_eq!(role, EcMaterialStableRole::Base);
        assert_eq!(reason, MainEcTextureRoleReason::RoleBase);
    }

    #[test]
    fn falls_back_to_worldart_when_no_base() {
        let texture_refs = [
            texture_ref_with_role(
                55,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::UnknownSupport,
                TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                66,
                EcMaterialLogicalFamily::WorldArt,
                EcMaterialStableRole::UnknownSupport,
                0,
            ),
        ];

        let (id, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(id, 55);
        assert_eq!(reason, MainEcTextureRoleReason::FallbackWorldArtPrimary);
    }

    #[test]
    fn low_confidence_when_fallback_any() {
        let texture_refs = [texture_ref_with_role(
            10,
            EcMaterialLogicalFamily::Textures,
            EcMaterialStableRole::NormalLike,
            0,
        )];

        let (_, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, None).expect("result");

        assert_eq!(reason, MainEcTextureRoleReason::FallbackAnyFirst);
        assert!(reason.is_low_confidence());
    }

    #[test]
    fn not_found_when_no_refs() {
        let result = choose_main_ec_texture_id_role_aware(&[], None);
        assert!(result.is_none());
    }

    #[test]
    fn legacy_ec_texture_id_used_as_last_resort() {
        // All refs are auxiliary — none qualify as non-aux candidates.
        let texture_refs = [TileMetaItemTextureRef {
            texture_id: 999,
            texture_type: EcMaterialLogicalFamily::WorldArt as u8,
            stable_role: EcMaterialStableRole::Base as u8,
            flags: TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY,
            ..TileMetaItemTextureRef::zeroed()
        }];
        let mut item_tile = TileMetaItemTile::zeroed();
        item_tile.ec_texture_id = 777;

        let (id, _, reason) =
            choose_main_ec_texture_id_role_aware(&texture_refs, Some(&item_tile)).expect("result");

        assert_eq!(id, 777);
        assert_eq!(reason, MainEcTextureRoleReason::FallbackLegacyEcTextureId);
        assert!(reason.is_low_confidence());
    }

    // --- Implementation readiness tests (Phase 12) ---

    fn ref_with_roles(
        stable: EcMaterialStableRole,
        speculative: EcMaterialSpeculativeRole,
    ) -> TileMetaItemTextureRef {
        TileMetaItemTextureRef {
            texture_id: 1,
            stable_role: stable as u8,
            speculative_role: speculative as u8,
            ..TileMetaItemTextureRef::zeroed()
        }
    }

    #[test]
    fn blend_ready_secondary_base_preserved() {
        let r = ref_with_roles(
            EcMaterialStableRole::SecondaryBase,
            EcMaterialSpeculativeRole::UnknownSupport,
        );
        assert_eq!(
            r.implementation_readiness(),
            EcMaterialImplementationReadiness::BlendReady
        );
    }

    #[test]
    fn liquid_ready_ripple_preserved() {
        let r = ref_with_roles(
            EcMaterialStableRole::ImageSupport,
            EcMaterialSpeculativeRole::LiquidRipple,
        );
        assert_eq!(
            r.implementation_readiness(),
            EcMaterialImplementationReadiness::LiquidReady
        );
    }

    #[test]
    fn normal_like_distinguished_from_liquid_ready() {
        let r = ref_with_roles(
            EcMaterialStableRole::NormalLike,
            EcMaterialSpeculativeRole::UnknownSupport,
        );
        assert_eq!(
            r.implementation_readiness(),
            EcMaterialImplementationReadiness::NormalLikeExperimental
        );
        // NormalLike + liquid speculative → LiquidReady, not NormalLikeExperimental.
        let r2 = ref_with_roles(
            EcMaterialStableRole::NormalLike,
            EcMaterialSpeculativeRole::LiquidEnvProbe,
        );
        assert_eq!(
            r2.implementation_readiness(),
            EcMaterialImplementationReadiness::LiquidReady
        );
    }

    #[test]
    fn extra_pass_foam_preserved() {
        let r = ref_with_roles(
            EcMaterialStableRole::ImageSupport,
            EcMaterialSpeculativeRole::FoamHighlight,
        );
        assert_eq!(
            r.implementation_readiness(),
            EcMaterialImplementationReadiness::ExtraPassOnly
        );
    }

    #[test]
    fn readiness_visible_from_ref_method_and_as_str() {
        let r = ref_with_roles(
            EcMaterialStableRole::Base,
            EcMaterialSpeculativeRole::UnknownSupport,
        );
        let readiness = r.implementation_readiness();
        assert_eq!(readiness, EcMaterialImplementationReadiness::BaseSafe);
        assert_eq!(readiness.as_str(), "base_safe");
    }
}
