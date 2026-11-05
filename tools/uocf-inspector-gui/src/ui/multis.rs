use crate::app::{
    ArtSource, HousingIndex, MultiCollectionSource, MultiPartFilters, MultisSource, UopInspectorApp, ViewMode,
};
use crate::ui::{arrow_delta, list_sort_controls, move_selection, multi_editor, sorted_indices_by};
use eframe::egui;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use uocf::classic::art::static_art_id_for_source;
use uocf::classic::multi::MultiPart;
use uocf::enhanced::housing::{housing_tile_type_name, HousingPieces, HousingSlot};
use uocf::enhanced::tileart::ArtTexture;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};

#[derive(Clone)]
struct PreviewPart {
    /// Index of the part in the multi definition, kept across filtering.
    source_index: usize,
    item_id: u16,
    x: i16,
    y: i16,
    z: i16,
    original_z: i16,
    flags: String,
    /// The client never draws this part.
    hidden: bool,
    /// The tile id is a housing.bin piece.
    housing_piece: bool,
}

/// What the multis tab worked out from the loaded data. egui redraws on every mouse move, so a redraw must not
/// search the tileart records again, look again for art it found missing, or read `multi.mul` again.
#[derive(Default)]
pub(crate) struct MultiPreviewCache {
    data: MultiPreviewData,
    tileart_by_tile: Option<HashMap<u32, usize>>,
    render_info: HashMap<(u16, u8), PartRenderInfo>,
    missing_art: HashSet<u64>,
    logged_missing: HashSet<(u16, u8)>,
    classic_parts: Option<(u32, Arc<Vec<MultiPart>>)>,
    classic_empty_multis: Option<Arc<HashSet<u32>>>,
}

/// The data the cache was worked out from, by address: a reload replaces it, an edit moves its revision.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct MultiPreviewData {
    tileart: usize,
    tileart_revision: u64,
    dictionary: usize,
    art: usize,
    tiledata: usize,
    multis: usize,
}

#[derive(Clone, Copy, Default)]
struct PartRenderInfo {
    is_floor: bool,
    width_delta: i32,
    height_delta: i32,
    offset_x: i32,
    offset_y: i32,
    texture_id: Option<u32>,
    clip_rect: Option<MultiSourceClip>,
    /// The source of the art when it is not the selected one: a tile without KR art.
    art_source: Option<ArtSource>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct MultiSourceClip {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

struct PreviewRenderPart {
    index: usize,
    part: PreviewPart,
    info: PartRenderInfo,
    scale: f32,
    texture: Option<egui::TextureHandle>,
    size: egui::Vec2,
}

const CLASSIC_STATIC_TILE_PIXEL_WIDTH: f32 = 44.0;
const ENHANCED_STATIC_TILE_PIXEL_WIDTH: f32 = 64.0;
const CLASSIC_MULTI_SIDEBAR_WIDTH: f32 = 250.0;
const UOP_MULTI_SIDEBAR_WIDTH: f32 = 300.0;
/// Classic `multi.mul` part flag set on the parts the client draws.
pub(super) const CLASSIC_MULTI_PART_VISIBLE_FLAG: u32 = 0x1;
/// Alpha of the tint applied to hidden parts in the preview.
const HIDDEN_PART_TINT_ALPHA: u8 = 96;
const HIDDEN_PART_OUTLINE_WIDTH: f32 = 1.0;
const HIDDEN_PART_OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 64, 255);
const MISSING_PART_FILL_COLOR: egui::Color32 = egui::Color32::BLUE;
/// Where a tile without KR art finds its art: the classic files first, as the official EC draws classic art unless a
/// tile asks for its new art.
const KR_ART_FALLBACK_SOURCES: [ArtSource; 3] = [ArtSource::CcUop, ArtSource::Mul, ArtSource::EcUopLegacy];
const KR_ART_FALLBACK_NOTE: &str = "KR art is optional: Texture.uop has images for only part of the tiles, and \
    the Enhanced Client draws them only for tileart records with the UseNewArt flag. The other tiles are drawn with their \
    classic art here, at its own scale.";
/// Context data key of the "housing.bin pieces" toggle of the MultiCollection.uop view.
const SHOW_HOUSING_PIECES_ID_SALT: &str = "multi_uop_show_housing_pieces";

fn arc_address<T>(value: &Option<Arc<T>>) -> usize {
    value.as_ref().map_or(0, |value| Arc::as_ptr(value) as usize)
}

/// Forgets what was worked out from data that has since been replaced or edited.
fn refresh_multi_preview_cache(app: &mut UopInspectorApp) {
    let client = app.client_data.as_ref();
    let data = MultiPreviewData {
        tileart: arc_address(&app.ec_tileart_entries),
        tileart_revision: app.tileart_editor.edits.revision(),
        dictionary: arc_address(&app.uo_string_dictionary),
        art: client.map_or(0, |client| Arc::as_ptr(&client.art) as usize),
        tiledata: client.map_or(0, |client| Arc::as_ptr(&client.tiledata) as usize),
        multis: client.map_or(0, |client| arc_address(&client.multis)),
    };
    if app.multi_preview_cache.data != data {
        app.multi_preview_cache = MultiPreviewCache { data, ..Default::default() };
    }
}

pub fn ui_multis(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    refresh_multi_preview_cache(app);
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);

    let has_classic = app
        .client_data
        .as_ref()
        .and_then(|client| client.multis.as_ref())
        .is_some();
    let has_uop = app.multi_collection.is_some();

    if !source_available(app.multis_source, has_classic, has_uop) {
        app.multis_source = if has_classic { MultisSource::ClassicMul } else { MultisSource::Uop };
    }

    egui::Panel::top("multis_source_tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            if has_classic {
                ui.selectable_value(&mut app.multis_source, MultisSource::ClassicMul, "multi.mul/.idx");
            }
            if has_uop {
                let label = format!("{} MultiCollection.uop", multi_collection_source_label(app));
                ui.selectable_value(&mut app.multis_source, MultisSource::Uop, label);
            }
            ui.separator();
            ui.label("Art:");
            egui::ComboBox::from_id_salt("multis_art_source")
                .selected_text(art_source_label(app.selected_legacy_source))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Mul, "MUL");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopLegacy, "EC Legacy UOP");
                    ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopKr, "KR/New UOP");
                });
        });
    });

    match app.multis_source {
        MultisSource::ClassicMul => ui_classic_multis(app, ctx, ui),
        MultisSource::Uop => ui_uop_multis(app, ctx, ui),
    }
}

fn source_available(source: MultisSource, has_classic: bool, has_uop: bool) -> bool {
    match source {
        MultisSource::ClassicMul => has_classic,
        MultisSource::Uop => has_uop,
    }
}

