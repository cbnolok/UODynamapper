//! Export routed material ownership and original parameters for one 32x32 chunk.

use std::{collections::{BTreeMap, BTreeSet}, fs::OpenOptions, path::PathBuf};

use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use serde_json::json;
use udd_assets::{map_metadata::decode_map_package_metadata, tex_land_ec::TexLandEcPackage};
use udd_container::{FileKey, UddpReader};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    packages: PathBuf,
    #[arg(long, default_value_t = 0)]
    map: u32,
    /// World cell inside the requested chunk.
    #[arg(long)]
    x: u32,
    #[arg(long)]
    y: u32,
    /// Optional loose TerrainTranscode.kdl, applied after embedded routing.
    #[arg(long)]
    routing: Option<PathBuf>,
    /// New JSON file; existing evidence is never overwritten.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let map = UddpReader::load(args.packages.join(format!("map{}.uddp", args.map)))?;
    let mut terrain = TexLandEcPackage::load(args.packages.join("tex_land_ec.uddp"))?;
    if let Some(path) = &args.routing {
        terrain.set_transcode(udd_assets::eckr_terrain_kdl::EckrTerrainRouting::load(path)?.to_map());
    }
    let FileKey::Id(id) = map.records().last().expect("map metadata record").key else {
        color_eyre::eyre::bail!("dense map IDs required");
    };
    let dimensions = decode_map_package_metadata(&map.read_file_by_dense_id(id)?)
        .map_err(|error| color_eyre::eyre::eyre!(error))?;
    let origin = [args.x / 32 * 32, args.y / 32 * 32];
    // The original edge test needs the neighboring chunk's ownership too.
    // Refuse facet-edge fixtures instead of inventing out-of-world materials.
    ensure!(origin[0] > 0 && origin[1] > 0
        && origin[0] + 32 < dimensions.width_tiles
        && origin[1] + 32 < dimensions.height_tiles, "a full one-cell border is required");
    let mut chunks = BTreeMap::new();
    let mut material_ids = BTreeSet::new();
    let mut cells = Vec::new();
    let mut land_ids = Vec::new();
    for y in origin[1] - 1..=origin[1] + 32 {
        let mut row = Vec::new();
        let mut land_row = Vec::new();
        for x in origin[0] - 1..=origin[0] + 32 {
            let chunk_id = x / 32 * dimensions.height_tiles.div_ceil(32) + y / 32;
            if !chunks.contains_key(&chunk_id) {
                let bytes = map.read_file_by_dense_id(chunk_id)?;
                ensure!(bytes.len() == 32 * 32 * 4, "invalid map chunk extent");
                chunks.insert(chunk_id, bytes);
            }
            let offset = ((y % 32 * 32 + x % 32) * 4) as usize;
            let bytes = &chunks[&chunk_id];
            let land_id = u16::from_le_bytes(bytes[offset..offset + 2].try_into()?);
            let material_id = terrain.resolve_material_decision(u32::from(land_id)).material_id
                .ok_or_else(|| color_eyre::eyre::eyre!("unresolved land {land_id} at {x},{y}"))?;
            material_ids.insert(material_id);
            row.push(material_id);
            land_row.push(land_id);
        }
        cells.push(row);
        land_ids.push(land_row);
    }
    let materials = material_ids.into_iter().map(|id| {
        terrain.source_material(id).cloned()
            .ok_or_else(|| color_eyre::eyre::eyre!("missing source material {id}"))
    }).collect::<Result<Vec<_>>>()?;
    let evidence = json!({
        "map": args.map,
        "packages": args.packages,
        "loose_routing": args.routing,
        "chunk_origin": origin,
        "ownership_source": "packed map and effective tex_land_ec routing",
        "cells": cells,
        "land_ids": land_ids,
        "materials": materials,
    });
    let output = OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    serde_json::to_writer_pretty(output, &evidence)?;
    println!("saved chunk {},{} ownership to {:?}", origin[0], origin[1], args.output);
    Ok(())
}
