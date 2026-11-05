//! The Tile Metadata tab: the tiles of tiledata.mul or the records of tileart.uop in a table (searched by name or by
//! any field, with a go-to by id and the arrow keys moving the selection), and below it the selected one's fields
//! beside its art or textures ([`tile_preview`]).

use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};
use uocf::classic::tiledata::TileData;
use uocf::enhanced::tileart::{tileart_entry_hash, PropertyKey, TileArtEntry};

use super::{
    arrow_delta, enhanced_files_availability, enhanced_folder_source, folder_files_availability, list_sort_controls,
    move_selection, sorted_indices_by, source_choice, tile_preview, tileart_editor, tiledata_editor, ClientSource,
    SourceAvailability, CLASSIC_FOLDER,
};
use crate::app::{concrete_art_tile_source, CcTileDataRow, TileArtDisplayRow, TileArtFileEntry, TileMetadataSource, UopInspectorApp};
use crate::logic::effects_editor::parse_id;

/// The art id of the first item tile; a land tile's art id is its tile id.
const ITEM_ART_ID_BASE: u32 = uocf::classic::art::STATIC_TILE_ID_BASE;
/// Starting height of a table above the selected record's details; the user can drag it.
const METADATA_TABLE_HEIGHT: f32 = 300.0;
/// A table may be dragged this low and still keep its header and a row.
const METADATA_TABLE_MIN_SCROLLED_HEIGHT: f32 = 40.0;
const TABLE_HEADER_HEIGHT: f32 = 20.0;
/// Starting width of the art and texture preview beside the record's fields.
const PREVIEW_PANEL_WIDTH: f32 = 440.0;
const GO_TO_FIELD_WIDTH: f32 = 90.0;
const HEX_DIGITS: usize = 4;
const TAB_STATE_ID: &str = "tile_metadata_tab";
const TILEDATA_FILE: &str = "tiledata.mul";
const TILEART_FILE: &str = "tileart.uop";
/// The tiledata.mul columns and their least widths; the last one takes the rest.
const TILEDATA_COLUMNS: [(&str, f32); 12] = [
    ("ID", 60.0),
    ("Hex", 60.0),
    ("Type", 50.0),
    ("Name", 160.0),
    ("Texture", 60.0),
    ("Height", 50.0),
    ("Weight", 50.0),
    ("Layer/Light", 80.0),
    ("Qty", 50.0),
    ("Anim", 60.0),
    ("Value", 50.0),
    ("Flags", 220.0),
];
/// The tileart.uop columns and their least widths; the last one takes the rest.
const TILEART_COLUMNS: [(&str, f32); 11] = [
    ("ID", 60.0),
    ("Hex", 60.0),
    ("Name", 160.0),
    ("Old ID", 60.0),
    ("Body", 50.0),
    ("Height", 50.0),
    ("Weight", 50.0),
    ("Qty", 50.0),
    ("Layer", 50.0),
    ("Value", 50.0),
    ("Flags", 220.0),
];
/// The properties the tileart.uop table shows, by column after the body type.
const TILEART_PROPERTY_COLUMNS: [PropertyKey; 5] =
    [PropertyKey::Height, PropertyKey::Weight, PropertyKey::Quantity, PropertyKey::Layer, PropertyKey::Value];

/// What the search field looks in.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum SearchScope {
    #[default]
    Name,
    AnyField,
}

/// The tab's controls that live for the session.
#[derive(Clone, Default, Debug)]
struct TabState {
    scope: SearchScope,
    go_to: String,
    /// Whether a tiledata.mul go-to means a land tile (else an item tile).
    go_to_land: bool,
}

pub fn ui_tile_metadata(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        contents(app, ctx, ui);
    });
}

