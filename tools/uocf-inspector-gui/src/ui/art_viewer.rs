use eframe::egui;
use egui_extras::{Column, TableBuilder};
use crate::app::{classic_tile_flags_summary, concrete_art_tile_source, ArtSource, ArtViewerRow, TileArtFileEntry, TileMetadataSource, UopInspectorApp, ViewMode};
use crate::logic::ClientData;
use super::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by, tileart_editor, tiledata_editor};
use super::classic_image_import::{ui_classic_image_panel, PreferredContainer};
use uocf::classic::image_files::ClassicImageKind;
use std::sync::Arc;
use uocf::enhanced::tileart::{TaeAppearanceFilter, TaeImgOffset, TextureItem, TextureType, TileArtEntry};
use uocf::classic::art::{static_art_id_for_source, STATIC_TILE_ID_BASE};

const TILEDATA_TABLE_MIN_WIDTH: f32 = 980.0;
const TILEART_TABLE_MIN_WIDTH: f32 = 1450.0;
/// The property lists are long: the column starts narrower than its text, which shows in full on hover.
const TILEART_PROPERTIES_WIDTH: f32 = 320.0;
/// Starting height of a table above the selected record's details; the user can drag it.
const METADATA_TABLE_HEIGHT: f32 = 300.0;
/// A table may be dragged this low and still keep its header and a row.
const METADATA_TABLE_MIN_SCROLLED_HEIGHT: f32 = 40.0;
/// Largest side of a texture in the Textures section.
const TEXTURE_PREVIEW_SIDE: f32 = 256.0;
/// From this id on UOETE draws no bounding box and a smaller image: these are a record's noise and other extra textures,
/// not the art its windows cut.
const FIRST_EXTRA_TEXTURE_ID: u32 = 1_000_000;
const EXTRA_TEXTURE_PREVIEW_SIDE: f32 = 128.0;
/// Small textures are enlarged up to this, so that the bounding box is easy to read.
const TEXTURE_PREVIEW_MAX_SCALE: f32 = 3.0;
const BOUNDING_BOX_COLOR: egui::Color32 = egui::Color32::RED;
const BOUNDING_BOX_STROKE_WIDTH: f32 = 1.0;
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const ART_VIEWER_LIST_PANEL_WIDTH: f32 = 300.0;

