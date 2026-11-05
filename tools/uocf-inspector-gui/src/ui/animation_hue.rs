//! The hue of the animations view: the picker, and the colours the clients draw a frame with under a hue.
//!
//! Classic (`DrawFrameRle`, see `ghidra-classic-frames/results/classic_frames.md`): hue `N` takes `hues.mul` entry
//! `N - 1`; a full hue writes `color_table[c >> 10]` for every pixel; the partial hue (flag 0x8000) writes it only
//! for greys, `(c & 0x3FF) == c >> 5`, and leaves the other colours as they are.
//! Enhanced: the hue table of `hue%04d.bmp` in `hues.uop` replaces the colour of every pixel whose AMOU palette
//! colour takes the hue (`DecodedFrame::hue_mask`); the client masks the 0x8000 flag away, so there is no grey test.

use crate::app::UopInspectorApp;
use eframe::egui;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use uocf::classic::hues::{HueEntry, HUE_COLOR_COUNT};
use uocf::enhanced::hues::{EcHueLookup, EcHueTable, EcHueTableRef, ShadowPaletteStats, MAX_EC_HUES};

/// Bits of one RGB555 channel.
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MASK: u16 = (1 << CHANNEL_BITS) - 1;
/// Shift of the red channel in an RGB555 colour.
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
/// Green and blue of an RGB555 colour: the grey test compares them with red and green.
const GREEN_BLUE_MASK: u16 = (1 << RED_SHIFT) - 1;
/// The Classic decoders widen a 5-bit channel to 8 bits by this shift (no low-bit fill).
const CHANNEL_WIDEN_SHIFT: u32 = 3;
const RGBA_CHANNELS: usize = 4;
const ALPHA_CHANNEL: usize = 3;
/// `hues.mul` colour shown as the swatch of a hue (the colour of a light grey).
const SWATCH_COLOUR_INDEX: usize = 24;
/// Samples of a hue table drawn as its swatch strip.
const SWATCH_STRIP_SAMPLES: usize = 32;
const SWATCH_STRIP_SAMPLE_WIDTH: f32 = 4.0;
const SWATCH_SIDE: f32 = 14.0;
const SEARCH_FIELD_WIDTH: f32 = 110.0;
const SEARCH_RESULTS_HEIGHT: f32 = 130.0;
/// Matches listed under the search field.
const MAX_SEARCH_RESULTS: usize = 200;
const HEX_PREFIX: &str = "0x";

/// The hue the frames are drawn with.
#[derive(Default)]
pub(crate) struct AnimationHueState {
    /// Hue number; 0 draws the stored colours.
    pub hue: u16,
    /// Classic partial hue (0x8000): only the greys take the hue.
    pub partial: bool,
    pub search: String,
    /// The table of the last Enhanced hue asked for.
    ec_table: Option<(u16, Option<Arc<EcHueTable>>)>,
}

/// How a frame's pixels change under the selected hue.
#[derive(Clone)]
pub(super) enum FrameHue {
    None,
    Classic { table: [u16; HUE_COLOR_COUNT], partial: bool },
    Enhanced(Arc<EcHueTable>),
}

/// Whether the Classic partial hue recolours this RGB555 colour.
pub(super) fn is_classic_grey(colour: u16) -> bool {
    colour & GREEN_BLUE_MASK == colour >> CHANNEL_BITS
}

/// The RGB555 colour the Classic client draws for palette colour `colour` under a hue of `table`.
pub(super) fn classic_hued_colour(colour: u16, table: &[u16; HUE_COLOR_COUNT], partial: bool) -> u16 {
    if partial && !is_classic_grey(colour) {
        return colour;
    }
    table[usize::from((colour >> RED_SHIFT) & CHANNEL_MASK)]
}

/// The hue of `hues.mul` hue number `hue`; 0 and numbers past the file are no hue.
pub(super) fn classic_frame_hue(hues: &[HueEntry], hue: u16, partial: bool) -> FrameHue {
    match usize::from(hue).checked_sub(1).and_then(|index| hues.get(index)) {
        Some(entry) => FrameHue::Classic { table: entry.color_table, partial },
        None => FrameHue::None,
    }
}

