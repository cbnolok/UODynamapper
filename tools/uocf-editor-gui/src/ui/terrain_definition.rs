use crate::app::{TerrainDefinitionFileEntry, UocfEditorApp};
use crate::ui::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by, terrain_definition_editor};
use eframe::egui;
use uocf::enhanced::terrain_definition::{TerrainDefinitionEntry, TerrainTextureType};
use std::collections::BTreeSet;

const TERRAIN_DEFINITION_PANEL_WIDTH: f32 = 300.0;

pub fn ui_terrain_definition(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let files_arc = if let Some(ref files) = app.terrain_def_files {
        files.clone()
    } else {
        return;
    };
    let files = files_arc.as_ref();
    let package = app
        .uop_cache
        .loaded_uops
        .iter()
        .find(|loaded| terrain_definition_editor::is_terrain_definition_package(&loaded.path))
        .cloned();

    if let Some(loaded) = package.as_ref().filter(|loaded| terrain_definition_editor::is_editing(app, &loaded.path)) {
        // The editor lays out the whole panel: its table and the selected definition scroll by themselves.
        egui::CentralPanel::default().show(ui, |ui| {
            terrain_definition_editor::ui_terrain_definition_editor(app, ui, &loaded.path, &loaded.package);
        });
        return;
    }

    egui::Panel::left("terrain_def_panel")
        .resizable(true)
        .default_size(TERRAIN_DEFINITION_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading(format!("Terrain Definition Files ({})", files.len()));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.searches.terrain_definition);
            });
            let sort = list_sort_controls(ui, "terrain_definition_files", &["ID", "Name", "Hash", "Bytes"], 0);
            ui.separator();

            let sorted_indices = sorted_indices_by(files, sort.ordering(), |left, right| {
                match sort.option_index {
                    1 => left
                        .entry
                        .name
                        .as_deref()
                        .unwrap_or("Unknown")
                        .to_ascii_lowercase()
                        .cmp(
                            &right
                                .entry
                                .name
                                .as_deref()
                                .unwrap_or("Unknown")
                                .to_ascii_lowercase(),
                        ),
                    2 => left.filename_hash.cmp(&right.filename_hash),
                    3 => left.byte_len.cmp(&right.byte_len),
                    _ => left.entry.id.cmp(&right.entry.id),
                }
            });
            let visible: Vec<usize> = sorted_indices
                .into_iter()
                .filter(|&index| {
                    let file = &files[index];
                    let entry = &file.entry;
                    let name = entry.name.as_deref().unwrap_or("Unknown");
                    app.searches.terrain_definition.is_empty()
                        || name.to_lowercase().contains(&app.searches.terrain_definition.to_lowercase())
                        || entry.id.to_string().contains(&app.searches.terrain_definition)
                        || format!("{:016X}", file.filename_hash).to_lowercase().contains(&app.searches.terrain_definition.to_lowercase())
                })
                .collect();
            // The arrow keys browse the list, unless a text field has the keyboard.
            let hashes: Vec<u64> = visible.iter().map(|&index| files[index].filename_hash).collect();
            let keyboard_moved = match arrow_delta(ui, ui.ctx().text_edit_focused())
                .and_then(|delta| move_selection(&hashes, app.selected_terrain_def_hash, delta))
            {
                Some(hash) if Some(hash) != app.selected_terrain_def_hash => {
                    app.selected_terrain_def_hash = Some(hash);
                    true
                }
                _ => false,
            };

            egui::ScrollArea::vertical().show(ui, |ui| {
                for index in visible {
                    let file = &files[index];
                    let entry = &file.entry;
                    let name = entry.name.as_deref().unwrap_or("Unknown");

                    let is_selected = app.selected_terrain_def_hash == Some(file.filename_hash);
                    let label = format!("[{}] {} ({:016X})", entry.id, name, file.filename_hash);

                    let response = ui.selectable_label(is_selected, label);
                    if keyboard_moved && is_selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        app.selected_terrain_def_hash = Some(file.filename_hash);
                    }
                }
            });
        });

    egui::Panel::top("terrain_definition_tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut app.view_mode,
                crate::app::ViewMode::TerrainDefinition,
                "Specialized",
            );
            if ui.button("Raw UOP").clicked() {
                if let Some(hash) = app.selected_terrain_def_hash {
                    app.select_raw_uop_entry("terraindefinition.uop", hash);
                }
            }
            if let Some(loaded) = &package {
                ui.separator();
                terrain_definition_editor::ui_terrain_definition_editor(app, ui, &loaded.path, &loaded.package);
            }
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(selected_hash) = app.selected_terrain_def_hash {
            if let Some(file) = files.iter().find(|file| file.filename_hash == selected_hash) {
                // Both ways: the texture layer grid is wider than a small window.
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    ui_terrain_file(app, ctx, ui, file);
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a TerrainDefinition.uop file from the left panel.");
            });
        }
    });
}

