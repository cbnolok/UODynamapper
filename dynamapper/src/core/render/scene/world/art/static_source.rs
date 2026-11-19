//! Which client's art, terrain texture and tile data draws a static (the source policy): its visual kind by style
//! and source, the surface-like statics' Enhanced terrain textures, the tile flags it reads, and the art source in effect.

use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::{LandTextureSource, Settings};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::texture_cache::ec_item_surface::KR_ITEM_RECORD_BASE;
use crate::core::uo_files_loader::{
    KrItemPackagesRes, TexLandKrPackageRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes,
};
use crate::prelude::*;
use bevy::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use super::static_placement::CC_TILE_PIXEL_WIDTH;
use super::static_batches::STATIC_FLAG_LAND_GRADE;

pub(super) const CLASSIC_STATIC_ART_ID_OFFSET: u32 = 0x4000;
const CLASSIC_WATER_LAND_TILE_ID: u32 = 168;
const EC_WATER_BASE_LAYER_INDEX: u32 = 0;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum StaticVisualKind {
    CcRegular { art_id: u32, fallback_art_id: u32 },
    EcRegular { art_id: u32 },
    EcSurface { art_id: u32 },
    TexLandEc { resolution: SurfaceLikeTexLandEcResolution },
    Missing,
}

pub(super) fn classic_static_visual_kind(graphic: u16,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> StaticVisualKind {
    StaticVisualKind::CcRegular {
        art_id: u32::from(graphic) + CLASSIC_STATIC_ART_ID_OFFSET,
        fallback_art_id: tilemeta.map_or(u32::from(graphic), |m| m.cc_texture_id)
            .saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
    }
}

pub(super) fn static_visual_renders_wet(kind: StaticVisualKind,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    package: Option<&udd_assets::tilemeta::TileMetaPackage>, graphic: u32) -> bool {
    match kind {
        StaticVisualKind::Missing => false,
        StaticVisualKind::CcRegular { .. } =>
            static_tile_renders_wet(tilemeta, package, graphic, Some(ClientTextureSource::Cc)),
        _ => static_tile_renders_wet(tilemeta, package, graphic, Some(ClientTextureSource::Ec)),
    }
}

pub(super) const TILE_FLAG_BACKGROUND: u64 = 0x01;
pub(super) const TILE_FLAG_SURFACE: u64 = 0x200;
pub(super) const TILE_FLAG_BRIDGE: u64 = 0x400;
pub(super) const TILE_FLAG_FOLIAGE: u64 = 0x20_000;
pub(super) const TILE_FLAG_ROOF: u64 = 0x1000_0000;
/// NoShoot: the built floors (`static_continues_land`).
pub(super) const TILE_FLAG_NO_SHOOT: u64 = 0x2000;
/// IsWet flag — tile has tidal/water properties (matches tiledata.mul bit 0x80). An object property;
/// whether the art shader applies the animated sin/cos UV distortion is `static_tile_renders_wet`.
pub(super) const TILE_FLAG_WET: u64 = 0x80;
/// MultiMovable (ClassicUO TileFlag 0x10000000000): one more priority z, as ClassicUO's `AddGameObject` gives it.
pub(super) const TILE_FLAG_MULTI_MOVABLE: u64 = 0x100_0000_0000;
/// Surface-like: authored SurfaceLike, the Surface flag, or a static that renders wet.
/// `renders_wet` is `static_tile_renders_wet` for the art mode in use, so Classic mode keeps the IsWet
/// flag and Enhanced mode follows the water shader for items that have an EC material.
pub(crate) fn static_tile_is_surface_like(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    renders_wet: bool,
) -> bool {
    let Some(meta) = tilemeta else {
        return false;
    };

    meta.is_surface_like() || meta.flags & TILE_FLAG_SURFACE != 0 || renders_wet
}

pub(super) fn static_tile_uses_tex_land_ec_surface_path(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    renders_wet: bool,
) -> bool {
    static_tile_is_surface_like(tilemeta, renders_wet)
}

fn resolve_surface_like_tex_land_ec_slot_id(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> Option<SurfaceLikeTexLandEcResolution> {
    let meta = tilemeta?;
    // This path is Enhanced-only: wetness is the water shader for items with an EC material.
    let renders_wet = static_tile_renders_wet(tilemeta, tilemeta_package, tile_id, Some(ClientTextureSource::Ec));
    if !static_tile_uses_tex_land_ec_surface_path(tilemeta, renders_wet) {
        return None;
    }

    let package = tex_land_ec?;

    // Match the original item material's sampler identities to the terrain
    // material graph, rather than treating an item graphic as a land tile id.
    if let Some(tilemeta_package) = tilemeta_package {
        let refs = tilemeta_package.item_texture_refs(tile_id);
        if let Some(material) = tilemeta_package.item_material(tile_id, 0) {
            if let Some(payload) = package.source_liquid_payload_for_item(material, refs) {
                if let Some(base) = package.resolve_material_render_layers(payload)[0] {
                    if let Some(runtime_slot_id) = base.runtime_slot_id {
                        return Some(SurfaceLikeTexLandEcResolution {
                            runtime_slot_id,
                            texture_repetition: base.texture_repetition,
                            liquid_payload: Some(payload),
                            kingdom_reborn: false,
                        });
                    }
                }
            }
        }
    }

    let main_ec_texture_id = tilemeta_package
        .and_then(|package| surface_like_visible_ec_texture_ref(tile_id, renders_wet, package))
        .map(|texture_ref| texture_ref.texture_id)
        .or_else(|| surface_like_legacy_ec_texture_id_fallback(meta, renders_wet));
    let Some(main_ec_texture_id) = main_ec_texture_id else {
        if let Some(resolution) = wet_surface_like_water_resolution(renders_wet, package) {
            return Some(resolution);
        }

        return package
            .resolve_runtime_slot_id(meta.cc_texture_id)
            .map(|runtime_slot_id| {
                surface_like_tex_land_ec_resolution(package, runtime_slot_id, None)
            });
    };

    if package.present_slot(main_ec_texture_id).is_some() {
        return Some(surface_like_tex_land_ec_resolution(
            package,
            main_ec_texture_id,
            Some(main_ec_texture_id),
        ));
    }

    let mut canonical_slots = BTreeSet::new();
    let mut alias_slots = BTreeSet::new();
    for record in package
        .terrain_provenance()
        .iter()
        .filter(|record| record.selected_texture_id == main_ec_texture_id)
    {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            canonical_slots.insert(record.canonical_slot_id);
        }

        if record.alias_slot_id != 0
            && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            alias_slots.insert(record.alias_slot_id);
        }
    }

    if canonical_slots.len() == 1 {
        return canonical_slots.into_iter().next().map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        });
    }

    if canonical_slots.is_empty() && alias_slots.len() == 1 {
        return alias_slots.into_iter().next().map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        });
    }

    if let Some(resolution) = wet_surface_like_water_resolution(renders_wet, package) {
        return Some(resolution);
    }

    package
        .resolve_runtime_slot_id(meta.cc_texture_id)
        .map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        })
}

