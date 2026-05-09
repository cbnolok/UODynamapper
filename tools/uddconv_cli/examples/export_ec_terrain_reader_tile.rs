use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::terrain::TerrainReader;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let ecdir = PathBuf::from(
        args.next().ok_or_else(|| {
            eyre::eyre!(
                "usage: export_ec_terrain_reader_tile <ecdir> <tile_id> <output.png> [--legacy]"
            )
        })?,
    );
    let tile_id: u32 = args
        .next()
        .ok_or_else(|| eyre::eyre!("missing tile_id"))?
        .to_string_lossy()
        .parse()
        .wrap_err("parse tile_id")?;
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing output path"))?,
    );
    let use_legacy = args.next().is_some_and(|arg| arg == "--legacy");

    let reader = TerrainReader::new(
        ecdir.join("Texture.uop")
            .to_str()
            .ok_or_else(|| eyre::eyre!("invalid Texture.uop path"))?,
        ecdir.join("LegacyTexture.uop")
            .to_str()
            .ok_or_else(|| eyre::eyre!("invalid LegacyTexture.uop path"))?,
    )?;

    let file = reader
        .get_terrain_texture(tile_id, use_legacy)?
        .ok_or_else(|| eyre::eyre!("terrain texture {tile_id} not found"))?;

    let image = file.decode_to_rgba()?;
    image
        .save(&output)
        .wrap_err_with(|| format!("save {}", output.display()))?;

    println!(
        "exported terrain texture {} ({}) as {}x{} {:?} -> {}",
        tile_id,
        if use_legacy { "legacy" } else { "world" },
        image.width(),
        image.height(),
        file.format,
        output.display()
    );

    Ok(())
}
