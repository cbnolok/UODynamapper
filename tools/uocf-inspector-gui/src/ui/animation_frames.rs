//! The frames of the selected action: a player of one direction above a grid with a row per stored direction and
//! a column per frame.

use crate::app::UopInspectorApp;
use crate::ui::animation_hue::{apply_frame_hue, FrameHue};
use crate::ui::image_export::{export_rgba_png, sanitize_file_stem};
use eframe::egui;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Duration;
use uocf::animation_sequence::{
    AnimationSequence, DIRECTION_COUNT, FIRST_FRAME_NUMBER, FIRST_STORED_DIRECTION, MIRROR_DIRECTION_SUM,
    STORED_DIRECTION_COUNT,
};
use uocf::classic::anim::AnimFrame;

/// World directions, numbered as the clients number them (0 is north, clockwise).
const COMPASS: [&str; DIRECTION_COUNT] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
const PLAYER_ZOOMS: RangeInclusive<f32> = 0.5..=8.0;
const PLAYER_DEFAULT_ZOOM: f32 = 2.0;
const GRID_ZOOMS: RangeInclusive<f32> = 0.125..=4.0;
const GRID_DEFAULT_ZOOM: f32 = 0.5;
/// Share of the central panel's height the player may take; the grid gets the rest.
const PLAYER_HEIGHT_SHARE: f32 = 0.45;
const ROW_LABEL_WIDTH: f32 = 96.0;
/// Frames are shown numbered from 1, in the player and the grid.
const DISPLAYED_FIRST_FRAME: usize = 1;
/// Seconds a frame shows at speed 1, as the player always did.
const BASE_FRAME_SECONDS: f64 = 0.1;
const CELL_ROUNDING: f32 = 2.0;
const SELECTED_STROKE_WIDTH: f32 = 2.0;
/// Texture coordinates of a horizontally mirrored image.
const MIRRORED_UV: egui::Rect = egui::Rect { min: egui::pos2(1.0, 0.0), max: egui::pos2(0.0, 1.0) };

/// One decoded frame.
pub(super) struct FrameImage {
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub rgba: Vec<u8>,
    /// Enhanced frames: the pixels whose palette colour takes the hue.
    pub hue_mask: Option<Vec<u8>>,
    /// Enhanced frames: the 1-based local frame number.
    pub number: Option<u16>,
}

impl FrameImage {
    fn from_classic(frame: AnimFrame) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            center_x: frame.center_x,
            center_y: frame.center_y,
            rgba: frame.data,
            hue_mask: None,
            number: None,
        }
    }

    fn size(&self) -> egui::Vec2 {
        egui::vec2(f32::from(self.width), f32::from(self.height))
    }
}

/// The frames of one stored direction, as indices into `ActionFrames::images` (`None`: a frame number missing).
pub(super) struct DirectionRow {
    /// The stored direction in the source's numbering (Classic 0..5, Enhanced 3..8).
    pub direction: u8,
    pub frames: Vec<Option<usize>>,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct ActionFrames {
    pub images: Vec<FrameImage>,
    pub rows: Vec<DirectionRow>,
    pub is_ec: bool,
    /// Enhanced: frames whose number falls in no direction.
    pub unplaced: usize,
    /// How the frames were split into directions.
    pub split: String,
    /// Enhanced: frames that failed to decode.
    pub failed: usize,
    /// Why there are no frames at all.
    pub error: Option<String>,
}

impl ActionFrames {
    pub(super) fn from_error(error: String) -> Self {
        Self { error: Some(error), ..Self::default() }
    }

    /// The Classic frames of the stored directions, decoded per direction by `decode`.
    pub(super) fn classic(decode: impl Fn(u8) -> Option<color_eyre::eyre::Result<Vec<AnimFrame>>>, split: String) -> Self {
        let mut action = Self { split, ..Self::default() };
        for direction in 0..STORED_DIRECTION_COUNT as u8 {
            let mut row = DirectionRow { direction, frames: Vec::new(), error: None };
            match decode(direction) {
                None => row.error = Some("no entry".to_string()),
                Some(Err(error)) => row.error = Some(format!("{error:#}")),
                Some(Ok(frames)) => {
                    for frame in frames {
                        row.frames.push(Some(action.images.len()));
                        action.images.push(FrameImage::from_classic(frame));
                    }
                }
            }
            action.rows.push(row);
        }
        action
    }

