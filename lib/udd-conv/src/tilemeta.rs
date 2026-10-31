//! Build-time and runtime support for `tilemeta.uddp`.
//!
//! Unlike the atlas packages, this module is mostly metadata plumbing. Its job is to
//! normalize classic `tiledata.mul` and Enhanced Client `tileart.uop` information into
//! two dense runtime tables with stable binary layouts.
//!
//! The high-level contract is:
//! - `TileMetaLandTile` is the dense runtime record for land tile ids.
//! - `TileMetaItemTile` is the dense runtime record for item/static tile ids.
//! - the package stores those tables verbatim so runtime code can bulk-load them
//!   without interpreting the original source formats again.
//!
//! Package layout:
//! - `metadata/land.bin`: dense table for `TileMetaLandTile` entries.
//! - `metadata/items.bin`: dense table for `TileMetaItemTile` entries.
//! - `metadata/item_materials.json`: original per-item shader block parameters.
//! - `metadata/item_texture_refs.v2.bin`: the texture refs; the path names `TILEMETA_FORMAT_VERSION`.
//!
//! Texture ref roles come from the block shader and the item position
//! (`EcMaterialStableRole::for_texture_position`); no rule reads the file name.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use crate::progress::{ProgressBar, ProgressStyle};
use log::info;

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::package_progress::build_and_write_package_with_progress;
use crate::source_paths::find_first_existing_file;
use crate::tex_art_ec::{
    compute_tex_art_ec_crop_adjustments_from_loaded_sources, load_tex_art_ec_sources, TexArtEcLoadedSources,
};
use udd_assets::tex_art_ec::TexArtEcCropAdjustment;
use udd_assets::tilemeta::{
    EcMaterialLogicalFamily, EcMaterialPhysicalPackage,
    EcMaterialStableRole, TileMetaItemTextureRef, TileMetaItemTextureRefSpan, TileMetaItemTile,
    TileMetaItemVisualKind, TileMetaLandTile, TILEMETA_ITEM_ENTRY_PATH,
    TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART, TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY,
    TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED, TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH,
    TILEMETA_LAND_ENTRY_PATH, TILEMETA_ITEM_MATERIAL_ENTRY_PATH,
};
use udd_container::{
    xxh64_virtual_path, AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
    UddpReader,
};
use uocf::classic::tiledata::{
    TileData, TILE_FLAG_ALPHA_BLEND, TILE_FLAG_MULTI_MOVABLE, TILE_FLAG_NO_DRAW, TILE_FLAG_NO_HOUSE,
    TILE_FLAG_NO_SHADOW, TILE_FLAG_PIXEL_BLEED, TILE_FLAG_PLAY_ANIM_ONCE, TILE_FLAG_ROTATED,
};
use uocf::enhanced::tileart::{primary_texture_position, ArtData, TaeFlag, TextureItem};

pub struct TileMetaBuildOptions {
    /// When `true`, use radar colors from `tileart.uop` (EC data).
    /// When `false`, use radar colors from `radarcol.mul` (Classic data).
    pub use_ec_radarcol: bool,
    pub classic_patches: ClassicPatchOptions,
    pub package_compression: CompressionFlag,
}

impl Default for TileMetaBuildOptions {
    fn default() -> Self {
        Self {
            use_ec_radarcol: false,
            classic_patches: ClassicPatchOptions::default(),
            package_compression: CompressionFlag::ZstdNoDict,
        }
    }
}

struct BuiltTileMetaTables {
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    item_texture_ref_spans: Vec<TileMetaItemTextureRefSpan>,
    item_texture_refs: Vec<TileMetaItemTextureRef>,
    item_materials: std::collections::BTreeMap<u32, Vec<uocf::enhanced::tileart::ArtMaterial>>,
    package_compression: CompressionFlag,
}

pub fn classify_item_visual_kind(
    tile_id: u32,
    ec_data: Option<&ArtData>,
) -> TileMetaItemVisualKind {
    if let Some(ec_data) = ec_data {
        if ec_data.tile_type != uocf::enhanced::tileart::TileType::Static {
            return TileMetaItemVisualKind::SurfaceLike;
        }
    }

    let _ = tile_id;
    TileMetaItemVisualKind::RegularArt
}

fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("unpack {path}"))
}

