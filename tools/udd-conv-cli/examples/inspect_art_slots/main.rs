//! Inspect packed sprite dimensions and the metadata used by renderer routing.
use std::path::PathBuf;
use clap::Parser;
use color_eyre::eyre::Result;
use udd_assets::{tex_art_cc::TexArtCcPackage, tex_art_ec::TexArtEcPackage, tilemeta::TileMetaPackage};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    packages: PathBuf,
    #[arg(required = true)]
    ids: Vec<u32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let cc = TexArtCcPackage::load(args.packages.join("tex_art_cc.uddp"))?;
    let ec = TexArtEcPackage::load(args.packages.join("tex_art_ec.uddp"))?;
    let meta = TileMetaPackage::load(args.packages.join("tilemeta.uddp"))?;
    for id in args.ids {
        println!("item={id} metadata={:?}", meta.item_tile(id));
        println!("  CC={:?}", cc.present_slot(id + 0x4000));
        println!("  EC={:?}", ec.present_slot(id));
    }
    Ok(())
}