    /// The frames of an Enhanced AnimationFrame entry, placed by their frame numbers in the directions of
    /// `sequence` (its entry for the body and `action`), else in an even split of the highest number.
    pub(super) fn enhanced(payload: &[u8], sequence: Option<&AnimationSequence>, action: u16) -> color_eyre::eyre::Result<Self> {
        let animation = uocf::enhanced::animationframe::AnimationFrame::load(payload)?;
        let mut frames = Self { is_ec: true, ..Self::default() };
        let mut numbers = Vec::new();
        for entry in &animation.frames {
            match animation.decode_frame(entry) {
                Ok(decoded) => {
                    numbers.push(entry.frame_number);
                    frames.images.push(FrameImage {
                        width: decoded.width,
                        height: decoded.height,
                        center_x: decoded.center_x,
                        center_y: decoded.center_y,
                        rgba: decoded.data,
                        hue_mask: Some(decoded.hue_mask),
                        number: Some(entry.frame_number),
                    });
                }
                Err(error) => {
                    log::warn!("frame {} of action {action}: {error:#}", entry.frame_number);
                    frames.failed += 1;
                }
            }
        }
        let (spans, from_sequence) = ec_direction_spans(sequence, action, &numbers);
        frames.split = if from_sequence {
            "frames per direction from the AnimationSequence entry".to_string()
        } else {
            format!("no AnimationSequence entry: the highest frame number split in {STORED_DIRECTION_COUNT}")
        };
        let (rows, unplaced) = place_frames(&numbers, &spans);
        frames.unplaced = unplaced;
        frames.rows = rows
            .into_iter()
            .enumerate()
            .map(|(row, frames)| DirectionRow { direction: FIRST_STORED_DIRECTION + row as u8, frames, error: None })
            .collect();
        Ok(frames)
    }

    fn largest(&self, row: Option<&DirectionRow>) -> egui::Vec2 {
        let indices: Box<dyn Iterator<Item = usize>> = match row {
            Some(row) => Box::new(row.frames.iter().flatten().copied()),
            None => Box::new(0..self.images.len()),
        };
        indices.fold(egui::Vec2::ZERO, |largest, index| largest.max(self.images[index].size()))
    }
}

/// First frame number and frame count of each stored direction: from the sequence's action when it has one, else
/// the highest frame number split evenly. The flag says which.
pub(super) fn ec_direction_spans(sequence: Option<&AnimationSequence>, action: u16, numbers: &[u16]) -> (Vec<(u32, u32)>, bool) {
    let from_sequence: Option<Vec<(u32, u32)>> = sequence.and_then(|sequence| {
        (FIRST_STORED_DIRECTION..DIRECTION_COUNT as u8)
            .map(|direction| {
                let frames = sequence.direction_frames(u32::from(action), direction)?;
                Some((frames.first_frame_number, frames.count))
            })
            .collect()
    });
    if let Some(spans) = from_sequence {
        return (spans, true);
    }
    let highest = numbers.iter().copied().max().map_or(0, u32::from);
    let per_direction = highest.div_ceil(STORED_DIRECTION_COUNT);
    let spans = (0..STORED_DIRECTION_COUNT).map(|row| (row * per_direction + FIRST_FRAME_NUMBER, per_direction)).collect();
    (spans, false)
}

/// The frames of each span by number (frame numbers can have gaps: a missing number is `None`), and how many
/// frames fall in no span.
pub(super) fn place_frames(numbers: &[u16], spans: &[(u32, u32)]) -> (Vec<Vec<Option<usize>>>, usize) {
    let by_number: HashMap<u32, usize> = numbers.iter().enumerate().map(|(index, &number)| (u32::from(number), index)).collect();
    let rows: Vec<Vec<Option<usize>>> =
        spans.iter().map(|&(first, count)| (first..first + count).map(|number| by_number.get(&number).copied()).collect()).collect();
    let placed = rows.iter().flatten().flatten().count();
    (rows, numbers.len() - placed)
}

/// The world direction of stored direction `stored`. The Enhanced client numbers stored directions as world
/// directions; Classic stores world direction `FIRST_STORED_DIRECTION + k` at `k` (ClassicUO's mapping: not traced).
pub(super) fn world_direction(is_ec: bool, stored: u8) -> u8 {
    if is_ec { stored } else { stored + FIRST_STORED_DIRECTION }
}

/// Between the parts of a direction label; the grid puts them on lines of their own.
const LABEL_SEPARATOR: &str = " · ";

/// "3 SE", plus the world direction drawn as its mirror.
pub(super) fn direction_label(is_ec: bool, stored: u8) -> String {
    let world = world_direction(is_ec, stored);
    let Some(name) = COMPASS.get(usize::from(world)) else {
        return stored.to_string();
    };
    match MIRROR_DIRECTION_SUM.checked_sub(world).filter(|&mirror| mirror < FIRST_STORED_DIRECTION && mirror != world) {
        Some(mirror) => format!("{stored} {name}{LABEL_SEPARATOR}{} mirrored", COMPASS[usize::from(mirror)]),
        None => format!("{stored} {name}"),
    }
}

/// The grid row of the selected direction, and whether the client draws it mirrored. Enhanced directions below
/// `FIRST_STORED_DIRECTION` show the stored direction `MIRROR_DIRECTION_SUM - d` flipped.
pub(super) fn row_of_direction(is_ec: bool, direction: u8) -> Option<(usize, bool)> {
    let stored_count = STORED_DIRECTION_COUNT as u8;
    if !is_ec {
        return (direction < stored_count).then_some((usize::from(direction), false));
    }
    if direction < FIRST_STORED_DIRECTION {
        return Some((usize::from(MIRROR_DIRECTION_SUM - direction - FIRST_STORED_DIRECTION), true));
    }
    (direction < FIRST_STORED_DIRECTION + stored_count).then_some((usize::from(direction - FIRST_STORED_DIRECTION), false))
}

/// The scroll offset that keeps the content under the pointer in place when the content scales by `factor`.
pub(super) fn anchored_offset(pointer: egui::Vec2, offset: egui::Vec2, factor: f32) -> egui::Vec2 {
    ((offset + pointer) * factor - pointer).max(egui::Vec2::ZERO)
}

/// Zoom of a scrolled view: `zoom_delta` (pinch, Shift or Ctrl + wheel) changes it while the pointer is over the view,
/// anchored at the pointer. Plain wheel scrolling is left to the scroll area.
#[derive(Clone, Copy)]
pub(super) struct AnchoredZoom {
    pub zoom: f32,
    viewport: Option<egui::Rect>,
    offset: egui::Vec2,
    /// Another action is shown: it opens at the top left.
    scroll_to_start: bool,
}

impl AnchoredZoom {
    const fn new(zoom: f32) -> Self {
        Self { zoom, viewport: None, offset: egui::Vec2::ZERO, scroll_to_start: false }
    }

