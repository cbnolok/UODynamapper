use eframe::egui;
use crate::app::UopInspectorApp;
use uocf::enhanced::tileart::TileArtEntry;

pub fn ui_uop_browser(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("package_panel")
        .resizable(true)
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.heading("Packages");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, loaded) in app.uop_cache.loaded_uops.iter().enumerate() {
                    let name = loaded.path.file_name().unwrap_or_default().to_string_lossy();
                    if ui.selectable_label(app.selected_uop_idx == Some(i), name).clicked() {
                        app.selected_uop_idx = Some(i);
                        app.selected_file_hash = None;
                    }
                }
            });
        });

    if let Some(uop_idx) = app.selected_uop_idx {
        let loaded_uop = app.uop_cache.loaded_uops[uop_idx].clone();
        
        egui::SidePanel::left("entry_panel")
            .resizable(true)
            .default_width(350.0)
            .show(ctx, |ui| {
                ui.heading("Entries");
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut app.search_query);
                });
                ui.horizontal(|ui| {
                    ui.label("Find Hash:");
                    let res = ui.text_edit_singleline(&mut app.find_hash_query);
                    if res.changed() || ui.button("Find").clicked() {
                       if let Ok(h) = u64::from_str_radix(app.find_hash_query.trim_start_matches("0x"), 16) {
                           app.selected_file_hash = Some(h);
                       }
                    }
                });
                
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for file in loaded_uop.package.iter_files() {
                        let hash = file.filename_hash();
                        let resolved_name = app.dictionary.resolve(hash);
                        let display_name = match resolved_name {
                            Some(name) => name.clone(),
                            None => format!("{:016X}", hash),
                        };

                        if !app.search_query.is_empty() && !display_name.to_lowercase().contains(&app.search_query.to_lowercase()) {
                            continue;
                        }

                        if ui.selectable_label(app.selected_file_hash == Some(hash), display_name).clicked() {
                            app.selected_file_hash = Some(hash);
                        }
                    }
                });
            });
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        if let (Some(uop_idx), Some(file_hash)) = (app.selected_uop_idx, app.selected_file_hash) {
            let loaded_uop = app.uop_cache.loaded_uops[uop_idx].clone();
            if let Some(file) = loaded_uop.package.get_file_by_hash(file_hash) {
                let hash = file.filename_hash();
                let resolved_name = app.dictionary.resolve(hash).cloned().unwrap_or_else(|| format!("{:016X}", hash));
                
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
                if resolved_name.contains("tileart") && resolved_name.ends_with(".bin") {
                    ui_integrated_tileart_view(app, ctx, ui, file);
                } else if resolved_name.contains("terraindefinition") && resolved_name.ends_with(".bin") {
                    ui_integrated_terrain_view(app, ctx, ui, file);
                } else {
                    ui_generic_preview(app, ctx, ui, file, &resolved_name);
                }
            }
        } else {
            ui.centered_and_justified(|ui| { ui.label("Select a package and an entry"); });
        }
    });
}

fn ui_generic_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop::file::UopFile, name: &str) {
    if let Ok(data) = file.unpack() {
        if name.to_lowercase().ends_with(".dds") || name.to_lowercase().ends_with(".tga") || name.to_lowercase().ends_with(".bmp") {
            if let Some(handle) = app.get_uop_texture(ctx, file.filename_hash(), &data, name) {
                ui.label(format!("Image: {}x{}", handle.size()[0], handle.size()[1]));
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(&handle);
                });
            }
        } else if data.starts_with(b"<?xml") || name.ends_with(".xml") || name.ends_with(".def") {
            if let Ok(text) = String::from_utf8(data.clone()) {
                let mut text_ref = text.as_str();
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

fn ui_integrated_tileart_view(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop::file::UopFile) {
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
                    ui.label("Type"); ui.label(tae.type_val.to_string()); ui.end_row();
                    ui.label("FLAGS"); ui.label(format!("{:?}", tae.flags1)); ui.end_row();
                });
            });
        });
    }
}

fn ui_integrated_terrain_view(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop::file::UopFile) {
    let dict = app.uo_string_dictionary.as_ref().map(|d| &**d);
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
