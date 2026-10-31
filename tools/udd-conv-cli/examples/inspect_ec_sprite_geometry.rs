//! Inventory original sprite descriptors alongside packed EC image windows.

use std::{collections::BTreeMap, fs::OpenOptions, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use serde_json::json;
use udd_assets::tex_art_ec::TexArtEcPackage;
use uocf::{enhanced::{string_dictionary::UoStringDictionary, tile_database::ArtDefinition,
    tileart::{TaeFlag, TileArtEntry, tileart_entry_hash}},
    uop_container::package::UopPackage};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    client: PathBuf,
    #[arg(long)]
    package: PathBuf,
    /// Write a new evidence file; existing output is never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Compare lossless, untrimmed, native-size sprite pixels with original source windows.
    #[arg(long)]
    verify_pixels: Vec<u16>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "output already exists: {:?}", args.output);
    let package = UopPackage::load(args.client.join("tileart.uop"))?;
    let strings = UoStringDictionary::load(&args.client.join("string_dictionary.uop"))?;
    let packed = TexArtEcPackage::load(&args.package)?;
    let catalog = ArtDefinition::from_package(&package, &strings)?;
    let asset_ids: BTreeMap<_, _> = (u16::MIN..=u16::MAX)
        .map(|id| (tileart_entry_hash(u32::from(id)), id)).collect();
    let mut rows = Vec::new();
    let mut stored_ids = BTreeMap::<u32, usize>::new();
    let mut world_scales = BTreeMap::<String, usize>::new();
    let mut legacy_scales = BTreeMap::<String, usize>::new();
    let mut packed_families = BTreeMap::<String, usize>::new();
    let mut modes = [0usize; 3];
    for file in package.iter_files().filter(|file| file.has_size()) {
        let entry = TileArtEntry::parse_raw(file)?;
        *stored_ids.entry(entry.tile_id).or_default() += 1;
        let owner = asset_ids.get(&file.filename_hash()).copied();
        if let Some(id) = owner {
            ensure!(catalog.definitions[&id].source_tile_id == entry.tile_id,
                "catalog source differs for named asset {id}");
        }
        ensure!(entry.world_art_scale.is_finite() && entry.legacy_art_scale.is_finite(),
            "nonfinite sprite scale on tile {}", entry.tile_id);
        let art = entry.process(&strings);
        let mode = if entry.flags2.contains(TaeFlag::UseNewArt) { 1 }
            else if entry.uses_enhanced_tileart { 2 } else { 0 };
        modes[mode] += 1;
        *world_scales.entry(entry.world_art_scale.to_string()).or_default() += 1;
        *legacy_scales.entry(entry.legacy_art_scale.to_string()).or_default() += 1;
        let selected = art.ec_texture.as_ref().map(|texture| ("world", entry.world_art_scale, texture))
            .or_else(|| art.cc_texture.as_ref().map(|texture| ("legacy", entry.legacy_art_scale, texture)));
        let slot = owner.and_then(|id| packed.present_slot(u32::from(id)));
        let family = if entry.flags1.contains(TaeFlag::Rotated) { "camera_quad" }
            else if matches!(art.tile_type, uocf::enhanced::tileart::TileType::Static) { "sprite" }
            else { "material_surface" };
        if slot.is_some() {
            let source = selected.map(|value| value.0).unwrap_or("no_selected_source");
            *packed_families.entry(format!("{family}/{source}")).or_default() += 1;
        }
        rows.push(json!({
            "tile_id":entry.tile_id, "owner_id":owner,
            "entry_hash":format!("{:016x}", file.filename_hash()),
            "has_world_art":entry.has_world_art,
            "uses_enhanced_tileart":entry.uses_enhanced_tileart,
            "world_art_scale":entry.world_art_scale, "legacy_art_scale":entry.legacy_art_scale,
            "flags1":format!("0x{:016x}", entry.flags1.bits()),
            "flags2":format!("0x{:016x}", entry.flags2.bits()),
            "original_nonlegacy_descriptor_mode":mode, "family":family,
            "world_window":[entry.ec_img_offset.x_start, entry.ec_img_offset.y_start,
                entry.ec_img_offset.x_end, entry.ec_img_offset.y_end,
                entry.ec_img_offset.x_off, entry.ec_img_offset.y_off],
            "legacy_window":[entry.cc_img_offset.x_start, entry.cc_img_offset.y_start,
                entry.cc_img_offset.x_end, entry.cc_img_offset.y_end,
                entry.cc_img_offset.x_off, entry.cc_img_offset.y_off],
            "converter_selected":selected.map(|(source, scale, texture)| json!({
                "source":source, "texture_id":texture.texture_id, "descriptor_scale":scale,
                "source_window":[texture.start_x,texture.start_y,texture.end_x,texture.end_y],
                "draw_offset":[texture.offset_x,texture.offset_y],
                "source_pixel_ratio_to_default_legacy":scale / 1.5,
            })),
            "packed_slot":slot.map(|slot| json!({
                "page":slot.page_index, "pixel_extent":[slot.width,slot.height],
                "logical_extent":[slot.logical_width(),slot.logical_height()],
                "draw_offset":[slot.draw_offset_x,slot.draw_offset_y],
                "upscale_factor":slot.upscale_factor,
            })),
            "packed_geometry":owner.and_then(|id| packed.sprite_geometry(u32::from(id))),
        }));
    }
    rows.sort_by_key(|row| (row["owner_id"].as_u64(), row["tile_id"].as_u64()));
    let summary = json!({"records":rows.len(), "original_nonlegacy_descriptor_modes":modes,
        "named_static_owners":catalog.definitions.len(),
        "duplicate_stored_ids":stored_ids.iter().filter(|(_, count)| **count > 1)
            .map(|(id, count)| json!({"stored_id":id, "records":count})).collect::<Vec<_>>(),
        "named_static_stored_id_mismatches":rows.iter().filter(|row|
            row["owner_id"].as_u64().is_some_and(|owner| Some(owner) != row["tile_id"].as_u64())).count(),
        "world_scale_counts":world_scales, "legacy_scale_counts":legacy_scales,
        "packed_family_source_counts":packed_families});
    let mut pixel_checks = Vec::new();
    for id in &args.verify_pixels {
        let slot = packed.present_slot(u32::from(*id)).ok_or_else(|| color_eyre::eyre::eyre!("missing sprite {id}"))?;
        ensure!(slot.is_static() && slot.upscale_factor == 1, "pixel check requires a native ordinary sprite: {id}");
        let geometry = packed.sprite_geometry(u32::from(*id)).unwrap();
        let filename = if geometry.source_is_world_art { "Texture.uop" } else { "LegacyTexture.uop" };
        let textures = uocf::enhanced::textures::Textures::new(&args.client.join(filename), None, None)?;
        let source = textures.get_from_id(geometry.texture_id)?.ok_or_else(|| color_eyre::eyre::eyre!("missing source image {id}"))?.decode_to_rgba8()?;
        let window = if geometry.source_is_world_art { geometry.descriptor.world_window }
            else { geometry.descriptor.legacy_window };
        let [left, top, mut right, mut bottom, offset_x, offset_y] = window;
        ensure!(right > left && bottom > top, "degenerate authored window for {id}");
        if geometry.source_is_world_art { right = right.saturating_add(1); bottom = bottom.saturating_add(1); }
        let left = left.clamp(0, source.width() as i32) as u32;
        let top = top.clamp(0, source.height() as i32) as u32;
        let right = right.clamp(0, source.width() as i32) as u32;
        let bottom = bottom.clamp(0, source.height() as i32) as u32;
        ensure!(u32::from(slot.width) == right - left && u32::from(slot.height) == bottom - top,
            "packed dimensions do not match the untrimmed source window for {id}");
        ensure!(i32::from(slot.draw_offset_x) == offset_x && i32::from(slot.draw_offset_y) == offset_y,
            "packed offsets do not match untrimmed source for {id}");
        let pixels = packed.read_page_rgba(slot.page_index)?;
        let stride = packed.pages()[slot.page_index as usize].used_width as usize * 4;
        for y in 0..usize::from(slot.height) {
            let start = (usize::from(slot.y) + y) * stride + usize::from(slot.x) * 4;
            let actual = &pixels[start..start + usize::from(slot.width) * 4];
            let start = ((top as usize + y) * source.width() as usize + left as usize) * 4;
            let expected = &source.as_raw()[start..start + usize::from(slot.width) * 4];
            ensure!(actual == expected, "source pixel mismatch for sprite {id}, row {y}");
        }
        pixel_checks.push(json!({"owner":id,"source":filename,"texture":geometry.texture_id,
            "extent":[slot.width,slot.height],"exact_rgba_match":true}));
    }
    let output = OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    serde_json::to_writer_pretty(output, &json!({"client":args.client, "package":args.package,
        "scope":"Authored descriptors and packed slots; not proof of runtime routing, alpha trims or visible pixel parity.",
        "former_fixed_ec_pixel_ratio_to_44px_legacy":44.0 / 64.0,
        "summary":summary, "pixel_checks":pixel_checks, "records":rows}))?;
    println!("{summary}");
    Ok(())
}
