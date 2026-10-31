//! Compare complete Classic atlas coverage and pixels without depending on packing order.
use std::{fs::OpenOptions, path::PathBuf};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use udd_assets::tex_art_cc::{TexArtCcPackage, TexArtCcSlotRecord};
use uocf::classic::art::{ArtMap, ArtSource};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    before: PathBuf,
    #[arg(long)]
    after: PathBuf,
    /// New JSON report. All present slots are checked, including land art.
    #[arg(long)]
    output: PathBuf,
    /// Accept changed old content only when every resulting slot matches this original Classic UOP.
    /// Requires native unscaled RGBA. Missing old slots still fail.
    #[arg(long)]
    classic_client: Option<PathBuf>,
}

fn geometry(slot: &TexArtCcSlotRecord) -> [i32; 7] {
    [slot.width.into(), slot.height.into(), slot.upscale_factor.into(),
        slot.upscale_algorithm.into(), slot.draw_offset_x.into(), slot.draw_offset_y.into(), slot.flags.into()]
}

fn main() -> Result<()> {
    let args = Args::parse();
    let output = OpenOptions::new().write(true).create_new(true).open(&args.output)?;
    let before = TexArtCcPackage::load(&args.before)?;
    let after = TexArtCcPackage::load(&args.after)?;
    // Decode each page once; per-slot get_pixel_data would repeatedly copy entire pages.
    let before_pixels = before.pages().iter().map(|p| before.read_page_rgba(p.page_index)).collect::<Result<Vec<_>>>()?;
    let after_pixels = after.pages().iter().map(|p| after.read_page_rgba(p.page_index)).collect::<Result<Vec<_>>>()?;
    let mut lost = Vec::new();
    let mut changed_geometry = Vec::new();
    let mut changed_pixels = Vec::new();
    let mut common = 0;
    for slot in before.slots().iter().filter(|s| s.is_present()) {
        let Some(next) = after.present_slot(slot.art_id) else { lost.push(slot.art_id); continue; };
        common += 1;
        if geometry(slot) != geometry(next) { changed_geometry.push(slot.art_id); }
        if (slot.width, slot.height) != (next.width, next.height) {
            changed_pixels.push(slot.art_id);
            continue;
        }
        let left = &before_pixels[slot.page_index as usize];
        let right = &after_pixels[next.page_index as usize];
        let left_stride = before.pages()[slot.page_index as usize].used_width as usize * 4;
        let right_stride = after.pages()[next.page_index as usize].used_width as usize * 4;
        let length = usize::from(slot.width) * 4;
        if (0..usize::from(slot.height)).any(|y| {
            let a = (usize::from(slot.y) + y) * left_stride + usize::from(slot.x) * 4;
            let b = (usize::from(next.y) + y) * right_stride + usize::from(next.x) * 4;
            left[a..a + length] != right[b..b + length]
        }) { changed_pixels.push(slot.art_id); }
    }
    let added: Vec<_> = after.slots().iter().filter(|s| s.is_present() && before.present_slot(s.art_id).is_none())
        .map(|s| s.art_id).collect();
    let mut source_checked = 0;
    let mut source_mismatches = Vec::new();
    if let Some(client) = &args.classic_client {
        let art = ArtMap::load(client)?;
        let mut scratch = Vec::new();
        for slot in after.slots().iter().filter(|s| s.is_present()) {
            let (width, height, original) = if slot.is_land() {
                let mut rgba = [0; 44 * 44 * 4];
                art.decode_land_tile_from_source(slot.art_id, ArtSource::CcUop, &mut scratch, &mut rgba)?;
                (44, 44, rgba.to_vec())
            } else { art.decode_static_tile_from_source(slot.art_id, ArtSource::CcUop, &mut scratch)? };
            source_checked += 1;
            let mut matches = (width, height) == (slot.width, slot.height) && slot.upscale_factor == 1;
            if slot.is_static() {
                matches &= slot.draw_offset_x == -(i32::from(width) / 2) as i16 && slot.draw_offset_y == 0;
            }
            if matches {
                let page = &after_pixels[slot.page_index as usize];
                let stride = after.pages()[slot.page_index as usize].used_width as usize * 4;
                let length = usize::from(width) * 4;
                matches = (0..usize::from(height)).all(|y| {
                    let at = (usize::from(slot.y) + y) * stride + usize::from(slot.x) * 4;
                    page[at..at + length] == original[y * length..(y + 1) * length]
                });
            }
            if !matches { source_mismatches.push(slot.art_id); }
        }
    }
    serde_json::to_writer_pretty(output, &serde_json::json!({
        "before": args.before, "after": args.after, "common": common,
        "added": added, "lost": lost, "changed_geometry": changed_geometry, "changed_pixels": changed_pixels,
        "classic_client": args.classic_client, "source_checked": source_checked, "source_mismatches": source_mismatches,
        "scope": "All present slot dimensions, offsets, flags and decoded RGBA; atlas packing coordinates may differ."
    }))?;
    println!("Common {common}; added {}; lost {}; changed geometry {}; changed pixels {}",
        added.len(), lost.len(), changed_geometry.len(), changed_pixels.len());
    println!("Original source checked {source_checked}; mismatches {}", source_mismatches.len());
    ensure!(lost.is_empty() && source_mismatches.is_empty()
        && (args.classic_client.is_some() || (changed_geometry.is_empty() && changed_pixels.is_empty())),
        "existing Classic content differs; inspect {}", args.output.display());
    Ok(())
}