fn multi_collection_source_label(app: &UopInspectorApp) -> &'static str {
    match app.multi_collection_source {
        Some(MultiCollectionSource::ClassicClient) => "CC",
        Some(MultiCollectionSource::EnhancedClient) => "EC",
        None => "Loaded",
    }
}

fn art_source_label(source: ArtSource) -> &'static str {
    match source {
        ArtSource::Any => "CC UOP",
        ArtSource::Mul => "MUL",
        ArtSource::CcUop => "CC UOP",
        ArtSource::EcUop | ArtSource::EcUopLegacy => "EC Legacy UOP",
        ArtSource::EcUopKr => "KR/New UOP",
    }
}

fn ui_classic_multis(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::top("classic_multi_editor_bar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| super::classic_multi_editor::bar(app, ui));
    });
    if app.classic_multi_editor.enabled {
        let id = app.selected_multi_id;
        let part_id = multi_selected_part_id(app);
        let selected = ctx.data_mut(|data| data.get_temp::<usize>(part_id));
        let generation = app.classic_multi_editor.generation();
        let new_selection = egui::Panel::bottom("classic_multi_part_editor")
            .show(ui, |ui| super::classic_multi_editor::part_panel(app, ui, id, selected))
            .inner;
        match new_selection {
            Some(Some(index)) => {
                ctx.data_mut(|data| data.insert_temp(part_id, index));
            }
            Some(None) => {
                ctx.data_mut(|data| data.remove::<usize>(part_id));
            }
            None => {}
        }
        if app.classic_multi_editor.generation() != generation {
            app.multi_preview_cache.classic_parts = None;
            app.multi_preview_cache.classic_empty_multis = None;
        }
    }
    egui::Panel::left("classic_multi_sidebar")
        .resizable(true)
        .default_size(CLASSIC_MULTI_SIDEBAR_WIDTH)
        .show(ui, |ui| {
            ui.heading("multi.mul/.idx");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("ID:");
                ui.add(egui::DragValue::new(&mut app.selected_multi_id));
            });
            let sort = list_sort_controls(ui, "classic_multi_entries", &["ID"], 0);
            let housing_index = app.housing_index.clone();
            housing_multis_filter_checkbox(ui, &mut app.multi_part_filters, housing_index.as_deref());
            let only_housing_multis = housing_multis_filter_active(app.multi_part_filters, housing_index.as_deref());
            empty_multis_filter_checkbox(ui, &mut app.multi_part_filters);
            let empty_multis = app.multi_part_filters.hide_empty_multis.then(|| classic_empty_multis(app));

            if let Some(client) = &app.client_data {
                if let Some(multis) = &client.multis {
                    let max_id = app.classic_multi_editor.multi_count().unwrap_or_else(|| multis.max_id());
                    let ids = (0..max_id)
                        .filter(|id| !only_housing_multis || is_housing_multi(housing_index.as_deref(), *id))
                        .filter(|id| empty_multis.as_ref().is_none_or(|empty| !empty.contains(id)))
                        .collect::<Vec<_>>();
                    let sorted_indices = sorted_indices_by(&ids, sort.ordering(), |left, right| left.cmp(right));
                    let visible_ids: Vec<_> = sorted_indices.iter().map(|index| ids[*index]).collect();
                    let keyboard_moved = if let Some(delta) = arrow_delta(ui, false) {
                        if let Some(id) = move_selection(&visible_ids, Some(app.selected_multi_id), delta) {
                            select_multi(app, id, None);
                        }
                        true
                    } else {
                        false
                    };
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for index in sorted_indices {
                            let id = ids[index];
                            let selected = app.selected_multi_id == id;
                            let response = ui.selectable_label(
                                selected,
                                format!("Multi {}{}", id, housing_multi_marker(housing_index.as_deref(), id)),
                            );
                            if keyboard_moved && selected {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.clicked()
                            {
                                select_multi(app, id, None);
                            }
                        }
                    });
                }
            }
        });

    egui::CentralPanel::default().show(ui, |ui| {
        let parts = classic_multi_parts(app);
        let preview_parts: Vec<_> = parts
            .iter()
            .enumerate()
            .map(|(source_index, part)| PreviewPart {
                source_index,
                item_id: part.item_id,
                x: part.x,
                y: part.y,
                z: part.z,
                original_z: part.z,
                flags: format!("0x{:08X}", part.flags),
                hidden: part.flags & CLASSIC_MULTI_PART_VISIBLE_FLAG == 0,
                housing_piece: is_housing_piece(app.housing_index.as_deref(), part.item_id),
            })
            .collect();
        draw_multi_details(app, ctx, ui, "Classic Multi", &preview_parts, None);
    });
}

