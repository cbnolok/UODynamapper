//! Build-time support for the Kingdom Reborn items the Enhanced packages hold (`crate::kr_sections` nests them).
//!
//! Most of Kingdom Reborn's item world art is the Enhanced Client's: every entry of KR's `Texture.uop` is in the
//! Enhanced Client's under the same name, byte-identical or the same image re-encoded (the same extent, DDS format and
//! header, other DXT blocks), and most KR records sample it as the Enhanced records do. The Kingdom Reborn files are
//! read from a folder `crate::kr_delta` rebuilt, where every texture within [`KR_IMAGE_EQUIVALENCE_THRESHOLD`] of the
//! Enhanced one is the Enhanced texture already, so those measure a difference of 0. Only what differs is packed:
//! - [`pack_kr_art_and_tilemeta`] writes the Kingdom Reborn section of `tilemeta.uddp`: the `tilemeta.uddp` layout,
//!   Classic `tiledata.mul` with KR's `tileart.uop` (record version 3, its flags moved to the EC layout by `uocf`), and
//!   every KR tileart record byte for byte. Its sampling starts follow the trims of the art each item is drawn from:
//!   the KR art for the items the art section holds, the Enhanced art for the others. It also writes the section of
//!   `tex_art_ec.uddp`, an overlay over that package (`tex_art_ec::convert_tex_art_ec_overlay_from_loaded_sources`):
//!   the items whose world art differs from the Enhanced Client's ([`compare_world_art`]: the Enhanced Client lacks
//!   the item, its record names another texture, window, draw offset, kind or sprite descriptor, or the image differs
//!   by more than [`KR_IMAGE_EQUIVALENCE_THRESHOLD`]), lossless RGBA8888 with Zstd. It lists the other items with
//!   world art, drawn from `tex_art_ec.uddp`, with their image's measured difference. The items kept for another
//!   texture, window, kind or descriptor keep Kingdom Reborn's records, offsets and descriptors; their pixels are the
//!   Enhanced texture's where the Kingdom Reborn one only re-encoded it.
//! - [`pack_kr_aux`] writes the section of `tex_aux_ec.uddp`, an overlay over that package
//!   (`tex_aux_ec::pack_tex_aux_ec_overlay`): of KR's material inputs (masks, noise, water maps), its `missing.tga`,
//!   atmosphere maps and light textures, those whose image differs from the Enhanced Client's by more than
//!   [`KR_IMAGE_EQUIVALENCE_THRESHOLD`] (or that `tex_aux_ec` lacks), lossless RGBA8888 with Zstd. The others are read
//!   from `tex_aux_ec.uddp`; the overlay lists them with their measured difference.
//!
//! An overlay is nested in the package it is measured against, packed from the same Enhanced Client in the same run,
//! so it cannot be read over another one.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;

use crate::metadata::tilemeta::{
    build_item_texture_refs_and_lights, build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources,
    TileMetaBuildOptions,
};
use crate::textures::tex_art_ec::{
    compute_tex_art_ec_crop_adjustments_from_loaded_sources, convert_tex_art_ec_overlay_from_loaded_sources,
    load_tex_art_ec_sources, names_packed_world_art, primary_texture_stable_role, TexArtEcAtlasOptions, TexArtEcBuildSummary,
    TexArtEcLoadedSources,
};
use udd_assets::common::{EcEquivalentRecord, EcOverlayTable};
use uocf::enhanced::tileart::{ArtData, ArtTexture, TaeFlag, TileType};

use crate::textures::tex_aux_ec::{
    pack_tex_aux_ec_overlay, tex_aux_ec_texture_keys, TexAuxEcOverlayBase, TexAuxEcOverlaySummary, TexAuxEcPackOptions,
};

