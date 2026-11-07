//! Composition equivalence check: the repacked version 4 gump packages against a dump of the installed
//! version 3 ones (`dump_old_gumps` of the dev tree).
//!
//! Usage: check_paperdoll_bake <old cc dump> <old ec dump> <new gumps_cc.uddp> <new gumps_ec.uddp>
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use clap::Args;

use udd_assets::gumps::{gump_image_hash, GumpImage, GumpsPackage};

type Result<T> = color_eyre::eyre::Result<T>;

const PAPERDOLL_BODY_GUMPS: [u32; 6] = [12, 13, 14, 15, 665, 666];
const EQUIPMENT_FIRST: u32 = 50_001;
const EQUIPMENT_LAST: u32 = 69_999;
const FEMALE_EQUIPMENT_FIRST: u32 = 60_000;
const MALE_BODY: u32 = 12;
const FEMALE_BODY: u32 = 13;
const SKIPPED_SIZE: u32 = 256;
const GLOW_DARK_MAX: u8 = 32;
const GLOW_MIN_TEXELS: usize = 1024;
const RGBA: usize = 4;
const RGB: usize = 3;
const REPORTED_EXAMPLES: usize = 12;

struct Raw {
    width: u32,
    height: u32,
    factor: u16,
    rgba: Vec<u8>,
}

struct OldIndex {
    gumps: HashMap<u32, (u32, u32, u16, u64, String)>,
    shared: HashMap<u32, u64>,
}

fn is_paperdoll(id: u32) -> bool {
    PAPERDOLL_BODY_GUMPS.contains(&id) || (EQUIPMENT_FIRST..=EQUIPMENT_LAST).contains(&id)
}

fn read_index(dir: &Path) -> Result<OldIndex> {
    let mut gumps = HashMap::new();
    let mut shared = HashMap::new();
    for line in std::fs::read_to_string(dir.join("index.tsv"))?.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields[0] == "shared" {
            shared.insert(fields[1].parse()?, fields[2].parse()?);
        } else {
            gumps.insert(
                fields[0].parse()?,
                (fields[1].parse()?, fields[2].parse()?, fields[3].parse()?, fields[4].parse()?, fields[5].to_string()),
            );
        }
    }
    Ok(OldIndex { gumps, shared })
}

fn read_raw(dir: &Path) -> Result<HashMap<u32, Raw>> {
    let bytes = std::fs::read(dir.join("paperdoll.bin"))?;
    let mut raws = HashMap::new();
    let mut at = 0;
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    while at < bytes.len() {
        let (id, width, height) = (u32_at(at), u32_at(at + 4), u32_at(at + 8));
        let factor = u16::from_le_bytes(bytes[at + 12..at + 14].try_into().unwrap());
        at += 14;
        let len = (width * height) as usize * RGBA;
        raws.insert(id, Raw { width, height, factor, rgba: bytes[at..at + len].to_vec() });
        at += len;
    }
    Ok(raws)
}

/// The glow texels of the bake: 8-connected regions of alpha-0, dark, non-key texels of at least 1024 texels.
fn glow(raw: &Raw) -> Vec<bool> {
    let (w, h) = (raw.width as usize, raw.height as usize);
    let key = &raw.rgba[..RGB.min(raw.rgba.len())];
    let dark: Vec<bool> = raw
        .rgba
        .chunks_exact(RGBA)
        .map(|t| t[3] == 0 && t[..RGB] != *key && t[..RGB].iter().all(|&c| c <= GLOW_DARK_MAX))
        .collect();
    let mut seen = vec![false; w * h];
    let mut out = vec![false; w * h];
    for start in 0..w * h {
        if !dark[start] || seen[start] {
            continue;
        }
        let mut region = vec![start];
        seen[start] = true;
        let mut i = 0;
        while i < region.len() {
            let (x, y) = ((region[i] % w) as i64, (region[i] / w) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let n = ny as usize * w + nx as usize;
                    if dark[n] && !seen[n] {
                        seen[n] = true;
                        region.push(n);
                    }
                }
            }
            i += 1;
        }
        if region.len() >= GLOW_MIN_TEXELS {
            for n in region {
                out[n] = true;
            }
        }
    }
    out
}

