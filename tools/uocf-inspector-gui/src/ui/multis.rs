use crate::app::{MultisSource, UopInspectorApp};
use eframe::egui;

#[derive(Clone)]
struct PreviewPart {
    item_id: u16,
    x: i16,
    y: i16,
    z: i16,
    flags: String,
}

pub fn ui_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let has_classic = app
        .client_data
        .as_ref()
        .and_then(|client| client.multis.as_ref())
        .is_some();
    let has_uop = app.multi_collection.is_some();

    if app.multis_source == MultisSource::ClassicMul && !has_classic && has_uop {
        app.multis_source = MultisSource::Uop;
    } else if app.multis_source == MultisSource::Uop && !has_uop && has_classic {
        app.multis_source = MultisSource::ClassicMul;
    }

    egui::TopBottomPanel::top("multis_source_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if has_classic {
                ui.selectable_value(&mut app.multis_source, MultisSource::ClassicMul, "multi.mul/.idx");
            }
            if has_uop {
                ui.selectable_value(&mut app.multis_source, MultisSource::Uop, "MultiCollection.uop");
            }
        });
    });

    match app.multis_source {
        MultisSource::ClassicMul => ui_classic_multis(app, ctx),
        MultisSource::Uop => ui_uop_multis(app, ctx),
    }
}

fn ui_classic_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("classic_multi_sidebar")
        .resizable(true)
        .default_width(250.0)
        .show(ctx, |ui| {
            ui.heading("multi.mul/.idx");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("ID:");
                ui.add(egui::DragValue::new(&mut app.selected_multi_id));
            });

            if let Some(client) = &app.client_data {
                if let Some(multis) = &client.multis {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for id in 0..multis.max_id() {
                            if ui
                                .selectable_label(app.selected_multi_id == id, format!("Multi {}", id))
                                .clicked()
                            {
                                app.selected_multi_id = id;
                            }
                        }
                    });
                }
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        let parts = app
            .client_data
            .as_ref()
            .and_then(|client| client.multis.as_ref())
            .and_then(|multis| multis.get_parts(app.selected_multi_id).ok())
            .unwrap_or_default();
        let preview_parts: Vec<_> = parts
            .iter()
            .map(|part| PreviewPart {
                item_id: part.item_id,
                x: part.x,
                y: part.y,
                z: part.z,
                flags: format!("0x{:08X}", part.flags),
            })
            .collect();
        draw_multi_details(app, ctx, ui, "Classic Multi", &preview_parts, None);
    });
}

fn ui_uop_multis(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(collection) = app.multi_collection.clone() else {
        return;
    };

    egui::SidePanel::left("uop_multi_sidebar")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading(format!("MultiCollection.uop ({})", collection.items.len()));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            let query = app.search_query.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for item in &collection.items {
                    if !query.is_empty()
                        && !item.id.to_string().contains(&query)
                        && !item.path.to_lowercase().contains(&query)
                        && !format!("{:016X}", item.filename_hash).to_lowercase().contains(&query)
                    {
                        continue;
                    }

                    if ui
                        .selectable_label(
                            app.selected_multi_id == item.id,
                            format!("Multi {} ({:016X})", item.id, item.filename_hash),
                        )
                        .clicked()
                    {
                        app.selected_multi_id = item.id;
                        app.selected_multi_uop_hash = Some(item.filename_hash);
                    }
                }
            });
        });

    egui::TopBottomPanel::top("multi_uop_raw_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.multis_source, MultisSource::Uop, "Specialized");
            if ui.button("Raw selected").clicked() {
                app.select_raw_multi_collection_entry(app.selected_multi_id);
            }
            if collection.housing_hash.is_some() && ui.button("Raw housing").clicked() {
                app.select_raw_multi_collection_housing();
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(item) = collection.get(app.selected_multi_id) {
            let preview_parts: Vec<_> = item
                .parts
                .iter()
                .map(|part| PreviewPart {
                    item_id: part.item_id,
                    x: part.x,
                    y: part.y,
                    z: part.z,
                    flags: format!(
                        "0x{:04X} / {} clilocs",
                        part.flags,
                        part.cliloc_offsets.len()
                    ),
                })
                .collect();
            let raw = Some(format!("{} / 0x{:016X}", item.path, item.filename_hash));
            draw_multi_details(app, ctx, ui, "UOP Multi", &preview_parts, raw.as_deref());
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a MultiCollection.uop entry from the left panel.");
            });
        }
    });
}

fn draw_multi_details(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    title: &str,
    parts: &[PreviewPart],
    raw_label: Option<&str>,
) {
    if parts.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(format!("Multi {} is empty or missing", app.selected_multi_id));
        });
        return;
    }

    ui.heading(format!("{} {}: {} parts", title, app.selected_multi_id, parts.len()));
    if let Some(raw_label) = raw_label {
        ui.monospace(raw_label);
    }
    ui.separator();

    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading("Components");
            egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                egui::Grid::new("multi_parts_grid").striped(true).show(ui, |ui| {
                    ui.label("Item ID");
                    ui.label("X");
                    ui.label("Y");
                    ui.label("Z");
                    ui.label("Flags");
                    ui.end_row();
                    for part in parts {
                        ui.label(part.item_id.to_string());
                        ui.label(part.x.to_string());
                        ui.label(part.y.to_string());
                        ui.label(part.z.to_string());
                        ui.label(&part.flags);
                        ui.end_row();
                    }
                });
            });
        });

        ui.separator();

        ui.vertical(|ui| {
            ui.heading("2D Preview");
            draw_preview(app, ctx, ui, parts);
        });
    });
}

fn draw_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, parts: &[PreviewPart]) {
    let (min_x, max_x, min_y, max_y) = parts.iter().fold(
        (0i16, 0i16, 0i16, 0i16),
        |(min_x, max_x, min_y, max_y), p| {
            (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
        },
    );
    let width = (max_x - min_x + 1).max(1) as f32;
    let height = (max_y - min_y + 1).max(1) as f32;
    let cell_size = 20.0f32;
    let canvas_size = egui::vec2(width * cell_size, height * cell_size);

    egui::ScrollArea::both().show(ui, |ui| {
        let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(30));

        let mut sorted_parts = parts.to_vec();
        sorted_parts.sort_by(|a, b| a.z.cmp(&b.z).then(a.y.cmp(&b.y)).then(a.x.cmp(&b.x)));

        for part in sorted_parts {
            let x = (part.x - min_x) as f32 * cell_size;
            let y = (part.y - min_y) as f32 * cell_size;
            let part_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(x, y),
                egui::vec2(cell_size, cell_size),
            );

            let art_id = part.item_id as u32 + 0x4000;
            if let Some(handle) = app.get_tex_art_cc_texture(ctx, art_id) {
                painter.image(
                    handle.id(),
                    part_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                painter.rect_filled(part_rect.shrink(1.0), 2.0, egui::Color32::BLUE);
            }
        }
    });
}
