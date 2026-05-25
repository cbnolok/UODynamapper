//! Build-time support for `gumps_cc.uddp`.

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use uocf::classic::gump::GumpMap;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::gump_atlas::{
    add_gump_atlas_files, is_paperdoll_equipment_gump_id, DecodedGump, GumpAtlasOptions,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::{find_first_existing_file, source_path_label};

pub const GUMPS_CC_DEFAULT_OUTPUT: &str = "gumps_cc.uddp";

pub struct CcGumpsBuildSummary {
    pub gump_count: u32,
    pub single_gump_count: u32,
    pub atlas_gump_count: u32,
}

pub fn convert_gumps_to_uddp_from_sources_with_patches(
    source_dirs: &[PathBuf],
    output_path: &Path,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcGumpsBuildSummary> {
    let source_root = find_first_existing_file(source_dirs, &["gumpidx.mul", "gumpartLegacyMUL.uop"])
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or_else(|| eyre::eyre!("missing Classic gump source"))?;
    let gump_idx_path = ["gumpidx.mul", "Gumpidx.mul"]
        .iter()
        .map(|name| source_root.join(name))
        .find(|path| path.is_file());
    let gump_mul_path = ["gumpart.mul", "Gumpart.mul"]
        .iter()
        .map(|name| source_root.join(name))
        .find(|path| path.is_file());
    let gump_uop_path = [
        "gumpartLegacyMUL.uop",
        "GumpartLegacyMUL.uop",
        "gumpartlegacymul.uop",
    ]
    .iter()
    .map(|name| source_root.join(name))
    .find(|path| path.is_file());
    if let (Some(idx_path), Some(mul_path)) = (&gump_idx_path, &gump_mul_path) {
        println!("Using CC gump index source file: {}", source_path_label(&source_root, idx_path));
        println!("Using CC gump source file (MUL): {}", source_path_label(&source_root, mul_path));
    } else if let Some(uop_path) = &gump_uop_path {
        println!("Using CC gump source file (UOP): {}", source_path_label(&source_root, uop_path));
    }
    if gump_idx_path.is_some() && gump_mul_path.is_some() && gump_uop_path.is_some() {
        println!(
            "Using CC gump source preference: MUL first; UOP fallback available: {}",
            source_path_label(&source_root, gump_uop_path.as_ref().expect("checked above"))
        );
    }

    let mut gumps = GumpMap::load(&source_root)
        .wrap_err_with(|| format!("load Classic gumps from {}", source_root.display()))?;
    if let Some(verdata) = load_verdata_if_enabled(source_dirs, patch_options)? {
        gumps = gumps.with_verdata(verdata);
    }

    let max_id = gumps.max_id();
    let pb = ProgressBar::new(u64::from(max_id) + 1);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message("extracting CC gumps");

    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut scratch = Vec::new();
    let mut gump_count = 0u32;
    let mut single_gump_count = 0u32;
    let mut atlas_gumps = Vec::new();

    for gump_id in 0..=max_id {
        pb.inc(1);
        if !gumps.has_id(gump_id) {
            continue;
        }

        let (width, height, rgba) = match gumps.decode_gump(gump_id, &mut scratch) {
            Ok(decoded) => decoded,
            Err(error) => {
                log::warn!("Skipping gump {gump_id}: {error}");
                continue;
            }
        };

        if is_paperdoll_equipment_gump_id(gump_id) {
            atlas_gumps.push(DecodedGump {
                gump_id,
                width,
                height,
                upscale_factor: 1,
                rgba,
            });
        } else {
            add_single_gump(&mut builder, gump_id, u32::from(width), u32::from(height), &rgba)?;
            single_gump_count += 1;
        }
        gump_count += 1;
    }

    let atlas_gump_count =
        add_gump_atlas_files(&mut builder, atlas_gumps, &GumpAtlasOptions::default())?;
    pb.finish_with_message("CC gumps extracted");
    build_and_write_package(&mut builder, output_path)?;

    Ok(CcGumpsBuildSummary {
        gump_count,
        single_gump_count,
        atlas_gump_count,
    })
}

fn add_single_gump(
    builder: &mut UddpBuilder,
    gump_id: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> eyre::Result<()> {
    let mut payload = Vec::with_capacity(8 + rgba.len());
    payload.write_u32::<LittleEndian>(width)?;
    payload.write_u32::<LittleEndian>(height)?;
    payload.extend_from_slice(rgba);

    builder.add_file(AddFileRequest {
        data_type: DataType::Gump as u8,
        compression: CompressionFlag::ZstdNoDict,
        width,
        height,
        virtual_path: None,
        path_hash64: None,
        id: Some(gump_id),
        data: &payload,
    })?;
    Ok(())
}
