//! Build-time packer for `ec_tex_aux.uddp`.
//!
//! Packs image-bearing auxiliary texture refs from tileart entries — those whose
//! `EcMaterialStableRole` is not `Base` or `SecondaryBase` — into a UDDP keyed
//! by virtual path hash. Each payload is raw RGBA8888 pixels stored at the virtual
//! path `textures/{texture_id:08x}`.
//!
//! Package layout:
//! - `metadata/slots.bin`: flat sorted array of `TexAuxEcSlotRecord`.
//! - `textures/{texture_id:08x}`: RGBA8888 raw pixel bytes per `texture_id`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytemuck::cast_slice;
use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;

use udd_assets::tex_aux_ec::{
    texture_entry_path, TexAuxEcSlotRecord, TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH,
};
use udd_assets::tilemeta::{EcMaterialPhysicalPackage, EcMaterialStableRole, TileMetaPackage};
use udd_container::{
    AddFileRequest, AddOwnedFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder,
};
use uocf::enhanced::textures::Textures;

use crate::package_progress::build_and_write_package_with_progress;
use crate::source_paths::find_first_existing_file;

/// Breakdown of how refs were classified during packing.
#[derive(Debug, Clone, Copy, Default)]
pub struct EcTexAuxPackSummary {
    /// Total `TileMetaItemTextureRef` rows considered.
    pub refs_considered: u64,
    /// Refs with a base role — excluded (those belong to tex_art_ec).
    pub excluded_base_role: u64,
    /// Refs with no resolvable EC client path or unknown physical package.
    pub excluded_no_path: u64,
    /// Refs whose texture file could not be decoded to RGBA8888.
    pub excluded_decode_error: u64,
    /// Refs that are duplicate texture_ids already queued from a prior owner tile.
    pub excluded_duplicate_id: u64,
    /// Refs successfully packed as auxiliary image entries.
    pub included: u64,
}

/// Entry for deferred parallel decoding.
struct QualifiedRef {
    texture_id: u32,
    logical_family: u8,
    stable_role: u8,
    physical_package: EcMaterialPhysicalPackage,
}

