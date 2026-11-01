//! Packs `tilemeta.uddp` and `tex_aux_ec.uddp` from the real clients and checks that every block-0 support ref
//! of an item with an Enhanced material resolves in `tex_aux_ec`, with pixels of the declared size. The renderer
//! reads these refs (blend and coverage alpha, noise, hue mask, water layers) by texture id; a ref without a slot
//! draws without its support image.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix); the installs are the folders
//! named [`CLASSIC_FOLDER`] and [`ENHANCED_FOLDER`] in one of them. The clients are only read; the packages go to
//! `std::env::temp_dir()`. Run with `--ignored --release`.

use std::fs;
use std::path::PathBuf;

use udd_assets::tex_aux_ec::TexAuxEcPackage;
use udd_assets::tilemeta::EcMaterialLogicalFamily;
use udd_assets::TileMetaPackage;
use udd_conv::tex_aux_ec::{pack_tex_aux_ec, TexAuxEcPackOptions};
use udd_conv::tilemeta::{build_tilemeta_uddp_from_split_sources, TileMetaBuildOptions};

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const CLASSIC_FOLDER: &str = "_Ultima Online Classic";
const ENHANCED_FOLDER: &str = "_Ultima Online Enhanced";
/// The block whose refs the renderer reads as an item's material.
const MATERIAL_BLOCK_INDEX: u8 = 0;
/// RGBA8888 bytes per pixel, as `TexAuxEcPackage::load_rgba` returns them.
const RGBA_BYTES_PER_PIXEL: usize = 4;
/// Missing refs listed in the failure message.
const MAX_LISTED_MISSING: usize = 32;

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn every_block_0_support_ref_of_a_material_item_resolves_in_tex_aux_ec() {
    let classic = install(CLASSIC_FOLDER);
    let enhanced = install(ENHANCED_FOLDER);
    let dir = std::env::temp_dir().join(format!("uddconv_tex_aux_ec_client_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tilemeta_path = dir.join("tilemeta.uddp");
    let aux_path = dir.join("tex_aux_ec.uddp");

    build_tilemeta_uddp_from_split_sources(&classic, &enhanced, &tilemeta_path, &TileMetaBuildOptions::default())
        .unwrap();
    let summary =
        pack_tex_aux_ec(&[enhanced.clone()], &tilemeta_path, &aux_path, &TexAuxEcPackOptions::default()).unwrap();
    println!("tex_aux_ec: {summary:?}");

    let tilemeta = TileMetaPackage::load(&tilemeta_path).unwrap();
    let aux = TexAuxEcPackage::load(&aux_path).unwrap();

    let mut material_items = 0usize;
    let mut support_refs = 0usize;
    let mut missing_source_refs = 0usize;
    let mut missing = Vec::new();
    let mut bad_pixels = Vec::new();
    for item in tilemeta.item_tiles() {
        let block: Vec<_> = tilemeta
            .item_texture_refs(item.tile_id)
            .iter()
            .filter(|tex_ref| tex_ref.block_index == MATERIAL_BLOCK_INDEX)
            .collect();
        if !block.iter().any(|tex_ref| tex_ref.is_primary_selected()) {
            continue;
        }
        material_items += 1;
        for tex_ref in block.iter().filter(|tex_ref| {
            !tex_ref.is_primary_selected()
                && tex_ref.logical_family() == EcMaterialLogicalFamily::WorldArt
                && tex_ref.texture_id != 0
        }) {
            support_refs += 1;
            let Some(slot) = aux.slot_for_texture_id(tex_ref.texture_id) else {
                missing.push((item.tile_id, tex_ref.item_index, tex_ref.texture_id, tex_ref.stable_role()));
                continue;
            };
            if slot.is_missing_source() {
                missing_source_refs += 1;
            }
            let expected = usize::from(slot.width) * usize::from(slot.height) * RGBA_BYTES_PER_PIXEL;
            match aux.load_rgba(tex_ref.texture_id) {
                Ok(Some(rgba)) if rgba.len() == expected => {}
                other => bad_pixels.push((tex_ref.texture_id, other.map(|rgba| rgba.map(|rgba| rgba.len())))),
            }
        }
    }
    println!(
        "{material_items} material items, {support_refs} block-0 support refs ({missing_source_refs} read the \
         missing-texture image), {} missing, {} with bad pixels",
        missing.len(),
        bad_pixels.len()
    );
    fs::remove_dir_all(&dir).ok();

    assert!(material_items > 0 && support_refs > 0, "the clients gave no material items with support refs");
    assert!(
        missing.is_empty(),
        "{} support refs have no tex_aux_ec slot (tile, item, texture, role): {:?}",
        missing.len(),
        &missing[..missing.len().min(MAX_LISTED_MISSING)]
    );
    assert!(bad_pixels.is_empty(), "support refs whose pixels do not read back: {bad_pixels:?}");
}
