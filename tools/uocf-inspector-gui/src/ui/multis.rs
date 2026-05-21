use crate::app::{MultisSource, UopInspectorApp};
use eframe::egui;

#[derive(Clone)]
struct PreviewPart {
    item_id: u16,
    x: i16,
    y: i16,
    z: i16,
    original_z: i16,
    flags: String,
}

#[derive(Clone, Copy, Default)]
struct PartRenderInfo {
    is_floor: bool,
    width_delta: i32,
    height_delta: i32,
    offset_x: i32,
    offset_y: i32,
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
                original_z: part.z,
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
                    original_z: part.z,
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
    let (min_z, max_z) = parts.iter().fold((0i16, 0i16), |(min_z, max_z), p| {
        (min_z.min(p.z), max_z.max(p.z))
    });
    let span_x = (max_x - min_x + 1).max(1) as f32;
    let span_y = (max_y - min_y + 1).max(1) as f32;
    let tile_w = 22.0f32;
    let tile_h = 22.0f32;
    let canvas_w = (span_x * tile_w + span_y * tile_w + 360.0).max(700.0);
    let canvas_h = ((span_x + span_y) * tile_h + ((max_z - min_z).max(0) as f32 * 4.0) + 360.0).max(520.0);
    let canvas_size = egui::vec2(canvas_w, canvas_h);

    egui::ScrollArea::both().show(ui, |ui| {
        let (rect, _response) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));

        let mut sorted_parts = parts.to_vec();
        sorted_parts.sort_by(|a, b| {
            let a_info = part_render_info(app, a.item_id);
            let b_info = part_render_info(app, b.item_id);
            a.z.cmp(&b.z)
                .then(a.y.cmp(&b.y))
                .then(a.x.cmp(&b.x))
                .then((!a_info.is_floor).cmp(&(!b_info.is_floor)))
        });

        for part in sorted_parts {
            let art_id = part.item_id as u32 + 0x4000;
            let info = part_render_info(app, part.item_id);
            let x = (part.x - min_x) as f32;
            let y = (part.y - min_y) as f32;
            let base_x = canvas_w * 0.5 - (y * tile_w) + (x * tile_w) + info.height_delta as f32;
            let base_y = 80.0 + (y * tile_h) + (x * tile_h) + info.width_delta as f32 + 64.0
                + info.height_delta as f32 - (part.original_z as f32 * 4.0);

            if let Some(handle) = app.get_tex_art_cc_texture(ctx, art_id) {
                let size = handle.size_vec2();
                let draw_pos = rect.min + egui::vec2(
                    base_x + info.offset_x as f32,
                    base_y + info.offset_y as f32,
                );
                let part_rect = egui::Rect::from_min_size(draw_pos, size);
                painter.image(
                    handle.id(),
                    part_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let fallback_pos = rect.min + egui::vec2(base_x, base_y);
                let part_rect = egui::Rect::from_center_size(
                    fallback_pos,
                    egui::vec2(14.0, 14.0),
                );
                painter.rect_filled(part_rect.shrink(1.0), 2.0, egui::Color32::BLUE);
            }
        }
    });
}

fn part_render_info(app: &UopInspectorApp, item_id: u16) -> PartRenderInfo {
    let mut info = PartRenderInfo::default();

    if let Some(client) = &app.client_data {
        if let Some(tile) = client.tiledata.item_tiles().get(item_id as usize) {
            info.is_floor = tile.flags.surface() && !tile.flags.bridge();
        }
    }

    if let Some(entries) = &app.ec_tileart_entries {
        if let Some(file) = entries.iter().find(|file| file.entry.tile_id == item_id as u32) {
            info.width_delta = file.entry.cc_img_offset.y_start - file.entry.cc_img_offset.y_end;
            info.height_delta = file.entry.cc_img_offset.x_start;
            info.offset_x = file.entry.cc_img_offset.x_off;
            info.offset_y = file.entry.cc_img_offset.y_off;
        }
    }

    info
}
