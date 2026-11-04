//! Editing controls of the tiledata.mul view of the Tile Metadata tab: every stored field of the selected tile, and a
//! save that writes a copy through `TileData::to_bytes`. A tile keeps every byte the controls do not change, the block
//! headers and the unknown fields included. Field meanings follow `uocf::classic::tiledata`.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use uocf::classic::tiledata::{Flags, ItemTile, LandTile, TileData};
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{cc_item_row, cc_land_row, UopInspectorApp, ViewMode};
use crate::ui::tileart_editor::{group, record_columns};
use crate::ui::unsaved_changes::TabEditor;

const TILEDATA_FILE_NAME: &str = "tiledata.mul";
const MUL_FILTER: (&str, &[&str]) = ("MUL file", &["mul"]);
/// The art id of the first item tile; a land tile's art id is its tile id.
const ITEM_ART_ID_BASE: u32 = uocf::classic::art::STATIC_TILE_ID_BASE;
/// Room for one flag's checkbox and name: the group has as many flag columns as fit.
const FLAG_CELL_WIDTH: f32 = 140.0;
const FLAG_HEX_DIGITS: usize = 16;
const FLAG_FIELD_WIDTH: f32 = 150.0;
const NAME_FIELD_WIDTH: f32 = 160.0;
/// Drag speed of the number fields (typing is the main input).
const VALUE_DRAG_SPEED: f64 = 0.1;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const DIRTY_MARKER: &str = "*";
/// Largest side of the tile's art beside its fields.
const ART_PREVIEW_SIDE: f32 = 128.0;

/// The flags by bit, named as `classic_tile_flags_summary` names them. Unnamed bits stay reachable through the value.
const FLAG_BITS: [(u32, &str); 40] = [
    (0, "Background"),
    (1, "Weapon"),
    (2, "Transparent"),
    (3, "Translucent"),
    (4, "Wall"),
    (5, "Damaging"),
    (6, "Impassable"),
    (7, "Wet"),
    (8, "Unknown08"),
    (9, "Surface"),
    (10, "Bridge"),
    (11, "Generic/Stackable"),
    (12, "Window"),
    (13, "NoShoot"),
    (14, "ArticleA"),
    (15, "ArticleAn"),
    (16, "Internal"),
    (17, "Foliage"),
    (18, "PartialHue"),
    (19, "Unknown19"),
    (20, "Map"),
    (21, "Container"),
    (22, "Wearable"),
    (23, "LightSource"),
    (24, "Animated"),
    (25, "NoDiagonal"),
    (26, "Unknown26"),
    (27, "Armor"),
    (28, "Roof"),
    (29, "Door"),
    (30, "StairBack"),
    (31, "StairRight"),
    (32, "NoHouse"),
    (33, "NoDraw"),
    (34, "Rotated"),
    (35, "AlphaBlend"),
    (36, "NoShadow"),
    (37, "PixelBleed"),
    (39, "PlayAnimOnce"),
    (40, "MultiMovable"),
];

/// The state of the editor.
#[derive(Default)]
pub struct TileDataEditor {
    pub enabled: bool,
    /// The tiles as they were before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<Arc<TileData>>,
    error: Option<String>,
}

impl TileDataEditor {
    pub fn has_unsaved_changes(&self) -> bool {
        self.opened.is_some()
    }
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::TileMetadata,
    label: "the tiledata.mul tiles",
    is_dirty: |app| app.tiledata_editor.has_unsaved_changes(),
    save,
    discard: revert,
};

/// A tile's new contents, collected while drawing and stored by [`apply_change`].
pub(super) enum TileChange {
    Land(usize, LandTile),
    Item(usize, ItemTile),
}