    /// Applies this frame's gesture; returns the scroll offset to show it with.
    fn update(&mut self, ui: &egui::Ui, range: RangeInclusive<f32>) -> Option<egui::Vec2> {
        if std::mem::take(&mut self.scroll_to_start) {
            self.offset = egui::Vec2::ZERO;
            return Some(egui::Vec2::ZERO);
        }
        let viewport = self.viewport?;
        let pointer = ui.input(|input| input.pointer.hover_pos())?;
        if !ui.rect_contains_pointer(viewport) {
            return None;
        }
        let delta = ui.input(|input| input.zoom_delta());
        let zoom = (self.zoom * delta).clamp(*range.start(), *range.end());
        if zoom == self.zoom {
            return None;
        }
        let offset = anchored_offset(pointer - viewport.min, self.offset, zoom / self.zoom);
        self.zoom = zoom;
        Some(offset)
    }

    fn record<R>(&mut self, output: &egui::scroll_area::ScrollAreaOutput<R>) {
        self.viewport = Some(output.inner_rect);
        self.offset = output.state.offset;
    }
}

pub(crate) struct FramesViewState {
    /// The decoded action and the hash of what it was decoded from.
    decoded: Option<(u64, Arc<ActionFrames>)>,
    /// Textures of the decoded images, for `textures_key`.
    textures: HashMap<usize, Option<egui::TextureHandle>>,
    textures_key: u64,
    /// The image last given to the app's image preview.
    previewed: Option<u64>,
    player_zoom: AnchoredZoom,
    grid_zoom: AnchoredZoom,
}

impl Default for FramesViewState {
    fn default() -> Self {
        Self {
            decoded: None,
            textures: HashMap::new(),
            textures_key: 0,
            previewed: None,
            player_zoom: AnchoredZoom::new(PLAYER_DEFAULT_ZOOM),
            grid_zoom: AnchoredZoom::new(GRID_DEFAULT_ZOOM),
        }
    }
}

/// The decoded frames for `key` (hash of everything they are decoded from), decoding with `decode` on a change.
pub(super) fn decoded_frames(app: &mut UopInspectorApp, key: u64, decode: impl FnOnce(&mut UopInspectorApp) -> ActionFrames) -> Arc<ActionFrames> {
    if let Some((cached, frames)) = &app.animations_view.frames.decoded {
        if *cached == key {
            return frames.clone();
        }
    }
    let frames = Arc::new(decode(app));
    let view = &mut app.animations_view.frames;
    view.decoded = Some((key, frames.clone()));
    view.player_zoom.scroll_to_start = true;
    view.grid_zoom.scroll_to_start = true;
    frames
}

/// The hue the frames are drawn with (see `animation_hue::frame_hue`) and its key.
pub(super) type DrawnHue = (FrameHue, u64);

/// What the textures of the decoded action are drawn with: the decoded frames and the hue. A change drops them.
fn texture_key(app: &UopInspectorApp, hue: &DrawnHue) -> u64 {
    let decoded = app.animations_view.frames.decoded.as_ref().map_or(0, |(key, _)| *key);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (decoded, hue.1).hash(&mut hasher);
    hasher.finish()
}

/// The pixels `image` is drawn with: its own under the hue.
fn drawn_rgba(image: &FrameImage, hue: &DrawnHue) -> Vec<u8> {
    let mut rgba = image.rgba.clone();
    apply_frame_hue(&mut rgba, image.hue_mask.as_deref(), &hue.0);
    rgba
}

fn frame_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    frames: &ActionFrames,
    index: usize,
    hue: &DrawnHue,
) -> Option<egui::TextureHandle> {
    let key = texture_key(app, hue);
    let state = &mut app.animations_view.frames;
    if state.textures_key != key {
        state.textures.clear();
        state.textures_key = key;
    }
    if let Some(texture) = state.textures.get(&index) {
        return texture.clone();
    }
    let image = &frames.images[index];
    let texture = (image.width > 0 && image.height > 0)
        .then(|| {
            let size = [usize::from(image.width), usize::from(image.height)];
            app.checked_texture(ctx, format!("animation_frame_{key:016x}_{index}"), size, &drawn_rgba(image, hue), egui::TextureOptions::NEAREST)
        })
        .flatten();
    app.animations_view.frames.textures.insert(index, texture.clone());
    texture
}

