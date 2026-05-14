use eframe::egui;
use crate::app::{UopInspectorApp, ArtSource, ViewMode};

pub fn ui_art_viewer(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("tex_art_cc_list")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading("CC Art & TileData");
            
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("art_source_combo")
                    .selected_text(format!("{:?}", app.selected_legacy_source))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Any, "Any (Auto)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::Mul, "Legacy MUL (.mul)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP (artLegacyMUL)");
                        ui.selectable_value(&mut app.selected_legacy_source, ArtSource::EcUop, "EC UOP (LegacyTexture)");
                    });
            });

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
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
            
            if let Some(client) = &app.client_data {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.collapsing("Land Tiles", |ui| {
                        for id in 0..0x4000 {
                            if !client.art.has_id(id) && !client.tiledata.land_tiles().get(id as usize).is_some() { continue; }
                            
                            let tile_name = client.tiledata.land_tiles().get(id as usize).map(|t| t.name_ascii()).unwrap_or_default();
                            if !app.search_query.is_empty() && !id.to_string().contains(&app.search_query) && !tile_name.to_lowercase().contains(&app.search_query.to_lowercase()) {
                                continue;
                            }

                            if ui.selectable_label(app.selected_tex_art_cc_id == Some(id), format!("{}: {}", id, tile_name)).clicked() {
                                app.selected_tex_art_cc_id = Some(id);
                                app.view_mode = ViewMode::TexArtCc;
                            }
                        }
                    });

                    ui.collapsing("Static Tiles", |ui| {
                        let max_id = client.art.max_id().max(0x4000 + 32768);
                        for id in 0x4000..max_id {
                            let item_id = id - 0x4000;
                            if !client.art.has_id(id) && !client.tiledata.item_tiles().get(item_id as usize).is_some() { continue; }
                            
                            let tile_name = client.tiledata.item_tiles().get(item_id as usize).map(|t| t.name_ascii()).unwrap_or_default();
                            if !app.search_query.is_empty() && !id.to_string().contains(&app.search_query) && !tile_name.to_lowercase().contains(&app.search_query.to_lowercase()) {
                                continue;
                            }

                            if ui.selectable_label(app.selected_tex_art_cc_id == Some(id), format!("{}: {}", id, tile_name)).clicked() {
                                app.selected_tex_art_cc_id = Some(id);
                                app.view_mode = ViewMode::TexArtCc;
                            }
                        }
                    });
                });
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if app.view_mode == ViewMode::CcTileData {
            ui_cc_tiledata(app, ctx, ui);
        } else if let Some(id) = app.selected_tex_art_cc_id {
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
                        if id < 0x4000 {
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
                            let item_id = id - 0x4000;
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

fn ui_cc_tiledata(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("CC TileData Inspector");
    if let Some(client) = &app.client_data {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("tiledata_grid").striped(true).show(ui, |ui| {
                ui.label("ID");
                ui.label("Type");
                ui.label("Name");
                ui.label("Flags");
                ui.label("Height/Tex");
                ui.end_row();
                
                for tile in client.tiledata.land_tiles().iter().take(1000) { 
                    ui.label(tile.tile_id.to_string());
                    ui.label("Land");
                    ui.label(tile.name_ascii());
                    ui.label(format!("{:08X}", tile.flags.internal_flags));
                    ui.label(tile.texture_id.to_string());
                    ui.end_row();
                }
            });
        });
    }
}
