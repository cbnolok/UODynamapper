//! The texture refs of a `tilemeta` item, for the details panel: each ref's block and item position, stable
//! role, physical package and logical family, primary/aux flags and stretch, the main-texture choice with its
//! reason and confidence, and which package beside the tilemeta holds the image.

use std::path::Path;

use udd_assets::tex_aux_ec::TexAuxEcPackage;
use udd_assets::tilemeta::{
    EcMaterialPhysicalPackage, EcMaterialStableRole, TileMetaItemTextureRef, TileMetaPackage,
    TILEMETA_EC_TEXTURE_BLOCK_INDEX,
};
use udd_assets::{TexArtEcPackage, TexLandEcPackage};

use crate::models::{TileMetaMainTextureInfo, TileMetaTextureRefInfo};

pub const TEX_ART_EC_FILE_NAME: &str = "tex_art_ec.uddp";
pub const TEX_AUX_EC_FILE_NAME: &str = "tex_aux_ec.uddp";
pub const TEX_LAND_EC_FILE_NAME: &str = "tex_land_ec.uddp";

/// Water-shader maps a liquid item draws from its `tex_land_ec` payload (positions 0, 1 and 2).
const LIQUID_PAYLOAD_ROLES: [EcMaterialStableRole; 3] = [
    EcMaterialStableRole::WaterBump,
    EcMaterialStableRole::Base,
    EcMaterialStableRole::WaterReflection,
];

/// The packages beside a `tilemeta.uddp` that hold item images; each is optional.
#[derive(Default)]
pub struct ImageHolders {
    pub tex_art_ec: Option<TexArtEcPackage>,
    pub tex_aux_ec: Option<TexAuxEcPackage>,
    pub tex_land_ec: Option<TexLandEcPackage>,
}

impl ImageHolders {
    /// Load the image packages found in `directory`; a missing or unreadable one is left out.
    pub fn load_beside(directory: &Path) -> Self {
        fn load<T>(path: std::path::PathBuf, load: impl FnOnce(&Path) -> color_eyre::Result<T>) -> Option<T> {
            if !path.is_file() {
                return None;
            }
            load(&path)
                .map_err(|error| log::warn!("texture-ref view: {} not used: {error:#}", path.display()))
                .ok()
        }
        Self {
            tex_art_ec: load(directory.join(TEX_ART_EC_FILE_NAME), |path| TexArtEcPackage::load(path)),
            tex_aux_ec: load(directory.join(TEX_AUX_EC_FILE_NAME), |path| TexAuxEcPackage::load(path)),
            tex_land_ec: load(directory.join(TEX_LAND_EC_FILE_NAME), |path| TexLandEcPackage::load(path)),
        }
    }

    /// The package names loaded, for the panel header.
    pub fn loaded_names(&self) -> Vec<&'static str> {
        [
            (self.tex_art_ec.is_some(), TEX_ART_EC_FILE_NAME),
            (self.tex_aux_ec.is_some(), TEX_AUX_EC_FILE_NAME),
            (self.tex_land_ec.is_some(), TEX_LAND_EC_FILE_NAME),
        ]
        .into_iter()
        .filter_map(|(loaded, name)| loaded.then_some(name))
        .collect()
    }
}

/// What the holders know about one ref.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HolderFacts {
    /// The item's water material resolves to this `tex_land_ec` payload.
    pub liquid_payload: Option<u32>,
    /// `tex_art_ec` has a present slot for the item.
    pub art_slot_present: bool,
    /// `tex_aux_ec` has a slot for the ref's texture id; `Some(true)` when it is the missing-texture alias.
    pub aux_slot_missing_source: Option<bool>,
}

