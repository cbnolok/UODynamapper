use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let ecdir = PathBuf::from(args.next().ok_or_else(|| {
        eyre::eyre!(
            "usage: find_terrain_definition_layers_by_texture <ecdir> <texture_id> [<texture_id> ...]"
        )
    })?);
    let texture_ids = args
        .map(|value| {
            value
                .to_string_lossy()
                .parse::<u32>()
                .wrap_err("parse texture_id")
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if texture_ids.is_empty() {
        eyre::bail!("missing texture_id");
    }

    let terrain_definition_path = ecdir.join("TerrainDefinition.uop");
    let string_dictionary_path = ecdir.join("string_dictionary.uop");
    let package = if string_dictionary_path.exists() {
        TerrainDefinitionPackage::load_with_dictionary(
            &terrain_definition_path,
            &string_dictionary_path,
        )?
    } else {
        TerrainDefinitionPackage::load(&terrain_definition_path)?
    };

    for texture_id in texture_ids {
        println!("texture_id={texture_id}");
        let mut matches = 0usize;

        for entry in &package.entries {
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

            matches += 1;
            println!("  material_id={}", entry.id);
            println!("    name={:?}", entry.name);
            println!("    primary_texture_id={:?}", entry.primary_texture_id());
            println!("    aliases={:?}", entry.aliases.iter().map(|alias| alias.alias).collect::<Vec<_>>());
            println!("    shader_name={:?}", texture.shader_name);
            for (index, layer) in matching_layers {
                println!(
                    "    layer[{}]: path={:?} repetition={} unk4={} unk6={} unk7={}",
                    index,
                    layer.path,
                    layer.texture_repetition,
                    layer.unk4,
                    layer.unk6,
                    layer.unk7
                );
            }
        }

        if matches == 0 {
            println!("  <no matches>");
        }
    }

    Ok(())
}