fn ui_uop_multis(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(collection) = app.multi_collection.clone() else {
        return;
    };

    egui::Panel::left("uop_multi_sidebar")
        .resizable(true)
        .default_size(UOP_MULTI_SIDEBAR_WIDTH)
        .show(ui, |ui| {
            ui.heading(format!(
                "{} MultiCollection.uop ({})",
                multi_collection_source_label(app),
                collection.items.len()
            ));
            if let Some(path) = &app.multi_collection_path {
                ui.monospace(path.display().to_string());
            }
            ui.separator();
            let mut search_has_focus = false;
            ui.horizontal(|ui| {
                ui.label("Search:");
                search_has_focus = ui.text_edit_singleline(&mut app.searches.multis).has_focus();
            });
            let sort = list_sort_controls(ui, "uop_multi_entries", &["ID", "Path", "Hash", "Parts"], 0);
            let housing_index = app.housing_index.clone();
            housing_multis_filter_checkbox(ui, &mut app.multi_part_filters, housing_index.as_deref());
            let only_housing_multis = housing_multis_filter_active(app.multi_part_filters, housing_index.as_deref());
            empty_multis_filter_checkbox(ui, &mut app.multi_part_filters);
            let hide_empty_multis = app.multi_part_filters.hide_empty_multis;
            ui.separator();

            let query = app.searches.multis.to_lowercase();
            let entry_listed = |item: &uocf::enhanced::multis::MultiItem| {
                (query.is_empty()
                    || item.id.to_string().contains(&query)
                    || item.path.to_lowercase().contains(&query)
                    || format!("{:016X}", item.filename_hash).to_lowercase().contains(&query))
                    && (!only_housing_multis || is_housing_multi(housing_index.as_deref(), item.id))
                    && (!hide_empty_multis || !item.parts.is_empty())
            };
            let sorted_indices = sorted_indices_by(&collection.items, sort.ordering(), |left, right| {
                match sort.option_index {
                    1 => left.path.to_ascii_lowercase().cmp(&right.path.to_ascii_lowercase()),
                    2 => left.filename_hash.cmp(&right.filename_hash),
                    3 => left.parts.len().cmp(&right.parts.len()),
                    _ => left.id.cmp(&right.id),
                }
            });
            let visible_entries: Vec<_> = sorted_indices
                .iter()
                .map(|index| &collection.items[*index])
                .filter(|item| entry_listed(item))
                .map(|item| (item.id, item.filename_hash))
                .collect();
            let selected_entry = collection
                .get(app.selected_multi_id)
                .map(|item| (item.id, item.filename_hash))
                .or_else(|| app.selected_multi_uop_hash.map(|hash| (app.selected_multi_id, hash)));
            let keyboard_moved = if let Some(delta) = arrow_delta(ui, search_has_focus) {
                if let Some((id, hash)) = move_selection(&visible_entries, selected_entry, delta) {
                    select_multi(app, id, Some(hash));
                }
                true
            } else {
                false
            };
            egui::ScrollArea::vertical().show(ui, |ui| {
                for index in sorted_indices {
                    let item = &collection.items[index];
                    if !entry_listed(item) {
                        continue;
                    }

                    let selected = app.selected_multi_id == item.id;
                    let response = ui.selectable_label(
                        selected,
                        format!(
                            "Multi {} ({:016X}){}",
                            item.id,
                            item.filename_hash,
                            housing_multi_marker(housing_index.as_deref(), item.id)
                        ),
                    );
                    if keyboard_moved && selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked()
                    {
                        select_multi(app, item.id, Some(item.filename_hash));
                    }
                }
            });
        });

    let show_housing_id = egui::Id::new(SHOW_HOUSING_PIECES_ID_SALT);
    let mut show_housing = ctx
        .data_mut(|data| data.get_temp::<bool>(show_housing_id))
        .unwrap_or(false);
    let mut edits = Vec::new();
    egui::Panel::top("multi_uop_raw_tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.multis_source, MultisSource::Uop, "Specialized");
            if ui.button("Raw selected").clicked() {
                app.select_raw_multi_collection_entry(app.selected_multi_id);
            }
            if collection.housing_hash.is_some() && ui.button("Raw housing").clicked() {
                app.select_raw_multi_collection_housing();
            }
            if app.housing_pieces.is_some() {
                ui.toggle_value(&mut show_housing, "housing.bin pieces");
                crate::ui::help_mark(ui, crate::ui::housing::HOUSING_BIN_HELP);
            }
        });
        ui.horizontal_wrapped(|ui| multi_editor::toolbar(app, ctx, ui, &mut edits));
    });
    ctx.data_mut(|data| data.insert_temp(show_housing_id, show_housing));

    let part_id = multi_selected_part_id(app);
    if let Some(item) = collection.get(app.selected_multi_id).filter(|_| app.multi_editor.enabled && !show_housing) {
        let selected_part = ctx.data_mut(|data| data.get_temp::<usize>(part_id));
        egui::Panel::bottom("multi_component_editor")
            .show(ui, |ui| multi_editor::component_panel(ui, item, selected_part, &mut edits));
    }

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(housing) = app.housing_pieces.clone().filter(|_| show_housing) {
            draw_housing_pieces(ui, &housing);
        } else if let Some(item) = collection.get(app.selected_multi_id) {
            let preview_parts: Vec<_> = item
                .parts
                .iter()
                .enumerate()
                .map(|(source_index, part)| PreviewPart {
                    source_index,
                    item_id: part.item_id,
                    x: part.x,
                    y: part.y,
                    z: part.z,
                    original_z: part.z,
                    flags: format!(
                        "not_drawn {} / flag2 {} / {} strings",
                        u8::from(part.not_drawn),
                        part.flag2,
                        part.string_ids.len()
                    ),
                    hidden: part.not_drawn,
                    housing_piece: is_housing_piece(app.housing_index.as_deref(), part.item_id),
                })
                .collect();
            let raw = Some(format!(
                "{} / {} / 0x{:016X}",
                multi_collection_source_label(app),
                item.path,
                item.filename_hash
            ));
            draw_multi_details(app, ctx, ui, "UOP Multi", &preview_parts, raw.as_deref());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a MultiCollection.uop entry from the left panel.");
            });
        }
    });
    // The edits go into the loaded collection, which must not be shared then.
    drop(collection);
    multi_editor::apply_edits(app, ctx, edits, part_id);
}

fn draw_housing_pieces(ui: &mut egui::Ui, housing: &HousingPieces) {
    ui.heading(format!(
        "housing.bin: {} tile types, {} pieces",
        housing.tile_types.len(),
        housing.pieces().count()
    ));
    ui.separator();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("housing_pieces_grid").striped(true).show(ui, |ui| {
            for header in ["Tile type", "Category", "Group", "Feature mask", "Tooltip", "Solid", "Multis", "Windows"] {
                ui.label(header);
            }
            ui.end_row();
            for (tile_type, piece) in housing.pieces() {
                ui.label(format!("{} {}", tile_type, housing_tile_type_name(tile_type).unwrap_or("?")));
                ui.label(piece.category.to_string());
                ui.label(piece.group.to_string());
                ui.monospace(format!("0x{:X}", piece.feature_mask));
                ui.label(piece.tooltip_cliloc.to_string());
                ui.monospace(housing_slots_label(&piece.solid));
                ui.monospace(housing_slots_label(&piece.multis));
                ui.monospace(housing_slots_label(&piece.windows));
                ui.end_row();
            }
        });
    });
}

fn housing_slots_label(slots: &[HousingSlot]) -> String {
    slots
        .iter()
        .map(|slot| format!("{}:{}", slot.slot, slot.id))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_housing_piece(housing_index: Option<&HousingIndex>, item_id: u16) -> bool {
    housing_index.is_some_and(|index| index.piece_tile_ids.contains(&u32::from(item_id)))
}

fn is_housing_multi(housing_index: Option<&HousingIndex>, multi_id: u32) -> bool {
    housing_index.is_some_and(|index| index.multi_ids.contains(&multi_id))
}

fn housing_multi_marker(housing_index: Option<&HousingIndex>, multi_id: u32) -> &'static str {
    if is_housing_multi(housing_index, multi_id) {
        " [housing]"
    } else {
        ""
    }
}

fn housing_multis_filter_active(filters: MultiPartFilters, housing_index: Option<&HousingIndex>) -> bool {
    filters.only_housing_multis && housing_index.is_some()
}

fn housing_multis_filter_checkbox(
    ui: &mut egui::Ui,
    filters: &mut MultiPartFilters,
    housing_index: Option<&HousingIndex>,
) {
    if let Some(index) = housing_index {
        ui.checkbox(
            &mut filters.only_housing_multis,
            format!("Only multis used by housing.bin ({})", index.multi_ids.len()),
        )
        .on_hover_text("Multis referenced by the housing.bin multi lists (stair blocks), marked [housing].");
    }
}

