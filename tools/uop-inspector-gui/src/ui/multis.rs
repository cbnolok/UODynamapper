use eframe::egui;
use crate::app::UopInspectorApp;

pub fn ui_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("multi_sidebar")
        .resizable(true)
        .default_width(250.0)
        .show(ctx, |ui| {
            ui.heading("Multis");
            ui.separator();
            
            ui.horizontal(|ui| {
                ui.label("Search ID:");
                ui.add(egui::DragValue::new(&mut app.selected_multi_id));
            });

            if let Some(client) = &app.client_data {
                if let Some(multis) = &client.multis {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for id in 0..multis.max_id() {
                            // In classic, we don't always know if a multi exists without checking the index
                            // But we can try to get parts and see if it's empty
                            if ui.selectable_label(app.selected_multi_id == id, format!("Multi {}", id)).clicked() {
                                app.selected_multi_id = id;
                            }
                        }
                    });
                } else {
                    ui.label("No classic multis loaded");
                }
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(client) = &app.client_data {
            if let Some(multis) = &client.multis {
                match multis.get_parts(app.selected_multi_id) {
                    Ok(parts) => {
                        if parts.is_empty() {
                            ui.centered_and_justified(|ui| { ui.label(format!("Multi {} is empty or doesn't exist", app.selected_multi_id)); });
                        } else {
                            ui.heading(format!("Multi {}: {} parts", app.selected_multi_id, parts.len()));
                            
                            ui.horizontal(|ui| {
                                // List of parts
                                ui.vertical(|ui| {
                                    ui.heading("Components");
                                    egui::ScrollArea::vertical().show(ui, |ui| {
                                        egui::Grid::new("multi_parts_grid").striped(true).show(ui, |ui| {
                                            ui.label("Item ID");
                                            ui.label("X");
                                            ui.label("Y");
                                            ui.label("Z");
                                            ui.end_row();
                                            for part in &parts {
                                                ui.label(part.item_id.to_string());
                                                ui.label(part.x.to_string());
                                                ui.label(part.y.to_string());
                                                ui.label(part.z.to_string());
                                                ui.end_row();
                                            }
                                        });
                                    });
                                });

                                ui.separator();

                                // Visual Preview (Simple 2D layout)
                                ui.vertical(|ui| {
                                    ui.heading("2D Preview");
                                    let (min_x, max_x, min_y, max_y) = parts.iter().fold((0i16, 0i16, 0i16, 0i16), |(min_x, max_x, min_y, max_y), p| {
                                        (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
                                    });
                                    
                                    let width = (max_x - min_x + 1) as f32;
                                    let height = (max_y - min_y + 1) as f32;
                                    
                                    let cell_size = 20.0f32;
                                    let canvas_size = egui::vec2(width * cell_size, height * cell_size);
                                    
                                    egui::ScrollArea::both().show(ui, |ui| {
                                        let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
                                        
                                        let painter = ui.painter_at(rect);
                                        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(30));

                                        // Sort parts by Z then Y to get some depth feel
                                        let mut sorted_parts = parts.clone();
                                        sorted_parts.sort_by(|a, b| a.z.cmp(&b.z).then(a.y.cmp(&b.y)));

                                        for part in sorted_parts {
                                            let x = (part.x - min_x) as f32 * cell_size;
                                            let y = (part.y - min_y) as f32 * cell_size;
                                            let part_rect = egui::Rect::from_min_size(
                                                rect.min + egui::vec2(x, y),
                                                egui::vec2(cell_size, cell_size)
                                            );
                                            
                                            // Try to render the actual sprite if available
                                            let art_id = part.item_id as u32 + 0x4000;
                                            if let Some(handle) = app.get_cc_art_texture(ctx, art_id) {
                                                painter.image(handle.id(), part_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                                            } else {
                                                painter.rect_filled(part_rect.shrink(1.0), 2.0, egui::Color32::BLUE);
                                            }
                                        }
                                    });
                                });
                            });
                        }
                    }
                    Err(e) => {
                        ui.label(format!("Error loading multi: {}", e));
                    }
                }
            }
        } else {
            ui.centered_and_justified(|ui| { ui.label("Load client data first"); });
        }
    });
}