/// `TILEMETA_ITEM_EC_ART_FLAG_*` bits of a tileart record. `UseNewArt` is read from `flags2`, the
/// set the client tests; the same bit in `flags1` (merged into `TileMetaItemTile::flags`) is not it.
fn ec_art_flags(art: &ArtData) -> u16 {
    if art.flags2.contains(TaeFlag::UseNewArt) { TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::tileart::{TextureItem, TextureType};

    #[test]
    fn use_new_art_comes_from_flags2_only() {
        let kr = ArtData { flags2: TaeFlag::UseNewArt, ..ArtData::default() };
        let flags1_only = ArtData { flags: TaeFlag::UseNewArt, ..ArtData::default() };
        assert_eq!(ec_art_flags(&kr), TILEMETA_ITEM_EC_ART_FLAG_USE_NEW_ART);
        assert_eq!(ec_art_flags(&flags1_only), 0);
        let item = TileMetaItemTile { ec_art_flags: ec_art_flags(&kr), ..bytemuck::Zeroable::zeroed() };
        assert!(item.uses_new_art());
        assert!(!<TileMetaItemTile as bytemuck::Zeroable>::zeroed().uses_new_art());
    }

    #[test]
    fn classic_upper_flags_survive_the_tilemeta_mapping() {
        let impassable = TaeFlag::Impassable.bits();
        let upper = TILE_FLAG_NO_HOUSE | TILE_FLAG_PIXEL_BLEED | TILE_FLAG_MULTI_MOVABLE;
        let mapped = map_cc_flags_to_tilemeta(impassable | upper);
        assert_eq!(mapped, (TaeFlag::Impassable | TaeFlag::NoHouse | TaeFlag::PixelBleed | TaeFlag::MultiMovable).bits());
        assert_eq!(map_cc_flags_to_tilemeta(TaeFlag::Unused2.bits()), 0);
    }

    #[test]
    fn source_item_materials_survive_converter_package_roundtrip() {
        let path = std::env::temp_dir().join(format!("tilemeta_materials_{}.uddp", std::process::id()));
        let material = uocf::enhanced::tileart::ArtMaterial {
            block_index: 0, shader_name: Some("UOWaterShader".to_owned()),
            technique: vec![0, 2, 0, 0], tweaks: vec![-0.01, 0.0, 0.25, -0.5], field_04: 7,
        };
        write_tilemeta_uddp(&path, BuiltTileMetaTables {
            land_tiles: Vec::new(), item_tiles: Vec::new(),
            item_texture_ref_spans: Vec::new(), item_texture_refs: Vec::new(),
            item_materials: [(6038, vec![material.clone()])].into_iter().collect(),
            package_compression: CompressionFlag::ZstdNoDict,
        }).expect("write source material metadata");
        let package = udd_assets::tilemeta::TileMetaPackage::load(&path).expect("read source material metadata");
        assert_eq!(package.item_material(6038, 0), Some(&material));
        assert!(package.item_material(6038, 1).is_none());
        assert!(package.item_material(6057, 0).is_none());
        drop(package);
        std::fs::remove_file(path).expect("remove test package");
    }

    #[test]
    fn tileart_textures_refs_are_preserved_as_terraintexture_support() {
        assert_eq!(
            physical_package_for_tileart_texture_type(TextureType::Textures),
            EcMaterialPhysicalPackage::TerrainTexture
        );
    }

    fn water_record(items: &[(u8, u8, u32)]) -> ArtData {
        let mut art = ArtData::default();
        art.materials = vec![uocf::enhanced::tileart::ArtMaterial {
            block_index: 0, shader_name: Some(uocf::enhanced::tileart::WATER_SHADER_NAME.to_owned()), ..Default::default()
        }];
        let item = |&(block_index, item_index, id): &(u8, u8, u32)| TextureItem {
            block_index, item_index, id, texture_type: TextureType::WorldArt, ..TextureItem::default()
        };
        art.texture_items = vec![items.iter().map(item).collect()];
        art
    }

    #[test]
    fn roles_follow_the_block_shader_and_the_position_not_the_name() {
        let mut art = water_record(&[(0, 0, 2000520), (0, 1, 2000051), (0, 2, 2000530), (0, 3, 2000531)]);
        art.texture_items[0][0].path = "Data\\WorldArt\\02000520_surface.tga".to_owned();
        let roles: Vec<_> = art.texture_items[0].iter().map(|item| texture_item_role(&art, item)).collect();
        use EcMaterialStableRole as R;
        assert_eq!(roles, [R::WaterBump, R::Base, R::WaterReflection, R::WaterFourthStage]);
        assert_eq!(block_shader_name(&art, 1), None);
        art.materials[0].shader_name = Some("UOUnknownShader".to_owned());
        assert_eq!(texture_item_role(&art, &art.texture_items[0][1]), R::Unknown);
    }

    #[test]
    fn only_the_selected_block_zero_item_is_primary() {
        // Owner 1364 (stone roof): its base also feeds another map, and block 1 names
        // LegacyTexture image 1364, the legacy art set's counterpart.
        const BASE_ID: u32 = 1364;
        const OTHER_ID: u32 = 1372;
        let item = |block_index, item_index, id, texture_type| TextureItem {
            block_index, item_index, id, texture_type, ..TextureItem::default()
        };
        let mut art = ArtData::default();
        art.texture_items = vec![
            vec![item(0, 0, BASE_ID, TextureType::WorldArt),
                item(0, 1, OTHER_ID, TextureType::WorldArt),
                item(0, 2, BASE_ID, TextureType::WorldArt)],
            vec![item(1, 0, BASE_ID, TextureType::TileArtLegacy)],
        ];
        let primary = primary_ec_texture_item(&art, BASE_ID);
        assert_eq!(primary, Some((0, 0)), "no shader: position 0");
        let flagged: Vec<(u8, u8)> = art.texture_items.iter().flatten()
            .filter(|item| is_primary_selected_texture_item(item, primary, BASE_ID))
            .map(|item| (item.block_index, item.item_index)).collect();
        assert_eq!(flagged, [(0, 0), (1, 0)], "block 0 keeps only its primary item; the legacy counterpart stays primary");
        assert_eq!(primary_ec_texture_item(&art, OTHER_ID), None, "an id away from the primary position is not primary");
        assert_eq!(primary_ec_texture_item(&art, 0), None);

        let water = water_record(&[(0, 0, OTHER_ID), (0, 1, BASE_ID)]);
        assert_eq!(primary_ec_texture_item(&water, BASE_ID), Some((0, 1)), "water reads position 1");
    }
}

pub fn find_string_dictionary_path(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    find_first_existing_file(source_dirs, &["string_dictionary.uop"])
}

pub fn build_tilemeta_uddp(client_dir: &Path, out_file: &Path) -> eyre::Result<()> {
    build_tilemeta_uddp_from_sources(
        &[client_dir.to_path_buf()],
        out_file,
        &TileMetaBuildOptions::default(),
    )
}

pub fn build_tilemeta_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TileMetaBuildOptions,
) -> eyre::Result<()> {
    build_tilemeta_uddp_from_sources_with_progress(source_dirs, out_file, options, |_| {})
}