/// The largest [`image_difference`], in 8-bit levels, between a Kingdom Reborn image and the Enhanced Client's image
/// of the same name that still counts as the same image re-encoded. Of the 8,000-odd shared world-art textures, 98%
/// differ by under 5 levels, and side by side every one up to 8 differs only on DXT block edges and alpha fringes.
/// The KR textures drawn differently (the terrain and material textures: grass, dirt, forest, cobbles, cave floor)
/// differ by 19.7 to 127 levels across the whole image. Between 8 and about 28 some small or translucent sprites
/// (rune stones, fringed plants) are still re-encode noise, but no mean separates them from a real change, and the
/// few dozen kept cost little; so 8 is the lowest level that keeps every real change and skips all the noise below it.
pub const KR_IMAGE_EQUIVALENCE_THRESHOLD: f32 = 8.0;

/// RGBA8888 channels per texel.
const RGBA_CHANNELS: usize = 4;
/// Colour channels per texel, before alpha.
const COLOR_CHANNELS: usize = 3;

/// How far apart two decoded RGBA8888 images `(width, height, texels)` are, in 8-bit levels: the larger of the mean
/// absolute colour difference over the texels both draw (alpha above zero) and the mean absolute alpha difference over
/// all texels. `None` when their extents differ.
pub fn image_difference(a: (u32, u32, &[u8]), b: (u32, u32, &[u8])) -> Option<f32> {
    if (a.0, a.1) != (b.0, b.1) || a.2.len() != b.2.len() || a.2.is_empty() {
        return None;
    }
    let (mut color_sum, mut drawn, mut alpha_sum) = (0u64, 0u64, 0u64);
    for (x, y) in a.2.chunks_exact(RGBA_CHANNELS).zip(b.2.chunks_exact(RGBA_CHANNELS)) {
        alpha_sum += u64::from(x[COLOR_CHANNELS].abs_diff(y[COLOR_CHANNELS]));
        if x[COLOR_CHANNELS] > 0 && y[COLOR_CHANNELS] > 0 {
            drawn += 1;
            color_sum += (0..COLOR_CHANNELS).map(|channel| u64::from(x[channel].abs_diff(y[channel]))).sum::<u64>();
        }
    }
    let texels = (a.2.len() / RGBA_CHANNELS) as f64;
    let color = if drawn == 0 { 0.0 } else { color_sum as f64 / (drawn as f64 * COLOR_CHANNELS as f64) };
    Some(color.max(alpha_sum as f64 / texels) as f32)
}

/// Why a Kingdom Reborn item's world art cannot be drawn from the Enhanced Client's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldArtDifference {
    /// The Enhanced Client has no world art for the item that its art build packs.
    EnhancedLacksItem,
    /// The records name other textures.
    OtherTexture { kr: u32, ec: u32 },
    /// The same texture, with other sampling starts, ends or draw offsets (`[start_x, start_y, end_x, end_y, offset_x,
    /// offset_y]`).
    OtherGeometry { kr: [i32; 6], ec: [i32; 6] },
    /// Another tile type or rotation (which make the art slot land or static), or a liquid's primary-texture role
    /// (which decides whether it has a slot).
    OtherKind,
    /// Another authored sprite descriptor (scales and windows of the sprite geometry).
    OtherSpriteDescriptor,
    /// The same texture, whose image differs beyond the threshold (`None`: another extent, or one client lacks it).
    OtherImage { texture: u32, difference: Option<f32> },
}

impl WorldArtDifference {
    /// The variant's name, for the counts in the log.
    fn name(self) -> &'static str {
        match self {
            Self::EnhancedLacksItem => "Enhanced lacks the item",
            Self::OtherTexture { .. } => "other texture",
            Self::OtherGeometry { .. } => "other window or draw offset",
            Self::OtherKind => "other kind",
            Self::OtherSpriteDescriptor => "other sprite descriptor",
            Self::OtherImage { .. } => "other image",
        }
    }
}