fn ui_terrain_file(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    file: &TerrainDefinitionFileEntry,
) {
    let entry = &file.entry;
    ui.heading(format!(
        "Terrain Definition: {} ({})",
        entry.name.as_deref().unwrap_or("Unknown"),
        entry.id
    ));
    ui.label(format!("File hash: 0x{:016X}", file.filename_hash));
    ui.label(format!("Encoded byte length: {}", file.byte_len));
    ui.separator();

    ui.heading("Encoded Header");
    egui::Grid::new("td_encoded_header_grid").striped(true).show(ui, |ui| {
        ui.label("name_index");
        ui.label(entry.name_index.to_string());
        ui.end_row();
        ui.label("id");
        ui.label(entry.id.to_string());
        ui.end_row();
        ui.label("field_34");
        ui.label(entry.field_34.to_string());
        ui.end_row();
        ui.label("coverage_exponent");
        ui.label(entry.coverage_exponent().to_string());
        ui.end_row();
        ui.label("field_3c");
        ui.label(entry.field_3c.to_string());
        ui.end_row();
        ui.label("alias_count");
        ui.label(entry.aliases.len().to_string());
        ui.end_row();
        ui.label("texture_present");
        ui.label(entry.texture.is_some().to_string());
        ui.end_row();
    });

    ui.add_space(10.0);
    ui.heading("Aliases");
    if entry.aliases.is_empty() {
        ui.label("No aliases.");
    } else {
        egui::Grid::new("td_aliases_grid").striped(true).show(ui, |ui| {
            ui.label("subtype");
            ui.label("legacy_land_id");
            ui.label("tile_flags");
            ui.end_row();

            for alias in &entry.aliases {
                ui.label(alias.subtype.to_string());
                ui.label(format!("0x{:04X} ({})", alias.legacy_land_id, alias.legacy_land_id));
                ui.label(format!("0x{:016X}", alias.tile_flags));
                ui.end_row();
            }
        });
    }

    ui.add_space(10.0);
    ui_cc_alias_textures(app, ctx, ui, entry);

    ui.add_space(10.0);
    ui_texture_data(app, ctx, ui, entry);

    ui.add_space(10.0);
    ui.heading("Raw Encoded Prefix");
    ui.monospace(hex_prefix(&file.raw_prefix));
}

fn ui_cc_alias_textures(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &TerrainDefinitionEntry,
) {
    ui.heading("Linked CC Land Art");

    let mut aliases = entry
        .aliases
        .iter()
        .map(|alias| alias.legacy_land_id)
        .filter(|alias| *alias != 0)
        .collect::<BTreeSet<_>>();

    if aliases.is_empty() {
        aliases.insert(entry.id);
    }

    if app.client_data.is_none() {
        ui.label("Select a Classic Client path to preview linked CC land art.");
        return;
    }

    ui.horizontal_wrapped(|ui| {
        for alias in aliases {
            ui.vertical(|ui| {
                ui.label(format!("0x{:04X}", alias));
                if let Some(handle) = app.get_tex_art_cc_texture(ctx, alias) {
                    ui.add(thumbnail(&handle, 72.0));
                } else {
                    ui.label("missing");
                }
            });
        }
    });
}

