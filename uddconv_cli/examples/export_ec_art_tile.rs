use std::env;
use std::path::PathBuf;

use color_eyre::eyre::{self, WrapErr};
use uocf::enhanced::textures::Textures;

fn load_tile(ecdir: &PathBuf, tile_id: u32) -> eyre::Result<(String, uocf::enhanced::textures::TextureFile)> {
    for file_name in ["Texture.uop", "LegacyTexture.uop"] {
        let uop_path = ecdir.join(file_name);
        if !uop_path.exists() {
            continue;
        }

        let textures = Textures::new(&uop_path, None, None)
            .wrap_err_with(|| format!("load {}", uop_path.display()))?;
        if let Some(file) = textures.get_from_id(tile_id)? {
            return Ok((file_name.to_string(), file));
        }
    }

    eyre::bail!(
        "tile {tile_id} not found in {}/Texture.uop or {}/LegacyTexture.uop",
        ecdir.display(),
        ecdir.display()
    )
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut args = env::args_os().skip(1);
    let ecdir = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("usage: export_ec_art_tile <ecdir> <tile_id> <output.png>"))?,
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

    let (source_file, file) = load_tile(&ecdir, tile_id)?;

    let image = file.decode_to_rgba()?;
    image
        .save(&output)
        .wrap_err_with(|| format!("save {}", output.display()))?;

    println!(
        "exported tile {} from {} as {}x{} {:?} -> {}",
        tile_id,
        source_file,
        image.width(),
        image.height(),
        file.format,
        output.display()
    );

    Ok(())
}