/// [`compare_world_art`]'s result.
#[derive(Debug, Clone, Default)]
pub struct WorldArtComparison {
    /// Kingdom Reborn items with world art the art build packs.
    pub items: usize,
    /// Each world-art texture both clients name, with its [`image_difference`] (`None` as in `OtherImage`).
    pub textures: Vec<(u32, Option<f32>)>,
    /// The items whose world art differs, by item.
    pub differing: Vec<(u16, WorldArtDifference)>,
    /// The other items with world art, with their image's difference, by item.
    pub equivalents: Vec<EcEquivalentRecord>,
}

/// Compares each Kingdom Reborn item's world art with the Enhanced Client's: what decides its `tex_art_ec` slot (its
/// record's primary block-0 texture with its window and draw offset, its kind and sprite descriptor) and that
/// texture's image.
pub fn compare_world_art(
    kr: &TexArtEcLoadedSources,
    ec: &TexArtEcLoadedSources,
    max_difference: f32,
) -> WorldArtComparison {
    let mut comparison = WorldArtComparison::default();
    let mut shared = Vec::new();
    let mut items: Vec<_> = kr.art_definition.definitions.iter()
        .filter(|(_, data)| names_packed_world_art(data))
        .filter_map(|(&id, data)| data.ec_texture.as_ref().map(|texture| (id, texture)))
        .collect();
    items.sort_unstable_by_key(|(id, _)| *id);
    comparison.items = items.len();
    for &(id, kr_texture) in &items {
        let Some(ec_texture) = ec.art_definition.definitions.get(&id).filter(|data| names_packed_world_art(data))
            .and_then(|data| data.ec_texture.as_ref()) else {
            comparison.differing.push((id, WorldArtDifference::EnhancedLacksItem));
            continue;
        };
        let geometry = |t: &ArtTexture| [t.start_x, t.start_y, t.end_x, t.end_y, t.offset_x, t.offset_y];
        let (kr_data, ec_data) = (&kr.art_definition.definitions[&id], &ec.art_definition.definitions[&id]);
        let (kr_descriptor, ec_descriptor) =
            (kr.art_definition.sprite_descriptors.get(&id), ec.art_definition.sprite_descriptors.get(&id));
        if kr_texture.texture_id != ec_texture.texture_id {
            comparison.differing.push((id, WorldArtDifference::OtherTexture { kr: kr_texture.texture_id, ec: ec_texture.texture_id }));
        } else if geometry(kr_texture) != geometry(ec_texture) {
            comparison.differing.push((id, WorldArtDifference::OtherGeometry { kr: geometry(kr_texture), ec: geometry(ec_texture) }));
        } else if slot_kind(kr_data) != slot_kind(ec_data) {
            comparison.differing.push((id, WorldArtDifference::OtherKind));
        } else if kr_descriptor != ec_descriptor {
            comparison.differing.push((id, WorldArtDifference::OtherSpriteDescriptor));
        } else {
            shared.push((id, kr_texture.texture_id));
        }
    }
    let mut texture_ids: Vec<u32> = shared.iter().map(|&(_, texture)| texture).collect();
    texture_ids.sort_unstable();
    texture_ids.dedup();
    let decode = |sources: &TexArtEcLoadedSources, texture: u32| sources.world_textures.get_from_id(texture).ok().flatten()
        .and_then(|file| file.decode_to_rgba8().ok());
    comparison.textures = texture_ids.par_iter().map(|&texture| {
        let difference = decode(kr, texture).zip(decode(ec, texture)).and_then(|(a, b)| image_difference(
            (a.width(), a.height(), a.as_raw()), (b.width(), b.height(), b.as_raw())));
        (texture, difference)
    }).collect();
    let differences: HashMap<u32, Option<f32>> = comparison.textures.iter().copied().collect();
    for (id, texture) in shared {
        match differences[&texture] {
            Some(difference) if difference <= max_difference =>
                comparison.equivalents.push(EcEquivalentRecord { id: u32::from(id), difference }),
            difference => comparison.differing.push((id, WorldArtDifference::OtherImage { texture, difference })),
        }
    }
    comparison.differing.sort_unstable_by_key(|(id, _)| *id);
    comparison
}

