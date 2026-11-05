//! The texture preview of the Tile Metadata tab, after UOETE's: every texture a tileart.uop record references, a row
//! per shader block, with the window the client cuts from the art (the bounding box) and the point its offsets are
//! measured from drawn over it, each overlay with its own toggle. The record is read every frame, so an edit of a
//! window or an offset moves its overlay at once. A tiledata.mul tile shows its art alone. Shift or Ctrl + wheel, or a pinch, zooms,
//! anchored at the pointer; the plain wheel scrolls.
//!
//! The geometry is `UOTileArt::BuildSpriteDescriptor`'s (`docs/agent_notes/ec_sprite_descriptor_geometry.md`): the
//! world art block (client mode 1) is cut with the EC window, its ends inclusive; the legacy art block (mode 0) with the
//! CC window; the enhanced tileart block (mode 2) with the CC window, its ends times 1.5. The sprite's left edge lies
//! `x offset` right of the object's point and its bottom edge `y offset` below it, so that point is at
//! (x start - x offset, y end - y offset) in the image. The light block's offsets are in view space: they are listed,
//! not drawn.

use eframe::egui;
use uocf::enhanced::tileart::{
    primary_texture_position, TaeFlag, TaeImgOffset, TextureItem, TextureType, TileArtEntry, TILEART_KR_RECORD_VERSION,
};

use crate::app::UopInspectorApp;

/// Zoom of a new preview: classic art is small.
const DEFAULT_ZOOM: f32 = 2.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 16.0;
/// From this id on UOETE draws no bounding box and a smaller image: these are a record's noise and other extra
/// textures, not the art its windows cut.
const FIRST_EXTRA_TEXTURE_ID: u32 = 1_000_000;
/// UOETE draws the extra textures at half size.
const EXTRA_TEXTURE_SCALE: f32 = 0.5;
/// Room between two textures of a block, as in UOETE.
const TEXTURE_SPACING: f32 = 20.0;
/// The client adds this to the ends of the EC window: they are inclusive.
const EC_WINDOW_END_INCLUSIVE: i32 = 1;
/// The client multiplies the ends of the CC window by this for the enhanced tileart block (`BuildSpriteDescriptor`).
const ENHANCED_WINDOW_END_SCALE: f32 = 1.5;
/// ... and the legacy art scale by this (`DAT_00d05174`).
const ENHANCED_ART_SCALE_FACTOR: f32 = 2.0 / 3.0;
const BOUNDING_BOX_COLOR: egui::Color32 = egui::Color32::RED;
const OFFSETS_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 200, 255);
const OVERLAY_STROKE_WIDTH: f32 = 1.0;
/// Half the size of the cross at the offsets' point, in screen pixels.
const ORIGIN_MARK_RADIUS: f32 = 5.0;
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const PREVIEW_STATE_ID: &str = "tile_metadata_preview";

/// What a shader block draws, by its position and the record version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BlockRole {
    WorldArt,
    LegacyArt,
    EnhancedArt,
    Light,
    Unknown,
}

/// The art block positions of a version 4 record; a Kingdom Reborn record has no enhanced block, its light block
/// comes third.
pub(super) fn block_role(version: u16, block_index: u8) -> BlockRole {
    match (block_index, version == TILEART_KR_RECORD_VERSION) {
        (0, _) => BlockRole::WorldArt,
        (1, _) => BlockRole::LegacyArt,
        (2, true) | (3, false) => BlockRole::Light,
        (2, false) => BlockRole::EnhancedArt,
        _ => BlockRole::Unknown,
    }
}

fn block_role_label(role: BlockRole) -> &'static str {
    match role {
        BlockRole::WorldArt => "world art",
        BlockRole::LegacyArt => "legacy art",
        BlockRole::EnhancedArt => "enhanced tileart",
        BlockRole::Light => "light",
        BlockRole::Unknown => "unknown block",
    }
}