pub fn build_tilemeta_uddp_from_sources_with_progress(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &TileMetaBuildOptions,
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<()> {
    let built = build_tilemeta_tables_from_sources(source_dirs, options, "unifying tiledata")?;

    write_tilemeta_uddp_with_progress(out_file, built, package_progress)
}

pub fn build_tilemeta_uddp_from_split_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    out_file: &Path,
    options: &TileMetaBuildOptions,
) -> eyre::Result<()> {
    build_tilemeta_uddp_from_split_sources_with_progress(
        cc_source_dir,
        ec_source_dir,
        out_file,
        options,
        |_| {},
    )
}

pub fn build_tilemeta_uddp_from_split_sources_with_progress(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    out_file: &Path,
    options: &TileMetaBuildOptions,
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<()> {
    let built = build_tilemeta_tables_from_split_sources(
        cc_source_dir,
        ec_source_dir,
        options,
        "unifying tiledata",
    )?;

    write_tilemeta_uddp_with_progress(out_file, built, package_progress)
}

/// Builds `tilemeta.uddp` from EC sources the EC art atlas was just packed from, with the trims it recorded
/// ([`crate::tex_art_ec::TexArtEcBuildSummary::ec_crop_adjustments`]), so nothing is decoded twice.
pub fn build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
    cc_source_dir: &Path,
    ec_sources: &TexArtEcLoadedSources,
    ec_crop_adjustments: &[Option<TexArtEcCropAdjustment>],
    out_file: &Path,
    options: &TileMetaBuildOptions,
) -> eyre::Result<()> {
    let built = build_tilemeta_tables_with_ec_sources(
        &[cc_source_dir.to_path_buf()],
        ec_sources,
        ec_crop_adjustments,
        options,
        "unifying tiledata",
    )?;

    write_tilemeta_uddp(out_file, built)
}

