use eframe::egui;
use crate::app::{classic_tile_flags_summary, concrete_art_tile_source, ArtSource, ArtViewerRow, UopInspectorApp, ViewMode};
use crate::logic::ClientData;
use super::{arrow_delta, go_to_field, list_sort_controls, move_selection, sorted_indices_by};
use super::classic_image_import::{ui_classic_image_panel, PreferredContainer};
use uocf::classic::image_files::ClassicImageKind;
use std::sync::Arc;
use uocf::classic::art::{static_art_id_for_source, STATIC_TILE_ID_BASE};

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
            let go_to = go_to_field(ui, "art_viewer_rows");
            art_search_has_focus |= go_to.has_focus;

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
                } else if let Some(position) = go_to.position_in(visible_ids.iter().map(|&art_id| u64::from(art_id))) {
                    app.selected_tex_art_cc_id = Some(visible_ids[position]);
                    app.view_mode = ViewMode::TexArtCc;
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
    super::tile_metadata::contents(app, ctx, ui);
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