fn empty_multis_filter_checkbox(ui: &mut egui::Ui, filters: &mut MultiPartFilters) {
    ui.checkbox(&mut filters.hide_empty_multis, "Hide empty multis")
        .on_hover_text("Leave out of this list the entries with no parts.");
}

/// The parts of the selected classic multi, read once per selection.
fn classic_multi_parts(app: &mut UopInspectorApp) -> Arc<Vec<MultiPart>> {
    let id = app.selected_multi_id;
    if let Some((_, parts)) = app.multi_preview_cache.classic_parts.as_ref().filter(|(cached, _)| *cached == id) {
        return parts.clone();
    }
    let edited = app.classic_multi_editor.parts(id);
    let parts = edited
        .or_else(|| app.client_data.as_ref().and_then(|client| client.multis.as_ref()).and_then(|multis| multis.get_parts(id).ok()))
        .unwrap_or_default();
    let parts = Arc::new(parts);
    app.multi_preview_cache.classic_parts = Some((id, parts.clone()));
    parts
}

/// The classic multis with no parts, or that cannot be read. Reading them all is done once, the first time the filter
/// is on.
fn classic_empty_multis(app: &mut UopInspectorApp) -> Arc<HashSet<u32>> {
    if let Some(empty) = &app.multi_preview_cache.classic_empty_multis {
        return empty.clone();
    }
    let edited = app.classic_multi_editor.multi_count().map(|count| {
        (0..count).filter(|id| app.classic_multi_editor.parts(*id).is_none_or(|parts| parts.is_empty())).collect()
    });
    let empty: HashSet<u32> = edited
        .or_else(|| {
            app.client_data.as_ref().and_then(|client| client.multis.as_ref()).map(|multis| {
                (0..multis.max_id())
                    .filter(|id| multis.get_parts(*id).map_or(true, |parts| parts.is_empty()))
                    .collect()
            })
        })
        .unwrap_or_default();
    let empty = Arc::new(empty);
    app.multi_preview_cache.classic_empty_multis = Some(empty.clone());
    empty
}

/// Whether a part passes the panel filters; the housing.bin filters apply only when housing.bin is loaded.
fn part_passes_filters(part: &PreviewPart, filters: MultiPartFilters, housing_loaded: bool) -> bool {
    let housing_kept = !housing_loaded
        || if part.housing_piece {
            filters.show_housing_parts
        } else {
            filters.show_other_parts
        };
    (filters.show_hidden || !part.hidden) && housing_kept
}

fn part_cell_text(part: &PreviewPart, text: impl Into<String>) -> egui::RichText {
    let text = egui::RichText::new(text);
    if part.hidden {
        text.italics().weak()
    } else {
        text
    }
}

fn part_notes(part: &PreviewPart) -> String {
    let mut notes = Vec::new();
    if part.hidden {
        notes.push("hidden");
    }
    if part.housing_piece {
        notes.push("housing.bin");
    }
    notes.join(", ")
}

/// Switches to the Multis tab with a MultiCollection.uop multi selected.
pub(crate) fn show_multi_in_multis_tab(app: &mut UopInspectorApp, ctx: &egui::Context, multi_id: u32) {
    let hash = app
        .multi_collection
        .as_ref()
        .and_then(|collection| collection.get(multi_id))
        .map(|item| item.filename_hash);
    app.multis_source = MultisSource::Uop;
    select_multi(app, multi_id, hash);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(SHOW_HOUSING_PIECES_ID_SALT), false));
    app.view_mode = ViewMode::Multis;
}

/// Draws the 2D preview of a MultiCollection.uop multi (the parts the client draws) as the Multis tab does;
/// false when the multi is not in the loaded collection.
pub(crate) fn draw_collection_multi_preview(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    multi_id: u32,
) -> bool {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);
    let Some(collection) = app.multi_collection.clone() else {
        return false;
    };
    let Some(item) = collection.get(multi_id) else {
        return false;
    };
    let parts: Vec<_> = item
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| !part.not_drawn)
        .map(|(source_index, part)| PreviewPart {
            source_index,
            item_id: part.item_id,
            x: part.x,
            y: part.y,
            z: part.z,
            original_z: part.z,
            flags: String::new(),
            hidden: false,
            housing_piece: is_housing_piece(app.housing_index.as_deref(), part.item_id),
        })
        .collect();
    draw_preview(app, ctx, ui, &parts, None, false, "housing");
    true
}

/// Art of a static tile id in the art source of the Multis tab, from the multi preview texture cache. Unlike
/// the preview, a missing tile is not logged.
pub(crate) fn static_tile_thumbnail(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    tile_id: u32,
) -> Option<egui::TextureHandle> {
    let item_id = u16::try_from(tile_id).ok()?;
    refresh_multi_preview_cache(app);
    let source = concrete_art_tile_source(app.selected_legacy_source);
    let info = part_render_info(app, item_id, source);
    cached_multi_part_texture(app, ctx, item_id, &info)
}

fn select_multi(app: &mut UopInspectorApp, multi_id: u32, uop_hash: Option<u64>) {
    if app.selected_multi_id != multi_id || app.selected_multi_uop_hash != uop_hash {
        app.selected_multi_id = multi_id;
        app.selected_multi_uop_hash = uop_hash;
    }
}