pub fn ui_art_viewer(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);

    egui::Panel::left("tex_art_cc_list")
        .resizable(true)
        .default_size(ART_VIEWER_LIST_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading("Art & TileData");

            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("art_source_combo")
                    .selected_text(art_source_label(app.selected_legacy_source))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Mul, "Legacy MUL (.mul)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP (artLegacyMUL)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopLegacy, "EC Legacy UOP (LegacyTexture)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUopKr, "KR/New UOP (Texture)");
                    });
            });

            let mut art_search_has_focus = false;
            ui.horizontal(|ui| {
                ui.label("Search:");
                let response = ui.text_edit_singleline(&mut app.searches.art);
                art_search_has_focus |= response.has_focus();
            });

            ui.separator();
            ui.heading("Hue Options");
            ui.horizontal(|ui| {
                ui.label("Selected Hue:");
                ui.add(egui::DragValue::new(&mut app.selected_hue_id));
            });

            if let Some(client) = &app.client_data {
                if let Some(hues) = &client.hues {
                    if app.selected_hue_id > 0 && (app.selected_hue_id as usize) <= hues.len() {
                        let hue = &hues[app.selected_hue_id as usize - 1];
                        let name = std::str::from_utf8(&hue.name).unwrap_or("").trim_matches('\0');
                        ui.label(format!("Name: {}", name));
                    }
                }
            }
            ui.separator();

            ensure_art_viewer_rows(app);
            if let Some(rows) = app.art_viewer_rows.clone() {
                let query = app.searches.art.trim();
                let sort = list_sort_controls(ui, "art_viewer_rows", &["ID", "Name", "Group"], 0);
                let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
                    match sort.option_index {
                        1 => left.label.to_ascii_lowercase().cmp(&right.label.to_ascii_lowercase()),
                        2 => left.group.cmp(right.group).then_with(|| left.art_id.cmp(&right.art_id)),
                        _ => left.art_id.cmp(&right.art_id),
                    }
                });
                let filtered_indices = filtered_metadata_indices(&rows, Some(&sorted_indices), query, |row| &row.search_text);
                let visible_ids = filtered_indices
                    .iter()
                    .map(|index| rows[*index].art_id)
                    .collect::<Vec<_>>();
                let keyboard_moved = if let Some(delta) = arrow_delta(ui, art_search_has_focus) {
                    app.selected_tex_art_cc_id = move_selection(&visible_ids, app.selected_tex_art_cc_id, delta);
                    if app.selected_tex_art_cc_id.is_some() {
                        app.view_mode = ViewMode::TexArtCc;
                    }
                    true
                } else {
                    false
                };

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui_art_group_rows(app, ui, &rows, &filtered_indices, "Land Tiles", keyboard_moved);
                    ui_art_group_rows(app, ui, &rows, &filtered_indices, "Static Tiles", keyboard_moved);
                });
            }
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if app.view_mode == ViewMode::CcTileData {
            ui_cc_tiledata(app, ctx, ui);
        } else if let Some(id) = app.selected_tex_art_cc_id {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.view_mode, ViewMode::TexArtCc, "Specialized");
                if ui.button("Raw UOP").clicked() {
                    app.select_raw_art_entry(id, concrete_art_tile_source(app.selected_legacy_source));
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(format!("Art ID: {}", id));
                    if let Some(handle) = app.get_tex_art_cc_texture(ctx, id) {
                        ui.label(format!("Dimensions: {}x{}", handle.size()[0], handle.size()[1]));
                        ui.image(&handle);
                    } else {
                        ui.colored_label(egui::Color32::RED, "Art data not found");
                    }
                    // The Classic sources: art ids as the Classic files number them.
                    let preferred = match concrete_art_tile_source(app.selected_legacy_source) {
                        ArtSource::Mul => Some(PreferredContainer::Mul),
                        ArtSource::CcUop => Some(PreferredContainer::Uop),
                        _ => None,
                    };
                    if let Some(preferred) = preferred {
                        ui.separator();
                        ui_classic_image_panel(app, ctx, ui, ClassicImageKind::Art, id, preferred);
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.heading("TileData Properties");
                    if let Some(client) = &app.client_data {
                        let source = concrete_art_tile_source(app.selected_legacy_source);
                        if id < STATIC_TILE_ID_BASE && !source.is_ec_uop() {
                            if let Some(tile) = client.tiledata.land_tiles().get(id as usize) {
                                egui::Grid::new("land_tile_grid").striped(true).show(ui, |ui| {
                                    ui.label("Name"); ui.label(tile.name_ascii()); ui.end_row();
                                    ui.label("Flags"); ui.label(format!("{:016X}", tile.flags.internal_flags)); ui.end_row();
                                    ui.label("Texture ID"); ui.label(tile.texture_id.to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(classic_tile_flags_summary(tile.flags));
                            }
                        } else {
                            let item_id = if source.is_ec_uop() {
                                id
                            } else {
                                id - STATIC_TILE_ID_BASE
                            };
                            if let Some(tile) = client.tiledata.item_tiles().get(item_id as usize) {
                                egui::Grid::new("item_tile_grid").striped(true).show(ui, |ui| {
                                    ui.label("Name"); ui.label(tile.name_ascii()); ui.end_row();
                                    ui.label("Flags"); ui.label(format!("{:016X}", tile.flags.internal_flags)); ui.end_row();
                                    ui.label("Weight"); ui.label(tile.weight.to_string()); ui.end_row();
                                    ui.label("Layer/LightID"); ui.label(tile.quality.to_string()); ui.end_row();
                                    ui.label("Count/Quantity"); ui.label(tile.quantity.to_string()); ui.end_row();
                                    ui.label("Anim ID"); ui.label(tile.anim_id.to_string()); ui.end_row();
                                    ui.label("Hue Extra"); ui.label(tile.hue_extra.to_string()); ui.end_row();
                                    ui.label("Height"); ui.label(tile.height_raw().to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(classic_tile_flags_summary(tile.flags));
                            }
                        }
                    }
                });
            });
        } else {
            ui.centered_and_justified(|ui| { ui.label("Select a tile from the sidebar to inspect"); });
        }
    });
}

fn art_row_should_show(source: ArtSource, has_art: bool, has_metadata: bool) -> bool {
    match source {
        ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => has_art,
        ArtSource::Mul | ArtSource::Any => has_art || has_metadata,
    }
}

