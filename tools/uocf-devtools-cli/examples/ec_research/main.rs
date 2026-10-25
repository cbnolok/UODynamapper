//! Read original EC liquid graphs or export their images using the production readers.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, ensure};
use uocf::enhanced::{terrain_definition::TerrainDefinitionPackage, textures::Textures};

#[derive(Parser)]
struct Args {
    /// Matching original-client bundle containing the UOP files.
    #[arg(long)]
    client: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print source material layers, Technique and Tweaks. With no IDs, list liquids.
    Materials { ids: Vec<u32> },
    /// Print original item shader blocks, parameters, sampler refs and art windows.
    Items {
        #[arg(required = true)]
        ids: Vec<u32>,
    },
    /// Export original DDS payloads and decoded PNGs; refuses existing outputs.
    Images {
        #[arg(long)]
        output: PathBuf,
        #[arg(required = true)]
        ids: Vec<u32>,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Materials { ids } => {
            let package = TerrainDefinitionPackage::load_with_dictionary(
                &args.client.join("TerrainDefinition.uop"),
                &args.client.join("string_dictionary.uop"),
            )?;
            for entry in package.entries {
                let Some(texture) = entry.texture else { continue; };
                let selected = if ids.is_empty() {
                    texture.shader_name.as_deref() == Some("UOWaterTerrainLayer")
                } else { ids.contains(&entry.id) };
                if !selected { continue; }
                println!("material={} name={:?} shader={:?} Technique={:?} Tweaks={:?}",
                    entry.id, entry.name, texture.shader_name, texture.unk8, texture.unk9);
                for (index, layer) in texture.layers.iter().enumerate() {
                    println!("  source={index} id={:?} path={:?} repetition={} fields={}/{}/{}",
                        layer.texture_id, layer.path, layer.texture_repetition,
                        layer.unk4, layer.unk6, layer.unk7);
                }
            }
        }
        Command::Items { ids } => {
            let dictionary = uocf::enhanced::string_dictionary::UoStringDictionary::load(
                &args.client.join("string_dictionary.uop"),
            )?;
            let package = uocf::uop_container::package::UopPackage::load(args.client.join("tileart.uop"))?;
            for file in package.iter_files() {
                let entry = uocf::enhanced::tileart::TileArtEntry::parse_raw(file)?;
                if !ids.contains(&entry.tile_id) { continue; }
                let art = entry.process(&dictionary);
                println!("item={} type={:?} flags={:?} ec_window={:?}",
                    entry.tile_id, art.tile_type, art.flags, art.ec_texture);
                for (index, block) in entry.texture_vector.iter().enumerate() {
                    if block.has_texture == 0 { continue; }
                    let shader = block.type_string_off.checked_sub(1)
                        .and_then(|offset| dictionary.get_string(offset as usize));
                    println!("  block={index} shader={shader:?} Technique={:?} Tweaks={:?}",
                        block.unk8_vector, block.unk9_vector);
                }
                for reference in art.texture_items.iter().flatten() {
                    println!("  block={} source={} sampler={} id={} path={:?} repetition={} fields={}/{}",
                        reference.block_index, reference.item_index, reference.unk6,
                        reference.id, reference.raw_path, reference.texture_stretch,
                        reference.unk4, reference.unk7);
                }
            }
        }
        Command::Images { output, ids } => {
            let textures = Textures::new(&args.client.join("Texture.uop"), None, None)?;
            std::fs::create_dir_all(&output)?;
            for id in ids {
                let dds_path = output.join(format!("source-{id}.dds"));
                let png_path = output.join(format!("source-{id}.png"));
                ensure!(!dds_path.exists() && !png_path.exists(), "output already exists for {id}");
                let file = textures.get_from_id(id)?
                    .ok_or_else(|| color_eyre::eyre::eyre!("missing source image {id}"))?;
                let image = file.decode_to_rgba8()?;
                let alpha_min = image.pixels().map(|pixel| pixel[3]).min().unwrap_or(0);
                let alpha_max = image.pixels().map(|pixel| pixel[3]).max().unwrap_or(0);
                std::fs::write(&dds_path, file.raw_bytes())?;
                image.save(&png_path)?;
                println!("image={id} size={}x{} alpha={alpha_min}..{alpha_max}",
                    image.width(), image.height());
            }
        }
    }
    Ok(())
}