/// Hues straight-alpha RGBA pixels in place. `hue_mask` (Enhanced frames) marks the pixels whose palette colour
/// takes the hue; Classic frames have none and are hued from their RGB555 colour, which the decoders widen exactly.
pub(super) fn apply_frame_hue(rgba: &mut [u8], hue_mask: Option<&[u8]>, hue: &FrameHue) {
    match hue {
        FrameHue::None => {}
        FrameHue::Classic { table, partial } => {
            for pixel in rgba.chunks_exact_mut(RGBA_CHANNELS) {
                if pixel[ALPHA_CHANNEL] == 0 {
                    continue;
                }
                let hued = classic_hued_colour(rgb555_of(pixel), table, *partial);
                pixel[..ALPHA_CHANNEL].copy_from_slice(&rgb_of(hued));
            }
        }
        FrameHue::Enhanced(table) => {
            EcHueTableRef::Table(table).apply(rgba, hue_mask.unwrap_or_default(), &ShadowPaletteStats::default())
        }
    }
}

fn rgb555_of(pixel: &[u8]) -> u16 {
    let channel = |value: u8| u16::from(value >> CHANNEL_WIDEN_SHIFT);
    channel(pixel[0]) << RED_SHIFT | channel(pixel[1]) << CHANNEL_BITS | channel(pixel[2])
}

fn rgb_of(colour: u16) -> [u8; 3] {
    let channel = |shift: u32| (((colour >> shift) & CHANNEL_MASK) << CHANNEL_WIDEN_SHIFT) as u8;
    [channel(RED_SHIFT), channel(CHANNEL_BITS), channel(0)]
}

fn colour32(colour: u16) -> egui::Color32 {
    let [r, g, b] = rgb_of(colour);
    egui::Color32::from_rgb(r, g, b)
}

/// Whether hue `id` named `name` matches the search `query`: the number in decimal or `0x` hex, or the name.
pub(super) fn hue_matches(query: &str, id: u16, name: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    if let Some(hex) = query.strip_prefix(HEX_PREFIX) {
        return format!("{id:x}").starts_with(hex);
    }
    id.to_string().contains(&query) || name.to_ascii_lowercase().contains(&query)
}

fn classic_hue_name(entry: &HueEntry) -> String {
    String::from_utf8_lossy(&entry.name).trim_matches('\0').to_string()
}

/// The hue the frames of a Classic (`is_ec` false) or Enhanced source take, and a key of it for texture caches.
pub(super) fn frame_hue(app: &mut UopInspectorApp, is_ec: bool) -> (FrameHue, u64) {
    let (hue, partial) = (app.animations_view.hue.hue, app.animations_view.hue.partial);
    let resolved = if is_ec {
        match EcHueLookup::from_hue(hue) {
            EcHueLookup::Bitmap(number) => ec_hue_table(app, number).map_or(FrameHue::None, FrameHue::Enhanced),
            EcHueLookup::None | EcHueLookup::Shadow => FrameHue::None,
        }
    } else {
        let hues = app.client_data.as_ref().and_then(|client| client.hues.clone());
        hues.map_or(FrameHue::None, |hues| classic_frame_hue(&hues, hue, partial))
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match &resolved {
        FrameHue::None => 0u8.hash(&mut hasher),
        FrameHue::Classic { partial, .. } => (1u8, hue, *partial).hash(&mut hasher),
        FrameHue::Enhanced(_) => (2u8, hue).hash(&mut hasher),
    }
    (resolved, hasher.finish())
}

/// The table of Enhanced hue `number` from the loaded `hues.uop`, cached for the last number asked.
fn ec_hue_table(app: &mut UopInspectorApp, number: u16) -> Option<Arc<EcHueTable>> {
    if let Some((cached, table)) = &app.animations_view.hue.ec_table {
        if *cached == number {
            return table.clone();
        }
    }
    let hash = uocf::enhanced::hues::hue_bitmap_hash(number);
    let table = app
        .uop_cache
        .loaded_uops
        .iter()
        .filter(|loaded| {
            loaded
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(uocf::enhanced::hues::HUES_UOP_NAME))
        })
        .find_map(|loaded| loaded.package.unpack_file_by_hash(hash).ok().flatten())
        .and_then(|bytes| match EcHueTable::from_bitmap(&bytes) {
            Ok(table) => Some(Arc::new(table)),
            Err(error) => {
                log::warn!("hue {number}: {error:#}");
                None
            }
        });
    app.animations_view.hue.ec_table = Some((number, table.clone()));
    table
}

