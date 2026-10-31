use eframe::egui;
use crate::app::{guess_uop_image_format_from_payload, UopInspectorApp};
use super::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by};
use uocf::enhanced::tileart::TileArtEntry;
use uocf::enhanced::waypoints::{
    WaypointClilocDefinition, WaypointTypeDefinition, WaypointsPackage,
    KNOWN_WAYPOINTS_PAYLOAD_HASH, WAYPOINTS_PAYLOAD_PATH, WAYPOINTS_UOP_NAME,
};
use crate::logic::waypoint_editor::{WaypointDocument, WaypointField};
use super::UOP_PATCHER_NOTE;
use super::legacy_terrain;

const UOP_BROWSER_PACKAGE_PANEL_WIDTH: f32 = 200.0;
const UOP_BROWSER_ENTRY_PANEL_WIDTH: f32 = 350.0;
const WAYPOINT_RECORDS_MAX_HEIGHT: f32 = 360.0;
/// Drag speed of the waypoint number fields (typing is the main input).
const WAYPOINT_DRAG_SPEED: f64 = 0.1;
const WAYPOINT_ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WAYPOINT_WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);

pub fn has_visible_uop_explorer_package(app: &UopInspectorApp) -> bool {
    app.uop_cache
        .loaded_uops
        .iter()
        .any(|loaded| !package_has_specialized_viewer(loaded.path.file_name().and_then(|name| name.to_str())))
}

pub fn ui_uop_browser(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::left("package_panel")
        .resizable(true)
        .default_size(UOP_BROWSER_PACKAGE_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading("Packages");
            let sort = list_sort_controls(ui, "uop_packages", &["Source", "Name"], 0);
            let package_indices = sorted_indices_by(
                &app.uop_cache.loaded_uops,
                sort.ordering(),
                |left, right| match sort.option_index {
                    1 => left
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .cmp(
                            &right
                                .path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or_default()
                                .to_ascii_lowercase(),
                        ),
                    _ => std::cmp::Ordering::Equal,
                },
            );
            egui::ScrollArea::vertical().show(ui, |ui| {
                for i in package_indices {
                    let loaded = &app.uop_cache.loaded_uops[i];
                    let name = loaded.path.file_name().unwrap_or_default().to_string_lossy();
                    if package_has_specialized_viewer(Some(name.as_ref())) && app.selected_uop_idx != Some(i) {
                        continue;
                    }
                    if ui.selectable_label(app.selected_uop_idx == Some(i), name).clicked() {
                        app.selected_uop_idx = Some(i);
                        app.selected_file_hash = None;
                    }
                }
            });
        });

    if let Some(uop_idx) = app.selected_uop_idx {
        egui::Panel::left("entry_panel")
            .resizable(true)
            .default_size(UOP_BROWSER_ENTRY_PANEL_WIDTH)
            .show(ui, |ui| {
                ui.heading("Entries");
                let mut text_has_focus = false;
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    let res = ui.text_edit_singleline(&mut app.search_query);
                    text_has_focus |= res.has_focus();
                });
                ui.horizontal(|ui| {
                    ui.label("Find Hash:");
                    let res = ui.text_edit_singleline(&mut app.find_hash_query);
                    text_has_focus |= res.has_focus();
                    if res.changed() || ui.button("Find").clicked() {
                       if let Ok(h) = u64::from_str_radix(app.find_hash_query.trim_start_matches("0x"), 16) {
                           app.selected_file_hash = Some(h);
                       }
                    }
                });
                let mut hide_empty_entries = app.hide_empty_uop_entries;
                if ui.checkbox(&mut hide_empty_entries, "Hide empty/invalid entries").changed() {
                    app.set_hide_empty_uop_entries(hide_empty_entries);
                }
                if app.selected_uop_is_terrain_texture(uop_idx) {
                    let mut enabled = app.guess_terrain_texture_file_format;
                    if ui.checkbox(&mut enabled, "Guess file format").changed() {
                        app.set_guess_terrain_texture_file_format(enabled);
                    }
                }

                let entry_labels = app.get_uop_entry_labels(uop_idx);
                let sort = list_sort_controls(ui, "uop_entries", &["Source", "Name", "Hash"], 0);
                let sorted_indices = sorted_indices_by(&entry_labels, sort.ordering(), |left, right| {
                    match sort.option_index {
                        1 => left.search_name.cmp(&right.search_name),
                        2 => left.hash.cmp(&right.hash),
                        _ => std::cmp::Ordering::Equal,
                    }
                });
                let query = app.search_query.to_lowercase();
                let visible_hashes: Vec<u64> = sorted_indices
                    .iter()
                    .map(|index| &entry_labels[*index])
                    .filter(|file| query.is_empty() || file.search_name.contains(&query))
                    .map(|file| file.hash)
                    .collect();
                let keyboard_moved = if let Some(delta) = arrow_delta(ui, text_has_focus) {
                    app.selected_file_hash = move_selection(&visible_hashes, app.selected_file_hash, delta);
                    true
                } else {
                    false
                };

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for index in sorted_indices {
                        let file = &entry_labels[index];
                        if !query.is_empty() && !file.search_name.contains(&query) {
                            continue;
                        }

                        let selected = app.selected_file_hash == Some(file.hash);
                        let response = ui.selectable_label(selected, &file.display_name);
                        if keyboard_moved && selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            app.selected_file_hash = Some(file.hash);
                        }
                    }
                });
            });
    }

    egui::CentralPanel::default().show(ui, |ui| {
        if let (Some(uop_idx), Some(file_hash)) = (app.selected_uop_idx, app.selected_file_hash) {
            let loaded_uop = app.uop_cache.loaded_uops[uop_idx].clone();
            if let Some(file) = loaded_uop.package.get_file_by_hash(file_hash) {
                let hash = file.filename_hash();
                let resolved_name = app.resolve_uop_entry_display_name(uop_idx, hash);

                ui.horizontal(|ui| {
                    ui.heading("Entry Details");
                    ui.label(format!("(Hash: {:016X})", hash));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("📥 Extract...").clicked() {
                            app.save_entry(hash, &resolved_name);
                        }
                    });
                });
                ui.label(format!("Name: {}", resolved_name));

                ui.separator();

                // Specialized Visualizers
                if is_waypoint_payload(hash, &resolved_name) {
                    ui_integrated_waypoints_view(app, ui, file, &loaded_uop.path);
                } else if legacy_terrain::is_legacy_terrain_package(&loaded_uop.path) {
                    legacy_terrain::ui_legacy_terrain_view(app, ui, file, &loaded_uop.path, &loaded_uop.package);
                } else if resolved_name.contains("tileart") && resolved_name.ends_with(".bin") {
                    ui_integrated_tileart_view(app, ctx, ui, file);
                } else if resolved_name.contains("terraindefinition") && resolved_name.ends_with(".bin") {
                    ui_integrated_terrain_view(app, ctx, ui, file);
                } else {
                    ui_generic_preview(app, ctx, ui, uop_idx, hash, &resolved_name);
                }
            }
        } else {
            ui.centered_and_justified(|ui| { ui.label("Select a package and an entry"); });
        }
    });
}