fn ensure_art_viewer_rows(app: &mut UopInspectorApp) {
    let source = concrete_art_tile_source(app.selected_legacy_source);
    if app.art_viewer_rows_source == Some(source) && app.art_viewer_rows.is_some() {
        return;
    }

    let rows = app
        .client_data
        .as_ref()
        .map(|client| Arc::new(collect_art_viewer_rows(client, source)));
    app.art_viewer_rows = rows;
    app.art_viewer_rows_source = Some(source);
}

fn collect_art_viewer_rows(client: &ClientData, source: ArtSource) -> Vec<ArtViewerRow> {
    let mut rows = Vec::new();

    if !source.is_ec_uop() {
        for id in 0..STATIC_TILE_ID_BASE {
            let has_art = client.art.has_id_from_source(id, source);
            let has_metadata = client.tiledata.land_tiles().get(id as usize).is_some();
            if !art_row_should_show(source, has_art, has_metadata) {
                continue;
            }

            let tile_name = client
                .tiledata
                .land_tiles()
                .get(id as usize)
                .map(|tile| tile.name_ascii())
                .unwrap_or_default();
            rows.push(art_viewer_row(id, "Land Tiles", tile_name));
        }
    }

    let source_max_id = client.art.max_id_for_source(source);
    let max_item_id = if source.is_ec_uop() {
        source_max_id.max(32768)
    } else {
        source_max_id
            .max(STATIC_TILE_ID_BASE + 32768)
            .saturating_sub(STATIC_TILE_ID_BASE)
    };
    for item_id in 0..max_item_id {
        let id = static_art_id_for_source(item_id, source);
        let has_art = client.art.has_id_from_source(id, source);
        let has_metadata = client.tiledata.item_tiles().get(item_id as usize).is_some();
        if !art_row_should_show(source, has_art, has_metadata) {
            continue;
        }

        let tile_name = client
            .tiledata
            .item_tiles()
            .get(item_id as usize)
            .map(|tile| tile.name_ascii())
            .unwrap_or_default();
        rows.push(art_viewer_row(id, "Static Tiles", tile_name));
    }

    rows
}

fn art_viewer_row(art_id: u32, group: &'static str, tile_name: &str) -> ArtViewerRow {
    let label = format!("{}: {}", art_id, tile_name);
    ArtViewerRow {
        art_id,
        label,
        group,
        search_text: format!("{} {} {}", art_id, group.to_lowercase(), tile_name.to_lowercase()),
    }
}

fn ui_art_group_rows(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    rows: &[ArtViewerRow],
    filtered_indices: &[usize],
    group: &'static str,
    keyboard_moved: bool,
) {
    let has_rows = filtered_indices.iter().any(|index| rows[*index].group == group);
    if !has_rows {
        return;
    }

    ui.collapsing(group, |ui| {
        for index in filtered_indices.iter().copied().filter(|index| rows[*index].group == group) {
            ui_art_row(app, ui, &rows[index], keyboard_moved);
        }
    });
}

fn ui_art_row(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    row: &ArtViewerRow,
    keyboard_moved: bool,
) {
    let selected = app.selected_tex_art_cc_id == Some(row.art_id);
    let response = ui.selectable_label(selected, &row.label);
    if response.clicked() {
        app.selected_tex_art_cc_id = Some(row.art_id);
        app.view_mode = ViewMode::TexArtCc;
    }
    if keyboard_moved && selected {
        response.scroll_to_me(Some(egui::Align::Center));
    }
}

fn art_source_label(source: ArtSource) -> &'static str {
    match source {
        ArtSource::Any => "CC UOP (artLegacyMUL)",
        ArtSource::Mul => "Legacy MUL (.mul)",
        ArtSource::CcUop => "CC UOP (artLegacyMUL)",
        ArtSource::EcUop | ArtSource::EcUopLegacy => "EC Legacy UOP (LegacyTexture)",
        ArtSource::EcUopKr => "KR/New UOP (Texture)",
    }
}

fn ui_cc_tiledata(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui_tile_metadata_contents(app, ctx, ui);
}

pub fn ui_tile_metadata(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui_tile_metadata_contents(app, ctx, ui);
    });
}