/// The block the client draws the record with, and why (`BuildSpriteDescriptor`).
pub(super) fn drawn_block(entry: &TileArtEntry) -> (BlockRole, &'static str) {
    if entry.flags2.contains(TaeFlag::UseNewArt) {
        (BlockRole::WorldArt, "UseNewArt is set in flags 2 (unless the client is set to legacy art)")
    } else if entry.uses_enhanced_tileart {
        (BlockRole::EnhancedArt, "Enhanced tileart is set")
    } else {
        (BlockRole::LegacyArt, "the default")
    }
}

/// The art scale of a block's sprite (source pixels per legacy pixel times 1.5).
fn block_scale(entry: &TileArtEntry, role: BlockRole) -> Option<f32> {
    match role {
        BlockRole::WorldArt => Some(entry.world_art_scale),
        BlockRole::LegacyArt => Some(entry.legacy_art_scale),
        BlockRole::EnhancedArt => Some(entry.legacy_art_scale * ENHANCED_ART_SCALE_FACTOR),
        BlockRole::Light | BlockRole::Unknown => None,
    }
}

/// The window the client cuts a block's art with, in image pixels (right and bottom exclusive), and its offsets.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct SpriteWindow {
    pub label: &'static str,
    pub min: egui::Pos2,
    pub max: egui::Pos2,
    pub offset: egui::Vec2,
}

impl SpriteWindow {
    fn new(label: &'static str, window: &TaeImgOffset, end_extra: i32, end_scale: f32) -> Self {
        let end = |value: i32| value.saturating_add(end_extra) as f32 * end_scale;
        Self {
            label,
            min: egui::pos2(window.x_start as f32, window.y_start as f32),
            max: egui::pos2(end(window.x_end), end(window.y_end)),
            offset: egui::vec2(window.x_off as f32, window.y_off as f32),
        }
    }

    /// A window without pixels: the client has its own handling of those, so no box is drawn.
    pub fn is_empty(&self) -> bool {
        self.max.x <= self.min.x || self.max.y <= self.min.y
    }

    /// The point the offsets are measured from: the object's point on the sprite.
    pub fn origin(&self) -> egui::Pos2 {
        egui::pos2(self.min.x - self.offset.x, self.max.y - self.offset.y)
    }

    /// Whether the window lies inside an image of `size`.
    pub fn fits(&self, size: [usize; 2]) -> bool {
        self.min.x >= 0.0 && self.min.y >= 0.0 && self.max.x <= size[0] as f32 && self.max.y <= size[1] as f32
    }
}

/// The window of a block's art; None for the light and unknown blocks.
pub(super) fn sprite_window(entry: &TileArtEntry, role: BlockRole) -> Option<SpriteWindow> {
    match role {
        BlockRole::WorldArt => {
            let window = &entry.ec_img_offset;
            let positive = window.x_end > window.x_start && window.y_end > window.y_start;
            let end_extra = if positive { EC_WINDOW_END_INCLUSIVE } else { 0 };
            Some(SpriteWindow::new("EC window, ends inclusive", window, end_extra, 1.0))
        }
        BlockRole::LegacyArt => Some(SpriteWindow::new("CC window", &entry.cc_img_offset, 0, 1.0)),
        BlockRole::EnhancedArt => {
            Some(SpriteWindow::new("CC window, ends x1.5", &entry.cc_img_offset, 0, ENHANCED_WINDOW_END_SCALE))
        }
        BlockRole::Light | BlockRole::Unknown => None,
    }
}

/// The scroll offset that keeps the content point under `anchor` (relative to the view) in place when the zoom goes
/// from `old_zoom` to `new_zoom`.
pub(super) fn anchored_offset(offset: egui::Vec2, anchor: egui::Vec2, old_zoom: f32, new_zoom: f32) -> egui::Vec2 {
    ((offset + anchor) * (new_zoom / old_zoom) - anchor).max(egui::Vec2::ZERO)
}

/// The preview's zoom and overlay toggles, kept for the session.
#[derive(Clone, Copy, Debug)]
struct PreviewState {
    zoom: f32,
    overlays: Overlays,
    /// The scroll offset to apply next frame, after a zoom.
    pending_offset: Option<egui::Vec2>,
}