/// What of a record, besides its texture, decides the kind of its `tex_art_ec` slot and whether it has one: its tile
/// type, its rotation and, for a liquid, its primary texture's role.
fn slot_kind(data: &ArtData) -> (TileType, bool, Option<u8>) {
    (data.tile_type, data.flags.contains(TaeFlag::Rotated),
        (data.tile_type == TileType::Liquid).then(|| primary_texture_stable_role(data) as u8))
}

/// The smallest difference, in 8-bit levels, of the world-art textures left to `tex_art_ec.uddp` the log lists one by
/// one (the thousands closer are counted only).
const WORLD_ART_LOGGED_DIFFERENCE: f32 = 1.0;

/// The upper bounds, in 8-bit levels, of the difference bins [`difference_histogram`] counts in, the last one open.
const DIFFERENCE_BIN_BOUNDS: [f32; 9] = [1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 40.0, f32::INFINITY];

/// Counts of `differences` by [`DIFFERENCE_BIN_BOUNDS`], then of those with no measure, for the log.
fn difference_histogram(differences: impl IntoIterator<Item = Option<f32>>) -> String {
    let mut counts = [0usize; DIFFERENCE_BIN_BOUNDS.len()];
    let mut unmeasured = 0;
    for difference in differences {
        match difference {
            Some(difference) => counts[DIFFERENCE_BIN_BOUNDS.iter().position(|&bound| difference < bound)
                .unwrap_or(DIFFERENCE_BIN_BOUNDS.len() - 1)] += 1,
            None => unmeasured += 1,
        }
    }
    let mut lower = 0.0;
    let mut bins = Vec::new();
    for (bound, count) in DIFFERENCE_BIN_BOUNDS.iter().zip(counts) {
        bins.push(if bound.is_finite() { format!("{lower}-{bound}: {count}") } else { format!("{lower}+: {count}") });
        lower = *bound;
    }
    format!("{}, unmeasured: {unmeasured}", bins.join(", "))
}

/// What [`pack_kr_art_and_tilemeta`] packed.
#[derive(Debug, Clone)]
pub struct KrArtBuildSummary {
    pub world_art: WorldArtComparison,
    pub art: TexArtEcBuildSummary,
}

/// Where [`pack_kr_art_and_tilemeta`] writes the Kingdom Reborn tile data: `out`, with the Classic `tiledata.mul` (and
/// `radarcol.mul`) of `cc_dir` as the Enhanced tilemeta takes them.
pub struct KrTilemetaOutput<'a> {
    pub cc_dir: &'a Path,
    pub out: &'a Path,
}

