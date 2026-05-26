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

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use crate::tex_art_ec::{
    compute_tex_art_ec_crop_adjustments_from_loaded_sources,
    compute_tex_art_ec_crop_adjustments_from_sources,
    TexArtEcLoadedSources,
};
use udd_assets::tex_art_ec::TexArtEcCropAdjustment;
use udd_assets::tilemeta::{
    EcMaterialPhysicalPackage, EcMaterialSpeculativeRole, EcMaterialStableRole,
    TileMetaItemTextureRef, TileMetaItemTextureRefSpan, TileMetaItemTile, TileMetaItemVisualKind,
    TileMetaLandTile, TILEMETA_ITEM_ENTRY_PATH, TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY,
    TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED, TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH,
    TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, TILEMETA_LAND_ENTRY_PATH,
};
use udd_container::{
    xxh64_virtual_path, AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
    UddpReader,
};
use uocf::classic::tiledata::TileData;
use uocf::enhanced::{tile_database::ArtDefinition, tileart::ArtData};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileMetaBuildOptions {
    /// When `true`, subtract the EC-art crop delta from the stored EC sampling
    /// start coordinates so they remain aligned with the shared EC texture pass.
    pub adjust_tex_art_ec_sampling: bool,
    /// When `true`, use radar colors from `tileart.uop` (EC data).
    /// When `false`, use radar colors from `radarcol.mul` (Classic data).
    pub use_ec_radarcol: bool,
    pub classic_patches: ClassicPatchOptions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileMetaBuildSummary {
    pub adjusted_ec_item_count: u32,
}

struct BuiltTileMetaTables {
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    item_texture_ref_spans: Vec<TileMetaItemTextureRefSpan>,
    item_texture_refs: Vec<TileMetaItemTextureRef>,
    summary: TileMetaBuildSummary,
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

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::tileart::{TextureItem, TextureType};

    #[test]
    fn tileart_textures_refs_are_preserved_as_terraintexture_support() {
        let item = TextureItem {
            texture_type: TextureType::Textures,
            ..TextureItem::default()
        };

        assert_eq!(
            physical_package_for_tileart_texture_type(item.texture_type),
            EcMaterialPhysicalPackage::TerrainTexture
        );
        assert_eq!(
            stable_role_for_tileart_texture_ref(&item, 0),
            EcMaterialStableRole::ImageSupport
        );
    }

    #[test]
    fn primary_selected_tileart_ref_gets_base_role_without_changing_policy() {
        let item = TextureItem {
            texture_type: TextureType::WorldArt,
            ..TextureItem::default()
        };

        assert_eq!(
            stable_role_for_tileart_texture_ref(&item, TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED),
            EcMaterialStableRole::Base
        );
    }

    #[test]
    fn primary_selected_alpha_mask_keeps_support_role() {
        let item = TextureItem {
            texture_type: TextureType::WorldArt,
            path: "Data\\WorldArt\\02000042_alpha.dds".to_string(),
            ..TextureItem::default()
        };

        assert_eq!(
            stable_role_for_tileart_texture_ref(&item, TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED),
            EcMaterialStableRole::AlphaMask
        );
    }

    #[test]
    fn primary_selected_normal_map_keeps_support_role() {
        let item = TextureItem {
            texture_type: TextureType::Textures,
            path: "Data\\Textures\\water_normal.dds".to_string(),
            ..TextureItem::default()
        };

        assert_eq!(
            stable_role_for_tileart_texture_ref(&item, TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED),
            EcMaterialStableRole::NormalLike
        );
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
    let built = build_tilemeta_tables_from_sources(source_dirs, options, "unifying tiledata")?;

    write_tilemeta_uddp(out_file, built)
}

pub fn build_tilemeta_uddp_from_split_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    out_file: &Path,
    options: &TileMetaBuildOptions,
) -> eyre::Result<()> {
    let built = build_tilemeta_tables_from_split_sources(
        cc_source_dir,
        ec_source_dir,
        options,
        "unifying tiledata",
    )?;

    write_tilemeta_uddp(out_file, built)
}

pub fn build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    ec_sources: &TexArtEcLoadedSources,
    out_file: &Path,
    options: &TileMetaBuildOptions,
) -> eyre::Result<()> {
    let built = build_tilemeta_tables_from_split_sources_with_loaded_ec_sources(
        cc_source_dir,
        ec_source_dir,
        ec_sources,
        options,
        "unifying tiledata",
    )?;

    write_tilemeta_uddp(out_file, built)
}