/// Paints `texture` at `zoom` with its bottom centre on the bottom centre of `slot`.
fn paint_bottom_centred(ui: &egui::Ui, texture: &egui::TextureHandle, slot: egui::Rect, zoom: f32, mirrored: bool) {
    let size = texture.size_vec2() * zoom;
    let rect = egui::Rect::from_min_size(slot.center_bottom() - egui::vec2(size.x / 2.0, size.y), size);
    let mut image = egui::Image::new(texture).texture_options(egui::TextureOptions::NEAREST);
    if mirrored {
        image = image.uv(MIRRORED_UV);
    }
    image.paint_at(ui, rect);
}

/// Advances the player while it plays, over `frame_count` frames.
fn advance_playback(app: &mut UopInspectorApp, ctx: &egui::Context, frame_count: usize) {
    if !app.is_playing || frame_count == 0 {
        return;
    }
    let time = ctx.input(|input| input.time);
    let frame_delay = BASE_FRAME_SECONDS / f64::from(app.playback_speed);
    if time - app.last_frame_time > frame_delay {
        app.current_frame_idx += 1;
        if app.current_frame_idx >= frame_count {
            if app.loop_animation {
                app.current_frame_idx = 0;
            } else {
                app.current_frame_idx = frame_count - 1;
                app.is_playing = false;
            }
        }
        app.last_frame_time = time;
    }
    // egui draws only on input: wake up for the next frame while playing.
    if app.is_playing {
        let wait = (app.last_frame_time + frame_delay - time).max(0.0);
        ctx.request_repaint_after(Duration::from_secs_f64(wait));
    }
}