fn package_has_specialized_viewer(file_name: Option<&str>) -> bool {
    let Some(file_name) = file_name else {
        return false;
    };
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "artlegacymul.uop"
            | "gumpartlegacymul.uop"
            | "hues.uop"
            | "localizedstrings.uop"
            | "legacytexture.uop"
            | "multicollection.uop"
            | "string_dictionary.uop"
            | "terraindefinition.uop"
            | "tileart.uop"
    )
}

fn is_waypoint_payload(hash: u64, resolved_name: &str) -> bool {
    hash == KNOWN_WAYPOINTS_PAYLOAD_HASH
        || resolved_name.eq_ignore_ascii_case(WAYPOINTS_PAYLOAD_PATH)
        || resolved_name.eq_ignore_ascii_case("waypoint.bin")
}

fn ui_integrated_waypoints_view(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    file: &uocf::uop_container::file::UopFile,
    source: &std::path::Path,
) {
    let bytes = match file.unpack() {
        Ok(bytes) => bytes,
        Err(error) => {
            ui.label(format!("Failed to unpack waypoint payload: {error}"));
            return;
        }
    };
    let package = match WaypointsPackage::from_payload_bytes(&bytes) {
        Ok(package) => package,
        Err(error) => {
            ui.label(format!("Failed to parse waypoint payload: {error}"));
            return;
        }
    };

    ui.heading("EC Waypoints");
    ui.label(format!("version: {}", package.version));
    ui.label(format!(
        "{} icon definitions, {} effect definitions, {} type definitions, {} waypoint records",
        package.icon_definitions.len(),
        package.effect_definitions.len(),
        package.type_definitions.len(),
        package.waypoints.len()
    ));
    if !package.trailing_bytes.is_empty() {
        ui.label(format!("trailing_bytes: {}", package.trailing_bytes.len()));
    }

    ui.separator();
    let mut document = app.waypoint_document.take().filter(|document| document.source == source);
    ui.horizontal_wrapped(|ui| match document.as_mut() {
        None => {
            if ui.button("Edit").on_hover_text("Edit the waypoint records and save a copy of waypoint.uop.").clicked() {
                document = Some(WaypointDocument::new(source.to_path_buf(), package.clone()));
            }
        }
        Some(editing) => {
            if waypoint_toolbar(app, ui, editing) {
                document = None;
            }
        }
    });
    if let Some(mut editing) = document {
        egui::CollapsingHeader::new("Waypoint Records")
            .default_open(true)
            .show(ui, |ui| waypoint_editor_grid(app, ui, &mut editing));
        app.waypoint_document = Some(editing);
    } else {
    egui::CollapsingHeader::new("Waypoint Records")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::both().max_height(WAYPOINT_RECORDS_MAX_HEIGHT).show(ui, |ui| {
                egui::Grid::new("waypoint_records_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("#");
                        ui.label("x");
                        ui.label("y");
                        ui.label("z");
                        ui.label("facet");
                        ui.label("waypoint_type");
                        ui.label("value_6");
                        ui.label("name_id");
                        ui.label("name");
                        ui.end_row();

                        for (index, waypoint) in package.waypoints.iter().enumerate() {
                            ui.label(index.to_string());
                            ui.label(waypoint.x.to_string());
                            ui.label(waypoint.y.to_string());
                            ui.label(waypoint.z.to_string());
                            ui.label(waypoint.facet.to_string());
                            ui.label(waypoint_type_label(waypoint.waypoint_type));
                            ui.label(waypoint.value_6.to_string());
                            ui.label(waypoint.name_cliloc.to_string());
                            ui.label(resolve_localized_string(app, waypoint.name_cliloc));
                            ui.end_row();
                        }
                    });
            });
        });
    }

    ui.separator();
    draw_waypoint_cliloc_definitions(
        app,
        ui,
        "Icon Definitions",
        "waypoint_icon_definitions_grid",
        &package.icon_definitions,
    );
    draw_waypoint_cliloc_definitions(
        app,
        ui,
        "Effect Definitions",
        "waypoint_effect_definitions_grid",
        &package.effect_definitions,
    );

    egui::CollapsingHeader::new("Type Definitions").show(ui, |ui| {
        egui::ScrollArea::both().max_height(240.0).show(ui, |ui| {
            egui::Grid::new("waypoint_type_definitions_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("#");
                    ui.label("name_id");
                    ui.label("name");
                    ui.label("flags");
                    ui.label("links");
                    ui.end_row();

                    for (index, definition) in package.type_definitions.iter().enumerate() {
                        ui.label(index.to_string());
                        ui.label(definition.name_cliloc.to_string());
                        ui.label(resolve_localized_string(app, definition.name_cliloc));
                        ui.label(definition.flags.to_string());
                        ui.label(type_links_summary(definition));
                        ui.end_row();
                    }
                });
        });
    });
}

