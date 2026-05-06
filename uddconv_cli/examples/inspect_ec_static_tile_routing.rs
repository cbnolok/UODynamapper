use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uddconv::ec_art::{EcArtPackage, EcArtSlotRecord};
use uddconv::ec_land::{EcLandPackage, EcLandSlotRecord};
use uddconv::tilemeta::{TileMetaItemTile, TileMetaPackage};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let tilemeta_path = PathBuf::from(args.next().ok_or_else(|| {
        eyre::eyre!(
            "usage: inspect_ec_static_tile_routing <tilemeta.uddp> <ec_art.uddp> <ec_land.uddp> <tile_id> [tile_id ...]"
        )
    })?);
    let ec_art_path = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing <ec_art.uddp> argument"))?,
    );
    let ec_land_path = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing <ec_land.uddp> argument"))?,
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
            "usage: inspect_ec_static_tile_routing <tilemeta.uddp> <ec_art.uddp> <ec_land.uddp> <tile_id> [tile_id ...]"
        );
    }

    let tilemeta = TileMetaPackage::load(&tilemeta_path)
        .wrap_err_with(|| format!("load {}", tilemeta_path.display()))?;
    let ec_art = EcArtPackage::load(&ec_art_path)
        .wrap_err_with(|| format!("load {}", ec_art_path.display()))?;
    let ec_land = EcLandPackage::load(&ec_land_path)
        .wrap_err_with(|| format!("load {}", ec_land_path.display()))?;

    for tile_id in tile_ids {
        println!("tile_id={tile_id}");

        let Some(item) = tilemeta.item_tile(tile_id) else {
            println!("  tilemeta=missing");
            continue;
        };

        print_tilemeta(item);

        let ec_art_direct = ec_art.present_slot(tile_id);
        print_ec_art_slot("ec_art.present_slot(tile_id)", ec_art_direct);

        let ec_land_direct = ec_land.present_slot(tile_id);
        print_ec_land_slot("ec_land.present_slot(tile_id)", ec_land_direct);

        let ec_land_from_cc = ec_land.resolve_runtime_slot_id(item.cc_texture_id);
        println!(
            "  ec_land.resolve_runtime_slot_id(cc_texture_id={})={:?}",
            item.cc_texture_id, ec_land_from_cc
        );
        if let Some(runtime_slot_id) = ec_land_from_cc {
            let resolved_slot = ec_land.present_slot(runtime_slot_id);
            print_ec_land_slot("  ec_land.present_slot(resolved_runtime_slot)", resolved_slot);
        }

        let renderer_decision = if item.is_surface_like() {
            match ec_land_from_cc {
                Some(runtime_slot_id) => format!("EcLandArt(runtime_slot_id={runtime_slot_id})"),
                None => "EcLandArt(unresolved)".to_string(),
            }
        } else if ec_art_direct.is_some() {
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

fn print_ec_art_slot(label: &str, slot: Option<&EcArtSlotRecord>) {
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

fn print_ec_land_slot(label: &str, slot: Option<&EcLandSlotRecord>) {
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
