//! Build-time support for the Kingdom Reborn items the Enhanced packages hold (`crate::kr_sections` nests them).
//!
//! Most of Kingdom Reborn's item world art is the Enhanced Client's: every entry of KR's `Texture.uop` is in the
//! Enhanced Client's under the same name, byte-identical or the same image re-encoded (the same extent, DDS format and
//! header, other DXT blocks), and most KR records sample it as the Enhanced records do. The Kingdom Reborn files are
//! read from a folder `crate::kr_delta` rebuilt, where every texture that is the Enhanced one re-encoded where it is
//! drawn ([`TextureUse`]) is the Enhanced texture already, so those measure a difference of 0. Only what differs is
//! packed:
//! - [`pack_kr_art_and_tilemeta`] writes the Kingdom Reborn section of `tilemeta.uddp`: the `tilemeta.uddp` layout,
//!   Classic `tiledata.mul` with KR's `tileart.uop` (record version 3, its flags moved to the EC layout by `uocf`), and
//!   every KR tileart record byte for byte. Its sampling starts follow the trims of the art each item is drawn from:
//!   the KR art for the items the art section holds, the Enhanced art for the others. It also writes the section of
//!   `tex_art_ec.uddp`, an overlay over that package (`tex_art_ec::convert_tex_art_ec_overlay_from_loaded_sources`):
//!   the items whose world art differs from the Enhanced Client's ([`compare_world_art`]: the Enhanced Client lacks
//!   the item, its record names another texture, window, draw offset, kind or sprite descriptor (but for the legacy
//!   window, which the slot never draws), or the image differs where the item draws it beyond its
//!   [`TextureUse::threshold`]), in the Enhanced art's page format ([`kr_art_options`]). The Enhanced records it is
//!   compared with have taken Kingdom Reborn's world-art window where both name the same image at the same scale
//!   ([`adopt_kr_world_windows`]), so those windows are not differences. It lists the other items with
//!   world art, drawn from `tex_art_ec.uddp`, with their image's measured difference. The items kept for another
//!   texture, window, kind or descriptor keep Kingdom Reborn's records, offsets and descriptors; their pixels are the
//!   Enhanced texture's where the Kingdom Reborn one only re-encoded it.
//! - [`pack_kr_aux`] writes the section of `tex_aux_ec.uddp`, an overlay over that package
//!   (`tex_aux_ec::pack_tex_aux_ec_overlay`): of KR's material inputs (masks, noise, water maps), its `missing.tga`,
//!   atmosphere maps and light textures, those whose image differs from the Enhanced Client's by more than
//!   [`KR_IMAGE_EQUIVALENCE_THRESHOLD`] (or that `tex_aux_ec` lacks), lossless RGBA8888 with Zstd
//!   ([`kr_aux_options`]). The others are read from `tex_aux_ec.uddp`; the overlay lists them with their measured
//!   difference.
//!
//! An overlay is nested in the package it is measured against, packed from the same Enhanced Client in the same run,
//! so it cannot be read over another one.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;
use rayon::prelude::*;

use crate::metadata::tilemeta::{
    build_item_texture_refs_and_lights, build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources,
    find_string_dictionary_path, primary_ec_texture_item, TileMetaBuildOptions,
};
use crate::textures::tex_art_ec::{
    compute_tex_art_ec_crop_adjustments_from_loaded_sources, convert_tex_art_ec_overlay_from_loaded_sources,
    load_tex_art_ec_sources, names_packed_world_art, normalized_source_clip_rect, primary_texture_stable_role,
    selected_art_window, TexArtEcAtlasOptions, TexArtEcBuildSummary, TexArtEcLoadedSources,
};
use udd_assets::common::{EcEquivalentRecord, EcOverlayTable};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::ec_material_axes::EcPhysicalPackage;
use uocf::enhanced::tile_database::{ArtDefinition, ArtSpriteDescriptor};
use uocf::enhanced::tileart::{ArtData, ArtTexture, TaeFlag, TileType, TILEART_UOP_NAME};

use crate::textures::tex_aux_ec::{
    pack_tex_aux_ec_overlay, tex_aux_ec_texture_keys, TexAuxEcOverlayBase, TexAuxEcOverlaySummary, TexAuxEcPackOptions,
};
use crate::page_compression::LOSSLESS_PAGE_COMPRESSION;
use crate::PagePixelFormat;

/// The largest [`image_difference`], in 8-bit levels, between a Kingdom Reborn image and the Enhanced Client's image
/// of the same name that still counts as the same image re-encoded, for an image measured whole ([`TextureUse::Whole`]:
/// the terrain and material textures, and the land, material and light textures of the other sections). Of the
/// 8,000-odd shared world-art textures, 98% differ by under 5 levels, and side by side every one up to 8 differs only
/// on DXT block edges and alpha fringes. The KR textures drawn differently (the terrain and material textures: grass,
/// dirt, forest, cobbles, cave floor) differ by 19.7 to 127 levels across the whole image. Between 8 and about 28 some
/// small or translucent sprites (rune stones, fringed plants) are still re-encode noise, but no threshold on this
/// measure separates them from a real change, so sprites are measured with [`visible_difference`] instead
/// ([`KR_SPRITE_EQUIVALENCE_THRESHOLD`]). Of the 512 items the Kingdom Reborn art section held on 2026-11-17 (the
/// `kr-art-diff` study), every one with a terrain texture above 8 is a real change, the lava recoloured (23.9 to 26.8)
/// or a texture redrawn (23.2 to 41.4); so 8 stays the level for whole images.
pub const KR_IMAGE_EQUIVALENCE_THRESHOLD: f32 = 8.0;