fn ui_tile_metadata_contents(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Tile Metadata");
        ui.separator();
        ui.selectable_value(
            &mut app.tile_metadata_source,
            TileMetadataSource::CcTileData,
            "tiledata.mul",
        );
        ui.selectable_value(
            &mut app.tile_metadata_source,
            TileMetadataSource::EcTileArt,
            "tileart.uop",
        );
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

    let mut text_has_focus = false;
    ui.horizontal(|ui| {
        ui.label("Search:");
        let response = ui.text_edit_singleline(&mut app.searches.tile_metadata);
        text_has_focus |= response.has_focus();
    });
    ui.separator();

    match app.tile_metadata_source {
        TileMetadataSource::CcTileData => ui_cc_tiledata_table(app, ctx, ui, text_has_focus),
        TileMetadataSource::EcTileArt => ui_ec_tileart_table(app, ctx, ui, text_has_focus),
    }
}

fn ui_cc_tiledata_table(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, text_has_focus: bool) {
    ui.heading("CC TileData Inspector");
    if app.cc_tiledata.is_some() {
        ui.horizontal_wrapped(|ui| tiledata_editor::toolbar(app, ui));
    }
    let mut change = None;
    if let (Some(tiledata), Some(rows)) = (app.cc_tiledata.clone(), app.cc_tiledata_rows.clone()) {
        let query = app.searches.tile_metadata.trim();
        let sort = list_sort_controls(
            ui,
            "cc_tiledata_rows",
            &["ID", "Name", "Type", "Texture", "Height", "Weight", "Anim", "Value"],
            0,
        );
        let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
            match sort.option_index {
                1 => left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()),
                2 => left.kind.cmp(right.kind).then_with(|| left.art_id.cmp(&right.art_id)),
                3 => string_number(&left.texture_id).cmp(&string_number(&right.texture_id)),
                4 => string_number(&left.height).cmp(&string_number(&right.height)),
                5 => string_number(&left.weight).cmp(&string_number(&right.weight)),
                6 => string_number(&left.anim_id).cmp(&string_number(&right.anim_id)),
                7 => string_number(&left.value).cmp(&string_number(&right.value)),
                _ => left.art_id.cmp(&right.art_id),
            }
        });
        let filtered_indices = filtered_metadata_indices(&rows, Some(&sorted_indices), query, |row| &row.search_text);
        let row_count = filtered_indices.len();
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
        let visible_ids = filtered_indices
            .iter()
            .map(|index| rows[*index].art_id)
            .collect::<Vec<_>>();
        let keyboard_moved = if let Some(delta) = arrow_delta(ui, text_has_focus) {
            app.selected_tex_art_cc_id = move_selection(&visible_ids, app.selected_tex_art_cc_id, delta);
            true
        } else {
            false
        };
        ui.label(format!(
            "{} land tiles, {} item tiles",
            tiledata.land_tiles().len(),
            tiledata.item_tiles().len()
        ));
        egui::Panel::top("tiledata_table").resizable(true).default_size(METADATA_TABLE_HEIGHT).show(ui, |ui| egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_width(TILEDATA_TABLE_MIN_WIDTH);
            TableBuilder::new(ui)
                .auto_shrink([false, false])
                .min_scrolled_height(METADATA_TABLE_MIN_SCROLLED_HEIGHT)
                .max_scroll_height(f32::INFINITY)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::remainder().at_least(220.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("ID");
                    });
                    header.col(|ui| {
                        ui.strong("Type");
                    });
                    header.col(|ui| {
                        ui.strong("Name");
                    });
                    header.col(|ui| {
                        ui.strong("Texture");
                    });
                    header.col(|ui| {
                        ui.strong("Height");
                    });
                    header.col(|ui| {
                        ui.strong("Weight");
                    });
                    header.col(|ui| {
                        ui.strong("Layer/Light");
                    });
                    header.col(|ui| {
                        ui.strong("Qty");
                    });
                    header.col(|ui| {
                        ui.strong("Anim");
                    });
                    header.col(|ui| {
                        ui.strong("Value");
                    });
                    header.col(|ui| {
                        ui.strong("Flags");
                    });
                })
                .body(|body| {
                    body.rows(text_height, row_count, |mut row| {
                        let idx = filtered_indices[row.index()];
                        let item = &rows[idx];
                        row.col(|ui| {
                            let selected = app.selected_tex_art_cc_id == Some(item.art_id);
                            if ui
                                .selectable_label(selected, &item.id)
                                .clicked()
                            {
                                app.selected_tex_art_cc_id = Some(item.art_id);
                            }
                            if keyboard_moved && selected {
                                ui.scroll_to_cursor(Some(egui::Align::Center));
                            }
                        });
                        row.col(|ui| {
                            ui.label(item.kind);
                        });
                        row.col(|ui| {
                            ui.label(&item.name);
                        });
                        row.col(|ui| {
                            ui.label(&item.texture_id);
                        });
                        row.col(|ui| {
                            ui.label(&item.height);
                        });
                        row.col(|ui| {
                            ui.label(&item.weight);
                        });
                        row.col(|ui| {
                            ui.label(&item.quality);
                        });
                        row.col(|ui| {
                            ui.label(&item.quantity);
                        });
                        row.col(|ui| {
                            ui.label(&item.anim_id);
                        });
                        row.col(|ui| {
                            ui.label(&item.value);
                        });
                        row.col(|ui| {
                            ui.label(&item.flags_summary).on_hover_text(&item.flags_raw);
                        });
                    });
                });
        }));

        change = egui::CentralPanel::default().show(ui, |ui| ui_selected_tiledata_details(app, ctx, ui, &tiledata)).inner;
    } else {
        ui.label("Select a Classic Client path containing tiledata.mul.");
    }
    // The change goes into the loaded tiles, which must not be shared then (the copies above are dropped).
    if let Some(change) = change {
        tiledata_editor::apply_change(app, change);
    }
}

