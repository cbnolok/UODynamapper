use eframe::egui;
use egui_extras::{Column, TableBuilder};
use crate::app::{concrete_art_tile_source, tileart_flags_summary, tileart_property_name, tileart_type_name, ArtSource, ArtViewerRow, TileArtFileEntry, TileMetadataSource, UopInspectorApp, ViewMode};
use crate::logic::ClientData;
use super::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by};
use std::sync::Arc;
use uocf::enhanced::tileart::{TaeAnimationAppearance, TaeSittingAnimation};
use uocf::classic::art::{static_art_id_for_source, STATIC_TILE_ID_BASE};

const TILEDATA_TABLE_MIN_WIDTH: f32 = 980.0;
const TILEART_TABLE_MIN_WIDTH: f32 = 1450.0;

pub fn ui_art_viewer(app: &mut UopInspectorApp, ctx: &egui::Context) {
    app.selected_legacy_source = concrete_art_tile_source(app.selected_legacy_source);

    egui::SidePanel::left("tex_art_cc_list")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
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
                let response = ui.text_edit_singleline(&mut app.search_query);
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
                let query = app.search_query.trim();
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

    egui::CentralPanel::default().show(ctx, |ui| {
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
                                    ui.label("Flags"); ui.label(format!("{:08X}", tile.flags.internal_flags)); ui.end_row();
                                    ui.label("Texture ID"); ui.label(tile.texture_id.to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(format!("{:?}", tile.flags));
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
                                    ui.label("Flags"); ui.label(format!("{:?}", tile.flags)); ui.end_row();
                                    ui.label("Weight"); ui.label(tile.weight.to_string()); ui.end_row();
                                    ui.label("Layer/LightID"); ui.label(tile.quality.to_string()); ui.end_row();
                                    ui.label("Count/Quantity"); ui.label(tile.quantity.to_string()); ui.end_row();
                                    ui.label("Anim ID"); ui.label(tile.anim_id.to_string()); ui.end_row();
                                    ui.label("Hue Extra"); ui.label(tile.hue_extra.to_string()); ui.end_row();
                                    ui.label("Height"); ui.label(tile.height_raw().to_string()); ui.end_row();
                                });
                                ui.separator();
                                ui.label("Flags Breakdown:");
                                ui.label(format!("{:?}", tile.flags));
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

fn ui_cc_tiledata(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    ui_tile_metadata_contents(app, ui);
}

pub fn ui_tile_metadata(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui_tile_metadata_contents(app, ui);
    });
}

fn ui_tile_metadata_contents(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
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
        let response = ui.text_edit_singleline(&mut app.search_query);
        text_has_focus |= response.has_focus();
    });
    ui.separator();

    match app.tile_metadata_source {
        TileMetadataSource::CcTileData => ui_cc_tiledata_table(app, ui, text_has_focus),
        TileMetadataSource::EcTileArt => ui_ec_tileart_table(app, ui, text_has_focus),
    }
}

fn ui_cc_tiledata_table(app: &mut UopInspectorApp, ui: &mut egui::Ui, text_has_focus: bool) {
    ui.heading("CC TileData Inspector");
    if let (Some(tiledata), Some(rows)) = (&app.cc_tiledata, app.cc_tiledata_rows.clone()) {
        let query = app.search_query.trim();
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
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_width(TILEDATA_TABLE_MIN_WIDTH);
            TableBuilder::new(ui)
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
        });

        ui_selected_tiledata_details(app, ui);
    } else {
        ui.label("Select a Classic Client path containing tiledata.mul.");
    }
}