#[derive(Clone, Copy, Debug)]
struct Overlays {
    bounding_box: bool,
    offsets: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self { zoom: DEFAULT_ZOOM, overlays: Overlays { bounding_box: true, offsets: true }, pending_offset: None }
    }
}

fn load_state(ui: &egui::Ui) -> PreviewState {
    ui.data(|data| data.get_temp::<PreviewState>(egui::Id::new(PREVIEW_STATE_ID))).unwrap_or_default()
}

fn store_state(ui: &egui::Ui, state: PreviewState) {
    ui.data_mut(|data| data.insert_temp(egui::Id::new(PREVIEW_STATE_ID), state));
}

/// The zoom readout and its reset, and the overlay toggles when there are overlays.
fn controls(ui: &mut egui::Ui, state: &mut PreviewState, with_overlays: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Zoom {:.0}%", state.zoom * 100.0))
            .on_hover_text("Shift or Ctrl + wheel, or a pinch, over the image zooms; the wheel scrolls.");
        if ui.small_button("Reset").clicked() {
            state.zoom = DEFAULT_ZOOM;
            state.pending_offset = Some(egui::Vec2::ZERO);
        }
        if with_overlays {
            ui.separator();
            ui.checkbox(&mut state.overlays.bounding_box, egui::RichText::new("Bounding box").color(BOUNDING_BOX_COLOR))
                .on_hover_text("The window the client cuts from the art.");
            ui.checkbox(&mut state.overlays.offsets, egui::RichText::new("Offsets").color(OFFSETS_COLOR)).on_hover_text(
                "The point the window's x and y offsets are measured from (the cross), joined to the window's \
                 bottom-left corner.",
            );
        }
    });
}

/// A scroll area drawn at `state.zoom`; the zoom gesture over it zooms, anchored at the pointer.
fn zoomable(ui: &mut egui::Ui, state: &mut PreviewState, add_contents: impl FnOnce(&mut egui::Ui, f32)) {
    let mut area = egui::ScrollArea::both().id_salt(PREVIEW_STATE_ID).auto_shrink([false, false]);
    if let Some(offset) = state.pending_offset.take() {
        area = area.scroll_offset(offset);
    }
    let zoom = state.zoom;
    let output = area.show(ui, |ui| add_contents(ui, zoom));
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|_| ui.rect_contains_pointer(output.inner_rect))
    else {
        return;
    };
    let new_zoom = (zoom * ui.input(|input| input.zoom_delta())).clamp(MIN_ZOOM, MAX_ZOOM);
    if new_zoom != zoom {
        state.pending_offset = Some(anchored_offset(output.state.offset, pointer - output.inner_rect.min, zoom, new_zoom));
        state.zoom = new_zoom;
        ui.ctx().request_repaint();
    }
}

/// The art of a tiledata.mul tile, zoomable.
pub(super) fn art_preview(ui: &mut egui::Ui, art: Option<&egui::TextureHandle>) {
    let mut state = load_state(ui);
    ui.strong("Art");
    controls(ui, &mut state, false);
    match art {
        Some(art) => zoomable(ui, &mut state, |ui, zoom| {
            let size = art.size_vec2();
            let response = ui.add(image(art, size * zoom));
            let rect = response.rect;
            pixel_hover(response, rect, zoom);
            ui.weak(format!("{} x {}", size.x, size.y));
        }),
        None => {
            ui.weak("No art for this tile in the source chosen in the Art tab.");
        }
    }
    store_state(ui, state);
}