/// The largest [`visible_difference`], in 8-bit levels, between a sprite window of a Kingdom Reborn texture and the
/// same window of the Enhanced Client's that still counts as the same sprite re-encoded ([`TextureUse::Sprites`]).
/// Measured on the 512 items the Kingdom Reborn art section held on 2026-11-17 (the `kr-art-diff` study): the 46
/// sprites that differ only by re-encode noise (glowing runes, morning glories, gems, gas traps, arcane circles; 8.1 to
/// 28.2 by [`image_difference`]) measure 3.1 to 15.5, the glowing runes highest; the real changes measure 18.4 to 29.1
/// (terrain textures redrawn) and 23.7 to 26.6 (lava recoloured). 17 lies between the two.
pub const KR_SPRITE_EQUIVALENCE_THRESHOLD: f32 = 17.0;

/// The first texture id of the terrain and material textures of `Texture.uop` (cave floor, dirt, lava,
/// cobblestones): whole tiling images, not sprites, so always [`TextureUse::Whole`]. In the 2026-11-17 study every
/// item whose texture Kingdom Reborn redrew or recoloured names one of these, and no sprite does.
pub const KR_TERRAIN_TEXTURE_ID_START: u32 = 2_000_000;

/// The texels on every side [`visible_difference`] averages each texel with: one, enough to cancel the noise of a DXT
/// re-encode, which moves texels within a 4x4 block but keeps their neighbourhood's mean.
const VISIBLE_BLUR_RADIUS: usize = 1;
/// The largest 8-bit channel value, for premultiplying.
const CHANNEL_MAX: f32 = 255.0;

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

/// How far apart two decoded RGBA8888 images `(width, height, texels)` look when drawn, in 8-bit levels. Each image is
/// premultiplied by its alpha (a translucent texel shows only that share of its colour) and box-blurred over
/// [`VISIBLE_BLUR_RADIUS`] (DXT re-encode noise cancels out among neighbours), spreading into a transparent margin of
/// that radius so that a transparent border does not change the result; then the larger of the summed absolute colour
/// and alpha differences, each over the coverage (the summed larger blurred alpha of the two, as a fraction). `None`
/// when their extents differ; 0 when neither draws anything.
pub fn visible_difference(a: (u32, u32, &[u8]), b: (u32, u32, &[u8])) -> Option<f32> {
    if (a.0, a.1) != (b.0, b.1) || a.2.len() != b.2.len() || a.2.is_empty()
        || a.2.len() != a.0 as usize * a.1 as usize * RGBA_CHANNELS {
        return None;
    }
    let (blurred_a, blurred_b) = (blurred_premultiplied(a), blurred_premultiplied(b));
    let (mut color_sum, mut alpha_sum, mut coverage) = (0f64, 0f64, 0f64);
    for (x, y) in blurred_a.chunks_exact(RGBA_CHANNELS).zip(blurred_b.chunks_exact(RGBA_CHANNELS)) {
        coverage += f64::from(x[COLOR_CHANNELS].max(y[COLOR_CHANNELS]) / CHANNEL_MAX);
        alpha_sum += f64::from((x[COLOR_CHANNELS] - y[COLOR_CHANNELS]).abs());
        color_sum += (0..COLOR_CHANNELS).map(|channel| f64::from((x[channel] - y[channel]).abs())).sum::<f64>()
            / COLOR_CHANNELS as f64;
    }
    if coverage == 0.0 {
        return Some(0.0);
    }
    Some((color_sum / coverage).max(alpha_sum / coverage) as f32)
}

/// [`visible_difference`]'s view of `image`: premultiplied and box-blurred, grown by [`VISIBLE_BLUR_RADIUS`] on every
/// side; texels outside the image count as transparent.
fn blurred_premultiplied((width, height, texels): (u32, u32, &[u8])) -> Vec<f32> {
    let (width, height) = (width as usize, height as usize);
    let (out_width, out_height) = (width + 2 * VISIBLE_BLUR_RADIUS, height + 2 * VISIBLE_BLUR_RADIUS);
    let premultiplied = |x: usize, y: usize| -> [f32; RGBA_CHANNELS] {
        let texel = &texels[(y * width + x) * RGBA_CHANNELS..][..RGBA_CHANNELS];
        let alpha = f32::from(texel[COLOR_CHANNELS]);
        let mut out = [alpha; RGBA_CHANNELS];
        for channel in 0..COLOR_CHANNELS {
            out[channel] = f32::from(texel[channel]) * alpha / CHANNEL_MAX;
        }
        out
    };
    let taps = ((2 * VISIBLE_BLUR_RADIUS + 1) * (2 * VISIBLE_BLUR_RADIUS + 1)) as f32;
    let mut out = vec![0f32; out_width * out_height * RGBA_CHANNELS];
    // Output texel (x, y) is centred on image texel (x - radius, y - radius), so it averages image texels
    // x - 2 * radius ..= x by y - 2 * radius ..= y, those inside the image.
    for y in 0..out_height {
        for x in 0..out_width {
            let sum = &mut out[(y * out_width + x) * RGBA_CHANNELS..][..RGBA_CHANNELS];
            for source_y in y.saturating_sub(2 * VISIBLE_BLUR_RADIUS)..=y.min(height - 1) {
                for source_x in x.saturating_sub(2 * VISIBLE_BLUR_RADIUS)..=x.min(width - 1) {
                    for (total, value) in sum.iter_mut().zip(premultiplied(source_x, source_y)) {
                        *total += value;
                    }
                }
            }
            sum.iter_mut().for_each(|total| *total /= taps);
        }
    }
    out
}

