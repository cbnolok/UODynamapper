use eframe::egui;
use crate::app::UopInspectorApp;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::enhanced::waypoints::{
    WaypointClilocDefinition, WaypointTypeDefinition, WaypointsPackage,
    KNOWN_WAYPOINTS_PAYLOAD_HASH, WAYPOINTS_PAYLOAD_PATH,
};

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
        let entry_labels = app.get_uop_entry_labels(uop_idx);
        
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
                
                let query = app.search_query.to_lowercase();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for file in entry_labels.iter() {
                        if !query.is_empty() && !file.search_name.contains(&query) {
                            continue;
                        }

                        if ui.selectable_label(app.selected_file_hash == Some(file.hash), &file.display_name).clicked() {
                            app.selected_file_hash = Some(file.hash);
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
                let resolved_name = app.dictionary.resolve(hash).map(str::to_string).unwrap_or_else(|| format!("{:016X}", hash));
                
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
                    ui_integrated_waypoints_view(app, ui, file);
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

fn is_waypoint_payload(hash: u64, resolved_name: &str) -> bool {
    hash == KNOWN_WAYPOINTS_PAYLOAD_HASH
        || resolved_name.eq_ignore_ascii_case(WAYPOINTS_PAYLOAD_PATH)
        || resolved_name.eq_ignore_ascii_case("waypoint.bin")
}

fn ui_integrated_waypoints_view(
    app: &UopInspectorApp,
    ui: &mut egui::Ui,
    file: &uocf::uop_container::file::UopFile,
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
    egui::CollapsingHeader::new("Waypoint Records")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
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
        if lower_name.ends_with(".dds") || lower_name.ends_with(".tga") || lower_name.ends_with(".bmp") {
            if let Some(handle) = app.get_uop_texture(ctx, hash, &data, name) {
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
                    ui.label("Type"); ui.label(tae.type_val.to_string()); ui.end_row();
                    ui.label("FLAGS"); ui.label(format!("{:?}", tae.flags1)); ui.end_row();
                });
            });
        });
    }
}

fn ui_integrated_terrain_view(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, file: &uocf::uop_container::file::UopFile) {
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