/// The textures of a tileart.uop record with their overlays.
pub(super) fn tileart_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, entry: &TileArtEntry) {
    let mut state = load_state(ui);
    ui.strong("Textures");
    controls(ui, &mut state, true);
    let (drawn, reason) = drawn_block(entry);
    ui.weak(format!("The client draws the {} block: {reason}.", block_role_label(drawn)));
    let Some(dictionary) = app.uo_string_dictionary.clone() else {
        ui.label("The texture names are in the string dictionary, which is not loaded.");
        store_state(ui, state);
        return;
    };
    let blocks = entry.process(&dictionary).texture_items;
    if blocks.is_empty() {
        ui.weak("This record names no texture.");
    }
    let overlays = state.overlays;
    zoomable(ui, &mut state, |ui, zoom| {
        for block in &blocks {
            let Some(first) = block.first() else { continue };
            let role = block_role(entry.version, first.block_index);
            let shader = entry
                .texture_vector
                .get(usize::from(first.block_index))
                .and_then(|block| block.type_string_off.checked_sub(1))
                .and_then(|offset| dictionary.get_string(offset as usize));
            let primary = primary_texture_position(shader);
            ui.horizontal(|ui| {
                ui.strong(format!("Block {}: {}", first.block_index, block_role_label(role)));
                if let Some(shader) = shader {
                    ui.weak(shader);
                }
                if let Some(scale) = block_scale(entry, role) {
                    ui.weak(format!("scale {scale}"));
                }
            });
            if role == BlockRole::Light {
                ui.weak(format!(
                    "light scale {}, offsets {}, {} (view space)",
                    entry.light_scale, entry.light_offset_x, entry.light_offset_y
                ));
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = TEXTURE_SPACING;
                for item in block {
                    ui.vertical(|ui| texture(app, ctx, ui, entry, item, item.item_index == primary, zoom, overlays));
                }
            });
            ui.separator();
        }
    });
    store_state(ui, state);
}

fn image(texture: &egui::TextureHandle, size: egui::Vec2) -> egui::Image<'_> {
    egui::Image::new(texture).fit_to_exact_size(size).texture_options(egui::TextureOptions::NEAREST).sense(egui::Sense::hover())
}

/// The pixel of the image drawn in `image_rect` under the pointer, as a tooltip.
fn pixel_hover(response: egui::Response, image_rect: egui::Rect, scale: f32) -> egui::Response {
    let Some(pointer) = response.hover_pos().filter(|pointer| image_rect.contains(*pointer)) else {
        return response;
    };
    let pixel = (pointer - image_rect.min) / scale;
    let (width, height) = (image_rect.width() / scale, image_rect.height() / scale);
    response.on_hover_text(format!("pixel {}, {} of {width} x {height}", pixel.x.floor(), pixel.y.floor()))
}

/// The part of a texture's plane its image and shown overlays cover, in image pixels: the overlays may reach outside
/// the image, and the room drawn for the texture holds them too.
fn overlay_extent(size: egui::Vec2, window: Option<&SpriteWindow>, overlays: Overlays) -> egui::Rect {
    let mut extent = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    if let Some(window) = window {
        if overlays.bounding_box && !window.is_empty() {
            extent = extent.union(egui::Rect::from_two_pos(window.min, window.max));
        }
        if overlays.offsets {
            extent = extent.union(egui::Rect::from_two_pos(window.origin(), egui::pos2(window.min.x, window.max.y)));
        }
    }
    extent
}

