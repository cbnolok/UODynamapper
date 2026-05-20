use crate::app::UopInspectorApp;
use eframe::egui;

pub fn ui_terrain_definition(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let pkg_arc = if let Some(ref pkg) = app.terrain_def_package {
        pkg.clone()
    } else {
        return;
    };
    
    let pkg = pkg_arc.as_ref();

    egui::SidePanel::left("terrain_def_panel")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading(format!("Terrain Definitions ({})", pkg.entries.len()));
            ui.separator();
            
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in &pkg.entries {
                    let name = entry.name.as_deref().unwrap_or("Unknown");
                    
                    if !app.search_query.is_empty() {
                        if !name.to_lowercase().contains(&app.search_query.to_lowercase()) && 
                           !entry.id.to_string().contains(&app.search_query) {
                            continue;
                        }
                    }

                    let is_selected = app.selected_tex_art_cc_id == Some(entry.id);
                    let label = format!("[{}] {}", entry.id, name);
                    
                    if ui.selectable_label(is_selected, label).clicked() {
                        app.selected_tex_art_cc_id = Some(entry.id);
                    }
                }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(selected_id) = app.selected_tex_art_cc_id {
            if let Some(entry) = pkg.entries.iter().find(|e| e.id == selected_id) {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(format!("Terrain Definition: {} ({})", entry.name.as_deref().unwrap_or("Unknown"), entry.id));
                    ui.separator();

                    ui.label(format!("Name ID: {}", entry.name_id));
                    
                    ui.add_space(10.0);
                    ui.heading("Aliases (Land Tiles)");
                    if entry.aliases.is_empty() {
                        ui.label("No aliases.");
                    } else {
                        egui::Grid::new("td_aliases_grid").striped(true).show(ui, |ui| {
                            ui.label("Count Index");
                            ui.label("Alias (Tile ID)");
                            ui.label("Tile Flags");
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
                    if let Some(texture) = &entry.texture {
                        ui.heading("Texture Material");
                        ui.label(format!("Shader: {}", texture.shader_name.as_deref().unwrap_or("None")));
                        
                        ui.add_space(5.0);
                        ui.heading(format!("Layers ({})", texture.layers.len()));
                        for (i, layer) in texture.layers.iter().enumerate() {
                            ui.group(|ui| {
                                ui.strong(format!("Layer {}", i));
                                ui.label(format!("Path: {}", layer.path.as_deref().unwrap_or("None")));
                                ui.label(format!("Texture ID: {:?}", layer.texture_id));
                                ui.label(format!("Type: {:?}", layer.texture_type));
                                ui.label(format!("Repetition: {}", layer.texture_repetition));
                            });
                        }
                    } else {
                        ui.label("No texture definition.");
                    }
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a terrain definition from the left panel.");
            });
        }
    });
}