/// Undo, redo, save and close of the edited waypoints; true when the editor is closed.
fn waypoint_toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui, document: &mut WaypointDocument) -> bool {
    if ui.add_enabled(document.edits.can_undo(), egui::Button::new("Undo")).clicked() {
        document.undo();
    }
    if ui.add_enabled(document.edits.can_redo(), egui::Button::new("Redo")).clicked() {
        document.redo();
    }
    if ui
        .button("Save waypoint.uop...")
        .on_hover_text(format!(
            "Writes a copy of the loaded waypoint.uop with these waypoints. The loaded package itself is refused \
             as the target.\n{UOP_PATCHER_NOTE}"
        ))
        .clicked()
    {
        let (name, extensions) = UOP_FILTER;
        let output = crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(WAYPOINTS_UOP_NAME).save_file();
        if let Some(output) = output {
            match document.save(&output) {
                Ok(()) => {
                    document.error = None;
                    let message = format!("Saved {}.", output.display());
                    app.status_message = message.clone();
                    app.log(message);
                }
                Err(error) => document.error = Some(format!("Save failed: {error}")),
            }
        }
    }
    let dirty = document.edits.is_dirty();
    let close_label = if dirty { "Discard and close" } else { "Close editor" };
    let close = ui.button(close_label).clicked();
    if dirty {
        ui.colored_label(WAYPOINT_WARNING_COLOR, "* unsaved changes");
    }
    if let Some(error) = &document.error {
        ui.colored_label(WAYPOINT_ERROR_COLOR, error);
    }
    close
}

