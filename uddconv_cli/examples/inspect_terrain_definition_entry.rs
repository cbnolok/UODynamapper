use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::terrain_definition::TerrainDefinitionPackage;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let ecdir = PathBuf::from(
        args.next().ok_or_else(|| {
            eyre::eyre!(
                "usage: inspect_terrain_definition_entry <ecdir> <material_id> [<material_id> ...]"
            )
        })?,
    );
    let material_ids = args
        .map(|value| {
            value
                .to_string_lossy()
                .parse::<u32>()
                .wrap_err("parse material_id")
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if material_ids.is_empty() {
        eyre::bail!("missing material_id");
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

    for material_id in material_ids {
        let Some(entry) = package.entries.iter().find(|entry| entry.id == material_id) else {
            println!("material_id={} <missing>", material_id);
            continue;
        };

        println!("material_id={}", entry.id);
        println!("  name_id={}", entry.name_id);
        println!("  name={:?}", entry.name);
        println!("  primary_texture_id={:?}", entry.primary_texture_id());
        println!("  aliases:");
        for alias in &entry.aliases {
            println!(
                "    count_index={} alias={} tile_flags={}",
                alias.count_index,
                alias.alias,
                alias.tile_flags
            );
        }
        match &entry.texture {
            Some(texture) => {
                println!("  shader_name_id={}", texture.shader_name_id);
                println!("  shader_name={:?}", texture.shader_name);
                println!("  layers:");
                for (index, layer) in texture.layers.iter().enumerate() {
                    println!(
                        "    [{}] path={:?} texture_id={:?} type={:?} repetition={} unk4={} unk6={} unk7={}",
                        index,
                        layer.path,
                        layer.texture_id,
                        layer.texture_type,
                        layer.texture_repetition,
                        layer.unk4,
                        layer.unk6,
                        layer.unk7
                    );
                }
            }
            None => println!("  texture=<none>"),
        }
    }

    Ok(())
}
