use crate::app::{UopInspectorApp, UpscalePreviewAlgorithm};
use eframe::egui;

pub fn ui_upscale_preview_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    if !app.show_upscale_preview {
        return;
    }

    app.refresh_upscale_preview_textures(ctx);

    let source = app.current_image_preview().cloned();
    let original_texture = app.upscale_original_texture.clone();
    let upscaled_texture = app.upscale_preview_texture.clone();
    let upscaled_size = app.upscale_preview_size;
    let mut algorithm = app.upscale_preview_algorithm;
    let mut scale = app.upscale_preview_scale;
    let mut zoom = app.upscale_preview_zoom;
    let mut open = true;
    let mut filter_changed = false;

    egui::Window::new("Upscale Preview")
        .open(&mut open)
        .resizable(true)
        .default_size([1000.0, 720.0])
        .show(ctx, |ui| {
            let Some(source) = source.as_ref() else {
                ui.label("No image selected.");
                return;
            };

            ui.horizontal(|ui| {
                ui.label(&source.label);
                ui.separator();
                ui.label(format!("Original {}x{}", source.width, source.height));
                if upscaled_size[0] > 0 {
                    ui.label(format!("Upscaled {}x{}", upscaled_size[0], upscaled_size[1]));
                }
            });

            ui.horizontal(|ui| {
                filter_changed |= algorithm_combo(ui, &mut algorithm);
                filter_changed |= scale_combo(ui, algorithm, &mut scale);
                ui.separator();
                ui.monospace(format!("{:?}", algorithm.to_filter(scale)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset Zoom").clicked() {
                        zoom = 1.0;
                    }
                    ui.add(egui::Slider::new(&mut zoom, 0.1..=20.0).logarithmic(true));
                    if ui.button("+").clicked() {
                        zoom = (zoom * 1.2).min(20.0);
                    }
                    if ui.button("-").clicked() {
                        zoom = (zoom / 1.2).max(0.1);
                    }
                    ui.label(format!("{:.1}x", zoom));
                });
            });
            ui.separator();

            egui::ScrollArea::both().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(texture) = &original_texture {
                        ui.vertical(|ui| {
                            ui.label("Original");
                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(
                                source.width as f32 * zoom,
                                source.height as f32 * zoom,
                            )));
                        });
                    }

                    ui.add_space(20.0);

                    if let Some(texture) = &upscaled_texture {
                        ui.vertical(|ui| {
                            ui.label("Upscaled");
                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(
                                upscaled_size[0] as f32 * zoom,
                                upscaled_size[1] as f32 * zoom,
                            )));
                        });
                    }
                });
            });
        });

    if !open {
        app.show_upscale_preview = false;
    }
    let zoom_changed = (app.upscale_preview_zoom - zoom).abs() > f32::EPSILON;
    if filter_changed
        || app.upscale_preview_algorithm != algorithm
        || app.upscale_preview_scale != scale
    {
        app.upscale_preview_algorithm = algorithm;
        app.upscale_preview_scale = scale;
        app.upscale_preview_texture_key = None;
        ctx.request_repaint();
    }
    if zoom_changed {
        app.upscale_preview_zoom = zoom;
    }
}

fn algorithm_combo(ui: &mut egui::Ui, algorithm: &mut UpscalePreviewAlgorithm) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("uocf_upscale_preview_algorithm")
        .selected_text(algorithm.label())
        .width(180.0)
        .show_ui(ui, |ui| {
            for candidate in UpscalePreviewAlgorithm::all() {
                if ui
                    .selectable_value(algorithm, *candidate, candidate.label())
                    .changed()
                {
                    changed = true;
                }
            }
        });
    changed
}

fn scale_combo(
    ui: &mut egui::Ui,
    algorithm: UpscalePreviewAlgorithm,
    scale: &mut u32,
) -> bool {
    let options = algorithm.scale_options();
    if !options.contains(&*scale) {
        *scale = options[0];
    }

    let mut changed = false;
    ui.add_enabled_ui(options.len() > 1, |ui| {
        egui::ComboBox::from_id_salt("uocf_upscale_preview_scale")
            .selected_text(format!("{}x", *scale))
            .width(70.0)
            .show_ui(ui, |ui| {
                for candidate in options {
                    if ui
                        .selectable_value(scale, *candidate, format!("{candidate}x"))
                        .changed()
                    {
                        changed = true;
                    }
                }
            });
    });
    changed
}
