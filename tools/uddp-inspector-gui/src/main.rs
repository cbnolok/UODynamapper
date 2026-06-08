use color_eyre::eyre;
use eframe::egui;

mod app;
mod models;
mod utils;
mod logic;
mod ui;

use app::InspectorApp;
use models::{ViewMode, VirtualEntryMode};
use ui::mobile_anim::{ui_mobile_anim_cc, ui_mobile_anim_ec};
use utils::open_package_dialog;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("UDDP Package Inspector"),
        ..Default::default()
    };

    eframe::run_native(
        "uddp_inspector",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let app = InspectorApp::new(cc);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| eyre::eyre!("eframe error: {}", e))?;

    Ok(())
}

impl eframe::App for InspectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard_navigation(ctx);

        // Top panel: Package selector
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Open UDDP...").clicked() {
                    if let Some(path) = open_package_dialog(self.settings.last_package_dir.as_deref()) {
                        self.open_package(path);
                    }
                }
                if let Some(path) = &self.package_path {
                    ui.label(egui::RichText::new(path.to_string_lossy()).strong());
                }
            });
        });

        // Left panel: Metadata and filtering
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                self.ui_left_panel(ui);
            });

        let selected_idx = match self.view_mode {
            ViewMode::Package => self.selected_idx,
            ViewMode::Virtual => self.selected_virtual_idx,
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => None,
        };

        // Right panel: Entry/Asset details (only shown if something is selected)
        if let Some(idx) = selected_idx {
            egui::SidePanel::right("right_panel")
                .resizable(true)
                .default_width(450.0)
                .show(ctx, |ui| {
                    self.ui_details(ctx, ui, idx);
                });
        }

        // Central area: The big table
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.package.is_some() {
                match self.view_mode {
                    ViewMode::Package | ViewMode::Virtual => self.render_table(ctx, ui),
                    ViewMode::MobileAnimCc => ui_mobile_anim_cc(self, ctx, ui),
                    ViewMode::MobileAnimEc => ui_mobile_anim_ec(self, ctx, ui),
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a .uddp file to begin inspection.");
                });
            }
        });

        // Optional separate window for large images
        if self.image_window_open {
            self.ui_texture_viewer_window(ctx);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, app::SETTINGS_KEY, &self.settings);
    }
}

impl InspectorApp {
    fn ui_left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Package Info");

        let Some((version_major, version_minor, lookup_mode, file_count, dicts)) = self
            .package
            .as_ref()
            .map(|u_reader| {
                let header = u_reader.header();
                (
                    header.version_major,
                    header.version_minor,
                    u_reader.lookup_mode(),
                    header.file_count,
                    u_reader.dictionary_records(),
                )
            })
        else {
            ui.label("No package loaded.");
            return;
        };

