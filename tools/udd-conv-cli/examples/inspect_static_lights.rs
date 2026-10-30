//! Inspect actual packed light-source items and their required mask slots.

use std::path::PathBuf;
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use serde_json::json;
use udd_assets::{map_metadata::decode_map_package_metadata, tilemeta::TileMetaPackage,
    world_lights::WorldLightsPackage};
use udd_container::{FileKey, UddpReader};
use uocf::classic::statics::StaticTile;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    packages: PathBuf,
    #[arg(long, default_value_t = 0)]
    map: u32,
    #[arg(long)]
    x: u32,
    #[arg(long)]
    y: u32,
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u32).range(0..=128))]
    radius: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let map = UddpReader::load(args.packages.join(format!("map{}.uddp", args.map)))?;
    let statics = UddpReader::load(args.packages.join(format!("statics{}.uddp", args.map)))?;
    let meta = TileMetaPackage::load(args.packages.join("tilemeta.uddp"))?;
    let masks = WorldLightsPackage::load(args.packages.join("world_lights.uddp"))?;
    let FileKey::Id(id) = map.records().last().expect("map metadata record").key else {
        panic!("dense map IDs required");
    };
    let dimensions = decode_map_package_metadata(&map.read_file_by_dense_id(id)?)
        .map_err(|error| color_eyre::eyre::eyre!(error))?;
    ensure!(args.x < dimensions.width_tiles && args.y < dimensions.height_tiles, "position outside map");
    let min_x = args.x.saturating_sub(args.radius);
    let min_y = args.y.saturating_sub(args.radius);
    let max_x = args.x.saturating_add(args.radius).min(dimensions.width_tiles - 1);
    let max_y = args.y.saturating_add(args.radius).min(dimensions.height_tiles - 1);
    let mut items = Vec::new();
    for chunk_x in min_x / 32..=max_x / 32 {
        for chunk_y in min_y / 32..=max_y / 32 {
            let chunk = chunk_x * dimensions.height_tiles.div_ceil(32) + chunk_y;
            let bytes = statics.read_file_by_dense_id(chunk)?;
            let tiles: Vec<StaticTile> = udd_assets::common::read_pod_vec(&bytes, "static chunk")?;
            for tile in tiles {
                let x = chunk_x * 32 + u32::from(tile.x_offset);
                let y = chunk_y * 32 + u32::from(tile.y_offset);
                if x < min_x || x > max_x || y < min_y || y > max_y { continue; }
                let Some(metadata) = meta.item_tile(tile.graphic.into()) else { continue; };
                if metadata.flags & 0x00800000 == 0 { continue; }
                let mask = masks.present_slot(u32::from(metadata.quality));
                items.push(json!({"position": [x as i32, y as i32, i32::from(tile.z)],
                    "graphic": tile.graphic, "hue": tile.hue, "height": metadata.height,
                    "name": String::from_utf8_lossy(&metadata.name).trim_end_matches('\0'),
                    "mask_id": metadata.quality, "mask_extent": mask.map(|slot| [slot.width, slot.height])}));
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&json!({"map": args.map,
        "center": [args.x, args.y], "radius": args.radius,
        "available_masks": masks.slots().iter().filter(|slot| slot.is_present()).count(),
        "items": items}))?);
    Ok(())
}