fn draw_multi_details(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    title: &str,
    all_parts: &[PreviewPart],
    raw_label: Option<&str>,
) {
    if all_parts.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(format!("Multi {} is empty or missing", app.selected_multi_id));
        });
        return;
    }

    let hidden_count = all_parts.iter().filter(|part| part.hidden).count();
    ui.heading(format!(
        "{} {}: {} parts ({} hidden)",
        title,
        app.selected_multi_id,
        all_parts.len(),
        hidden_count
    ));
    if let Some(raw_label) = raw_label {
        ui.monospace(raw_label);
    }
    let housing_loaded = app.housing_index.is_some();
    let filters = &mut app.multi_part_filters;
    ui.horizontal(|ui| {
        ui.checkbox(&mut filters.show_hidden, "Show hidden parts")
            .on_hover_text("Parts the client never draws: EC not_drawn, classic parts without the visible flag.");
        if housing_loaded {
            ui.separator();
            ui.checkbox(&mut filters.show_housing_parts, "housing.bin parts")
                .on_hover_text("Parts whose tile id is in a housing.bin solid or windows list.");
            ui.checkbox(&mut filters.show_other_parts, "other parts")
                .on_hover_text("Parts whose tile id is not a housing.bin piece.");
        }
    });
    let filters = app.multi_part_filters;
    let parts: Vec<_> = all_parts
        .iter()
        .filter(|part| part_passes_filters(part, filters, housing_loaded))
        .cloned()
        .collect();
    let parts = parts.as_slice();
    ui.label(format!("Showing {} of {} parts", parts.len(), all_parts.len()));
    let detached_window_id = egui::Id::new("multi_preview_detached_window");
    let show_center_id = egui::Id::new("multi_preview_show_center");
    let selected_part_id = multi_selected_part_id(app);
    let mut detached_open = ctx
        .data_mut(|data| data.get_temp::<bool>(detached_window_id))
        .unwrap_or(false);
    let mut show_center = ctx
        .data_mut(|data| data.get_temp::<bool>(show_center_id))
        .unwrap_or(false);
    let mut selected_part_index = ctx
        .data_mut(|data| data.get_temp::<usize>(selected_part_id))
        .filter(|index| parts.iter().any(|part| part.source_index == *index));
    if selected_part_index.is_none() {
        ctx.data_mut(|data| data.remove::<usize>(selected_part_id));
    }

    ui.horizontal(|ui| {
        if ui.button("Open 2D Window").clicked() {
            detached_open = true;
        }
        ui.checkbox(&mut show_center, "Show center");
    });
    ctx.data_mut(|data| {
        data.insert_temp(detached_window_id, detached_open);
        data.insert_temp(show_center_id, show_center);
    });
    ui.separator();

    let available = ui.available_size();
    ui.horizontal(|ui| {
        let component_width = if available.x < 700.0 {
            available.x.min(320.0)
        } else {
            (available.x * 0.32).clamp(320.0, 480.0)
        };
        ui.allocate_ui_with_layout(
            egui::vec2(component_width, available.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.heading("Components");
                egui::ScrollArea::vertical()
                    .id_salt(("multi_parts_scroll", app.multis_source as u8, app.selected_multi_id))
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        egui::Grid::new("multi_parts_grid").striped(true).show(ui, |ui| {
                            ui.label("Item ID");
                            ui.label("X");
                            ui.label("Y");
                            ui.label("Z");
                            ui.label("Flags");
                            ui.label("Notes");
                            ui.end_row();
                            for part in parts {
                                let index = part.source_index;
                                let selected = selected_part_index == Some(index);
                                let mut clicked = false;
                                for text in [
                                    part.item_id.to_string(),
                                    part.x.to_string(),
                                    part.y.to_string(),
                                    part.z.to_string(),
                                    part.flags.clone(),
                                    part_notes(part),
                                ] {
                                    clicked |= ui.selectable_label(selected, part_cell_text(part, text)).clicked();
                                }
                                if clicked {
                                    selected_part_index = Some(index);
                                    ui.ctx().data_mut(|data| data.insert_temp(selected_part_id, index));
                                }
                                ui.end_row();
                            }
                        });
                    });
            },
        );

        ui.separator();

        let preview_size = ui.available_size();
        ui.allocate_ui_with_layout(
            preview_size,
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.heading("2D Preview");
                let preview_available = ui.available_size();
                ui.allocate_ui_with_layout(
                    preview_available,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        draw_preview(app, ctx, ui, parts, selected_part_index, show_center, "inline");
                    },
                );
            },
        );
    });

    if detached_open {
        let mut open = true;
        egui::Window::new(format!("2D Preview - {} {}", title, app.selected_multi_id))
            .id(egui::Id::new("multi_preview_detached_window_panel"))
            .open(&mut open)
            .default_size(egui::vec2(1000.0, 720.0))
            .resizable(true)
            .show(ctx, |ui| {
                draw_preview(app, ctx, ui, parts, selected_part_index, show_center, "detached");
            });
        ctx.data_mut(|data| data.insert_temp(detached_window_id, open));
    }
}

