//! The texture refs of a `tilemeta` item, for the details panel: each ref's block and item position, stable
//! role, physical package and logical family, primary/aux flags and stretch, the main-texture choice with its
//! reason and confidence, and which image package holds the image, with its pixels.

use std::fmt;
use std::path::{Path, PathBuf};

use color_eyre::eyre;

use udd_assets::tex_aux_ec::TexAuxEcPackage;
use udd_assets::tilemeta::{
    EcMaterialPhysicalPackage, EcMaterialStableRole, TileMetaItemTextureRef, TileMetaPackage,
    TILEMETA_EC_TEXTURE_BLOCK_INDEX,
};
use udd_assets::{TexArtEcPackage, TexLandEcPackage};

use crate::logic::companions::find_companion;
use crate::logic::decoding::decode_atlas_rect;
use crate::logic::discovery::atlas_page_info;
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

/// The packages that hold a `tilemeta.uddp`'s item images, found beside it or in the package folder; each is
/// optional.
#[derive(Default)]
pub struct ImageHolders {
    pub tex_art_ec: Option<TexArtEcPackage>,
    pub tex_aux_ec: Option<TexAuxEcPackage>,
    pub tex_land_ec: Option<TexLandEcPackage>,
    /// The files loaded, for the panel header.
    pub loaded_paths: Vec<PathBuf>,
}

/// The pixels of a ref's image.
pub struct RefImage {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

impl ImageHolders {
    /// Load the image packages of the tilemeta at `tilemeta_path` ([`find_companion`]); a missing or unreadable one
    /// is left out.
    pub fn load(tilemeta_path: &Path, companion_dir: Option<&Path>) -> Self {
        fn open<T>(path: Option<PathBuf>, loaded_paths: &mut Vec<PathBuf>, load: impl FnOnce(&Path) -> eyre::Result<T>)
            -> Option<T> {
            let path = path?;
            match load(&path) {
                Ok(package) => {
                    loaded_paths.push(path);
                    Some(package)
                }
                Err(error) => {
                    log::warn!("texture-ref view: {} not used: {error:#}", path.display());
                    None
                }
            }
        }
        let find = |file_name| find_companion(tilemeta_path, companion_dir, file_name);
        let mut loaded_paths = Vec::new();
        Self {
            tex_art_ec: open(find(TEX_ART_EC_FILE_NAME), &mut loaded_paths, |path| TexArtEcPackage::load(path)),
            tex_aux_ec: open(find(TEX_AUX_EC_FILE_NAME), &mut loaded_paths, |path| TexAuxEcPackage::load(path)),
            tex_land_ec: open(find(TEX_LAND_EC_FILE_NAME), &mut loaded_paths, |path| TexLandEcPackage::load(path)),
            loaded_paths,
        }
    }

    /// The pixels of `tex_ref`'s image in its [`ImageHolder`]: item `tile_id`'s art slot, its liquid payload's land
    /// slot, or the ref's aux slot.
    pub fn ref_image(&self, tile_id: u32, tex_ref: &TileMetaTextureRefInfo) -> eyre::Result<RefImage> {
        /// The pixels of present slot `id` of an atlas package (`tex_art_ec` or `tex_land_ec`).
        macro_rules! atlas_slot_image {
            ($package:expr, $name:literal, $id:expr) => {{
                let package = $package.as_ref().ok_or_else(|| eyre::eyre!("no {} loaded", $name))?;
                let slot = package.present_slot($id).ok_or_else(|| eyre::eyre!("{} has no slot {}", $name, $id))?;
                let page = package.pages().iter().find(|page| page.page_index == slot.page_index)
                    .ok_or_else(|| eyre::eyre!("{} has no page {}", $name, slot.page_index))?;
                let info = atlas_page_info(package.atlas_width(), package.atlas_height(), page.used_width,
                    page.used_height, page.pixel_format);
                let bytes = package.read_page_bytes(slot.page_index)?;
                let rect = [slot.x, slot.y, slot.width, slot.height].map(usize::from);
                let rgba = decode_atlas_rect(info, &bytes, rect)
                    .ok_or_else(|| eyre::eyre!("{} page {} does not decode", $name, slot.page_index))?;
                RefImage { width: slot.width, height: slot.height, rgba }
            }};
        }
        Ok(match tex_ref.image_holder {
            ImageHolder::ArtSlot => atlas_slot_image!(self.tex_art_ec, "tex_art_ec", tile_id),
            ImageHolder::LiquidPayload { land_tile } => {
                let land = self.tex_land_ec.as_ref().ok_or_else(|| eyre::eyre!("no tex_land_ec loaded"))?;
                let slot_id = land.resolve_runtime_slot_id(land_tile)
                    .ok_or_else(|| eyre::eyre!("tex_land_ec resolves no slot for land tile {land_tile}"))?;
                atlas_slot_image!(self.tex_land_ec, "tex_land_ec", slot_id)
            }
            ImageHolder::AuxSlot { .. } => {
                let aux = self.tex_aux_ec.as_ref().ok_or_else(|| eyre::eyre!("no tex_aux_ec loaded"))?;
                let slot = aux.slot_for_texture_id(tex_ref.texture_id)
                    .ok_or_else(|| eyre::eyre!("tex_aux_ec has no slot {}", tex_ref.texture_id))?;
                let rgba = aux.load_rgba(tex_ref.texture_id)?
                    .ok_or_else(|| eyre::eyre!("tex_aux_ec has no payload for {}", tex_ref.texture_id))?;
                RefImage { width: slot.width, height: slot.height, rgba }
            }
            ImageHolder::NotRead { .. } | ImageHolder::NotPacked => eyre::bail!("the ref has no packed image"),
        })
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

/// Where a ref's image is, as the renderer reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageHolder {
    /// The ref is in a block the renderer does not read.
    NotRead { block: u8 },
    /// A liquid item's water map: the `tex_land_ec` payload of land tile `land_tile`.
    LiquidPayload { land_tile: u32 },
    /// The item's primary: its `tex_art_ec` slot.
    ArtSlot,
    /// A support image: its `tex_aux_ec` slot, the missing-texture image for a source the client lacks.
    AuxSlot { missing_source: bool },
    NotPacked,
}

impl ImageHolder {
    /// The holder has pixels to show ([`ImageHolders::ref_image`]).
    pub fn has_image(self) -> bool {
        matches!(self, Self::LiquidPayload { .. } | Self::ArtSlot | Self::AuxSlot { .. })
    }
}

impl fmt::Display for ImageHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRead { block } => write!(f, "not read (block {block})"),
            Self::LiquidPayload { land_tile } => write!(f, "tex_land_ec (liquid payload {land_tile})"),
            Self::ArtSlot => f.write_str("tex_art_ec (item slot)"),
            Self::AuxSlot { missing_source: false } => f.write_str("tex_aux_ec"),
            Self::AuxSlot { missing_source: true } => f.write_str("tex_aux_ec (missing-texture image)"),
            Self::NotPacked => f.write_str("not packed"),
        }
    }
}