fn write_tilemeta_uddp(out_file: &Path, built: BuiltTileMetaTables) -> eyre::Result<()> {
    write_tilemeta_uddp_with_progress(out_file, built, |_| {})
}

fn write_tilemeta_uddp_with_progress(
    out_file: &Path,
    built: BuiltTileMetaTables,
    mut package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<()> {
    let land_bytes = bytemuck::cast_slice(&built.land_tiles);
    let item_bytes = bytemuck::cast_slice(&built.item_tiles);
    let item_texture_ref_span_bytes = bytemuck::cast_slice(&built.item_texture_ref_spans);
    let item_texture_ref_bytes = bytemuck::cast_slice(&built.item_texture_refs);
    let item_material_bytes = serde_json::to_vec(&built.item_materials)?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: built.package_compression,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_MATERIAL_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &item_material_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: built.package_compression,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_LAND_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: land_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: built.package_compression,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: built.package_compression,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_texture_ref_span_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: built.package_compression,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_texture_ref_bytes,
    })?;
    build_and_write_package_with_progress(package, out_file, &mut package_progress)?;

    Ok(())
}

fn build_tilemeta_tables_from_sources(
    source_dirs: &[PathBuf],
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let ec_sources = load_tex_art_ec_sources(source_dirs)?;
    let ec_crop_adjustments = compute_tex_art_ec_crop_adjustments_from_loaded_sources(&ec_sources)?;
    build_tilemeta_tables_with_ec_sources(source_dirs, &ec_sources, &ec_crop_adjustments, options, progress_label)
}

fn build_tilemeta_tables_from_split_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let ec_sources = load_tex_art_ec_sources(&[ec_source_dir.to_path_buf()])?;
    let ec_crop_adjustments = compute_tex_art_ec_crop_adjustments_from_loaded_sources(&ec_sources)?;
    build_tilemeta_tables_with_ec_sources(
        &[cc_source_dir.to_path_buf()],
        &ec_sources,
        &ec_crop_adjustments,
        options,
        progress_label,
    )
}

