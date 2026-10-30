//! Compare original terrain inputs with the effective runtime layer roles.

use std::{collections::BTreeSet, fs::OpenOptions, path::PathBuf};

use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use serde_json::json;
use udd_assets::tex_land_ec::{TexLandEcPackage, UDDP_SOURCE_MATERIALS_ENTRY_VPATH};
use udd_container::{UddpReader, xxh64_virtual_path};
use uocf::enhanced::terrain_definition::TerrainDefinitionEntry;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    package: PathBuf,
    /// Optional loose policy, applied after embedded overrides just as at runtime.
    #[arg(long)]
    overrides: Option<PathBuf>,
    /// A new JSON file. Existing evidence is never overwritten.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let reader = UddpReader::load(&args.package)?;
    let entries: Vec<TerrainDefinitionEntry> = serde_json::from_slice(
        &reader.read_file_by_path_hash(xxh64_virtual_path(UDDP_SOURCE_MATERIALS_ENTRY_VPATH))?,
    )?;
    let mut terrain = TexLandEcPackage::load(&args.package)?;
    if let Some(path) = &args.overrides {
        terrain.set_terrain_overrides_from_kdl(path)?;
    }
    let mut materials = Vec::new();
    for entry in entries {
        let queries: BTreeSet<_> = entry.aliases.iter().map(|alias| alias.alias)
            .chain(std::iter::once(entry.id)).collect();
        let query = queries.into_iter().find(|query|
            terrain.resolve_material_decision(*query).material_id == Some(entry.id));
        let roles = query.map(|query| terrain.resolve_material_render_layers(query));
        let roles = roles.map(|layers| layers.map(|layer| layer.map(|layer| {
            let slot = layer.runtime_slot_id.and_then(|id| terrain.present_slot(id));
            json!({
                "layer_index": layer.layer_index,
                "texture_id": layer.texture_id,
                "repetition": layer.texture_repetition,
                "slot": slot.map(|slot| json!({"id":slot.art_id,
                    "page":slot.page_index, "extent":[slot.width,slot.height]})),
            })
        })));
        materials.push(json!({"material":entry, "query":query, "render_roles":roles}));
    }
    let output = OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    serde_json::to_writer_pretty(output, &json!({
        "package":args.package, "loose_overrides":args.overrides,
        "role_order":["base","detail","mask","normal","animated"],
        "scope":"resolved slots and authored inputs, not proof of texture semantics or visible fragments",
        "materials":materials,
    }))?;
    println!("saved terrain layer inventory to {:?}", args.output);
    Ok(())
}