fn ui_ec_tileart_table(app: &mut UopInspectorApp, ui: &mut egui::Ui, text_has_focus: bool) {
    ui.heading("EC TileArt Inspector");
    if let (Some(entries), Some(rows)) = (app.ec_tileart_entries.clone(), app.ec_tileart_rows.clone()) {
        let query = app.search_query.trim();
        let sort = list_sort_controls(
            ui,
            "ec_tileart_rows",
            &["ID", "Old ID", "Type", "Hash"],
            0,
        );
        let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
            match sort.option_index {
                1 => string_number(&left.old_id).cmp(&string_number(&right.old_id)),
                2 => left.type_name.cmp(right.type_name).then_with(|| string_number(&left.tile_id).cmp(&string_number(&right.tile_id))),
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
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_width(TILEART_TABLE_MIN_WIDTH);
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(180.0))
                .column(Column::auto().at_least(220.0))
                .column(Column::auto().at_least(120.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(120.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder().at_least(180.0))
                .column(Column::auto().at_least(90.0))
                .column(Column::auto().at_least(170.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("ID");
                    });
                    header.col(|ui| {
                        ui.strong("Old ID");
                    });
                    header.col(|ui| {
                        ui.strong("Type");
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
                        ui.strong("Textures");
                    });
                    header.col(|ui| {
                        ui.strong("Sitting");
                    });
                    header.col(|ui| {
                        ui.strong("Appearance");
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
                            ui.label(item.type_name);
                        });
                        row.col(|ui| {
                            ui.label(&item.properties_summary);
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
                            ui.label(&item.texture_summary);
                        });
                        row.col(|ui| {
                            ui.label(&item.sitting_summary);
                        });
                        row.col(|ui| {
                            ui.label(&item.appearance_summary);
                        });
                    });
                });
        });

        ui.separator();
        if let Some(selected_hash) = app.selected_tileart_hash {
            if let Some(file) = entries.iter().find(|file| file.filename_hash == selected_hash) {
                ui.heading(format!("Selected TileArt {}", file.entry.tile_id));
                ui_selected_tileart_details(app, ui, file);
            }
        }
    } else {
        ui.label("Select an Enhanced Client path containing tileart.uop.");
    }
}

fn ui_selected_tiledata_details(app: &UopInspectorApp, ui: &mut egui::Ui) {
    let Some(selected_id) = app.selected_tex_art_cc_id else {
        return;
    };
    let Some(rows) = app.cc_tiledata_rows.as_ref() else {
        return;
    };
    let Some(row) = rows.iter().find(|row| row.art_id == selected_id) else {
        return;
    };

    ui.separator();
    ui.heading(format!("Selected TileData {} {}", row.kind, row.id));
    egui::Grid::new("tiledata_selected_details").striped(true).show(ui, |ui| {
        ui.label("Name");
        ui.label(&row.name);
        ui.end_row();
        ui.label("Flags Raw");
        ui.monospace(&row.flags_raw);
        ui.end_row();
        ui.label("Flags Decoded");
        ui.label(&row.flags_summary);
        ui.end_row();
        if row.kind == "Land" {
            ui.label("Texture ID");
            ui.label(&row.texture_id);
            ui.end_row();
        } else {
            ui.label("Height");
            ui.label(&row.height);
            ui.end_row();
            ui.label("Weight");
            ui.label(&row.weight);
            ui.end_row();
            ui.label("Quality / Layer / Light");
            ui.label(&row.quality);
            ui.end_row();
            ui.label("Quantity");
            ui.label(&row.quantity);
            ui.end_row();
            ui.label("Animation ID");
            ui.label(&row.anim_id);
            ui.end_row();
            ui.label("Hue Extra");
            ui.label(&row.hue_extra);
            ui.end_row();
            ui.label("Stacking Offset");
            ui.label(&row.stacking_offset);
            ui.end_row();
            ui.label("Value");
            ui.label(&row.value);
            ui.end_row();
        }
    });
}