fn ui_ec_tileart_table(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, text_has_focus: bool) {
    ui.heading("EC TileArt Inspector");
    let mut edits = Vec::new();
    if app.ec_tileart_entries.is_some() {
        ui.horizontal_wrapped(|ui| tileart_editor::toolbar(app, ctx, ui, &mut edits));
    }
    if let (Some(entries), Some(rows)) = (app.ec_tileart_entries.clone(), app.ec_tileart_rows.clone()) {
        let query = app.searches.tile_metadata.trim();
        let sort = list_sort_controls(
            ui,
            "ec_tileart_rows",
            &["ID", "Old ID", "Body Type", "Hash"],
            0,
        );
        let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
            match sort.option_index {
                1 => string_number(&left.old_id).cmp(&string_number(&right.old_id)),
                2 => string_number(&left.body_type).cmp(&string_number(&right.body_type)).then_with(|| string_number(&left.tile_id).cmp(&string_number(&right.tile_id))),
                3 => left.filename_hash.cmp(&right.filename_hash),
                _ => string_number(&left.tile_id).cmp(&string_number(&right.tile_id)),
            }
        });
        let filtered_indices = filtered_metadata_indices(&rows, Some(&sorted_indices), query, |row| &row.search_text);
        let row_count = filtered_indices.len();
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
        let visible_hashes = filtered_indices
            .iter()
            .map(|index| rows[*index].filename_hash)
            .collect::<Vec<_>>();
        let keyboard_moved = if let Some(delta) = arrow_delta(ui, text_has_focus) {
            app.selected_tileart_hash = move_selection(&visible_hashes, app.selected_tileart_hash, delta);
            true
        } else {
            false
        };
        ui.label(format!("{} tileart.uop entries", entries.len()));
        egui::Panel::top("tileart_table").resizable(true).default_size(METADATA_TABLE_HEIGHT).show(ui, |ui| egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_width(TILEART_TABLE_MIN_WIDTH);
            TableBuilder::new(ui)
                .auto_shrink([false, false])
                .min_scrolled_height(METADATA_TABLE_MIN_SCROLLED_HEIGHT)
                .max_scroll_height(f32::INFINITY)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::initial(TILEART_PROPERTIES_WIDTH).at_least(180.0).clip(true))
                .column(Column::auto().at_least(220.0))
                .column(Column::auto().at_least(120.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(120.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(90.0))
                .column(Column::auto().at_least(170.0))
                .column(Column::remainder().at_least(180.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("ID");
                    });
                    header.col(|ui| {
                        ui.strong("Old ID");
                    });
                    header.col(|ui| {
                        ui.strong("Body Type");
                    });
                    header.col(|ui| {
                        ui.strong("Properties");
                    });
                    header.col(|ui| {
                        ui.strong("Flags");
                    });
                    header.col(|ui| {
                        ui.strong("EC Window");
                    });
                    header.col(|ui| {
                        ui.strong("EC Offset");
                    });
                    header.col(|ui| {
                        ui.strong("CC Window");
                    });
                    header.col(|ui| {
                        ui.strong("CC Offset");
                    });
                    header.col(|ui| {
                        ui.strong("Sitting");
                    });
                    header.col(|ui| {
                        ui.strong("Appearance");
                    });
                    header.col(|ui| {
                        ui.strong("Textures");
                    });
                })
                .body(|body| {
                    body.rows(text_height, row_count, |mut row| {
                        let idx = filtered_indices[row.index()];
                        let item = &rows[idx];
                        row.col(|ui| {
                            let selected = app.selected_tileart_hash == Some(item.filename_hash);
                            if ui
                                .selectable_label(
                                    selected,
                                    &item.tile_id,
                                )
                                .clicked()
                            {
                                app.selected_tileart_hash = Some(item.filename_hash);
                            }
                            if keyboard_moved && selected {
                                ui.scroll_to_cursor(Some(egui::Align::Center));
                            }
                        });
                        row.col(|ui| {
                            ui.label(&item.old_id);
                        });
                        row.col(|ui| {
                            ui.label(&item.body_type);
                        });
                        row.col(|ui| {
                            ui.label(&item.properties_summary).on_hover_text(&item.properties_summary);
                        });
                        row.col(|ui| {
                            ui.label(&item.flags_summary).on_hover_text(&item.flags_raw);
                        });
                        row.col(|ui| {
                            ui.label(&item.ec_window);
                        });
                        row.col(|ui| {
                            ui.label(&item.ec_offset);
                        });
                        row.col(|ui| {
                            ui.label(&item.cc_window);
                        });
                        row.col(|ui| {
                            ui.label(&item.cc_offset);
                        });
                        row.col(|ui| {
                            ui.label(&item.sitting_summary);
                        });
                        row.col(|ui| {
                            ui.label(&item.appearance_summary);
                        });
                        row.col(|ui| {
                            ui.label(&item.texture_summary);
                        });
                    });
                });
        }));

        egui::CentralPanel::default().show(ui, |ui| {
            let selected = app.selected_tileart_hash.and_then(|hash| entries.iter().find(|file| file.filename_hash == hash));
            let Some(file) = selected else {
                return;
            };
            egui::ScrollArea::vertical().id_salt("tileart_record").auto_shrink([false, false]).show(ui, |ui| {
                ui.heading(format!("Selected TileArt {}", file.entry.tile_id));
                let dictionary = app.uo_string_dictionary.clone();
                let name = dictionary.as_deref().and_then(|dictionary| dictionary.get_string((file.entry.string_dict_off as usize).checked_sub(1)?));
                tileart_editor::record_panel(ui, file, name, app.tileart_editor.enabled, &mut edits);
                ui_selected_tileart_details(app, ctx, ui, file);
            });
        });
    } else {
        ui.label("Select an Enhanced Client path containing tileart.uop.");
    }
    // The edits go into the loaded records, which must not be shared then (the table's copies are dropped).
    tileart_editor::apply_edits(app, edits);
}

