use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::tile_database::ArtDefinition;

fn find_string_dictionary_path(ecdir: &PathBuf) -> eyre::Result<PathBuf> {
    for file_name in ["string_dictionary.uop"] {
        let path = ecdir.join(file_name);
        if path.exists() {
            return Ok(path);
        }
    }

    eyre::bail!(
        "missing string_dictionary.uop in {}",
        ecdir.display()
    )
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let ecdir = PathBuf::from(
        args.next().ok_or_else(|| {
            eyre::eyre!(
                "usage: inspect_ec_tileart_entry <ecdir> <art_id> [art_id ...]"
            )
        })?,
    );
    let tileart_path = ecdir.join("tileart.uop");
    let stringdict_path = find_string_dictionary_path(&ecdir)?;

    let art_ids = args
        .map(|arg| {
            arg.to_string_lossy()
                .parse::<u16>()
                .wrap_err("parse art_id")
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if art_ids.is_empty() {
        eyre::bail!("usage: inspect_ec_tileart_entry <ecdir> <art_id> [art_id ...]");
    }

    let art_definition = ArtDefinition::load(&tileart_path, &stringdict_path)
        .wrap_err("load tileart-driven art definition")?;

    for art_id in art_ids {
        println!("art_id={art_id}");
        match art_definition.definitions.get(&art_id) {
            Some(art_data) => {
                println!("  tile_type={:?}", art_data.tile_type);
                println!("  flags={:?}", art_data.flags);
                println!("  flags_bits=0x{:x}", art_data.flags.bits());
                println!("  height={}", art_data.height);
                println!("  ec_texture={:?}", art_data.ec_texture);
                println!("  cc_texture={:?}", art_data.cc_texture);
                for (block_index, block) in art_data.texture_items.iter().enumerate() {
                    println!("  texture_block[{block_index}]");
                    for (item_index, item) in block.iter().enumerate() {
                        println!(
                            "    item[{item_index}] type={:?} id={} path={}",
                            item.texture_type, item.id, item.path
                        );
                    }
                }
            }
            None => {
                println!("  missing");
            }
        }
    }

    Ok(())
}