/// One texture of a record, with its overlays when its block has a window.
fn texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &TileArtEntry,
    item: &TextureItem,
    primary: bool,
    zoom: f32,
    overlays: Overlays,
) {
    let extra = item.id >= FIRST_EXTRA_TEXTURE_ID;
    ui.label(format!("{} {}", texture_type_label(item.texture_type), item.id)).on_hover_text(&item.path);
    ui.weak(format!("item {}{}, stretch {}", item.item_index, if primary { ", primary" } else { "" }, item.texture_stretch));
    let Some(handle) = app.tileart_item_texture(ctx, item.texture_type, item.id) else {
        ui.colored_label(WARNING_COLOR, "missing");
        return;
    };
    let scale = if extra { zoom * EXTRA_TEXTURE_SCALE } else { zoom };
    let size = handle.size_vec2();
    let window = if extra { None } else { sprite_window(entry, block_role(entry.version, item.block_index)) };
    let extent = overlay_extent(size, window.as_ref(), overlays);
    // The cross at the offsets' point reaches past the extent by its radius.
    let margin = egui::Vec2::splat(ORIGIN_MARK_RADIUS);
    let (room, response) = ui.allocate_exact_size(extent.size() * scale + margin + margin, egui::Sense::hover());
    let image_rect = egui::Rect::from_min_size(room.min + margin - extent.min.to_vec2() * scale, size * scale);
    image(&handle, size * scale).paint_at(ui, image_rect);
    pixel_hover(response, image_rect, scale);
    ui.weak(format!("{} x {}", size.x, size.y));
    let Some(window) = window else {
        return;
    };
    let to_screen = |point: egui::Pos2| image_rect.min + point.to_vec2() * scale;
    let painter = ui.painter();
    if overlays.bounding_box && !window.is_empty() {
        let bounds = egui::Rect::from_two_pos(to_screen(window.min), to_screen(window.max));
        let stroke = egui::Stroke::new(OVERLAY_STROKE_WIDTH, BOUNDING_BOX_COLOR);
        painter.rect_stroke(bounds, 0.0, stroke, egui::StrokeKind::Outside);
    }
    if overlays.offsets {
        let stroke = egui::Stroke::new(OVERLAY_STROKE_WIDTH, OFFSETS_COLOR);
        let origin = to_screen(window.origin());
        let corner = to_screen(egui::pos2(window.min.x, window.max.y));
        painter.line_segment([origin, egui::pos2(corner.x, origin.y)], stroke);
        painter.line_segment([egui::pos2(corner.x, origin.y), corner], stroke);
        painter.line_segment([origin - egui::vec2(ORIGIN_MARK_RADIUS, 0.0), origin + egui::vec2(ORIGIN_MARK_RADIUS, 0.0)], stroke);
        painter.line_segment([origin - egui::vec2(0.0, ORIGIN_MARK_RADIUS), origin + egui::vec2(0.0, ORIGIN_MARK_RADIUS)], stroke);
    }
    ui.weak(format!(
        "{}: {}, {} to {}, {}; offsets {}, {}",
        window.label, window.min.x, window.min.y, window.max.x, window.max.y, window.offset.x, window.offset.y
    ));
    if window.is_empty() {
        ui.weak("The window is empty: no box.");
    } else if !window.fits(handle.size()) {
        ui.colored_label(WARNING_COLOR, "The window is not inside this image.");
    }
}