fn surface_like_visible_ec_texture_ref<'a>(
    tile_id: u32,
    renders_wet: bool,
    package: &'a udd_assets::tilemeta::TileMetaPackage,
) -> Option<&'a udd_assets::tilemeta::TileMetaItemTextureRef> {
    if !renders_wet {
        return package.main_ec_texture_ref(tile_id);
    }

    choose_visible_surface_texture_ref(package.item_texture_refs(tile_id), tile_id)
}

pub(super) fn choose_visible_surface_texture_ref(
    refs: &[udd_assets::tilemeta::TileMetaItemTextureRef],
    tile_id: u32,
) -> Option<&udd_assets::tilemeta::TileMetaItemTextureRef> {
    visible_surface_texture_ref_candidates(refs, tile_id)
        .find(|texture_ref| texture_ref.is_primary_selected())
        .or_else(|| visible_surface_texture_ref_candidates(refs, tile_id).next())
}

fn visible_surface_texture_ref_candidates<'a>(
    refs: &'a [udd_assets::tilemeta::TileMetaItemTextureRef],
    tile_id: u32,
) -> impl Iterator<Item = &'a udd_assets::tilemeta::TileMetaItemTextureRef> {
    refs.iter().filter(move |texture_ref| {
        !texture_ref.is_auxiliary()
            && texture_ref.texture_id != tile_id
            && matches!(
                texture_ref.stable_role(),
                udd_assets::tilemeta::EcMaterialStableRole::Base
                    | udd_assets::tilemeta::EcMaterialStableRole::SecondaryBase
            )
    })
}

pub(super) fn surface_like_legacy_ec_texture_id_fallback(
    tilemeta: &udd_assets::tilemeta::TileMetaItemTile,
    renders_wet: bool,
) -> Option<u32> {
    if tilemeta.ec_texture_id == 0 || renders_wet {
        return None;
    }

    Some(tilemeta.ec_texture_id)
}

/// The classic-water land fallback for a surface-like static that renders wet in Enhanced mode
/// (`static_tile_renders_wet`), not merely one with the IsWet property.
fn wet_surface_like_water_resolution(
    renders_wet: bool,
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
) -> Option<SurfaceLikeTexLandEcResolution> {
    if !renders_wet {
        return None;
    }

    if let Some(layer) =
        package.resolve_material_layer_slot(CLASSIC_WATER_LAND_TILE_ID, EC_WATER_BASE_LAYER_INDEX)
    {
        if let Some(runtime_slot_id) = layer.runtime_slot_id {
            return Some(SurfaceLikeTexLandEcResolution {
                runtime_slot_id,
                texture_repetition: valid_texture_repetition(layer.texture_repetition),
                liquid_payload: None,
                kingdom_reborn: false,
            });
        }
    }

    package
        .resolve_runtime_slot_id(CLASSIC_WATER_LAND_TILE_ID)
        .map(|runtime_slot_id| surface_like_tex_land_ec_resolution(package, runtime_slot_id, None))
}

