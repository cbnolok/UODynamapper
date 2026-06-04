use crate::app::{
    upscale_pass_cli_value, UopInspectorApp, UpscalePreviewAlgorithm, UpscalePreviewPass,
};
use eframe::egui;
use image_postprocess::upscaling::UpscaleFilter;

pub fn ui_upscale_preview_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    if !app.show_upscale_preview {
        return;
    }

    app.refresh_upscale_preview_textures(ctx);

    let source = app.current_image_preview().cloned();
    let original_texture = app.upscale_original_texture.clone();
    let upscaled_texture = app.upscale_preview_texture.clone();
    let upscaled_size = app.upscale_preview_size;
    let elapsed_ms = app.upscale_preview_elapsed_ms;
    let status = app.upscale_preview_status.clone();
    let is_computing = app.upscale_preview_worker_key.is_some();
    let mut passes = app.upscale_preview_passes.clone();
    let mut zoom = app.upscale_preview_zoom;
    let mut open = true;
    let mut original_frame_open = true;
    let mut upscaled_frame_open = true;
    let mut passes_changed = false;

    egui::Window::new("Upscale Preview")
        .open(&mut open)
        .resizable(true)
        .default_size([760.0, 320.0])
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
                if let Some(elapsed_ms) = elapsed_ms {
                    ui.label(format!("{} ms", elapsed_ms));
                    if !status.is_empty() {
                        ui.label(status.as_str());
                    }
                } else if is_computing {
                    ui.label("Computing...");
                } else if !status.is_empty() {
                    ui.label(status.as_str());
                }
            });

            ui.separator();
            passes_changed |= pass_controls(ui, &mut passes);

            let filters = filters_for_passes(&passes);
            let enum_values = enum_pass_values(&passes);
            let cli_values = cli_pass_values(&passes);
            ui.horizontal(|ui| {
                ui.label("Passes:");
                ui.monospace(enum_values.as_str());
                if ui.button("Copy Enums").clicked() {
                    ui.ctx().copy_text(enum_values.clone());
                }
                if ui.button("Copy CLI Values").clicked() {
                    ui.ctx().copy_text(cli_values.clone());
                }
                ui.label(format!("Total {}x", total_scale(&filters)));
            });

            ui.horizontal(|ui| {
                ui.label("CLI:");
                ui.monospace(cli_values.as_str());
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("-").clicked() {
                    zoom = (zoom / 1.2).max(0.1);
                }
                if ui.button("+").clicked() {
                    zoom = (zoom * 1.2).min(20.0);
                }
                ui.add(egui::Slider::new(&mut zoom, 0.1..=20.0).logarithmic(true));
                if ui.button("Reset Zoom").clicked() {
                    zoom = 1.0;
                }
                ui.label(format!("{:.1}x", zoom));
            });
        });

    if let Some(source) = source.as_ref() {
        egui::Window::new("Upscale Preview Original")
            .open(&mut original_frame_open)
            .resizable(true)
            .default_size([420.0, 520.0])
            .show(ctx, |ui| {
                egui::ScrollArea::both()
                    .id_salt("uocf_upscale_preview_original_image")
                    .show(ui, |ui| {
                        ui.label(format!("Original {}x{}", source.width, source.height));
                        if let Some(texture) = &original_texture {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(
                                source.width as f32 * zoom,
                                source.height as f32 * zoom,
                            )));
                        }
                    });
            });

        egui::Window::new("Upscale Preview Upscaled")
            .open(&mut upscaled_frame_open)
            .resizable(true)
            .default_size([520.0, 620.0])
            .show(ctx, |ui| {
                egui::ScrollArea::both()
                    .id_salt("uocf_upscale_preview_upscaled_image")
                    .show(ui, |ui| {
                        if upscaled_size[0] > 0 {
                            ui.label(format!(
                                "Upscaled {}x{}",
                                upscaled_size[0], upscaled_size[1]
                            ));
                        } else {
                            ui.label("Upscaled");
                        }
                        if let Some(texture) = &upscaled_texture {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(
                                upscaled_size[0] as f32 * zoom,
                                upscaled_size[1] as f32 * zoom,
                            )));
                        } else if is_computing {
                            ui.label("Computing...");
                        } else if !status.is_empty() {
                            ui.label(status.as_str());
                        }
                    });
            });
    }

    if !open || !original_frame_open || !upscaled_frame_open {
        app.show_upscale_preview = false;
    }

    if passes_changed || app.upscale_preview_passes != passes {
        app.upscale_preview_passes = passes;
        app.upscale_preview_texture = None;
        app.upscale_preview_texture_key = None;
        app.upscale_preview_worker_key = None;
        app.upscale_preview_worker_rx = None;
        app.upscale_preview_size = [0, 0];
        app.upscale_preview_elapsed_ms = None;
        app.upscale_preview_status.clear();
        ctx.request_repaint();
    }

    if (app.upscale_preview_zoom - zoom).abs() > f32::EPSILON {
        app.upscale_preview_zoom = zoom;
    }
}