/// tiledata.mul is MUL, tileart.uop KR UOP or EC UOP by the Enhanced folder's client. The Classic client keeps its
/// tile metadata in tiledata.mul only: no CC UOP.
fn source_selector(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let cc_path = app.settings.cc_path.as_deref();
    let ec_path = app.settings.ec_path.as_deref();
    let has_tiledata = app.cc_tiledata.is_some();
    let has_tileart = app.ec_tileart_entries.is_some();
    let selected = match app.tile_metadata_source {
        TileMetadataSource::CcTileData => Some(ClientSource::Mul),
        TileMetadataSource::EcTileArt => ec_path.map(|folder| enhanced_folder_source(&ctx, folder)),
    };
    let clicked = source_choice(ui, selected, |source| match source {
        ClientSource::Mul => folder_files_availability(cc_path, CLASSIC_FOLDER, TILEDATA_FILE, has_tiledata),
        ClientSource::CcUop => SourceAvailability::NotApplicable,
        ClientSource::KrUop | ClientSource::EcUop => {
            enhanced_files_availability(&ctx, ec_path, source, TILEART_FILE, has_tileart)
        }
    });
    match clicked {
        Some(ClientSource::Mul) => app.tile_metadata_source = TileMetadataSource::CcTileData,
        Some(_) => app.tile_metadata_source = TileMetadataSource::EcTileArt,
        None => {}
    }
}

/// The tab's contents, also drawn by the Art tab's tiledata view.
pub(super) fn contents(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Tile Metadata");
        ui.separator();
        source_selector(app, ui);
        ui.separator();
        if ui.button("Raw UOP").clicked() {
            match app.tile_metadata_source {
                TileMetadataSource::CcTileData => {
                    app.status_message = "tiledata.mul is not UOP-backed.".to_string();
                }
                TileMetadataSource::EcTileArt => {
                    if let Some(hash) = app.selected_tileart_hash {
                        app.select_raw_uop_entry("tileart.uop", hash);
                    }
                }
            }
        }
    });

    let state_id = egui::Id::new(TAB_STATE_ID);
    let mut state = ui.data(|data| data.get_temp::<TabState>(state_id)).unwrap_or_default();
    let classic = app.tile_metadata_source == TileMetadataSource::CcTileData;
    let mut go_to = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Search:");
        ui.text_edit_singleline(&mut app.searches.tile_metadata);
        egui::ComboBox::from_id_salt("tile_metadata_search_scope")
            .selected_text(match state.scope {
                SearchScope::Name => "in names",
                SearchScope::AnyField => "in any field",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.scope, SearchScope::Name, "in names");
                ui.selectable_value(&mut state.scope, SearchScope::AnyField, "in any field");
            });
        ui.separator();
        ui.label("Go to:");
        if classic {
            ui.selectable_value(&mut state.go_to_land, false, "Item");
            ui.selectable_value(&mut state.go_to_land, true, "Land");
        }
        let field = ui.add(egui::TextEdit::singleline(&mut state.go_to).desired_width(GO_TO_FIELD_WIDTH).hint_text("id or 0x..."));
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.button("Go").clicked() || entered {
            go_to = Some(parse_id(&state.go_to).ok_or_else(|| format!("{:?} is not a tile id (decimal or 0x hex).", state.go_to.trim())));
        }
    });
    ui.separator();
    let go_to = match go_to {
        Some(Err(message)) => {
            app.status_message = message;
            None
        }
        Some(Ok(id)) => Some(id),
        None => None,
    };
    // Arrow keys move the selection unless they edit text: the search, the go-to or a field of the record.
    let keys_free = !ctx.text_edit_focused();
    let (scope, go_to_land) = (state.scope, state.go_to_land);
    ui.data_mut(|data| data.insert_temp(state_id, state));
    match app.tile_metadata_source {
        TileMetadataSource::CcTileData => cc_tiledata(app, ctx, ui, scope, go_to.map(|id| (id, go_to_land)), keys_free),
        TileMetadataSource::EcTileArt => ec_tileart(app, ctx, ui, scope, go_to, keys_free),
    }
}

/// Whether a row is shown for `query` (lowercase; empty shows every row).
fn matches(scope: SearchScope, query: &str, name: &str, search_text: &str) -> bool {
    query.is_empty()
        || match scope {
            SearchScope::Name => contains_ignoring_ascii_case(name, query),
            SearchScope::AnyField => search_text.contains(query),
        }
}

