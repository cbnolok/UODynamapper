use eframe::egui;
use crate::app::UopInspectorApp;

pub fn ui_hues(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("hues_list")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading("Hue Entries");
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
                    ui.label("Hues not loaded");
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
                                    // RGB555 to RGBA8888
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
                    if let Some(art_id) = app.selected_cc_art_id {
                        if let Some(handle) = app.get_cc_art_texture(ctx, art_id) {
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