pub(super) fn valid_texture_repetition(repetition: f32) -> f32 {
    if repetition.is_finite() && repetition > 0.0 {
        repetition
    } else {
        1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SurfaceLikeTexLandEcResolution {
    pub(super) runtime_slot_id: u32,
    pub(super) texture_repetition: f32,
    pub(super) liquid_payload: Option<u32>,
    /// A slot of the Kingdom Reborn land package (`kingdom_reborn_static_visual_kind`), not of the Enhanced one.
    pub(super) kingdom_reborn: bool,
}

fn surface_like_tex_land_ec_resolution(
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
    runtime_slot_id: u32,
    selected_texture_id: Option<u32>,
) -> SurfaceLikeTexLandEcResolution {
    let texture_repetition = selected_texture_id
        .and_then(|texture_id| {
            package
                .terrain_provenance()
                .iter()
                .filter(|record| record.selected_texture_id == texture_id)
                .find(|record| {
                    provenance_record_matches_runtime_slot(package, record, runtime_slot_id)
                })
                .map(|record| record.selected_texture_repetition)
        })
        .filter(|repetition| repetition.is_finite() && *repetition > 0.0)
        .or_else(|| {
            package.present_slot(runtime_slot_id).map(|slot| {
                let width = f32::from(slot.width).max(1.0);
                width / CC_TILE_PIXEL_WIDTH
            })
        })
        .unwrap_or(1.0);

    SurfaceLikeTexLandEcResolution {
        runtime_slot_id,
        texture_repetition,
        liquid_payload: None,
        kingdom_reborn: false,
    }
}

pub(super) fn surface_like_ground_material_payload(liquid_payload: Option<u32>) -> (u32, u32) {
    liquid_payload.map_or((0, 0), |payload| (payload, 1))
}

fn provenance_record_matches_runtime_slot(
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
    record: &udd_assets::tex_land_ec::TexLandEcTerrainProvenanceRecord,
    runtime_slot_id: u32,
) -> bool {
    if record.canonical_slot_id != 0
        && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
        && package.present_slot(record.canonical_slot_id).is_some()
    {
        return record.canonical_slot_id == runtime_slot_id;
    }

    if record.alias_slot_id != 0
        && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
        && package.present_slot(record.alias_slot_id).is_some()
    {
        return record.alias_slot_id == runtime_slot_id;
    }

    false
}

#[cfg(test)]
pub(super) fn resolve_static_visual_kind(
    art_source: ClientTextureSource,
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> StaticVisualKind {
    match art_source {
        ClientTextureSource::Cc => {
            classic_static_visual_kind(tile_graphic, tilemeta)
        }
        ClientTextureSource::Ec => resolve_ec_static_visual_kind(
            tile_graphic,
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_static()),
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_land())
                && tileart_surface_has_visible_primary(tilemeta_package.map_or(&[], |package| package.item_texture_refs(tile_graphic as u32))),
            resolve_surface_like_tex_land_ec_slot_id(
                tile_graphic as u32,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            ),
        ),
    }
}

/// The style of the land (`StaticArtSourceState::land_style`), which the statics that continue it take whatever the art
/// source (`static_continues_land`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StaticLandStyle {
    /// No land cache yet: every static keeps its art source.
    #[default]
    Unknown,
    /// The land page atlas holds Classic land (`LandRoutes::atlas`).
    Classic,
    /// The land page atlas holds Enhanced land.
    Enhanced,
    /// The land page atlas holds Kingdom Reborn land: the statics that continue it are drawn as for Enhanced land, from
    /// the Kingdom Reborn item packages where they hold them (`kingdom_reborn_static_visual_kind`).
    KingdomReborn,
}

impl StaticLandStyle {
    pub(super) fn of_land_atlas(atlas: LandTextureSource) -> Self {
        match atlas {
            LandTextureSource::Kr => Self::KingdomReborn,
            atlas if atlas.uses_enhanced_land() => Self::Enhanced,
            _ => Self::Classic,
        }
    }

    /// EC-layout land: Enhanced or Kingdom Reborn.
    pub(super) fn is_enhanced(self) -> bool {
        matches!(self, Self::Enhanced | Self::KingdomReborn)
    }
}