fn contains_ignoring_ascii_case(text: &str, lowercase_query: &str) -> bool {
    let needle = lowercase_query.as_bytes();
    needle.is_empty() || text.as_bytes().windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle))
}

/// The art id of tile `id` of tiledata.mul, a land tile when `land`; None when the file has no such tile.
fn cc_go_to_target(tiledata: &TileData, id: u32, land: bool) -> Option<u32> {
    if land {
        ((id as usize) < tiledata.land_tiles().len()).then_some(id)
    } else {
        ((id as usize) < tiledata.item_tiles().len()).then(|| id + ITEM_ART_ID_BASE)
    }
}

/// The entry of tile `id`: the one named after it, else the first that stores it.
fn ec_go_to_target(entries: &[TileArtFileEntry], id: u32) -> Option<u64> {
    let hash = tileart_entry_hash(id);
    entries
        .iter()
        .find(|file| file.filename_hash == hash)
        .or_else(|| entries.iter().find(|file| file.entry.tile_id == id))
        .map(|file| file.filename_hash)
}

/// The tile id of a tiledata.mul art id.
fn cc_tile_id(art_id: u32) -> u32 {
    art_id.checked_sub(ITEM_ART_ID_BASE).unwrap_or(art_id)
}

/// A property's value as the client reads it (props1 first, then props2); empty when the record has none.
fn property_text(entry: &TileArtEntry, key: PropertyKey) -> String {
    let key = key as u8;
    entry
        .prop_vector1
        .iter()
        .chain(&entry.prop_vector2)
        .find(|prop| prop.id == key)
        .map(|prop| prop.val.to_string())
        .unwrap_or_default()
}

/// A resizable table on top of the remaining space with `columns`; `cells` draws the shown row at a position.
fn metadata_table(
    ui: &mut egui::Ui,
    id: &'static str,
    columns: &[(&str, f32)],
    row_count: usize,
    scroll_to: Option<usize>,
    mut cells: impl FnMut(usize, &mut TableRow<'_, '_>),
) {
    let min_width = columns.iter().map(|(_, width)| width).sum::<f32>();
    let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
    egui::Panel::top(id).resizable(true).default_size(METADATA_TABLE_HEIGHT).show(ui, |ui| {
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_width(min_width);
            let mut builder = TableBuilder::new(ui)
                .auto_shrink([false, false])
                .min_scrolled_height(METADATA_TABLE_MIN_SCROLLED_HEIGHT)
                .max_scroll_height(f32::INFINITY)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
            let (last, others) = columns.split_last().expect("a table has columns");
            for (_, width) in others {
                builder = builder.column(Column::auto().at_least(*width));
            }
            builder = builder.column(Column::remainder().at_least(last.1));
            if let Some(row) = scroll_to {
                builder = builder.scroll_to_row(row, Some(egui::Align::Center));
            }
            builder
                .header(TABLE_HEADER_HEIGHT, |mut header| {
                    for (title, _) in columns {
                        header.col(|ui| {
                            ui.strong(*title);
                        });
                    }
                })
                .body(|body| {
                    body.rows(text_height, row_count, |mut row| {
                        let index = row.index();
                        cells(index, &mut row);
                    });
                });
        });
    });
}

/// The art or textures of the selection, on the right of the space under a table; drawn before [`record_pane`].
fn preview_pane(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Panel::right("tile_metadata_preview").resizable(true).default_size(PREVIEW_PANEL_WIDTH).show(ui, add_contents);
}

/// The fields of the selection, in the rest of the space under a table.
fn record_pane<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::CentralPanel::default().show(ui, add_contents).inner
}