/// Writes the Kingdom Reborn item art overlay to `art_out` and, with `tilemeta`, the Kingdom Reborn tile data (see the
/// module documentation), from the Kingdom Reborn files in `kr_dir`, against `ec`, the Enhanced sources the Enhanced
/// art is packed from.
pub fn pack_kr_art_and_tilemeta(kr_dir: &Path, ec: &TexArtEcLoadedSources, art_out: &Path,
    tilemeta: Option<KrTilemetaOutput>) -> eyre::Result<KrArtBuildSummary> {
    info!("Converting the Kingdom Reborn items to {}{}", art_out.display(),
        tilemeta.as_ref().map(|tilemeta| format!(" and {}", tilemeta.out.display())).unwrap_or_default());
    let mut kr = load_tex_art_ec_sources(&[kr_dir.to_path_buf()])
        .wrap_err_with(|| format!("load the Kingdom Reborn tileart and textures from {}", kr_dir.display()))?;

    let world_art = compare_world_art(&kr, ec, KR_IMAGE_EQUIVALENCE_THRESHOLD);
    log_world_art(&world_art);

    // The overlay art: the KR art build of the differing items alone.
    let differing: HashSet<u16> = world_art.differing.iter().map(|&(id, _)| id).collect();
    let full_definition = kr.art_definition.clone();
    kr.art_definition.definitions.retain(|id, _| differing.contains(id));
    let art_table = EcOverlayTable { equivalents: world_art.equivalents.clone() };
    let art = convert_tex_art_ec_overlay_from_loaded_sources(&kr, art_out, &TexArtEcAtlasOptions::default(),
        &art_table);
    kr.art_definition = full_definition;
    let art = art?;

    if let Some(tilemeta) = tilemeta {
        // Each item's sampling starts follow the trims of the art it is drawn from.
        let ec_crop_adjustments = compute_tex_art_ec_crop_adjustments_from_loaded_sources(ec)?;
        let crop_adjustments: Vec<_> = ec_crop_adjustments.iter().zip(&art.ec_crop_adjustments).enumerate()
            .map(|(id, (&ec, &kr))| if u16::try_from(id).is_ok_and(|id| differing.contains(&id)) { kr } else { ec })
            .collect();
        build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
            tilemeta.cc_dir,
            &kr,
            &crop_adjustments,
            tilemeta.out,
            &TileMetaBuildOptions::default(),
        )?;
    }
    info!("Kingdom Reborn items: {} art slots on {} pages, {} items left to tex_art_ec.uddp", art.populated_slot_count,
        art.page_count, world_art.equivalents.len());
    Ok(KrArtBuildSummary { world_art, art })
}

/// Writes the Kingdom Reborn material texture overlay to `aux_out` (see the module documentation), from the Kingdom
/// Reborn files in `kr_dir` and their tile data `kr_tilemeta` ([`pack_kr_art_and_tilemeta`]), against the Enhanced
/// Client in `ec_dir` and the Classic `tiledata.mul` of `cc_dir` the Enhanced `tex_aux_ec.uddp` is packed from.
pub fn pack_kr_aux(kr_dir: &Path, ec_dir: &Path, cc_dir: &Path, kr_tilemeta: &Path, aux_out: &Path)
    -> eyre::Result<TexAuxEcOverlaySummary> {
    info!("Kingdom Reborn items: the Enhanced Client's tables, for the textures tex_aux_ec.uddp holds");
    let ec = load_tex_art_ec_sources(&[ec_dir.to_path_buf()])
        .wrap_err_with(|| format!("load the Enhanced Client tileart and textures from {}", ec_dir.display()))?;
    let ec_crop_adjustments = compute_tex_art_ec_crop_adjustments_from_loaded_sources(&ec)?;
    let (ec_refs, ec_lights) = build_item_texture_refs_and_lights(cc_dir, &ec, &ec_crop_adjustments)?;
    let base = TexAuxEcOverlayBase {
        source_dirs: vec![ec_dir.to_path_buf()],
        keys: tex_aux_ec_texture_keys(&ec_refs, &ec_lights)?,
        max_difference: KR_IMAGE_EQUIVALENCE_THRESHOLD,
    };
    let aux = pack_tex_aux_ec_overlay(&[kr_dir.to_path_buf()], kr_tilemeta, aux_out, &TexAuxEcPackOptions::default(),
        &base)?;
    log_aux(&aux);
    info!("Kingdom Reborn items: {} material textures, {} left to tex_aux_ec.uddp", aux.kept.len(),
        aux.equivalents.len());
    Ok(aux)
}