fn texture_type_label(texture_type: TextureType) -> &'static str {
    match texture_type {
        TextureType::Undefined => "Undefined",
        TextureType::WorldArt => "WorldArt",
        TextureType::TileArtLegacy => "TileArtLegacy",
        TextureType::TileArtEnhanced => "TileArtEnhanced",
        TextureType::Textures => "Textures",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::tileart::TILEART_RECORD_VERSION;

    const IMAGE_SIZE: [usize; 2] = [64, 64];

    fn window(x_start: i32, y_start: i32, x_end: i32, y_end: i32, x_off: i32, y_off: i32) -> TaeImgOffset {
        TaeImgOffset { x_start, y_start, x_end, y_end, x_off, y_off }
    }

    #[test]
    fn blocks_take_their_role_by_position_and_version() {
        assert_eq!(block_role(TILEART_RECORD_VERSION, 0), BlockRole::WorldArt);
        assert_eq!(block_role(TILEART_RECORD_VERSION, 1), BlockRole::LegacyArt);
        assert_eq!(block_role(TILEART_RECORD_VERSION, 2), BlockRole::EnhancedArt);
        assert_eq!(block_role(TILEART_RECORD_VERSION, 3), BlockRole::Light);
        assert_eq!(block_role(TILEART_KR_RECORD_VERSION, 2), BlockRole::Light);
        assert_eq!(block_role(TILEART_KR_RECORD_VERSION, 3), BlockRole::Unknown);
    }

    /// 0xE75's windows: the EC window ends gain a pixel, the CC window is as stored, the enhanced one grows by half.
    #[test]
    fn each_art_block_is_cut_as_the_client_cuts_it() {
        let entry = TileArtEntry {
            ec_img_offset: window(0, 0, 25, 30, 19, -15),
            cc_img_offset: window(0, 0, 44, 32, 0, 0),
            ..TileArtEntry::default()
        };
        let world = sprite_window(&entry, BlockRole::WorldArt).unwrap();
        assert_eq!((world.min, world.max), (egui::pos2(0.0, 0.0), egui::pos2(26.0, 31.0)));
        assert_eq!(world.origin(), egui::pos2(-19.0, 46.0));
        let legacy = sprite_window(&entry, BlockRole::LegacyArt).unwrap();
        assert_eq!(legacy.max, egui::pos2(44.0, 32.0));
        assert_eq!(legacy.origin(), egui::pos2(0.0, 32.0));
        let enhanced = sprite_window(&entry, BlockRole::EnhancedArt).unwrap();
        assert_eq!(enhanced.max, egui::pos2(66.0, 48.0));
        assert_eq!(sprite_window(&entry, BlockRole::Light), None);
    }

    /// 0xE75's world art is 32 x 32 and its offsets' point lies left of and below it: the room drawn holds it.
    #[test]
    fn the_room_of_a_texture_holds_the_overlays_shown() {
        const WORLD_ART_SIZE: egui::Vec2 = egui::vec2(32.0, 32.0);
        let entry = TileArtEntry { ec_img_offset: window(0, 0, 25, 30, 19, -15), ..TileArtEntry::default() };
        let world = sprite_window(&entry, BlockRole::WorldArt).unwrap();
        let all = Overlays { bounding_box: true, offsets: true };
        let image = egui::Rect::from_min_size(egui::Pos2::ZERO, WORLD_ART_SIZE);
        assert_eq!(overlay_extent(WORLD_ART_SIZE, Some(&world), all), egui::Rect::from_min_max(egui::pos2(-19.0, 0.0), egui::pos2(32.0, 46.0)));
        assert_eq!(overlay_extent(WORLD_ART_SIZE, Some(&world), Overlays { offsets: false, ..all }), image);
        assert_eq!(overlay_extent(WORLD_ART_SIZE, None, all), image);
    }

    #[test]
    fn an_empty_ec_window_stays_empty() {
        let entry = TileArtEntry { ec_img_offset: window(0, 0, 0, 0, 0, 0), ..TileArtEntry::default() };
        assert!(sprite_window(&entry, BlockRole::WorldArt).unwrap().is_empty());
    }

    #[test]
    fn a_window_outside_its_image_does_not_fit() {
        let legacy = |x_start, y_start, x_end, y_end| {
            let entry = TileArtEntry { cc_img_offset: window(x_start, y_start, x_end, y_end, 0, 0), ..TileArtEntry::default() };
            sprite_window(&entry, BlockRole::LegacyArt).unwrap()
        };
        assert!(legacy(0, 0, 64, 64).fits(IMAGE_SIZE));
        assert!(!legacy(0, 0, 65, 64).fits(IMAGE_SIZE));
        assert!(!legacy(-1, 0, 64, 64).fits(IMAGE_SIZE));
    }

    #[test]
    fn the_drawn_block_follows_the_flags() {
        let mut entry = TileArtEntry::default();
        assert_eq!(drawn_block(&entry).0, BlockRole::LegacyArt);
        entry.uses_enhanced_tileart = true;
        assert_eq!(drawn_block(&entry).0, BlockRole::EnhancedArt);
        entry.flags2 = TaeFlag::UseNewArt;
        assert_eq!(drawn_block(&entry).0, BlockRole::WorldArt);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        const OLD_ZOOM: f32 = 2.0;
        const NEW_ZOOM: f32 = 3.0;
        let (offset, anchor) = (egui::vec2(40.0, 10.0), egui::vec2(25.0, 30.0));
        let content_point = (offset + anchor) / OLD_ZOOM;
        let new_offset = anchored_offset(offset, anchor, OLD_ZOOM, NEW_ZOOM);
        assert_eq!((new_offset + anchor) / NEW_ZOOM, content_point);
        assert_eq!(anchored_offset(egui::Vec2::ZERO, anchor, NEW_ZOOM, OLD_ZOOM), egui::Vec2::ZERO);
    }
}
