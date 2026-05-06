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

use bytemuck::{Pod, Zeroable};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;

use crate::ec_art::{compute_ec_art_crop_adjustments_from_sources, EcArtCropAdjustment};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use uocf::classic::tiledata::TileData;
use uocf::enhanced::{tile_database::ArtDefinition, tileart::ArtData};
use uocf::udd::{
    xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
    LookupMode, UddpBuilder, UddpReader,
};

pub const TILEMETA_LAND_ENTRY_PATH: &str = "metadata/land.bin";
pub const TILEMETA_ITEM_ENTRY_PATH: &str = "metadata/items.bin";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileMetaBuildOptions {
    /// When `true`, subtract the EC-art crop delta from the stored EC sampling
    /// start coordinates so they remain aligned with the shared EC texture pass.
    pub adjust_ec_art_sampling: bool,
    /// When `true`, use radar colors from `tileart.uop` (EC data).
    /// When `false`, use radar colors from `radarcol.mul` (Classic data).
    pub use_ec_radarcol: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileMetaBuildSummary {
    pub adjusted_ec_item_count: u32,
}

struct BuiltTileMetaTables {
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    summary: TileMetaBuildSummary,
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
    pub ec_offset_x: i16,
    pub ec_offset_y: i16,

    pub cc_texture_id: u32,
    pub cc_start_x: i16,
    pub cc_start_y: i16,
    pub cc_offset_x: i16,
    pub cc_offset_y: i16,
} // 48 + 4+2+2+2+2 (12) + 12 = 72 bytes. Aligned to 8.

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

fn classify_item_visual_kind(
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

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::tileart::TileType;

    #[test]
    fn classify_item_visual_kind_marks_solid_entries_as_surface_like() {
        let art_data = ArtData {
            tile_type: TileType::Solid,
            ..ArtData::default()
        };

        assert_eq!(
            classify_item_visual_kind(1444, Some(&art_data)),
            TileMetaItemVisualKind::SurfaceLike,
        );
    }

    #[test]
    fn classify_item_visual_kind_keeps_static_entries_as_regular_art() {
        let art_data = ArtData {
            tile_type: TileType::Static,
            ..ArtData::default()
        };

        assert_eq!(
            classify_item_visual_kind(172, Some(&art_data)),
            TileMetaItemVisualKind::RegularArt,
        );
    }
}

pub struct TileMetaPackage {
    #[allow(dead_code)]
    package: UddpReader,
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
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
        let land_bytes = read_path_entry(&package, TILEMETA_LAND_ENTRY_PATH)?;
        let item_bytes = read_path_entry(&package, TILEMETA_ITEM_ENTRY_PATH)?;

        Ok(Self {
            package,
            land_tiles: read_pod_vec(&land_bytes, TILEMETA_LAND_ENTRY_PATH)?,
            item_tiles: read_pod_vec(&item_bytes, TILEMETA_ITEM_ENTRY_PATH)?,
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
}

fn read_pod_vec<T: Pod>(bytes: &[u8], entry_path: &str) -> eyre::Result<Vec<T>> {
    let record_size = std::mem::size_of::<T>();
    if bytes.len() % record_size != 0 {
        eyre::bail!(
            "{} size {} is not a multiple of record size {}",
            entry_path,
            bytes.len(),
            record_size,
        );
    }

    Ok(bytes
        .chunks_exact(record_size)
        .map(bytemuck::pod_read_unaligned)
        .collect())
}

fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("unpack {path}"))
}

pub fn find_string_dictionary_path(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    find_first_existing_file(
        source_dirs,
        &["string_dictionary.uop"],
    )
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

    let land_bytes = bytemuck::cast_slice(&built.land_tiles);
    let item_bytes = bytemuck::cast_slice(&built.item_tiles);

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        apply_planar: false,
        virtual_path: Some(TILEMETA_LAND_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: land_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        apply_planar: false,
        virtual_path: Some(TILEMETA_ITEM_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_bytes,
    })?;
    build_and_write_package(&mut package, out_file)?;

    Ok(())
}

pub fn build_tilemeta_item_payload_from_sources(
    source_dirs: &[PathBuf],
    options: &TileMetaBuildOptions,
) -> eyre::Result<(Vec<u8>, TileMetaBuildSummary)> {
    let built = build_tilemeta_tables_from_sources(source_dirs, options, "updating tilemeta")?;
    Ok((bytemuck::cast_slice(&built.item_tiles).to_vec(), built.summary))
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

    println!("Using tiledata.mul: {}", tiledata_path.display());
    println!("Using tileart.uop: {}", tileart_path.display());
    println!("Using string dictionary: {}", stringdict_path.display());
    if let Some(ref p) = radarcol_path {
        println!("Using radarcol.mul: {}", p.display());
    }

    info!("Converting Tile Metadata tables from MUL/UOP sources");

    let cc_tiledata = TileData::load(tiledata_path.clone())?;
    let ec_art = ArtDefinition::load(&tileart_path, &stringdict_path)?;
    let ec_art_crop_adjustments = if options.adjust_ec_art_sampling {
        compute_ec_art_crop_adjustments_from_sources(source_dirs)?
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

    let get_radar_color = |id: u32, is_item: bool, ec_radar: Option<&uocf::enhanced::tileart::TaeRadarcol>| -> [u8; 4] {
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
            radar_color: get_radar_color(tile.tile_id as u32, false, ec_art.definitions.get(&(tile.tile_id as u16)).map(|d| &d.radar_color)),
            name: tile.name,
        });
    }

    let mut tilemeta_items = Vec::with_capacity(cc_tiledata.item_tiles().len());
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

        if let Some(ec_data) = ec_art.definitions.get(&(tile.tile_id as u16)) {
            tile_meta_item.flags |= ec_data.flags.bits();
            tile_meta_item.set_visual_kind(classify_item_visual_kind(tile.tile_id as u32, Some(ec_data)));

            // Unify radar color
            tile_meta_item.radar_color = get_radar_color(tile.tile_id as u32, true, Some(&ec_data.radar_color));

            // Unify EC Texture
            if let Some(ec_tex) = &ec_data.ec_texture {
                tile_meta_item.ec_texture_id = ec_tex.texture_id;
                let crop_adjustment = ec_art_crop_adjustments
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
        } else {
            tile_meta_item.set_visual_kind(classify_item_visual_kind(tile.tile_id as u32, None));
        }
        tilemeta_items.push(tile_meta_item);
    }
    pb.finish_with_message("Tile Metadata unified");

    Ok(BuiltTileMetaTables {
        land_tiles: tilemeta_land,
        item_tiles: tilemeta_items,
        summary: TileMetaBuildSummary {
            adjusted_ec_item_count,
        },
    })
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
    adjustment: Option<EcArtCropAdjustment>,
) -> eyre::Result<(i16, i16)> {
    let adjusted_x = start_x - adjustment.map_or(0, |adjustment| i32::from(adjustment.left));
    let adjusted_y = start_y - adjustment.map_or(0, |adjustment| i32::from(adjustment.top));
    Ok((
        i16::try_from(adjusted_x)
            .map_err(|_| eyre::eyre!("tile {texture_id} adjusted EC start_x {adjusted_x} does not fit in i16"))?,
        i16::try_from(adjusted_y)
            .map_err(|_| eyre::eyre!("tile {texture_id} adjusted EC start_y {adjusted_y} does not fit in i16"))?,
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

