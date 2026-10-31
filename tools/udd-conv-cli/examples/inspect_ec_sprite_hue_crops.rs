//! Audit the runtime UOSprite hue-mask eligibility and native crop bounds.

use std::{collections::{BTreeMap, BTreeSet, HashMap}, fs::OpenOptions, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use serde_json::json;
use udd_assets::{tex_art_ec::TexArtEcPackage, tex_aux_ec::TexAuxEcPackage,
    tilemeta::{EcMaterialPhysicalPackage, TileMetaItemTextureRef, TileMetaPackage}};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    tilemeta: PathBuf,
    #[arg(long)]
    art: PathBuf,
    #[arg(long)]
    aux: PathBuf,
    /// Check missing auxiliary images against the original Texture.uop.
    #[arg(long)]
    client: Option<PathBuf>,
    /// Require every image from an earlier auxiliary package to remain identical.
    #[arg(long)]
    compare_aux: Option<PathBuf>,
    /// A new report; existing evidence is never overwritten.
    #[arg(long)]
    output: PathBuf,
}

// Match ec_item_surface::{source,sprite_source}: the first supported material
// source wins, before applying the ordinary sprite's stricter UV constraints.
fn source(refs: &[TileMetaItemTextureRef], position: u8) -> Option<&TileMetaItemTextureRef> {
    refs.iter().find(|r| r.block_index == 0 && r.item_index == position && r.is_world_art()
        && r.physical_package() == EcMaterialPhysicalPackage::Texture
        && r.uv_set <= 1 && r.texture_stretch.is_finite() && r.texture_stretch > 0.0)
        .filter(|r| r.uv_set == 0 && r.texture_stretch == 1.0)
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let meta = TileMetaPackage::load(&args.tilemeta)?;
    let art = TexArtEcPackage::load(&args.art)?;
    let aux = TexAuxEcPackage::load(&args.aux)?;
    let original = args.client.as_ref().map(|p|
        uocf::enhanced::textures::Textures::new(&p.join("Texture.uop"), None, None)).transpose()?;
    let mut pixels = HashMap::new();
    let mut crops = BTreeSet::new();
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut registered = 0;
    let mut rows = Vec::new();
    for tile in meta.item_tiles() {
        if tile.tile_id >= 65_536 { continue; }
        if !meta.item_material(tile.tile_id, 0).is_some_and(|m|
            m.shader_name.as_deref() == Some("UOSpriteShader")) { continue; }
        let Some(slot) = art.present_slot(tile.tile_id).filter(|s| s.is_static()) else { continue; };
        let refs = meta.item_texture_refs(tile.tile_id);
        if !source(refs, 0).is_some_and(|r| r.is_primary_selected()
            && r.texture_id == tile.ec_texture_id) { continue; }
        registered += 1;
        if !refs.iter().any(|r| r.block_index == 0 && r.item_index == 2) { continue; }
        let mask = source(refs, 2);
        let upscale = slot.upscale_factor.max(1);
        let extent = [slot.width / upscale, slot.height / upscale];
        let start = [i32::from(tile.ec_start_x), i32::from(tile.ec_start_y)];
        let packed_mask = mask.and_then(|m| aux.slot_for_texture_id(m.texture_id));
        let reason = (|| -> Result<(), String> {
            let mask = mask.ok_or("unsupported_mask_reference")?;
            let packed = packed_mask.as_ref().ok_or("missing_auxiliary_texture")?;
            if packed.logical_family != mask.texture_type { return Err("logical_family_mismatch".into()); }
            if start.iter().any(|v| *v < 0) { return Err("negative_source_origin".into()); }
            if slot.width == 0 || slot.height == 0
                || slot.width % upscale != 0 || slot.height % upscale != 0 {
                return Err("non_native_extent".into());
            }
            if start[0] as u32 + u32::from(extent[0]) > u32::from(packed.width)
                || start[1] as u32 + u32::from(extent[1]) > u32::from(packed.height) {
                return Err("crop_outside_mask".into());
            }
            let bytes = pixels.entry(mask.texture_id).or_insert_with(|| aux.load_rgba(mask.texture_id)
                .map_err(|e| format!("decode_error: {e}")).and_then(|p| p.ok_or("missing_pixels".into())));
            let bytes = bytes.as_ref().map_err(Clone::clone)?;
            if bytes.len() != usize::from(packed.width) * usize::from(packed.height) * 4 {
                return Err("invalid_pixel_extent".into());
            }
            crops.insert((mask.texture_id, start, extent));
            Ok(())
        })().err().unwrap_or_else(|| "available".into());
        *reasons.entry(reason.clone()).or_default() += 1;
        let original_extent = if reason == "missing_auxiliary_texture" {
            if let (Some(original), Some(mask)) = (&original, mask) {
                original.get_from_id(mask.texture_id)?.map(|file| file.decode_to_rgba8()
                    .map(|image| [image.width(), image.height()])).transpose()?
            } else { None }
        } else { None };
        rows.push(json!({"owner":tile.tile_id,"mask":mask.map(|m|m.texture_id),
            "source_origin":start,"native_extent":extent,"status":reason,
            "mask_extent":packed_mask.map(|p|[p.width,p.height]),"missing_mask_original_extent":original_extent}));
    }
    let summary = json!({"registered_sprites":registered,"distinct_crops":crops.len(),"statuses":reasons});
    let retained_auxiliary_images = if let Some(path) = &args.compare_aux {
        let previous = TexAuxEcPackage::load(path)?;
        for before in previous.slots() {
            let after = aux.slot_for_texture_id(before.texture_id)
                .ok_or_else(|| color_eyre::eyre::eyre!("removed auxiliary image {}", before.texture_id))?;
            ensure!((before.width, before.height, before.logical_family, before.stable_role)
                == (after.width, after.height, after.logical_family, after.stable_role),
                "changed auxiliary descriptor {}", before.texture_id);
            ensure!(previous.load_rgba(before.texture_id)? == aux.load_rgba(before.texture_id)?,
                "changed auxiliary pixels {}", before.texture_id);
        }
        Some(previous.slots().len())
    } else { None };
    let output = OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    serde_json::to_writer_pretty(output, &json!({"tilemeta":args.tilemeta,"art":args.art,"aux":args.aux,"client":args.client,
        "scope":"Runtime binding predicates, decoded auxiliary presence and crop bounds; not rendered hue parity.",
        "summary":summary,"declared_masks":rows,"compare_aux":args.compare_aux,
        "retained_auxiliary_images":retained_auxiliary_images,"auxiliary_images":aux.slots().len()}))?;
    println!("{summary}");
    Ok(())
}