fn write_tilemeta_uddp(out_file: &Path, built: BuiltTileMetaTables) -> eyre::Result<()> {
    let land_bytes = bytemuck::cast_slice(&built.land_tiles);
    let item_bytes = bytemuck::cast_slice(&built.item_tiles);
    let item_texture_ref_span_bytes = bytemuck::cast_slice(&built.item_texture_ref_spans);
    let item_texture_ref_bytes = bytemuck::cast_slice(&built.item_texture_refs);

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_LAND_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: land_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_texture_ref_span_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_texture_ref_bytes,
    })?;
    build_and_write_package(&mut package, out_file)?;

    Ok(())
}

pub fn build_tilemeta_item_payload_from_sources(
    source_dirs: &[PathBuf],
    options: &TileMetaBuildOptions,
) -> eyre::Result<(Vec<u8>, TileMetaBuildSummary)> {
    let built = build_tilemeta_tables_from_sources(source_dirs, options, "updating tilemeta")?;
    Ok((
        bytemuck::cast_slice(&built.item_tiles).to_vec(),
        built.summary,
    ))
}

fn build_tilemeta_tables_from_sources(
    source_dirs: &[PathBuf],
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let tiledata_path = find_first_existing_file(source_dirs, &["tiledata.mul"])
        .ok_or_else(|| eyre::eyre!("missing tiledata.mul"))?;
    let tileart_path = find_first_existing_file(source_dirs, &["tileart.uop"])
        .ok_or_else(|| eyre::eyre!("missing tileart.uop"))?;
    let stringdict_path = find_string_dictionary_path(source_dirs)
        .ok_or_else(|| eyre::eyre!("missing string_dictionary.uop"))?;
    let radarcol_path = find_first_existing_file(source_dirs, &["radarcol.mul"]);

    build_tilemeta_tables_from_resolved_paths(
        tiledata_path,
        tileart_path,
        stringdict_path,
        radarcol_path,
        source_dirs,
        source_dirs,
        None,
        options,
        progress_label,
    )
}

fn build_tilemeta_tables_from_split_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let cc_source_dirs = [cc_source_dir.to_path_buf()];
    let ec_source_dirs = [ec_source_dir.to_path_buf()];
    let tiledata_path = find_first_existing_file(&cc_source_dirs, &["tiledata.mul"])
        .ok_or_else(|| eyre::eyre!("missing tiledata.mul"))?;
    let tileart_path = find_first_existing_file(&ec_source_dirs, &["tileart.uop"])
        .ok_or_else(|| eyre::eyre!("missing tileart.uop"))?;
    let stringdict_path = find_string_dictionary_path(&ec_source_dirs)
        .ok_or_else(|| eyre::eyre!("missing string_dictionary.uop"))?;
    let radarcol_path = find_first_existing_file(&cc_source_dirs, &["radarcol.mul"]);

    build_tilemeta_tables_from_resolved_paths(
        tiledata_path,
        tileart_path,
        stringdict_path,
        radarcol_path,
        &cc_source_dirs,
        &ec_source_dirs,
        None,
        options,
        progress_label,
    )
}

