use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use udd_assets::{
    tex_art_ec::{TexArtEcPackage, TexArtEcSlotRecord},
    tex_land_ec::{TexLandEcPackage, TexLandEcSlotRecord},
    tilemeta::{TileMetaItemTile, TileMetaPackage},
};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let tilemeta_path = PathBuf::from(args.next().ok_or_else(|| {
        eyre::eyre!(
            "usage: inspect_ec_static_tile_routing <tilemeta.uddp> <tex_art_ec.uddp> <tex_land_ec.uddp> <tile_id> [tile_id ...]"
        )
    })?);
    let tex_art_ec_path = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing <tex_art_ec.uddp> argument"))?,
    );
    let tex_land_ec_path = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing <tex_land_ec.uddp> argument"))?,
    );

    let tile_ids = args
        .map(|arg| {
            arg.to_string_lossy()
                .parse::<u32>()
                .wrap_err("parse tile_id")
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if tile_ids.is_empty() {
        eyre::bail!(
            "usage: inspect_ec_static_tile_routing <tilemeta.uddp> <tex_art_ec.uddp> <tex_land_ec.uddp> <tile_id> [tile_id ...]"
        );
    }

    let tilemeta = TileMetaPackage::load(&tilemeta_path)
        .wrap_err_with(|| format!("load {}", tilemeta_path.display()))?;
    let tex_art_ec = TexArtEcPackage::load(&tex_art_ec_path)
        .wrap_err_with(|| format!("load {}", tex_art_ec_path.display()))?;
    let tex_land_ec = TexLandEcPackage::load(&tex_land_ec_path)
        .wrap_err_with(|| format!("load {}", tex_land_ec_path.display()))?;

    for tile_id in tile_ids {
        println!("tile_id={tile_id}");

        let Some(item) = tilemeta.item_tile(tile_id) else {
            println!("  tilemeta=missing");
            continue;
        };

        print_tilemeta(item);

        let tex_art_ec_direct = tex_art_ec.present_slot(tile_id);
        print_tex_art_ec_slot("tex_art_ec.present_slot(tile_id)", tex_art_ec_direct);

        let tex_land_ec_direct = tex_land_ec.present_slot(tile_id);
        print_tex_land_ec_slot("tex_land_ec.present_slot(tile_id)", tex_land_ec_direct);

        let tex_land_ec_from_cc = tex_land_ec.resolve_runtime_slot_id(item.cc_texture_id);
        println!(
            "  tex_land_ec.resolve_runtime_slot_id(cc_texture_id={})={:?}",
            item.cc_texture_id, tex_land_ec_from_cc
        );
        if let Some(runtime_slot_id) = tex_land_ec_from_cc {
            let resolved_slot = tex_land_ec.present_slot(runtime_slot_id);
            print_tex_land_ec_slot("  tex_land_ec.present_slot(resolved_runtime_slot)", resolved_slot);
        }

        let renderer_decision = if item.is_surface_like() {
            match tex_land_ec_from_cc {
                Some(runtime_slot_id) => format!("TexLandEcArt(runtime_slot_id={runtime_slot_id})"),
                None => "TexLandEcArt(unresolved)".to_string(),
            }
        } else if tex_art_ec_direct.is_some() {
            format!("EcRegularArt(art_id={tile_id})")
        } else {
            format!("EcRegularArt(missing art_id={tile_id})")
        };
        println!("  renderer_decision={renderer_decision}");
        println!();
    }

    Ok(())
}

fn print_tilemeta(item: &TileMetaItemTile) {
    println!("  tilemeta.visual_kind={:?}", item.visual_kind());
    println!("  tilemeta.is_surface_like={}", item.is_surface_like());
    println!("  tilemeta.ec_texture_id={}", item.ec_texture_id);
    println!("  tilemeta.cc_texture_id={}", item.cc_texture_id);
    println!(
        "  tilemeta.ec_offsets=({}, {}) cc_offsets=({}, {})",
        item.ec_offset_x, item.ec_offset_y, item.cc_offset_x, item.cc_offset_y
    );
    println!(
        "  tilemeta.ec_start=({}, {}) cc_start=({}, {})",
        item.ec_start_x, item.ec_start_y, item.cc_start_x, item.cc_start_y
    );
}

fn print_tex_art_ec_slot(label: &str, slot: Option<&TexArtEcSlotRecord>) {
    match slot {
        Some(slot) => {
            println!(
                "  {label}=present page={} pos=({}, {}) size={}x{} flags=0x{:04x}",
                slot.page_index, slot.x, slot.y, slot.width, slot.height, slot.flags
            );
        }
        None => println!("  {label}=missing"),
    }
}

fn print_tex_land_ec_slot(label: &str, slot: Option<&TexLandEcSlotRecord>) {
    match slot {
        Some(slot) => {
            println!(
                "  {label}=present page={} pos=({}, {}) size={}x{} flags=0x{:04x}",
                slot.page_index, slot.x, slot.y, slot.width, slot.height, slot.flags
            );
        }
        None => println!("  {label}=missing"),
    }
}
