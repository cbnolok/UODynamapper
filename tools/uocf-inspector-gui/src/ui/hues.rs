use crate::app::{HuesSource, UopInspectorApp};
use eframe::egui;
use uocf::enhanced::hues::{
    atlas_coord_for_hue, hue_bitmap_path, HUES_ATLAS_PATH, HUENAMES_PATH, FIXED_PALETTE_HASH,
    FIXED_PALETTE_NAME, MAX_EC_HUES,
};

pub fn ui_hues(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let has_cc_hues = app
        .client_data
        .as_ref()
        .and_then(|client| client.hues.as_ref())
        .is_some();
    let has_ec_hues = app.ec_hues.is_some();

    if app.hues_source == HuesSource::CcMul && !has_cc_hues && has_ec_hues {
        app.hues_source = HuesSource::EcUop;
    } else if app.hues_source == HuesSource::EcUop && !has_ec_hues && has_cc_hues {
        app.hues_source = HuesSource::CcMul;
    }

    egui::TopBottomPanel::top("hues_file_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if has_cc_hues {
                ui.selectable_value(&mut app.hues_source, HuesSource::CcMul, "hues.mul");
            }
            if has_ec_hues {
                ui.selectable_value(&mut app.hues_source, HuesSource::EcUop, "hues.uop");
            }
        });
    });

    match app.hues_source {
        HuesSource::CcMul => ui_cc_hues(app, ctx),
        HuesSource::EcUop => ui_ec_hues(app, ctx),
    }
}

fn ui_cc_hues(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("cc_hues_list")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading("hues.mul Entries");
            ui.separator();

            if let Some(client) = &app.client_data {
                if let Some(hues) = &client.hues {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, hue) in hues.iter().enumerate() {
                            let hue_id = (i + 1) as u16;
                            let name = String::from_utf8_lossy(&hue.name).trim_matches('\0').to_string();
                            let label = if name.is_empty() {
                                format!("Hue {}", hue_id)
                            } else {
                                format!("Hue {}: {}", hue_id, name)
                            };

                            if ui.selectable_label(app.selected_hue_id == hue_id, label).clicked() {
                                app.selected_hue_id = hue_id;
                            }
                        }
                    });
                } else {
                    ui.label("hues.mul not loaded");
                }
            } else {
                ui.label("Load client data first");
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(client) = &app.client_data {
            if let Some(hues) = &client.hues {
                let idx = (app.selected_hue_id as usize).saturating_sub(1);
                if let Some(hue) = hues.get(idx) {
                    ui.heading(format!("Hue {}: {}", app.selected_hue_id, String::from_utf8_lossy(&hue.name).trim_matches('\0')));
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Color Table (32 colors):");
                            ui.spacing_mut().item_spacing.y = 2.0;

                            egui::Grid::new("hue_colors").num_columns(8).show(ui, |ui| {
                                for (i, &color) in hue.color_table.iter().enumerate() {
                                    let r = (((color >> 10) & 0x1F) << 3) as u8;
                                    let g = (((color >> 5) & 0x1F) << 3) as u8;
                                    let b = ((color & 0x1F) << 3) as u8;

                                    let (rect, _response) = ui.allocate_at_least(egui::vec2(24.0, 24.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));

                                    if (i + 1) % 8 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui.label("Properties:");
                            ui.label(format!("Start: {}", hue.table_start));
                            ui.label(format!("End: {}", hue.table_end));
                        });
                    });

                    ui.separator();
                    ui.label("Preview on last selected CC Art:");
                    if let Some(art_id) = app.selected_tex_art_cc_id {
                        if let Some(handle) = app.get_tex_art_cc_texture(ctx, art_id) {
                            ui.image(&handle);
                        }
                    } else {
                        ui.label("(Select an art tile in CC Art tab first)");
                    }
                }
            }
        }
    });
}

