//! Export original and packed CC sprites without guessing the source family.
use std::path::PathBuf;
use clap::Parser;
use color_eyre::eyre::{Result, ensure, ContextCompat};
use uocf::classic::{art::{ArtMap, ArtSource}, tiledata::TileData};
use udd_assets::tex_art_cc::TexArtCcPackage;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    client: PathBuf,
    #[arg(long)]
    package: PathBuf,
    #[arg(long, value_parser = ["mul", "uop"])]
    source: String,
    #[arg(long, required = true, num_args = 1..)]
    graphic: Vec<u16>,
    /// New output directory; original assets and existing evidence are untouched.
    #[arg(long)]
    output: PathBuf,
    /// Export available source pixels and report absent packed slots instead of stopping.
    #[arg(long)]
    allow_missing_packed: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let art = ArtMap::load(&args.client)?;
    let source = if args.source == "mul" { ArtSource::Mul } else { ArtSource::CcUop };
    let packed = TexArtCcPackage::load(&args.package)?;
    let tiledata = TileData::load(args.client.join("tiledata.mul"))?;
    std::fs::create_dir_all(&args.output)?;
    let mut rows = Vec::new();
    let mut scratch = Vec::new();
    for graphic in &args.graphic {
        let id = u32::from(*graphic) + 0x4000;
        let (width, height, original) = art.decode_static_tile_from_source(id, source, &mut scratch)?;
        image::save_buffer(args.output.join(format!("original-{graphic}.png")), &original,
            u32::from(width), u32::from(height), image::ColorType::Rgba8)?;
        let slot = packed.present_slot(id);
        if args.allow_missing_packed && slot.is_none() {
            rows.push(serde_json::json!({"graphic":graphic, "art_slot":id,
                "source_extent":[width,height], "packed_extent":null,
                "raw_classic_flags":tiledata.item_tiles().get(usize::from(*graphic)).map(|tile|tile.flags.internal_flags),
                "exact_rgba_match":false, "reason":"missing_packed_slot"}));
            continue;
        }
        let slot = slot.context("missing packed CC sprite")?;
        let pixels = packed.get_pixel_data(id).context("decode packed CC sprite")?;
        image::save_buffer(args.output.join(format!("packed-{graphic}.png")), &pixels,
            u32::from(slot.width), u32::from(slot.height), image::ColorType::Rgba8)?;
        let same_extent = (width, height) == (slot.width, slot.height);
        let maximum = same_extent.then(|| original.iter().zip(&pixels).map(|(a,b)| a.abs_diff(*b)).max().unwrap_or(0));
        rows.push(serde_json::json!({"graphic":graphic, "art_slot":id,
            "source_extent":[width,height], "packed_extent":[slot.width,slot.height],
            "upscale_factor":slot.upscale_factor,
            "raw_classic_flags":tiledata.item_tiles().get(usize::from(*graphic)).map(|tile|tile.flags.internal_flags),
            "exact_rgba_match":same_extent && original==pixels, "maximum_byte_difference":maximum}));
    }
    std::fs::write(args.output.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "client":args.client,"package":args.package,"source":args.source,"sprites":rows,
        "classic_item_metadata_count":tiledata.item_tiles().len(),
        "scope":"Original pixels and packed coverage; not a claim of rendered shader/color/depth parity."
    }))?)?;
    println!("Exported {} original/packed sprite pairs to {}", rows.len(), args.output.display());
    Ok(())
}
