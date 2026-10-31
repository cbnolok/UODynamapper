//! The EC art atlas is always alpha-trimmed, and the trim is written once per consumer: the atlas slot draw offsets
//! keep placement, and `tilemeta.uddp` shifts the EC sampling starts to the trimmed image. Both are the same whether
//! tilemeta is built in the `pack-ec-textures` run that packed the atlas (from its recorded trims) or by a separate
//! `pack-tilemeta` (which recomputes them). Needs real clients.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use udd_assets::{TexArtEcPackage, TileMetaPackage};
use udd_container::CompressionFlag;
use udd_conv::tex_art_ec::{
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources,
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources, load_tex_art_ec_sources, TexArtEcAtlasOptions,
};
use udd_conv::tilemeta::{
    build_tilemeta_uddp_from_split_sources, build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources,
    TileMetaBuildOptions,
};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";

fn client_dirs() -> Option<(PathBuf, PathBuf)> {
    let ec = std::env::var_os(EC_CLIENT_DIR_ENV).map(PathBuf::from);
    let cc = std::env::var_os(CC_CLIENT_DIR_ENV).map(PathBuf::from);
    if ec.is_none() || cc.is_none() {
        println!("set {EC_CLIENT_DIR_ENV} and {CC_CLIENT_DIR_ENV}; skipped");
    }
    ec.zip(cc)
}

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).expect("system time is after unix epoch").as_nanos();
    let dir = std::env::temp_dir().join(format!("uddconv_{test_name}_{timestamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Uncompressed pages, so the test spends its time decoding and trimming, not encoding.
fn atlas_options() -> TexArtEcAtlasOptions {
    TexArtEcAtlasOptions { compression: CompressionFlag::None, ..TexArtEcAtlasOptions::default() }
}

/// Each present slot's art id, size and draw offsets.
fn slot_placements(path: &Path) -> Vec<(u32, u16, u16, i16, i16)> {
    let package = TexArtEcPackage::load_in_memory(path).expect("load tex_art_ec.uddp");
    package
        .slots()
        .iter()
        .filter(|slot| slot.is_present())
        .map(|slot| (slot.art_id, slot.width, slot.height, slot.draw_offset_x, slot.draw_offset_y))
        .collect()
}

/// Each item tile's EC texture and sampling start.
fn ec_starts(path: &Path) -> Vec<(u32, u32, i16, i16)> {
    let package = TileMetaPackage::load_in_memory(path).expect("load tilemeta.uddp");
    package.item_tiles().iter().map(|tile| (tile.tile_id, tile.ec_texture_id, tile.ec_start_x, tile.ec_start_y)).collect()
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR and UO_CC_CLIENT_DIR, run with --ignored"]
fn same_run_and_standalone_tilemeta_store_the_same_trims() {
    let Some((ec_dir, cc_dir)) = client_dirs() else { return };
    let dir = temp_dir("ec_art_trim_client");
    let options = TileMetaBuildOptions::default();

    let sources = load_tex_art_ec_sources(std::slice::from_ref(&ec_dir)).expect("load EC sources");
    let same_run_art = dir.join("same_run_tex_art_ec.uddp");
    let summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(&sources, &same_run_art, &atlas_options())
        .expect("pack EC art");
    let same_run_meta = dir.join("same_run_tilemeta.uddp");
    build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
        &cc_dir,
        &sources,
        &summary.ec_crop_adjustments,
        &same_run_meta,
        &options,
    )
    .expect("build tilemeta from the recorded trims");

    let standalone_art = dir.join("standalone_tex_art_ec.uddp");
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources(
        std::slice::from_ref(&ec_dir),
        &standalone_art,
        &atlas_options(),
    )
    .expect("pack EC art standalone");
    let standalone_meta = dir.join("standalone_tilemeta.uddp");
    build_tilemeta_uddp_from_split_sources(&cc_dir, &ec_dir, &standalone_meta, &options)
        .expect("build tilemeta standalone");

    assert_eq!(slot_placements(&same_run_art), slot_placements(&standalone_art));
    let same_run_starts = ec_starts(&same_run_meta);
    assert_eq!(same_run_starts, ec_starts(&standalone_meta));

    // The start is the trimmed image's origin, and some tiles are really shifted.
    let mut shifted = 0usize;
    for &(tile_id, _, start_x, start_y) in &same_run_starts {
        let Some(texture) = u16::try_from(tile_id)
            .ok()
            .and_then(|id| sources.art_definition.definitions.get(&id))
            .and_then(|art| art.ec_texture.as_ref())
        else {
            continue;
        };
        let Some(adjustment) = summary.ec_crop_adjustments.get(tile_id as usize).copied().flatten() else {
            continue;
        };
        assert_eq!(
            (start_x, start_y),
            (adjustment.source_left, adjustment.source_top),
            "tile {tile_id}: tilemeta start is the trimmed image's origin"
        );
        // An empty window packs the whole texture, whose origin is 0; a window starting inside the texture is
        // packed from its start.
        let window_is_used = texture.start_x >= 0
            && texture.start_y >= 0
            && texture.end_x > texture.start_x
            && texture.end_y > texture.start_y;
        if window_is_used {
            assert_eq!(
                (i32::from(start_x) - texture.start_x, i32::from(start_y) - texture.start_y),
                (i32::from(adjustment.trim_left), i32::from(adjustment.trim_top)),
                "tile {tile_id}: the tileart start {:?} shifted by the top/left trim",
                (texture.start_x, texture.start_y, texture.end_x, texture.end_y)
            );
        }
        if adjustment.trim_left != 0 || adjustment.trim_top != 0 {
            shifted += 1;
        }
    }
    println!("{shifted} item tiles have EC starts shifted by a top/left trim");
    assert!(shifted > 0, "the client's EC art has trimmed borders");

    let _ = fs::remove_dir_all(dir);
}