/// How the packages draw a texture, which decides how its Kingdom Reborn and Enhanced images are compared
/// ([`TextureUse::difference`]) and how far apart they may be and still count as one image ([`TextureUse::threshold`]).
#[derive(Debug, Clone, Copy)]
pub enum TextureUse<'a> {
    /// Whole: the terrain and material textures, and every texture drawn other than as an item's world-art sprite.
    /// [`image_difference`] over the whole image, against [`KR_IMAGE_EQUIVALENCE_THRESHOLD`].
    Whole,
    /// Only as items' world-art sprites, cut through these windows ([`selected_art_window`]): [`visible_difference`]
    /// over each window, the largest, against [`KR_SPRITE_EQUIVALENCE_THRESHOLD`].
    Sprites(&'a [ArtTexture]),
}

impl TextureUse<'_> {
    /// The use of texture `texture_id` when items draw it through `windows` alone: [`Self::Whole`] for a terrain or
    /// material texture ([`KR_TERRAIN_TEXTURE_ID_START`]).
    pub fn sprites(texture_id: u32, windows: &[ArtTexture]) -> TextureUse<'_> {
        if texture_id >= KR_TERRAIN_TEXTURE_ID_START { TextureUse::Whole } else { TextureUse::Sprites(windows) }
    }

    /// How far the images `a` and `b` of the texture are apart where it is drawn, in 8-bit levels; `None` when their
    /// extents differ.
    pub fn difference(self, a: (u32, u32, &[u8]), b: (u32, u32, &[u8])) -> Option<f32> {
        let windows = match self {
            TextureUse::Whole => return image_difference(a, b),
            TextureUse::Sprites(windows) => windows,
        };
        if (a.0, a.1) != (b.0, b.1) {
            return None;
        }
        let (Ok(width), Ok(height)) = (u16::try_from(a.0), u16::try_from(a.1)) else {
            return None;
        };
        let mut largest = 0f32;
        for window in windows {
            let difference = match normalized_source_clip_rect(width, height, window) {
                // The window is the whole image or names no texel: the slot is cut from the whole image.
                None => visible_difference(a, b),
                Some(clip) => {
                    let crop = |image: &[u8]| -> Vec<u8> {
                        let row = usize::from(width) * RGBA_CHANNELS;
                        let (left, right) = (usize::from(clip.left) * RGBA_CHANNELS, usize::from(clip.right) * RGBA_CHANNELS);
                        (usize::from(clip.top)..usize::from(clip.bottom))
                            .flat_map(|y| &image[y * row + left..y * row + right]).copied().collect()
                    };
                    let extent = (u32::from(clip.right - clip.left), u32::from(clip.bottom - clip.top));
                    visible_difference((extent.0, extent.1, &crop(a.2)), (extent.0, extent.1, &crop(b.2)))
                }
            }?;
            largest = largest.max(difference);
        }
        Some(largest)
    }

    /// The largest [`Self::difference`] at which the Kingdom Reborn image still counts as the Enhanced one re-encoded.
    pub fn threshold(self) -> f32 {
        match self {
            TextureUse::Whole => KR_IMAGE_EQUIVALENCE_THRESHOLD,
            TextureUse::Sprites(_) => KR_SPRITE_EQUIVALENCE_THRESHOLD,
        }
    }
}

/// The `Texture.uop` entry name hash of world-art texture `texture_id`, as `uocf::enhanced::textures::Textures` names
/// it.
fn world_art_entry_hash(texture_id: u32) -> u64 {
    hash_file_name_single(&format!("build/worldart/{texture_id:08}.dds"))
}

/// How the records of one client draw the world-art textures of its `Texture.uop` ([`TextureUse`]), by entry name hash.
#[derive(Debug, Clone, Default)]
pub struct WorldArtTextureUses {
    windows: HashMap<u64, Vec<ArtTexture>>,
}