/// Hue number, swatch, search and the partial toggle, for the hues of the selected source's client.
pub(super) fn hue_picker(app: &mut UopInspectorApp, ui: &mut egui::Ui, is_ec: bool) {
    let classic_hues = if is_ec { None } else { app.client_data.as_ref().and_then(|client| client.hues.clone()) };
    let ec_names = if is_ec { app.ec_hues.clone() } else { None };
    let max_hue = if is_ec { MAX_EC_HUES } else { classic_hues.as_ref().map_or(0, |hues| hues.len().min(usize::from(u16::MAX)) as u16) };
    let name_of = |id: u16| -> String {
        match (&classic_hues, &ec_names) {
            (Some(hues), _) => hues.get(usize::from(id).wrapping_sub(1)).map(classic_hue_name).unwrap_or_default(),
            (_, Some(names)) => names.hue_name(id).unwrap_or_default().to_string(),
            _ => String::new(),
        }
    };
    let ec_strip = if is_ec {
        match EcHueLookup::from_hue(app.animations_view.hue.hue) {
            EcHueLookup::Bitmap(number) => ec_hue_table(app, number),
            _ => None,
        }
    } else {
        None
    };

    let state = &mut app.animations_view.hue;
    if max_hue == 0 {
        ui.weak("Load hues.mul to hue the frames.");
        return;
    }
    if is_ec && !app.uop_cache.loaded_uops.iter().any(|loaded| {
        loaded.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
            name.eq_ignore_ascii_case(uocf::enhanced::hues::HUES_UOP_NAME)
        })
    }) {
        ui.weak("Load hues.uop to hue Enhanced frames.");
        return;
    }
    state.hue = state.hue.min(max_hue);
    ui.horizontal_wrapped(|ui| {
        ui.label("Hue:");
        ui.add(egui::DragValue::new(&mut state.hue).range(0..=max_hue))
            .on_hover_text("0 draws the stored colours.");
        if state.hue > 0 {
            match (&classic_hues, &ec_strip) {
                (Some(hues), _) => {
                    if let Some(entry) = hues.get(usize::from(state.hue) - 1) {
                        swatch_strip(ui, entry.color_table.iter().map(|&colour| colour32(colour)));
                    }
                }
                (_, Some(table)) => {
                    let step = table.entries.len() / SWATCH_STRIP_SAMPLES;
                    swatch_strip(
                        ui,
                        table.entries.iter().step_by(step.max(1)).map(|[r, g, b, _]| egui::Color32::from_rgb(*r, *g, *b)),
                    );
                }
                _ => {
                    ui.weak("no table");
                }
            }
            ui.label(name_of(state.hue));
            if ui.small_button("None").clicked() {
                state.hue = 0;
            }
        }
        ui.label("Find:");
        ui.add(egui::TextEdit::singleline(&mut state.search).desired_width(SEARCH_FIELD_WIDTH).hint_text("number or name"));
        let partial_text = if is_ec {
            "The Enhanced client masks the partial flag (0x8000) away: the AMOU palette's per-colour flag decides \
             which pixels take the hue."
        } else {
            "Partial hue (flag 0x8000): only grey colours take the hue; off, every colour does."
        };
        ui.add_enabled(!is_ec, egui::Checkbox::new(&mut state.partial, "Partial hue"))
            .on_hover_text(partial_text)
            .on_disabled_hover_text(partial_text);
    });

    if state.search.trim().is_empty() {
        return;
    }
    let matches: Vec<u16> =
        (1..=max_hue).filter(|&id| hue_matches(&state.search, id, &name_of(id))).take(MAX_SEARCH_RESULTS).collect();
    egui::ScrollArea::vertical().id_salt("animation_hue_search").max_height(SEARCH_RESULTS_HEIGHT).show(ui, |ui| {
        if matches.is_empty() {
            ui.weak("No hue matches.");
        }
        for id in matches {
            ui.horizontal(|ui| {
                if let Some(entry) = classic_hues.as_ref().and_then(|hues| hues.get(usize::from(id) - 1)) {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(SWATCH_SIDE, SWATCH_SIDE), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, colour32(entry.color_table[SWATCH_COLOUR_INDEX]));
                }
                if ui.selectable_label(state.hue == id, format!("{id} (0x{id:04X}) {}", name_of(id))).clicked() {
                    state.hue = id;
                }
            });
        }
    });
}