fn ui_selected_tiledata_details(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, tiledata: &uocf::classic::tiledata::TileData) -> Option<tiledata_editor::TileChange> {
    let selected_id = app.selected_tex_art_cc_id?;
    let rows = app.cc_tiledata_rows.clone()?;
    let row = rows.iter().find(|row| row.art_id == selected_id)?;
    let source = concrete_art_tile_source(app.selected_legacy_source);
    let art = app.get_tex_art_texture_from_source(ctx, selected_id, source);
    let editable = app.tiledata_editor.enabled;
    egui::ScrollArea::vertical().id_salt("tiledata_record").auto_shrink([false, false]).show(ui, |ui| {
        ui.heading(format!("Selected TileData {} {}", row.kind, row.id));
        tiledata_editor::record_panel(ui, tiledata, selected_id, art.as_ref(), editable)
    }).inner
}

/// What the record panel does not draw: the appearance filters (read only) and, last, the textures.
fn ui_selected_tileart_details(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &TileArtFileEntry) {
    let entry = &file.entry;

    ui.collapsing(format!("Appearance: {} (read only)", tileart_appearance_summary(&entry.appearance_vector)), |ui| {
        egui::Grid::new("tileart_selected_details").striped(true).show(ui, |ui| {
            for (index, appearance) in entry.appearance_vector.iter().enumerate() {
                ui.label(format!("Appearance {}", index));
                ui.label(tileart_appearance_detail(appearance));
                ui.end_row();
            }
        });
    });

    ui.collapsing("Texture blocks (read only)", |ui| {
        egui::Grid::new("tileart_texture_block_details").striped(true).show(ui, |ui| {
            ui.label("Block");
            ui.label("Has");
            ui.label("Type Offset");
            ui.label("Items");
            ui.label("Vectors");
            ui.end_row();
            for (block_index, block) in entry.texture_vector.iter().enumerate() {
                ui.label(block_index.to_string());
                ui.label(block.has_texture.to_string());
                ui.label(block.type_string_off.to_string());
                ui.label(block.texture_items_count.to_string());
                ui.label(format!(
                    "technique {} / tweaks {}",
                    block.technique_count,
                    block.tweak_count
                ));
                ui.end_row();
                for (item_index, item) in block.texture_items.iter().enumerate() {
                    ui.label(format!("{}.{}", block_index, item_index));
                    ui.label(format!("name off {}", item.name_string_off));
                    ui.label(format!("stretch {}", item.texture_stretch));
                    ui.label(format!("source_kind {}", item.source_kind));
                    ui.label(format!("sampler {} / uv_set {}", item.sampler_index, item.uv_set));
                    ui.end_row();
                }
            }
        });
    });

    ui.separator();
    ui.strong("Textures");
    let Some(dictionary) = app.uo_string_dictionary.clone() else {
        ui.label("The texture names are in the string dictionary, which is not loaded.");
        return;
    };
    for (row, block) in entry.process(&dictionary).texture_items.into_iter().enumerate() {
        egui::ScrollArea::horizontal().id_salt(("tileart_texture_row", row)).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for item in &block {
                    ui.vertical(|ui| ui_tileart_texture(app, ctx, ui, entry, item));
                }
            });
        });
    }
}

