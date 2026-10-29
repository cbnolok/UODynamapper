//! Find repeatable native hue fixtures without modifying runtime map packages.
use std::{collections::HashSet, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::Result;
use udd_assets::map_metadata::decode_map_package_metadata;
use udd_container::{FileKey, UddpReader};
use uocf::classic::statics::StaticTile;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    packages: PathBuf,
    #[arg(long, default_value_t = 0)]
    map: u32,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long, num_args = 1..)]
    graphics: Vec<u16>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let map = UddpReader::load(args.packages.join(format!("map{}.uddp", args.map)))?;
    let statics = UddpReader::load(args.packages.join(format!("statics{}.uddp", args.map)))?;
    let FileKey::Id(metadata) = map.records().last().expect("map metadata record").key else {
        color_eyre::eyre::bail!("dense map IDs required");
    };
    let dimensions = decode_map_package_metadata(&map.read_file_by_dense_id(metadata)?)
        .map_err(|error| color_eyre::eyre::eyre!(error))?;
    let rows = dimensions.height_tiles.div_ceil(32);
    let count = dimensions.width_tiles.div_ceil(32) * rows;
    let graphics: HashSet<u16> = args.graphics.into_iter().collect();
    let mut found = 0;
    println!("x,y,graphic,z,hue");
    'chunks: for record in statics.records() {
        let FileKey::Id(id) = record.key else { continue; };
        if id >= count { continue; }
        let bytes = statics.read_file_by_dense_id(id)?;
        let tiles: Vec<StaticTile> = udd_assets::common::read_pod_vec(&bytes, "static chunk")?;
        for tile in tiles {
            if tile.hue == 0 || (!graphics.is_empty() && !graphics.contains(&tile.graphic)) { continue; }
            println!("{},{},{},{},{}", id / rows * 32 + u32::from(tile.x_offset),
                id % rows * 32 + u32::from(tile.y_offset), tile.graphic, tile.z, tile.hue);
            found += 1;
            if found >= args.limit { break 'chunks; }
        }
    }
    Ok(())
}