fn draw_preview(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    parts: &[PreviewPart],
    selected_part_index: Option<usize>,
    show_center: bool,
    scroll_id_salt: &'static str,
) {
    let (min_x, max_x, min_y, max_y) = parts.iter().fold(
        (0i16, 0i16, 0i16, 0i16),
        |(min_x, max_x, min_y, max_y), p| {
            (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
        },
    );
    let (min_z, max_z) = parts.iter().fold((0i16, 0i16), |(min_z, max_z), p| {
        (min_z.min(p.z), max_z.max(p.z))
    });
    let span_x = ((max_x as i32) - (min_x as i32) + 1).max(1);
    let span_y = ((max_y as i32) - (min_y as i32) + 1).max(1);
    let span_z = ((max_z as i32) - (min_z as i32)).max(0);
    let tile_w = 22.0f32;
    let tile_h = 22.0f32;
    refresh_multi_preview_cache(app);
    let art_source = concrete_art_tile_source(app.selected_legacy_source);
    let mut sorted_parts: Vec<_> = parts
        .iter()
        .map(|part| {
            let info = part_render_info(app, part.item_id, art_source);
            let texture = get_multi_part_texture(app, ctx, part.item_id, &info);
            let art_scale = multi_preview_art_scale(info.art_source.unwrap_or(art_source));
            let size = texture
                .as_ref()
                .map(|handle| handle.size_vec2() * art_scale)
                .unwrap_or_else(|| egui::vec2(14.0, 14.0) * art_scale);
            PreviewRenderPart {
                index: part.source_index,
                part: part.clone(),
                info,
                scale: art_scale,
                texture,
                size,
            }
        })
        .collect();
    let fallback_parts = sorted_parts.iter().filter(|part| part.info.art_source.is_some()).count();
    if fallback_parts > 0 {
        ui.weak(format!("{fallback_parts} of {} parts have no KR art: drawn with classic art.", sorted_parts.len()))
            .on_hover_text(KR_ART_FALLBACK_NOTE);
    }
    let viewport_size = ui.available_size();
    sorted_parts.sort_by(|a, b| {
        a.part.z.cmp(&b.part.z)
            .then(a.part.y.cmp(&b.part.y))
            .then(a.part.x.cmp(&b.part.x))
            .then((!a.info.is_floor).cmp(&(!b.info.is_floor)))
    });

    let padding = 96.0f32;
    let mut content_min = egui::pos2(f32::INFINITY, f32::INFINITY);
    let mut content_max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for render_part in &sorted_parts {
        let pos = preview_part_position(render_part, min_x, min_y, min_z, tile_w, tile_h);
        content_min.x = content_min.x.min(pos.x);
        content_min.y = content_min.y.min(pos.y);
        content_max.x = content_max.x.max(pos.x + render_part.size.x);
        content_max.y = content_max.y.max(pos.y + render_part.size.y);
    }
    if show_center {
        let center_pos = multi_center_position(min_x, min_y, min_z, tile_w, tile_h);
        content_min.x = content_min.x.min(center_pos.x - 24.0);
        content_min.y = content_min.y.min(center_pos.y - 24.0);
        content_max.x = content_max.x.max(center_pos.x + 24.0);
        content_max.y = content_max.y.max(center_pos.y + 24.0);
    }
    if !content_min.x.is_finite() {
        content_min = egui::pos2(0.0, 0.0);
        content_max = egui::pos2(
            (span_x as f32 + span_y as f32) * tile_w,
            (span_x as f32 + span_y as f32) * tile_h + span_z as f32 * 4.0,
        );
    }

    let content_w = (content_max.x - content_min.x + padding * 2.0).max(1.0);
    let content_h = (content_max.y - content_min.y + padding * 2.0).max(1.0);
    let canvas_w = content_w.max(viewport_size.x.max(1.0));
    let canvas_h = content_h.max(viewport_size.y.max(1.0));
    let canvas_size = egui::vec2(canvas_w, canvas_h);

    egui::ScrollArea::both()
        .id_salt(("multi_preview_scroll", scroll_id_salt, app.multis_source as u8, app.selected_multi_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
        let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));

        let origin = rect.min
            + egui::vec2(
                (canvas_w - content_w).max(0.0) * 0.5 + padding - content_min.x,
                (canvas_h - content_h).max(0.0) * 0.5 + padding - content_min.y,
            );

        for render_part in sorted_parts {
            let part_pos = preview_part_position(&render_part, min_x, min_y, min_z, tile_w, tile_h);
            if let Some(handle) = render_part.texture {
                let draw_pos = (rect.min + egui::vec2(
                    origin.x - rect.min.x + part_pos.x,
                    origin.y - rect.min.y + part_pos.y,
                ))
                .round();
                let part_rect = egui::Rect::from_min_size(draw_pos, render_part.size);
                let tint = if render_part.part.hidden {
                    egui::Color32::from_white_alpha(HIDDEN_PART_TINT_ALPHA)
                } else {
                    egui::Color32::WHITE
                };
                painter.image(
                    handle.id(),
                    part_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
                paint_hidden_part_outline(&painter, part_rect, render_part.part.hidden);
                paint_selected_part_highlight(&painter, part_rect, render_part.index, selected_part_index);
            } else {
                let fallback_pos = origin + part_pos.to_vec2();
                let part_rect = egui::Rect::from_center_size(
                    fallback_pos,
                    render_part.size,
                );
                let fill = if render_part.part.hidden {
                    MISSING_PART_FILL_COLOR.gamma_multiply(f32::from(HIDDEN_PART_TINT_ALPHA) / f32::from(u8::MAX))
                } else {
                    MISSING_PART_FILL_COLOR
                };
                painter.rect_filled(part_rect.shrink(1.0), 2.0, fill);
                paint_hidden_part_outline(&painter, part_rect, render_part.part.hidden);
                paint_selected_part_highlight(&painter, part_rect, render_part.index, selected_part_index);
            }
        }

        if show_center {
            let center = origin + multi_center_position(min_x, min_y, min_z, tile_w, tile_h).to_vec2();
            let stroke = egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 220, 64));
            painter.line_segment(
                [center + egui::vec2(-18.0, 0.0), center + egui::vec2(18.0, 0.0)],
                stroke,
            );
            painter.line_segment(
                [center + egui::vec2(0.0, -18.0), center + egui::vec2(0.0, 18.0)],
                stroke,
            );
            painter.circle_stroke(center, 6.0, stroke);
        }
    });
}

fn preview_part_position(
    render_part: &PreviewRenderPart,
    min_x: i16,
    min_y: i16,
    min_z: i16,
    tile_w: f32,
    tile_h: f32,
) -> egui::Pos2 {
    let part = &render_part.part;
    let info = render_part.info;
    let x = ((part.x as i32) - (min_x as i32)) as f32;
    let y = ((part.y as i32) - (min_y as i32)) as f32;
    let scale = render_part.scale;
    let base_x = (x - y) * tile_w + info.height_delta as f32 * scale;
    let base_y = (x + y) * tile_h
        + info.width_delta as f32 * scale
        + info.height_delta as f32 * scale
        - (part.original_z as f32 * 4.0);

    egui::pos2(
        base_x + info.offset_x as f32 * scale,
        base_y + info.offset_y as f32 * scale + (min_z as f32 * 4.0),
    )
}

fn multi_center_position(min_x: i16, min_y: i16, min_z: i16, tile_w: f32, tile_h: f32) -> egui::Pos2 {
    let x = -(min_x as f32);
    let y = -(min_y as f32);
    egui::pos2((x - y) * tile_w, (x + y) * tile_h + (min_z as f32 * 4.0))
}

fn paint_hidden_part_outline(painter: &egui::Painter, rect: egui::Rect, hidden: bool) {
    if hidden {
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(HIDDEN_PART_OUTLINE_WIDTH, HIDDEN_PART_OUTLINE_COLOR),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_selected_part_highlight(
    painter: &egui::Painter,
    rect: egui::Rect,
    index: usize,
    selected_part_index: Option<usize>,
) {
    if selected_part_index == Some(index) {
        painter.rect_stroke(
            rect.expand(3.0),
            0.0,
            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 220, 64)),
            egui::StrokeKind::Outside,
        );
    }
}

fn multi_selected_part_id(app: &UopInspectorApp) -> egui::Id {
    egui::Id::new(("multi_selected_part", app.multis_source as u8, app.selected_multi_id))
}

fn multi_part_art_id(item_id: u16, info: &PartRenderInfo, source: ArtSource) -> u32 {
    info.texture_id
        .unwrap_or_else(|| static_art_id_for_source(item_id as u32, source))
}

/// The art of a part. A part without art is reported once.
fn get_multi_part_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    item_id: u16,
    info: &PartRenderInfo,
) -> Option<egui::TextureHandle> {
    let texture = cached_multi_part_texture(app, ctx, item_id, info);
    if texture.is_none() {
        let source = info.art_source.unwrap_or(concrete_art_tile_source(app.selected_legacy_source));
        if app.multi_preview_cache.logged_missing.insert((item_id, source as u8)) {
            let art_id = multi_part_art_id(item_id, info, source);
            log_missing_multi_part_texture(app, item_id, art_id, source);
        }
    }
    texture
}

