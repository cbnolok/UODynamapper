use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::Parser;
use color_eyre::eyre::{self, WrapErr};
use udd_assets::{
    tex_land_ec::TexLandEcPackage,
    tilemeta::TileMetaPackage,
};
use uocf::enhanced::terrain_definition::{
    TerrainDefinitionEntry, TerrainDefinitionPackage, TerrainDefinitionTextureLayer,
};
use uocf::enhanced::tile_database::ArtDefinition;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    ecdir: PathBuf,
    #[arg(long)]
    tilemeta: Option<PathBuf>,
    #[arg(long = "ec-land")]
    tex_land_ec: Option<PathBuf>,
    #[arg(required = true)]
    art_ids: Vec<u16>,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let tileart_path = args.ecdir.join("tileart.uop");
    let stringdict_path = find_string_dictionary_path(&args.ecdir)?;
    let terrain_definition_path = args.ecdir.join("TerrainDefinition.uop");

    let art_definition = ArtDefinition::load(&tileart_path, &stringdict_path)
        .wrap_err("load tileart-driven art definition")?;
    let terrain_definition = TerrainDefinitionPackage::load_with_dictionary(
        &terrain_definition_path,
        &stringdict_path,
    )
    .wrap_err("load TerrainDefinition.uop")?;
    let tilemeta = args
        .tilemeta
        .as_ref()
        .map(TileMetaPackage::load)
        .transpose()
        .wrap_err("load tilemeta package")?;
    let tex_land_ec = args
        .tex_land_ec
        .as_ref()
        .map(TexLandEcPackage::load)
        .transpose()
        .wrap_err("load tex_land_ec package")?;

    for art_id in args.art_ids {
        println!("art_id={art_id}");

        let Some(art_data) = art_definition.definitions.get(&art_id) else {
            println!("  tileart=missing\n");
            continue;
        };

        println!("  tile_type={:?}", art_data.tile_type);
        match art_data.ec_texture.as_ref() {
            Some(texture) => {
                println!(
                    "  ec_texture_id={} window=({}, {})..({}, {}) offset=({}, {})",
                    texture.texture_id,
                    texture.start_x,
                    texture.start_y,
                    texture.end_x,
                    texture.end_y,
                    texture.offset_x,
                    texture.offset_y
                );

                if let Some(tilemeta) = tilemeta.as_ref().and_then(|package| package.item_tile(art_id as u32)) {
                    println!(
                        "  tilemeta.visual_kind={:?} surface_like={} cc_texture_id={} ec_texture_id={}",
                        tilemeta.visual_kind(),
                        tilemeta.is_surface_like(),
                        tilemeta.cc_texture_id,
                        tilemeta.ec_texture_id,
                    );
                }

                inspect_terrain_candidates(texture.texture_id, &terrain_definition, tex_land_ec.as_ref());
            }
            None => println!("  ec_texture=missing"),
        }

        println!();
    }

    Ok(())
}

fn find_string_dictionary_path(ecdir: &Path) -> eyre::Result<PathBuf> {
    let path = ecdir.join("string_dictionary.uop");
    if path.exists() {
        Ok(path)
    } else {
        eyre::bail!("missing string_dictionary.uop in {}", ecdir.display())
    }
}

fn inspect_terrain_candidates(
    texture_id: u32,
    terrain_definition: &TerrainDefinitionPackage,
    tex_land_ec: Option<&TexLandEcPackage>,
) {
    let mut material_matches = Vec::new();
    let mut primary_runtime_slots = BTreeSet::new();
    let mut any_runtime_slots = BTreeSet::new();

    for entry in &terrain_definition.entries {
        let Some(texture) = entry.texture.as_ref() else {
            continue;
        };

        let matching_layers = texture
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.texture_id == Some(texture_id))
            .collect::<Vec<_>>();
        if matching_layers.is_empty() {
            continue;
        }

        let primary_match = entry.primary_texture_id() == Some(texture_id);
        let runtime_slot_ids = entry.runtime_slot_ids();
        if let Some(tex_land_ec) = tex_land_ec {
            for runtime_slot_id in &runtime_slot_ids {
                if let Some(slot_id) = resolve_runtime_slot_for_material(tex_land_ec, entry, *runtime_slot_id) {
                    any_runtime_slots.insert(slot_id);
                    if primary_match {
                        primary_runtime_slots.insert(slot_id);
                    }
                }
            }
        }

        material_matches.push((entry, primary_match, matching_layers, runtime_slot_ids));
    }

    println!("  terrain_definition.material_matches={}", material_matches.len());
    if material_matches.is_empty() {
        println!("  terrain_candidate_verdict=no_match");
        return;
    }

    let primary_match_count = material_matches
        .iter()
        .filter(|(_, primary_match, _, _)| *primary_match)
        .count();
    println!("  terrain_definition.primary_matches={primary_match_count}");

    let verdict = if primary_match_count == 0 {
        format!(
            "layer_only_matches materials={} runtime_slots={}",
            material_matches.len(),
            any_runtime_slots.len()
        )
    } else if primary_runtime_slots.len() == 1 {
        format!(
            "unique_primary_runtime_slot slot={}",
            primary_runtime_slots.iter().next().copied().unwrap_or_default()
        )
    } else {
        format!(
            "ambiguous_primary_runtime_slots count={}",
            primary_runtime_slots.len()
        )
    };
    println!("  terrain_candidate_verdict={verdict}");

    for (entry, primary_match, matching_layers, runtime_slot_ids) in material_matches {
        println!("  material_id={}", entry.id);
        println!("    name={:?}", entry.name);
        println!("    shader_name={:?}", entry.texture.as_ref().and_then(|texture| texture.shader_name.as_deref()));
        println!("    primary_texture_id={:?}", entry.primary_texture_id());
        println!("    primary_match={primary_match}");
        println!("    runtime_slot_ids={runtime_slot_ids:?}");
        if let Some(tex_land_ec) = tex_land_ec {
            let resolved_slots = runtime_slot_ids
                .iter()
                .filter_map(|runtime_slot_id| resolve_runtime_slot_for_material(tex_land_ec, entry, *runtime_slot_id))
                .collect::<Vec<_>>();
            println!("    resolved_tex_land_ec_slots={resolved_slots:?}");
        }
        for (index, layer) in matching_layers {
            print_matching_layer(index, layer);
        }
    }
}

fn resolve_runtime_slot_for_material(
    tex_land_ec: &TexLandEcPackage,
    entry: &TerrainDefinitionEntry,
    runtime_slot_id: u32,
) -> Option<u32> {
    if tex_land_ec.present_slot(runtime_slot_id).is_some() {
        return Some(runtime_slot_id);
    }

    tex_land_ec
        .terrain_provenance()
        .iter()
        .filter(|record| record.material_id == entry.id && record.alias_slot_id == runtime_slot_id)
        .find_map(|record| {
            if record.canonical_slot_id != 0 && tex_land_ec.present_slot(record.canonical_slot_id).is_some() {
                Some(record.canonical_slot_id)
            } else if record.alias_slot_id != 0 && tex_land_ec.present_slot(record.alias_slot_id).is_some() {
                Some(record.alias_slot_id)
            } else {
                None
            }
        })
}

fn print_matching_layer(index: usize, layer: &TerrainDefinitionTextureLayer) {
    println!(
        "    layer[{index}] path={:?} repetition={} unk4={} unk6={} unk7={}",
        layer.path,
        layer.texture_repetition,
        layer.unk4,
        layer.unk6,
        layer.unk7,
    );
}