/// The window a texture's art is cut with, as UOETE picks it: legacy art uses the CC window, other art the EC window,
/// and the noise and mask textures have none.
fn tileart_texture_window<'a>(entry: &'a TileArtEntry, item: &TextureItem) -> Option<(&'static str, &'a TaeImgOffset)> {
    if item.id >= FIRST_EXTRA_TEXTURE_ID {
        return None;
    }
    match item.texture_type {
        TextureType::TileArtLegacy => Some(("CC window", &entry.cc_img_offset)),
        TextureType::WorldArt | TextureType::TileArtEnhanced => Some(("EC window", &entry.ec_img_offset)),
        TextureType::Textures | TextureType::Undefined => None,
    }
}

/// Whether a window lies inside an image of `size`.
fn window_fits(window: &TaeImgOffset, size: [usize; 2]) -> bool {
    window.x_start >= 0
        && window.y_start >= 0
        && window.x_end >= window.x_start
        && window.y_end >= window.y_start
        && window.x_end as usize <= size[0]
        && window.y_end as usize <= size[1]
}

/// One texture of a record, with its bounding box drawn when it has a window.
fn ui_tileart_texture(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, entry: &TileArtEntry, item: &TextureItem) {
    ui.label(format!("{} {}", tileart_texture_type_label(item.texture_type), item.id)).on_hover_text(&item.path);
    ui.weak(format!("block {}, item {}, stretch {}", item.block_index, item.item_index, item.texture_stretch));
    let window = tileart_texture_window(entry, item);
    let Some(handle) = app.tileart_item_texture(ctx, item.texture_type, item.id) else {
        ui.colored_label(WARNING_COLOR, "missing");
        return;
    };
    let size = handle.size_vec2();
    let side = if item.id >= FIRST_EXTRA_TEXTURE_ID { EXTRA_TEXTURE_PREVIEW_SIDE } else { TEXTURE_PREVIEW_SIDE };
    let scale = (side / size.max_elem()).min(TEXTURE_PREVIEW_MAX_SCALE);
    let image = ui.add(egui::Image::new(&handle).fit_to_exact_size(size * scale).texture_options(egui::TextureOptions::NEAREST));
    ui.weak(format!("{} x {}", size.x, size.y));
    let Some((label, window)) = window else {
        return;
    };
    let corner = |x: i32, y: i32| image.rect.min + egui::vec2(x as f32, y as f32) * scale;
    let bounds = egui::Rect::from_two_pos(corner(window.x_start, window.y_start), corner(window.x_end, window.y_end));
    ui.painter().rect_stroke(bounds, 0.0, egui::Stroke::new(BOUNDING_BOX_STROKE_WIDTH, BOUNDING_BOX_COLOR), egui::StrokeKind::Outside);
    ui.label(format!("Box: {label}"));
    if !window_fits(window, handle.size()) {
        ui.colored_label(WARNING_COLOR, "The window is not inside this image.");
    }
}