/// `land_style`: the statics that continue the land take its style (`static_continues_land`). Under Classic land
/// Enhanced art draws them from their Classic art, as Classic art mode does; under Enhanced or KR land Classic art draws
/// them the Enhanced way, as Enhanced art mode does.
pub(super) fn resolve_static_visual_kind_cached(
    art_source: ClientTextureSource,
    land_style: StaticLandStyle,
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_cc: Option<&udd_assets::tex_art_cc::TexArtCcPackage>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> StaticVisualKind {
    let mut enhanced = || enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec,
        tex_land_ec, surface_resolution_cache);
    match art_source {
        ClientTextureSource::Cc => {
            let classic = classic_static_visual_kind(tile_graphic, tilemeta);
            if !land_style.is_enhanced() {
                return classic;
            }
            enhanced().filter(|&kind| static_continues_land(kind, tilemeta,
                surface_resolution_cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)))
                .unwrap_or(classic)
        }
        ClientTextureSource::Ec => {
            let Some(enhanced) = enhanced() else {
                return present_classic_static_visual_kind(tile_graphic, tilemeta, tex_art_cc)
                    .unwrap_or(StaticVisualKind::Missing);
            };
            (land_style == StaticLandStyle::Classic && static_continues_land(enhanced, tilemeta,
                surface_resolution_cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)))
                .then(|| present_classic_static_visual_kind(tile_graphic, tilemeta, tex_art_cc))
                .flatten()
                .unwrap_or(enhanced)
        }
    }
}

/// What Enhanced art draws for `tile_graphic`, or `None` when the EC art package has no image for it (Enhanced art mode
/// then falls back to its Classic art).
pub(super) fn enhanced_static_visual_kind(
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> Option<StaticVisualKind> {
    let slot = tex_art_ec?.present_slot(u32::from(tile_graphic))?;
    Some(resolve_ec_static_visual_kind(
        tile_graphic,
        slot.is_static(),
        slot.is_land()
            && tileart_surface_has_visible_primary(tilemeta_package.map_or(&[], |package| package.item_texture_refs(tile_graphic as u32))),
        surface_resolution_cache.resolve_surface_like_tex_land_ec_slot_id(
            ClientTextureSource::Ec,
            tile_graphic as u32,
            tilemeta_package,
            tilemeta,
            tex_land_ec,
        ),
    ))
}

/// The Classic art of `tile_graphic` (`classic_static_visual_kind`), when the Classic art package has it.
pub(super) fn present_classic_static_visual_kind(
    tile_graphic: u16,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_cc: Option<&udd_assets::tex_art_cc::TexArtCcPackage>,
) -> Option<StaticVisualKind> {
    let classic = classic_static_visual_kind(tile_graphic, tilemeta);
    let StaticVisualKind::CcRegular { art_id, fallback_art_id } = classic else { unreachable!() };
    tex_art_cc.is_some_and(|p| p.present_slot(art_id).is_some() || p.present_slot(fallback_art_id).is_some())
        .then_some(classic)
}

/// The Kingdom Reborn packages the statics that continue the Kingdom Reborn land are drawn from
/// (`kingdom_reborn_static_visual_kind`).
#[derive(bevy::ecs::system::SystemParam)]
pub struct KrStaticPackages<'w> {
    items: Option<Res<'w, KrItemPackagesRes>>,
    land: Option<Res<'w, TexLandKrPackageRes>>,
}

impl KrStaticPackages<'_> {
    /// The packages, when the item packages and the land package are all there.
    pub(super) fn sources(&self) -> Option<KrStaticSources<'_>> {
        let (items, land) = (self.items.as_deref()?, self.land.as_deref()?);
        Some(KrStaticSources { tilemeta: &items.tilemeta, art: &items.art, land: &land.0 })
    }

    /// Whether a package arrived or changed.
    pub(super) fn is_changed(&self) -> bool {
        self.items.as_ref().is_some_and(|items| items.is_changed())
            || self.land.as_ref().is_some_and(|land| land.is_changed())
    }
}

/// The Kingdom Reborn tile data, item art (in the Enhanced layouts) and land.
#[derive(Clone, Copy)]
pub(super) struct KrStaticSources<'a> {
    pub(super) tilemeta: &'a udd_assets::tilemeta::TileMetaPackage,
    /// An overlay over the Enhanced art: the items whose world art Kingdom Reborn draws differently ([`Self::art_slot`]).
    pub(super) art: &'a udd_assets::tex_art_ec::TexArtEcPackage,
    pub(super) land: &'a udd_assets::tex_land_ec::TexLandEcPackage,
}