fn swatch_strip(ui: &mut egui::Ui, colours: impl Iterator<Item = egui::Color32>) {
    let colours: Vec<_> = colours.collect();
    let size = egui::vec2(SWATCH_STRIP_SAMPLE_WIDTH * colours.len() as f32, SWATCH_SIDE);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    for (index, colour) in colours.into_iter().enumerate() {
        let min = rect.min + egui::vec2(SWATCH_STRIP_SAMPLE_WIDTH * index as f32, 0.0);
        ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::vec2(SWATCH_STRIP_SAMPLE_WIDTH, SWATCH_SIDE)), 0.0, colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::animationframe::{HUE_MASK_HUEABLE, HUE_MASK_KEEP_COLOUR};

    /// A table whose colour `i` is pure blue of level `i`, so a hued pixel shows which entry it took.
    fn blue_table() -> [u16; HUE_COLOR_COUNT] {
        std::array::from_fn(|index| index as u16)
    }

    fn rgb555(r: u16, g: u16, b: u16) -> u16 {
        r << RED_SHIFT | g << CHANNEL_BITS | b
    }

    fn pixel(colour: u16) -> Vec<u8> {
        let [r, g, b] = rgb_of(colour);
        vec![r, g, b, u8::MAX]
    }

    #[test]
    fn the_grey_test_needs_three_equal_channels_and_black_is_grey() {
        assert!(is_classic_grey(rgb555(16, 16, 16)));
        assert!(is_classic_grey(0));
        assert!(!is_classic_grey(rgb555(16, 16, 15)));
        assert!(!is_classic_grey(rgb555(31, 0, 0)));
        assert!(!is_classic_grey(0x0001), "new palettes store black as 0x0001 to keep it out of the partial hue");
    }

    #[test]
    fn a_full_hue_takes_every_colour_by_red_and_a_partial_hue_only_the_greys() {
        let table = blue_table();
        let grey = rgb555(16, 16, 16);
        let red = rgb555(31, 0, 0);
        assert_eq!(classic_hued_colour(grey, &table, false), 16);
        assert_eq!(classic_hued_colour(grey, &table, true), 16);
        assert_eq!(classic_hued_colour(red, &table, false), 31);
        assert_eq!(classic_hued_colour(red, &table, true), red);
    }

    #[test]
    fn a_classic_frame_is_hued_per_pixel_and_its_transparent_pixels_are_left() {
        let hue = FrameHue::Classic { table: blue_table(), partial: true };
        let mut rgba = [pixel(rgb555(20, 20, 20)), pixel(rgb555(31, 0, 0)), vec![0, 0, 0, 0]].concat();
        apply_frame_hue(&mut rgba, None, &hue);
        assert_eq!(rgba[..4], pixel(20)[..], "the grey takes table colour 20");
        assert_eq!(rgba[4..8], pixel(rgb555(31, 0, 0))[..], "the red is no grey");
        assert_eq!(rgba[8..], [0, 0, 0, 0], "a pixel outside the runs stays transparent");
    }

    #[test]
    fn hue_zero_and_hues_past_the_file_are_no_hue() {
        let hues = vec![HueEntry { id: 1, color_table: blue_table(), table_start: 0, table_end: 0, name: [0; 20] }];
        assert!(matches!(classic_frame_hue(&hues, 0, false), FrameHue::None));
        assert!(matches!(classic_frame_hue(&hues, 2, false), FrameHue::None));
        assert!(matches!(classic_frame_hue(&hues, 1, true), FrameHue::Classic { partial: true, .. }));
        let mut rgba = pixel(rgb555(16, 16, 16));
        let before = rgba.clone();
        apply_frame_hue(&mut rgba, None, &classic_frame_hue(&hues, 0, false));
        assert_eq!(rgba, before);
        assert_eq!(EcHueLookup::from_hue(0), EcHueLookup::None, "the Enhanced client has no table for hue 0");
    }

    #[test]
    fn an_enhanced_frame_hues_the_flagged_colours_whatever_their_channels() {
        let mut entries = [[0u8; RGBA_CHANNELS]; uocf::enhanced::hues::HUE_TABLE_ENTRIES];
        entries[200] = [1, 2, 3, u8::MAX];
        let hue = FrameHue::Enhanced(Arc::new(EcHueTable { entries }));
        let mut rgba = [vec![200, 10, 90, u8::MAX], vec![200, 10, 90, u8::MAX]].concat();
        apply_frame_hue(&mut rgba, Some(&[HUE_MASK_HUEABLE, HUE_MASK_KEEP_COLOUR]), &hue);
        assert_eq!(rgba[..4], [1, 2, 3, u8::MAX], "a flagged colour takes the table entry of its red byte, grey or not");
        assert_eq!(rgba[4..], [200, 10, 90, u8::MAX], "a keep-colour pixel is drawn as stored");
    }

    #[test]
    fn the_hue_search_matches_numbers_hex_and_names() {
        assert!(hue_matches("", 5, "any"));
        assert!(hue_matches("33", 1133, ""));
        assert!(hue_matches("0x4", 0x45, ""));
        assert!(!hue_matches("0x4", 0x54, ""));
        assert!(hue_matches("Blue", 7, "dark blue"));
        assert!(!hue_matches("red", 7, "dark blue"));
    }
}