/// The waypoint records as editable rows, with duplicate and delete per row.
fn waypoint_editor_grid(app: &UopInspectorApp, ui: &mut egui::Ui, document: &mut WaypointDocument) {
    let mut add_after = None;
    let mut remove = None;
    if ui.button("Add waypoint").clicked() {
        document.add_waypoint(None);
    }
    egui::ScrollArea::both().max_height(WAYPOINT_RECORDS_MAX_HEIGHT).show(ui, |ui| {
        egui::Grid::new("waypoint_records_editor").striped(true).show(ui, |ui| {
            for header in ["#", "x", "y", "z", "facet", "waypoint_type", "value_6", "name_id", "name", ""] {
                ui.label(header);
            }
            ui.end_row();
            for index in 0..document.package.waypoints.len() {
                let mut waypoint = document.package.waypoints[index];
                ui.label(index.to_string());
                let mut changed = None;
                for (field, response) in [
                    (WaypointField::X, ui.add(egui::DragValue::new(&mut waypoint.x).speed(WAYPOINT_DRAG_SPEED))),
                    (WaypointField::Y, ui.add(egui::DragValue::new(&mut waypoint.y).speed(WAYPOINT_DRAG_SPEED))),
                    (WaypointField::Z, ui.add(egui::DragValue::new(&mut waypoint.z).speed(WAYPOINT_DRAG_SPEED))),
                    (WaypointField::Facet, ui.add(egui::DragValue::new(&mut waypoint.facet).speed(WAYPOINT_DRAG_SPEED))),
                    (
                        WaypointField::Type,
                        ui.add(egui::DragValue::new(&mut waypoint.waypoint_type).speed(WAYPOINT_DRAG_SPEED))
                            .on_hover_text(waypoint_type_label(waypoint.waypoint_type)),
                    ),
                    (
                        WaypointField::Value6,
                        ui.add(egui::DragValue::new(&mut waypoint.value_6).speed(WAYPOINT_DRAG_SPEED))
                            .on_hover_text("Meaning unknown; 1, or the shop kind on shop waypoints."),
                    ),
                    (
                        WaypointField::NameCliloc,
                        ui.add(egui::DragValue::new(&mut waypoint.name_cliloc).speed(WAYPOINT_DRAG_SPEED)),
                    ),
                ] {
                    if response.drag_started() || response.gained_focus() {
                        document.edits.break_coalescing();
                    }
                    if response.changed() {
                        changed = Some(field);
                    }
                }
                ui.label(resolve_localized_string(app, waypoint.name_cliloc));
                ui.horizontal(|ui| {
                    if ui.small_button("Duplicate").clicked() {
                        add_after = Some(index);
                    }
                    if ui.small_button("Delete").clicked() {
                        remove = Some(index);
                    }
                });
                ui.end_row();
                if let Some(field) = changed {
                    document.edit_waypoint(index, field, |record| *record = waypoint);
                }
            }
        });
    });
    if let Some(index) = add_after {
        document.add_waypoint(Some(index));
    }
    if let Some(index) = remove {
        document.remove_waypoint(index);
    }
}

fn draw_waypoint_cliloc_definitions(
    app: &UopInspectorApp,
    ui: &mut egui::Ui,
    title: &str,
    grid_id: &str,
    definitions: &[WaypointClilocDefinition],
) {
    egui::CollapsingHeader::new(title).show(ui, |ui| {
        egui::ScrollArea::both().max_height(220.0).show(ui, |ui| {
            egui::Grid::new(grid_id).striped(true).show(ui, |ui| {
                ui.label("#");
                ui.label("id");
                ui.label("name_id");
                ui.label("name");
                ui.end_row();

                for (index, definition) in definitions.iter().enumerate() {
                    ui.label(index.to_string());
                    ui.label(definition.id.to_string());
                    ui.label(definition.name_cliloc.to_string());
                    ui.label(resolve_localized_string(app, definition.name_cliloc));
                    ui.end_row();
                }
            });
        });
    });
}

fn type_links_summary(definition: &WaypointTypeDefinition) -> String {
    definition
        .links
        .iter()
        .map(|link| format!("{}:{}", link.value_1, link.value_2))
        .collect::<Vec<_>>()
        .join(", ")
}

fn waypoint_type_label(waypoint_type: u16) -> String {
    let label = match waypoint_type {
        1 => "corpse",
        2 => "party",
        4 => "quest giver",
        5 => "new player quest",
        6 => "wandering healer",
        7 => "danger",
        9 => "city",
        10 => "dungeon",
        11 => "shrine",
        12 => "moongate",
        14 => "player",
        15 => "custom",
        _ => "unknown",
    };
    format!("{waypoint_type} ({label})")
}

fn resolve_localized_string(app: &UopInspectorApp, string_id: u32) -> String {
    app
        .localized_strings
        .as_ref()
        .and_then(|package| package.files.iter().find_map(|file| file.strings.get(string_id)))
        .map(str::to_string)
        .unwrap_or_default()
}