impl<'a> KrStaticSources<'a> {
    /// The present Kingdom Reborn art slot of `graphic`, read through the overlay over the Enhanced art `enhanced`, with
    /// the package holding it.
    pub(super) fn art_slot(self, enhanced: Option<&'a udd_assets::tex_art_ec::TexArtEcPackage>, graphic: u32)
        -> Option<(&'a udd_assets::tex_art_ec::TexArtEcPackage, &'a udd_assets::tex_art_ec::TexArtEcSlotRecord)> {
        self.art.overlay_slot(enhanced.unwrap_or(self.art), graphic)
    }
}

impl StaticVisualKind {
    /// The kind drawn from the Kingdom Reborn packages: its art ids and `material_item_id` from
    /// `KR_ITEM_RECORD_BASE`, its ground slots in the Kingdom Reborn land.
    pub(super) fn kingdom_reborn(self) -> Self {
        match self {
            Self::EcRegular { art_id } => Self::EcRegular { art_id: art_id + KR_ITEM_RECORD_BASE },
            Self::EcSurface { art_id } => Self::EcSurface { art_id: art_id + KR_ITEM_RECORD_BASE },
            Self::TexLandEc { resolution } => Self::TexLandEc {
                resolution: SurfaceLikeTexLandEcResolution { kingdom_reborn: true, ..resolution },
            },
            kind @ (Self::CcRegular { .. } | Self::Missing) => kind,
        }
    }

    pub(super) fn is_kingdom_reborn(self) -> bool {
        match self {
            Self::EcRegular { art_id } | Self::EcSurface { art_id } => art_id >= KR_ITEM_RECORD_BASE,
            Self::TexLandEc { resolution } => resolution.kingdom_reborn,
            Self::CcRegular { .. } | Self::Missing => false,
        }
    }

    /// The item of the item lookups (`ec_item_surface`, `ec_item_liquid`) a static of `graphic` drawn as this kind reads.
    pub(super) fn material_item_id(self, graphic: u16) -> u32 {
        u32::from(graphic) + if self.is_kingdom_reborn() { KR_ITEM_RECORD_BASE } else { 0 }
    }
}

/// Under the Kingdom Reborn land (`StaticLandStyle::KingdomReborn`), a static that continues the land
/// (`static_continues_land`) by the Enhanced data or by Kingdom Reborn's is drawn as Kingdom Reborn draws it: its item
/// art, material and land textures, so it matches the land under and around it (the grass, liquids, lava and swamp, and
/// the water 0x34D2, a sprite in KR). `None` keeps the kind of `resolve_static_visual_kind_cached`: a static that does
/// not continue the land, or one the Kingdom Reborn art lacks (logged once).
pub(super) fn kingdom_reborn_static_visual_kind(
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    kr: KrStaticSources<'_>,
    cache: &mut StaticSurfaceLikeResolutionCache,
) -> Option<StaticVisualKind> {
    if let Some(&cached) = cache.kingdom_reborn_kinds.get(&tile_graphic) {
        return cached;
    }
    let graphic = u32::from(tile_graphic);
    let enhanced_continues = enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec,
        tex_land_ec, cache).is_some_and(|kind| static_continues_land(kind, tilemeta,
            cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)));
    let kr_tilemeta = kr.tilemeta.item_tile(graphic);
    let kr_kind = kr.art_slot(tex_art_ec, graphic).map(|(_, slot)| resolve_ec_static_visual_kind(
        tile_graphic,
        slot.is_static(),
        slot.is_land() && tileart_surface_has_visible_primary(kr.tilemeta.item_texture_refs(graphic)),
        resolve_surface_like_tex_land_ec_slot_id(graphic, Some(kr.tilemeta), kr_tilemeta, Some(kr.land)),
    ));
    let kr_terrain_textures = cache.kingdom_reborn_terrain_textures.get_or_insert_with(|| terrain_texture_ids(kr.land));
    let kr_continues = kr_kind.is_some_and(|kind| static_continues_land(kind, kr_tilemeta,
        item_draws_terrain_texture(graphic, Some(kr.tilemeta), kr_terrain_textures)));
    let kind = if !(enhanced_continues || kr_continues) {
        None
    } else if let Some(kind) = kr_kind {
        Some(kind.kingdom_reborn())
    } else {
        console_logger::one(LogSev::Info, LogAbout::RenderWorldArt, &format!(
            "Static {tile_graphic:#06x} continues the land, but the Kingdom Reborn art (the overlay in \
             tex_art_ec.uddp) lacks it: it keeps the Enhanced item data under the Kingdom Reborn land."));
        None
    };
    cache.kingdom_reborn_kinds.insert(tile_graphic, kind);
    kind
}