fn ui_texture_data(
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &TerrainDefinitionEntry,
) {
    if let Some(texture) = &entry.texture {
        ui.heading("Texture Item");
        egui::Grid::new("td_texture_header_grid").striped(true).show(ui, |ui| {
            ui.label("field_04");
            ui.label(texture.field_04.to_string());
            ui.end_row();
            ui.label("shader_name_index");
            ui.label(texture.shader_name_index.to_string());
            ui.end_row();
            ui.label("shader_name");
            ui.label(texture.shader_name.as_deref().unwrap_or("None"));
            ui.end_row();
            ui.label("layer_count");
            ui.label(texture.layers.len().to_string());
            ui.end_row();
            ui.label("techniques");
            ui.label(format!("{:?}", texture.techniques()));
            ui.end_row();
            ui.label("tweaks");
            ui.label(format!("{:?}", texture.tweaks()));
            ui.end_row();
        });

        ui.add_space(5.0);
        ui.heading("Texture Layers");
        egui::Grid::new("td_layers_grid").striped(true).show(ui, |ui| {
            ui.label("#");
            ui.label("path_index");
            ui.label("path");
            ui.label("texture_id");
            ui.label("type");
            ui.label("role");
            ui.label("source_kind");
            ui.label("repetition");
            ui.label("sampler_index");
            ui.label("uv_set");
            ui.label("preview");
            ui.end_row();

            for (i, layer) in texture.layers.iter().enumerate() {
                ui.label(i.to_string());
                ui.label(layer.path_index.to_string());
                ui.label(layer.path.as_deref().unwrap_or("None"));
                ui.label(layer.texture_id.map(|id| id.to_string()).unwrap_or_else(|| "None".to_string()));
                ui.label(terrain_texture_type_label(layer.texture_type));
                ui.label(texture.layer_role(i).as_str());
                ui.label(layer.source_kind.to_string());
                ui.label(layer.texture_repetition.to_string());
                ui.label(layer.sampler_index.to_string());
                ui.label(layer.uv_set.to_string());
                if let Some(texture_id) = layer.texture_id {
                    if let Some(handle) = app.get_ec_texture_by_id(ctx, texture_id) {
                        ui.add(thumbnail(&handle, 128.0));
                    } else {
                        ui.label("missing");
                    }
                } else {
                    ui.label("None");
                }
                ui.end_row();
            }
        });
        ui.add_space(5.0);
        ui.collapsing("Layer Analysis", |ui| {
            if let Some((primary_idx, primary_layer)) = entry.primary_texture_layer() {
                egui::Grid::new("td_primary_analysis_grid").show(ui, |ui| {
                    ui.label("Primary layer:");
                    ui.label(primary_idx.to_string());
                    ui.end_row();
                    ui.label("Layer class:");
                    ui.label(texture.layer_class().as_str());
                    ui.end_row();
                    ui.label("Path:");
                    ui.label(primary_layer.path.as_deref().unwrap_or("None"));
                    ui.end_row();
                    ui.label("Repetition:");
                    ui.label(primary_layer.texture_repetition.to_string());
                    ui.end_row();
                });
            } else {
                ui.label("No primary layer selected.");
            }

            ui.add_space(5.0);
            for (i, layer) in texture.layers.iter().enumerate() {
                let header = format!(
                    "Layer {} — {} ({})",
                    i,
                    layer.path.as_deref().unwrap_or("no path"),
                    terrain_texture_type_label(layer.texture_type)
                );
                ui.collapsing(header, |ui| {
                    egui::Grid::new(format!("td_layer_analysis_{i}")).show(ui, |ui| {
                        ui.label("role (by position):");
                        ui.label(texture.layer_role(i).as_str());
                        ui.end_row();
                        ui.label("stretch (UV divisor):");
                        ui.label(layer.texture_repetition.to_string());
                        ui.end_row();
                    });
                    if let Some(texture_id) = layer.texture_id {
                        if let Some(handle) = app.get_ec_texture_by_id(ctx, texture_id) {
                            ui.add(egui::Image::new(&handle).texture_options(egui::TextureOptions::LINEAR));
                        } else {
                            ui.label("texture missing");
                        }
                    } else {
                        ui.label("no texture id");
                    }
                });
            }
        });
    } else {
        ui.heading("Texture Item");
        ui.label("No texture definition.");
    }
}

fn terrain_texture_type_label(texture_type: TerrainTextureType) -> &'static str {
    match texture_type {
        TerrainTextureType::Undefined => "Undefined",
        TerrainTextureType::WorldArt => "WorldArt",
        TerrainTextureType::TileArtLegacy => "TileArtLegacy",
        TerrainTextureType::TileArtEnhanced => "TileArtEnhanced",
        TerrainTextureType::Textures => "Textures",
    }
}

fn thumbnail(handle: &egui::TextureHandle, max_side: f32) -> egui::Image<'_> {
    let [width, height] = handle.size();
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = (max_side / width).min(max_side / height).min(1.0);
    egui::Image::new(handle).fit_to_exact_size(egui::vec2(width * scale, height * scale))
}

fn hex_prefix(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 3);
    for byte in bytes {
        hex.push_str(&format!("{:02X} ", byte));
    }
    hex
}
