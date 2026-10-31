//! Rebuild matching EC art/metadata without regenerating unrelated terrain atlases.
use std::path::PathBuf;
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_conv::{tex_art_ec::{load_tex_art_ec_sources,
    convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources, TexArtEcAtlasOptions},
    tilemeta::{build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources, TileMetaBuildOptions}};
use udd_container::CompressionFlag;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    ec_client: PathBuf,
    #[arg(long)]
    cc_client: PathBuf,
    /// New directory; existing assets are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output directory already exists: {:?}", args.output);
    let sources = load_tex_art_ec_sources(&[args.ec_client.clone()])?;
    std::fs::create_dir_all(&args.output)?;
    let options = TexArtEcAtlasOptions {
        compression: CompressionFlag::ZstdNoDict, ..TexArtEcAtlasOptions::default()
    };
    let art_summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(&sources,
        &args.output.join("tex_art_ec.uddp"), &options)?;
    build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
        &args.cc_client, &sources, &art_summary.ec_crop_adjustments, &args.output.join("tilemeta.uddp"),
        &TileMetaBuildOptions::default())?;
    let art = udd_assets::tex_art_ec::TexArtEcPackage::load(args.output.join("tex_art_ec.uddp"))?;
    let meta = udd_assets::tilemeta::TileMetaPackage::load(args.output.join("tilemeta.uddp"))?;
    let present = art.slots().iter().filter(|slot| slot.is_present()).count();
    for slot in art.slots().iter().filter(|slot| slot.is_present()) {
        ensure!(meta.item_tile(slot.art_id).is_some(), "art {} missing item metadata", slot.art_id);
        ensure!(art.sprite_geometry(slot.art_id).is_some(), "art {} missing source geometry", slot.art_id);
    }
    println!("Validated {present} packed sprite owners with matching item metadata in {}", args.output.display());
    Ok(())
}