fn build_tilemeta_tables_from_split_sources_with_loaded_ec_sources(
    cc_source_dir: &Path,
    ec_source_dir: &Path,
    ec_sources: &TexArtEcLoadedSources,
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    let cc_source_dirs = [cc_source_dir.to_path_buf()];
    let ec_source_dirs = [ec_source_dir.to_path_buf()];
    let tiledata_path = find_first_existing_file(&cc_source_dirs, &["tiledata.mul"])
        .ok_or_else(|| eyre::eyre!("missing tiledata.mul"))?;
    let radarcol_path = find_first_existing_file(&cc_source_dirs, &["radarcol.mul"]);

    build_tilemeta_tables_from_resolved_paths(
        tiledata_path,
        ec_sources.tileart_path.clone(),
        ec_sources.stringdict_path.clone(),
        radarcol_path,
        &cc_source_dirs,
        &ec_source_dirs,
        Some(ec_sources),
        options,
        progress_label,
    )
}

fn build_tilemeta_tables_from_resolved_paths(
    tiledata_path: PathBuf,
    tileart_path: PathBuf,
    stringdict_path: PathBuf,
    radarcol_path: Option<PathBuf>,
    cc_source_dirs: &[PathBuf],
    ec_source_dirs: &[PathBuf],
    loaded_ec_sources: Option<&TexArtEcLoadedSources>,
    options: &TileMetaBuildOptions,
    progress_label: &str,
) -> eyre::Result<BuiltTileMetaTables> {
    info!("Using tiledata.mul: {}", tiledata_path.display());
    info!("Using tileart.uop: {}", tileart_path.display());
    info!("Using string dictionary: {}", stringdict_path.display());
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
    let loaded_art_definition;
    let tex_art_ec = if let Some(sources) = loaded_ec_sources {
        &sources.art_definition
    } else {
        loaded_art_definition = ArtDefinition::load(&tileart_path, &stringdict_path)?;
        &loaded_art_definition
    };
    let tex_art_ec_crop_adjustments = if options.adjust_tex_art_ec_sampling {
        if let Some(sources) = loaded_ec_sources {
            compute_tex_art_ec_crop_adjustments_from_loaded_sources(sources)?
        } else {
            compute_tex_art_ec_crop_adjustments_from_sources(ec_source_dirs)?
        }
    } else {
        Vec::new()
    };

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
    let mut adjusted_ec_item_count = 0u32;
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
            _pad2: 0,
            radar_color: [0, 0, 0, 0],
            name: tile.name,
            ec_texture_id: 0,
            ec_start_x: 0,
            ec_start_y: 0,
            ec_offset_x: 0,
            ec_offset_y: 0,
            cc_texture_id: tile.tile_id as u32,
            cc_start_x: 0,
            cc_start_y: 0,
            cc_offset_x: 0,
            cc_offset_y: 0,
        };

        let ref_start = u32::try_from(item_texture_refs.len())
            .map_err(|_| eyre::eyre!("tilemeta texture-ref table exceeds u32 address space"))?;

        if let Some(ec_data) = tex_art_ec.definitions.get(&(tile.tile_id as u16)) {
            tile_meta_item.flags |= ec_data.flags.bits();
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
                let crop_adjustment = tex_art_ec_crop_adjustments
                    .get(tile.tile_id as usize)
                    .and_then(|adjustment| *adjustment);
                let (ec_start_x, ec_start_y) = adjusted_ec_sampling_start(
                    tile.tile_id as u32,
                    ec_tex.start_x,
                    ec_tex.start_y,
                    crop_adjustment,
                )?;
                tile_meta_item.ec_start_x = ec_start_x;
                tile_meta_item.ec_start_y = ec_start_y;
                tile_meta_item.ec_offset_x = ec_tex.offset_x as i16;
                tile_meta_item.ec_offset_y = ec_tex.offset_y as i16;
                if ec_start_x != ec_tex.start_x as i16 || ec_start_y != ec_tex.start_y as i16 {
                    adjusted_ec_item_count += 1;
                }
            }

            // Unify CC Texture override
            if let Some(cc_tex) = &ec_data.cc_texture {
                tile_meta_item.cc_texture_id = cc_tex.texture_id;
                tile_meta_item.cc_start_x = cc_tex.start_x as i16;
                tile_meta_item.cc_start_y = cc_tex.start_y as i16;
                tile_meta_item.cc_offset_x = cc_tex.offset_x as i16;
                tile_meta_item.cc_offset_y = cc_tex.offset_y as i16;
            }

            item_texture_refs.extend(ec_data.texture_items.iter().flatten().map(|item| {
                let mut flags = 0u8;
                if item.is_auxiliary {
                    flags |= TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY;
                }
                if tile_meta_item.ec_texture_id != 0 && item.id == tile_meta_item.ec_texture_id {
                    flags |= TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED;
                }

                TileMetaItemTextureRef {
                    texture_id: item.id,
                    texture_type: item.texture_type as u8,
                    block_index: item.block_index,
                    item_index: item.item_index,
                    flags,
                    texture_stretch: item.texture_stretch,
                    unk4: item.unk4,
                    physical_package: physical_package_for_tileart_texture_type(item.texture_type)
                        as u8,
                    stable_role: stable_role_for_tileart_texture_ref(item, flags) as u8,
                    speculative_role: EcMaterialSpeculativeRole::UnknownSupport as u8,
                    unk6: item.unk6,
                    unk7: item.unk7,
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

    Ok(BuiltTileMetaTables {
        land_tiles: tilemeta_land,
        item_tiles: tilemeta_items,
        item_texture_ref_spans,
        item_texture_refs,
        summary: TileMetaBuildSummary {
            adjusted_ec_item_count,
        },
    })
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

fn stable_role_for_tileart_texture_ref(
    item: &uocf::enhanced::tileart::TextureItem,
    flags: u8,
) -> EcMaterialStableRole {
    let path_role = stable_role_from_tileart_texture_path(&item.path);
    if path_role != EcMaterialStableRole::UnknownSupport {
        path_role
    } else if flags & TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED != 0 {
        EcMaterialStableRole::Base
    } else if item.is_auxiliary
        || item.texture_type == uocf::enhanced::tileart::TextureType::Textures
    {
        EcMaterialStableRole::ImageSupport
    } else {
        EcMaterialStableRole::UnknownSupport
    }
}

fn stable_role_from_tileart_texture_path(path: &str) -> EcMaterialStableRole {
    let lower = path.to_ascii_lowercase();
    let basename = lower.rsplit(['\\', '/']).next().unwrap_or(lower.as_str());
    let stem = basename.split('.').next().unwrap_or(basename);

    if stem.contains("normal") || stem.contains("_n") {
        EcMaterialStableRole::NormalLike
    } else if stem.contains("alpha") {
        EcMaterialStableRole::AlphaMask
    } else if stem.contains("mask") {
        EcMaterialStableRole::GenericMask
    } else if stem.contains("noise") {
        EcMaterialStableRole::Noise
    } else if stem.contains("detail") {
        EcMaterialStableRole::Detail
    } else if stem.contains("light") || stem.contains("glow") {
        EcMaterialStableRole::Overlay
    } else {
        EcMaterialStableRole::UnknownSupport
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

pub fn adjusted_ec_sampling_start(
    texture_id: u32,
    start_x: i32,
    start_y: i32,
    adjustment: Option<TexArtEcCropAdjustment>,
) -> eyre::Result<(i16, i16)> {
    let adjusted_x = start_x - adjustment.map_or(0, |adjustment| i32::from(adjustment.left));
    let adjusted_y = start_y - adjustment.map_or(0, |adjustment| i32::from(adjustment.top));
    Ok((
        i16::try_from(adjusted_x).map_err(|_| {
            eyre::eyre!("tile {texture_id} adjusted EC start_x {adjusted_x} does not fit in i16")
        })?,
        i16::try_from(adjusted_y).map_err(|_| {
            eyre::eyre!("tile {texture_id} adjusted EC start_y {adjusted_y} does not fit in i16")
        })?,
    ))
}

/// Explicitly translates Classic Client 32-bit flags into the Enhanced Client 64-bit flag space.
/// While historically many of the lower 32-bits share the same integer values across clients,
/// explicit mapping ensures no assumptions are made and allows divergent flags to be mapped
/// correctly into a single dictionary.
fn map_cc_flags_to_tilemeta(cc: u32) -> u64 {
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

    ec
}
