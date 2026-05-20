use eframe::egui;
use crate::app::{UopInspectorApp, ArtSource, TileMetadataSource, ViewMode};
use uocf::enhanced::tileart::{PropertyKey, TileArtEntry};

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
                ui.selectable_value(&mut app.view_mode, ViewMode::TexArtCc, "Specialized");
                if ui.button("Raw UOP").clicked() {
                    app.select_raw_art_entry(id, app.selected_legacy_source);
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

    ui.horizontal(|ui| {
        ui.label("Search:");
        ui.text_edit_singleline(&mut app.search_query);
    });
    ui.separator();

    match app.tile_metadata_source {
        TileMetadataSource::CcTileData => ui_cc_tiledata_table(app, ui),
        TileMetadataSource::EcTileArt => ui_ec_tileart_table(app, ui),
    }
}

fn ui_cc_tiledata_table(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.heading("CC TileData Inspector");
    if let Some(tiledata) = &app.cc_tiledata {
        let query = app.search_query.to_lowercase();
        ui.label(format!(
            "{} land tiles, {} item tiles",
            tiledata.land_tiles().len(),
            tiledata.item_tiles().len()
        ));
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("tiledata_grid").striped(true).show(ui, |ui| {
                ui.label("ID");
                ui.label("Type");
                ui.label("Name");
                ui.label("Flags");
                ui.label("Height/Tex");
                ui.end_row();
                
                for tile in tiledata.land_tiles() {
                    if !tiledata_row_matches(&query, tile.tile_id, tile.name_ascii()) {
                        continue;
                    }
                    ui.label(tile.tile_id.to_string());
                    ui.label("Land");
                    ui.label(tile.name_ascii());
                    ui.label(format!("{:08X}", tile.flags.internal_flags));
                    ui.label(tile.texture_id.to_string());
                    ui.end_row();
                }

                for tile in tiledata.item_tiles() {
                    if !tiledata_row_matches(&query, tile.tile_id, tile.name_ascii()) {
                        continue;
                    }
                    ui.label(tile.tile_id.to_string());
                    ui.label("Item");
                    ui.label(tile.name_ascii());
                    ui.label(format!("{:08X}", tile.flags.internal_flags));
                    ui.label(tile.height_raw().to_string());
                    ui.end_row();
                }
            });
        });
    } else {
        ui.label("Select a Classic Client path containing tiledata.mul.");
    }
}

fn ui_ec_tileart_table(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.heading("EC TileArt Inspector");
    if let Some(entries) = app.ec_tileart_entries.clone() {
        let query = app.search_query.to_lowercase();
        ui.label(format!("{} tileart.uop entries", entries.len()));
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("tileart_grid").striped(true).show(ui, |ui| {
                ui.label("ID");
                ui.label("Old ID");
                ui.label("Type");
                ui.label("Height");
                ui.label("Flags");
                ui.label("EC Rect");
                ui.label("CC Rect");
                ui.label("Textures");
                ui.end_row();

                for file in entries.iter() {
                    let entry = &file.entry;
                    if !tileart_row_matches(&query, entry) {
                        continue;
                    }

                    let texture_summary = tileart_texture_summary(app, entry);
                    if ui
                        .selectable_label(
                            app.selected_tileart_hash == Some(file.filename_hash),
                            entry.tile_id.to_string(),
                        )
                        .clicked()
                    {
                        app.selected_tileart_hash = Some(file.filename_hash);
                    }
                    ui.label(entry.old_id.to_string());
                    ui.label(tileart_type_name(entry.type_val));
                    ui.label(tileart_property(entry, PropertyKey::Height).unwrap_or(0).to_string());
                    ui.label(format!("{:016X}", entry.flags1.bits()));
                    ui.label(format!(
                        "{},{} -> {},{} off {},{}",
                        entry.ec_img_offset.x_start,
                        entry.ec_img_offset.y_start,
                        entry.ec_img_offset.x_end,
                        entry.ec_img_offset.y_end,
                        entry.ec_img_offset.x_off,
                        entry.ec_img_offset.y_off
                    ));
                    ui.label(format!(
                        "{},{} -> {},{} off {},{}",
                        entry.cc_img_offset.x_start,
                        entry.cc_img_offset.y_start,
                        entry.cc_img_offset.x_end,
                        entry.cc_img_offset.y_end,
                        entry.cc_img_offset.x_off,
                        entry.cc_img_offset.y_off
                    ));
                    ui.label(texture_summary);
                    ui.end_row();
                }
            });
        });
    } else {
        ui.label("Select an Enhanced Client path containing tileart.uop.");
    }
}

fn tiledata_row_matches(query: &str, id: i32, name: &str) -> bool {
    query.is_empty() || id.to_string().contains(query) || name.to_lowercase().contains(query)
}

fn tileart_row_matches(query: &str, entry: &TileArtEntry) -> bool {
    query.is_empty()
        || entry.tile_id.to_string().contains(query)
        || entry.old_id.to_string().contains(query)
        || tileart_type_name(entry.type_val).to_lowercase().contains(query)
        || format!("{:016X}", entry.flags1.bits()).to_lowercase().contains(query)
}

fn tileart_type_name(type_val: i32) -> &'static str {
    match type_val {
        0 => "Static",
        1 => "Solid",
        2 => "Liquid",
        _ => "Unknown",
    }
}

fn tileart_property(entry: &TileArtEntry, key: PropertyKey) -> Option<u32> {
    entry
        .prop_vector1
        .iter()
        .chain(entry.prop_vector2.iter())
        .find(|prop| prop.id == key as u8)
        .map(|prop| prop.val)
}

fn tileart_texture_summary(app: &UopInspectorApp, entry: &TileArtEntry) -> String {
    if let Some(dict) = app.uo_string_dictionary.as_ref() {
        let art_data = entry.process(dict);
        let mut parts = Vec::new();
        for block in art_data.texture_items {
            for item in block {
                parts.push(format!("{}:{:?}:{}", item.id, item.texture_type, item.path));
            }
        }
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }

    let mut parts = Vec::new();
    for (block_index, block) in entry.texture_vector.iter().enumerate() {
        if block.has_texture == 1 {
            parts.push(format!("block {}: {} refs", block_index, block.texture_items_count));
        }
    }
    parts.join(" | ")
}