/// The tile data a static of `graphic` drawn as `kind` is drawn and picked by, the package and its item: Kingdom
/// Reborn's (`kr`) for a kind drawn from the Kingdom Reborn packages, as the KR client draws its art by its own tileart
/// (its NoDraw, Surface and Rotated flags differ from the Enhanced ones on some of the statics that continue the land);
/// `cc_art`, the clients' tables without the tilemeta overrides (`TileMetaPackageRes::for_art`), for one drawn with
/// Classic art, which the overrides were not measured against; else the Enhanced `enhanced`.
pub(super) fn drawn_tile_data<'a>(
    kind: Option<StaticVisualKind>,
    graphic: u16,
    enhanced: Option<&'a udd_assets::tilemeta::TileMetaPackage>,
    cc_art: Option<&'a udd_assets::tilemeta::TileMetaPackage>,
    kr: Option<KrStaticSources<'a>>,
) -> (Option<&'a udd_assets::tilemeta::TileMetaPackage>, Option<&'a udd_assets::tilemeta::TileMetaItemTile>) {
    let package = match (kind, kr) {
        (Some(kind), Some(kr)) if kind.is_kingdom_reborn() => Some(kr.tilemeta),
        (Some(StaticVisualKind::CcRegular { .. }), _) => cc_art,
        _ => enhanced,
    };
    (package, package.and_then(|package| package.item_tile(u32::from(graphic))))
}

/// Whether a static continues the land, so it takes the land's style (`StaticLandStyle`) whatever the art source.
/// `enhanced` is what Enhanced art draws for it; `terrain_texture` is `landlike_item_texture`. The data decides:
/// - its Enhanced material is drawn from the terrain: a land liquid (`TexLandEc` with a liquid payload: the water and
///   foam 0x1796–0x17B2 and 0x346E–0x34D2, the acid and the lava the same water shader draws), or an item surface or
///   land-textured item whose visible texture a terrain material draws (grass, dirt, snow, swamp, the lava fields);
/// - and it is not a built floor (NoShoot): cobblestones, pavers, marble, flagstones, bricks and the other town floors
///   share the terrain textures of the town floor land, but the maps place them almost only raised or roofed, as house,
///   castle and dungeon floors, so they keep their art source (docs/agent_notes/water_terrain_static_resolution.md).
pub(super) fn static_continues_land(
    enhanced: StaticVisualKind,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    terrain_texture: bool,
) -> bool {
    let built_floor = tilemeta.is_some_and(|meta| meta.flags & TILE_FLAG_NO_SHOOT != 0);
    match enhanced {
        StaticVisualKind::TexLandEc { resolution } if resolution.liquid_payload.is_some() => true,
        StaticVisualKind::TexLandEc { .. } | StaticVisualKind::EcSurface { .. } => terrain_texture && !built_floor,
        StaticVisualKind::CcRegular { .. } | StaticVisualKind::EcRegular { .. } | StaticVisualKind::Missing => false,
    }
}

/// `STATIC_FLAG_LAND_GRADE` for a static of `tile_graphic` drawn as `visual_kind`: set when it continues the land by
/// the Enhanced data (`static_continues_land`, whatever the art source and land), or is drawn from the Kingdom Reborn
/// packages, which only the statics that continue the KR land are (`kingdom_reborn_static_visual_kind`), or is foliage
/// by the tile data it is drawn by (`drawn_flags`, `drawn_tile_data`): the leaves take the knee tuned for the grass, as
/// without it they stood out of the KR screenshots' foliage, while trunks without the Foliage flag keep their colour.
pub(super) fn land_grade_flag(
    visual_kind: StaticVisualKind,
    tile_graphic: u16,
    drawn_flags: u64,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    cache: &mut StaticSurfaceLikeResolutionCache,
) -> u32 {
    let continues = visual_kind.is_kingdom_reborn()
        || drawn_flags & TILE_FLAG_FOLIAGE != 0
        || enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec, tex_land_ec, cache)
            .is_some_and(|kind| static_continues_land(kind, tilemeta,
                cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)));
    if continues { STATIC_FLAG_LAND_GRADE } else { 0 }
}

/// Whether one of the visible Enhanced textures of item `tile_id` (its world-art base layers) is a texture a terrain
/// material of the EC land package draws.
fn item_draws_terrain_texture(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    terrain_textures: &HashSet<u32>,
) -> bool {
    tilemeta_package.is_some_and(|package| package.item_texture_refs(tile_id).iter().any(|texture_ref|
        texture_ref.is_world_art() && !texture_ref.is_auxiliary()
            && matches!(texture_ref.stable_role(), udd_assets::tilemeta::EcMaterialStableRole::Base
                | udd_assets::tilemeta::EcMaterialStableRole::SecondaryBase)
            && terrain_textures.contains(&texture_ref.texture_id)))
}

/// The textures the terrain materials of the EC land package draw: each terrain record's selected and primary texture.
fn terrain_texture_ids(tex_land_ec: &udd_assets::tex_land_ec::TexLandEcPackage) -> HashSet<u32> {
    tex_land_ec.terrain_provenance().iter()
        .flat_map(|record| [record.selected_texture_id, record.primary_texture_id])
        .filter(|&texture_id| texture_id != udd_assets::tex_land_ec::MISSING_TEXTURE_ID)
        .collect()
}

