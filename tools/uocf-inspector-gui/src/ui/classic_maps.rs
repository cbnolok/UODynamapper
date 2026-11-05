//! Read-only view of a Classic facet: a radar-colour render of the map and its statics (`radarcol.mul`), with the
//! land tile and the statics of the tile under the pointer, and of a clicked tile.
//!
//! A tile takes the colour of its highest static (by its top, `z` plus the `tiledata.mul` height, then by `z`)
//! when that static's top is not below the land, else the land's colour, as `udd_conv::cc_radar` picks.
//!
//! The map zooms at the pointer and can move to a window of its own, which stays open while other tabs are in use.
//!
//! The facet's files are not edited here; [`super::classic_maps_convert`] converts and saves a whole facet.

use color_eyre::eyre::{self, WrapErr};
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use uocf::classic::facet_sources::{CLASSIC_FACET_NAMES, ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
use uocf::classic::map::{MapBlockRelPos, MapPlane};
use uocf::classic::radarcol::load_radarcol;
use uocf::classic::statics::{PackedStaticTile, StaticsStore, UO_BLOCK_DIM};
use uocf::classic::tiledata::TileData;
use uocf::map_edition::MapEdition;
use uocf::utils::color::Rgb555;
use uocf::utils::path::find_file_ignoring_case;

use crate::app::UopInspectorApp;

const RADARCOL_FILE: &str = "radarcol.mul";
const TILEDATA_FILE: &str = "tiledata.mul";
/// `radarcol.mul` holds the land colours, then the item colours from this index on.
const RADARCOL_ITEM_BASE: usize = 0x4000;
/// MUL first, as the radar builders read it; a UOP map when there is no MUL.
const MAP_FORMATS: [ClassicMapFormat; 2] = [ClassicMapFormat::Mul, ClassicMapFormat::Uop];
const RGBA_CHANNELS: usize = 4;
const OPAQUE: u8 = 255;
/// Shift of a 5-bit channel to 8 bits, and of its high bits into the low ones.
const CHANNEL_SHIFT: u32 = 3;
const CHANNEL_REFILL_SHIFT: u32 = 2;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 16.0;
const DEFAULT_ZOOM: f32 = 0.25;
/// Zoom factor of one press of the zoom buttons.
const ZOOM_STEP: f32 = 1.25;
const ACTUAL_SIZE_ZOOM: f32 = 1.0;
const TOOLS_PANEL_WIDTH: f32 = 300.0;
const MAP_WINDOW_ID: &str = "uocf_classic_maps_window";
const MAP_WINDOW_WIDTH: f32 = 1000.0;
const MAP_WINDOW_HEIGHT: f32 = 700.0;

/// A facet read and drawn.
struct LoadedFacet {
    facet: u32,
    sources: String,
    width: u32,
    height: u32,
    /// Tiles per texture pixel (1 unless the facet is larger than the GPU's largest texture).
    scale: u32,
    plane: MapPlane,
    statics: StaticsStore,
    tiledata: Option<Arc<TileData>>,
}

pub struct ClassicMapsState {
    facet: u32,
    map_difs: bool,
    static_difs: bool,
    zoom: f32,
    loaded: Option<LoadedFacet>,
    texture: Option<egui::TextureHandle>,
    worker: Option<mpsc::Receiver<Result<(LoadedFacet, egui::ColorImage), String>>>,
    error: Option<String>,
    pinned: Option<(u32, u32)>,
    /// The map is in its own window.
    detached: bool,
    /// The scroll offset for the next frame, after a zoom change.
    pending_offset: Option<egui::Vec2>,
    convert: super::classic_maps_convert::ConvertState,
}

impl Default for ClassicMapsState {
    fn default() -> Self {
        Self {
            facet: 0,
            map_difs: false,
            static_difs: false,
            zoom: DEFAULT_ZOOM,
            loaded: None,
            texture: None,
            worker: None,
            error: None,
            pinned: None,
            detached: false,
            pending_offset: None,
            convert: Default::default(),
        }
    }
}

/// The scroll offset that keeps the tile at `anchor` (from the top left of the view) in place when the zoom changes.
fn anchored_offset(offset: egui::Vec2, anchor: egui::Vec2, old_zoom: f32, new_zoom: f32) -> egui::Vec2 {
    (offset + anchor) / old_zoom * new_zoom - anchor
}

/// The zoom at which a whole facet fits a view.
fn fit_zoom(view: egui::Vec2, width: u32, height: u32) -> f32 {
    (view.x / width.max(1) as f32).min(view.y / height.max(1) as f32).clamp(MIN_ZOOM, MAX_ZOOM)
}

fn expand_channel(channel: u8) -> u8 {
    (channel << CHANNEL_SHIFT) | (channel >> CHANNEL_REFILL_SHIFT)
}

fn radar_rgba(color: Option<&Rgb555>) -> [u8; RGBA_CHANNELS] {
    color.map_or([0, 0, 0, OPAQUE], |color| [expand_channel(color.r()), expand_channel(color.g()), expand_channel(color.b()), OPAQUE])
}

fn item_height(tiledata: Option<&TileData>, graphic: u16) -> i16 {
    tiledata.and_then(|tiledata| tiledata.item_tiles().get(usize::from(graphic))).map_or(0, |tile| i16::from(tile.height_raw()))
}

/// The static drawn on the radar at a cell of a block: the highest top, then the highest `z`.
fn radar_static(tiles: &[PackedStaticTile], cell_x: u32, cell_y: u32, tiledata: Option<&TileData>) -> Option<PackedStaticTile> {
    tiles
        .iter()
        .copied()
        .filter(|tile| u32::from(tile.x_offset()) == cell_x && u32::from(tile.y_offset()) == cell_y)
        .max_by_key(|tile| (i16::from(tile.z) + item_height(tiledata, tile.graphic), tile.z))
}

fn load_facet(cc_path: PathBuf, facet: u32, difs: ClassicDifOptions, tiledata: Option<Arc<TileData>>, max_side: u32) -> eyre::Result<(LoadedFacet, egui::ColorImage)> {
    let dirs = [cc_path.clone()];
    let sources = ClassicFacetSources::resolve(&dirs, facet, &MAP_FORMATS, difs, MapEdition::Standard)?;
    let mut plane = sources.load_map_plane(None, None)?;
    let size = plane.size_cells();
    let statics = sources.load_statics_reader(size.width, size.height, None)?.load_all()?;
    let radarcol_path = find_file_ignoring_case(&cc_path, RADARCOL_FILE).ok_or_else(|| eyre::eyre!("no {RADARCOL_FILE} in {}", cc_path.display()))?;
    let radarcol = load_radarcol(&radarcol_path).wrap_err_with(|| radarcol_path.display().to_string())?;
    let tiledata = match tiledata {
        Some(tiledata) => Some(tiledata),
        None => find_file_ignoring_case(&cc_path, TILEDATA_FILE).and_then(|path| TileData::load(path).ok()).map(Arc::new),
    };

    let scale = size.width.max(size.height).div_ceil(max_side.max(1)).max(1);
    let (image_width, image_height) = (size.width.div_ceil(scale), size.height.div_ceil(scale));
    let mut rgba = vec![0u8; image_width as usize * image_height as usize * RGBA_CHANNELS];
    let blocks_y = size.height / UO_BLOCK_DIM;
    for block_x in 0..size.width / UO_BLOCK_DIM {
        let mut column: Vec<MapBlockRelPos> = (0..blocks_y).map(|y| MapBlockRelPos { x: block_x, y }).collect();
        plane.load_blocks(&mut column)?;
        for block_y in 0..blocks_y {
            let block = plane.block(MapBlockRelPos { x: block_x, y: block_y }).ok_or_else(|| eyre::eyre!("block {block_x},{block_y} not read"))?;
            let tiles = statics.block_tiles(block_x, block_y);
            for cell_y in 0..UO_BLOCK_DIM {
                for cell_x in 0..UO_BLOCK_DIM {
                    let (x, y) = (block_x * UO_BLOCK_DIM + cell_x, block_y * UO_BLOCK_DIM + cell_y);
                    if x % scale != 0 || y % scale != 0 {
                        continue;
                    }
                    let land = block.cell(cell_x, cell_y)?;
                    let color = match radar_static(tiles, cell_x, cell_y, tiledata.as_deref()) {
                        Some(tile) if i16::from(tile.z) + item_height(tiledata.as_deref(), tile.graphic) >= i16::from(land.z) => {
                            radarcol.get(RADARCOL_ITEM_BASE + usize::from(tile.graphic))
                        }
                        _ => radarcol.get(usize::from(land.id)),
                    };
                    let at = ((y / scale) as usize * image_width as usize + (x / scale) as usize) * RGBA_CHANNELS;
                    rgba[at..at + RGBA_CHANNELS].copy_from_slice(&radar_rgba(color));
                }
            }
        }
        plane.evict_idle_blocks(std::time::Duration::ZERO);
    }
    let mut label = format!("{} ({})", sources.map.path.display(), sources.map.format.label());
    if let Some(files) = &sources.statics {
        label.push_str(&format!(", {}", files.statics.display()));
    }
    if sources.map_difs.is_some() || sources.static_difs.is_some() {
        label.push_str(", difs");
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([image_width as usize, image_height as usize], &rgba);
    Ok((LoadedFacet { facet, sources: label, width: size.width, height: size.height, scale, plane, statics, tiledata }, image))
}

fn facet_name(facet: u32) -> String {
    CLASSIC_FACET_NAMES.get(facet as usize).map_or_else(|| format!("map{facet}"), |name| format!("{facet}: {name}"))
}

fn poll_worker(state: &mut ClassicMapsState, ctx: &egui::Context) {
    let Some(worker) = &state.worker else { return };
    match worker.try_recv() {
        Ok(result) => {
            state.worker = None;
            match result {
                Ok((loaded, image)) => {
                    match super::textures::load_color_image(ctx, format!("classic_facet_{}", loaded.facet), image, egui::TextureOptions::NEAREST) {
                        Ok(texture) => {
                            state.texture = Some(texture);
                            state.error = None;
                        }
                        Err(problem) => {
                            state.texture = None;
                            state.error = Some(format!("Cannot show the facet: {problem}."));
                        }
                    }
                    state.loaded = Some(loaded);
                    state.pinned = None;
                }
                Err(error) => state.error = Some(error),
            }
        }
        Err(mpsc::TryRecvError::Empty) => ctx.request_repaint(),
        Err(mpsc::TryRecvError::Disconnected) => {
            state.worker = None;
            state.error = Some("the facet reader stopped".to_owned());
        }
    }
}

pub fn ui_classic_maps(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let cc_path = app.settings.cc_path.clone();
    let tiledata = app.cc_tiledata.clone();
    let state = &mut app.classic_file_views.maps;

    egui::Panel::left("classic_maps_tools").resizable(true).default_size(TOOLS_PANEL_WIDTH).show(ui, |ui| {
        ui.heading("Maps and statics");
        ui.separator();
        let Some(cc_path) = cc_path.clone() else {
            ui.label("Set the CC path (File > Search Paths).");
            return;
        };
        let classic_dir = cc_path.clone();
        egui::ComboBox::from_id_salt("classic_maps_facet").selected_text(facet_name(state.facet)).show_ui(ui, |ui| {
            for facet in 0..CLASSIC_FACET_NAMES.len() as u32 {
                ui.selectable_value(&mut state.facet, facet, facet_name(facet));
            }
        });
        ui.checkbox(&mut state.map_difs, "Map difs");
        ui.checkbox(&mut state.static_difs, "Static difs");
        let busy = state.worker.is_some();
        if ui.add_enabled(!busy, egui::Button::new(if busy { "Reading..." } else { "Read facet" })).clicked() {
            let (sender, receiver) = mpsc::channel();
            let (facet, difs) = (state.facet, ClassicDifOptions { map_difs: state.map_difs, static_difs: state.static_difs });
            let max_side = u32::try_from(ctx.input(|input| input.max_texture_side)).unwrap_or(u32::MAX);
            std::thread::spawn(move || {
                let _ = sender.send(load_facet(cc_path, facet, difs, tiledata, max_side).map_err(|error| format!("{error:#}")));
            });
            state.worker = Some(receiver);
        }
        if let Some(error) = &state.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(loaded) = &state.loaded {
            ui.separator();
            ui.label(format!("{} x {} tiles", loaded.width, loaded.height));
            if loaded.scale > 1 {
                ui.label(format!("drawn at 1 pixel per {0} x {0} tiles (the GPU's largest texture)", loaded.scale));
            }
            ui.small(&loaded.sources);
        }
        ui.weak("Shift+wheel, Ctrl+wheel or a pinch zooms at the pointer; the wheel and dragging move the map.");
        ui.weak("Hover a tile for its land and statics; click to keep it.");
        ui.separator();
        super::classic_maps_convert::section(&mut state.convert, &classic_dir, ui, ctx);
    });

    egui::CentralPanel::default().show(ui, |ui| {
        if state.detached {
            ui.label("The map is in its own window.");
            if ui.button("Bring it back to this tab").clicked() {
                state.detached = false;
            }
            return;
        }
        map_view(state, ui);
    });
}

/// The map's own window. It runs every frame, whichever tab is open, so a read finishes and the window stays open
/// while other tabs are in use.
pub fn ui_classic_maps_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let state = &mut app.classic_file_views.maps;
    poll_worker(state, ctx);
    let Some(facet) = state.loaded.as_ref().filter(|_| state.detached).map(|loaded| loaded.facet) else { return };
    let window_id = egui::ViewportId::from_hash_of(MAP_WINDOW_ID);
    if !super::can_show_immediate_viewport(ctx, window_id) {
        return;
    }
    ctx.show_viewport_immediate(
        window_id,
        egui::ViewportBuilder::default()
            .with_title(format!("Facet {}", facet_name(facet)))
            .with_inner_size([MAP_WINDOW_WIDTH, MAP_WINDOW_HEIGHT]),
        |ui, _class| {
            if ui.input(|input| input.viewport().close_requested()) {
                state.detached = false;
            }
            egui::CentralPanel::default().show(ui, |ui| map_view(state, ui));
        },
    );
}

/// The zoom bar, the map and the tile details, in the tab or in the map's own window.
fn map_view(state: &mut ClassicMapsState, ui: &mut egui::Ui) {
    let (Some(loaded), Some(texture)) = (&state.loaded, &state.texture) else {
        ui.centered_and_justified(|ui| ui.label("Read a facet."));
        return;
    };
    let (width, height) = (loaded.width, loaded.height);
    let mut zoom = state.zoom;
    let mut fit = false;
    ui.horizontal(|ui| {
        if ui.button("-").on_hover_text("Zoom out").clicked() {
            zoom /= ZOOM_STEP;
        }
        if ui.button("+").on_hover_text("Zoom in").clicked() {
            zoom *= ZOOM_STEP;
        }
        ui.add(egui::Slider::new(&mut zoom, MIN_ZOOM..=MAX_ZOOM).logarithmic(true).text("Zoom"));
        if ui.button("1:1").on_hover_text("One pixel per tile").clicked() {
            zoom = ACTUAL_SIZE_ZOOM;
        }
        fit = ui.button("Fit").on_hover_text("Show the whole facet").clicked();
        let can_detach = !ui.ctx().embed_viewports();
        let label = if state.detached { "Back to the tab" } else { "Open in a separate window" };
        if ui.add_enabled(can_detach, egui::Button::new(label)).clicked() {
            state.detached = !state.detached;
        }
    });
    if fit {
        zoom = fit_zoom(ui.available_size(), width, height);
    }

    let size = egui::vec2(width as f32 * state.zoom, height as f32 * state.zoom);
    let mut area = egui::ScrollArea::both().id_salt("classic_maps_map").scroll_source(egui::scroll_area::ScrollSource {
        scroll_bar: true,
        drag: egui::scroll_area::DragScroll::Always,
        mouse_wheel: true,
    });
    if let Some(offset) = state.pending_offset.take() {
        area = area.scroll_offset(offset);
    }
    let shown_zoom = state.zoom;
    let mut pinned = None;
    let output = area.show(ui, |ui| {
        let response = ui.add(egui::Image::new(texture).fit_to_exact_size(size).sense(egui::Sense::click()));
        let tile_at = |pos: egui::Pos2| {
            let offset = (pos - response.rect.min) / shown_zoom;
            (offset.x >= 0.0 && offset.y >= 0.0)
                .then(|| (offset.x as u32, offset.y as u32))
                .filter(|&(x, y)| x < width && y < height)
        };
        if response.clicked() {
            pinned = Some(response.interact_pointer_pos().and_then(tile_at));
        }
        response.hover_pos().and_then(tile_at)
    });
    let hovered = output.inner;
    if let Some(pinned) = pinned {
        state.pinned = pinned;
    }

    let pointer = ui.input(|input| input.pointer.hover_pos()).filter(|_| ui.rect_contains_pointer(output.inner_rect));
    if pointer.is_some() {
        // Shift+wheel, Ctrl+wheel and a pinch; the plain wheel scrolls the area above (gui_shared's input options).
        zoom *= ui.input(|input| input.zoom_delta());
    }
    let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    if zoom != state.zoom {
        let anchor = pointer.map_or(output.inner_rect.size() / 2.0, |pos| pos - output.inner_rect.min);
        state.pending_offset = Some(if fit { egui::Vec2::ZERO } else { anchored_offset(output.state.offset, anchor, state.zoom, zoom) });
        state.zoom = zoom;
        ui.ctx().request_repaint();
    }

    let pinned = state.pinned;
    let Some(loaded) = &mut state.loaded else { return };
    egui::Window::new("Tile").default_pos(egui::pos2(TOOLS_PANEL_WIDTH * 2.0, 80.0)).resizable(false).show(ui.ctx(), |ui| {
        for (label, tile) in [("Pointer", hovered), ("Clicked", pinned)] {
            let Some((x, y)) = tile else { continue };
            ui.strong(format!("{label}: {x}, {y}"));
            tile_details(ui, loaded, x, y);
            ui.separator();
        }
    });
}

fn tile_details(ui: &mut egui::Ui, loaded: &mut LoadedFacet, x: u32, y: u32) {
    let block = MapBlockRelPos { x: x / UO_BLOCK_DIM, y: y / UO_BLOCK_DIM };
    let (cell_x, cell_y) = (x % UO_BLOCK_DIM, y % UO_BLOCK_DIM);
    if !loaded.plane.is_block_cached(&block) {
        if let Err(error) = loaded.plane.load_blocks(&mut [block]) {
            ui.label(format!("block {},{}: {error:#}", block.x, block.y));
            return;
        }
    }
    let tiledata = loaded.tiledata.clone();
    match loaded.plane.block(block).map(|block| block.cell(cell_x, cell_y)) {
        Some(Ok(cell)) => {
            let name = tiledata.as_deref().and_then(|t| t.land_tiles().get(usize::from(cell.id))).map_or("", |tile| tile.name_ascii());
            ui.monospace(format!("land 0x{:04X} z {} {}", cell.id, cell.z, name));
        }
        Some(Err(error)) => {
            ui.label(format!("{error:#}"));
        }
        None => {
            ui.label("the block is not read");
        }
    }
    let statics: Vec<PackedStaticTile> = loaded
        .statics
        .block_tiles(block.x, block.y)
        .iter()
        .copied()
        .filter(|tile| u32::from(tile.x_offset()) == cell_x && u32::from(tile.y_offset()) == cell_y)
        .collect();
    if statics.is_empty() {
        ui.weak("no statics");
    }
    for tile in statics {
        let (graphic, z, hue) = (tile.graphic, tile.z, tile.hue);
        let item = tiledata.as_deref().and_then(|t| t.item_tiles().get(usize::from(graphic)));
        let (name, height) = item.map_or(("", 0), |item| (item.name_ascii(), item.height_raw()));
        ui.monospace(format!("static 0x{graphic:04X} z {z} h {height} hue {hue} {name}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
    /// Tokuno, the smallest facet.
    const FACET: u32 = 4;
    /// A texture side that makes the render scale down.
    const SMALL_TEXTURE_SIDE: u32 = 512;
    const CLOSE: f32 = 0.001;

    #[test]
    fn zooming_keeps_the_tile_under_the_anchor_in_place() {
        let (offset, anchor) = (egui::vec2(300.0, 40.0), egui::vec2(100.0, 60.0));
        let (old_zoom, new_zoom) = (0.5, 2.0);
        let tile = (offset + anchor) / old_zoom;
        let moved = anchored_offset(offset, anchor, old_zoom, new_zoom);
        let tile_after = (moved + anchor) / new_zoom;
        assert!((tile - tile_after).length() < CLOSE, "{tile:?} moved to {tile_after:?}");
    }

    #[test]
    fn fitting_takes_the_tighter_side_and_stays_in_the_zoom_range() {
        assert_eq!(fit_zoom(egui::vec2(1000.0, 1000.0), 2000, 500), 0.5);
        assert_eq!(fit_zoom(egui::vec2(1.0, 1.0), 100_000, 100_000), MIN_ZOOM);
        assert_eq!(fit_zoom(egui::vec2(100_000.0, 100_000.0), 1, 1), MAX_ZOOM);
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
    fn a_shipped_facet_renders_and_reports_its_tiles() {
        let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
        for root in std::env::split_paths(&roots) {
            let Ok((mut loaded, image)) = load_facet(root.clone(), FACET, ClassicDifOptions::NONE, None, u32::MAX) else {
                println!("{}: no facet {FACET}", root.display());
                continue;
            };
            assert_eq!((loaded.scale, image.size), (1, [loaded.width as usize, loaded.height as usize]));
            let colors: HashSet<[u8; 4]> = image.pixels.iter().map(|pixel| pixel.to_array()).collect();
            assert!(colors.len() > 1, "{}: one colour", root.display());
            let block = MapBlockRelPos { x: loaded.width / 2 / UO_BLOCK_DIM, y: loaded.height / 2 / UO_BLOCK_DIM };
            loaded.plane.load_blocks(&mut [block]).unwrap();
            assert!(loaded.plane.block(block).is_some());
            let (_, small) = load_facet(root.clone(), FACET, ClassicDifOptions::NONE, None, SMALL_TEXTURE_SIDE).unwrap();
            assert!(small.size[0] <= SMALL_TEXTURE_SIDE as usize && small.size[1] <= SMALL_TEXTURE_SIDE as usize);
            println!("{}: facet {FACET} {} x {}, {} colours, {}", root.display(), loaded.width, loaded.height, colors.len(), loaded.sources);
        }
    }

    use std::collections::HashSet;
}