impl WorldArtTextureUses {
    /// From the records of `art`: a texture is drawn only through windows ([`TextureUse::Sprites`]) when it is the
    /// primary block-0 image of items the art build packs (`names_packed_world_art`), each cut through its own window
    /// ([`selected_art_window`]), and no other texture item of a record names it. Any other texture is drawn whole: a
    /// terrain or material texture, one another block or item of a record names (a mask, say), and one no record names.
    pub fn from_art(art: &ArtDefinition) -> Self {
        let window_key = |window: &ArtTexture| [window.start_x, window.start_y, window.end_x, window.end_y];
        let mut windows: HashMap<u32, Vec<ArtTexture>> = HashMap::new();
        let mut whole = HashSet::new();
        for data in art.definitions.values() {
            let sprite = data.ec_texture.as_ref().filter(|_| names_packed_world_art(data));
            let primary_item = sprite.and_then(|texture| primary_ec_texture_item(data, texture.texture_id));
            if let Some(texture) = sprite {
                let window = selected_art_window(data, texture);
                let known = windows.entry(texture.texture_id).or_default();
                if !known.iter().any(|known| window_key(known) == window_key(&window)) {
                    known.push(window);
                }
            }
            whole.extend(data.texture_items.iter().flatten()
                .filter(|item| item.ec_physical_package == EcPhysicalPackage::TextureUop)
                .filter(|item| primary_item != Some((item.block_index, item.item_index)))
                .map(|item| item.id));
        }
        windows.retain(|texture_id, _| !whole.contains(texture_id)
            && matches!(TextureUse::sprites(*texture_id, &[]), TextureUse::Sprites(_)));
        Self { windows: windows.into_iter().map(|(texture_id, windows)| (world_art_entry_hash(texture_id), windows)).collect() }
    }

    /// [`Self::from_art`] of the records in the client folder `dir`.
    pub fn load(dir: &Path) -> eyre::Result<Self> {
        Ok(Self::from_art(&load_art_definition(dir)?))
    }

    /// The use of the `Texture.uop` entry `name_hash`.
    pub fn texture_use(&self, name_hash: u64) -> TextureUse<'_> {
        self.windows.get(&name_hash).map_or(TextureUse::Whole, |windows| TextureUse::Sprites(windows))
    }

    /// How many textures are drawn only through windows.
    pub fn sprite_texture_count(&self) -> usize {
        self.windows.len()
    }
}

/// The tileart records of the client folder `dir`.
fn load_art_definition(dir: &Path) -> eyre::Result<ArtDefinition> {
    let strings = find_string_dictionary_path(&[dir.to_path_buf()])
        .ok_or_else(|| eyre::eyre!("missing string_dictionary.uop in {}", dir.display()))?;
    ArtDefinition::load(&dir.join(TILEART_UOP_NAME), &strings)
        .wrap_err_with(|| format!("load the tileart of {}", dir.display()))
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
    /// Another authored sprite descriptor (the source tile, scales and world window of the sprite geometry; not the
    /// legacy window, see [`drawn_descriptor`]).
    OtherSpriteDescriptor,
    /// The same texture, whose image differs where the item draws it beyond its [`TextureUse::threshold`] (`None`:
    /// another extent, or one client lacks it).
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
    /// Each world-art texture both clients name for the same window, with its largest difference over the items'
    /// windows ([`TextureUse::difference`]; `None` as in `OtherImage`).
    pub textures: Vec<(u32, Option<f32>)>,
    /// The items whose world art differs, by item.
    pub differing: Vec<(u16, WorldArtDifference)>,
    /// The other items with world art, with their image's difference, by item.
    pub equivalents: Vec<EcEquivalentRecord>,
}

/// The fields of `descriptor` the item's world art is drawn by, all but its legacy window: that is the window of the
/// item's legacy art (the record's `cc_img_offset`), which the world-art slot never draws, and Kingdom Reborn authored
/// its own over its own legacy art. The record that holds it stays exact in the Kingdom Reborn tile data section, so an
/// item whose descriptors differ in it alone draws the Enhanced Client's world art and is left to it.
fn drawn_descriptor(descriptor: &ArtSpriteDescriptor) -> (u32, f32, f32, [i32; 6]) {
    (descriptor.source_tile_id, descriptor.world_art_scale, descriptor.legacy_art_scale, descriptor.world_window)
}