/// Builds the tables from `tiledata.mul` (and `radarcol.mul`) in `cc_source_dirs` and the loaded EC sources. The
/// EC sampling starts are shifted by `ec_crop_adjustments`, the trims the EC art atlas applied to the same sources
/// ([`adjusted_ec_sampling_start`]).
fn build_tilemeta_tables_with_ec_sources(
    cc_source_dirs: &[PathBuf],
    ec_sources: &TexArtEcLoadedSources,
    ec_crop_adjustments: &[Option<TexArtEcCropAdjustment>],
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let tiledata_path = find_first_existing_file(cc_source_dirs, &["tiledata.mul"])
        .ok_or_else(|| eyre::eyre!("missing tiledata.mul"))?;
    let radarcol_path = find_first_existing_file(cc_source_dirs, &["radarcol.mul"]);
    info!("Using tiledata.mul: {}", tiledata_path.display());
    info!("Using tileart.uop: {}", ec_sources.tileart_path.display());
    info!("Using string dictionary: {}", ec_sources.stringdict_path.display());
    if let Some(ref p) = radarcol_path {
        info!("Using radarcol.mul: {}", p.display());
    }
    println!("Using CC tiledata source file: {}", tiledata_path.display());
    if let Some(ref p) = radarcol_path {
        println!("Using CC radar color source file: {}", p.display());
    }

    info!("Converting Tile Metadata tables from MUL/UOP sources");

    let cc_tiledata = TileData::load_with_verdata(
        tiledata_path.clone(),
        load_verdata_if_enabled(cc_source_dirs, &options.classic_patches)?,
    )?;
    let tex_art_ec = &ec_sources.art_definition;

    let cc_radarcol = if !options.use_ec_radarcol || radarcol_path.is_some() {
        if let Some(p) = radarcol_path {
            uocf::classic::radarcol::load_radarcol(&p).ok()
        } else {
            None
        }
    } else {
        None
    };

    let get_radar_color = |id: u32,
                           is_item: bool,
                           ec_radar: Option<&uocf::enhanced::tileart::TaeRadarcol>|
     -> [u8; 4] {
        if options.use_ec_radarcol {
            if let Some(ec) = ec_radar {
                return [ec.r, ec.g, ec.b, ec.a];
            }
        }

        // Fallback to radarcol.mul
        if let Some(ref colors) = cc_radarcol {
            let index = if is_item { id + 0x4000 } else { id } as usize;
            if index < colors.len() {
                let (r, g, b, a) = colors[index].as_rgba8888().components();
                return [r, g, b, a];
            }
        }

        [0, 0, 0, 0]
    };

    let mut tilemeta_land = Vec::with_capacity(cc_tiledata.land_tiles().len());
    let pb = tilemeta_progress_bar(
        (cc_tiledata.land_tiles().len() + cc_tiledata.item_tiles().len()) as u64,
        progress_label,
    );

    for tile in cc_tiledata.land_tiles() {
        pb.inc(1);
        // Land tiles in EC are not mapped via tileart.uop directly in the same way,
        // so for now we map their baseline properties.
        tilemeta_land.push(TileMetaLandTile {
            tile_id: tile.tile_id as u32,
            texture_id: tile.texture_id,
            tile_type: 0,
            _pad1: 0,
            flags: map_cc_flags_to_tilemeta(tile.flags.internal_flags),
            radar_color: get_radar_color(
                tile.tile_id as u32,
                false,
                tex_art_ec
                    .definitions
                    .get(&(tile.tile_id as u16))
                    .map(|d| &d.radar_color),
            ),
            name: tile.name,
        });
    }

    let mut tilemeta_items = Vec::with_capacity(cc_tiledata.item_tiles().len());
    let mut item_texture_ref_spans = Vec::with_capacity(cc_tiledata.item_tiles().len());
    let mut item_texture_refs = Vec::new();
    let mut unknown_shader_refs = std::collections::BTreeMap::<String, usize>::new();
    let mut item_materials = std::collections::BTreeMap::new();
    for tile in cc_tiledata.item_tiles() {
        pb.inc(1);
        let mut tile_meta_item = TileMetaItemTile {
            tile_id: tile.tile_id as u32,
            weight: tile.weight,
            quality: tile.quality,
            quantity: tile.quantity,
            hue_extra: tile.hue_extra,
            flags: map_cc_flags_to_tilemeta(tile.flags.internal_flags),
            anim_id: tile.anim_id,
            stacking_offset: tile.stacking_offset,
            value: tile.value,
            height: tile.height_raw(),
            _pad1: 0,
            ec_art_flags: 0,
            radar_color: [0, 0, 0, 0],
            name: tile.name,
            ec_texture_id: 0,
            ec_start_x: 0,
            ec_start_y: 0,
            cc_texture_id: tile.tile_id as u32,
            cc_start_x: 0,
            cc_start_y: 0,
        };

        let ref_start = u32::try_from(item_texture_refs.len())
            .map_err(|_| eyre::eyre!("tilemeta texture-ref table exceeds u32 address space"))?;

        if let Some(ec_data) = tex_art_ec.definitions.get(&(tile.tile_id as u16)) {
            if !ec_data.materials.is_empty() {
                item_materials.insert(tile.tile_id as u32, ec_data.materials.clone());
            }
            tile_meta_item.flags |= ec_data.flags.bits();
            tile_meta_item.ec_art_flags = ec_art_flags(ec_data);
            tile_meta_item.set_visual_kind(classify_item_visual_kind(
                tile.tile_id as u32,
                Some(ec_data),
            ));

            // Unify radar color
            tile_meta_item.radar_color =
                get_radar_color(tile.tile_id as u32, true, Some(&ec_data.radar_color));

            // Unify EC Texture
            if let Some(ec_tex) = &ec_data.ec_texture {
                tile_meta_item.ec_texture_id = ec_tex.texture_id;
                let crop_adjustment = ec_crop_adjustments.get(tile.tile_id as usize).copied().flatten();
                (tile_meta_item.ec_start_x, tile_meta_item.ec_start_y) = adjusted_ec_sampling_start(
                    tile.tile_id as u32,
                    ec_tex.start_x,
                    ec_tex.start_y,
                    crop_adjustment,
                )?;
            }

            // Unify CC Texture override
            if let Some(cc_tex) = &ec_data.cc_texture {
                tile_meta_item.cc_texture_id = cc_tex.texture_id;
                tile_meta_item.cc_start_x = cc_tex.start_x as i16;
                tile_meta_item.cc_start_y = cc_tex.start_y as i16;
            }

            let primary_item = primary_ec_texture_item(ec_data, tile_meta_item.ec_texture_id);
            item_texture_refs.extend(ec_data.texture_items.iter().flatten().map(|item| {
                let shader_name = block_shader_name(ec_data, item.block_index);
                let stable_role = EcMaterialStableRole::for_texture_position(item.block_index, shader_name, item.item_index);
                if stable_role == EcMaterialStableRole::Unknown {
                    *unknown_shader_refs.entry(shader_name.unwrap_or_default().to_owned()).or_default() += 1;
                }
                let mut flags = 0u8;
                if item.item_index != primary_texture_position(shader_name) {
                    flags |= TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY;
                }
                if is_primary_selected_texture_item(item, primary_item, tile_meta_item.ec_texture_id) {
                    flags |= TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED;
                }

                TileMetaItemTextureRef {
                    texture_id: item.id,
                    // Use ec_logical_family (path-inferred) to populate the logical_family
                    // field; it resolves Effects/ShaderResources that the old TextureType
                    // enum left as Undefined.
                    texture_type: ec_logical_family_as_material_u8(item.ec_logical_family),
                    block_index: item.block_index,
                    item_index: item.item_index,
                    flags,
                    texture_stretch: item.texture_stretch,
                    source_kind: item.source_kind,
                    physical_package: physical_package_for_tileart_texture_type(item.texture_type)
                        as u8,
                    stable_role: stable_role as u8,
                    _reserved: 0,
                    sampler_index: item.sampler_index,
                    uv_set: item.uv_set,
                    // TODO Phase 5: populate raw_path and normalized_path sidecar fields
                    // when TileMetaItemTextureRef is extended for variable-length data.
                }
            }));
        } else {
            tile_meta_item.set_visual_kind(classify_item_visual_kind(tile.tile_id as u32, None));
        }

        let ref_len =
            u16::try_from(item_texture_refs.len() - ref_start as usize).map_err(|_| {
                eyre::eyre!(
                    "tile {} references too many EC textures for u16 span",
                    tile.tile_id
                )
            })?;
        item_texture_ref_spans.push(TileMetaItemTextureRefSpan {
            start: ref_start,
            len: ref_len,
            _pad: 0,
        });
        tilemeta_items.push(tile_meta_item);
    }
    pb.finish_with_message("Tile Metadata unified");
    for (shader_name, refs) in &unknown_shader_refs {
        log::warn!("tilemeta: {refs} texture refs in blocks with unknown shader {shader_name:?} get the Unknown role");
    }

    Ok(BuiltTileMetaTables {
        land_tiles: tilemeta_land,
        item_tiles: tilemeta_items,
        item_texture_ref_spans,
        item_texture_refs,
        item_materials,
        package_compression: options.package_compression,
    })
}