fn cc_tiledata(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    scope: SearchScope,
    go_to: Option<(u32, bool)>,
    keys_free: bool,
) {
    ui.heading("CC TileData Inspector");
    if app.cc_tiledata.is_some() {
        ui.horizontal_wrapped(|ui| tiledata_editor::toolbar(app, ui));
    }
    let (Some(tiledata), Some(rows)) = (app.cc_tiledata.clone(), app.cc_tiledata_rows.clone()) else {
        ui.label("Select a Classic Client path containing tiledata.mul.");
        return;
    };
    let mut scroll = false;
    if let Some((id, land)) = go_to {
        match cc_go_to_target(&tiledata, id, land) {
            Some(art_id) => {
                let query = app.searches.tile_metadata.trim().to_lowercase();
                if let Some(row) = rows.iter().find(|row| row.art_id == art_id) {
                    if !matches(scope, &query, &row.name, &row.search_text) {
                        app.searches.tile_metadata.clear();
                    }
                }
                app.selected_tex_art_cc_id = Some(art_id);
                scroll = true;
            }
            None => {
                let kind = if land { "land" } else { "item" };
                app.status_message = format!("tiledata.mul has no {kind} tile {id} (0x{id:X}).");
            }
        }
    }
    let query = app.searches.tile_metadata.trim().to_lowercase();
    let sort = list_sort_controls(ui, "cc_tiledata_rows", &["ID", "Name", "Type", "Texture", "Height", "Weight", "Anim", "Value"], 0);
    let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| match sort.option_index {
        1 => left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()),
        2 => left.kind.cmp(right.kind).then_with(|| left.art_id.cmp(&right.art_id)),
        3 => string_number(&left.texture_id).cmp(&string_number(&right.texture_id)),
        4 => string_number(&left.height).cmp(&string_number(&right.height)),
        5 => string_number(&left.weight).cmp(&string_number(&right.weight)),
        6 => string_number(&left.anim_id).cmp(&string_number(&right.anim_id)),
        7 => string_number(&left.value).cmp(&string_number(&right.value)),
        _ => left.art_id.cmp(&right.art_id),
    });
    let shown: Vec<usize> =
        sorted_indices.into_iter().filter(|&index| matches(scope, &query, &rows[index].name, &rows[index].search_text)).collect();
    let shown_ids: Vec<u32> = shown.iter().map(|&index| rows[index].art_id).collect();
    if let Some(delta) = arrow_delta(ui, !keys_free) {
        app.selected_tex_art_cc_id = move_selection(&shown_ids, app.selected_tex_art_cc_id, delta);
        scroll = true;
    }
    let scroll_to = scroll.then(|| app.selected_tex_art_cc_id.and_then(|id| shown_ids.iter().position(|&shown| shown == id))).flatten();
    ui.label(format!(
        "{} land tiles, {} item tiles ({} flag bits); {} shown",
        tiledata.land_tiles().len(),
        tiledata.item_tiles().len(),
        if tiledata.has_64_bit_flags() { 64 } else { 32 },
        shown.len()
    ));
    let selected = &mut app.selected_tex_art_cc_id;
    metadata_table(ui, "tiledata_table", &TILEDATA_COLUMNS, shown.len(), scroll_to, |position, row| {
        cc_row_cells(row, &rows[shown[position]], selected);
    });

    let change = match app.selected_tex_art_cc_id {
        Some(art_id) if rows.iter().any(|row| row.art_id == art_id) => {
            let art = app.get_tex_art_texture_from_source(ctx, art_id, concrete_art_tile_source(app.selected_legacy_source));
            let editable = app.tiledata_editor.enabled;
            let kind = if art_id < ITEM_ART_ID_BASE { "land" } else { "item" };
            preview_pane(ui, |ui| tile_preview::art_preview(ui, art.as_ref()));
            record_pane(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("tiledata_record").auto_shrink([false, false]).show(ui, |ui| {
                    let tile_id = cc_tile_id(art_id);
                    ui.heading(format!("Selected {kind} tile {tile_id} (0x{tile_id:0HEX_DIGITS$X})"));
                    tiledata_editor::record_panel(ui, &tiledata, art_id, editable)
                })
                .inner
            })
        }
        _ => None,
    };
    // The change goes into the loaded tiles, which must not be shared then (the copies above are dropped).
    drop((tiledata, rows));
    if let Some(change) = change {
        tiledata_editor::apply_change(app, change);
    }
}