/// The package holding a ref's image, as the renderer reads it: a liquid item's water maps from its
/// `tex_land_ec` payload, the item's primary from its `tex_art_ec` slot, support images from `tex_aux_ec`.
/// Only the EC block is read; the legacy and enhanced art blocks are not.
pub fn image_holder(tex_ref: &TileMetaItemTextureRef, facts: HolderFacts) -> String {
    if tex_ref.block_index != TILEMETA_EC_TEXTURE_BLOCK_INDEX {
        return format!("not read (block {})", tex_ref.block_index);
    }
    if let Some(payload) = facts.liquid_payload {
        let is_liquid_map = tex_ref.is_world_art()
            && tex_ref.physical_package() == EcMaterialPhysicalPackage::Texture
            && LIQUID_PAYLOAD_ROLES.contains(&tex_ref.stable_role());
        if is_liquid_map {
            return format!("tex_land_ec (liquid payload {payload})");
        }
    }
    if tex_ref.is_primary_selected() && facts.art_slot_present {
        return "tex_art_ec (item slot)".into();
    }
    match facts.aux_slot_missing_source {
        Some(false) => "tex_aux_ec".into(),
        Some(true) => "tex_aux_ec (missing-texture image)".into(),
        None => "not packed".into(),
    }
}

/// The texture refs of item `tile_id` and its main-texture choice.
pub fn item_texture_refs(
    tilemeta: &TileMetaPackage,
    tile_id: u32,
    holders: &ImageHolders,
) -> (Vec<TileMetaTextureRefInfo>, Option<TileMetaMainTextureInfo>) {
    let refs = tilemeta.item_texture_refs(tile_id);
    let main = tilemeta
        .main_ec_texture_id_role_aware(tile_id)
        .map(|(texture_id, role, reason)| TileMetaMainTextureInfo {
            texture_id,
            stable_role: role.as_str(),
            reason: reason.as_str(),
            low_confidence: reason.is_low_confidence(),
        });
    let main_position = main.as_ref().and_then(|main| {
        refs.iter().position(|tex_ref| {
            tex_ref.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX && tex_ref.texture_id == main.texture_id
        })
    });

    let liquid_payload = holders.tex_land_ec.as_ref().and_then(|tex_land_ec| {
        tilemeta
            .item_material(tile_id, TILEMETA_EC_TEXTURE_BLOCK_INDEX)
            .and_then(|material| tex_land_ec.source_liquid_payload_for_item(material, refs))
    });
    let art_slot_present = holders
        .tex_art_ec
        .as_ref()
        .is_some_and(|tex_art_ec| tex_art_ec.present_slot(tile_id).is_some());

    let rows = refs
        .iter()
        .enumerate()
        .map(|(index, tex_ref)| {
            let facts = HolderFacts {
                liquid_payload,
                art_slot_present,
                aux_slot_missing_source: holders
                    .tex_aux_ec
                    .as_ref()
                    .and_then(|aux| aux.slot_for_texture_id(tex_ref.texture_id))
                    .map(|slot| slot.is_missing_source()),
            };
            TileMetaTextureRefInfo {
                block_index: tex_ref.block_index,
                item_index: tex_ref.item_index,
                texture_id: tex_ref.texture_id,
                stable_role: tex_ref.stable_role().as_str(),
                physical_package: format!("{:?}", tex_ref.physical_package()),
                logical_family: format!("{:?}", tex_ref.logical_family()),
                primary_selected: tex_ref.is_primary_selected(),
                auxiliary: tex_ref.is_auxiliary(),
                stretch: tex_ref.texture_stretch,
                is_main: main_position == Some(index),
                image_holder: image_holder(tex_ref, facts),
            }
        })
        .collect();
    (rows, main)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use udd_assets::tilemeta::{EcMaterialLogicalFamily, TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED};

    fn world_art_ref(block_index: u8, role: EcMaterialStableRole, primary: bool) -> TileMetaItemTextureRef {
        TileMetaItemTextureRef {
            texture_id: 1234,
            texture_type: EcMaterialLogicalFamily::WorldArt as u8,
            block_index,
            physical_package: EcMaterialPhysicalPackage::Texture as u8,
            stable_role: role as u8,
            flags: if primary { TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED } else { 0 },
            ..TileMetaItemTextureRef::zeroed()
        }
    }

    #[test]
    fn a_liquid_items_water_maps_come_from_its_tex_land_ec_payload() {
        let facts = HolderFacts { liquid_payload: Some(203), art_slot_present: true, aux_slot_missing_source: Some(false) };
        for role in LIQUID_PAYLOAD_ROLES {
            assert_eq!(image_holder(&world_art_ref(0, role, false), facts), "tex_land_ec (liquid payload 203)");
        }
        // The fourth water map is a support image, and other blocks are not the item's material.
        assert_eq!(image_holder(&world_art_ref(0, EcMaterialStableRole::WaterFourthStage, false), facts), "tex_aux_ec");
        assert_eq!(image_holder(&world_art_ref(1, EcMaterialStableRole::Base, true), facts), "not read (block 1)");
    }

    #[test]
    fn the_primary_of_an_item_with_an_art_slot_comes_from_tex_art_ec() {
        let facts = HolderFacts { liquid_payload: None, art_slot_present: true, aux_slot_missing_source: None };
        assert_eq!(image_holder(&world_art_ref(0, EcMaterialStableRole::Base, true), facts), "tex_art_ec (item slot)");
        let no_slot = HolderFacts { art_slot_present: false, ..facts };
        assert_eq!(image_holder(&world_art_ref(0, EcMaterialStableRole::Base, true), no_slot), "not packed");
    }

    /// Directory with a `tilemeta.uddp` and its image packages, for [`installed_packages_name_a_holder_for_every_support_ref`].
    const PACKAGE_DIR_ENV: &str = "UDD_PACKAGE_DIR";

    #[test]
    #[ignore = "needs packages: set UDD_PACKAGE_DIR to a folder with tilemeta.uddp and its image packages"]
    fn installed_packages_name_a_holder_for_every_support_ref() {
        let dir = std::path::PathBuf::from(std::env::var_os(PACKAGE_DIR_ENV).expect("set UDD_PACKAGE_DIR"));
        let tilemeta = TileMetaPackage::load(dir.join("tilemeta.uddp")).expect("load tilemeta");
        let holders = ImageHolders::load_beside(&dir);
        assert_eq!(holders.loaded_names().len(), 3, "all three image packages beside the tilemeta");

        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let mut unpacked_support = Vec::new();
        for item in tilemeta.item_tiles() {
            let (rows, main) = item_texture_refs(&tilemeta, item.tile_id, &holders);
            let has_material = rows.iter().any(|row| row.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX && row.primary_selected);
            assert!(!has_material || main.is_some(), "item {} has a material but no main texture", item.tile_id);
            for row in rows {
                let holder = row.image_holder.split(" (").next().unwrap_or_default().to_string();
                let is_support = has_material
                    && row.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX
                    && !row.primary_selected
                    && row.logical_family == format!("{:?}", EcMaterialLogicalFamily::WorldArt)
                    && row.texture_id != 0;
                if is_support && holder == "not packed" {
                    unpacked_support.push((item.tile_id, row.item_index, row.texture_id));
                }
                *counts.entry(holder).or_default() += 1;
            }
        }
        println!("image holders: {counts:?}");
        assert!(unpacked_support.is_empty(), "support refs without a holder: {unpacked_support:?}");
    }

    #[test]
    fn support_refs_come_from_tex_aux_ec_or_are_not_packed() {
        let support = world_art_ref(0, EcMaterialStableRole::BlendAlpha, false);
        let aux = |missing| HolderFacts { aux_slot_missing_source: missing, ..HolderFacts::default() };
        assert_eq!(image_holder(&support, aux(Some(false))), "tex_aux_ec");
        assert_eq!(image_holder(&support, aux(Some(true))), "tex_aux_ec (missing-texture image)");
        assert_eq!(image_holder(&support, aux(None)), "not packed");
    }
}