pub(super) fn tileart_surface_has_visible_primary(
    refs: &[udd_assets::tilemeta::TileMetaItemTextureRef],
) -> bool {
    use udd_assets::tilemeta::EcMaterialStableRole;
    // Some packed land slots contain the first source support image, not albedo.
    // Water's primary-selected alpha/normal input must use its visible material
    // route instead of painting that support image onto a tileart ground quad.
    // The packer uses ArtData.ec_texture, preserved as PRIMARY_SELECTED. The
    // runtime's role-aware main selector can choose a different source image;
    // it cannot change the pixels already stored in the tileart atlas.
    // The rejected roles are the shader inputs that the former file-name roles (alpha mask, generic
    // mask, noise, normal-like, image support, effect-only) were mapped to.
    !refs.iter().find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected())
        .is_some_and(|texture_ref| matches!(texture_ref.stable_role(),
        EcMaterialStableRole::BlendAlpha | EcMaterialStableRole::HueMask
            | EcMaterialStableRole::Noise | EcMaterialStableRole::WaterBump
            | EcMaterialStableRole::Unused | EcMaterialStableRole::Unknown
    ))
}

pub(super) fn resolve_ec_static_visual_kind(
    tile_graphic: u16,
    has_tex_art_ec_static_slot: bool,
    has_tex_art_ec_surface_slot: bool,
    surface_like_land_resolution: Option<SurfaceLikeTexLandEcResolution>,
) -> StaticVisualKind {
    if has_tex_art_ec_static_slot {
        // Walkable sprites (stairs, bridges, furniture) still own their art.
        // The Surface tiledata flag does not make their graphic id a land id.
        StaticVisualKind::EcRegular { art_id: tile_graphic as u32 }
    } else if let Some(resolution) = surface_like_land_resolution.filter(|r| r.liquid_payload.is_some()) {
        StaticVisualKind::TexLandEc { resolution }
    } else if has_tex_art_ec_surface_slot {
        // Tileart owns this image and its sampling window. A terrain texture with
        // the same source id cannot replace an authored item-owned surface.
        StaticVisualKind::EcSurface { art_id: tile_graphic as u32 }
    } else if let Some(resolution) = surface_like_land_resolution {
        StaticVisualKind::TexLandEc { resolution }
    } else {
        StaticVisualKind::EcRegular {
            art_id: tile_graphic as u32,
        }
    }
}

/// Whether a static takes the wet render path (animated water UV distortion). This is a render
/// decision, not the object property: `TileMetaItemTile::is_wet` (IsWet) stays the gameplay flag.
/// - Classic art mode: the IsWet flag.
/// - Enhanced art mode, item with an EC material: only its shader decides (`UOWaterShader`); IsWet
///   does not override it, so water-like sprite-shader items render dry.
/// - Enhanced art mode, item without an EC material: it draws the Classic fallback art, so IsWet.
pub(crate) fn static_tile_renders_wet(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    art_id: u32,
    art_source: Option<ClientTextureSource>,
) -> bool {
    let classic_wet = tilemeta.is_some_and(udd_assets::tilemeta::TileMetaItemTile::is_wet);
    if art_source != Some(ClientTextureSource::Ec) {
        return classic_wet;
    }
    match tilemeta_package.filter(|package| package.has_ec_material(art_id)) {
        Some(package) => package.uses_water_shader(art_id),
        None => classic_wet,
    }
}

#[derive(Resource, Default)]
pub struct StaticArtSourceState {
    pub active_source: Option<ClientTextureSource>,
    pub warned_unavailable_source: Option<ClientTextureSource>,
    /// The style of the land page atlas (`LandRoutes::atlas`), which the statics that continue the land take
    /// (`static_continues_land`).
    pub land_style: StaticLandStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct StaticSurfaceLikeResolutionCacheKey {
    pub(super) art_source: u8,
    pub(super) tile_id: u32,
}

#[derive(Resource, Default)]
pub struct StaticSurfaceLikeResolutionCache {
    entries: HashMap<StaticSurfaceLikeResolutionCacheKey, Option<SurfaceLikeTexLandEcResolution>>,
    /// `terrain_texture_ids`, read once from the EC land package.
    terrain_textures: Option<HashSet<u32>>,
    /// `item_draws_terrain_texture` by item.
    terrain_texture_items: HashMap<u16, bool>,
    /// `kingdom_reborn_static_visual_kind` by item.
    kingdom_reborn_kinds: HashMap<u16, Option<StaticVisualKind>>,
    /// `terrain_texture_ids` of the Kingdom Reborn land package.
    kingdom_reborn_terrain_textures: Option<HashSet<u32>>,
}

impl StaticSurfaceLikeResolutionCache {
    /// Whether item `tile_graphic` draws a terrain texture in Enhanced art (`item_draws_terrain_texture`).
    pub(super) fn landlike_item_texture(
        &mut self,
        tile_graphic: u16,
        tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
        tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    ) -> bool {
        if let Some(&cached) = self.terrain_texture_items.get(&tile_graphic) {
            return cached;
        }
        let Some(tex_land_ec) = tex_land_ec else { return false; };
        let terrain_textures = self.terrain_textures.get_or_insert_with(|| terrain_texture_ids(tex_land_ec));
        let draws = item_draws_terrain_texture(u32::from(tile_graphic), tilemeta_package, terrain_textures);
        self.terrain_texture_items.insert(tile_graphic, draws);
        draws
    }