/// The art of a part, decoded once. Art that could not be had is remembered too: a redraw does not look for it again.
fn cached_multi_part_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    item_id: u16,
    info: &PartRenderInfo,
) -> Option<egui::TextureHandle> {
    let source = info.art_source.unwrap_or(concrete_art_tile_source(app.selected_legacy_source));
    let art_id = multi_part_art_id(item_id, info, source);
    let key = multi_preview_texture_key(art_id, source, info.clip_rect);
    if let Some(handle) = app.texture_previews.get(&key) {
        return Some(handle.clone());
    }
    if app.multi_preview_cache.missing_art.contains(&key) {
        return None;
    }
    let texture = decode_multi_part_rgba(app, art_id, source, info.clip_rect).and_then(|(width, height, pixels)| {
        app.checked_texture(
            ctx,
            format!("multi_preview_art_{}_{:?}", art_id, source),
            [width as usize, height as usize],
            &pixels,
            egui::TextureOptions::NEAREST,
        )
    });
    match &texture {
        Some(handle) => {
            app.texture_previews.insert(key, handle.clone());
        }
        None => {
            app.multi_preview_cache.missing_art.insert(key);
        }
    }
    texture
}

fn multi_preview_texture_key(art_id: u32, source: ArtSource, clip_rect: Option<MultiSourceClip>) -> u64 {
    let mut hasher = DefaultHasher::new();
    "multi_preview_art".hash(&mut hasher);
    (source as u8).hash(&mut hasher);
    art_id.hash(&mut hasher);
    clip_rect.hash(&mut hasher);
    hasher.finish()
}

fn decode_multi_part_rgba(
    app: &UopInspectorApp,
    art_id: u32,
    source: ArtSource,
    clip_rect: Option<MultiSourceClip>,
) -> Option<(u32, u32, Vec<u8>)> {
    let client = app.client_data.as_ref()?;
    let mut scratch = Vec::new();
    if client
        .art
        .get_raw_art_data_from_source(art_id, source, &mut scratch)
        .is_err()
    {
        return None;
    }

    if art_payload_is_image_file(&scratch, source) {
        if let Some((width, height, pixels)) = decode_multi_uop_art_rgba(&scratch, source) {
            return Some(crop_multi_part_rgba(width, height, pixels, clip_rect));
        }
    }

    if art_id < uocf::classic::art::STATIC_TILE_ID_BASE {
        let mut pixels = [0u8; 44 * 44 * 4];
        if uocf::classic::art::decode_land_tile_from_raw(&scratch, &mut pixels).is_ok() {
            return Some(crop_multi_part_rgba(44, 44, pixels.to_vec(), clip_rect));
        }
    } else if let Ok((width, height, pixels)) =
        client.art.decode_static_tile_from_source(art_id, source, &mut scratch)
    {
        return Some(crop_multi_part_rgba(width as u32, height as u32, pixels, clip_rect));
    }

    None
}

fn crop_multi_part_rgba(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    clip_rect: Option<MultiSourceClip>,
) -> (u32, u32, Vec<u8>) {
    let Some(clip_rect) = clip_rect else {
        return (width, height, pixels);
    };
    let left = clip_rect.left.max(0).min(width as i32) as u32;
    let top = clip_rect.top.max(0).min(height as i32) as u32;
    let right = clip_rect.right.max(0).min(width as i32) as u32;
    let bottom = clip_rect.bottom.max(0).min(height as i32) as u32;
    if right <= left || bottom <= top {
        return (width, height, pixels);
    }
    if left == 0 && top == 0 && right == width && bottom == height {
        return (width, height, pixels);
    }

    let cropped_width = right - left;
    let cropped_height = bottom - top;
    let mut cropped = vec![0u8; cropped_width as usize * cropped_height as usize * 4];
    let src_stride = width as usize * 4;
    let dst_stride = cropped_width as usize * 4;
    for row in 0..cropped_height as usize {
        let src_start = ((top as usize + row) * src_stride) + left as usize * 4;
        let dst_start = row * dst_stride;
        cropped[dst_start..dst_start + dst_stride]
            .copy_from_slice(&pixels[src_start..src_start + dst_stride]);
    }

    (cropped_width, cropped_height, cropped)
}

/// Whether art is kept in an image file format. `artLegacyMUL.uop` keeps the classic encoding, whatever the `.tga` in
/// its entry names says: a TGA reader refuses 279 of the 11,895 tiles of the shipped multis, and takes 82 more for tiny
/// garbage images.
fn art_payload_is_image_file(payload: &[u8], source: ArtSource) -> bool {
    payload.starts_with(b"DDS ") || !matches!(source, ArtSource::Mul | ArtSource::CcUop)
}