fn cc_row_cells(row: &mut TableRow<'_, '_>, item: &CcTileDataRow, selected: &mut Option<u32>) {
    row.col(|ui| {
        if ui.selectable_label(*selected == Some(item.art_id), &item.id).clicked() {
            *selected = Some(item.art_id);
        }
    });
    row.col(|ui| {
        ui.monospace(format!("0x{:0HEX_DIGITS$X}", cc_tile_id(item.art_id)));
    });
    let texts = [
        item.kind,
        item.name.as_str(),
        item.texture_id.as_str(),
        item.height.as_str(),
        item.weight.as_str(),
        item.quality.as_str(),
        item.quantity.as_str(),
        item.anim_id.as_str(),
        item.value.as_str(),
    ];
    for text in texts {
        row.col(|ui| {
            ui.label(text);
        });
    }
    row.col(|ui| {
        ui.label(&item.flags_summary).on_hover_text(&item.flags_raw);
    });
}

fn ec_tileart(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    scope: SearchScope,
    go_to: Option<u32>,
    keys_free: bool,
) {
    ui.heading("EC TileArt Inspector");
    let mut edits = Vec::new();
    if app.ec_tileart_entries.is_some() {
        ui.horizontal_wrapped(|ui| tileart_editor::toolbar(app, ctx, ui, &mut edits));
    }
    let (Some(entries), Some(rows)) = (app.ec_tileart_entries.clone(), app.ec_tileart_rows.clone()) else {
        ui.label("Select an Enhanced Client path containing tileart.uop.");
        tileart_editor::apply_edits(app, edits);
        return;
    };
    let mut scroll = false;
    if let Some(id) = go_to {
        match ec_go_to_target(&entries, id) {
            Some(hash) => {
                let query = app.searches.tile_metadata.trim().to_lowercase();
                if let Some(row) = rows.iter().find(|row| row.filename_hash == hash) {
                    if !matches(scope, &query, &row.name, &row.search_text) {
                        app.searches.tile_metadata.clear();
                    }
                }
                app.selected_tileart_hash = Some(hash);
                scroll = true;
            }
            None => app.status_message = format!("tileart.uop has no record of tile {id} (0x{id:X})."),
        }
    }
    let query = app.searches.tile_metadata.trim().to_lowercase();
    let sort = list_sort_controls(ui, "ec_tileart_rows", &["ID", "Name", "Old ID", "Body Type", "Hash"], 0);
    let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| match sort.option_index {
        1 => left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()),
        2 => string_number(&left.old_id).cmp(&string_number(&right.old_id)),
        3 => string_number(&left.body_type)
            .cmp(&string_number(&right.body_type))
            .then_with(|| string_number(&left.tile_id).cmp(&string_number(&right.tile_id))),
        4 => left.filename_hash.cmp(&right.filename_hash),
        _ => string_number(&left.tile_id).cmp(&string_number(&right.tile_id)),
    });
    let shown: Vec<usize> =
        sorted_indices.into_iter().filter(|&index| matches(scope, &query, &rows[index].name, &rows[index].search_text)).collect();
    let shown_hashes: Vec<u64> = shown.iter().map(|&index| rows[index].filename_hash).collect();
    if let Some(delta) = arrow_delta(ui, !keys_free) {
        app.selected_tileart_hash = move_selection(&shown_hashes, app.selected_tileart_hash, delta);
        scroll = true;
    }
    let scroll_to =
        scroll.then(|| app.selected_tileart_hash.and_then(|hash| shown_hashes.iter().position(|&shown| shown == hash))).flatten();
    ui.label(format!("{} tileart.uop entries; {} shown", entries.len(), shown.len()));
    let selected = &mut app.selected_tileart_hash;
    metadata_table(ui, "tileart_table", &TILEART_COLUMNS, shown.len(), scroll_to, |position, row| {
        let index = shown[position];
        ec_row_cells(row, &rows[index], &entries[index].entry, selected);
    });

    if let Some(file) = app.selected_tileart_hash.and_then(|hash| entries.iter().find(|file| file.filename_hash == hash)) {
        preview_pane(ui, |ui| tile_preview::tileart_preview(app, ctx, ui, &file.entry));
        record_pane(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("tileart_record").auto_shrink([false, false]).show(ui, |ui| {
                let tile_id = file.entry.tile_id;
                ui.heading(format!("Selected tileart {tile_id} (0x{tile_id:0HEX_DIGITS$X})"));
                let dictionary = app.uo_string_dictionary.clone();
                let name = dictionary
                    .as_deref()
                    .and_then(|dictionary| dictionary.get_string((file.entry.string_dict_off as usize).checked_sub(1)?));
                let editable = app.tileart_editor.enabled;
                tileart_editor::record_panel(ui, file, name, dictionary.as_deref(), &mut app.tileart_editor.effects_draft, editable, &mut edits);
            });
        });
    }
    // The edits go into the loaded records, which must not be shared then (the table's copies are dropped).
    drop((entries, rows));
    tileart_editor::apply_edits(app, edits);
}