/// Compares each Kingdom Reborn item's world art with the Enhanced Client's: what decides its `tex_art_ec` slot (its
/// record's primary block-0 texture with its window and draw offset, its kind and sprite descriptor) and that
/// texture's image where the item draws it ([`TextureUse::sprites`] of the item's window).
pub fn compare_world_art(kr: &TexArtEcLoadedSources, ec: &TexArtEcLoadedSources) -> WorldArtComparison {
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
        } else if kr_descriptor.map(drawn_descriptor) != ec_descriptor.map(drawn_descriptor) {
            comparison.differing.push((id, WorldArtDifference::OtherSpriteDescriptor));
        } else {
            shared.push((id, kr_texture.texture_id, selected_art_window(kr_data, kr_texture)));
        }
    }
    // Each texture decoded once, each window items draw it through measured once.
    let window_key = |window: &ArtTexture| [window.start_x, window.start_y, window.end_x, window.end_y];
    let mut windows: BTreeMap<u32, Vec<&ArtTexture>> = BTreeMap::new();
    for (_, texture, window) in &shared {
        let windows = windows.entry(*texture).or_default();
        if !windows.iter().any(|known| window_key(known) == window_key(window)) {
            windows.push(window);
        }
    }
    let windows: Vec<(u32, Vec<&ArtTexture>)> = windows.into_iter().collect();
    let decode = |sources: &TexArtEcLoadedSources, texture: u32| sources.world_textures.get_from_id(texture).ok().flatten()
        .and_then(|file| file.decode_to_rgba8().ok());
    let measured: Vec<Vec<Option<f32>>> = windows.par_iter().map(|(texture, windows)| {
        let images = decode(kr, *texture).zip(decode(ec, *texture));
        windows.iter().map(|&window| images.as_ref().and_then(|(a, b)|
            TextureUse::sprites(*texture, std::slice::from_ref(window))
                .difference((a.width(), a.height(), a.as_raw()), (b.width(), b.height(), b.as_raw())))).collect()
    }).collect();
    comparison.textures = windows.iter().zip(&measured).map(|((texture, _), differences)| {
        (*texture, differences.iter().try_fold(0f32, |largest, difference| difference.map(|d| largest.max(d))))
    }).collect();
    let differences: HashMap<(u32, [i32; 4]), Option<f32>> = windows.iter().zip(&measured)
        .flat_map(|((texture, windows), differences)| windows.iter().zip(differences)
            .map(|(window, difference)| ((*texture, window_key(window)), *difference)))
        .collect();
    for (id, texture, window) in &shared {
        let threshold = TextureUse::sprites(*texture, std::slice::from_ref(window)).threshold();
        match differences[&(*texture, window_key(window))] {
            Some(difference) if difference <= threshold =>
                comparison.equivalents.push(EcEquivalentRecord { id: u32::from(*id), difference }),
            difference => comparison.differing.push((*id, WorldArtDifference::OtherImage { texture: *texture, difference })),
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

/// Gives the Enhanced records of `ec` Kingdom Reborn's world-art window where theirs was never authored, and returns
/// their ids.
///
/// For 215 items (the creature statuettes, the desert ostard, a bird, the blank wisp and death vortex) the Enhanced
/// record names the same block-0 texture at the same `world_art_scale` as Kingdom Reborn's but another enhanced window:
/// a copy of its legacy window, sometimes with one or two of Kingdom Reborn's values, which the Enhanced Client never
/// reads (those records lack `UseNewArt`, so it draws their legacy art). Kingdom Reborn, which draws that art, reads its
/// own window, and that window fits the figure (`docs/client_research/world-art-windows`). Each such item takes Kingdom
/// Reborn's window and draw offsets, in `ArtData::ec_texture` and `ArtSpriteDescriptor::world_window`; the rest of its
/// record stays the Enhanced Client's.
pub fn adopt_kr_world_windows(ec: &mut ArtDefinition, kr: &ArtDefinition) -> Vec<u16> {
    let window = |t: &ArtTexture| [t.start_x, t.start_y, t.end_x, t.end_y, t.offset_x, t.offset_y];
    let mut adopted = Vec::new();
    for (id, ec_data) in &mut ec.definitions {
        let (Some(ec_texture), Some(kr_texture)) = (ec_data.ec_texture.as_mut(),
            kr.definitions.get(id).and_then(|kr_data| kr_data.ec_texture.as_ref())) else { continue };
        let (Some(ec_descriptor), Some(kr_descriptor)) =
            (ec.sprite_descriptors.get_mut(id), kr.sprite_descriptors.get(id)) else { continue };
        if ec_texture.texture_id != kr_texture.texture_id
            || ec_descriptor.world_art_scale != kr_descriptor.world_art_scale
            || window(ec_texture) == window(kr_texture) {
            continue;
        }
        *ec_texture = kr_texture.clone();
        ec_descriptor.world_window = kr_descriptor.world_window;
        adopted.push(*id);
    }
    adopted.sort_unstable();
    adopted
}

/// [`load_tex_art_ec_sources`] of `ec_dirs`, with the world-art windows of the Kingdom Reborn records in `kr_dir`
/// adopted ([`adopt_kr_world_windows`]): the Enhanced sources every package with a Kingdom Reborn section is packed
/// from.
pub fn load_ec_sources_with_kr_windows(ec_dirs: &[PathBuf], kr_dir: &Path) -> eyre::Result<TexArtEcLoadedSources> {
    let mut ec = load_tex_art_ec_sources(ec_dirs)?;
    let kr = load_art_definition(kr_dir)?;
    let adopted = adopt_kr_world_windows(&mut ec.art_definition, &kr);
    info!("Enhanced items with Kingdom Reborn's world-art window ({}): {:x?}", adopted.len(), adopted);
    Ok(ec)
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

/// The Kingdom Reborn item art's options: the page layout and encoding of the Enhanced art's `ec_art_options` (page
/// size, gutter, pixel format, compression and BC7 tuning), so the two share the Enhanced art atlas as stored, with
/// pages of one size, without upscaling.
pub fn kr_art_options(ec_art_options: &TexArtEcAtlasOptions) -> TexArtEcAtlasOptions {
    TexArtEcAtlasOptions {
        atlas_width: ec_art_options.atlas_width,
        atlas_height: ec_art_options.atlas_height,
        gutter: ec_art_options.gutter,
        compression: ec_art_options.compression,
        pixel_format: ec_art_options.pixel_format,
        bc7_rdo_lambda: ec_art_options.bc7_rdo_lambda,
        bc7_rdo_lookback_blocks: ec_art_options.bc7_rdo_lookback_blocks,
        ..TexArtEcAtlasOptions::default()
    }
}

/// The Kingdom Reborn material textures' options: lossless RGBA8888 with Zstd, whatever `tex_aux_ec` takes (BC7 by
/// default); they are few, and each texture keeps its own format.
pub fn kr_aux_options() -> TexAuxEcPackOptions {
    TexAuxEcPackOptions {
        compression: LOSSLESS_PAGE_COMPRESSION,
        pixel_format: PagePixelFormat::Rgba8888,
        ..TexAuxEcPackOptions::default()
    }
}

/// Where [`pack_kr_art_and_tilemeta`] writes the Kingdom Reborn tile data: `out`, with the Classic `tiledata.mul` (and
/// `radarcol.mul`) of `cc_dir` as the Enhanced tilemeta takes them.
pub struct KrTilemetaOutput<'a> {
    pub cc_dir: &'a Path,
    pub out: &'a Path,
}

/// Writes the Kingdom Reborn item art overlay to `art_out` and, with `tilemeta`, the Kingdom Reborn tile data (see the
/// module documentation), from the Kingdom Reborn files in `kr_dir`, against `ec`, the Enhanced sources the Enhanced
/// art is packed from. The art takes the encoding of `ec_art_options`, the Enhanced art's ([`kr_art_options`]).
pub fn pack_kr_art_and_tilemeta(kr_dir: &Path, ec: &TexArtEcLoadedSources, art_out: &Path,
    ec_art_options: &TexArtEcAtlasOptions, tilemeta: Option<KrTilemetaOutput>) -> eyre::Result<KrArtBuildSummary> {
    info!("Converting the Kingdom Reborn items to {}{}", art_out.display(),
        tilemeta.as_ref().map(|tilemeta| format!(" and {}", tilemeta.out.display())).unwrap_or_default());
    let mut kr = load_tex_art_ec_sources(&[kr_dir.to_path_buf()])
        .wrap_err_with(|| format!("load the Kingdom Reborn tileart and textures from {}", kr_dir.display()))?;

    let world_art = compare_world_art(&kr, ec);
    log_world_art(&world_art);

    // The overlay art: the KR art build of the differing items alone.
    let differing: HashSet<u16> = world_art.differing.iter().map(|&(id, _)| id).collect();
    let full_definition = kr.art_definition.clone();
    kr.art_definition.definitions.retain(|id, _| differing.contains(id));
    let art_table = EcOverlayTable { equivalents: world_art.equivalents.clone() };
    let art = convert_tex_art_ec_overlay_from_loaded_sources(&kr, art_out, &kr_art_options(ec_art_options),
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
    let aux = pack_tex_aux_ec_overlay(&[kr_dir.to_path_buf()], kr_tilemeta, aux_out, &kr_aux_options(), &base)?;
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
        {KR_SPRITE_EQUIVALENCE_THRESHOLD} levels from the Enhanced Client's where a sprite is drawn, \
        {KR_IMAGE_EQUIVALENCE_THRESHOLD} for a terrain texture)", world_art.differing.len(), world_art.equivalents.len());
    info!("Kingdom Reborn world art in its section (item: why): {:?}", world_art.differing);
    info!("Kingdom Reborn world-art items left to tex_art_ec.uddp at {WORLD_ART_LOGGED_DIFFERENCE} levels or more \
        (item: difference; the others are closer): {:?}", world_art.equivalents.iter()
        .filter(|record| record.difference >= WORLD_ART_LOGGED_DIFFERENCE)
        .map(|record| (record.id, record.difference))
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

    const OPAQUE: u8 = 255;
    const SIDE: u32 = 8;
    /// The 8-bit levels a [`visible_difference`] result may be off from the exact value by float rounding.
    const LEVEL_EPSILON: f32 = 1e-3;

    /// A `SIDE` x `SIDE` RGBA image, each texel `texel(x, y)`.
    fn image(texel: impl Fn(u32, u32) -> [u8; RGBA_CHANNELS]) -> Vec<u8> {
        (0..SIDE * SIDE).flat_map(|index| texel(index % SIDE, index / SIDE)).collect()
    }

    /// `image` (`SIDE` x `SIDE`) inside a transparent border `margin` texels wide.
    fn padded(image: &[u8], margin: u32) -> Vec<u8> {
        let side = SIDE + 2 * margin;
        (0..side * side).flat_map(|index| {
            let (x, y) = (index % side, index / side);
            if (margin..margin + SIDE).contains(&x) && (margin..margin + SIDE).contains(&y) {
                let at = (((y - margin) * SIDE + x - margin) as usize) * RGBA_CHANNELS;
                <[u8; RGBA_CHANNELS]>::try_from(&image[at..at + RGBA_CHANNELS]).unwrap()
            } else {
                [0; RGBA_CHANNELS]
            }
        }).collect()
    }

    #[test]
    fn re_encode_noise_cancels_out_among_neighbours_and_a_recolour_does_not() {
        const GREY: u8 = 120;
        const NOISE: u8 = 12;
        const RECOLOUR: u8 = 24;
        let grey = image(|_, _| [GREY, GREY, GREY, OPAQUE]);
        // A DXT re-encode moves texels up and down within a block and keeps its mean: a checkerboard of +-NOISE.
        let noisy = image(|x, y| {
            let level = if (x + y) % 2 == 0 { GREY + NOISE } else { GREY - NOISE };
            [level, level, level, OPAQUE]
        });
        let recoloured = image(|_, _| [GREY + RECOLOUR, GREY, GREY - RECOLOUR, OPAQUE]);
        let measure = |a: &[u8], b: &[u8]| visible_difference((SIDE, SIDE, a), (SIDE, SIDE, b)).unwrap();
        assert_eq!(image_difference((SIDE, SIDE, &grey), (SIDE, SIDE, &noisy)), Some(f32::from(NOISE)));
        assert!(measure(&grey, &noisy) < f32::from(NOISE) / 4.0, "{}", measure(&grey, &noisy));
        assert!(measure(&grey, &recoloured) > KR_SPRITE_EQUIVALENCE_THRESHOLD / 2.0, "{}", measure(&grey, &recoloured));
    }

    #[test]
    fn a_nearly_clear_texel_shows_only_its_share_of_its_colour() {
        const FAINT: u8 = 8;
        let base = |x: u32, y: u32| if (x, y) == (0, 0) { [0, 0, 0, FAINT] } else { [90, 90, 90, OPAQUE] };
        let a = image(base);
        let b = image(|x, y| if (x, y) == (0, 0) { [OPAQUE, OPAQUE, OPAQUE, FAINT] } else { base(x, y) });
        let raw = image_difference((SIDE, SIDE, &a), (SIDE, SIDE, &b)).unwrap();
        let visible = visible_difference((SIDE, SIDE, &a), (SIDE, SIDE, &b)).unwrap();
        assert!(raw > 3.0 && visible < raw / 10.0, "raw {raw}, visible {visible}");
    }

    #[test]
    fn a_transparent_border_changes_nothing() {
        const MARGIN: u32 = 3;
        let a = image(|x, y| [(x * 30) as u8, (y * 30) as u8, 60, if x < 2 { 0 } else { OPAQUE }]);
        let b = image(|x, y| [(x * 30) as u8 + 5, (y * 30) as u8, 60, if x < 2 { 0 } else { OPAQUE - 9 }]);
        let side = SIDE + 2 * MARGIN;
        let tight = visible_difference((SIDE, SIDE, &a), (SIDE, SIDE, &b)).unwrap();
        let loose = visible_difference((side, side, &padded(&a, MARGIN)), (side, side, &padded(&b, MARGIN))).unwrap();
        assert!((tight - loose).abs() < LEVEL_EPSILON, "{tight} against {loose}");
    }

    #[test]
    fn a_sprite_is_measured_through_its_window_alone() {
        const CUT: u32 = SIDE / 2;
        let a = image(|_, _| [100, 100, 100, OPAQUE]);
        // The right half differs; items draw the left half alone.
        let b = image(|x, _| if x < CUT { [100, 100, 100, OPAQUE] } else { [200, 40, 40, OPAQUE] });
        let left = ArtTexture { end_x: CUT as i32, end_y: SIDE as i32, ..ArtTexture::default() };
        let whole = ArtTexture { end_x: SIDE as i32, end_y: SIDE as i32, ..ArtTexture::default() };
        let (a, b) = ((SIDE, SIDE, a.as_slice()), (SIDE, SIDE, b.as_slice()));
        assert_eq!(TextureUse::Sprites(std::slice::from_ref(&left)).difference(a, b), Some(0.0));
        let both = [left.clone(), whole.clone()];
        assert!(TextureUse::Sprites(&both).difference(a, b).unwrap() > KR_SPRITE_EQUIVALENCE_THRESHOLD,
            "the largest of the windows");
        assert_eq!(TextureUse::Sprites(std::slice::from_ref(&whole)).difference(a, b),
            visible_difference(a, b), "a window over the whole image");
        assert_eq!(TextureUse::sprites(KR_TERRAIN_TEXTURE_ID_START, std::slice::from_ref(&left)).difference(a, b),
            image_difference(a, b), "a terrain texture is measured whole");
    }

    /// Measured on 2026-11-17 (the `kr-art-diff` study), each by the measure its texture's use takes: the item, its
    /// texture, the difference, and whether its Kingdom Reborn image counts as the Enhanced one.
    const MEASURED_ITEMS: [(u16, u32, f32, bool); 4] = [
        // A glowing rune: re-encode noise, the highest of the sprites (28.2 over the whole image).
        (0x0E5D, 3677, 15.5, true),
        // A gas trap: re-encode noise (8.07 over the whole image, the lowest world art kept until then).
        (0x113A, 4410, 4.7, true),
        // The lava, recoloured.
        (0x1324, 2_000_103, 26.8, false),
        // The cave floor, redrawn.
        (0x040B, 2_000_130, 23.2, false),
    ];
    /// The closest real change by [`visible_difference`] in the same study, a redrawn texture measured as a sprite.
    const CLOSEST_SPRITE_CHANGE: f32 = 18.4;

    #[test]
    fn the_thresholds_part_the_measured_noise_from_the_measured_changes() {
        let window = ArtTexture { end_x: 44, end_y: 44, ..ArtTexture::default() };
        for (item, texture, difference, equivalent) in MEASURED_ITEMS {
            let texture_use = TextureUse::sprites(texture, std::slice::from_ref(&window));
            assert_eq!(difference <= texture_use.threshold(), equivalent, "item {item:#06x}, texture {texture}");
        }
        assert!(CLOSEST_SPRITE_CHANGE > TextureUse::Sprites(std::slice::from_ref(&window)).threshold());
    }

    /// A packed static sprite record of `texture` with the window `[start_x, start_y, end_x, end_y, offset_x, offset_y]`.
    fn sprite_record(texture: u32, window: [i32; 6]) -> ArtData {
        let [start_x, start_y, end_x, end_y, offset_x, offset_y] = window;
        ArtData {
            ec_texture: Some(ArtTexture { texture_id: texture, start_x, start_y, end_x, end_y, offset_x, offset_y }),
            texture_items: vec![vec![uocf::enhanced::tileart::TextureItem {
                id: texture, ec_physical_package: EcPhysicalPackage::TextureUop, ..Default::default()
            }]],
            ..ArtData::default()
        }
    }

    fn descriptor(scale: f32, window: [i32; 6]) -> ArtSpriteDescriptor {
        ArtSpriteDescriptor { source_tile_id: 0, world_art_scale: scale,
            legacy_art_scale: 1.5, world_window: window, legacy_window: [0, 0, 44, 44, 0, 0] }
    }

    fn definition(records: &[(u16, u32, f32, [i32; 6])]) -> ArtDefinition {
        ArtDefinition {
            definitions: records.iter().map(|&(id, texture, _, window)| (id, sprite_record(texture, window))).collect(),
            sprite_descriptors: records.iter().map(|&(id, _, scale, window)| (id, descriptor(scale, window))).collect(),
        }
    }

    #[test]
    fn an_enhanced_record_takes_the_kingdom_reborn_window_of_the_same_image_and_scale() {
        // 0x20D5 Dog Frame: the legacy window copied in the Enhanced record, the figure's bounds in Kingdom Reborn's.
        const COPIED: [i32; 6] = [0, 0, 44, 44, 0, 0];
        const FIGURE: [i32; 6] = [0, 0, 28, 65, 21, -9];
        const SCALE: f32 = 0.8;
        let mut ec = definition(&[(0x20D5, 8405, SCALE, COPIED), (1, 1, SCALE, COPIED), (2, 2, SCALE, COPIED),
            (3, 3, SCALE, COPIED), (4, 4, SCALE, COPIED)]);
        let kr = definition(&[(0x20D5, 8405, SCALE, FIGURE), (1, 10, SCALE, FIGURE), (2, 2, SCALE / 2.0, FIGURE),
            (3, 3, SCALE, COPIED)]);
        assert_eq!(adopt_kr_world_windows(&mut ec, &kr), [0x20D5]);
        let dog = &ec.definitions[&0x20D5];
        assert_eq!(dog.ec_texture.as_ref().map(|t| [t.start_x, t.start_y, t.end_x, t.end_y, t.offset_x, t.offset_y]),
            Some(FIGURE));
        assert_eq!(ec.sprite_descriptors[&0x20D5].world_window, FIGURE);
        assert_eq!(ec.sprite_descriptors[&0x20D5].legacy_window, [0, 0, 44, 44, 0, 0], "the rest stays the Enhanced");
        for (id, why) in [(1, "another texture"), (2, "another scale"), (3, "the same window"), (4, "no KR record")] {
            assert_eq!(ec.sprite_descriptors[&id].world_window, COPIED, "{why}");
        }
    }

    #[test]
    fn a_texture_is_drawn_as_sprites_only_when_no_record_names_it_otherwise() {
        const WINDOW: [i32; 6] = [0, 0, 20, 30, 0, 0];
        let mut art = definition(&[(1, 100, 1.0, WINDOW), (2, 100, 1.0, [2, 2, 10, 10, 0, 0]), (3, 200, 1.0, WINDOW),
            (4, KR_TERRAIN_TEXTURE_ID_START, 1.0, WINDOW)]);
        // Item 5 uses texture 200 as a mask in its block 0.
        let mut mask_user = sprite_record(300, WINDOW);
        mask_user.texture_items[0].push(uocf::enhanced::tileart::TextureItem { id: 200, item_index: 1,
            ec_physical_package: EcPhysicalPackage::TextureUop, ..Default::default() });
        art.definitions.insert(5, mask_user);
        let uses = WorldArtTextureUses::from_art(&art);
        match uses.texture_use(world_art_entry_hash(100)) {
            TextureUse::Sprites(windows) => assert_eq!(windows.len(), 2, "each item's window"),
            TextureUse::Whole => panic!("texture 100 is drawn only as sprites"),
        }
        assert!(matches!(uses.texture_use(world_art_entry_hash(200)), TextureUse::Whole), "also a mask");
        assert!(matches!(uses.texture_use(world_art_entry_hash(KR_TERRAIN_TEXTURE_ID_START)), TextureUse::Whole));
        assert!(matches!(uses.texture_use(world_art_entry_hash(999)), TextureUse::Whole), "named by no record");
        assert_eq!(uses.sprite_texture_count(), 2, "100 and 300");
    }

    #[test]
    fn the_histogram_bins_by_the_bounds() {
        let line = difference_histogram([Some(0.5), Some(2.5), Some(50.0), None]);
        assert!(line.starts_with("0-1: 1, 1-2: 0, 2-3: 1,"), "{line}");
        assert!(line.ends_with("40+: 1, unmeasured: 1"), "{line}");
    }
}