/// The package holding a ref's image, as the renderer reads it: a liquid item's water maps from its
/// `tex_land_ec` payload, the item's primary from its `tex_art_ec` slot, support images from `tex_aux_ec`.
/// Only the EC block is read; the legacy and enhanced art blocks are not.
pub fn image_holder(tex_ref: &TileMetaItemTextureRef, facts: HolderFacts) -> ImageHolder {
    if tex_ref.block_index != TILEMETA_EC_TEXTURE_BLOCK_INDEX {
        return ImageHolder::NotRead { block: tex_ref.block_index };
    }
    if let Some(land_tile) = facts.liquid_payload {
        let is_liquid_map = tex_ref.is_world_art()
            && tex_ref.physical_package() == EcMaterialPhysicalPackage::Texture
            && LIQUID_PAYLOAD_ROLES.contains(&tex_ref.stable_role());
        if is_liquid_map {
            return ImageHolder::LiquidPayload { land_tile };
        }
    }
    if tex_ref.is_primary_selected() && facts.art_slot_present {
        return ImageHolder::ArtSlot;
    }
    match facts.aux_slot_missing_source {
        Some(missing_source) => ImageHolder::AuxSlot { missing_source },
        None => ImageHolder::NotPacked,
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

    /// [`image_holder`] as the panel shows it.
    fn holder(tex_ref: &TileMetaItemTextureRef, facts: HolderFacts) -> String {
        image_holder(tex_ref, facts).to_string()
    }

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
            assert_eq!(holder(&world_art_ref(0, role, false), facts), "tex_land_ec (liquid payload 203)");
        }
        // The fourth water map is a support image, and other blocks are not the item's material.
        assert_eq!(holder(&world_art_ref(0, EcMaterialStableRole::WaterFourthStage, false), facts), "tex_aux_ec");
        assert_eq!(holder(&world_art_ref(1, EcMaterialStableRole::Base, true), facts), "not read (block 1)");
    }

    #[test]
    fn the_primary_of_an_item_with_an_art_slot_comes_from_tex_art_ec() {
        let facts = HolderFacts { liquid_payload: None, art_slot_present: true, aux_slot_missing_source: None };
        assert_eq!(holder(&world_art_ref(0, EcMaterialStableRole::Base, true), facts), "tex_art_ec (item slot)");
        let no_slot = HolderFacts { art_slot_present: false, ..facts };
        assert_eq!(holder(&world_art_ref(0, EcMaterialStableRole::Base, true), no_slot), "not packed");
    }

    /// Directory with a `tilemeta.uddp` and its image packages, for [`installed_packages_name_a_holder_for_every_support_ref`].
    const PACKAGE_DIR_ENV: &str = "UDD_PACKAGE_DIR";

    #[test]
    #[ignore = "needs packages: set UDD_PACKAGE_DIR to a folder with tilemeta.uddp and its image packages"]
    fn installed_packages_name_a_holder_for_every_support_ref() {
        let dir = std::path::PathBuf::from(std::env::var_os(PACKAGE_DIR_ENV).expect("set UDD_PACKAGE_DIR"));
        let tilemeta = TileMetaPackage::load(dir.join("tilemeta.uddp")).expect("load tilemeta");
        let holders = ImageHolders::load(&dir.join("tilemeta.uddp"), None);
        assert_eq!(holders.loaded_paths.len(), 3, "all three image packages beside the tilemeta");

        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let mut unpacked_support = Vec::new();
        for item in tilemeta.item_tiles() {
            let (rows, main) = item_texture_refs(&tilemeta, item.tile_id, &holders);
            let has_material = rows.iter().any(|row| row.block_index == TILEMETA_EC_TEXTURE_BLOCK_INDEX && row.primary_selected);
            assert!(!has_material || main.is_some(), "item {} has a material but no main texture", item.tile_id);
            for row in rows {
                let holder = row.image_holder.to_string().split(" (").next().unwrap_or_default().to_string();
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
        assert_eq!(holder(&support, aux(Some(false))), "tex_aux_ec");
        assert_eq!(holder(&support, aux(Some(true))), "tex_aux_ec (missing-texture image)");
        assert_eq!(holder(&support, aux(None)), "not packed");
    }
}