fn ui_selected_tileart_details(app: &UopInspectorApp, ui: &mut egui::Ui, file: &TileArtFileEntry) {
    let entry = &file.entry;

    ui.collapsing("Core", |ui| {
        egui::Grid::new("tileart_core_details").striped(true).show(ui, |ui| {
            ui.label("Filename Hash");
            ui.monospace(format!("0x{:016X}", file.filename_hash));
            ui.end_row();
            ui.label("Version");
            ui.label(entry.version.to_string());
            ui.end_row();
            ui.label("String Dict Offset");
            ui.label(entry.string_dict_off.to_string());
            ui.end_row();
            ui.label("Old ID");
            ui.label(entry.old_id.to_string());
            ui.end_row();
            ui.label("Type");
            ui.label(format!("{} ({})", tileart_type_name(entry.type_val), entry.type_val));
            ui.end_row();
            ui.label("Facing");
            ui.label(entry.facing.to_string());
            ui.end_row();
            ui.label("Light");
            ui.label(format!("{}, {}", entry.light1, entry.light2));
            ui.end_row();
            ui.label("Radar RGBA");
            ui.label(format!(
                "{}, {}, {}, {}",
                entry.radarcol.r,
                entry.radarcol.g,
                entry.radarcol.b,
                entry.radarcol.a
            ));
            ui.end_row();
        });
    });

    ui.collapsing("Flags", |ui| {
        egui::Grid::new("tileart_flags_details").striped(true).show(ui, |ui| {
            ui.label("Flags1 Raw");
            ui.monospace(format!("0x{:016X}", entry.flags1.bits()));
            ui.end_row();
            ui.label("Flags1 Decoded");
            ui.label(tileart_flags_summary(entry.flags1));
            ui.end_row();
            ui.label("Flags2 Raw");
            ui.monospace(format!("0x{:016X}", entry.flags2.bits()));
            ui.end_row();
            ui.label("Flags2 Decoded");
            ui.label(tileart_flags_summary(entry.flags2));
            ui.end_row();
        });
    });

    ui.collapsing("Image Windows", |ui| {
        egui::Grid::new("tileart_image_window_details").striped(true).show(ui, |ui| {
            ui.label("EC Window");
            ui.label(format!(
                "{},{} -> {},{}",
                entry.ec_img_offset.x_start,
                entry.ec_img_offset.y_start,
                entry.ec_img_offset.x_end,
                entry.ec_img_offset.y_end
            ));
            ui.end_row();
            ui.label("EC Offset");
            ui.label(format!("{},{}", entry.ec_img_offset.x_off, entry.ec_img_offset.y_off));
            ui.end_row();
            ui.label("CC Window");
            ui.label(format!(
                "{},{} -> {},{}",
                entry.cc_img_offset.x_start,
                entry.cc_img_offset.y_start,
                entry.cc_img_offset.x_end,
                entry.cc_img_offset.y_end
            ));
            ui.end_row();
            ui.label("CC Offset");
            ui.label(format!("{},{}", entry.cc_img_offset.x_off, entry.cc_img_offset.y_off));
            ui.end_row();
        });
    });

    ui.collapsing("Properties", |ui| {
        egui::Grid::new("tileart_property_details").striped(true).show(ui, |ui| {
            ui.label("Vector");
            ui.label("Key");
            ui.label("Value");
            ui.end_row();
            for (vector_name, props) in [
                ("prop_vector1", entry.prop_vector1.as_slice()),
                ("prop_vector2", entry.prop_vector2.as_slice()),
            ] {
                for prop in props {
                    ui.label(vector_name);
                    ui.label(format!("{} ({})", tileart_property_name(prop.id), prop.id));
                    ui.label(prop.val.to_string());
                    ui.end_row();
                }
            }
        });
    });

    ui.collapsing("Stack Aliases", |ui| {
        if entry.stack_alias_vector.is_empty() {
            ui.label("none");
            return;
        }
        egui::Grid::new("tileart_stack_alias_details").striped(true).show(ui, |ui| {
            ui.label("Amount");
            ui.label("Amount ID");
            ui.end_row();
            for alias in &entry.stack_alias_vector {
                ui.label(alias.amount.to_string());
                ui.label(alias.amount_id.to_string());
                ui.end_row();
            }
        });
    });

    ui.collapsing("Textures", |ui| {
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
                    "unk8 {} / unk9 {}",
                    block.unk8_count,
                    block.unk9_count
                ));
                ui.end_row();
                for (item_index, item) in block.texture_items.iter().enumerate() {
                    ui.label(format!("{}.{}", block_index, item_index));
                    ui.label(format!("name off {}", item.name_string_off));
                    ui.label(format!("stretch {}", item.texture_stretch));
                    ui.label(format!("unk4 {}", item.unk4));
                    ui.label(format!("unk6 {} / unk7 {}", item.unk6, item.unk7));
                    ui.end_row();
                }
            }
        });

        if let Some(dict) = app.uo_string_dictionary.as_ref() {
            let art_data = entry.process(dict);
            ui.separator();
            ui.label("Resolved texture items");
            egui::Grid::new("tileart_resolved_texture_details").striped(true).show(ui, |ui| {
                ui.label("Block.Item");
                ui.label("Type");
                ui.label("ID");
                ui.label("Stretch");
                ui.label("Path");
                ui.end_row();
                for block in art_data.texture_items {
                    for item in block {
                        ui.label(format!("{}.{}", item.block_index, item.item_index));
                        ui.label(format!("{:?}", item.texture_type));
                        ui.label(item.id.to_string());
                        ui.label(item.texture_stretch.to_string());
                        ui.label(item.path);
                        ui.end_row();
                    }
                }
            });
        }
    });

    ui.collapsing("Sitting And Appearance", |ui| {
        egui::Grid::new("tileart_selected_details").striped(true).show(ui, |ui| {
            ui.label("Sitting");
            ui.label(tileart_sitting_summary(entry.sitting.as_ref()));
            ui.end_row();
            ui.label("Appearance");
            ui.label(tileart_appearance_summary(&entry.appearance_vector));
            ui.end_row();
            if let Some(sitting) = &entry.sitting {
                ui.label("Sitting Values");
                ui.monospace(format!(
                    "{}, {}, {}, {}",
                    sitting.unk1, sitting.unk2, sitting.unk3, sitting.unk4
                ));
                ui.end_row();
            }
            for (index, appearance) in entry.appearance_vector.iter().enumerate() {
                ui.label(format!("Appearance {}", index));
                ui.label(tileart_appearance_detail(appearance));
                ui.end_row();
            }
        });
    });
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

