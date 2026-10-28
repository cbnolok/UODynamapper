//! Read original EC liquid graphs or export their images using the production readers.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, ensure};
use uocf::enhanced::{terrain_definition::TerrainDefinitionPackage, textures::Textures};
mod census;

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
        ids: Vec<u32>,
        /// Count shader families and sampler stages; omit IDs to inspect all items.
        #[arg(long)]
        summary: bool,
    },
    /// Export original DDS payloads and decoded PNGs; refuses existing outputs.
    Images {
        #[arg(long)]
        output: PathBuf,
        /// Physical archive; numeric IDs can refer to different images in each.
        #[arg(long, default_value = "Texture.uop")]
        archive: PathBuf,
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
                println!("  terrain_fields={}/{}/{}", entry.unk, entry.unk2, entry.unk3);
                for (index, layer) in texture.layers.iter().enumerate() {
                    println!("  source={index} id={:?} path={:?} repetition={} fields={}/{}/{}",
                        layer.texture_id, layer.path, layer.texture_repetition,
                        layer.unk4, layer.unk6, layer.unk7);
                }
            }
        }
        Command::Items { ids, summary } => {
            ensure!(summary || !ids.is_empty(), "supply item IDs or --summary");
            let mut census = census::ItemCensus::default();
            let dictionary = uocf::enhanced::string_dictionary::UoStringDictionary::load(
                &args.client.join("string_dictionary.uop"),
            )?;
            let package = uocf::uop_container::package::UopPackage::load(args.client.join("tileart.uop"))?;
            for file in package.iter_files() {
                let entry = uocf::enhanced::tileart::TileArtEntry::parse_raw(file)?;
                if !ids.is_empty() && !ids.contains(&entry.tile_id) { continue; }
                let art = entry.process(&dictionary);
                if summary {
                    census.record(&art);
                    continue;
                }
                println!("item={} type={:?} flags={:?} ec_window={:?}",
                    entry.tile_id, art.tile_type, art.flags, art.ec_texture);
                for (index, block) in entry.texture_vector.iter().enumerate() {
                    if block.has_texture == 0 { continue; }
                    let shader = block.type_string_off.checked_sub(1)
                        .and_then(|offset| dictionary.get_string(offset as usize));
                    println!("  block={index} shader={shader:?} Technique={:?} Tweaks={:?}",
                        block.technique, block.tweaks);
                }
                for reference in art.texture_items.iter().flatten() {
                    println!("  block={} source={} sampler={} id={} path={:?} repetition={} source_kind={} uv_set={}",
                        reference.block_index, reference.item_index, reference.sampler_index,
                        reference.id, reference.raw_path, reference.texture_stretch,
                        reference.source_kind, reference.uv_set);
                }
            }
            if summary { census.print(); }
        }
        Command::Images { output, archive, ids } => {
            let textures = Textures::new(&args.client.join(archive), None, None)?;
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