/// The selected direction's frames, one at a time, at the player zoom; `name` names exported files.
pub(super) fn show_player(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    frames: &ActionFrames,
    hue: &DrawnHue,
    name: &str,
) {
    let Some((row_index, mirrored)) = row_of_direction(frames.is_ec, app.selected_direction).filter(|(row, _)| *row < frames.rows.len()) else {
        ui.label(format!("Direction {} is not one of this source's directions.", app.selected_direction));
        return;
    };
    let row = &frames.rows[row_index];
    advance_playback(app, ctx, row.frames.len());
    let frame_count = row.frames.len();
    if frame_count == 0 {
        ui.label(format!("Direction {}: {}", direction_label(frames.is_ec, row.direction), row.error.as_deref().unwrap_or("no frames")));
        return;
    }
    let frame_index = app.current_frame_idx.min(frame_count - 1);
    let image_index = row.frames[frame_index];
    let mirror_note = if mirrored { format!(" (drawn mirrored as {})", app.selected_direction) } else { String::new() };
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "Direction {}{mirror_note} · frame {} / {frame_count}",
            direction_label(frames.is_ec, row.direction),
            frame_index + DISPLAYED_FIRST_FRAME
        ));
        if let Some(image) = image_index.map(|index| &frames.images[index]) {
            ui.label(format!("· {}x{}, center {},{}", image.width, image.height, image.center_x, image.center_y));
            if let Some(number) = image.number {
                ui.label(format!("· frame number {number}"));
            }
            if image.width > 0 && image.height > 0 && ui.button("Export PNG").clicked() {
                let stem = sanitize_file_stem(&format!("{name}_dir_{}_frame_{frame_index}", row.direction), "animation_frame");
                let rgba = drawn_rgba(image, hue);
                match export_rgba_png(format!("{stem}.png"), u32::from(image.width), u32::from(image.height), &rgba) {
                    Ok(Some(path)) => app.status_message = format!("Exported animation frame to {path}."),
                    Ok(None) => {}
                    Err(error) => app.status_message = format!("Failed to export animation frame: {error}"),
                }
            }
        }
        ui.weak(format!("zoom {:.0}% (Shift or Ctrl + wheel)", app.animations_view.frames.player_zoom.zoom * 100.0));
    });

    let zoom_offset = app.animations_view.frames.player_zoom.update(ui, PLAYER_ZOOMS);
    let zoom = app.animations_view.frames.player_zoom.zoom;
    let largest = frames.largest(Some(row)) * zoom;
    let mut area = egui::ScrollArea::both()
        .id_salt("animation_player")
        .auto_shrink([false, true])
        .max_height(ui.available_height() * PLAYER_HEIGHT_SHARE);
    if let Some(offset) = zoom_offset {
        area = area.scroll_offset(offset);
    }
    let texture = image_index.and_then(|index| frame_texture(app, ctx, frames, index, hue));
    let output = area.show(ui, |ui| {
        let (slot, _) = ui.allocate_exact_size(largest, egui::Sense::hover());
        match &texture {
            Some(texture) => paint_bottom_centred(ui, texture, slot, zoom, mirrored),
            None => {
                ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, "(no image)", egui::FontId::default(), ui.visuals().weak_text_color());
            }
        }
    });
    app.animations_view.frames.player_zoom.record(&output);

    if let (Some(index), Some(_)) = (image_index, &texture) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        ("animation_player", texture_key(app, hue), index).hash(&mut hasher);
        let preview_key = hasher.finish();
        if app.animations_view.frames.previewed == Some(preview_key) {
            app.select_image_preview(preview_key);
        } else {
            let image = &frames.images[index];
            let label = format!("{name} direction {} frame {frame_index}", row.direction);
            app.register_current_image_preview(preview_key, label, u32::from(image.width), u32::from(image.height), &drawn_rgba(image, hue));
            app.animations_view.frames.previewed = Some(preview_key);
        }
    }
}