/// The editing toolbar row.
pub(super) fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.toggle_value(&mut app.tiledata_editor.enabled, "Edit").on_hover_text("Edit the selected tile; save a copy of tiledata.mul.");
    if !app.tiledata_editor.enabled {
        return;
    }
    ui.separator();
    if ui.add_enabled(app.tiledata_editor.has_unsaved_changes(), egui::Button::new("Revert to loaded")).clicked() {
        revert(app);
    }
    if ui
        .button("Save tiledata.mul...")
        .on_hover_text("Writes a copy of the loaded tiles, verdata patches included when the client has them. The loaded file itself is refused as the target.")
        .clicked()
    {
        save(app);
    }
    if app.tiledata_editor.has_unsaved_changes() {
        ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
    }
    if let Some(error) = app.tiledata_editor.error.clone() {
        ui.colored_label(ERROR_COLOR, error);
        if ui.small_button("Dismiss").clicked() {
            app.tiledata_editor.error = None;
        }
    }
}

fn source_path(app: &UopInspectorApp) -> Option<PathBuf> {
    find_file_ignoring_case(app.settings.cc_path.as_deref()?, TILEDATA_FILE_NAME)
}

fn save(app: &mut UopInspectorApp) {
    let Some(tiledata) = app.cc_tiledata.clone() else {
        return;
    };
    let (name, extensions) = MUL_FILTER;
    let Some(output) = crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(TILEDATA_FILE_NAME).save_file() else {
        return;
    };
    if source_path(app).is_some_and(|source| same_file(&source, &output)) {
        app.tiledata_editor.error = Some("The loaded tiledata.mul is refused as the target: save a copy.".to_owned());
        return;
    }
    match tiledata.save(&output) {
        Ok(()) => {
            app.tiledata_editor.opened = None;
            app.tiledata_editor.error = None;
            let message = format!("Saved {}.", output.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.tiledata_editor.error = Some(format!("Save failed: {error:#}")),
    }
}

fn same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn revert(app: &mut UopInspectorApp) {
    if let Some(opened) = app.tiledata_editor.opened.take() {
        app.cc_tiledata_rows = Some(Arc::new(crate::app::collect_cc_tiledata_rows(&opened)));
        app.cc_tiledata = Some(opened);
    }
}

/// The tile of an art id: land tiles first, then item tiles.
fn tile_change_slot(tiledata: &TileData, art_id: u32) -> Option<TileChange> {
    let index = art_id.checked_sub(ITEM_ART_ID_BASE);
    match index {
        None => tiledata.land_tiles().get(art_id as usize).map(|tile| TileChange::Land(art_id as usize, tile.clone())),
        Some(index) => tiledata.item_tiles().get(index as usize).map(|tile| TileChange::Item(index as usize, tile.clone())),
    }
}

/// The fields of the tile `art_id`, beside its art; returns its new contents when one changed.
pub(super) fn record_panel(ui: &mut egui::Ui, tiledata: &TileData, art_id: u32, art: Option<&egui::TextureHandle>, editable: bool) -> Option<TileChange> {
    let mut tile = tile_change_slot(tiledata, art_id)?;
    let mut changed = false;
    record_columns(ui, editable, |columns| {
        group(columns, "Tile", |ui| {
            egui::Grid::new(("tiledata_fields", art_id)).num_columns(2).striped(true).show(ui, |ui| match &mut tile {
                TileChange::Land(_, tile) => changed |= land_fields(ui, tile),
                TileChange::Item(_, tile) => changed |= item_fields(ui, tile),
            });
        });
        let flags = match &mut tile {
            TileChange::Land(_, tile) => &mut tile.flags,
            TileChange::Item(_, tile) => &mut tile.flags,
        };
        group(columns, "Flags", |ui| changed |= flag_fields(ui, art_id, flags));
        group(columns, "Art", |ui| match art {
            Some(art) => {
                let scale = (ART_PREVIEW_SIDE / art.size_vec2().max_elem()).min(1.0);
                ui.add(egui::Image::new(art).fit_to_exact_size(art.size_vec2() * scale));
            }
            None => {
                ui.weak("no art for this tile in the source chosen in the Art tab");
            }
        });
    });
    changed.then_some(tile)
}

fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, label: &str, value: &mut T) -> bool {
    ui.label(label);
    let changed = ui.add(egui::DragValue::new(value).speed(VALUE_DRAG_SPEED)).changed();
    ui.end_row();
    changed
}

/// A name field: ASCII up to the field's size, the rest of the field zeroed. An unchanged name keeps its bytes.
fn name_field(ui: &mut egui::Ui, name: &mut [u8], current: &str) -> bool {
    ui.label("Name");
    let mut text = current.to_owned();
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(NAME_FIELD_WIDTH).char_limit(name.len()));
    ui.end_row();
    if !response.changed() || !text.is_ascii() {
        return false;
    }
    store_name(name, &text);
    true
}

