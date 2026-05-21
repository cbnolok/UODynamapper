use crate::app::UopInspectorApp;
use eframe::egui;

pub fn ui_animdata(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(animdata) = app
        .client_data
        .as_ref()
        .and_then(|client| client.animdata.clone())
    else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select a Classic Client path containing animdata.mul.");
            });
        });
        return;
    };

    if animdata
        .get(app.selected_animdata_id)
        .map(|entry| !entry.is_active())
        .unwrap_or(true)
    {
        if let Some(entry) = animdata.active_entries().next() {
            app.selected_animdata_id = entry.id;
        }
    }

    egui::SidePanel::left("animdata_list")
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.heading(format!(
                "animdata.mul ({} active / {} records)",
                animdata.active_count(),
                animdata.entries.len()
            ));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            let query = app.search_query.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in animdata.active_entries() {
                    if !query.is_empty() && !entry.id.to_string().contains(&query) {
                        continue;
                    }

                    let label = format!(
                        "{}: {} frames @ {}",
                        entry.id, entry.frame_count, entry.frame_interval
                    );
                    if ui
                        .selectable_label(app.selected_animdata_id == entry.id, label)
                        .clicked()
                    {
                        app.selected_animdata_id = entry.id;
                    }
                }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        let Some(entry) = animdata.get(app.selected_animdata_id) else {
            ui.centered_and_justified(|ui| {
                ui.label("Select an animdata entry from the left panel.");
            });
            return;
        };

        ui.heading(format!("AnimData {}", entry.id));
        ui.separator();
        egui::Grid::new("animdata_header_grid").striped(true).show(ui, |ui| {
            ui.label("Chunk header");
            ui.monospace(format!("0x{:08X}", entry.chunk_header as u32));
            ui.end_row();
            ui.label("Unknown");
            ui.label(format!("0x{:02X}", entry.unknown));
            ui.end_row();
            ui.label("Frame count");
            ui.label(entry.frame_count.to_string());
            ui.end_row();
            ui.label("Frame interval");
            ui.label(entry.frame_interval.to_string());
            ui.end_row();
            ui.label("Frame start");
            ui.label(entry.frame_start.to_string());
            ui.end_row();
        });

        ui.separator();
        ui.heading("Frame Offsets");
        egui::Grid::new("animdata_frames_grid").striped(true).show(ui, |ui| {
            ui.label("Index");
            ui.label("Offset");
            ui.label("Tile ID");
            ui.label("Art");
            ui.end_row();

            for index in 0..entry.frame_count as usize {
                let Some(offset) = entry.frame_offset(index) else {
                    continue;
                };
                let tile_id = entry.id as i32 + offset as i32;
                ui.label(index.to_string());
                ui.label(offset.to_string());
                ui.label(tile_id.to_string());

                if tile_id >= 0 {
                    let art_id = tile_id as u32 + 0x4000;
                    if let Some(handle) = app.get_tex_art_cc_texture(ctx, art_id) {
                        ui.add(egui::Image::new(&handle).fit_to_exact_size(egui::vec2(36.0, 36.0)));
                    } else {
                        ui.label("-");
                    }
                } else {
                    ui.label("-");
                }
                ui.end_row();
            }
        });

        if !animdata.trailing_bytes.is_empty() {
            ui.separator();
            ui.label(format!("Trailing bytes: {}", animdata.trailing_bytes.len()));
        }
    });
}
