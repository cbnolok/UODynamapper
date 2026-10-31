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
    /// Include ordinary placements when locating a source-geometry fixture.
    #[arg(long)]
    include_unhued: bool,
    /// Match the packed EC sprite's actual source rather than guessing from its ID.
    #[arg(long, value_parser = ["legacy", "scaled-world"])]
    ec_source: Option<String>,
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
    let ec_art = args.ec_source.as_ref().map(|_|
        udd_assets::tex_art_ec::TexArtEcPackage::load(args.packages.join("tex_art_ec.uddp"))).transpose()?;
    let tilemeta = args.ec_source.as_ref().map(|_|
        udd_assets::tilemeta::TileMetaPackage::load(args.packages.join("tilemeta.uddp"))).transpose()?;
    let mut found = 0;
    println!("x,y,graphic,z,hue");
    'chunks: for record in statics.records() {
        let FileKey::Id(id) = record.key else { continue; };
        if id >= count { continue; }
        let bytes = statics.read_file_by_dense_id(id)?;
        let tiles: Vec<StaticTile> = udd_assets::common::read_pod_vec(&bytes, "static chunk")?;
        for tile in tiles {
            if (!args.include_unhued && tile.hue == 0)
                || (!graphics.is_empty() && !graphics.contains(&tile.graphic)) { continue; }
            if let Some(art) = &ec_art {
                let id = u32::from(tile.graphic);
                if !art.present_slot(id).is_some_and(|slot| slot.is_static()) { continue; }
                if tilemeta.as_ref().and_then(|meta| meta.item_tile(id)).is_some_and(|meta| meta.is_nodraw()) { continue; }
                let Some(geometry) = art.sprite_geometry(id) else { continue; };
                let matches = match args.ec_source.as_deref() {
                    Some("legacy") => !geometry.source_is_world_art,
                    Some("scaled-world") => geometry.source_is_world_art && geometry.descriptor.world_art_scale != 1.0,
                    _ => false,
                };
                if !matches { continue; }
            }
            println!("{},{},{},{},{}", id / rows * 32 + u32::from(tile.x_offset),
                id % rows * 32 + u32::from(tile.y_offset), tile.graphic, tile.z, tile.hue);
            found += 1;
            if found >= args.limit { break 'chunks; }
        }
    }
    Ok(())
}