/// Maps `EcLogicalFamily` (from `uocf::ec_material_axes`) to the numeric discriminant
/// used by `EcMaterialLogicalFamily` (from `udd_assets::tilemeta`). Both enums share the
/// same variant ordering; this function makes the conversion explicit and safe.
fn ec_logical_family_as_material_u8(family: uocf::ec_material_axes::EcLogicalFamily) -> u8 {
    use uocf::ec_material_axes::EcLogicalFamily;
    match family {
        EcLogicalFamily::Unknown => EcMaterialLogicalFamily::Unknown as u8,
        EcLogicalFamily::WorldArt => EcMaterialLogicalFamily::WorldArt as u8,
        EcLogicalFamily::TileArtLegacy => EcMaterialLogicalFamily::TileArtLegacy as u8,
        EcLogicalFamily::TileArtEnhanced => EcMaterialLogicalFamily::TileArtEnhanced as u8,
        EcLogicalFamily::Textures => EcMaterialLogicalFamily::Textures as u8,
        EcLogicalFamily::Effects => EcMaterialLogicalFamily::Effects as u8,
        EcLogicalFamily::SystemTextures => EcMaterialLogicalFamily::SystemTextures as u8,
        EcLogicalFamily::ShaderResources => EcMaterialLogicalFamily::ShaderResources as u8,
    }
}

/// Texture block the EC primary image is selected from (`ArtData::ec_texture`, the first tileart block).
const EC_PRIMARY_TEXTURE_BLOCK_INDEX: u8 = 0;

/// Shader name of block `block_index` of a record, if the block is present and named.
pub fn block_shader_name(art: &ArtData, block_index: u8) -> Option<&str> {
    art.materials.iter().find(|material| material.block_index == block_index)?.shader_name.as_deref()
}

/// Role of a tileart texture item, from its block shader and position.
pub fn texture_item_role(art: &ArtData, item: &TextureItem) -> EcMaterialStableRole {
    EcMaterialStableRole::for_texture_position(item.block_index, block_shader_name(art, item.block_index), item.item_index)
}

