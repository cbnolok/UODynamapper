//! Inspect original hue rows and export selected native payloads for comparison.
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use clap::Parser;
use color_eyre::eyre::Result;
use uocf::{enhanced::{hues::{EcHuePackage, decode_hue_image_to_rgba},
    textures::{TextureFile, TextureItem, ECImageFormat}}, uop_container::package::{LoadMode, UopPackage}};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    archive: PathBuf,
    #[arg(long)]
    output: PathBuf,
    ids: Vec<u16>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.output)?;
    let hues = EcHuePackage::load(&args.archive)?;
    let mut package = UopPackage::load_with_mode(&args.archive, LoadMode::Lazy)?;
    println!("atlas={:?} fixed_palette={:?} bitmaps={} names={}",
        hues.atlas_hash, hues.fixed_palette_hash, hues.bitmaps.len(), hues.names.len());
    for (name, hash) in [("atlas", hues.atlas_hash), ("fixed-palette", hues.fixed_palette_hash)] {
        let Some(bytes) = hash.map(|hash| package.unpack_file_by_hash_cached(hash)).transpose()?.flatten() else { continue; };
        std::fs::write(args.output.join(format!("{name}.dds")), &bytes)?;
        let texture = TextureFile { metadata: TextureItem::absent(), is_ec: true, format: ECImageFormat::DDS,
            props: None, raw_data: Arc::from(bytes), image_data_offset: 0 };
        let image = texture.decode_to_rgba8()?;
        println!("{name}: {}x{}", image.width(), image.height());
        image.save(args.output.join(format!("{name}.png")))?;
    }
    for id in args.ids {
        let Some(entry) = hues.bitmap_for_hue(id) else { continue; };
        let bytes = package.unpack_file_by_hash_cached(entry.filename_hash)?.expect("indexed hue bitmap");
        let (width, height, rgba) = decode_hue_image_to_rgba(&bytes)?;
        let row = &rgba[(height / 2 * width * 4) as usize..((height / 2 + 1) * width * 4) as usize];
        let colors: HashSet<&[u8]> = row.chunks_exact(4).collect();
        println!("hue={id} name={:?} {width}x{height} center-row-colors={}", hues.hue_name(id), colors.len());
        std::fs::write(args.output.join(format!("hue{id:04}.bmp")), bytes)?;
    }
    Ok(())
}