/// Logs the world-art comparison: what the overlay holds and why, and what it leaves to `tex_art_ec.uddp`.
fn log_world_art(world_art: &WorldArtComparison) {
    info!("Kingdom Reborn world art: {} items, {} textures both clients name for the same window, by difference from \
        the Enhanced Client's (levels): {}", world_art.items, world_art.textures.len(),
        difference_histogram(world_art.textures.iter().map(|&(_, difference)| difference)));
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for (_, difference) in &world_art.differing {
        match counts.iter_mut().find(|(name, _)| *name == difference.name()) {
            Some((_, count)) => *count += 1,
            None => counts.push((difference.name(), 1)),
        }
    }
    info!("Kingdom Reborn world art: {} items in the Kingdom Reborn art section ({counts:?}), {} left to tex_art_ec.uddp (at most \
        {KR_IMAGE_EQUIVALENCE_THRESHOLD} levels from the Enhanced Client's)", world_art.differing.len(),
        world_art.equivalents.len());
    info!("Kingdom Reborn world art in its section (item: why): {:?}", world_art.differing);
    info!("Kingdom Reborn world-art textures left to tex_art_ec.uddp at {WORLD_ART_LOGGED_DIFFERENCE} levels or more \
        (texture: difference; the others are closer): {:?}", world_art.textures.iter()
        .filter(|(_, difference)| difference.is_some_and(|difference| (WORLD_ART_LOGGED_DIFFERENCE..=KR_IMAGE_EQUIVALENCE_THRESHOLD)
            .contains(&difference)))
        .collect::<Vec<_>>());
}

/// Logs the material-texture overlay: what it keeps and what it leaves to `tex_aux_ec.uddp`.
fn log_aux(aux: &TexAuxEcOverlaySummary) {
    info!("Kingdom Reborn material textures: {} kept, {} left to tex_aux_ec.uddp (at most {KR_IMAGE_EQUIVALENCE_THRESHOLD} \
        levels from the Enhanced Client's); {} source-absent ids aliased to the missing texture",
        aux.kept.len(), aux.equivalents.len(), aux.pack.excluded_source_absent);
    info!("Kingdom Reborn material textures kept, by difference (levels): {}",
        difference_histogram(aux.kept.iter().map(|&(_, difference)| difference)));
    info!("Kingdom Reborn material textures left to tex_aux_ec.uddp, by difference (levels): {}",
        difference_histogram(aux.equivalents.iter().map(|record| Some(record.difference))));
    info!("Kingdom Reborn material textures kept (key: difference, none where tex_aux_ec lacks the key or the image \
        has another extent): {:?}", aux.kept);
    info!("Kingdom Reborn material textures left to tex_aux_ec.uddp (key: difference): {:?}",
        aux.equivalents.iter().map(|record| (record.id, record.difference)).collect::<Vec<_>>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_difference_weighs_colour_where_both_draw_and_alpha_everywhere() {
        const OPAQUE: u8 = 255;
        const COLOR_STEP: u8 = 6;
        const ALPHA_STEP: u8 = 40;
        let a = [10, 20, 30, OPAQUE, 0, 0, 0, 0];
        // The second texel is clear in both: its colour does not count.
        let b = [10 + COLOR_STEP, 20, 30 - COLOR_STEP, OPAQUE, 200, 100, 50, 0];
        assert_eq!(image_difference((2, 1, &a), (2, 1, &b)), Some(f32::from(COLOR_STEP) * 2.0 / 3.0));
        let c = [10, 20, 30, OPAQUE - ALPHA_STEP, 0, 0, 0, 0];
        assert_eq!(image_difference((2, 1, &a), (2, 1, &c)), Some(f32::from(ALPHA_STEP) / 2.0));
        assert_eq!(image_difference((2, 1, &a), (1, 2, &a)), None, "another extent");
        assert_eq!(image_difference((2, 1, &a), (2, 1, &a)), Some(0.0));
    }

    #[test]
    fn the_histogram_bins_by_the_bounds() {
        let line = difference_histogram([Some(0.5), Some(2.5), Some(50.0), None]);
        assert!(line.starts_with("0-1: 1, 1-2: 0, 2-3: 1,"), "{line}");
        assert!(line.ends_with("40+: 1, unmeasured: 1"), "{line}");
    }
}