/// The dev tree's Enhanced compose: per layer, key on texel 0's RGB, write every other texel's RGB, then set the canvas
/// alpha from its colour.
fn old_compose(layers: &[&Raw], width: u32, height: u32) -> Vec<u8> {
    let mut canvas = vec![0u8; (width * height) as usize * RGBA];
    for layer in layers {
        if layer.width == SKIPPED_SIZE && layer.height == SKIPPED_SIZE {
            continue;
        }
        let key = layer.rgba[..RGB].to_vec();
        for y in 0..layer.height as usize {
            for x in 0..layer.width as usize {
                let s = (y * layer.width as usize + x) * RGBA;
                if layer.rgba[s..s + RGB] == key[..] {
                    continue;
                }
                let d = (y * width as usize + x) * RGBA;
                canvas[d..d + RGB].copy_from_slice(&layer.rgba[s..s + RGB]);
            }
        }
        for t in canvas.chunks_exact_mut(RGBA) {
            t[3] = if t[..RGB].iter().any(|&c| c != 0) { u8::MAX } else { 0 };
        }
    }
    canvas
}

/// The new compose: each layer's stored texels blended by alpha at their offset, premultiplied as Dynamapper's.
fn new_compose(layers: &[&GumpImage], width: u32, height: u32) -> Vec<u8> {
    let mut canvas = vec![0u8; (width * height) as usize * RGBA];
    for layer in layers {
        if layer.logical_width == SKIPPED_SIZE && layer.logical_height == SKIPPED_SIZE {
            continue;
        }
        let rect = layer.stored_rect;
        for y in 0..rect.height as usize {
            for x in 0..rect.width as usize {
                let s = (y * rect.width as usize + x) * RGBA;
                let src = &layer.stored_rgba[s..s + RGBA];
                let d = ((y + rect.y as usize) * width as usize + x + rect.x as usize) * RGBA;
                let alpha = u32::from(src[3]);
                for c in 0..RGBA {
                    let source = if c == 3 { u32::from(u8::MAX) } else { u32::from(src[c]) };
                    let dst = u32::from(canvas[d + c]);
                    canvas[d + c] = ((source * alpha + dst * (u32::from(u8::MAX) - alpha) + 127) / 255) as u8;
                }
            }
        }
    }
    canvas
}

fn validate_unscaled_canvas(id: u32, baseline: &Raw, candidate: &GumpImage) -> Result<()> {
    color_eyre::eyre::ensure!(baseline.factor == 1, "gump {id}: the baseline must be unscaled");
    color_eyre::eyre::ensure!(candidate.upscale_factor == 1, "gump {id}: the candidate must be unscaled");
    color_eyre::eyre::ensure!(
        (candidate.physical_width, candidate.physical_height) == (baseline.width, baseline.height),
        "gump {id}: physical dimensions differ"
    );
    color_eyre::eyre::ensure!(
        (candidate.logical_width, candidate.logical_height) == (baseline.width, baseline.height),
        "gump {id}: logical dimensions differ"
    );
    Ok(())
}

/// A known bake exception omits the item and leaves exactly the body underneath.
fn is_expected_omission(item: &GumpImage, x: usize, y: usize, composite: &[u8], body_only: &[u8]) -> bool {
    if composite != body_only {
        return false;
    }
    let rect = item.stored_rect;
    let Some(local_x) = x.checked_sub(rect.x as usize) else { return true };
    let Some(local_y) = y.checked_sub(rect.y as usize) else { return true };
    if local_x >= rect.width as usize || local_y >= rect.height as usize {
        return true;
    }
    item.stored_rgba[(local_y * rect.width as usize + local_x) * RGBA + 3] == 0
}

#[derive(Default)]
struct CompareStats {
    composites: usize,
    identical: usize,
    glow_texels: usize,
    black_hole_texels: usize,
    other_texels: usize,
    other_examples: Vec<String>,
    skipped_256: usize,
}

