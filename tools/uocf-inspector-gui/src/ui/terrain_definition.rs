use crate::app::{TerrainDefinitionFileEntry, UopInspectorApp};
use crate::ui::{list_sort_controls, sorted_indices_by};
use eframe::egui;
use uocf::enhanced::terrain_definition::TerrainDefinitionEntry;
use std::collections::BTreeSet;

const TERRAIN_DEFINITION_PANEL_WIDTH: f32 = 300.0;

pub fn ui_terrain_definition(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let files_arc = if let Some(ref files) = app.terrain_def_files {
        files.clone()
    } else {
        return;
    };
    let files = files_arc.as_ref();

    egui::Panel::left("terrain_def_panel")
        .resizable(true)
        .default_size(TERRAIN_DEFINITION_PANEL_WIDTH)
        .show_inside(ui, |ui| {
            ui.heading(format!("Terrain Definition Files ({})", files.len()));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            let sort = list_sort_controls(ui, "terrain_definition_files", &["ID", "Name", "Hash", "Bytes"], 0);
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
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
                for index in sorted_indices {
                    let file = &files[index];
                    let entry = &file.entry;
                    let name = entry.name.as_deref().unwrap_or("Unknown");

                    if !app.search_query.is_empty() {
                        if !name.to_lowercase().contains(&app.search_query.to_lowercase()) &&
                           !entry.id.to_string().contains(&app.search_query) &&
                           !format!("{:016X}", file.filename_hash).to_lowercase().contains(&app.search_query.to_lowercase()) {
                            continue;
                        }
                    }

                    let is_selected = app.selected_terrain_def_hash == Some(file.filename_hash);
                    let label = format!("[{}] {} ({:016X})", entry.id, name, file.filename_hash);

                    if ui.selectable_label(is_selected, label).clicked() {
                        app.selected_terrain_def_hash = Some(file.filename_hash);
                    }
                }
            });
        });

    egui::Panel::top("terrain_definition_tabs").show_inside(ui, |ui| {
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
        });
    });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(selected_hash) = app.selected_terrain_def_hash {
            if let Some(file) = files.iter().find(|file| file.filename_hash == selected_hash) {
                egui::ScrollArea::vertical().show(ui, |ui| {
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
    app: &mut UopInspectorApp,
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
        ui.label("name_id");
        ui.label(entry.name_id.to_string());
        ui.end_row();
        ui.label("id");
        ui.label(entry.id.to_string());
        ui.end_row();
        ui.label("unk");
        ui.label(entry.unk.to_string());
        ui.end_row();
        ui.label("unk2");
        ui.label(entry.unk2.to_string());
        ui.end_row();
        ui.label("unk3");
        ui.label(entry.unk3.to_string());
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
            ui.label("count_index");
            ui.label("alias");
            ui.label("tile_flags");
            ui.end_row();

            for alias in &entry.aliases {
                ui.label(alias.count_index.to_string());
                ui.label(format!("0x{:04X} ({})", alias.alias, alias.alias));
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
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &TerrainDefinitionEntry,
) {
    ui.heading("Linked CC Land Art");

    let mut aliases = entry
        .aliases
        .iter()
        .map(|alias| alias.alias)
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
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &TerrainDefinitionEntry,
) {
    if let Some(texture) = &entry.texture {
        ui.heading("Texture Item");
        egui::Grid::new("td_texture_header_grid").striped(true).show(ui, |ui| {
            ui.label("unk1");
            ui.label(texture.unk1.to_string());
            ui.end_row();
            ui.label("shader_name_id");
            ui.label(texture.shader_name_id.to_string());
            ui.end_row();
            ui.label("shader_name");
            ui.label(texture.shader_name.as_deref().unwrap_or("None"));
            ui.end_row();
            ui.label("layer_count");
            ui.label(texture.layers.len().to_string());
            ui.end_row();
            ui.label("unk8");
            ui.label(format!("{:?}", texture.unk8));
            ui.end_row();
            ui.label("unk9");
            ui.label(format!("{:?}", texture.unk9));
            ui.end_row();
        });

        ui.add_space(5.0);
        ui.heading("Texture Layers");
        egui::Grid::new("td_layers_grid").striped(true).show(ui, |ui| {
            ui.label("#");
            ui.label("name_string_off");
            ui.label("path");
            ui.label("texture_id");
            ui.label("type");
            ui.label("unk4");
            ui.label("repetition");
            ui.label("unk6");
            ui.label("unk7");
            ui.label("preview");
            ui.end_row();

            for (i, layer) in texture.layers.iter().enumerate() {
                ui.label(i.to_string());
                ui.label(layer.name_string_off.to_string());
                ui.label(layer.path.as_deref().unwrap_or("None"));
                ui.label(layer.texture_id.map(|id| id.to_string()).unwrap_or_else(|| "None".to_string()));
                ui.label(format!("{:?}", layer.texture_type));
                ui.label(layer.unk4.to_string());
                ui.label(layer.texture_repetition.to_string());
                ui.label(layer.unk6.to_string());
                ui.label(layer.unk7.to_string());
                if let Some(texture_id) = layer.texture_id {
                    if let Some(handle) = app.get_ec_texture_by_id(ctx, texture_id) {
                        ui.add(thumbnail(&handle, 96.0));
                    } else {
                        ui.label("missing");
                    }
                } else {
                    ui.label("None");
                }
                ui.end_row();
            }
        });
    } else {
        ui.heading("Texture Item");
        ui.label("No texture definition.");
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
