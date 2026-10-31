//! Inspect item shader inputs against packed auxiliary pixels, without changing packages.

use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::{tex_aux_ec::TexAuxEcPackage, tilemeta::TileMetaPackage};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    tilemeta: PathBuf,
    #[arg(long)]
    aux: PathBuf,
    /// Export referenced packed images to a fresh directory.
    #[arg(long)]
    output: Option<PathBuf>,
    /// With no IDs, inspect every original UOWaterShader block.
    ids: Vec<u32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let tilemeta = TileMetaPackage::load(args.tilemeta)?;
    let aux = TexAuxEcPackage::load(args.aux)?;
    if let Some(output) = &args.output {
        std::fs::create_dir(output)?;
    }
    let mut exported = std::collections::HashSet::new();
    for tile in tilemeta.item_tiles() {
        for material in tilemeta.item_materials(tile.tile_id) {
            if (args.ids.is_empty() && material.shader_name.as_deref() != Some("UOWaterShader"))
                || (!args.ids.is_empty() && !args.ids.contains(&tile.tile_id)) {
                continue;
            }
            println!("item={} block={} Technique={:?} Tweaks={:?}",
                tile.tile_id, material.block_index, material.technique, material.tweaks);
            for reference in tilemeta.item_texture_refs(tile.tile_id).iter()
                .filter(|r| r.block_index == material.block_index) {
                let slot = aux.slot_for_texture_id(reference.texture_id);
                println!("  position={} role={:?} id={} family={:?} package={:?} repetition={} uv_set={} aux={slot:?}",
                    reference.item_index, reference.stable_role(), reference.texture_id, reference.logical_family(),
                    reference.physical_package(), reference.texture_stretch, reference.uv_set);
                if let (Some(slot), Some(output)) = (slot, &args.output) {
                    if !exported.insert(reference.texture_id) { continue; }
                    let rgba = aux.load_rgba(reference.texture_id)?.expect("present auxiliary pixels");
                    let image = image::RgbaImage::from_raw(slot.width.into(), slot.height.into(), rgba)
                        .ok_or_else(|| color_eyre::eyre::eyre!("invalid packed image extent"))?;
                    let path = output.join(format!("aux-{}.png", reference.texture_id));
                    ensure!(!path.exists(), "output already exists: {}", path.display());
                    image.save(path)?;
                }
            }
        }
    }
    Ok(())
}