fn ec_row_cells(row: &mut TableRow<'_, '_>, item: &TileArtDisplayRow, entry: &TileArtEntry, selected: &mut Option<u64>) {
    row.col(|ui| {
        if ui.selectable_label(*selected == Some(item.filename_hash), &item.tile_id).clicked() {
            *selected = Some(item.filename_hash);
        }
    });
    row.col(|ui| {
        ui.monospace(format!("0x{:0HEX_DIGITS$X}", entry.tile_id));
    });
    for text in [&item.name, &item.old_id, &item.body_type] {
        row.col(|ui| {
            ui.label(text);
        });
    }
    for key in TILEART_PROPERTY_COLUMNS {
        row.col(|ui| {
            ui.label(property_text(entry, key));
        });
    }
    row.col(|ui| {
        ui.label(&item.flags_summary).on_hover_text(&item.flags_raw);
    });
}

fn string_number(value: &str) -> u32 {
    value.parse().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::enhanced::tileart::TaeProp;

    #[test]
    fn a_name_search_ignores_case_and_other_fields() {
        assert!(matches(SearchScope::Name, "ankh", "Ankh of Sacrifice", "1 item ankh"));
        assert!(!matches(SearchScope::Name, "1234", "Ankh", "1234 item ankh"));
        assert!(matches(SearchScope::AnyField, "1234", "Ankh", "1234 item ankh"));
        assert!(matches(SearchScope::Name, "", "", ""));
        assert!(!matches(SearchScope::Name, "ankh", "", "ankh"));
    }

    #[test]
    fn a_go_to_finds_the_entry_named_after_the_tile_before_one_storing_it() {
        const NAMED: u32 = 43957;
        const STORED: u32 = 43956;
        let file = |hash, tile_id| TileArtFileEntry { filename_hash: hash, entry: TileArtEntry { tile_id, ..TileArtEntry::default() } };
        let entries = vec![file(tileart_entry_hash(STORED), STORED), file(tileart_entry_hash(NAMED), STORED)];
        assert_eq!(ec_go_to_target(&entries, NAMED), Some(tileart_entry_hash(NAMED)));
        assert_eq!(ec_go_to_target(&entries, STORED), Some(tileart_entry_hash(STORED)));
        let unnamed = vec![file(1, NAMED)];
        assert_eq!(ec_go_to_target(&unnamed, NAMED), Some(1));
        assert_eq!(ec_go_to_target(&unnamed, STORED), None);
    }

    #[test]
    fn tiledata_art_ids_hold_the_tile_id() {
        assert_eq!(cc_tile_id(0x3FFF), 0x3FFF);
        assert_eq!(cc_tile_id(ITEM_ART_ID_BASE + 0x0E75), 0x0E75);
    }

    #[test]
    fn a_property_column_reads_props1_before_props2() {
        const WEIGHT: u8 = PropertyKey::Weight as u8;
        let entry = TileArtEntry {
            prop_vector1: vec![TaeProp { id: WEIGHT, val: 3 }],
            prop_vector2: vec![TaeProp { id: WEIGHT, val: 9 }],
            ..TileArtEntry::default()
        };
        assert_eq!(property_text(&entry, PropertyKey::Weight), "3");
        assert_eq!(property_text(&entry, PropertyKey::Height), "");
    }
}