/// Pack auxiliary EC texture refs from `tilemeta_path` into `output`.
///
/// Source images are looked up from the EC source directories using the
/// `physical_package` field on each `TileMetaItemTextureRef` to select the
/// right UOP archive. Only refs with a decodable RGBA image and a non-base
/// stable role are included.
pub fn pack_ec_tex_aux(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    output: &Path,
) -> eyre::Result<EcTexAuxPackSummary> {
    info!(
        "Building ec_tex_aux.uddp from tilemeta '{}' -> '{}'",
        tilemeta_path.display(),
        output.display()
    );

    let tilemeta = TileMetaPackage::load(tilemeta_path)
        .wrap_err("load tilemeta for ec_tex_aux packing")?;

    // Open the source texture UOP archives once.
    let texture_uop_path = find_first_existing_file(source_dirs, &["Texture.uop"]);
    let legacy_texture_uop_path = find_first_existing_file(source_dirs, &["LegacyTexture.uop"]);
    let terrain_texture_uop_path = find_first_existing_file(source_dirs, &["TerrainTexture.uop"]);
    let effect_texture_uop_path = find_first_existing_file(source_dirs, &["EffectTexture.uop"]);

    let world_textures = texture_uop_path
        .as_ref()
        .map(|p| Textures::new(p, None, None).wrap_err_with(|| format!("load {}", p.display())))
        .transpose()?;
    let legacy_textures = legacy_texture_uop_path
        .as_ref()
        .map(|p| Textures::new(p, None, None).wrap_err_with(|| format!("load {}", p.display())))
        .transpose()?;
    let terrain_textures = terrain_texture_uop_path
        .as_ref()
        .map(|p| Textures::new(p, None, None).wrap_err_with(|| format!("load {}", p.display())))
        .transpose()?;
    let effect_textures = effect_texture_uop_path
        .as_ref()
        .map(|p| Textures::new(p, None, None).wrap_err_with(|| format!("load {}", p.display())))
        .transpose()?;

    let mut summary = EcTexAuxPackSummary::default();

    // Collect unique qualifying refs.
    // Deduplication is by texture_id — the same texture may appear for multiple
    // owner tile ids, but we pack one image per texture_id.
    let mut qualifying_refs: Vec<QualifiedRef> = Vec::new();
    let mut seen_ids: HashMap<u32, ()> = HashMap::new();

    for item_tile in tilemeta.item_tiles() {
        let tile_id = item_tile.tile_id;
        let refs = tilemeta.item_texture_refs(tile_id);
        for tex_ref in refs {
            summary.refs_considered += 1;
            let role = tex_ref.stable_role();
            if role == EcMaterialStableRole::Base || role == EcMaterialStableRole::SecondaryBase {
                summary.excluded_base_role += 1;
                continue;
            }
            let package = tex_ref.physical_package();
            if package == EcMaterialPhysicalPackage::Unknown {
                summary.excluded_no_path += 1;
                continue;
            }
            let texture_id = tex_ref.texture_id;
            if texture_id == 0 {
                summary.excluded_no_path += 1;
                continue;
            }
            if seen_ids.contains_key(&texture_id) {
                summary.excluded_duplicate_id += 1;
                continue;
            }
            seen_ids.insert(texture_id, ());
            qualifying_refs.push(QualifiedRef {
                texture_id,
                logical_family: tex_ref.texture_type,
                stable_role: tex_ref.stable_role,
                physical_package: package,
            });
        }
    }

    info!(
        "ec_tex_aux: {} refs considered, {} qualifying unique texture ids before decode",
        summary.refs_considered,
        qualifying_refs.len()
    );

    struct DecodedEntry {
        texture_id: u32,
        logical_family: u8,
        stable_role: u8,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    // Decode images in parallel.
    let decoded: Vec<Option<DecodedEntry>> = qualifying_refs
        .par_iter()
        .map(|q| {
            let source: Option<&Textures> = match q.physical_package {
                EcMaterialPhysicalPackage::Texture => world_textures.as_ref(),
                EcMaterialPhysicalPackage::LegacyTexture => legacy_textures.as_ref(),
                EcMaterialPhysicalPackage::TerrainTexture => terrain_textures.as_ref(),
                EcMaterialPhysicalPackage::EffectTexture => effect_textures.as_ref(),
                _ => None,
            };
            let Some(source) = source else {
                return None;
            };
            let file = match source.get_from_id(q.texture_id) {
                Ok(Some(f)) => f,
                _ => return None,
            };
            let rgba8 = match file.decode_to_rgba8() {
                Ok(img) => img,
                Err(_) => return None,
            };
            let width = rgba8.width();
            let height = rgba8.height();
            Some(DecodedEntry {
                texture_id: q.texture_id,
                logical_family: q.logical_family,
                stable_role: q.stable_role,
                width,
                height,
                rgba: rgba8.into_raw(),
            })
        })
        .collect();

    let mut slot_records: Vec<TexAuxEcSlotRecord> = Vec::new();
    let mut image_entries: Vec<(u32, u32, u32, Vec<u8>)> = Vec::new(); // (texture_id, w, h, rgba)

    for opt in decoded {
        if let Some(entry) = opt {
            summary.included += 1;
            slot_records.push(TexAuxEcSlotRecord {
                texture_id: entry.texture_id,
                width: entry.width as u16,
                height: entry.height as u16,
                logical_family: entry.logical_family,
                stable_role: entry.stable_role,
                _pad: 0,
            });
            image_entries.push((entry.texture_id, entry.width, entry.height, entry.rgba));
        } else {
            summary.excluded_decode_error += 1;
        }
    }

    // Sort by texture_id for binary-search at runtime.
    slot_records.sort_by_key(|slot| slot.texture_id);
    image_entries.sort_by_key(|(tid, _, _, _)| *tid);

    info!(
        "ec_tex_aux: {} included, {} excluded (base role: {}, no path: {}, decode error: {}, duplicates: {})",
        summary.included,
        summary.excluded_base_role
            + summary.excluded_no_path
            + summary.excluded_decode_error
            + summary.excluded_duplicate_id,
        summary.excluded_base_role,
        summary.excluded_no_path,
        summary.excluded_decode_error,
        summary.excluded_duplicate_id,
    );

    // Build UDDP.
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    let slot_bytes = cast_slice::<TexAuxEcSlotRecord, u8>(&slot_records);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some(TEX_AUX_EC_SLOT_MANIFEST_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: slot_bytes,
    })?;

    for (texture_id, width, height, rgba) in image_entries {
        let vpath = texture_entry_path(texture_id);
        package.add_owned_file(AddOwnedFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::ZstdNoDict,
            width,
            height,
            virtual_path: Some(vpath),
            path_hash64: None,
            id: None,
            data: rgba,
        })?;
    }

    build_and_write_package_with_progress(&mut package, output, &mut |_| {})?;

    info!(
        "ec_tex_aux: wrote {} auxiliary texture slots to '{}'",
        summary.included,
        output.display()
    );

    Ok(summary)
}
