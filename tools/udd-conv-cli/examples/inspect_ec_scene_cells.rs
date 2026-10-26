//! Inspect packed terrain heights and water items beneath a renderer fixture.

use std::{collections::HashMap, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::{map_metadata::decode_map_package_metadata, tilemeta::TileMetaPackage};
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
    #[arg(long, default_value_t = 8)]
    radius: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let map = UddpReader::load(args.packages.join(format!("map{}.uddp", args.map)))?;
    let statics = UddpReader::load(args.packages.join(format!("statics{}.uddp", args.map)))?;
    let meta = TileMetaPackage::load(args.packages.join("tilemeta.uddp"))?;
    let records = map.records();
    let FileKey::Id(id) = records.last().expect("map metadata record").key else { panic!("dense map IDs required"); };
    let dimensions = decode_map_package_metadata(&map.read_file_by_dense_id(id)?)
        .map_err(|error| color_eyre::eyre::eyre!(error))?;
    let mut chunks = HashMap::new();
    println!("x,y,land_id,land_z,land_wet,wet_items");
    for y in args.y.saturating_sub(args.radius)..=args.y.saturating_add(args.radius).min(dimensions.height_tiles - 1) {
        for x in args.x.saturating_sub(args.radius)..=args.x.saturating_add(args.radius).min(dimensions.width_tiles - 1) {
            let chunk = x / 32 * dimensions.height_tiles.div_ceil(32) + y / 32;
            if !chunks.contains_key(&chunk) {
                let cells = map.read_file_by_dense_id(chunk)?;
                ensure!(cells.len() == 32 * 32 * 4, "invalid map chunk extent");
                let bytes = statics.read_file_by_dense_id(chunk)?;
                let items: Vec<StaticTile> = udd_assets::common::read_pod_vec(&bytes, "static chunk")?;
                chunks.insert(chunk, (cells, items));
            }
            let (cells, items) = &chunks[&chunk];
            let offset = ((y % 32 * 32 + x % 32) * 4) as usize;
            let land_id = u16::from_le_bytes(cells[offset..offset + 2].try_into()?);
            let z = i16::from(cells[offset + 2]) - 128;
            let wet = items.iter().filter(|item| u32::from(item.x_offset) == x % 32
                && u32::from(item.y_offset) == y % 32
                && meta.item_tile(item.graphic.into()).is_some_and(|m| m.flags & 0x80 != 0))
                .map(|item| format!("{}@{}", item.graphic, item.z)).collect::<Vec<_>>().join(";");
            let land_wet = meta.land_tile(land_id.into()).is_some_and(|m| m.flags & 0x80 != 0);
            println!("{x},{y},{land_id},{z},{land_wet},{wet}");
        }
    }
    Ok(())
}