    fn resolve_surface_like_tex_land_ec_slot_id(
        &mut self,
        art_source: ClientTextureSource,
        tile_id: u32,
        tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
        tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
        tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    ) -> Option<SurfaceLikeTexLandEcResolution> {
        let key = StaticSurfaceLikeResolutionCacheKey {
            art_source: static_surface_like_resolution_art_source_tag(art_source),
            tile_id,
        };
        self.get_or_insert_with(key, || {
            resolve_surface_like_tex_land_ec_slot_id(
                tile_id,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            )
        })
    }

    pub(super) fn get_or_insert_with(
        &mut self,
        key: StaticSurfaceLikeResolutionCacheKey,
        resolve: impl FnOnce() -> Option<SurfaceLikeTexLandEcResolution>,
    ) -> Option<SurfaceLikeTexLandEcResolution> {
        if let Some(cached) = self.entries.get(&key).copied() {
            return cached;
        }

        let resolved = resolve();
        self.entries.insert(key, resolved);
        resolved
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(super) fn static_surface_like_resolution_art_source_tag(source: ClientTextureSource) -> u8 {
    match source {
        ClientTextureSource::Cc => 0,
        ClientTextureSource::Ec => 1,
    }
}

fn source_available(
    source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> bool {
    match source {
        ClientTextureSource::Cc => tex_art_cc_res.is_some(),
        ClientTextureSource::Ec => {
            tex_art_ec_res.is_some() && tex_land_ec_res.is_some() && tilemeta_res.is_some()
        }
    }
}

pub(super) fn resolve_effective_art_source(
    requested_source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> Option<ClientTextureSource> {
    if source_available(
        requested_source,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
    ) {
        return Some(requested_source);
    }

    ClientTextureSource::ALL.into_iter().find(|candidate| {
        *candidate != requested_source
            && source_available(
                *candidate,
                tex_art_cc_res,
                tex_art_ec_res,
                tex_land_ec_res,
                tilemeta_res,
            )
    })
}

pub fn sys_sync_static_art_source(
    settings: Res<Settings>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    land_cache: Option<Res<crate::core::texture_cache::land::cache::LandTextureCache>>,
    mut source_state: ResMut<StaticArtSourceState>,
) {
    let land_style = land_cache.map_or(StaticLandStyle::Unknown,
        |cache| StaticLandStyle::of_land_atlas(cache.routes().atlas()));
    // Written only on a change, like `warned_unavailable_source` below.
    if source_state.land_style != land_style {
        source_state.land_style = land_style;
    }

    let requested_source = settings.graphics.art_texture_source;
    let effective_source = resolve_effective_art_source(
        requested_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    if effective_source != Some(requested_source)
        && source_state.warned_unavailable_source != Some(requested_source)
    {
        source_state.warned_unavailable_source = Some(requested_source);
        let fallback_label = effective_source
            .map(|source| source.art_label())
            .unwrap_or("none");
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "Requested {} for static sprites, but the required package/metadata is unavailable. Falling back to {}.",
                requested_source.art_label(),
                fallback_label,
            ),
        );
    } else if effective_source == Some(requested_source) && source_state.warned_unavailable_source.is_some() {
        // A redundant write here invalidates asynchronous visibility queries on
        // every frame even though the selected artwork has not changed.
        source_state.warned_unavailable_source = None;
    }

    if source_state.active_source == effective_source {
        return;
    }

    log_system_add_update::<super::DrawStaticSpritesPlugin>(fname!());

    source_state.active_source = effective_source;

    match effective_source {
        Some(source) if source == requested_source => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!("Switched static sprite source to {}.", source.art_label()),
            );
        }
        Some(source) => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!(
                    "Switched static sprite source to {} (requested {}).",
                    source.art_label(),
                    requested_source.art_label(),
                ),
            );
        }
        None => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::RenderWorldArt,
                "No static sprite art package is available; non-dot statics will be skipped.",
            );
        }
    }
}