/// Every stored direction as a row of frame thumbnails; a click shows that frame in the player.
pub(super) fn show_grid(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, frames: &ActionFrames, hue: &DrawnHue) {
    let selected = row_of_direction(frames.is_ec, app.selected_direction).map(|(row, _)| row);
    ui.horizontal(|ui| {
        ui.label("All directions");
        ui.weak(format!("· zoom {:.0}% (Shift or Ctrl + wheel) · click a frame to play it", app.animations_view.frames.grid_zoom.zoom * 100.0));
    });
    let zoom_offset = app.animations_view.frames.grid_zoom.update(ui, GRID_ZOOMS);
    let zoom = app.animations_view.frames.grid_zoom.zoom;
    let cell = (frames.largest(None) * zoom).max(egui::Vec2::splat(ui.spacing().interact_size.y));
    let mut area = egui::ScrollArea::both().id_salt("animation_frame_grid").auto_shrink([false, false]);
    if let Some(offset) = zoom_offset {
        area = area.scroll_offset(offset);
    }
    let mut clicked = None;
    let output = area.show(ui, |ui| {
        for (row_index, row) in frames.rows.iter().enumerate() {
            ui.horizontal(|ui| {
                let label = match &row.error {
                    Some(error) => format!("{}\n{error}", direction_label(frames.is_ec, row.direction)),
                    None => format!("{}\n{} frames", direction_label(frames.is_ec, row.direction), row.frames.len()),
                };
                let label = label.replace(LABEL_SEPARATOR, "\n");
                let size = egui::vec2(ROW_LABEL_WIDTH, cell.y);
                ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_size(size);
                    ui.add(egui::Label::new(label).wrap());
                });
                for (column, image_index) in row.frames.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(cell, egui::Sense::click());
                    let is_selected = selected == Some(row_index) && app.current_frame_idx.min(row.frames.len() - 1) == column;
                    ui.painter().rect_filled(rect, CELL_ROUNDING, ui.visuals().extreme_bg_color);
                    match image_index.and_then(|index| frame_texture(app, ctx, frames, index, hue).map(|texture| (index, texture))) {
                        Some((index, texture)) => {
                            paint_bottom_centred(ui, &texture, rect, zoom, false);
                            let image = &frames.images[index];
                            let number = image.number.map(|number| format!(", number {number}")).unwrap_or_default();
                            let shown = column + DISPLAYED_FIRST_FRAME;
                            response.clone().on_hover_text(format!("Frame {shown}{number}: {}x{}", image.width, image.height));
                        }
                        None => {
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "gap", egui::FontId::default(), ui.visuals().weak_text_color());
                            let shown = column + DISPLAYED_FIRST_FRAME;
                            response.clone().on_hover_text(format!("Frame {shown}: no frame with this number"));
                        }
                    }
                    if is_selected {
                        let stroke = egui::Stroke::new(SELECTED_STROKE_WIDTH, ui.visuals().selection.stroke.color);
                        ui.painter().rect_stroke(rect, CELL_ROUNDING, stroke, egui::StrokeKind::Inside);
                    }
                    if response.clicked() {
                        clicked = Some((row.direction, column));
                    }
                }
            });
        }
    });
    app.animations_view.frames.grid_zoom.record(&output);
    if let Some((direction, column)) = clicked {
        app.selected_direction = direction;
        app.current_frame_idx = column;
        app.is_playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_frames_go_by_number_and_a_missing_number_leaves_a_gap() {
        let numbers = [1, 2, 3, 4, 5, 6, 8, 9, 10];
        let (spans, from_sequence) = ec_direction_spans(None, 0, &numbers);
        assert!(!from_sequence);
        assert_eq!(spans, vec![(1, 2), (3, 2), (5, 2), (7, 2), (9, 2)]);
        let (rows, unplaced) = place_frames(&numbers, &spans);
        assert_eq!(rows[0], vec![Some(0), Some(1)]);
        assert_eq!(rows[3], vec![None, Some(6)], "number 7 is missing");
        assert_eq!(rows[4], vec![Some(7), Some(8)]);
        assert_eq!(unplaced, 0);
        let (_, unplaced) = place_frames(&[1, 2, 40], &[(1, 2)]);
        assert_eq!(unplaced, 1);
    }

    #[test]
    fn rows_follow_the_stored_directions_and_mirrors_reuse_them() {
        assert_eq!(row_of_direction(true, 3), Some((0, false)));
        assert_eq!(row_of_direction(true, 7), Some((4, false)));
        assert_eq!(row_of_direction(true, 0), Some((3, true)), "north is west mirrored");
        assert_eq!(row_of_direction(true, 2), Some((1, true)), "east is south mirrored");
        assert_eq!(row_of_direction(true, 8), None);
        assert_eq!(row_of_direction(false, 0), Some((0, false)));
        assert_eq!(row_of_direction(false, 5), None);
        assert_eq!(direction_label(true, 4), "4 S · E mirrored");
        assert_eq!(direction_label(false, 1), "1 S · E mirrored");
        assert_eq!(direction_label(true, 7), "7 NW");
    }

    #[test]
    fn zooming_keeps_the_content_under_the_pointer() {
        let pointer = egui::vec2(100.0, 50.0);
        let offset = egui::vec2(20.0, 10.0);
        let zoomed = anchored_offset(pointer, offset, 2.0);
        // The content point under the pointer was (120, 60); at twice the size it is at (240, 120).
        assert_eq!(zoomed + pointer, egui::vec2(240.0, 120.0));
        assert_eq!(anchored_offset(pointer, egui::Vec2::ZERO, 0.5), egui::Vec2::ZERO, "the offset never goes negative");
    }
}