fn store_name(name: &mut [u8], text: &str) {
    name.fill(0);
    name[..text.len()].copy_from_slice(text.as_bytes());
}

fn land_fields(ui: &mut egui::Ui, tile: &mut LandTile) -> bool {
    ui.label("Id");
    ui.label(format!("{0} (0x{0:04X})", tile.tile_id));
    ui.end_row();
    let current = tile.name_ascii().to_owned();
    let mut changed = name_field(ui, &mut tile.name, &current);
    changed |= number(ui, "Texture id", &mut tile.texture_id);
    changed
}

fn item_fields(ui: &mut egui::Ui, tile: &mut ItemTile) -> bool {
    ui.label("Id");
    ui.label(format!("{0} (0x{0:04X})", tile.tile_id));
    ui.end_row();
    let current = tile.name_ascii().to_owned();
    let mut changed = name_field(ui, &mut tile.name, &current);
    changed |= number(ui, "Weight", &mut tile.weight);
    changed |= number(ui, "Quality / Layer / Light", &mut tile.quality);
    changed |= number(ui, "Quantity", &mut tile.quantity);
    changed |= number(ui, "Animation id", &mut tile.anim_id);
    changed |= number(ui, "Hue extra", &mut tile.hue_extra);
    changed |= number(ui, "Stacking offset", &mut tile.stacking_offset);
    changed |= number(ui, "Value", &mut tile.value);
    changed |= number(ui, "Height", &mut tile.height);
    changed |= number(ui, "unknown0", &mut tile.unknown0);
    changed |= number(ui, "unknown1", &mut tile.unknown1);
    changed |= number(ui, "unknown2", &mut tile.unknown2);
    changed
}

/// The flag value (applied when the field loses focus) and a checkbox per named bit.
fn flag_fields(ui: &mut egui::Ui, art_id: u32, flags: &mut Flags) -> bool {
    let mut changed = false;
    let buffer_id = ui.make_persistent_id(("tiledata_flags_hex", art_id));
    let mut text = ui
        .data_mut(|data| data.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| format!("{:0FLAG_HEX_DIGITS$X}", flags.internal_flags));
    let response = ui
        .horizontal(|ui| {
            ui.label("Val:");
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(FLAG_FIELD_WIDTH))
        })
        .inner;
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, text));
    } else {
        ui.data_mut(|data| data.remove::<String>(buffer_id));
        if response.lost_focus() {
            if let Ok(bits) = u64::from_str_radix(text.trim().trim_start_matches("0x"), 16) {
                changed |= bits != flags.internal_flags;
                flags.internal_flags = bits;
            }
        }
    }
    let flag_columns = ((ui.available_width() / FLAG_CELL_WIDTH) as usize).max(1);
    egui::Grid::new(("tiledata_flag_bits", art_id)).num_columns(flag_columns).show(ui, |ui| {
        for (column, (bit, name)) in FLAG_BITS.into_iter().enumerate() {
            let mask = 1u64 << bit;
            let mut set = flags.internal_flags & mask != 0;
            if ui.checkbox(&mut set, name).changed() {
                flags.internal_flags = if set { flags.internal_flags | mask } else { flags.internal_flags & !mask };
                changed = true;
            }
            if (column + 1) % flag_columns == 0 {
                ui.end_row();
            }
        }
    });
    changed
}

