//! Inspect native image extents without exporting pixels or stopping at absent IDs.
use std::path::PathBuf;
use clap::Parser;
use color_eyre::eyre::Result;
use uocf::enhanced::textures::Textures;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    client: PathBuf,
    #[arg(long, default_value = "Texture.uop")]
    archive: PathBuf,
    #[arg(required = true)]
    ids: Vec<u32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let textures = Textures::new(&args.client.join(args.archive), None, None)?;
    println!("texture_id,status,width,height");
    for id in args.ids {
        match textures.get_from_id(id)? {
            Some(file) => {
                let image = file.decode_to_rgba8()?;
                println!("{id},present,{},{}", image.width(), image.height());
            }
            None => println!("{id},missing,0,0"),
        }
    }
    Ok(())
}