fn decode_multi_uop_art_rgba(scratch: &[u8], source: ArtSource) -> Option<(u32, u32, Vec<u8>)> {
    let format = if scratch.starts_with(b"DDS ") {
        ECImageFormat::DDS
    } else {
        ECImageFormat::TGA
    };
    let tex_file = TextureFile {
        metadata: RawTextureItem::absent(),
        is_ec: source.is_ec_uop(),
        format,
        props: None,
        raw_data: Arc::from(scratch),
        image_data_offset: 0,
    };
    let img = tex_file.decode_to_rgba().ok()?;
    let rgba = img.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn log_missing_multi_part_texture(
    app: &mut UopInspectorApp,
    item_id: u16,
    art_id: u32,
    source: ArtSource,
) {
    let has_art = app
        .client_data
        .as_ref()
        .map(|client| client.art.has_id_from_source(art_id, source))
        .unwrap_or(false);
    let message = if app.client_data.is_none() {
        format!(
            "Multi preview cannot render item 0x{item_id:04X}: no CC art map is loaded for {:?}.",
            source
        )
    } else if has_art {
        format!(
            "Multi preview failed to decode item 0x{item_id:04X} as art 0x{art_id:04X} from {:?}.",
            source
        )
    } else {
        format!(
            "Multi preview missing item 0x{item_id:04X}: art 0x{art_id:04X} is not present in {:?}.",
            source
        )
    };
    app.log_once(message);
}

fn concrete_art_tile_source(source: ArtSource) -> ArtSource {
    match source {
        ArtSource::Any => ArtSource::CcUop,
        ArtSource::EcUop => ArtSource::EcUopLegacy,
        source => source,
    }
}

fn multi_preview_art_scale(source: ArtSource) -> f32 {
    match source {
        ArtSource::EcUopKr => {
            CLASSIC_STATIC_TILE_PIXEL_WIDTH / ENHANCED_STATIC_TILE_PIXEL_WIDTH
        }
        ArtSource::Mul | ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::Any => 1.0,
    }
}

/// How to draw a part, worked out once per tile and art source.
fn part_render_info(app: &mut UopInspectorApp, item_id: u16, source: ArtSource) -> PartRenderInfo {
    let key = (item_id, source as u8);
    if let Some(info) = app.multi_preview_cache.render_info.get(&key) {
        return *info;
    }
    if app.multi_preview_cache.tileart_by_tile.is_none() {
        let mut index = HashMap::new();
        for (position, file) in app.ec_tileart_entries.iter().flat_map(|entries| entries.iter().enumerate()) {
            index.entry(file.entry.tile_id).or_insert(position);
        }
        app.multi_preview_cache.tileart_by_tile = Some(index);
    }
    let info = part_render_info_with_fallback(app, item_id, source);
    app.multi_preview_cache.render_info.insert(key, info);
    info
}

/// `source`, or for a tile without KR art the first classic source that has the tile.
fn part_render_info_with_fallback(app: &UopInspectorApp, item_id: u16, source: ArtSource) -> PartRenderInfo {
    let info = work_out_part_render_info(app, item_id, source);
    let Some(art) = app.client_data.as_ref().map(|client| &client.art) else {
        return info;
    };
    if source != ArtSource::EcUopKr || art.has_id_from_source(multi_part_art_id(item_id, &info, source), source) {
        return info;
    }
    KR_ART_FALLBACK_SOURCES
        .into_iter()
        .find(|fallback| art.has_id_from_source(static_art_id_for_source(item_id as u32, *fallback), *fallback))
        .map_or(info, |fallback| PartRenderInfo {
            art_source: Some(fallback),
            ..work_out_part_render_info(app, item_id, fallback)
        })
}

fn work_out_part_render_info(app: &UopInspectorApp, item_id: u16, source: ArtSource) -> PartRenderInfo {
    let mut info = PartRenderInfo::default();

    if let Some(client) = &app.client_data {
        if let Some(tile) = client.tiledata.item_tiles().get(item_id as usize) {
            info.is_floor = tile.flags.surface() && !tile.flags.bridge();
        }
    }

    if let Some(entries) = &app.ec_tileart_entries {
        let position = app.multi_preview_cache.tileart_by_tile.as_ref().and_then(|index| index.get(&(item_id as u32)));
        if let Some(file) = position.and_then(|position| entries.get(*position)) {
            if let Some(texture) = multi_tileart_texture(app, file, source) {
                info.texture_id = Some(texture.texture_id);
                info.clip_rect = Some(MultiSourceClip {
                    left: texture.start_x,
                    top: texture.start_y,
                    right: texture.end_x,
                    bottom: texture.end_y,
                });
                info.width_delta = texture.start_y - texture.end_y;
                info.offset_x = texture.offset_x;
                info.offset_y = texture.offset_y;
            } else {
                let image_offset = match source {
                    ArtSource::EcUopKr => &file.entry.ec_img_offset,
                    ArtSource::EcUop | ArtSource::EcUopLegacy => &file.entry.cc_img_offset,
                    ArtSource::Mul | ArtSource::CcUop | ArtSource::Any => &file.entry.cc_img_offset,
                };
                info.width_delta = image_offset.y_start - image_offset.y_end;
                info.height_delta = image_offset.x_start;
                info.offset_x = image_offset.x_off;
                info.offset_y = image_offset.y_off;
            }
        }
    }

    info
}

fn multi_tileart_texture(
    app: &UopInspectorApp,
    file: &crate::app::TileArtFileEntry,
    source: ArtSource,
) -> Option<ArtTexture> {
    let dictionary = app.uo_string_dictionary.as_deref()?;
    let art_data = file.entry.process(dictionary);
    match source {
        ArtSource::EcUopKr => art_data.ec_texture,
        ArtSource::Mul | ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::Any => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MISSING_TILE: u16 = 0x82D1;
    const TEST_PAYLOAD_BYTES: usize = 32;
    const REDRAWS: usize = 3;

    fn part(hidden: bool, housing_piece: bool) -> PreviewPart {
        PreviewPart {
            source_index: 0,
            item_id: 0,
            x: 0,
            y: 0,
            z: 0,
            original_z: 0,
            flags: String::new(),
            hidden,
            housing_piece,
        }
    }

    #[test]
    fn classic_art_in_a_uop_is_not_read_as_an_image_file() {
        let classic_payload = [0u8; TEST_PAYLOAD_BYTES];
        assert!(!art_payload_is_image_file(&classic_payload, ArtSource::CcUop));
        assert!(!art_payload_is_image_file(&classic_payload, ArtSource::Mul));
        assert!(art_payload_is_image_file(&classic_payload, ArtSource::EcUopLegacy));
        assert!(art_payload_is_image_file(b"DDS ", ArtSource::CcUop));
    }

    #[test]
    fn a_missing_part_is_looked_for_and_logged_once_however_many_redraws() {
        let ctx = egui::Context::default();
        let mut app = crate::app::tests::test_app();
        let info = PartRenderInfo::default();
        let logged = |app: &UopInspectorApp| app.logs.iter().filter(|log| log.contains("Multi preview")).count();

        assert!(get_multi_part_texture(&mut app, &ctx, MISSING_TILE, &info).is_none());
        let first = logged(&app);
        assert_eq!(first, 1);
        for _ in 0..REDRAWS {
            assert!(get_multi_part_texture(&mut app, &ctx, MISSING_TILE, &info).is_none());
        }
        assert_eq!(logged(&app), first);
    }

    #[test]
    fn a_part_is_worked_out_once_and_again_after_the_data_changed() {
        let mut app = crate::app::tests::test_app();
        refresh_multi_preview_cache(&mut app);
        part_render_info(&mut app, MISSING_TILE, ArtSource::CcUop);
        part_render_info(&mut app, MISSING_TILE, ArtSource::CcUop);
        assert_eq!(app.multi_preview_cache.render_info.len(), 1);

        refresh_multi_preview_cache(&mut app);
        assert_eq!(app.multi_preview_cache.render_info.len(), 1, "nothing changed");

        app.ec_tileart_entries = Some(Arc::new(Vec::new()));
        refresh_multi_preview_cache(&mut app);
        assert!(app.multi_preview_cache.render_info.is_empty());
    }

    #[test]
    fn hidden_parts_are_filtered_out_by_default() {
        let filters = MultiPartFilters::default();

        assert!(part_passes_filters(&part(false, false), filters, true));
        assert!(!part_passes_filters(&part(true, false), filters, true));
        assert!(part_passes_filters(
            &part(true, false),
            MultiPartFilters { show_hidden: true, ..filters },
            true
        ));
    }

    #[test]
    fn housing_filters_split_parts_only_when_housing_is_loaded() {
        let only_housing = MultiPartFilters { show_other_parts: false, ..MultiPartFilters::default() };
        let only_other = MultiPartFilters { show_housing_parts: false, ..MultiPartFilters::default() };

        assert!(part_passes_filters(&part(false, true), only_housing, true));
        assert!(!part_passes_filters(&part(false, false), only_housing, true));
        assert!(part_passes_filters(&part(false, false), only_other, true));
        assert!(!part_passes_filters(&part(false, true), only_other, true));
        assert!(part_passes_filters(&part(false, false), only_housing, false));
    }
}