fn tileart_sitting_summary(sitting: Option<&TaeSittingAnimation>) -> String {
    if let Some(sitting) = sitting {
        format!(
            "yes ({}, {}, {}, {})",
            sitting.unk1, sitting.unk2, sitting.unk3, sitting.unk4
        )
    } else {
        "no".to_string()
    }
}

fn tileart_appearance_summary(appearance: &[TaeAnimationAppearance]) -> String {
    if appearance.is_empty() {
        return "none".to_string();
    }

    let mut counts = [0usize; 2];
    let mut other = 0usize;
    for item in appearance {
        match item.sub_type {
            0 => counts[0] += 1,
            1 => counts[1] += 1,
            _ => other += 1,
        }
    }

    format!(
        "{} records (type0 {}, type1 {}, other {})",
        appearance.len(),
        counts[0],
        counts[1],
        other
    )
}

fn tileart_appearance_detail(appearance: &TaeAnimationAppearance) -> String {
    if let Some(sub1) = &appearance.sub1 {
        return format!("type {}: {}, {}", appearance.sub_type, sub1.unk1, sub1.unk2);
    }
    if let Some(sub2) = &appearance.sub2 {
        let pairs = sub2
            .sub3_vector
            .iter()
            .map(|sub3| format!("{}:{}", sub3.unk1, sub3.unk2))
            .collect::<Vec<_>>()
            .join(", ");
        return format!(
            "type {}: {} records [{}]",
            appearance.sub_type,
            sub2.sub_count,
            pairs
        );
    }

    format!("type {}: empty", appearance.sub_type)
}