        crate::ui::metadata_grid("header_grid").show(ui, |ui| {
            crate::ui::metadata_label(ui, "Version:");
            ui.label(format!("{}.{}", version_major, version_minor));
            ui.end_row();
            crate::ui::metadata_label(ui, "Lookup Mode:");
            ui.label(format!("{:?}", lookup_mode));
            ui.end_row();
            crate::ui::metadata_label(ui, "Files:");
            ui.label(file_count.to_string());
            ui.end_row();
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // View Mode toggle (only if virtual entries were detected)
        if !self.virtual_entries.is_empty()
            || self.mobile_anim_cc_package.is_some()
            || self.mobile_anim_ec_package.is_some()
        {
            ui.heading("View Mode");
            ui.selectable_value(&mut self.view_mode, ViewMode::Package, "Package");
            if !self.virtual_entries.is_empty() {
                ui.selectable_value(&mut self.view_mode, ViewMode::Virtual, "Virtual");
            }
            if self.view_mode == ViewMode::Virtual && !self.virtual_material_entries.is_empty() {
                ui.add_space(6.0);
                ui.label("Virtual Grouping");
                let old_mode = self.virtual_entry_mode;
                ui.selectable_value(&mut self.virtual_entry_mode, VirtualEntryMode::Entry, "Entry");
                ui.selectable_value(&mut self.virtual_entry_mode, VirtualEntryMode::Material, "Material");
                if self.virtual_entry_mode != old_mode {
                    self.selected_virtual_idx = None;
                    self.clear_preview_state();
                }
            }
            if self.mobile_anim_cc_package.is_some() {
                ui.selectable_value(&mut self.view_mode, ViewMode::MobileAnimCc, "CC Mobile Anim");
            }
            if self.mobile_anim_ec_package.is_some() {
                ui.selectable_value(&mut self.view_mode, ViewMode::MobileAnimEc, "EC Mobile Anim");
            }
            ui.add_space(10.0);
        }

        ui.label("Filter ID / Hash / Name:");
        let resp = ui.text_edit_singleline(&mut self.filter);
        if self.focus_filter {
            resp.request_focus();
            self.focus_filter = false;
        }
        if ui.checkbox(&mut self.hide_empty_entries, "Hide empty entries").changed() {
            self.selected_idx = None;
            self.selected_virtual_idx = None;
            self.clear_preview_state();
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("Dictionaries");
        if dicts.is_empty() {
            ui.label("No dictionaries.");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            crate::ui::metadata_grid("dict_grid").show(ui, |ui| {
                for (dtype, codec, size) in dicts {
                    crate::ui::metadata_label(ui, format!("T{}:", dtype));
                    ui.label(format!("{:?} ({})", codec, crate::utils::format_size(size as u64)));
                    ui.end_row();
                }
            });
        });
    }

    fn ui_texture_viewer_window(&mut self, ctx: &egui::Context) {
        let viewer_data = if let Some((texture, size, label)) = self.image_for_window() {
            Some((texture.clone(), size, label.to_string()))
        } else {
            None
        };

        let mut close_requested = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("texture_viewer"),
            egui::ViewportBuilder::default()
                .with_title("Texture Viewer")
                .with_inner_size([960.0, 720.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_requested = true;
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    let Some((texture, size, label)) = &viewer_data else {
                        ui.label("No image available for the current selection.");
                        return;
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label.as_str()).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Reset").clicked() {
                                self.texture_zoom = 1.0;
                            }
                            if ui.button("Fit").clicked() {
                                let available = ui.available_size();
                                let zoom_x = available.x / size[0] as f32;
                                let zoom_y = (available.y - 10.0) / size[1] as f32;
                                self.texture_zoom = zoom_x.min(zoom_y).clamp(0.1, 1.0);
                            }
                            ui.add(
                                egui::Slider::new(&mut self.texture_zoom, 0.1..=50.0)
                                    .logarithmic(true)
                                    .text("Zoom"),
                            );
                            if ui.button("➕").clicked() {
                                self.texture_zoom = (self.texture_zoom * 1.2).min(50.0);
                            }
                            if ui.button("➖").clicked() {
                                self.texture_zoom = (self.texture_zoom / 1.2).max(0.1);
                            }
                            ui.label(format!("{:.1}x", self.texture_zoom));
                        });
                    });
                    ui.separator();

                    // Handle mouse wheel zoom if hovering over the central panel and holding Command/Ctrl
                    let zoom_delta = ui.input(|i| i.smooth_scroll_delta.y);
                    if zoom_delta != 0.0 && ui.input(|i| i.modifiers.command) {
                        self.texture_zoom =
                            (self.texture_zoom * (zoom_delta * 0.005).exp()).clamp(0.1, 50.0);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Plus)) {
                         self.texture_zoom = (self.texture_zoom * 1.2).min(50.0);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
                         self.texture_zoom = (self.texture_zoom / 1.2).max(0.1);
                    }

                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let zoom = self.texture_zoom;
                            let new_size = egui::vec2(size[0] as f32 * zoom, size[1] as f32 * zoom);
                            ui.add(
                                egui::Image::new(texture)
                                    .maintain_aspect_ratio(true)
                                    .texture_options(egui::TextureOptions::NEAREST)
                                    .fit_to_exact_size(new_size),
                            );
                        });
                });
            },
        );

        if close_requested {
            self.image_window_open = false;
        }
    }
}