fn filtered_metadata_indices<T>(
    rows: &[T],
    sorted_indices: Option<&[usize]>,
    query: &str,
    search_text: impl Fn(&T) -> &str,
) -> Vec<usize> {
    let source_indices: Vec<usize> = sorted_indices
        .map(|indices| indices.to_vec())
        .unwrap_or_else(|| (0..rows.len()).collect());
    if query.is_empty() {
        return source_indices;
    }

    let query = query.to_lowercase();
    source_indices
        .into_iter()
        .filter(|index| search_text(&rows[*index]).contains(&query))
        .collect()
}

fn string_number(value: &str) -> u32 {
    value.parse().unwrap_or(u32::MAX)
}

fn tileart_appearance_summary(appearance: &[TaeAppearanceFilter]) -> String {
    if appearance.is_empty() {
        return "none".to_string();
    }

    let mut by_body = 0usize;
    let mut by_gender = 0usize;
    let mut animation_by_body = 0usize;
    for item in appearance {
        match item {
            TaeAppearanceFilter::ByOwnerBodyType(_) => by_body += 1,
            TaeAppearanceFilter::ByOwnerGender { .. } => by_gender += 1,
            TaeAppearanceFilter::AnimationByOwnerBodyType(_) => animation_by_body += 1,
        }
    }

    format!(
        "{} filters (body {}, gender {}, animation body {})",
        appearance.len(),
        by_body,
        by_gender,
        animation_by_body
    )
}

fn tileart_texture_type_label(texture_type: TextureType) -> &'static str {
    match texture_type {
        TextureType::Undefined => "Undefined",
        TextureType::WorldArt => "WorldArt",
        TextureType::TileArtLegacy => "TileArtLegacy",
        TextureType::TileArtEnhanced => "TileArtEnhanced",
        TextureType::Textures => "Textures",
    }
}

fn tileart_appearance_detail(appearance: &TaeAppearanceFilter) -> String {
    let (label, mappings) = match appearance {
        TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
            return format!("gender {}: appearance {}", gender, appearance);
        }
        TaeAppearanceFilter::ByOwnerBodyType(mappings) => ("body", mappings),
        TaeAppearanceFilter::AnimationByOwnerBodyType(mappings) => ("animation body", mappings),
    };
    let pairs = mappings
        .iter()
        .map(|mapping| format!("{}:{}", mapping.body_type, mapping.appearance))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}: {} records [{}]", label, mappings.len(), pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_SIZE: [usize; 2] = [64, 64];

    fn window_label(texture_type: TextureType, id: u32) -> Option<&'static str> {
        let item = TextureItem { texture_type, id, ..TextureItem::default() };
        tileart_texture_window(&TileArtEntry::default(), &item).map(|(label, _)| label)
    }

    #[test]
    fn a_texture_takes_the_window_of_its_family_and_extra_textures_have_none() {
        assert_eq!(window_label(TextureType::WorldArt, 5118), Some("EC window"));
        assert_eq!(window_label(TextureType::TileArtEnhanced, 5118), Some("EC window"));
        assert_eq!(window_label(TextureType::TileArtLegacy, 5118), Some("CC window"));
        assert_eq!(window_label(TextureType::Textures, 5118), None);
        assert_eq!(window_label(TextureType::WorldArt, FIRST_EXTRA_TEXTURE_ID - 1), Some("EC window"));
        assert_eq!(window_label(TextureType::WorldArt, FIRST_EXTRA_TEXTURE_ID), None);
    }

    #[test]
    fn a_window_outside_its_image_does_not_fit() {
        let window = |x_start, y_start, x_end, y_end| TaeImgOffset { x_start, y_start, x_end, y_end, ..TaeImgOffset::default() };
        assert!(window_fits(&window(0, 0, 64, 64), IMAGE_SIZE));
        assert!(!window_fits(&window(0, 0, 65, 64), IMAGE_SIZE));
        assert!(!window_fits(&window(-1, 0, 64, 64), IMAGE_SIZE));
        assert!(!window_fits(&window(10, 0, 5, 64), IMAGE_SIZE));
    }
}