#[derive(Args, Debug)]
pub struct Arguments {
    /// Pre-bake Classic dump containing index.tsv and paperdoll.bin.
    pub classic_dump: PathBuf,
    /// Pre-bake Enhanced dump containing index.tsv and paperdoll.bin.
    pub enhanced_dump: PathBuf,
    pub classic_package: PathBuf,
    pub enhanced_package: PathBuf,
}

pub fn run(args: Arguments) -> Result<()> {
    let (old_cc_dir, old_ec_dir) = (args.classic_dump.as_path(), args.enhanced_dump.as_path());
    let new_cc = GumpsPackage::load(&args.classic_package)?;
    let new_ec = GumpsPackage::load(&args.enhanced_package)?;
    let mut failed = false;

    for (name, old_dir, new) in [("gumps_cc", old_cc_dir, &new_cc), ("gumps_ec", old_ec_dir, &new_ec)] {
        let old = read_index(old_dir)?;
        let old_ids: BTreeSet<u32> = old.gumps.keys().copied().collect();
        let new_ids: BTreeSet<u32> = new.available_gump_ids().into_iter().collect();
        println!(
            "{name}: ids old {} new {}, only old {:?}, only new {:?}",
            old_ids.len(),
            new_ids.len(),
            old_ids.difference(&new_ids).take(REPORTED_EXAMPLES).collect::<Vec<_>>(),
            new_ids.difference(&old_ids).take(REPORTED_EXAMPLES).collect::<Vec<_>>()
        );
        let new_shared: HashMap<u32, u64> = new.shared_classic_gumps().map(|g| (g.gump_id, g.image_hash)).collect();
        let old_resolved: BTreeSet<u32> = old_ids.iter().chain(old.shared.keys()).copied().collect();
        let new_resolved: BTreeSet<u32> = new_ids.iter().chain(new_shared.keys()).copied().collect();
        failed |= old_resolved != new_resolved;
        // The bake can move body gumps into the shared Classic table. Validate
        // the resolved images rather than requiring an identical storage split.
        for (&id, &hash) in &new_shared {
            color_eyre::eyre::ensure!(gump_image_hash(&new_cc.read_gump_image(id)?) == hash, "shared gump {id}: Classic hash mismatch");
        }
        let resolved_image = |id| -> Result<GumpImage> {
            if new_shared.contains_key(&id) {
                new_cc.read_gump_image(id)
            } else {
                new.read_gump_image(id)
            }
        };
        for (&id, &hash) in &old.shared {
            if !is_paperdoll(id) {
                failed |= gump_image_hash(&resolved_image(id)?) != hash;
            }
        }
        let shared_same = old.shared.iter().filter(|(id, hash)| new_shared.get(id) == Some(hash)).count();
        println!(
            "{name}: shared old {} new {}, same id and hash {shared_same}, only old {:?}, only new {:?}",
            old.shared.len(),
            new_shared.len(),
            old.shared.keys().filter(|id| !new_shared.contains_key(id)).collect::<BTreeSet<_>>(),
            new_shared.keys().filter(|id| !old.shared.contains_key(id)).collect::<BTreeSet<_>>()
        );
        let (mut same, mut differ, mut paperdoll_same, mut paperdoll_differ) = (0, Vec::new(), 0, Vec::new());
        for id in &old_ids {
            let image = resolved_image(*id)?;
            let hash_same = gump_image_hash(&image) == old.gumps[id].3;
            match (is_paperdoll(*id), hash_same) {
                (false, true) => same += 1,
                (false, false) => differ.push(*id),
                (true, true) => paperdoll_same += 1,
                (true, false) => paperdoll_differ.push(*id),
            }
        }
        println!("{name}: (a) non-paperdoll gumps identical {same}, differing {} {:?}", differ.len(), &differ[..differ.len().min(REPORTED_EXAMPLES)]);
        println!(
            "{name}: paperdoll gumps padded back identical {paperdoll_same}, differing {} {:?}",
            paperdoll_differ.len(),
            &paperdoll_differ[..paperdoll_differ.len().min(REPORTED_EXAMPLES)]
        );
        failed |= !differ.is_empty();
        let trimmed = new_ids.iter().filter(|id| new.atlas_slot(**id).is_some()).count();
        let trimmed_area: u64 = new.atlas_slots().iter().map(|s| u64::from(s.width) * u64::from(s.height)).sum();
        let canvas_area: u64 = new
            .atlas_slots()
            .iter()
            .map(|s| u64::from(s.canvas_width) * u64::from(s.canvas_height) * u64::from(s.upscale_factor).pow(2))
            .sum();
        println!(
            "{name}: atlas gumps {trimmed}, stored texels {trimmed_area} of {canvas_area} canvas texels, pages {}",
            new.atlas_pages().len()
        );
    }

    // (c) Every Enhanced paperdoll gump over its body: the dev compose of the installed package against the new one.
    let old_ec_raw = read_raw(old_ec_dir)?;
    let old_cc_raw = read_raw(old_cc_dir)?;
    let new_image = |id: u32| -> Result<GumpImage> {
        match new_ec.shared_classic_gump_hash(id) {
            Some(_) => new_cc.read_gump_image(id),
            None => new_ec.read_gump_image(id),
        }
    };
    let mut stats = CompareStats::default();
    let mut ids: Vec<u32> = old_ec_raw.keys().copied().collect();
    ids.sort();
    for id in ids {
        let item_old = &old_ec_raw[&id];
        let body_id = if PAPERDOLL_BODY_GUMPS.contains(&id) {
            None
        } else if id >= FEMALE_EQUIPMENT_FIRST {
            Some(FEMALE_BODY)
        } else {
            Some(MALE_BODY)
        };
        let body_old = body_id.map(|b| old_ec_raw.get(&b).or_else(|| old_cc_raw.get(&b)).expect("old body"));
        let item_new = new_image(id)?;
        let body_new = body_id.map(new_image).transpose()?;
        validate_unscaled_canvas(id, item_old, &item_new)?;
        if let (Some(body_id), Some(old), Some(new)) = (body_id, body_old, body_new.as_ref()) {
            validate_unscaled_canvas(body_id, old, new)?;
        }
        if item_old.width == SKIPPED_SIZE && item_old.height == SKIPPED_SIZE {
            stats.skipped_256 += 1;
        }
        let width = item_old.width.max(body_old.map_or(0, |b| b.width));
        let height = item_old.height.max(body_old.map_or(0, |b| b.height));
        let old_layers: Vec<&Raw> = body_old.into_iter().chain([item_old]).collect();
        let new_layers: Vec<&GumpImage> = body_new.as_ref().into_iter().chain([&item_new]).collect();
        let old = old_compose(&old_layers, width, height);
        let new = new_compose(&new_layers, width, height);
        stats.composites += 1;
        if old == new {
            stats.identical += 1;
            continue;
        }
        let glow_mask = glow(item_old);
        let body_layers: Vec<&GumpImage> = body_new.as_ref().into_iter().collect();
        let body_only = new_compose(&body_layers, width, height);
        let key = &item_old.rgba[..RGB];
        let mut other = 0;
        for (i, (o, n)) in old.chunks_exact(RGBA).zip(new.chunks_exact(RGBA)).enumerate() {
            if o == n {
                continue;
            }
            let (x, y) = (i % width as usize, i / width as usize);
            let item_texel = (x < item_old.width as usize && y < item_old.height as usize)
                .then(|| &item_old.rgba[(y * item_old.width as usize + x) * RGBA..][..RGBA]);
            let in_glow = item_texel.is_some() && glow_mask[y * item_old.width as usize + x];
            let black_hole = item_texel.is_some_and(|t| t[..RGB] != *key && t[..RGB] == [0, 0, 0]) && o == [0, 0, 0, 0];
            let omitted = (in_glow || black_hole)
                && is_expected_omission(&item_new, x, y, n, &body_only[i * RGBA..][..RGBA]);
            if in_glow && omitted {
                stats.glow_texels += 1;
            } else if black_hole && omitted {
                stats.black_hole_texels += 1;
            } else {
                other += 1;
                if stats.other_examples.len() < REPORTED_EXAMPLES {
                    stats.other_examples.push(format!("gump {id} at {x},{y}: old {o:?} new {n:?} item {item_texel:?}"));
                }
            }
        }
        stats.other_texels += other;
    }
    println!(
        "gumps_ec: (c) composites {}, identical {}, 256x256 skipped {}, differing texels: glow {}, drawn pure black \
         over the body {}, other {}",
        stats.composites, stats.identical, stats.skipped_256, stats.glow_texels, stats.black_hole_texels, stats.other_texels
    );
    for example in &stats.other_examples {
        println!("  {example}");
    }
    color_eyre::eyre::ensure!(!failed, "gump membership, sharing or non-paperdoll pixels differ");
    color_eyre::eyre::ensure!(stats.composites > 0, "no paperdoll composites were checked");
    color_eyre::eyre::ensure!(stats.other_texels == 0, "unexpected paperdoll composition differences");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::gumps::GumpRect;

    #[test]
    fn migration_rejects_scaled_or_resized_canvases() {
        let mut old = Raw { width: 2, height: 2, factor: 1, rgba: vec![0; 16] };
        let image = GumpImage::whole(2, 2, 1, old.rgba.clone());
        validate_unscaled_canvas(50_001, &old, &image).unwrap();
        old.factor = 2;
        assert!(validate_unscaled_canvas(50_001, &old, &image).is_err());
        old.factor = 1;
        let scaled = GumpImage::whole(2, 2, 2, old.rgba.clone());
        assert!(validate_unscaled_canvas(50_001, &old, &scaled).is_err());
        let mut resized = image.clone();
        resized.physical_width = 3;
        assert!(validate_unscaled_canvas(50_001, &old, &resized).is_err());
        resized = image;
        resized.logical_height = 1;
        assert!(validate_unscaled_canvas(50_001, &old, &resized).is_err());
    }

    #[test]
    fn glow_exception_requires_an_omitted_item_and_exact_body_pixels() {
        const WIDTH: u32 = 32;
        const HEIGHT: u32 = 33;
        let mut old = Raw { width: WIDTH, height: HEIGHT, factor: 1, rgba: [8, 8, 8, 0].repeat((WIDTH * HEIGHT) as usize) };
        old.rgba[..RGBA].copy_from_slice(&[255, 255, 255, 0]);
        assert!(glow(&old)[1]);
        let body = GumpImage::whole(WIDTH, HEIGHT, 1, [40, 80, 120, 255].repeat((WIDTH * HEIGHT) as usize));
        let omitted = GumpImage::whole(WIDTH, HEIGHT, 1, vec![0; old.rgba.len()]);
        let body_only = new_compose(&[&body], WIDTH, HEIGHT);
        let combined = new_compose(&[&body, &omitted], WIDTH, HEIGHT);
        assert!(is_expected_omission(&omitted, 1, 0, &combined[RGBA..2 * RGBA], &body_only[RGBA..2 * RGBA]));

        let mut corrupted = omitted.clone();
        corrupted.stored_rgba[RGBA..2 * RGBA].copy_from_slice(&[0, 255, 0, 255]);
        let combined = new_compose(&[&body, &corrupted], WIDTH, HEIGHT);
        assert!(!is_expected_omission(&corrupted, 1, 0, &combined[RGBA..2 * RGBA], &body_only[RGBA..2 * RGBA]));
        // Neither an opaque item that happens to match the body, nor corruption
        // elsewhere in the composite, is an omission.
        assert!(!is_expected_omission(&corrupted, 1, 0, &body_only[RGBA..2 * RGBA], &body_only[RGBA..2 * RGBA]));
        assert!(!is_expected_omission(&omitted, 1, 0, &[0, 255, 0, 255], &body_only[RGBA..2 * RGBA]));
    }

    #[test]
    fn texels_outside_trimmed_storage_are_omitted_only_when_the_body_matches() {
        let mut item = GumpImage::whole(2, 1, 1, vec![0; 8]);
        item.stored_rect = GumpRect { x: 1, y: 0, width: 1, height: 1 };
        item.stored_rgba = vec![40, 80, 120, 255];
        let body = [40, 80, 120, 255];
        assert!(is_expected_omission(&item, 0, 0, &body, &body));
        assert!(!is_expected_omission(&item, 0, 0, &[255, 0, 0, 255], &body));
        assert!(!is_expected_omission(&item, 1, 0, &body, &body));
    }
}