/// Stores a tile's new contents in the loaded tiles (a copy of what the rest of the app holds) and in its table row.
pub(super) fn apply_change(app: &mut UopInspectorApp, change: TileChange) {
    let (Some(tiledata), Some(rows)) = (app.cc_tiledata.as_mut(), app.cc_tiledata_rows.as_mut()) else {
        return;
    };
    app.tiledata_editor.opened.get_or_insert_with(|| Arc::clone(tiledata));
    let tiledata = Arc::make_mut(tiledata);
    let rows = Arc::make_mut(rows);
    let land_count = tiledata.land_tiles().len();
    match change {
        TileChange::Land(index, tile) => {
            rows[index] = cc_land_row(&tile);
            tiledata.land_tiles_mut()[index] = tile;
        }
        TileChange::Item(index, tile) => {
            rows[land_count + index] = cc_item_row(&tile);
            tiledata.item_tiles_mut()[index] = tile;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::classic_tile_flags_summary;
    use crate::app::tests::test_app;

    /// A tiledata file of this layout: zeroed tiles in the older record sizes (26-byte land, 37-byte item).
    const FILE_BYTES: usize = 512 * (4 + 32 * 26) + 512 * (4 + 32 * 37);
    const EDITED_TILE: usize = 3;
    const EDITED_WEIGHT: u8 = 7;

    /// An app with a tiledata whose item tile `EDITED_TILE` was changed through the panel's path.
    fn app_with_an_edited_tile() -> UopInspectorApp {
        let path = std::env::temp_dir().join(format!("inspector_tiledata_editor_{}.mul", std::process::id()));
        std::fs::write(&path, vec![0u8; FILE_BYTES]).unwrap();
        let tiledata = TileData::load(path.clone()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut app = test_app();
        app.cc_tiledata_rows = Some(Arc::new(crate::app::collect_cc_tiledata_rows(&tiledata)));
        app.cc_tiledata = Some(Arc::new(tiledata));
        let mut tile = app.cc_tiledata.as_ref().unwrap().item_tiles()[EDITED_TILE].clone();
        tile.weight = EDITED_WEIGHT;
        apply_change(&mut app, TileChange::Item(EDITED_TILE, tile));
        app
    }

    #[test]
    fn an_edit_reaches_the_tile_and_its_row_and_discarding_takes_both_back() {
        let weight = |app: &UopInspectorApp| app.cc_tiledata.as_ref().unwrap().item_tiles()[EDITED_TILE].weight;
        let row_weight = |app: &UopInspectorApp| {
            let rows = app.cc_tiledata_rows.as_ref().unwrap();
            rows[app.cc_tiledata.as_ref().unwrap().land_tiles().len() + EDITED_TILE].weight.clone()
        };
        let mut app = app_with_an_edited_tile();
        assert!((TAB_EDITOR.is_dirty)(&app));
        assert_eq!(weight(&app), EDITED_WEIGHT);
        assert_eq!(row_weight(&app), EDITED_WEIGHT.to_string());

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(weight(&app), 0);
        assert_eq!(row_weight(&app), "0");
    }

    #[test]
    fn the_flag_names_match_the_summary_names() {
        for (bit, name) in FLAG_BITS {
            assert_eq!(classic_tile_flags_summary(Flags { internal_flags: 1 << bit }), name);
        }
    }

    #[test]
    fn a_new_name_zeroes_the_rest_of_the_field() {
        let mut name = [b'x'; 8];
        store_name(&mut name, "ab");
        assert_eq!(name, [b'a', b'b', 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
    fn a_shipped_tiledata_saves_the_same_bytes_and_an_edit_changes_only_its_field() {
        const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
        /// Any item tile.
        const ITEM_INDEX: usize = 0x1000;
        let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
        for root in std::env::split_paths(&roots) {
            let Some(path) = find_file_ignoring_case(&root, TILEDATA_FILE_NAME) else {
                println!("{}: no {TILEDATA_FILE_NAME}", root.display());
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            let mut tiledata = TileData::load(path).unwrap();
            assert_eq!(tiledata.to_bytes().unwrap(), bytes, "{}", root.display());

            let tile = &mut tiledata.item_tiles_mut()[ITEM_INDEX];
            tile.weight = tile.weight.wrapping_add(1);
            let edited = tiledata.to_bytes().unwrap();
            assert_eq!(edited.len(), bytes.len());
            let differing = edited.iter().zip(&bytes).filter(|(left, right)| left != right).count();
            assert_eq!(differing, 1, "{}", root.display());
            println!("{}: same bytes; one edit changes one byte", root.display());
        }
    }
}