fn pass_controls(ui: &mut egui::Ui, passes: &mut Vec<UpscalePreviewPass>) -> bool {
    if passes.is_empty() {
        passes.push(UpscalePreviewPass::default());
    }

    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.button("Add Pass").clicked() {
            passes.push(UpscalePreviewPass::default());
            changed = true;
        }
        if ui.button("Reset").clicked() {
            passes.clear();
            passes.push(UpscalePreviewPass::default());
            changed = true;
        }
    });

    let mut remove_index = None;
    let mut move_up_index = None;
    let mut move_down_index = None;
    egui::Grid::new("uocf_upscale_preview_pass_grid")
        .num_columns(6)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("#");
            ui.label("Algorithm");
            ui.label("Scale/Tune");
            ui.label("Filter");
            ui.label("Order");
            ui.label("");
            ui.end_row();

            let pass_count = passes.len();
            for index in 0..pass_count {
                ui.label((index + 1).to_string());
                let pass = &mut passes[index];
                let old_algorithm = pass.algorithm;
                changed |= algorithm_combo(ui, index, &mut pass.algorithm);
                if old_algorithm != pass.algorithm {
                    pass.clamp_scale();
                    changed = true;
                }
                changed |= tunable_combo(ui, index, pass.algorithm, &mut pass.scale);
                ui.monospace(pass.display_value());

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(index > 0, egui::Button::new("Up"))
                        .clicked()
                    {
                        move_up_index = Some(index);
                    }
                    if ui
                        .add_enabled(index + 1 < pass_count, egui::Button::new("Down"))
                        .clicked()
                    {
                        move_down_index = Some(index);
                    }
                });
                if ui
                    .add_enabled(pass_count > 1, egui::Button::new("Remove"))
                    .clicked()
                {
                    remove_index = Some(index);
                }
                ui.end_row();
            }
        });

    if let Some(index) = remove_index {
        passes.remove(index);
        changed = true;
    }
    if let Some(index) = move_up_index {
        passes.swap(index, index - 1);
        changed = true;
    }
    if let Some(index) = move_down_index {
        passes.swap(index, index + 1);
        changed = true;
    }

    changed
}

fn algorithm_combo(
    ui: &mut egui::Ui,
    index: usize,
    algorithm: &mut UpscalePreviewAlgorithm,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(format!("uocf_upscale_preview_algorithm_{index}"))
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

fn tunable_combo(
    ui: &mut egui::Ui,
    index: usize,
    algorithm: UpscalePreviewAlgorithm,
    scale: &mut u32,
) -> bool {
    let options = algorithm.scale_options();
    if !options.contains(&*scale) {
        *scale = options[0];
    }

    let mut changed = false;
    ui.add_enabled_ui(options.len() > 1, |ui| {
        egui::ComboBox::from_id_salt(format!("uocf_upscale_preview_scale_{index}"))
            .selected_text(algorithm.scale_value_label(*scale))
            .width(70.0)
            .show_ui(ui, |ui| {
                for candidate in options {
                    if ui
                        .selectable_value(scale, *candidate, algorithm.scale_value_label(*candidate))
                        .changed()
                    {
                        changed = true;
                    }
                }
            });
    });
    changed
}

fn filters_for_passes(passes: &[UpscalePreviewPass]) -> Vec<UpscaleFilter> {
    passes.iter().map(|pass| pass.filter()).collect()
}

fn enum_pass_values(passes: &[UpscalePreviewPass]) -> String {
    let values = passes
        .iter()
        .copied()
        .filter(|pass| !matches!(pass.filter(), UpscaleFilter::None) || pass.algorithm.is_palette_snap())
        .map(|pass| {
            if pass.algorithm.is_palette_snap() {
                format!("{:?}", pass.algorithm)
            } else {
                format!("{:?}", pass.filter())
            }
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        "None".to_string()
    } else {
        values.join(", ")
    }
}

fn cli_pass_values(passes: &[UpscalePreviewPass]) -> String {
    let values = passes
        .iter()
        .copied()
        .filter(|pass| !matches!(pass.filter(), UpscaleFilter::None) || pass.algorithm.is_palette_snap())
        .map(upscale_pass_cli_value)
        .collect::<Vec<_>>();
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(" ")
    }
}

fn total_scale(filters: &[UpscaleFilter]) -> u32 {
    filters
        .iter()
        .copied()
        .filter(|filter| !matches!(filter, UpscaleFilter::None))
        .fold(1u32, |scale, filter| scale.saturating_mul(filter.scale_factor()))
}