/// Whether a texture ref gets `PRIMARY_SELECTED`.
///
/// In the EC block only the item `ec_texture_id` was taken from is primary: the item at the block's
/// primary position. Another item of that block naming the same number reuses the image on another
/// shader map (a terrain mask, say), and `tex_aux_ec` must keep it. In the other blocks (the
/// `TileArtLegacy`/`TileArtEnhanced` art sets) an item with the same number is that set's
/// counterpart of the primary image and stays primary, as the main-texture selectors expect.
fn is_primary_selected_texture_item(item: &TextureItem, primary_item: Option<(u8, u8)>, ec_texture_id: u32) -> bool {
    if item.block_index == EC_PRIMARY_TEXTURE_BLOCK_INDEX {
        primary_item == Some((item.block_index, item.item_index))
    } else {
        ec_texture_id != 0 && item.id == ec_texture_id
    }
}

/// Returns `(block_index, item_index)` of the item `ec_texture_id` was taken from: the item at the
/// primary position of block 0 (`uocf::enhanced::tileart::primary_texture_position`), when it names
/// that id.
fn primary_ec_texture_item(ec_data: &ArtData, ec_texture_id: u32) -> Option<(u8, u8)> {
    if ec_texture_id == 0 {
        return None;
    }
    let position = primary_texture_position(block_shader_name(ec_data, EC_PRIMARY_TEXTURE_BLOCK_INDEX));
    ec_data.texture_items.iter().flatten()
        .find(|item| item.block_index == EC_PRIMARY_TEXTURE_BLOCK_INDEX && item.item_index == position && item.id == ec_texture_id)
        .map(|item| (item.block_index, item.item_index))
}

fn physical_package_for_tileart_texture_type(
    texture_type: uocf::enhanced::tileart::TextureType,
) -> EcMaterialPhysicalPackage {
    match texture_type {
        uocf::enhanced::tileart::TextureType::WorldArt => EcMaterialPhysicalPackage::Texture,
        uocf::enhanced::tileart::TextureType::TileArtLegacy => {
            EcMaterialPhysicalPackage::LegacyTexture
        }
        uocf::enhanced::tileart::TextureType::Textures => EcMaterialPhysicalPackage::TerrainTexture,
        uocf::enhanced::tileart::TextureType::TileArtEnhanced
        | uocf::enhanced::tileart::TextureType::Undefined => EcMaterialPhysicalPackage::Unknown,
    }
}

fn tilemeta_progress_bar(total: u64, progress_label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "{{spinner:.green}} [{{elapsed_precise}}] [{{bar:40.cyan/blue}}] {{pos}}/{{len}} {progress_label} ({{eta}})"
            ))
            .unwrap()
            .progress_chars("#>-"),
    );
    pb
}

/// The EC sampling start `tilemeta.uddp` stores for a tile: where its packed, alpha-trimmed image starts in the
/// source texture, i.e. the tileart start shifted by the top/left trim ([`TexArtEcCropAdjustment::source_left`] and
/// `source_top`, the window start clamped to the texture plus the trim). Consumers that crop the source texture at
/// this start with the packed slot's extent (the EC sprite hue masks) get the pixels the slot holds. A tile the
/// atlas did not pack from its EC texture keeps its tileart start.
pub fn adjusted_ec_sampling_start(
    tile_id: u32,
    start_x: i32,
    start_y: i32,
    adjustment: Option<TexArtEcCropAdjustment>,
) -> eyre::Result<(i16, i16)> {
    if let Some(adjustment) = adjustment {
        return Ok((adjustment.source_left, adjustment.source_top));
    }
    Ok((
        i16::try_from(start_x)
            .map_err(|_| eyre::eyre!("tile {tile_id} EC start_x {start_x} does not fit in i16"))?,
        i16::try_from(start_y)
            .map_err(|_| eyre::eyre!("tile {tile_id} EC start_y {start_y} does not fit in i16"))?,
    ))
}