fn ui_ec_hues(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(ec_hues_arc) = app.ec_hues.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select an Enhanced Client path containing hues.uop.");
            });
        });
        return;
    };
    let ec_hues = ec_hues_arc.as_ref();

    egui::SidePanel::left("ec_hues_list")
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.heading(format!("hues.uop Entries ({})", ec_hues.bitmaps.len()));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            let query = app.search_query.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for hue_id in 1..=MAX_EC_HUES {
                    let name = ec_hues.hue_name(hue_id).unwrap_or("");
                    let path = hue_bitmap_path(hue_id);
                    if !query.is_empty()
                        && !hue_id.to_string().contains(&query)
                        && !name.to_lowercase().contains(&query)
                        && !path.to_lowercase().contains(&query)
                    {
                        continue;
                    }

                    let label = if name.is_empty() {
                        format!("Hue {}", hue_id)
                    } else {
                        format!("Hue {}: {}", hue_id, name)
                    };

                    if ui
                        .selectable_label(app.selected_ec_hue_id == hue_id, label)
                        .clicked()
                    {
                        app.selected_ec_hue_id = hue_id;
                        app.selected_ec_hue_hash =
                            ec_hues.bitmap_for_hue(hue_id).map(|entry| entry.filename_hash);
                    }
                }
            });
        });

    egui::TopBottomPanel::top("ec_hues_raw_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.hues_source, HuesSource::EcUop, "Specialized");
            if ui.button("Raw selected BMP").clicked() {
                app.select_raw_ec_hue_bitmap(app.selected_ec_hue_id);
            }
            if ui.button("Raw atlas").clicked() {
                app.select_raw_ec_hues_atlas();
            }
            if ui.button("Raw names").clicked() {
                app.select_raw_ec_huenames();
            }
            if ui.button("Raw palette").clicked() {
                app.select_raw_ec_hue_palette();
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("EC Hues");
                ui.separator();
                ui.label(format!("{} bitmap entries", ec_hues.bitmaps.len()));
                ui.label(format!("{} names", ec_hues.names.len()));
            });
            ui.separator();

            let hue_id = app.selected_ec_hue_id.clamp(1, MAX_EC_HUES);
            if app.selected_ec_hue_id != hue_id {
                app.selected_ec_hue_id = hue_id;
            }

            ui.heading(format!(
                "Hue {}{}",
                hue_id,
                ec_hues
                    .hue_name(hue_id)
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(": {}", name))
                    .unwrap_or_default()
            ));

            egui::Grid::new("ec_hue_details").striped(true).show(ui, |ui| {
                ui.label("BMP path");
                ui.monospace(hue_bitmap_path(hue_id));
                ui.end_row();

                if let Some(entry) = ec_hues.bitmap_for_hue(hue_id) {
                    ui.label("BMP hash");
                    ui.monospace(format!("0x{:016X}", entry.filename_hash));
                    ui.end_row();
                    ui.label("BMP bytes");
                    ui.label(entry.byte_len.to_string());
                    ui.end_row();
                }

                if let Some(coord) = atlas_coord_for_hue(hue_id) {
                    ui.label("Atlas strip");
                    ui.label(format!(
                        "column {}, row {}, x {}, y {}",
                        coord.column, coord.row, coord.x, coord.y
                    ));
                    ui.end_row();
                }

                if let Some(hash) = ec_hues.atlas_hash {
                    ui.label("Atlas hash");
                    ui.monospace(format!("0x{:016X}", hash));
                    ui.end_row();
                }

                if let Some(hash) = ec_hues.huenames_hash {
                    ui.label("Names hash");
                    ui.monospace(format!("0x{:016X}", hash));
                    ui.end_row();
                }

                ui.label("Fixed palette hash");
                ui.monospace(format!("0x{:016X}", FIXED_PALETTE_HASH));
                ui.end_row();
            });

            ui.separator();
            ui.label("Selected hue BMP:");
            if let Some(entry) = ec_hues.bitmap_for_hue(hue_id) {
                if let Some(handle) = app.get_hues_uop_texture(ctx, entry.filename_hash, &entry.path) {
                    ui.add(egui::Image::new(&handle).fit_to_exact_size(egui::vec2(512.0, 32.0)));
                } else {
                    ui.label("Preview unavailable");
                }
            } else {
                ui.label("BMP entry not present in hues.uop");
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(HUES_ATLAS_PATH);
                    if let Some(hash) = ec_hues.atlas_hash {
                        if let Some(handle) = app.get_hues_uop_texture(ctx, hash, HUES_ATLAS_PATH) {
                            ui.add(egui::Image::new(&handle).fit_to_exact_size(egui::vec2(256.0, 256.0)));
                        } else {
                            ui.label("Preview unavailable");
                        }
                    } else {
                        ui.label("Not present");
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.label(FIXED_PALETTE_NAME);
                    if let Some(hash) = ec_hues.fixed_palette_hash {
                        if let Some(handle) = app.get_hues_uop_texture(ctx, hash, FIXED_PALETTE_NAME) {
                            ui.add(egui::Image::new(&handle).fit_to_exact_size(egui::vec2(256.0, 256.0)));
                        } else {
                            ui.label("Preview unavailable");
                        }
                    } else {
                        ui.label("Not present");
                    }
                });
            });

            ui.separator();
            ui.label(HUENAMES_PATH);
            ui.label(format!(
                "{}",
                ec_hues
                    .huenames_byte_len
                    .map(|len| format!("{len} bytes"))
                    .unwrap_or_else(|| "Not present".to_string())
            ));
        });
    });
}