fn ui_generic_preview(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    uop_idx: usize,
    hash: u64,
    name: &str,
) {
    if let Some(data) = app.get_uop_entry_payload(uop_idx, hash) {
        let data = data.as_ref();
        let lower_name = name.to_lowercase();
        let guessed_image_name = app
            .guess_terrain_texture_file_format
            .then(|| guess_uop_image_format_from_payload(data).map(|(extension, _)| format!("{hash:016X}.{extension}")))
            .flatten();
        if lower_name.ends_with(".dds")
            || lower_name.ends_with(".tga")
            || lower_name.ends_with(".bmp")
            || guessed_image_name.is_some()
        {
            let texture_name = guessed_image_name.as_deref().unwrap_or(name);
            if let Some(handle) = app.get_uop_texture(ctx, hash, data, texture_name) {
                ui.label(format!("Image: {}x{}", handle.size()[0], handle.size()[1]));
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(&handle);
                });
            }
        } else if data.starts_with(b"<?xml") || name.ends_with(".xml") || name.ends_with(".def") {
            if let Ok(text) = std::str::from_utf8(data) {
                let mut text_ref = text;
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text_ref)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY));
                });
            }
        } else {
            ui.label("Hex Preview (first 1KB):");
            egui::ScrollArea::vertical().show(ui, |ui| {
                let limit = data.len().min(1024);
                let mut hex_text = String::with_capacity(limit * 3);
                for byte in &data[..limit] {
                    hex_text.push_str(&format!("{:02X} ", byte));
                }
                ui.monospace(hex_text);
            });
        }
    }
}

fn ui_integrated_tileart_view(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop_container::file::UopFile) {
    if let Ok(tae) = TileArtEntry::parse_raw(file) {
        ui.horizontal(|ui| {
            // Image Column
            ui.vertical(|ui| {
                if let Some(dict) = app.uo_string_dictionary.as_ref() {
                    let art_data = tae.process(dict);
                    for (i, row) in art_data.texture_items.iter().enumerate() {
                         for (j, item) in row.iter().enumerate() {
                             if let Some(handle) = app.get_ec_texture_by_id(ctx, item.id) {
                                 ui.label(format!("Layer {}.{}: {}", i, j, item.path));
                                 ui.image(&handle);
                             }
                         }
                    }
                } else {
                    ui.label("Load UO String Dictionary to resolve texture paths");
                }
            });

            ui.separator();

            // Properties Column
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("tae_props").striped(true).show(ui, |ui| {
                    ui.label("Version"); ui.label(tae.version.to_string()); ui.end_row();
                    ui.label("StrDic Offset"); ui.label(tae.string_dict_off.to_string()); ui.end_row();
                    ui.label("ID"); ui.label(tae.tile_id.to_string()); ui.end_row();
                    ui.label("OldID"); ui.label(format!("{} {:06X}", tae.old_id, tae.old_id)); ui.end_row();
                    ui.label("Body Type"); ui.label(tae.body_type.to_string()); ui.end_row();
                    ui.label("FLAGS"); ui.label(format!("{:?}", tae.flags1)); ui.end_row();
                });
            });
        });
    }
}

fn ui_integrated_terrain_view(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop_container::file::UopFile) {
    let dict = app.uo_string_dictionary.as_deref();
    if let Ok(entry) = uocf::enhanced::terrain_definition::parse_entry(file, dict) {
        ui.heading(format!("Terrain: {}", entry.name.as_deref().unwrap_or("Unknown")));
        ui.label(format!("ID: {}", entry.id));

        ui.separator();

        ui.horizontal(|ui| {
             ui.vertical(|ui| {
                 ui.heading("Textures");
                 if let Some(tex) = &entry.texture {
                     ui.label(format!("Shader: {}", tex.shader_name.as_deref().unwrap_or("None")));
                     for layer in &tex.layers {
                         ui.horizontal(|ui| {
                             ui.label(format!("- {}", layer.path.as_deref().unwrap_or("?")));
                             if let Some(tid) = layer.texture_id {
                                 if let Some(handle) = app.get_ec_texture_by_id(ctx, tid) {
                                     ui.image(&handle);
                                 }
                             }
                         });
                     }
                 }
             });

             ui.separator();

             ui.vertical(|ui| {
                 ui.heading("Aliases");
                 for alias in &entry.aliases {
                     ui.label(format!("Slot: {}, Alias: {}, Flags: {:016X}", alias.count_index, alias.alias, alias.tile_flags));
                 }
             });
        });
    }
}