/// Explicitly translates Classic Client flags into the Enhanced Client 64-bit flag space.
/// While historically many of the lower 32-bits share the same integer values across clients,
/// explicit mapping ensures no assumptions are made and allows divergent flags to be mapped
/// correctly into a single dictionary. Files from High Seas on store 64 bits, whose known upper bits
/// are carried over too ([`CC_UPPER_FLAGS_TO_TILEMETA`]).
fn map_cc_flags_to_tilemeta(cc: u64) -> u64 {
    let mut ec = 0u64;

    if (cc & 0x00000001) != 0 {
        ec |= 0x1;
    } // Background
    if (cc & 0x00000002) != 0 {
        ec |= 0x2;
    } // Weapon
    if (cc & 0x00000004) != 0 {
        ec |= 0x4;
    } // Transparent
    if (cc & 0x00000008) != 0 {
        ec |= 0x8;
    } // Translucent
    if (cc & 0x00000010) != 0 {
        ec |= 0x10;
    } // Wall
    if (cc & 0x00000020) != 0 {
        ec |= 0x20;
    } // Damaging
    if (cc & 0x00000040) != 0 {
        ec |= 0x40;
    } // Impassable
    if (cc & 0x00000080) != 0 {
        ec |= 0x80;
    } // Wet
    if (cc & 0x00000100) != 0 {
        ec |= 0x100;
    } // CC: Unknown -> EC: Ignored
    if (cc & 0x00000200) != 0 {
        ec |= 0x200;
    } // Surface
    if (cc & 0x00000400) != 0 {
        ec |= 0x400;
    } // Bridge
    if (cc & 0x00000800) != 0 {
        ec |= 0x800;
    } // Generic / Stackable
    if (cc & 0x00001000) != 0 {
        ec |= 0x1000;
    } // Window
    if (cc & 0x00002000) != 0 {
        ec |= 0x2000;
    } // NoShoot
    if (cc & 0x00004000) != 0 {
        ec |= 0x4000;
    } // ArticleA
    if (cc & 0x00008000) != 0 {
        ec |= 0x8000;
    } // ArticleAn
    if (cc & 0x00010000) != 0 {
        ec |= 0x10000;
    } // Internal / Mongen
    if (cc & 0x00020000) != 0 {
        ec |= 0x20000;
    } // Foliage
    if (cc & 0x00040000) != 0 {
        ec |= 0x40000;
    } // PartialHue
    if (cc & 0x00080000) != 0 {
        ec |= 0x80000;
    } // CC: Unknown1 -> EC: UseNewArt
    if (cc & 0x00100000) != 0 {
        ec |= 0x100000;
    } // Map
    if (cc & 0x00200000) != 0 {
        ec |= 0x200000;
    } // Container
    if (cc & 0x00400000) != 0 {
        ec |= 0x400000;
    } // Wearable
    if (cc & 0x00800000) != 0 {
        ec |= 0x800000;
    } // LightSource
    if (cc & 0x01000000) != 0 {
        ec |= 0x1000000;
    } // Animated
    if (cc & 0x02000000) != 0 {
        ec |= 0x2000000;
    } // NoDiagonal / HoverOver
    if (cc & 0x04000000) != 0 {
        ec |= 0x4000000;
    } // CC: Unknown2 -> EC: ArtUsed
    if (cc & 0x08000000) != 0 {
        ec |= 0x8000000;
    } // Armor
    if (cc & 0x10000000) != 0 {
        ec |= 0x10000000;
    } // Roof
    if (cc & 0x20000000) != 0 {
        ec |= 0x20000000;
    } // Door
    if (cc & 0x40000000) != 0 {
        ec |= 0x40000000;
    } // StairBack
    if (cc & 0x80000000) != 0 {
        ec |= 0x80000000;
    } // StairRight

    for (cc_flag, ec_flag) in CC_UPPER_FLAGS_TO_TILEMETA {
        if (cc & cc_flag) != 0 {
            ec |= ec_flag.bits();
        }
    }

    ec
}

/// The upper 32 bits of the 64-bit Classic flags that tilemeta keeps, with their Enhanced Client flag. They have
/// the EC tileart layout bit for bit; bit 38 (`TaeFlag::Unused2`) has no known meaning and is dropped.
const CC_UPPER_FLAGS_TO_TILEMETA: [(u64, TaeFlag); 8] = [
    (TILE_FLAG_NO_HOUSE, TaeFlag::NoHouse),
    (TILE_FLAG_NO_DRAW, TaeFlag::NoDraw),
    (TILE_FLAG_ROTATED, TaeFlag::Rotated),
    (TILE_FLAG_ALPHA_BLEND, TaeFlag::AlphaBlend),
    (TILE_FLAG_NO_SHADOW, TaeFlag::NoShadow),
    (TILE_FLAG_PIXEL_BLEED, TaeFlag::PixelBleed),
    (TILE_FLAG_PLAY_ANIM_ONCE, TaeFlag::PlayAnimOnce),
    (TILE_FLAG_MULTI_MOVABLE, TaeFlag::MultiMovable),
];
